//! `hale lsp` — a stdio Language Server (v1: diagnostics only).
//!
//! The staged design from `notes/build-latency-and-lsp.md`: with
//! `hale check` at ~10 ms whole-program, the server needs no
//! incrementality — every document event re-parses and re-checks
//! the changed file's whole SEED (its directory, per the F.19
//! per-directory model) with the in-memory overlay text, then
//! publishes diagnostics for every file in the seed (publishing
//! empties clears stale squiggles without bookkeeping).
//!
//! Protocol surface v1:
//!   - initialize / initialized / shutdown / exit
//!   - textDocument/didOpen | didChange (full sync) | didSave |
//!     didClose → check + publishDiagnostics
//! Everything else is politely ignored (requests get a null
//! result so clients don't hang).
//!
//! Diagnostics carried: the full `hale check` set — parse errors,
//! type errors, and the advisory warnings (unbounded-alloc survey,
//! hot-path lint, accept/release, blocking-placement...) — each
//! mapped to LSP severity (error → 1, warning → 2) with UTF-16
//! column positions per the LSP default encoding.

use std::collections::BTreeMap;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde_json::{json, Value};

use hale_syntax::ast::Program;

pub fn run_lsp() -> ExitCode {
    let stdin = std::io::stdin();
    let mut reader = stdin.lock();
    let stdout = std::io::stdout();
    let mut writer = stdout.lock();

    // uri-decoded path → live buffer text (the editor's truth;
    // wins over the disk copy for that file).
    let mut overlays: BTreeMap<PathBuf, String> = BTreeMap::new();
    let mut shutdown_requested = false;

    loop {
        let msg = match read_message(&mut reader) {
            Some(m) => m,
            None => break, // EOF — client went away
        };
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let id = msg.get("id").cloned();

        match method {
            "initialize" => {
                let result = json!({
                    "capabilities": {
                        "textDocumentSync": {
                            "openClose": true,
                            "change": 1,           // full-document sync
                            "save": { "includeText": true }
                        },
                        "positionEncoding": "utf-16"
                    },
                    "serverInfo": {
                        "name": "hale-lsp",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                });
                respond(&mut writer, id, result);
            }
            "initialized" => {}
            "shutdown" => {
                shutdown_requested = true;
                respond(&mut writer, id, Value::Null);
            }
            "exit" => {
                return if shutdown_requested {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::from(1)
                };
            }
            "textDocument/didOpen" => {
                if let Some((path, text)) = did_open_params(&msg) {
                    overlays.insert(path.clone(), text);
                    check_and_publish(&mut writer, &path, &overlays);
                }
            }
            "textDocument/didChange" => {
                if let Some((path, text)) = did_change_params(&msg) {
                    overlays.insert(path.clone(), text);
                    check_and_publish(&mut writer, &path, &overlays);
                }
            }
            "textDocument/didSave" => {
                if let Some(path) = text_document_path(&msg) {
                    // includeText is requested; use it when present
                    // (guards against a stale disk read racing the
                    // editor's write).
                    if let Some(text) = msg
                        .pointer("/params/text")
                        .and_then(Value::as_str)
                    {
                        overlays.insert(path.clone(), text.to_string());
                    }
                    check_and_publish(&mut writer, &path, &overlays);
                }
            }
            "textDocument/didClose" => {
                if let Some(path) = text_document_path(&msg) {
                    overlays.remove(&path);
                    // Re-check from disk so remaining files' diags
                    // reflect the on-disk truth again.
                    check_and_publish(&mut writer, &path, &overlays);
                }
            }
            _ => {
                // Unknown REQUESTS (they carry an id) get a null
                // result so the client doesn't hang; notifications
                // are dropped silently.
                if let Some(id) = id {
                    respond(&mut writer, Some(id), Value::Null);
                }
            }
        }
    }
    ExitCode::SUCCESS
}

// ---- transport -------------------------------------------------------

/// Read one Content-Length-framed JSON-RPC message. None on EOF.
fn read_message(reader: &mut impl BufRead) -> Option<Value> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break; // header/body separator
        }
        if let Some(v) = line.strip_prefix("Content-Length:") {
            content_length = v.trim().parse().ok();
        }
        // Content-Type header (rare) is ignored.
    }
    let n = content_length?;
    let mut buf = vec![0u8; n];
    reader.read_exact(&mut buf).ok()?;
    serde_json::from_slice(&buf).ok()
}

fn send(writer: &mut impl Write, v: &Value) {
    let body = v.to_string();
    let _ = write!(writer, "Content-Length: {}\r\n\r\n{}", body.len(), body);
    let _ = writer.flush();
}

fn respond(writer: &mut impl Write, id: Option<Value>, result: Value) {
    send(
        writer,
        &json!({
            "jsonrpc": "2.0",
            "id": id.unwrap_or(Value::Null),
            "result": result
        }),
    );
}

fn notify(writer: &mut impl Write, method: &str, params: Value) {
    send(
        writer,
        &json!({ "jsonrpc": "2.0", "method": method, "params": params }),
    );
}

// ---- params extraction ----------------------------------------------

fn text_document_path(msg: &Value) -> Option<PathBuf> {
    let uri = msg
        .pointer("/params/textDocument/uri")
        .and_then(Value::as_str)?;
    uri_to_path(uri)
}

fn did_open_params(msg: &Value) -> Option<(PathBuf, String)> {
    let path = text_document_path(msg)?;
    let text = msg
        .pointer("/params/textDocument/text")
        .and_then(Value::as_str)?
        .to_string();
    Some((path, text))
}

fn did_change_params(msg: &Value) -> Option<(PathBuf, String)> {
    let path = text_document_path(msg)?;
    // Full sync (change: 1): the last contentChanges entry carries
    // the whole document.
    let changes = msg.pointer("/params/contentChanges")?.as_array()?;
    let text = changes.last()?.get("text")?.as_str()?.to_string();
    Some((path, text))
}

/// `file://` URI → filesystem path, with %XX percent-decoding.
fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let mut out = Vec::with_capacity(rest.len());
    let bytes = rest.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    Some(PathBuf::from(String::from_utf8(out).ok()?))
}

fn path_to_uri(path: &Path) -> String {
    // Minimal percent-encoding: spaces and '%' — hale project paths
    // are overwhelmingly plain; expand if a real client trips.
    let s = path.display().to_string();
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            ' ' => out.push_str("%20"),
            '%' => out.push_str("%25"),
            c => out.push(c),
        }
    }
    format!("file://{}", out)
}

// ---- check + publish -------------------------------------------------

/// Re-parse and re-check the SEED containing `changed`, then publish
/// diagnostics for every .hl file in it (empties clear stale ones).
fn check_and_publish(
    writer: &mut impl Write,
    changed: &Path,
    overlays: &BTreeMap<PathBuf, String>,
) {
    let seed_dir = changed
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    // Gather the seed's .hl files (sorted, mirroring collect_ap_files);
    // the changed file itself is included even if not yet on disk.
    let mut files: Vec<PathBuf> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&seed_dir) {
        for entry in rd.flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) == Some("hl") {
                files.push(p);
            }
        }
    }
    if !files.iter().any(|f| same_file(f, changed)) {
        files.push(changed.to_path_buf());
    }
    files.sort();

    // Parse each file at a distinct base (overlay text wins).
    let mut programs: BTreeMap<PathBuf, Program> = BTreeMap::new();
    let mut sources: BTreeMap<PathBuf, String> = BTreeMap::new();
    let mut file_bases: Vec<(u32, PathBuf, u32)> = Vec::new();
    // path → its own (never merged-span) parse diags.
    let mut parse_diags: BTreeMap<PathBuf, Vec<hale_syntax::Diag>> =
        BTreeMap::new();
    for f in &files {
        let source = overlay_or_disk(f, overlays);
        let source = match source {
            Some(s) => s,
            None => continue,
        };
        let base = file_bases
            .last()
            .map(|(b, _, l)| b + l + 1)
            .unwrap_or(0);
        file_bases.push((base, f.clone(), source.len() as u32));
        match hale_syntax::parse_source_at(&source, base) {
            Ok(p) => {
                programs.insert(f.clone(), p);
            }
            Err(diags) => {
                // Un-shift the spans back to file-local offsets so
                // position mapping below is uniform.
                let local: Vec<_> = diags
                    .into_iter()
                    .map(|d| d.shifted(base.wrapping_neg()))
                    .collect();
                parse_diags.insert(f.clone(), local);
            }
        }
        sources.insert(f.clone(), source);
    }

    // path → published diagnostics (start EMPTY for every file so a
    // clean pass clears old squiggles).
    let mut per_file: BTreeMap<PathBuf, Vec<Value>> = BTreeMap::new();
    for f in files.iter() {
        per_file.insert(f.clone(), Vec::new());
    }
    for (f, diags) in &parse_diags {
        let src = sources.get(f).map(String::as_str).unwrap_or("");
        let out = per_file.entry(f.clone()).or_default();
        for d in diags {
            out.push(diag_to_lsp(d, src));
        }
    }

    // Typecheck only when the whole seed parsed (the bundle needs
    // every program; a parse hole would cascade phantom errors).
    if parse_diags.is_empty() && !programs.is_empty() {
        for prog in programs.values_mut() {
            hale_syntax::json_gen::generate_json_parsers(prog);
            let _ = hale_types::apply_sync_inference(prog);
        }
        let bundle_programs: BTreeMap<String, &Program> = programs
            .iter()
            .map(|(p, prog)| (p.display().to_string(), prog))
            .collect();
        let bundle = hale_types::Bundle {
            programs: bundle_programs,
        };
        let mut diags = hale_types::check_bundle_opts(&bundle, false);
        diags.extend(hale_types::unbounded_alloc_warnings(&bundle, true));
        for d in &diags {
            let off = d.span.start.as_usize() as u32;
            for (base, path, len) in &file_bases {
                if off >= *base && off < base.saturating_add(*len) {
                    if let Some(src) = sources.get(path) {
                        let local = d.clone().shifted(base.wrapping_neg());
                        per_file
                            .entry(path.clone())
                            .or_default()
                            .push(diag_to_lsp(&local, src));
                    }
                    break;
                }
            }
        }
    }

    for (path, diags) in per_file {
        notify(
            writer,
            "textDocument/publishDiagnostics",
            json!({
                "uri": path_to_uri(&path),
                "diagnostics": diags
            }),
        );
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => a == b,
    }
}

fn overlay_or_disk(
    path: &Path,
    overlays: &BTreeMap<PathBuf, String>,
) -> Option<String> {
    if let Some(s) = overlays.get(path) {
        return Some(s.clone());
    }
    // Overlay keys come from URIs (usually canonical); direct-read
    // fallbacks handle the mismatch.
    if let Ok(canon) = path.canonicalize() {
        if let Some(s) = overlays.get(&canon) {
            return Some(s.clone());
        }
    }
    std::fs::read_to_string(path).ok()
}

/// A file-local Diag → LSP diagnostic object with UTF-16 positions.
fn diag_to_lsp(d: &hale_syntax::Diag, src: &str) -> Value {
    let (sl, sc) = offset_to_lsp_pos(src, d.span.start.as_usize());
    let (el, ec) = offset_to_lsp_pos(src, d.span.end.as_usize());
    // A zero-width span still needs a visible range: extend one col.
    let (el, ec) = if (el, ec) <= (sl, sc) { (sl, sc + 1) } else { (el, ec) };
    json!({
        "range": {
            "start": { "line": sl, "character": sc },
            "end":   { "line": el, "character": ec }
        },
        "severity": if d.is_error() { 1 } else { 2 },
        "source": "hale",
        "code": d.kind_str(),
        "message": d.message
    })
}

/// Byte offset → (0-based line, 0-based UTF-16 column).
fn offset_to_lsp_pos(src: &str, offset: usize) -> (u32, u32) {
    let offset = offset.min(src.len());
    let mut line: u32 = 0;
    let mut line_start = 0usize;
    for (i, b) in src.as_bytes().iter().enumerate() {
        if i >= offset {
            break;
        }
        if *b == b'\n' {
            line += 1;
            line_start = i + 1;
        }
    }
    let line_prefix = &src[line_start..offset];
    let col: u32 = line_prefix
        .chars()
        .map(|c| c.len_utf16() as u32)
        .sum();
    (line, col)
}

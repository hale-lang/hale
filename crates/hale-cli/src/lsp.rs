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
                        "positionEncoding": "utf-16",
                        "hoverProvider": true
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
            "textDocument/hover" => {
                let result = hover(&msg, &overlays).unwrap_or(Value::Null);
                respond(&mut writer, id, result);
            }
            // hale-only custom method: the whole seed's bus graph —
            // per subject: publishers, subscribers (locus + handler +
            // placement), payload types, devirt eligibility. Params:
            // { textDocument: { uri } } picking the seed.
            "hale/busGraph" => {
                let result = bus_graph(&msg, &overlays)
                    .unwrap_or_else(|| json!({ "subjects": [] }));
                respond(&mut writer, id, result);
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

// ---- v2: shared seed analysis ---------------------------------------

/// One parsed seed (overlay-aware). Built on demand per request —
/// the ~10 ms front-end makes caching pointless.
struct SeedAnalysis {
    sources: BTreeMap<PathBuf, String>,
    file_bases: Vec<(u32, PathBuf, u32)>,
    programs: BTreeMap<PathBuf, Program>,
    parse_ok: bool,
}

fn analyze_seed(
    changed: &Path,
    overlays: &BTreeMap<PathBuf, String>,
) -> SeedAnalysis {
    let seed_dir = changed
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
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

    let mut sources = BTreeMap::new();
    let mut file_bases: Vec<(u32, PathBuf, u32)> = Vec::new();
    let mut programs = BTreeMap::new();
    let mut parse_ok = true;
    for f in &files {
        let source = match overlay_or_disk(f, overlays) {
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
            Err(_) => {
                parse_ok = false;
            }
        }
        sources.insert(f.clone(), source);
    }
    SeedAnalysis { sources, file_bases, programs, parse_ok }
}

impl SeedAnalysis {
    fn base_of(&self, path: &Path) -> Option<u32> {
        self.file_bases
            .iter()
            .find(|(_, p, _)| same_file(p, path))
            .map(|(b, _, _)| *b)
    }
    fn bundle(&self) -> hale_types::Bundle<'_> {
        hale_types::Bundle {
            programs: self
                .programs
                .iter()
                .map(|(p, prog)| (p.display().to_string(), prog))
                .collect(),
        }
    }
}

/// LSP (0-based line, UTF-16 col) → byte offset.
fn lsp_pos_to_offset(src: &str, line: u32, character: u32) -> usize {
    let mut cur_line = 0u32;
    let mut i = 0usize;
    let bytes = src.as_bytes();
    while cur_line < line && i < bytes.len() {
        if bytes[i] == b'\n' {
            cur_line += 1;
        }
        i += 1;
    }
    // Walk `character` UTF-16 units into the line.
    let mut units = 0u32;
    let line_str = &src[i..];
    for (ci, c) in line_str.char_indices() {
        if units >= character || c == '\n' {
            return i + ci;
        }
        units += c.len_utf16() as u32;
    }
    src.len()
}

// ---- v2: hover -------------------------------------------------------

fn hover(msg: &Value, overlays: &BTreeMap<PathBuf, String>) -> Option<Value> {
    let path = text_document_path(msg)?;
    let line = msg.pointer("/params/position/line")?.as_u64()? as u32;
    let character =
        msg.pointer("/params/position/character")?.as_u64()? as u32;

    let analysis = analyze_seed(&path, overlays);
    let src = analysis.sources.get(&path).or_else(|| {
        let canon = path.canonicalize().ok()?;
        analysis.sources.get(&canon)
    })?.clone();
    let offset = lsp_pos_to_offset(&src, line, character);

    // Token at position (file-local lex; parse errors don't matter).
    let tokens = hale_syntax::lexer::lex(&src).ok()?;
    let idx = tokens.iter().position(|t| {
        t.span.start.as_usize() <= offset && offset < t.span.end.as_usize()
    })?;
    let tok = &tokens[idx];
    let word = match &tok.kind {
        hale_syntax::lexer::TokenKind::Ident(name) => name.clone(),
        _ => return None,
    };

    // Assemble a `::`-joined path around the token.
    let mut lo = idx;
    while lo >= 2
        && matches!(tokens[lo - 1].kind, hale_syntax::lexer::TokenKind::ColonColon)
        && matches!(tokens[lo - 2].kind, hale_syntax::lexer::TokenKind::Ident(_))
    {
        lo -= 2;
    }
    let mut hi = idx;
    while hi + 2 < tokens.len()
        && matches!(tokens[hi + 1].kind, hale_syntax::lexer::TokenKind::ColonColon)
        && matches!(tokens[hi + 2].kind, hale_syntax::lexer::TokenKind::Ident(_))
    {
        hi += 2;
    }
    let mut segs: Vec<String> = Vec::new();
    let mut k = lo;
    while k <= hi {
        if let hale_syntax::lexer::TokenKind::Ident(n) = &tokens[k].kind {
            segs.push(n.clone());
        }
        k += 2;
    }

    let text = hover_text(&analysis, &path, &tokens, idx, &word, &segs)?;
    let (sl, sc) = offset_to_lsp_pos(&src, tok.span.start.as_usize());
    let (el, ec) = offset_to_lsp_pos(&src, tok.span.end.as_usize());
    Some(json!({
        "contents": { "kind": "markdown", "value": text },
        "range": {
            "start": { "line": sl, "character": sc },
            "end":   { "line": el, "character": ec }
        }
    }))
}

fn hover_text(
    analysis: &SeedAnalysis,
    path: &Path,
    tokens: &[hale_syntax::lexer::Token],
    idx: usize,
    word: &str,
    segs: &[String],
) -> Option<String> {
    use hale_syntax::lexer::TokenKind as TK;

    // std:: paths — the stdlib signature table.
    if segs.len() >= 2 && segs[0] == "std" {
        let seg_refs: Vec<&str> = segs.iter().map(String::as_str).collect();
        if let Some(sig) = hale_types::stdlib_surface::signature_for(&seg_refs)
        {
            let params = sig
                .params
                .iter()
                .map(sig_ty_str)
                .collect::<Vec<_>>()
                .join(", ");
            let mut out = format!(
                "```hale\nfn {}({}) -> {}\n```",
                segs.join("::"),
                params,
                sig_ty_str(&sig.ret)
            );
            if let Some(f) = sig.fallible {
                out.push_str(&format!(
                    "\n\n`fallible({})` — address with `or raise` / \
                     `or <substitute>` / `or self.handler(err)`",
                    f
                ));
            }
            return Some(out);
        }
        return Some(format!("`{}` — stdlib surface", segs.join("::")));
    }

    // `self.<field>` — the enclosing locus's param.
    if idx >= 2
        && matches!(tokens[idx - 1].kind, TK::Dot)
        && matches!(tokens[idx - 2].kind, TK::KwSelf)
    {
        if analysis.parse_ok {
            let base = analysis.base_of(path)?;
            let merged = base as usize + tokens[idx].span.start.as_usize();
            let bundle = analysis.bundle();
            let (top, _) = hale_types::resolve::build_top_scope(&bundle);
            for sym in top.symbols.values() {
                if let hale_types::symbol::TopSymbol::Locus(l) = sym {
                    let sp = sym.span();
                    if sp.start.as_usize() <= merged
                        && merged < sp.end.as_usize()
                    {
                        if let Some(p) =
                            l.params.iter().find(|p| p.name == word)
                        {
                            return Some(format!(
                                "```hale\nself.{}: {}\n```\n\nparam of \
                                 `locus {}`",
                                p.name,
                                p.ty.display(),
                                l.name
                            ));
                        }
                    }
                }
            }
        }
        return None;
    }

    // Top-level symbol lookup.
    if !analysis.parse_ok {
        return None;
    }
    let bundle = analysis.bundle();
    let (top, _) = hale_types::resolve::build_top_scope(&bundle);
    let sym = top.lookup(word)?;
    use hale_types::symbol::{TopSymbol, TypeKind};
    let text = match sym {
        TopSymbol::Fn(f) => {
            let params = f
                .params
                .iter()
                .map(|(n, t)| format!("{}: {}", n, t.display()))
                .collect::<Vec<_>>()
                .join(", ");
            let mut out = format!(
                "```hale\nfn {}({}) -> {}\n```",
                f.name,
                params,
                f.ret.display()
            );
            if let Some(e) = &f.fallible {
                out.push_str(&format!(
                    "\n\n`fallible({})` — callers must address the error",
                    e.display()
                ));
            }
            // Enforcement status from the AST decl.
            for prog in analysis.programs.values() {
                for item in &prog.items {
                    if let hale_syntax::ast::TopDecl::Fn(fd) = item {
                        if fd.name.name == f.name {
                            if fd.hot {
                                out.push_str(
                                    "\n\n`@hot` — hot-path lint enforced \
                                     as errors here",
                                );
                            }
                            if let Some(b) = fd.budget {
                                out.push_str(&format!(
                                    "\n\n`@budget(alloc_per_call = {})` — \
                                     compiler-enforced allocation ceiling",
                                    b
                                ));
                            }
                        }
                    }
                }
            }
            out
        }
        TopSymbol::Locus(l) => {
            let mut out = format!("```hale\nlocus {}\n```", l.name);
            if !l.params.is_empty() {
                out.push_str("\n\nparams: ");
                out.push_str(
                    &l.params
                        .iter()
                        .map(|p| format!("`{}: {}`", p.name, p.ty.display()))
                        .collect::<Vec<_>>()
                        .join(", "),
                );
            }
            if let Some((n, t)) = &l.accept_param {
                out.push_str(&format!(
                    "\n\naccepts children: `{}: {}`",
                    n,
                    t.display()
                ));
            }
            if !l.bus_subscribes.is_empty() || !l.bus_publishes.is_empty() {
                out.push_str(&format!(
                    "\n\nbus: {} subscription(s), {} publish(es)",
                    l.bus_subscribes.len(),
                    l.bus_publishes.len()
                ));
            }
            out
        }
        TopSymbol::Type(t) => match &t.kind {
            TypeKind::Struct(fields) => {
                let fs = fields
                    .iter()
                    .map(|f| format!("    {}: {};", f.name, f.ty.display()))
                    .collect::<Vec<_>>()
                    .join("\n");
                format!("```hale\ntype {} {{\n{}\n}}\n```", t.name, fs)
            }
            TypeKind::Enum(vs) => {
                let names = vs
                    .iter()
                    .map(|v| v.name.clone())
                    .collect::<Vec<_>>()
                    .join(" | ");
                format!("```hale\ntype {} = enum {{ {} }}\n```", t.name, names)
            }
            TypeKind::Alias(inner) => format!(
                "```hale\ntype {} = {}\n```",
                t.name,
                inner.display()
            ),
        },
        TopSymbol::Topic(ti) => {
            let mut out = format!(
                "```hale\ntopic {} {{ payload: {}; subject: \"{}\" }}\n```",
                ti.name,
                ti.payload.display(),
                ti.subject
            );
            if let Some(k) = &ti.keyed_by {
                out.push_str(&format!(
                    "\n\nrouted: `keyed_by {}` — subscribers filter with \
                     `where key == …`",
                    k
                ));
            }
            out
        }
        TopSymbol::Interface(i) => {
            let ms = i
                .methods
                .iter()
                .map(|m| {
                    let ps = m
                        .params
                        .iter()
                        .map(|(n, t)| format!("{}: {}", n, t.display()))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("    fn {}({}) -> {};", m.name, ps, m.ret.display())
                })
                .collect::<Vec<_>>()
                .join("\n");
            format!(
                "```hale\ninterface {} {{\n{}\n}}\n```\n\nstructural \
                 satisfaction — any locus with matching methods qualifies",
                i.name, ms
            )
        }
        TopSymbol::Const(c) => {
            format!("```hale\nconst {}: {}\n```", c.name, c.ty.display())
        }
        _ => return None,
    };
    Some(text)
}

fn sig_ty_str(t: &hale_types::stdlib_surface::SigTy) -> &'static str {
    use hale_types::stdlib_surface::SigTy::*;
    match t {
        Int => "Int",
        Uint => "Uint",
        Float => "Float",
        Bool => "Bool",
        Str => "String",
        Bytes => "Bytes",
        BytesMut => "Bytes",
        Decimal => "Decimal",
        Duration => "Duration",
        Time => "Time",
        Unit => "()",
        Any => "…",
        _ => "…",
    }
}

// ---- v2: hale/busGraph ----------------------------------------------

fn bus_graph(
    msg: &Value,
    overlays: &BTreeMap<PathBuf, String>,
) -> Option<Value> {
    let path = text_document_path(msg).or_else(|| {
        // No textDocument param: fall back to the sole open document.
        if overlays.len() == 1 {
            overlays.keys().next().cloned()
        } else {
            None
        }
    })?;
    let analysis = analyze_seed(&path, overlays);
    if !analysis.parse_ok {
        return Some(json!({ "subjects": [], "parseErrors": true }));
    }
    let bundle = analysis.bundle();
    let (top, _) = hale_types::resolve::build_top_scope(&bundle);
    let graph = hale_types::bus_graph::build_bus_graph(&bundle, &top);
    let subjects: Vec<Value> = graph
        .subjects
        .iter()
        .map(|(subject, info)| {
            json!({
                "subject": subject,
                "publishers": info.publishers.iter().map(|p| json!({
                    "locus": p.locus,
                    "payload": p.payload,
                })).collect::<Vec<_>>(),
                "subscribers": info.subscribers.iter().map(|s| json!({
                    "locus": s.locus,
                    "handler": s.handler,
                    "payload": s.payload,
                    "placement": format!("{:?}", s.placement),
                })).collect::<Vec<_>>(),
                "staticDispatchEligible": info.eligible,
                "directCallEligible": info.direct_call_eligible,
                "ineligibleReason": info.ineligible_reason.as_ref()
                    .map(|r| format!("{:?}", r)),
            })
        })
        .collect();
    Some(json!({ "subjects": subjects }))
}

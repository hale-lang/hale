//! `hale lsp` — the Hale language server, compiled into the hale
//! binary (its own crate since 2026-07-19 — same binary, cleaner
//! boundary; the CLI re-exports it as `lsp`).
//!
//! The staged design from `notes/build-latency-and-lsp.md`: with
//! `hale check` at ~10 ms whole-program, the server needs no
//! incrementality — every document event re-parses and re-checks
//! the changed file's whole SEED (its directory, per the F.19
//! per-directory model) with the in-memory overlay text, then
//! publishes diagnostics for every file in the seed (an empty list
//! clears a file's stale squiggles; one bookkeeping map, the files each
//! seed's last publication covered, clears a file that has left the
//! seed's import graph — `check_and_publish`). A run of document events
//! already queued when the server gets to them is applied in order and
//! checked ONCE, never past a request (`next_steps`).
//!
//! A seed's check is published in two stages (F.40 phase 3, X1): the
//! snapshot's typing stage first, everything that needs no model, then
//! the laws judged over the model, which replace the first publication
//! of every file they add a finding to. A publication the client would
//! read as the latest while a newer document event is already queued is
//! discarded, with the rest of its pass (`Superseded`).
//!
//! Protocol surface v1:
//!   - initialize / initialized / shutdown / exit
//!   - textDocument/didOpen | didChange (full sync) | didSave |
//!     didClose → check + publishDiagnostics
//! Everything else is politely ignored (requests get a null
//! result so clients don't hang).
//!
//! Panic containment: a parser or checker panic on a half-typed
//! file must not take the server down with the editor's session. Every
//! message and every publish pass runs under `catch_unwind` (see
//! `contained`); a pass that panics publishes one diagnostic on the file
//! whose event started it ("the compiler hit an internal error on this
//! file: ..."), a request answers with a JSON-RPC internal error, and
//! the loop goes on to the next message.
//!
//! Diagnostics carried: the full `hale check` set — parse errors,
//! type errors, and the advisory warnings (unbounded-alloc survey,
//! hot-path lint, accept/release, blocking-placement...) — each
//! mapped to LSP severity (error → 1, warning → 2) with UTF-16
//! column positions per the LSP default encoding.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde_json::{json, Value};

use hale_frontend::frontend::{retain_owned_advisories, seed_dir_of, LoadMode};
use hale_frontend::parse_cache::ParseCache;
use hale_frontend::snapshot::{unreadable_message, Config, LoadError, Snapshot};
use hale_frontend::source::{Overlay, SourceProvider};
use hale_syntax::ast::Program;

pub fn run_lsp() -> ExitCode {
    let stdout = std::io::stdout();
    serve(std::io::BufReader::new(std::io::stdin()), &mut stdout.lock())
}

/// The editor's live state: uri-decoded path → buffer text (wins over
/// the disk copy for that file), what each seed's last publication
/// covered, and whether `shutdown` was asked.
#[derive(Default)]
struct State {
    overlays: BTreeMap<PathBuf, String>,
    /// Per checked seed (its directory, `seed_key`): the files its last
    /// publication covered, so the next one can clear a file that has
    /// left the seed's graph. The snapshot describes the current graph;
    /// clearing needs what the client saw before.
    published: BTreeMap<PathBuf, BTreeSet<PathBuf>>,
    /// The files of a publish pass a newer document event superseded
    /// before it finished: the next pass checks their seeds too, so a
    /// closed buffer's seed, found only through the closed file, is not
    /// left with the publication the discarded pass never sent.
    pending: Vec<PathBuf>,
    /// Per checked seed (`seed_key`): the last snapshot whose typing
    /// stage ran, which the next pass's typing reuses (F.40 phase 3, X2).
    typed: BTreeMap<PathBuf, Snapshot>,
    shutdown_requested: bool,
}

/// The message loop, over any pair of streams so a test can drive it.
///
/// A reader thread frames stdin into a channel, so the loop can see
/// what else has arrived while it was busy: a check of a large seed
/// takes seconds, and a burst of keystrokes must cost one check, not
/// one per event (F.40 phase 2.4). Each turn takes everything queued and
/// works through it by `next_steps`.
///
/// The reader ends at EOF, at a frame it refuses, or by panicking; the
/// loop finds out when the channel disconnects (after everything sent
/// before it has been handled) and joins it. Only EOF is a clean end:
/// a refused frame or a panic exits non-zero and says so on stderr.
fn serve(reader: impl BufRead + Send + 'static, writer: &mut impl Write) -> ExitCode {
    install_panic_capture();
    let (tx, rx) = std::sync::mpsc::channel();
    let reading = std::thread::spawn(move || -> Result<(), String> {
        let mut reader = reader;
        while let Some(msg) = read_message(&mut reader)? {
            if tx.send(msg).is_err() {
                break;
            }
        }
        Ok(())
    });
    let mut state = State::default();
    let mut queue = VecDeque::new();
    loop {
        if queue.is_empty() {
            match rx.recv() {
                Ok(msg) => queue.push_back(msg),
                Err(_) => {
                    return match reading.join() {
                        Ok(Ok(())) => ExitCode::SUCCESS, // EOF — client went away
                        Ok(Err(why)) => {
                            eprintln!("hale-lsp: {why}; ending the session");
                            ExitCode::from(1)
                        }
                        Err(_) => {
                            eprintln!("hale-lsp: the thread reading the client's messages panicked; ending the session");
                            ExitCode::from(1)
                        }
                    };
                }
            }
        }
        queue.extend(rx.try_iter());
        for step in next_steps(&mut queue) {
            // Whether a document event has arrived at the front of the
            // queue since the run being published was applied: the
            // buffers the pass read are then not the client's any more.
            let mut superseded = || {
                queue.extend(rx.try_iter());
                queue.front().is_some_and(|m| document_event(m).is_some())
            };
            if let Some(code) = run_step(step, &mut state, writer, &mut superseded) {
                return code;
            }
        }
    }
}

/// What a document event does to its buffer.
#[derive(Debug, PartialEq)]
enum Buffer {
    /// The buffer's text is now this (open, change, a save with text).
    Set(String),
    /// Untouched (a save without text: the disk copy is what changed).
    Keep,
    /// Gone (close): the file reads from the disk again.
    Remove,
}

/// One unit of the loop's work.
#[derive(Debug, PartialEq)]
enum Step {
    /// A document event's buffer update, without its check.
    Apply(PathBuf, Buffer),
    /// One publish pass after a run of updates: the seeds of these files
    /// (the run's distinct files, the latest first), then every other
    /// open seed — `check_open_seeds`.
    Publish(Vec<PathBuf>),
    /// Any other message, dispatched as it arrived.
    Handle(Value),
}

/// The steps for the front of `queue`. A document event there starts a
/// run: it and every document event queued right behind it are applied
/// in order, then ONE publish pass checks what the run changed. The run
/// stops at the first message that is not a document event — a request,
/// above all — which is left at the front for the next call.
///
/// The invariant: a request is answered after every publish of the
/// check the notifications before it caused, and messages are never
/// reordered. The run cannot reach past a request, because the request
/// must see the buffers as they stood when it was sent (a hover on text
/// typed after it would answer the wrong question) and its answer is
/// the client's fence for those publishes. Nor past any other
/// notification — `exit` ends the session, and whatever else there is
/// runs where it was sent. Everything a run collapses is a check whose
/// publishes the next pass of the same run would overwrite.
fn next_steps(queue: &mut VecDeque<Value>) -> Vec<Step> {
    let Some(first) = queue.pop_front() else { return Vec::new() };
    let Some(event) = document_event(&first) else { return vec![Step::Handle(first)] };
    let mut steps = vec![Step::Apply(event.0, event.1)];
    while let Some(event) = queue.front().and_then(document_event) {
        queue.pop_front();
        steps.push(Step::Apply(event.0, event.1));
    }
    let mut files: Vec<PathBuf> = Vec::new();
    for step in steps.iter().rev() {
        if let Step::Apply(path, _) = step {
            if !files.contains(path) {
                files.push(path.clone());
            }
        }
    }
    steps.push(Step::Publish(files));
    steps
}

/// A document event's file and what it does to the buffer; `None` for
/// any other message (and for an event missing what its arm reads).
fn document_event(msg: &Value) -> Option<(PathBuf, Buffer)> {
    match msg.get("method").and_then(Value::as_str)? {
        "textDocument/didOpen" => did_open_params(msg).map(|(p, t)| (p, Buffer::Set(t))),
        "textDocument/didChange" => did_change_params(msg).map(|(p, t)| (p, Buffer::Set(t))),
        // includeText is requested; use it when present (guards against
        // a stale disk read racing the editor's write).
        "textDocument/didSave" => {
            let path = text_document_path(msg)?;
            let text = msg.pointer("/params/text").and_then(Value::as_str);
            Some((path, text.map_or(Buffer::Keep, |t| Buffer::Set(t.to_string()))))
        }
        // The pass re-checks from disk, so the remaining files' diagnostics
        // reflect the on-disk truth again — in this seed and in every open
        // seed that imported the buffer.
        "textDocument/didClose" => text_document_path(msg).map(|p| (p, Buffer::Remove)),
        _ => None,
    }
}

/// Carry out one step. `Some` ends the session with that exit code.
/// `superseded` says whether a newer document event is queued (a
/// publish pass asks it before each publication).
fn run_step(
    step: Step,
    state: &mut State,
    writer: &mut impl Write,
    superseded: &mut dyn FnMut() -> bool,
) -> Option<ExitCode> {
    match step {
        Step::Apply(path, Buffer::Set(text)) => {
            state.overlays.insert(path, text);
        }
        Step::Apply(_, Buffer::Keep) => {}
        Step::Apply(path, Buffer::Remove) => {
            state.overlays.remove(&path);
        }
        Step::Publish(mut files) => {
            for p in std::mem::take(&mut state.pending) {
                if !files.contains(&p) {
                    files.push(p);
                }
            }
            if let Err(why) = contained(|| check_open_seeds(writer, &files, state, superseded)) {
                publish_internal_error(writer, &files[0], &why);
            }
        }
        Step::Handle(msg) => match contained(|| dispatch(&msg, state, writer)) {
            Ok(code) => return code,
            Err(why) => report_internal_error(writer, &msg, &why),
        },
    }
    None
}

// ---- panic containment ------------------------------------------------

thread_local! {
    /// True only while `contained` runs a handler on this thread, so
    /// the hook below takes a panic for its own only then.
    static CAPTURING: Cell<bool> = const { Cell::new(false) };
    /// What the hook saw of the panic being contained: message and place.
    static CAPTURED: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Install, once per process, a panic hook that records the message and
/// location of a panic raised inside `contained` and lets every other
/// panic (another thread, a test, anything outside a handler) reach the
/// hook that was there before. The scope is the thread-local flag, not
/// the installation, so nothing has to be swapped back per message.
fn install_panic_capture() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if !CAPTURING.with(Cell::get) {
                previous(info);
                return;
            }
            let payload = info.payload();
            let msg = payload
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "a panic with no message".to_string());
            let at = info
                .location()
                .map(|l| format!("{}:{}", l.file(), l.line()))
                .unwrap_or_else(|| "an unknown place".to_string());
            let text = format!("{msg} (at {at})");
            eprintln!("hale-lsp: internal error, contained: {text}");
            CAPTURED.with(|c| *c.borrow_mut() = Some(text));
        }));
    });
}

/// Run one handler; a panic inside it comes back as its text and the
/// server carries on.
fn contained<R>(f: impl FnOnce() -> R) -> Result<R, String> {
    struct Scope;
    impl Drop for Scope {
        fn drop(&mut self) {
            CAPTURING.with(|c| c.set(false));
        }
    }
    CAPTURED.with(|c| *c.borrow_mut() = None);
    CAPTURING.with(|c| c.set(true));
    let _scope = Scope;
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).map_err(|_| {
        CAPTURED
            .with(|c| c.borrow_mut().take())
            .unwrap_or_else(|| "a panic with no message".to_string())
    })
}

/// The text the editor shows for a contained panic.
fn internal_error_message(why: &str) -> String {
    format!(
        "the compiler hit an internal error on this file: {why}; please report it with the file"
    )
}

/// Tell the client about a contained panic in a publish pass without
/// ending the session: one diagnostic on the file whose event started
/// the pass, the place the editor was already looking. The publish
/// replaces the file's diagnostics, the right trade for the check that
/// just failed.
fn publish_internal_error(writer: &mut impl Write, path: &Path, why: &str) {
    notify(
        writer,
        "textDocument/publishDiagnostics",
        json!({
            "uri": path_to_uri(path),
            "diagnostics": [{
                "range": {
                    "start": { "line": 0, "character": 0 },
                    "end": { "line": 0, "character": 1 }
                },
                "severity": 1,
                "source": "hale",
                "message": internal_error_message(why)
            }]
        }),
    );
}

/// Tell the client about a contained panic in a message's handler: a
/// request answers with a JSON-RPC internal error, so the client is not
/// left waiting (a publish would be the wrong trade for a hover); a
/// notification has no one to answer.
fn report_internal_error(writer: &mut impl Write, msg: &Value, why: &str) {
    let message = internal_error_message(why);
    if let Some(id) = msg.get("id").cloned() {
        send(
            writer,
            &json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32603, "message": message }
            }),
        );
    }
}

/// Handle one message. `Some` ends the session with that exit code.
fn dispatch(
    msg: &Value,
    state: &mut State,
    mut writer: &mut impl Write,
) -> Option<ExitCode> {
    let overlays = &mut state.overlays;
    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
    let id = msg.get("id").cloned();

    // Test-only: a handler that panics, for the containment test.
    #[cfg(test)]
    if method == "hale/testPanic" {
        panic!("injected handler panic");
    }

    {
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
                        "hoverProvider": true,
                        "definitionProvider": true,
                        "referencesProvider": true,
                        "completionProvider": {
                            "triggerCharacters": [".", ":"]
                        },
                        "documentFormattingProvider": true,
                        "documentSymbolProvider": true
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
                state.shutdown_requested = true;
                respond(&mut writer, id, Value::Null);
            }
            "exit" => {
                return Some(if state.shutdown_requested {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::from(1)
                });
            }
            // The document events never reach here: `next_steps` applies
            // them and runs their publish pass.
            "textDocument/completion" => {
                let result = completion(&msg, &overlays)
                    .unwrap_or_else(|| json!({
                        "isIncomplete": false, "items": []
                    }));
                respond(&mut writer, id, result);
            }
            "textDocument/formatting" => {
                let result = formatting(&msg, &overlays)
                    .unwrap_or(Value::Null);
                respond(&mut writer, id, result);
            }
            "textDocument/documentSymbol" => {
                let result = document_symbols(&msg, &overlays)
                    .unwrap_or_else(|| json!([]));
                respond(&mut writer, id, result);
            }
            // hale-only: per-fn enforcement map — every user fn and
            // locus method with its @hot / @budget / fallible /
            // @unbounded status. Params: { textDocument: { uri } }.
            "hale/enforcement" => {
                let result = enforcement(&msg, &overlays)
                    .unwrap_or_else(|| json!({ "fns": [] }));
                respond(&mut writer, id, result);
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
            "textDocument/definition" => {
                let result = definition(&msg, &overlays).unwrap_or(Value::Null);
                respond(&mut writer, id, result);
            }
            "textDocument/references" => {
                let result = references(&msg, &overlays)
                    .unwrap_or_else(|| json!([]));
                respond(&mut writer, id, result);
            }
            // hale-only: the main locus's placement map — every params
            // field with its resolved placement spec + constraints
            // (unlisted fields default to cooperative(pool = main)).
            "hale/placement" => {
                let result = placement(&msg, &overlays)
                    .unwrap_or_else(|| json!({ "fields": [] }));
                respond(&mut writer, id, result);
            }
            // hale-only: the allocation-bound survey — the leak sites
            // the default-on unbounded-alloc analysis reports, with
            // positions, plus the full text dump.
            "hale/allocSummary" => {
                let result = alloc_summary(&msg, &overlays)
                    .unwrap_or_else(|| json!({ "leakSites": [] }));
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
    None
}

// ---- transport -------------------------------------------------------

/// The largest frame body the server reads, in bytes. A document is
/// sent whole on every change, so a frame is one source file plus its
/// envelope; 64 MiB is far past any seed file, and a `Content-Length`
/// above it is a corrupt or hostile stream, refused rather than
/// allocated.
const MAX_FRAME_BYTES: usize = 64 << 20;

/// Read one Content-Length-framed JSON-RPC message. `Ok(None)` on EOF;
/// `Err` for a `Content-Length` that does not parse or exceeds
/// [`MAX_FRAME_BYTES`], whose body is never read (there is no way to
/// skip it and find the next frame).
fn read_message(reader: &mut impl BufRead) -> Result<Option<Value>, String> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => return Ok(None),
            Ok(_) => {}
        }
        let line = line.trim_end();
        if line.is_empty() {
            break; // header/body separator
        }
        if let Some(v) = line.strip_prefix("Content-Length:") {
            let n: usize = v
                .trim()
                .parse()
                .map_err(|_| format!("a frame's Content-Length `{}` is not a byte count", v.trim()))?;
            if n > MAX_FRAME_BYTES {
                return Err(format!(
                    "a frame's Content-Length {n} exceeds the {MAX_FRAME_BYTES}-byte bound; it is refused, not read"
                ));
            }
            content_length = Some(n);
        }
        // Content-Type header (rare) is ignored.
    }
    let Some(n) = content_length else { return Ok(None) };
    let mut buf = vec![0u8; n];
    if reader.read_exact(&mut buf).is_err() {
        return Ok(None);
    }
    Ok(serde_json::from_slice(&buf).ok())
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

/// Is this path inside the materialized stdlib cache
/// (`<cache>/hale/stdlib-<version>/`)? Those files are read-only
/// jump targets, not user seeds: analyzed standalone they spray
/// spurious errors over correct stdlib code (mangled declarations
/// and path-call primitives only resolve in the merged program),
/// so the LSP publishes no diagnostics for them.
fn is_stdlib_cache_path(path: &Path) -> bool {
    let canon = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut comps = canon.components().rev();
    let _file = comps.next();
    let (Some(ver_dir), Some(hale_dir)) = (comps.next(), comps.next())
    else {
        return false;
    };
    hale_dir.as_os_str() == "hale"
        && ver_dir
            .as_os_str()
            .to_string_lossy()
            .starts_with("stdlib-")
}

/// The publish pass of a run of document events that touched `changed`
/// (`next_steps`; the latest first): check each of their seeds, then
/// every OTHER seed an open buffer sits in, each through
/// `check_and_publish`. The snapshot reads an imported file from its
/// buffer, so editing a library's buffer changes the program of every
/// open seed that imports it, and closing that buffer changes it again
/// (the importer then reads the disk copy). Every open seed is
/// rechecked, not only the ones that import `changed`: a
/// reverse-dependency index is phase 3's incremental load to build. A
/// closed buffer's seed is checked through the closed file, since it may
/// have no open buffer left to be found by.
///
/// A pass a newer document event supersedes stops where it is, its
/// publication discarded and its files left for the next pass
/// (`State::pending`), which rechecks every open seed anyway.
fn check_open_seeds(
    writer: &mut impl Write,
    changed: &[PathBuf],
    state: &mut State,
    superseded: &mut dyn FnMut() -> bool,
) {
    // Test-only: a check that panics, for the containment test.
    #[cfg(test)]
    if changed.iter().any(|p| state.overlays.get(p).is_some_and(|t| t.contains("hale-lsp-test-panic"))) {
        panic!("injected checker panic");
    }
    // (seed key, the file it is checked through): the changed files'
    // first, then each other seed through one of its open buffers, so a
    // file-level diagnostic lands on a file the editor has open.
    let mut seeds: Vec<(PathBuf, PathBuf)> = Vec::new();
    for path in changed.iter().chain(state.overlays.keys().filter(|p| !is_stdlib_cache_path(p))) {
        let key = seed_key(path);
        if !seeds.iter().any(|(k, _)| *k == key) {
            seeds.push((key, path.clone()));
        }
    }
    let checked: BTreeSet<PathBuf> = seeds.iter().map(|(k, _)| k.clone()).collect();
    for (_, via) in &seeds {
        if let Pass::Superseded = check_and_publish(writer, via, &state.overlays, &mut state.published, &mut state.typed, &checked, superseded) {
            state.pending = changed.to_vec();
            return;
        }
    }
}

/// How a seed's publish pass ended.
enum Pass {
    /// Every publication of the seed's check was sent.
    Done,
    /// A newer document event was queued before the next publication,
    /// which was discarded (the first, or the laws').
    Superseded,
}

/// Check the seed of `changed` and publish it: every file the check
/// placed a list on, and, EMPTY, every file the last publication for
/// this seed covered that this one does not — a library the seed no
/// longer imports, or one it no longer reaches because a parse hole
/// stops the imports from being followed. A client keeps a URI's
/// diagnostics until that URI is published again, so what is cleared
/// is decided by what the client was sent (`published`, keyed by the
/// seed's directory), not by the graph the snapshot now describes.
///
/// A file whose own seed is among `checked` (the seeds this event
/// checks) is that seed's to publish, and this one neither publishes
/// nor clears it: the passes of one event would otherwise overwrite
/// each other's answer for one file — an importer drops a library's
/// own advisories, and would clear them.
///
/// A seed the snapshot checks is published twice (F.40 phase 3, X1).
/// The first publication is the check's typing stage, everything that
/// needs no model, with the clearing above. The second is the whole
/// check, the laws after the typing, sent only for the files whose list
/// it changes: the laws add a finding, never take one away, so a file's
/// final list is its first followed by the laws placed in it, and every
/// file's last publication is what `hale check` reports for it. A seed
/// with no law, or none broken, gets one publication, as before the
/// stages. A seed the snapshot does not check (a load the import graph
/// refused, a refusal, a member that did not parse or read, the stdlib
/// cache) gets its one publication.
///
/// Before each publication, and before the laws are judged, the pass
/// asks `superseded`: once a newer document event is queued, the
/// buffers this check read are not the client's any more, so what is
/// left unsent is discarded (the second publication alone, or both),
/// `published` keeps what was sent, and the next pass rechecks.
///
/// The typing stage reuses the seed's last typed snapshot (F.40 phase 3,
/// X2, `Snapshot::reusing_typing`): `typed` holds one per seed, the
/// snapshot this pass typed replaces it, and a pass that typed nothing (a
/// hole, a refused load) leaves it for the next.
fn check_and_publish(
    writer: &mut impl Write,
    changed: &Path,
    overlays: &BTreeMap<PathBuf, String>,
    published: &mut BTreeMap<PathBuf, BTreeSet<PathBuf>>,
    typed: &mut BTreeMap<PathBuf, Snapshot>,
    checked: &BTreeSet<PathBuf>,
    superseded: &mut dyn FnMut() -> bool,
) -> Pass {
    let own = seed_key(changed);
    let (first, snap) = match seed_typing(changed, overlays, typed.remove(&own)) {
        SeedCheck::Once(per_file, previous) => {
            typed.extend(previous.map(|p| (own.clone(), p)));
            (per_file, None)
        }
        SeedCheck::Staged(snap, per_file) => (per_file, Some(snap)),
    };
    let pass = publish_stages(writer, &own, first, snap.as_ref(), published, checked, superseded);
    if let Some(snap) = snap {
        typed.insert(own, snap);
    }
    pass
}

/// The publications of [`check_and_publish`], from the seed's typing
/// stage (`first`) and the snapshot the laws are judged from.
fn publish_stages(
    writer: &mut impl Write,
    own: &Path,
    mut first: BTreeMap<PathBuf, Vec<Value>>,
    snap: Option<&Snapshot>,
    published: &mut BTreeMap<PathBuf, BTreeSet<PathBuf>>,
    checked: &BTreeSet<PathBuf>,
    superseded: &mut dyn FnMut() -> bool,
) -> Pass {
    let own = own.to_path_buf();
    let ours = |p: &Path| {
        let key = seed_key(p);
        key == own || !checked.contains(&key)
    };
    first.retain(|p, _| ours(p));
    if superseded() {
        return Pass::Superseded;
    }
    let covered: BTreeSet<PathBuf> = first.keys().cloned().collect();
    let before = published.insert(own.clone(), covered).unwrap_or_default();
    let mut sent = first.clone();
    for gone in before.into_iter().filter(|p| ours(p)) {
        sent.entry(gone).or_default();
    }
    publish_all(writer, sent);
    let Some(snap) = snap else { return Pass::Done };
    if superseded() {
        return Pass::Superseded;
    }
    let mut last = seed_laws(snap);
    last.retain(|p, diags| ours(p) && first.get(p) != Some(diags));
    if superseded() {
        return Pass::Superseded;
    }
    published.entry(own).or_default().extend(last.keys().cloned());
    publish_all(writer, last);
    Pass::Done
}

/// A seed's key in the server's memory: its directory, canonical when
/// it is on disk, so two spellings of one directory are one seed.
fn seed_key(file: &Path) -> PathBuf {
    let dir = seed_dir_of(file);
    dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf())
}

/// A seed's check as its publications carry it: path → the diagnostics
/// to publish on it, an EMPTY list for every seed file found clean.
enum SeedCheck {
    /// The one publication of a seed the snapshot does not check, and
    /// the previous typed snapshot it was offered, unused.
    Once(BTreeMap<PathBuf, Vec<Value>>, Option<Snapshot>),
    /// The first publication of a seed it checks, the typing stage's,
    /// and the snapshot the laws are judged from ([`seed_laws`]).
    Staged(Snapshot, BTreeMap<PathBuf, Vec<Value>>),
}

/// The seed of `changed`, through the check's typing stage, which
/// reuses what it can of `previous`, the seed's last typed snapshot.
fn seed_typing(changed: &Path, overlays: &BTreeMap<PathBuf, String>, previous: Option<Snapshot>) -> SeedCheck {
    // A file inside the stdlib cache gets an EMPTY publish — it is
    // a definition-jump target, not a seed member, and clearing
    // (rather than skipping) removes anything a client already
    // showed for it.
    if is_stdlib_cache_path(changed) {
        return SeedCheck::Once(BTreeMap::from([(changed.to_path_buf(), Vec::new())]), previous);
    }
    // F.40 phase 2.3: the seed as `hale check <dir>` loads it — the
    // file's directory and every seed its imports reach — read through
    // the buffers over the disk (`LoadMode::Editor`), shaped and minted
    // once, and the check demanded from it. The editor's config holds
    // the whole-program rules (GH #721) and the build rules `hale
    // check` runs beside its check: the snapshot checks only a seed
    // whose every member read and parsed, so it answers `hale check
    // <dir>` exactly — including an identifier that binds nothing, a
    // typo the editor shows while it is typed. The first stage needs no
    // model; the second, `seed_laws`, demands it only for a program
    // that declares a law.
    // path → published diagnostics (start EMPTY for every file so a
    // clean pass clears old squiggles).
    let mut per_file: BTreeMap<PathBuf, Vec<Value>> = BTreeMap::new();
    let snap = match editor_snapshot(changed, overlays) {
        Ok(snap) => snap,
        // The import graph refused the seed (an import that does not
        // resolve, a library that does not parse or read): what `hale
        // check` prints, placed as it would place it.
        Err(LoadError::Load(f)) => {
            for (_, p, _) in &f.file_bases {
                per_file.insert(p.clone(), Vec::new());
            }
            per_file.entry(changed.to_path_buf()).or_default();
            place_checker_diags(&f.diags, &f.file_bases, &f.sources, &mut per_file);
            for io in &f.io {
                publish_file_level(&mut per_file, &io.path, changed, &io.text);
            }
            return SeedCheck::Once(per_file, previous);
        }
        Err(LoadError::Refused(msg)) => {
            per_file.entry(changed.to_path_buf()).or_default().push(file_level_diag(&msg));
            return SeedCheck::Once(per_file, previous);
        }
    };
    let snap = match previous {
        Some(previous) => snap.reusing_typing(previous),
        None => snap,
    };
    let typed = match snap.demand_typing() {
        // The editor's config carries the build rules and the allocation
        // advisory in the typing stage.
        Ok(typed) => placed(&snap, &typed.diags),
        // A seed with a member that did not parse or read is not checked
        // (a hole would cascade phantom errors), as `hale check` checks
        // none. Its parse diagnostics are published against the files
        // that hold them, un-shifted to file-local offsets; a member
        // that would not read is a file-level diagnostic against itself
        // and against the file being edited, which is open.
        Err(_) => {
            let (sources, file_bases) = (snap.sources(), snap.file_bases());
            for f in snap.files() {
                per_file.insert(f.clone(), Vec::new());
            }
            for (f, diags) in snap.unparsed() {
                let base = file_bases
                    .iter()
                    .find(|(_, p, _)| p == f)
                    .map_or(0, |(b, _, _)| *b);
                let src = sources.get(f).map(String::as_str).unwrap_or("");
                let out = per_file.entry(f.clone()).or_default();
                for d in diags {
                    out.push(diag_to_lsp(&d.clone().shifted(base.wrapping_neg()), src));
                }
            }
            for (f, os_error) in snap.unreadable() {
                publish_file_level(&mut per_file, f, changed, &unreadable_message(f, os_error));
            }
            // The hole keeps the last typed snapshot for the next pass.
            return SeedCheck::Once(per_file, snap.take_previous());
        }
    };
    SeedCheck::Staged(snap, typed)
}

/// The seed's whole check, the laws judged after its typing stage: what
/// `hale check` reports, placed as the first publication was. Every
/// file's list is its typing-stage list followed by the laws placed in
/// it, since each diagnostic is spelled, suppressed and placed on its
/// own. Blocked never: the typing stage it follows was not.
fn seed_laws(snap: &Snapshot) -> BTreeMap<PathBuf, Vec<Value>> {
    match snap.demand_check() {
        Ok(checked) => placed(snap, &checked.diags),
        Err(_) => BTreeMap::new(),
    }
}

/// `diags` of a checked snapshot, as a publication carries them: what
/// `hale check` does last — every name in the author's spelling, and an
/// advisory about a seed the target imports left to that seed's own
/// check — then each placed on the file that holds it, over an EMPTY
/// list for every file of the seed.
fn placed(snap: &Snapshot, diags: &[hale_syntax::Diag]) -> BTreeMap<PathBuf, Vec<Value>> {
    let mut per_file: BTreeMap<PathBuf, Vec<Value>> =
        snap.files().iter().map(|f| (f.clone(), Vec::new())).collect();
    let mut diags = diags.to_vec();
    hale_types::stdlib_bodies::demangle_imports(&mut diags, snap.import_renames());
    retain_owned_advisories(&mut diags, snap.own_files(), snap.file_bases());
    place_checker_diags(&diags, snap.file_bases(), snap.sources(), &mut per_file);
    per_file
}

/// The snapshot every document event and every request reads: the seed
/// of `changed` as `hale check <dir>` loads it, through the buffers over
/// the disk (`LoadMode::Editor`), under the editor's config. One load
/// per event or request, no snapshot kept: the snapshot's key names a
/// whole load, so it is no key for reusing one member. What is reused is
/// each file's parse, per path and text ([`PARSES`]). A seed
/// whose imports did not link is refused as `hale check` refuses it
/// ([`Snapshot::linked`]); only the outline reads its members
/// ([`editor_load`]).
fn editor_snapshot(
    changed: &Path,
    overlays: &BTreeMap<PathBuf, String>,
) -> Result<Snapshot, LoadError> {
    editor_load(changed, overlays)?.linked().map_err(LoadError::Load)
}

/// The editor's load as it read, a seed whose imports did not link
/// included: its members as they parsed, with their sources and bases,
/// and every family blocked.
fn editor_load(
    changed: &Path,
    overlays: &BTreeMap<PathBuf, String>,
) -> Result<Snapshot, LoadError> {
    Snapshot::load(changed, LoadMode::Editor, &Overlay::new(overlays).reusing(&PARSES), Config::editor())
}

/// The server's parse products, one cache across every load it makes
/// (F.40 phase 3, X1, `hale_frontend::parse_cache`): an edit reparses
/// the edited file, and the seed's other files and every library it
/// imports are reused while their text and the effect-class table they
/// are parsed from stay as they were.
static PARSES: ParseCache = ParseCache::new();

/// A diagnostic about a whole file (one that would not read has no
/// position): the range 0:0–0:0, an error.
fn file_level_diag(message: &str) -> Value {
    json!({
        "range": {
            "start": { "line": 0, "character": 0 },
            "end":   { "line": 0, "character": 0 }
        },
        "severity": 1,
        "source": "hale",
        "code": "io error",
        "message": message
    })
}

/// Publish a file-level diagnostic against `file` and against the file
/// being edited, so an editor that has only that one open still shows it.
fn publish_file_level(
    per_file: &mut BTreeMap<PathBuf, Vec<Value>>,
    file: &Path,
    changed: &Path,
    message: &str,
) {
    per_file.entry(file.to_path_buf()).or_default().push(file_level_diag(message));
    if !same_file(file, changed) {
        per_file.entry(changed.to_path_buf()).or_default().push(file_level_diag(message));
    }
}

fn publish_all(writer: &mut impl Write, per_file: BTreeMap<PathBuf, Vec<Value>>) {
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

/// Demultiplex whole-program (merged-bundle) diagnostics back to the
/// seed files they were raised in, appending each to that file's
/// publish list.
///
/// The window is [`hale_syntax::file_owns_offset`], which owns its
/// one-past-the-last-byte position. GH #805: this loop tested
/// `off < base + len`, so a diagnostic positioned at a file's END —
/// where an end-of-file span sits — matched no window and was
/// dropped without a trace, and the editor showed nothing for a
/// program `hale check` rejects. Parse diagnostics never reach here
/// (they are kept per-file and un-shifted), which is why the hole
/// only ever swallowed checker findings.
fn place_checker_diags(
    diags: &[hale_syntax::Diag],
    file_bases: &[(u32, PathBuf, u32)],
    sources: &BTreeMap<PathBuf, String>,
    per_file: &mut BTreeMap<PathBuf, Vec<Value>>,
) {
    for d in diags {
        // GH #856: a diagnostic raised INSIDE a stdlib body has no
        // seed range at all — its offset measures the embedded
        // stdlib's own parse space. The window test cannot tell, so
        // such a span landed in whichever seed file it numerically
        // collided with, squiggling a line the reader never wrote.
        // There is nothing in the document to point at, so it is
        // not published as a document diagnostic; the finding it
        // belongs to carries its own witness path, and the CLI
        // prints the stdlib location as a note.
        if d.origin == hale_syntax::SpanOrigin::Stdlib {
            continue;
        }
        let off = d.span.start.as_usize() as u32;
        for (base, path, len) in file_bases {
            if !hale_syntax::file_owns_offset(*base, *len, off) {
                continue;
            }
            if let Some(src) = sources.get(path) {
                let local = d.clone().shifted(base.wrapping_neg());
                let mut v = diag_to_lsp(&local, src);
                // Related spans (downstream handoff, 2026-08-11)
                // resolve from the UN-shifted diagnostic — each may
                // live in a different file than the primary — and
                // publish as `relatedInformation`, which clients
                // render as a clickable second location.
                let rel: Vec<Value> = d
                    .related
                    .iter()
                    .filter_map(|r| related_to_lsp(r, file_bases, sources))
                    .collect();
                if !rel.is_empty() {
                    v["relatedInformation"] = json!(rel);
                }
                // A secondary location in the stdlib cannot be a
                // `DiagnosticRelatedInformation` — that carries a
                // range, and there is no seed range to give it. It
                // becomes a note on the message instead, so the
                // reader still learns where the effect happens
                // without the editor jumping into a wrong file.
                let notes = stdlib_related_notes(d);
                if !notes.is_empty() {
                    v["message"] = json!(format!(
                        "{}\n{}",
                        d.message,
                        notes.join("\n")
                    ));
                }
                per_file.entry(path.clone()).or_default().push(v);
            }
            break;
        }
    }
}

/// The `note:` lines a diagnostic's STDLIB-origin related locations
/// render as (GH #856). Empty for the ordinary diagnostic, whose
/// secondary locations are all seed spans and all clickable.
fn stdlib_related_notes(d: &hale_syntax::Diag) -> Vec<String> {
    d.related
        .iter()
        .filter(|r| r.origin == hale_syntax::SpanOrigin::Stdlib)
        .map(|r| {
            format!(
                "note: {} ({})",
                r.label,
                hale_types::stdlib_bodies::stdlib_span_note(r.span)
            )
        })
        .collect()
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => a == b,
    }
}

/// A merged-coordinate related span → LSP `DiagnosticRelatedInformation`,
/// resolved to its own file through the file-base table.
///
/// `None` for a STDLIB-origin entry: it has no seed range, and
/// `relatedInformation` is nothing but a range. It is published as a
/// note on the message instead — see [`stdlib_related_notes`].
fn related_to_lsp(
    r: &hale_syntax::Related,
    file_bases: &[(u32, PathBuf, u32)],
    sources: &BTreeMap<PathBuf, String>,
) -> Option<Value> {
    if r.origin == hale_syntax::SpanOrigin::Stdlib {
        return None;
    }
    let (rspan, label) = (r.span, r.label.as_str());
    let off = rspan.start.as_usize() as u32;
    let (base, path, _) = file_bases.iter().find(|(base, _, len)| {
        hale_syntax::file_owns_offset(*base, *len, off)
    })?;
    let src = sources.get(path)?;
    let local = rspan.shifted(base.wrapping_neg());
    let (sl, sc) = offset_to_lsp_pos(src, local.start.as_usize());
    let (el, ec) = offset_to_lsp_pos(src, local.end.as_usize());
    let (el, ec) = if (el, ec) <= (sl, sc) { (sl, sc + 1) } else { (el, ec) };
    Some(json!({
        "location": {
            "uri": path_to_uri(path),
            "range": {
                "start": { "line": sl, "character": sc },
                "end":   { "line": el, "character": ec }
            }
        },
        "message": label
    }))
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

// ---- v2: the requests read the snapshot --------------------------------
//
// F.40 phase 2.3: every request loads the snapshot `check_and_publish`
// loads (`editor_snapshot`) and demands the family it answers from —
// the scope, the editor's scope, the bus graph — so an answer and the
// diagnostics describe one program. A request whose family the snapshot
// did not build (a seed with a hole, a load the import graph refused)
// does not answer from a scope of its own; the requests that answered
// while the user types (completion, hover, enforcement, references)
// answer from the editor's scope over the members that parsed.

/// The base of `path`'s window in the snapshot's bundle-global spans.
fn base_of(snap: &Snapshot, path: &Path) -> Option<u32> {
    snap.file_bases()
        .iter()
        .find(|(_, p, _)| same_file(p, path))
        .map(|(b, _, _)| *b)
}

/// The text the snapshot read for `path`, however the path is spelled.
fn source_of<'s>(snap: &'s Snapshot, path: &Path) -> Option<&'s String> {
    snap.sources().get(path).or_else(|| {
        let canon = path.canonicalize().ok()?;
        snap.sources().get(&canon)
    })
}

/// The file a bundle-global offset falls in, with its text and base.
fn file_at(snap: &Snapshot, offset: usize) -> Option<(&PathBuf, &String, u32)> {
    let (base, path, _) = snap
        .file_bases()
        .iter()
        .find(|(base, _, len)| hale_syntax::file_owns_offset(*base, *len, offset as u32))?;
    Some((path, snap.sources().get(path)?, *base))
}

/// The locus whose declaration spans the bundle-global `offset`.
fn enclosing_locus<'t>(
    top: &'t hale_types::resolve::TopScope,
    offset: usize,
) -> Option<&'t hale_types::symbol::LocusInfo> {
    top.symbols.values().find_map(|sym| match sym {
        hale_types::symbol::TopSymbol::Locus(l) => {
            let sp = sym.span();
            (sp.start.as_usize() <= offset && offset < sp.end.as_usize()).then_some(l)
        }
        _ => None,
    })
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



// ---- library surface for `hale mcp` ---------------------------------
//
// The custom requests, callable without a JSON-RPC transport: the
// MCP subcommand exposes these as agent tools by direct library
// call — the same snapshot and families, no drift possible.

fn doc_msg(path: &Path) -> Value {
    json!({ "params": { "textDocument": {
        "uri": format!("file://{}", path.display())
    }}})
}

/// The seed's bus graph (see the `hale/busGraph` LSP request).
pub fn bus_graph_for_path(path: &Path) -> Value {
    bus_graph(&doc_msg(path), &BTreeMap::new())
        .unwrap_or_else(|| json!({ "subjects": [] }))
}

/// The main locus's placement map (`hale/placement`).
pub fn placement_for_path(path: &Path) -> Value {
    placement(&doc_msg(path), &BTreeMap::new())
        .unwrap_or_else(|| json!({ "fields": [] }))
}

/// The allocation-bound survey (`hale/allocSummary`).
pub fn alloc_summary_for_path(path: &Path) -> Value {
    alloc_summary(&doc_msg(path), &BTreeMap::new())
        .unwrap_or_else(|| json!({ "leakSites": [] }))
}

/// The per-fn enforcement map (`hale/enforcement`).
pub fn enforcement_for_path(path: &Path) -> Value {
    enforcement(&doc_msg(path), &BTreeMap::new())
        .unwrap_or_else(|| json!({ "fns": [] }))
}

// ---- v5: formatting + document symbols + hale/enforcement ------------

/// Whole-document formatting via the hale fmt core: one TextEdit
/// replacing the full document with its canonical form. Null when
/// the buffer doesn't lex (formatting a broken buffer would eat
/// text) or is already canonical (empty edit list).
fn formatting(
    msg: &Value,
    overlays: &BTreeMap<PathBuf, String>,
) -> Option<Value> {
    let path = text_document_path(msg)?;
    let src = Overlay::new(overlays).read(&path).ok()?;
    let out = match hale_syntax::fmt::format_source(&src) {
        Ok(o) => o,
        Err(_) => return None,
    };
    if out == src {
        return Some(json!([]));
    }
    let (el, ec) = offset_to_lsp_pos(&src, src.len());
    Some(json!([{
        "range": {
            "start": { "line": 0, "character": 0 },
            "end":   { "line": el, "character": ec }
        },
        "newText": out
    }]))
}

/// LSP SymbolKind constants used below.
mod sym_kind {
    pub const METHOD: u64 = 6;
    pub const FIELD: u64 = 8;
    pub const CLASS: u64 = 5;
    pub const INTERFACE: u64 = 11;
    pub const FUNCTION: u64 = 12;
    pub const CONSTANT: u64 = 14;
    pub const STRUCT: u64 = 23;
    pub const EVENT: u64 = 24;
    pub const ENUM: u64 = 10;
}

fn sym_range(src: &str, span: hale_syntax::Span) -> Value {
    let (sl, sc) = offset_to_lsp_pos(src, span.start.as_usize());
    let (el, ec) = offset_to_lsp_pos(src, span.end.as_usize());
    json!({
        "start": { "line": sl, "character": sc },
        "end":   { "line": el, "character": ec }
    })
}

/// Per-document outline: hierarchical DocumentSymbols from the
/// snapshot's member program for the open file (what the file itself
/// declares, before the merge and the sequence), answered while another
/// member of the seed does not parse and while an import does not
/// resolve: the outline needs no linked scope.
fn document_symbols(
    msg: &Value,
    overlays: &BTreeMap<PathBuf, String>,
) -> Option<Value> {
    use hale_syntax::ast::{LocusMember, TopDecl, TypeDeclBody};
    let path = text_document_path(msg)?;
    let snap = editor_load(&path, overlays).ok()?;
    let program = snap.member(&path)?;
    let src = source_of(&snap, &path)?;
    let base = base_of(&snap, &path)?;

    let mk = |name: &str,
              kind: u64,
              full: hale_syntax::Span,
              sel: hale_syntax::Span,
              children: Vec<Value>| {
        let mut v = json!({
            "name": name,
            "kind": kind,
            "range": sym_range(src, full.shifted(base.wrapping_neg())),
            "selectionRange": sym_range(src, sel.shifted(base.wrapping_neg())),
        });
        if !children.is_empty() {
            v["children"] = json!(children);
        }
        v
    };

    let mut out: Vec<Value> = Vec::new();
    for item in &program.items {
        match item {
            TopDecl::Fn(f) => {
                out.push(mk(
                    &f.name.name,
                    sym_kind::FUNCTION,
                    f.span,
                    f.name.span,
                    Vec::new(),
                ));
            }
            TopDecl::Locus(l) => {
                let mut children = Vec::new();
                for m in &l.members {
                    match m {
                        LocusMember::Params(pb) => {
                            for p in &pb.params {
                                children.push(mk(
                                    &p.name.name,
                                    sym_kind::FIELD,
                                    p.span,
                                    p.name.span,
                                    Vec::new(),
                                ));
                            }
                        }
                        LocusMember::Fn(f) => {
                            children.push(mk(
                                &f.name.name,
                                sym_kind::METHOD,
                                f.span,
                                f.name.span,
                                Vec::new(),
                            ));
                        }
                        _ => {}
                    }
                }
                out.push(mk(
                    &l.name.name,
                    sym_kind::CLASS,
                    l.span,
                    l.name.span,
                    children,
                ));
            }
            TopDecl::Type(t) => {
                let kind = match &t.body {
                    TypeDeclBody::Enum(_) => sym_kind::ENUM,
                    _ => sym_kind::STRUCT,
                };
                out.push(mk(
                    &t.name.name,
                    kind,
                    t.span,
                    t.name.span,
                    Vec::new(),
                ));
            }
            TopDecl::Topic(t) => {
                out.push(mk(
                    &t.name.name,
                    sym_kind::EVENT,
                    t.name.span,
                    t.name.span,
                    Vec::new(),
                ));
            }
            TopDecl::Interface(i) => {
                out.push(mk(
                    &i.name.name,
                    sym_kind::INTERFACE,
                    i.name.span,
                    i.name.span,
                    Vec::new(),
                ));
            }
            TopDecl::Const(c) => {
                out.push(mk(
                    &c.name.name,
                    sym_kind::CONSTANT,
                    c.span,
                    c.name.span,
                    Vec::new(),
                ));
            }
            _ => {}
        }
    }
    Some(json!(out))
}

/// hale/enforcement: every user fn + locus method in the seed with
/// its enforcement contract — @hot, @budget(N), fallible payload,
/// @unbounded — the certification map an agent consults before
/// touching a hot path.
fn enforcement(
    msg: &Value,
    overlays: &BTreeMap<PathBuf, String>,
) -> Option<Value> {
    let path = text_document_path(msg)?;
    let snap = editor_snapshot(&path, overlays).ok()?;
    Some(enforcement_of(&snap))
}

/// `hale/enforcement` over one snapshot: every fn of the programs the
/// editor's scope was built over (a hole leaves the members that
/// parsed), in the file each was written in. A fn the load generated
/// or an import renamed (`__`) is not the author's to certify.
fn enforcement_of(snap: &Snapshot) -> Value {
    use hale_syntax::ast::{LocusMember, TopDecl};
    if snap.demand_editor_scope().is_err() {
        return json!({ "fns": [], "parseErrors": true });
    }
    let mut fns: Vec<Value> = Vec::new();
    for program in snap.programs().values() {
        let mut push_fn = |f: &hale_syntax::ast::FnDecl,
                           locus: Option<&str>| {
            if f.name.name.starts_with("__") {
                return;
            }
            let Some((fpath, src, base)) = file_at(snap, f.name.span.start.as_usize()) else {
                return;
            };
            let local = f.name.span.start.as_usize().saturating_sub(base as usize);
            let (line, _) = offset_to_lsp_pos(src, local);
            fns.push(json!({
                "name": match locus {
                    Some(l) => format!("{}.{}", l, f.name.name),
                    None => f.name.name.clone(),
                },
                "file": fpath.display().to_string(),
                "line": line,
                "hot": f.hot,
                "budget": f.budget,
                "unbounded": f.unbounded,
                "fallible": f.fallible.as_ref().map(type_expr_str),
            }));
        };
        for item in &program.items {
            match item {
                TopDecl::Fn(f) => push_fn(f, None),
                TopDecl::Locus(l) => {
                    for m in &l.members {
                        if let LocusMember::Fn(f) = m {
                            push_fn(f, Some(&l.name.name));
                        }
                    }
                }
                _ => {}
            }
        }
    }
    json!({ "fns": fns })
}

// ---- v4: completion --------------------------------------------------
//
// Same design as everything else in this server: no index, no
// incremental state — every request re-derives what it needs from
// the overlay text + a fresh snapshot (the ~10 ms front-end).
// Context comes from the RAW TEXT left of the cursor (robust
// mid-keystroke, when the buffer usually doesn't parse):
//
//   `self.<partial>`          → the enclosing locus's params +
//                               user-declared methods
//   `std::io::tcp::<partial>` → stdlib namespace: free fns (with
//                               signatures) + locus paths + child
//                               namespaces
//   bare `<partial>`          → seed top-level symbols (fns, loci,
//                               types, topics, interfaces, consts)
//                               + keywords + primitive type names
//
// The client filters against the partial word; we pre-filter to
// keep payloads small but always return isIncomplete: false.
fn completion(
    msg: &Value,
    overlays: &BTreeMap<PathBuf, String>,
) -> Option<Value> {
    let path = text_document_path(msg)?;
    let line = msg.pointer("/params/position/line")?.as_u64()? as u32;
    let character =
        msg.pointer("/params/position/character")?.as_u64()? as u32;

    let src = Overlay::new(overlays).read(&path).ok()?;
    let offset = lsp_pos_to_offset(&src, line, character);
    let before = &src[..offset.min(src.len())];

    // Partial word being typed (may be empty right after a trigger).
    let word_start = before
        .rfind(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .map(|i| i + 1)
        .unwrap_or(0);
    let partial = &before[word_start..];
    let ctx = &before[..word_start];

    // Mid-keystroke the buffer usually does NOT parse (that's when
    // completion fires): `self.` or a half-typed word is not a
    // statement. The snapshot reads the buffer with that fragment
    // blanked — spaces, byte for byte, so every offset holds — which
    // is the program around the cursor as it parses. Where it still
    // does not (a hole elsewhere), the editor's scope covers the
    // members that did.
    let typed = |from: usize| -> Option<Snapshot> {
        let mut masked = overlays.clone();
        let mut text = src.clone();
        text.replace_range(from..offset, &" ".repeat(offset - from));
        masked.insert(path.clone(), text);
        editor_snapshot(&path, &masked).ok()
    };

    let mut items: Vec<Value> = Vec::new();

    if ctx.ends_with("self.") {
        let snap = typed(word_start - "self.".len());
        if let Some(snap) = &snap {
            complete_self_members(snap, &path, offset, partial, &mut items);
        }
    } else if ctx.ends_with("::") {
        // Collect the `::`-joined path segments left of the cursor.
        let mut segs: Vec<String> = Vec::new();
        let mut rest = &ctx[..ctx.len() - 2];
        loop {
            let seg_start = rest
                .rfind(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                .map(|i| i + 1)
                .unwrap_or(0);
            let seg = &rest[seg_start..];
            if seg.is_empty() {
                break;
            }
            segs.insert(0, seg.to_string());
            if seg_start >= 2 && rest[..seg_start].ends_with("::") {
                rest = &rest[..seg_start - 2];
            } else {
                break;
            }
        }
        // The stdlib's surface table answers; no family is read.
        if segs.first().map(String::as_str) == Some("std") {
            complete_std_path(&segs[1..], partial, &mut items);
        }
    } else {
        let snap = typed(word_start);
        complete_top_level(snap.as_ref(), partial, &mut items);
    }

    Some(json!({ "isIncomplete": false, "items": items }))
}

/// LSP CompletionItemKind constants (the handful we use).
mod ci_kind {
    pub const METHOD: u64 = 2;
    pub const FUNCTION: u64 = 3;
    pub const FIELD: u64 = 5;
    pub const CLASS: u64 = 7; // locus
    pub const INTERFACE: u64 = 8;
    pub const MODULE: u64 = 9; // namespace
    pub const ENUM: u64 = 13;
    pub const KEYWORD: u64 = 14;
    pub const STRUCT: u64 = 22;
    pub const EVENT: u64 = 23; // topic
    pub const CONSTANT: u64 = 21;
}

fn push_item(
    items: &mut Vec<Value>,
    label: &str,
    kind: u64,
    detail: Option<String>,
) {
    let mut v = json!({ "label": label, "kind": kind });
    if let Some(d) = detail {
        v["detail"] = json!(d);
    }
    items.push(v);
}

/// `self.` members: the enclosing locus's params, from the editor's
/// scope, and its declared methods, from the programs that scope was
/// built over (the scope carries no method lists).
fn complete_self_members(
    snap: &Snapshot,
    path: &Path,
    offset: usize,
    partial: &str,
    items: &mut Vec<Value>,
) {
    let Ok(scope) = snap.demand_editor_scope() else { return };
    let Some(base) = base_of(snap, path) else { return };
    // Enclosing locus by span containment.
    let Some(l) = enclosing_locus(scope.top, base as usize + offset) else { return };
    for p in &l.params {
        if p.name.starts_with(partial) {
            push_item(
                items,
                &p.name,
                ci_kind::FIELD,
                Some(p.ty.display()),
            );
        }
    }
    let lname = &l.name;
    for prog in snap.programs().values() {
        for item in &prog.items {
            let hale_syntax::ast::TopDecl::Locus(l) = item else {
                continue;
            };
            if &l.name.name != lname {
                continue;
            }
            for m in &l.members {
                let hale_syntax::ast::LocusMember::Fn(f) = m else {
                    continue;
                };
                // A method the load generated is not the author's.
                if !f.name.name.starts_with(partial) || f.name.name.starts_with("__") {
                    continue;
                }
                let ps = f
                    .params
                    .iter()
                    .map(|p| {
                        format!("{}: {}", p.name.name, type_expr_str(&p.ty))
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                let ret = f
                    .ret
                    .as_ref()
                    .map(type_expr_str)
                    .unwrap_or_else(|| "()".to_string());
                push_item(
                    items,
                    &f.name.name,
                    ci_kind::METHOD,
                    Some(format!("fn({}) -> {}", ps, ret)),
                );
            }
        }
    }
}

fn complete_std_path(
    ns: &[String],
    partial: &str,
    items: &mut Vec<Value>,
) {
    use hale_types::stdlib_surface as surf;
    let ns_refs: Vec<&str> = ns.iter().map(String::as_str).collect();

    // Free fns in the exact namespace, with signatures as detail.
    for surface in surf::SURFACES {
        if surface.ns == ns_refs.as_slice() {
            for entry in surface.fns {
                let f = entry.name;
                if !f.starts_with(partial) {
                    continue;
                }
                let mut segs: Vec<&str> = vec!["std"];
                segs.extend(ns_refs.iter().copied());
                segs.push(f);
                let detail =
                    surf::signature_for(&segs).map(|sig| {
                        let ps = sig
                            .params
                            .iter()
                            .map(|t| sig_ty_str(t).to_string())
                            .collect::<Vec<_>>()
                            .join(", ");
                        let mut d = format!(
                            "fn({}) -> {}",
                            ps,
                            sig_ty_str(&sig.ret)
                        );
                        if let Some(e) = sig.fallible {
                            d.push_str(&format!(" fallible({})", e));
                        }
                        d
                    });
                push_item(items, f, ci_kind::FUNCTION, detail);
            }
        }
    }

    // Stdlib locus/type paths one segment below the cursor's path
    // (`std::metrics::` → Registry, Counter, …) and child
    // namespaces (`std::` → io, str, bytes, …; `std::io::` → tcp,
    // udp, tls, fs).
    let mut seen_ns: std::collections::BTreeSet<&str> =
        std::collections::BTreeSet::new();
    for lp in surf::LOCUS_PATHS {
        // LOCUS_PATHS entries carry the leading "std".
        let rest = match lp.split_first() {
            Some((&"std", rest)) => rest,
            _ => continue,
        };
        if rest.len() == ns_refs.len() + 1
            && rest[..ns_refs.len()] == ns_refs[..]
        {
            let leaf = rest[ns_refs.len()];
            if leaf.starts_with(partial) {
                push_item(items, leaf, ci_kind::CLASS, None);
            }
        }
    }
    for surface in surf::SURFACES {
        if surface.ns.len() > ns_refs.len()
            && surface.ns[..ns_refs.len()] == ns_refs[..]
        {
            seen_ns.insert(surface.ns[ns_refs.len()]);
        }
    }
    for child in seen_ns {
        if child.starts_with(partial) {
            push_item(items, child, ci_kind::MODULE, None);
        }
    }
}

/// Bare words: the editor's scope's top-level symbols (none when the
/// seed did not load, or no member parsed), then keywords, primitive
/// type names and the std root.
fn complete_top_level(
    snap: Option<&Snapshot>,
    partial: &str,
    items: &mut Vec<Value>,
) {
    use hale_types::symbol::{TopSymbol, TypeKind};
    if let Some(scope) = snap.and_then(|s| s.demand_editor_scope().ok()) {
        for (name, sym) in &scope.top.symbols {
            if !name.starts_with(partial) || name.starts_with("__") {
                continue;
            }
            match sym {
                TopSymbol::Fn(f) => {
                    let ps = f
                        .params
                        .iter()
                        .map(|(n, t)| format!("{}: {}", n, t.display()))
                        .collect::<Vec<_>>()
                        .join(", ");
                    push_item(
                        items,
                        name,
                        ci_kind::FUNCTION,
                        Some(format!("fn({}) -> {}", ps, f.ret.display())),
                    );
                }
                TopSymbol::Locus(_) => {
                    push_item(items, name, ci_kind::CLASS, Some("locus".into()));
                }
                TopSymbol::Type(t) => {
                    let kind = match &t.kind {
                        TypeKind::Enum(_) => ci_kind::ENUM,
                        _ => ci_kind::STRUCT,
                    };
                    push_item(items, name, kind, Some("type".into()));
                }
                TopSymbol::Topic(t) => {
                    push_item(
                        items,
                        name,
                        ci_kind::EVENT,
                        Some(format!("topic \"{}\"", t.subject)),
                    );
                }
                TopSymbol::Interface(_) => {
                    push_item(
                        items,
                        name,
                        ci_kind::INTERFACE,
                        Some("interface".into()),
                    );
                }
                TopSymbol::Const(c) => {
                    push_item(
                        items,
                        name,
                        ci_kind::CONSTANT,
                        Some(c.ty.display()),
                    );
                }
                _ => {}
            }
        }
    }
    // Keywords + primitive type names + the std root.
    for kw in hale_syntax::keywords::HARD_KEYWORDS {
        if kw.starts_with(partial) {
            push_item(items, kw, ci_kind::KEYWORD, None);
        }
    }
    for prim in [
        "Int", "Uint", "Float", "Decimal", "String", "Bool", "Time",
        "Duration", "Bytes",
    ] {
        if prim.starts_with(partial) {
            push_item(items, prim, ci_kind::STRUCT, None);
        }
    }
    if "std".starts_with(partial) {
        push_item(items, "std", ci_kind::MODULE, None);
    }
}

// ---- v2: hover -------------------------------------------------------

fn hover(msg: &Value, overlays: &BTreeMap<PathBuf, String>) -> Option<Value> {
    let path = text_document_path(msg)?;
    let line = msg.pointer("/params/position/line")?.as_u64()? as u32;
    let character =
        msg.pointer("/params/position/character")?.as_u64()? as u32;

    let src = Overlay::new(overlays).read(&path).ok()?;
    let snap = editor_snapshot(&path, overlays).ok();
    hover_at(snap.as_ref(), &path, &src, line, character)
}

/// Hover at a position of `path`, whose text is `src`: a `std::` path
/// from the stdlib's table, anything else from the editor's scope of
/// `snap` (a hole leaves the members that parsed). `None` for a seed
/// that did not load answers only the `std::` paths.
fn hover_at(
    snap: Option<&Snapshot>,
    path: &Path,
    src: &str,
    line: u32,
    character: u32,
) -> Option<Value> {
    let offset = lsp_pos_to_offset(src, line, character);
    // Token at position (file-local lex; parse errors don't matter).
    let (tokens, idx, word, segs) = token_context(src, offset)?;
    let tok = &tokens[idx];

    let text = hover_text(snap, path, &tokens, idx, &word, &segs)?;
    let (sl, sc) = offset_to_lsp_pos(src, tok.span.start.as_usize());
    let (el, ec) = offset_to_lsp_pos(src, tok.span.end.as_usize());
    Some(json!({
        "contents": { "kind": "markdown", "value": text },
        "range": {
            "start": { "line": sl, "character": sc },
            "end":   { "line": el, "character": ec }
        }
    }))
}

fn hover_text(
    snap: Option<&Snapshot>,
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

    // Everything else is the seed's: the editor's scope.
    let snap = snap?;
    let scope = snap.demand_editor_scope().ok()?;

    // `self.<field>` — the enclosing locus's param.
    if idx >= 2
        && matches!(tokens[idx - 1].kind, TK::Dot)
        && matches!(tokens[idx - 2].kind, TK::KwSelf)
    {
        let base = base_of(snap, path)?;
        let merged = base as usize + tokens[idx].span.start.as_usize();
        let l = enclosing_locus(scope.top, merged)?;
        let p = l.params.iter().find(|p| p.name == word)?;
        return Some(format!(
            "```hale\nself.{}: {}\n```\n\nparam of \
             `locus {}`",
            p.name,
            p.ty.display(),
            l.name
        ));
    }

    // Top-level symbol lookup.
    let sym = scope.top.lookup(word)?;
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
                // Polish: a payload naming a stdlib-injected error
                // type (IoError et al.) resolves as Unknown and
                // displays `?` — recover the written name from the
                // AST decl.
                let mut shown = e.display();
                if shown == "?" {
                    for prog in snap.programs().values() {
                        for item in &prog.items {
                            if let hale_syntax::ast::TopDecl::Fn(fd) = item {
                                if fd.name.name == f.name {
                                    if let Some(te) = &fd.fallible {
                                        shown = type_expr_str(te);
                                    }
                                }
                            }
                        }
                    }
                }
                out.push_str(&format!(
                    "\n\n`fallible({})` — callers must address the error",
                    shown
                ));
            }
            // Enforcement status from the AST decl.
            for prog in snap.programs().values() {
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

pub fn sig_ty_str(t: &hale_types::stdlib_surface::SigTy) -> String {
    use hale_types::stdlib_surface::SigTy::*;
    match t {
        Int => "Int".to_string(),
        Uint => "Uint".to_string(),
        Float => "Float".to_string(),
        Bool => "Bool".to_string(),
        Str => "String".to_string(),
        Bytes => "Bytes".to_string(),
        BytesMut => "Bytes".to_string(),
        Decimal => "Decimal".to_string(),
        Duration => "Duration".to_string(),
        Time => "Time".to_string(),
        Unit => "()".to_string(),
        Any => "…".to_string(),
        // GH #771: a struct return/param carries the MANGLED name
        // the checker unifies on (`__JsonString`); hover and the
        // generated reference are read by people, who write the
        // public path. Reverse the rename table rather than showing
        // either the mangled name or the old `…`.
        Named(n) => hale_stdlib::PATH_RENAMES
            .iter()
            .find(|(_, target)| target == n)
            .map(|(path, _)| path.join("::"))
            .unwrap_or_else(|| (*n).to_string()),
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
    let snap = editor_snapshot(&path, overlays).ok()?;
    Some(bus_graph_of(&snap))
}

/// `hale/busGraph` over one snapshot: the bus graph the model reads
/// (`demand_bus_graph`, over the checked programs), so a subject's
/// eligibility here is the diagnostics pass's. A seed with a hole has
/// no graph.
fn bus_graph_of(snap: &Snapshot) -> Value {
    let Ok(graph) = snap.demand_bus_graph() else {
        return json!({ "subjects": [], "parseErrors": true });
    };
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
    json!({ "subjects": subjects })
}

// ---- v3: shared token context ---------------------------------------

/// The Ident token at `offset` plus its `::`-joined path context.
fn token_context(
    src: &str,
    offset: usize,
) -> Option<(Vec<hale_syntax::lexer::Token>, usize, String, Vec<String>)> {
    use hale_syntax::lexer::TokenKind as TK;
    let tokens = hale_syntax::lexer::lex(src).ok()?;
    let idx = tokens.iter().position(|t| {
        t.span.start.as_usize() <= offset && offset < t.span.end.as_usize()
    })?;
    let word = match &tokens[idx].kind {
        TK::Ident(name) => name.clone(),
        _ => return None,
    };
    let mut lo = idx;
    while lo >= 2
        && matches!(tokens[lo - 1].kind, TK::ColonColon)
        && matches!(tokens[lo - 2].kind, TK::Ident(_))
    {
        lo -= 2;
    }
    let mut hi = idx;
    while hi + 2 < tokens.len()
        && matches!(tokens[hi + 1].kind, TK::ColonColon)
        && matches!(tokens[hi + 2].kind, TK::Ident(_))
    {
        hi += 2;
    }
    let mut segs: Vec<String> = Vec::new();
    let mut k = lo;
    while k <= hi {
        if let TK::Ident(n) = &tokens[k].kind {
            segs.push(n.clone());
        }
        k += 2;
    }
    Some((tokens, idx, word, segs))
}

/// Merged-bundle span → (file, LSP range).
fn merged_span_to_location(
    snap: &Snapshot,
    span: hale_syntax::Span,
) -> Option<Value> {
    let off = span.start.as_usize() as u32;
    for (base, path, len) in snap.file_bases() {
        if hale_syntax::file_owns_offset(*base, *len, off) {
            let src = snap.sources().get(path)?;
            let local_start = span.start.as_usize() - *base as usize;
            let local_end =
                (span.end.as_usize() - *base as usize).min(src.len());
            let (sl, sc) = offset_to_lsp_pos(src, local_start);
            let (el, ec) = offset_to_lsp_pos(src, local_end);
            return Some(json!({
                "uri": path_to_uri(path),
                "range": {
                    "start": { "line": sl, "character": sc },
                    "end":   { "line": el, "character": ec }
                }
            }));
        }
    }
    None
}

// ---- v3: definition / references ------------------------------------

fn definition(
    msg: &Value,
    overlays: &BTreeMap<PathBuf, String>,
) -> Option<Value> {
    let path = text_document_path(msg)?;
    let line = msg.pointer("/params/position/line")?.as_u64()? as u32;
    let character =
        msg.pointer("/params/position/character")?.as_u64()? as u32;
    // A definition is a location in the program the snapshot scoped:
    // a seed with a hole has none to give.
    let snap = editor_snapshot(&path, overlays).ok()?;
    let top = snap.demand_scope().ok()?;
    let src = source_of(&snap, &path)?;
    let offset = lsp_pos_to_offset(src, line, character);
    let (tokens, idx, word, segs) = token_context(src, offset)?;

    // std:: paths resolve into the EMBEDDED stdlib source
    // (downstream handoff, 2026-08-11): the rename table maps the
    // user-facing path to the mangled name `AP_SOURCE` declares,
    // and the declaration's file is materialized to a read-only
    // cache so the location is a plain `file://` URI — no client-
    // side virtual-document support needed, which matters for the
    // editors that have none (an `install.sh` binary has no stdlib
    // checkout on disk). C-backed path-call primitives have no
    // Hale definition and still return None.
    if segs.first().map(String::as_str) == Some("std") {
        return stdlib_definition(&segs);
    }

    // self.<field> → the param decl on the enclosing locus.
    use hale_syntax::lexer::TokenKind as TK;
    if idx >= 2
        && matches!(tokens[idx - 1].kind, TK::Dot)
        && matches!(tokens[idx - 2].kind, TK::KwSelf)
    {
        let base = base_of(&snap, &path)?;
        let merged = base as usize + tokens[idx].span.start.as_usize();
        let l = enclosing_locus(top, merged)?;
        let p = l.params.iter().find(|p| p.name == word)?;
        return merged_span_to_location(&snap, p.span);
    }

    let sym = top.lookup(&word)?;
    merged_span_to_location(&snap, sym.span())
}

/// The stdlib AST, parsed once per process from the embedded
/// source. `None` if the embedded stdlib ever fails to parse
/// (which the build would have caught long before an LSP ran).
fn stdlib_ast() -> Option<&'static hale_syntax::ast::Program> {
    use std::sync::OnceLock;
    static AST: OnceLock<Option<hale_syntax::ast::Program>> =
        OnceLock::new();
    AST.get_or_init(|| {
        hale_syntax::parse_source(hale_stdlib::AP_SOURCE).ok()
    })
    .as_ref()
}

/// The name ident of a top-level stdlib declaration, if it is a
/// kind the rename table can point at.
fn top_decl_name(
    d: &hale_syntax::ast::TopDecl,
) -> Option<&hale_syntax::ast::Ident> {
    use hale_syntax::ast::TopDecl as TD;
    match d {
        TD::Locus(l) => Some(&l.name),
        TD::Type(t) => Some(&t.name),
        TD::Fn(f) => Some(&f.name),
        TD::Interface(i) => Some(&i.name),
        TD::Const(c) => Some(&c.name),
        TD::Topic(t) => Some(&t.name),
        _ => None,
    }
}

/// Materialize one embedded stdlib file into the versioned
/// read-only cache and return its path. Content-checked, so a
/// version bump (or a dev build changing the stdlib) refreshes it.
fn materialize_stdlib_file(
    name: &str,
    content: &str,
) -> Option<PathBuf> {
    let cache_root = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(|h| PathBuf::from(h).join(".cache"))
        })?;
    let dir = cache_root
        .join("hale")
        .join(format!("stdlib-{}", env!("CARGO_PKG_VERSION")));
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(name);
    let fresh = std::fs::read_to_string(&path)
        .map_or(true, |existing| existing != content);
    if fresh {
        // The file is left read-only as a "this is not yours to
        // edit" signal, so a refresh must lift that first.
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, content).ok()?;
        let mut perm =
            std::fs::metadata(&path).ok()?.permissions();
        perm.set_readonly(true);
        let _ = std::fs::set_permissions(&path, perm);
    }
    Some(path)
}

/// `textDocument/definition` for a `std::` path: rename-table
/// lookup → declaration span in the embedded source → location in
/// the materialized cache file.
fn stdlib_definition(segs: &[String]) -> Option<Value> {
    // Exact path match first; else the longest table entry that
    // prefixes the cursor's path (`std::http::Server` inside a
    // longer chain).
    let matches_entry = |entry: &[&str], exact: bool| {
        (entry.len() == segs.len()
            || (!exact && entry.len() < segs.len()))
            && entry.iter().zip(segs).all(|(a, b)| a == b)
    };
    let mangled = hale_stdlib::PATH_RENAMES
        .iter()
        .find(|(p, _)| matches_entry(p, true))
        .or_else(|| {
            hale_stdlib::PATH_RENAMES
                .iter()
                .filter(|(p, _)| matches_entry(p, false))
                .max_by_key(|(p, _)| p.len())
        })
        .map(|(_, m)| *m)?;
    let ast = stdlib_ast()?;
    let name_span = ast.items.iter().find_map(|d| {
        let n = top_decl_name(d)?;
        (n.name == mangled).then_some(n.span)
    })?;
    let (file_name, content, local_start) =
        hale_stdlib::ap_file_at(name_span.start.as_usize())?;
    let local_end = (local_start
        + (name_span.end.as_usize() - name_span.start.as_usize()))
    .min(content.len());
    let path = materialize_stdlib_file(file_name, content)?;
    let (sl, sc) = offset_to_lsp_pos(content, local_start);
    let (el, ec) = offset_to_lsp_pos(content, local_end);
    Some(json!({
        "uri": path_to_uri(&path),
        "range": {
            "start": { "line": sl, "character": sc },
            "end":   { "line": el, "character": ec }
        }
    }))
}

fn references(
    msg: &Value,
    overlays: &BTreeMap<PathBuf, String>,
) -> Option<Value> {
    let path = text_document_path(msg)?;
    let line = msg.pointer("/params/position/line")?.as_u64()? as u32;
    let character =
        msg.pointer("/params/position/character")?.as_u64()? as u32;
    let include_decl = msg
        .pointer("/params/context/includeDeclaration")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let snap = editor_snapshot(&path, overlays).ok()?;
    let src = source_of(&snap, &path)?;
    let offset = lsp_pos_to_offset(src, line, character);
    let (_, _, word, _) = token_context(src, offset)?;

    // The declaration's merged span, for includeDeclaration=false:
    // the editor's scope (a hole leaves the members that parsed).
    let decl_span = snap
        .demand_editor_scope()
        .ok()
        .and_then(|scope| scope.top.lookup(&word).map(|s| s.span()));

    // Name-scoped scan of every file's Ident tokens. Honest v3
    // semantics: references-by-name across the seed (hale's flat
    // per-seed namespace makes this accurate for top-level symbols;
    // shadowing locals will over-report — a documented limitation).
    // The files are the snapshot's: the seed and every seed its
    // imports reach.
    let mut out: Vec<Value> = Vec::new();
    for (file, source) in snap.sources() {
        let Ok(tokens) = hale_syntax::lexer::lex(source) else {
            continue;
        };
        let base = base_of(&snap, file).unwrap_or(0);
        for t in &tokens {
            if let hale_syntax::lexer::TokenKind::Ident(n) = &t.kind {
                if n == &word {
                    if !include_decl {
                        if let Some(ds) = decl_span {
                            let merged =
                                base as usize + t.span.start.as_usize();
                            if ds.start.as_usize() <= merged
                                && merged < ds.end.as_usize()
                            {
                                continue;
                            }
                        }
                    }
                    let (sl, sc) =
                        offset_to_lsp_pos(source, t.span.start.as_usize());
                    let (el, ec) =
                        offset_to_lsp_pos(source, t.span.end.as_usize());
                    out.push(json!({
                        "uri": path_to_uri(file),
                        "range": {
                            "start": { "line": sl, "character": sc },
                            "end":   { "line": el, "character": ec }
                        }
                    }));
                }
            }
        }
    }
    Some(Value::Array(out))
}

// ---- v3: hale/placement ---------------------------------------------

fn placement_spec_str(e: &hale_syntax::ast::PlacementEntry) -> String {
    use hale_syntax::ast::PlacementSpec;
    let mut out = match &e.spec {
        PlacementSpec::Cooperative { pool, .. } => match pool {
            Some(p) => format!("cooperative(pool = {})", p.name),
            None => "cooperative(pool = main)".to_string(),
        },
        PlacementSpec::Pinned { affinity, replicas } => {
            // Debug-format affinity compactly; `PinAffinity::Any`
            // renders as bare `pinned`.
            let mut s = if format!("{:?}", affinity).contains("Any") {
                "pinned".to_string()
            } else {
                format!("pinned({:?})", affinity)
            };
            if let Some(k) = replicas {
                s.push_str(&format!(" replicas = {}", k));
            }
            s
        }
    };
    if !e.constraints.is_empty() {
        let cs: Vec<String> = e
            .constraints
            .iter()
            .map(|c| format!("{:?}", c.kind))
            .collect();
        out.push_str(&format!(" where {}", cs.join(", ").to_lowercase()));
    }
    out
}

fn placement(
    msg: &Value,
    overlays: &BTreeMap<PathBuf, String>,
) -> Option<Value> {
    let path = text_document_path(msg)?;
    let snap = editor_snapshot(&path, overlays).ok()?;
    Some(placement_of(&snap))
}

/// `hale/placement` over one snapshot: the main locus of the program
/// the snapshot scoped, its params and its `placement` block. A seed
/// with a hole has no scope, and no main locus to read.
fn placement_of(snap: &Snapshot) -> Value {
    if snap.demand_scope().is_err() {
        return json!({ "fields": [], "parseErrors": true });
    }
    use hale_syntax::ast::{LocusMember, TopDecl};
    for prog in snap.programs().values() {
        for item in &prog.items {
            let TopDecl::Locus(l) = item else { continue };
            if !l.is_main {
                continue;
            }
            let mut placements: BTreeMap<String, String> = BTreeMap::new();
            let mut params: Vec<(String, String)> = Vec::new();
            for m in &l.members {
                match m {
                    LocusMember::Placement(pb) => {
                        for e in &pb.entries {
                            placements.insert(
                                e.field.name.clone(),
                                placement_spec_str(e),
                            );
                        }
                    }
                    LocusMember::Params(ps) => {
                        for pd in &ps.params {
                            let ty = pd
                                .ty
                                .as_ref()
                                .map(type_expr_str)
                                .unwrap_or_else(|| "?".to_string());
                            params.push((pd.name.name.clone(), ty));
                        }
                    }
                    _ => {}
                }
            }
            let fields: Vec<Value> = params
                .iter()
                .map(|(name, ty)| {
                    json!({
                        "field": name,
                        "locus": ty,
                        "placement": placements
                            .get(name)
                            .cloned()
                            .unwrap_or_else(|| {
                                "cooperative(pool = main)".to_string()
                            }),
                        "explicit": placements.contains_key(name),
                    })
                })
                .collect();
            return json!({
                "mainLocus": l.name.name,
                "fields": fields
            });
        }
    }
    json!({ "fields": [], "noMainLocus": true })
}

pub fn type_expr_str(t: &hale_syntax::ast::TypeExpr) -> String {
    use hale_syntax::ast::TypeExpr;
    match t {
        TypeExpr::Primitive(p, _) => format!("{:?}", p),
        TypeExpr::Named { path, .. } => path
            .segments
            .iter()
            .map(|s| s.name.clone())
            .collect::<Vec<_>>()
            .join("::"),
        _ => "…".to_string(),
    }
}

// ---- v3: hale/allocSummary ------------------------------------------

fn alloc_summary(
    msg: &Value,
    overlays: &BTreeMap<PathBuf, String>,
) -> Option<Value> {
    let path = text_document_path(msg)?;
    let snap = editor_snapshot(&path, overlays).ok()?;
    Some(alloc_summary_of(&snap))
}

/// `hale/allocSummary` over one snapshot: the survey over the programs
/// the snapshot scoped, judged over the snapshot's allocation summary,
/// the one the diagnostics pass's unbounded-allocation warnings read. A
/// seed with a hole has none. A site is listed exactly when the
/// diagnostics would place it: both read `advisory_leak_sites`.
fn alloc_summary_of(snap: &Snapshot) -> Value {
    let Ok(summary) = snap.demand_alloc_summary() else {
        return json!({ "leakSites": [], "parseErrors": true });
    };
    let progs: Vec<&Program> = snap.programs().values().collect();
    let sites: Vec<Value> = hale_types::alloc_summary::advisory_leak_sites(
        summary,
        &progs,
        snap.identities(),
        snap.source_map(),
    )
    .iter()
    .map(|site| {
        json!({
            "fn": site.owner.display(),
            "kind": format!("{:?}", site.kind),
            "escape": format!("{:?}", site.escape),
            "reason": format!("{:?}", site.reason),
            "location": merged_span_to_location(snap, site.span),
        })
    })
    .collect();
    json!({
        "leakSites": sites,
        "text": hale_types::dump_alloc_summary(summary),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const A_SRC: &str = "locus A {\n    params { n: Int = 0; }\n}\n";
    const B_SRC: &str = "fn main() {\n    let x = 1;\n}\n";

    /// The two-file seed the server builds for every document
    /// event: bases spaced `len + 1` apart, each file's own text
    /// beside it.
    fn seed() -> (Vec<(u32, PathBuf, u32)>, BTreeMap<PathBuf, String>) {
        let a = PathBuf::from("/seed/a.hl");
        let b = PathBuf::from("/seed/b.hl");
        let la = A_SRC.len() as u32;
        let lb = B_SRC.len() as u32;
        let bases = vec![(0, a.clone(), la), (la + 1, b.clone(), lb)];
        let mut sources = BTreeMap::new();
        sources.insert(a, A_SRC.to_string());
        sources.insert(b, B_SRC.to_string());
        (bases, sources)
    }

    fn empty_publish(
        bases: &[(u32, PathBuf, u32)],
    ) -> BTreeMap<PathBuf, Vec<Value>> {
        bases.iter().map(|(_, p, _)| (p.clone(), Vec::new())).collect()
    }

    /// GH #805: a checker diagnostic positioned at a file's END —
    /// the one-past-the-last-byte position an end-of-file span
    /// carries — is published against THAT file, with a range the
    /// editor can place. The half-open window matched no file, so
    /// the diagnostic was dropped on the floor: nothing published,
    /// nothing logged, for a program the CLI refuses.
    #[test]
    fn a_diagnostic_at_a_files_end_is_published_against_that_file() {
        let (bases, sources) = seed();
        let eof = A_SRC.len();
        let d = hale_syntax::Diag::ty(
            hale_syntax::Span::new(eof, eof),
            "something at the end of a.hl",
        );
        let mut per_file = empty_publish(&bases);
        place_checker_diags(&[d], &bases, &sources, &mut per_file);

        let published = &per_file[&PathBuf::from("/seed/a.hl")];
        assert_eq!(
            published.len(),
            1,
            "the EOF-positioned diagnostic must reach a.hl: {:?}",
            per_file
        );
        assert!(
            per_file[&PathBuf::from("/seed/b.hl")].is_empty(),
            "and must not be attributed to the next file"
        );
        // Three lines of text, so the position past the last byte
        // is the start of the (empty) fourth — 0-based line 3.
        // Zero-width spans are widened by one column so the
        // squiggle is visible.
        assert_eq!(published[0]["range"]["start"]["line"], 3);
        assert_eq!(published[0]["range"]["start"]["character"], 0);
        assert_eq!(published[0]["range"]["end"]["line"], 3);
        assert_eq!(published[0]["range"]["end"]["character"], 1);
        assert_eq!(published[0]["severity"], 1);
    }

    /// The boundary the inclusive end must not steal: the next
    /// file's FIRST byte is its own, not the previous file's
    /// one-past-the-end. Bases are spaced `len + 1` for exactly
    /// this reason.
    #[test]
    fn the_next_files_first_byte_still_belongs_to_the_next_file() {
        let (bases, sources) = seed();
        let b_base = A_SRC.len() + 1;
        let d = hale_syntax::Diag::ty(
            hale_syntax::Span::new(b_base, b_base + 5),
            "at the first byte of b.hl",
        );
        let mut per_file = empty_publish(&bases);
        place_checker_diags(&[d], &bases, &sources, &mut per_file);

        assert!(
            per_file[&PathBuf::from("/seed/a.hl")].is_empty(),
            "a.hl's window must end one byte before b.hl's base: {:?}",
            per_file
        );
        let published = &per_file[&PathBuf::from("/seed/b.hl")];
        assert_eq!(published.len(), 1, "{:?}", per_file);
        assert_eq!(published[0]["range"]["start"]["line"], 0);
        assert_eq!(published[0]["range"]["start"]["character"], 0);
    }

    /// A span in no file at all — the embedded stdlib parses at
    /// base 0 with its own coordinate space — is still placed
    /// nowhere rather than attributed to whichever seed file it
    /// numerically collides with.
    #[test]
    fn a_span_past_every_window_is_placed_nowhere() {
        let (bases, sources) = seed();
        let past = A_SRC.len() + 1 + B_SRC.len() + 1;
        let d = hale_syntax::Diag::ty(
            hale_syntax::Span::new(past, past + 4),
            "from outside the seed",
        );
        let mut per_file = empty_publish(&bases);
        place_checker_diags(&[d], &bases, &sources, &mut per_file);
        assert!(per_file.values().all(|v| v.is_empty()), "{:?}", per_file);
    }

    /// A related location at a file's end resolves the same way —
    /// the second location is what makes a two-place diagnostic
    /// clickable, and it went through its own copy of the window.
    #[test]
    fn a_related_location_at_a_files_end_resolves() {
        let (bases, sources) = seed();
        let eof = A_SRC.len();
        let rel = related_to_lsp(
            &hale_syntax::Related {
                span: hale_syntax::Span::new(eof, eof),
                label: "declared here".to_string(),
                origin: hale_syntax::SpanOrigin::Seed,
            },
            &bases,
            &sources,
        )
        .expect("an end-of-file related span must resolve to its file");
        assert!(
            rel["location"]["uri"]
                .as_str()
                .unwrap_or("")
                .ends_with("a.hl"),
            "{}",
            rel
        );
        assert_eq!(rel["location"]["range"]["start"]["line"], 3);
    }

    /// GH #856: a STDLIB-origin span numerically inside a seed
    /// file's window is not that file's. The embedded stdlib parses
    /// at base 0 in its own space, so offset 5 means byte 5 of
    /// `core.hl` — and the window test, which can only compare
    /// numbers, published it against `a.hl` at a position the
    /// reader never wrote. Origin is the only thing that can tell
    /// them apart, and it travels with the diagnostic.
    #[test]
    fn a_stdlib_origin_diagnostic_is_not_published_against_a_seed_file() {
        let (bases, sources) = seed();
        let inside = 5usize;
        assert!(
            hale_syntax::file_owns_offset(0, A_SRC.len() as u32, inside as u32),
            "the offset must COLLIDE with a.hl's window — that is \
             the case the window test cannot decide"
        );
        let d = hale_syntax::Diag::ty(
            hale_syntax::Span::new(inside, inside + 4),
            "the `alloc` effect happens here",
        )
        .in_stdlib();
        let mut per_file = empty_publish(&bases);
        place_checker_diags(&[d], &bases, &sources, &mut per_file);
        assert!(
            per_file.values().all(|v| v.is_empty()),
            "a stdlib span has no seed range to publish: {:?}",
            per_file
        );
    }

    /// …and a stdlib-origin SECONDARY location is published as a
    /// note on the message instead of a `relatedInformation` entry:
    /// that carries a range, and a range in the seed is exactly what
    /// this location does not have. The primary keeps its own range,
    /// so the finding still lands on the line the reader must change.
    #[test]
    fn a_stdlib_origin_related_location_becomes_a_note() {
        let (bases, sources) = seed();
        let d = hale_syntax::Diag::ty(
            hale_syntax::Span::new(0, 5),
            "effect assertion violated",
        )
        .with_related(hale_syntax::Span::new(10, 16), "declared here")
        .with_stdlib_related(
            hale_syntax::Span::new(5, 9),
            "the `alloc` effect happens here",
        );
        let mut per_file = empty_publish(&bases);
        place_checker_diags(&[d], &bases, &sources, &mut per_file);

        let published = &per_file[&PathBuf::from("/seed/a.hl")];
        assert_eq!(published.len(), 1, "{:?}", per_file);
        let v = &published[0];
        assert_eq!(v["range"]["start"]["line"], 0, "{}", v);
        // The seed-origin secondary is still clickable; the stdlib
        // one is NOT among them.
        let rel = v["relatedInformation"].as_array().expect("related");
        assert_eq!(rel.len(), 1, "only the seed location is a location: {}", v);
        assert_eq!(rel[0]["message"], "declared here", "{}", v);
        let msg = v["message"].as_str().unwrap_or("");
        assert!(
            msg.contains("note: the `alloc` effect happens here"),
            "the stdlib location rides as a note: {}",
            v
        );
        assert!(
            msg.contains("core.hl"),
            "and the note names the stdlib file the offset is in: {}",
            v
        );
    }
    // ---- demand accounting --------------------------------------------

    /// A seed with a keyed topic and no claims.
    const BUS_SRC: &str = "type Msg { room: String; text: String; }\n\
topic Posted { payload: Msg; subject: \"posted\"; keyed_by room; }\n\
locus Room {\n    params { name: String = \"lobby\"; }\n    bus { subscribe Posted as on_post where key == self.name; }\n    fn on_post(m: Msg) { println(self.name, m.text); }\n}\n\
main locus App {\n    params { r: Room = Room { }; }\n    bus { publish Posted; }\n    run() { Posted <- Msg { room: \"lobby\", text: \"t\" }; }\n}\n\
fn main() { App { }; }\n";

    /// The editor's snapshot of `file`'s seed, from the disk.
    fn load(file: &Path) -> Snapshot {
        match editor_snapshot(file, &BTreeMap::new()) {
            Ok(s) => s,
            Err(_) => panic!("the editor's load of {} failed", file.display()),
        }
    }

    /// F.40 phase 2.3: a request demands the families it reads and
    /// nothing else. `hale/busGraph` builds the scope once and the bus
    /// graph once (the model's graph, not one of its own) and no check;
    /// a hover builds the scope and, on a seed with no claims, no model.
    #[test]
    fn a_request_builds_only_the_families_it_reads() {
        let dir = std::env::temp_dir().join(format!("hale_lsp_demand_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("main.hl");
        std::fs::write(&file, BUS_SRC).unwrap();
        let unread = ["expression_typing", "ownership", "handler_routing", "model", "claims", "lowering_view"];

        let snap = load(&file);
        let graph = bus_graph_of(&snap);
        assert_eq!(graph["subjects"][0]["subject"], "Posted", "{graph}");
        assert_eq!(graph["subjects"][0]["staticDispatchEligible"], false, "{graph}");
        let builds = snap.builds();
        assert_eq!(builds["top_scope"], 1, "{builds:?}");
        assert_eq!(builds["bus_graph"], 1, "{builds:?}");
        for f in unread {
            assert_eq!(builds[f], 0, "hale/busGraph built {f}: {builds:?}");
        }

        let snap = load(&file);
        let line = BUS_SRC.lines().position(|l| l.contains("r: Room")).unwrap() as u32;
        let character = BUS_SRC.lines().nth(line as usize).unwrap().find("Room").unwrap() as u32;
        let h = hover_at(Some(&snap), &file, BUS_SRC, line, character).expect("a hover");
        assert!(h["contents"]["value"].as_str().unwrap_or("").contains("locus Room"), "{h}");
        let builds = snap.builds();
        assert_eq!(builds["top_scope"], 1, "{builds:?}");
        assert_eq!(builds["bus_graph"], 0, "{builds:?}");
        for f in unread {
            assert_eq!(builds[f], 0, "a hover built {f}: {builds:?}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// While a member does not parse, completion and hover answer from
    /// the editor's scope over the members that did, and nothing is
    /// checked; the requests that read a whole program say so.
    #[test]
    fn a_request_over_a_seed_with_a_hole_answers_from_the_members_that_parsed() {
        let dir = std::env::temp_dir().join(format!("hale_lsp_hole_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("main.hl");
        std::fs::write(&file, BUS_SRC).unwrap();
        std::fs::write(dir.join("broken.hl"), "fn broken( {\n").unwrap();

        let snap = load(&file);
        let line = BUS_SRC.lines().position(|l| l.contains("r: Room")).unwrap() as u32;
        let character = BUS_SRC.lines().nth(line as usize).unwrap().find("Room").unwrap() as u32;
        let h = hover_at(Some(&snap), &file, BUS_SRC, line, character).expect("a hover");
        assert!(h["contents"]["value"].as_str().unwrap_or("").contains("locus Room"), "{h}");
        let mut items = Vec::new();
        complete_top_level(Some(&snap), "Ro", &mut items);
        assert!(items.iter().any(|i| i["label"] == "Room"), "{items:?}");
        assert_eq!(bus_graph_of(&snap)["parseErrors"], true);
        assert_eq!(placement_of(&snap)["parseErrors"], true);
        assert_eq!(alloc_summary_of(&snap)["parseErrors"], true);
        assert!(enforcement_of(&snap)["fns"].as_array().is_some_and(|f| !f.is_empty()));
        let builds = snap.builds();
        assert_eq!(builds["top_scope"], 1, "the scope over the members that parsed, once: {builds:?}");
        assert_eq!(builds["expression_typing"], 0, "nothing is checked: {builds:?}");
        assert_eq!(builds["bus_graph"], 0, "{builds:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- a run of document events -------------------------------------

    /// F.40 phase 2.4: the document events queued at the front are
    /// applied in order and cost ONE publish pass, over the files they
    /// touched (the latest first); the request behind them is handled
    /// after that pass and before anything queued after it, which starts
    /// a run of its own.
    #[test]
    fn a_run_of_document_events_costs_one_pass_and_stops_at_a_request() {
        let (a, b) = (PathBuf::from("/seed/a.hl"), PathBuf::from("/seed/b.hl"));
        let change = |path: &Path, text: &str| json!({
            "jsonrpc": "2.0", "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": path_to_uri(path) },
                "contentChanges": [{ "text": text }]
            }
        });
        let hover = request(7, "textDocument/hover");
        let mut queue = VecDeque::from([
            change(&a, "1"),
            change(&a, "2"),
            change(&b, "1"),
            hover.clone(),
            change(&a, "3"),
        ]);
        let set = |p: &PathBuf, t: &str| Step::Apply(p.clone(), Buffer::Set(t.to_string()));
        assert_eq!(
            next_steps(&mut queue),
            vec![set(&a, "1"), set(&a, "2"), set(&b, "1"), Step::Publish(vec![b.clone(), a.clone()])]
        );
        assert_eq!(next_steps(&mut queue), vec![Step::Handle(hover)]);
        assert_eq!(next_steps(&mut queue), vec![set(&a, "3"), Step::Publish(vec![a.clone()])]);
        assert_eq!(next_steps(&mut queue), vec![]);
    }

    // ---- panic containment --------------------------------------------

    fn frame(v: Value) -> Vec<u8> {
        let body = v.to_string();
        format!("Content-Length: {}\r\n\r\n{}", body.len(), body).into_bytes()
    }

    /// Drive `serve` with these messages; return what it wrote and how
    /// it ended.
    fn run_session(messages: Vec<Value>) -> (Vec<Value>, String) {
        let input: Vec<u8> = messages.into_iter().flat_map(frame).collect();
        let mut out: Vec<u8> = Vec::new();
        let code = serve(std::io::Cursor::new(input), &mut out);
        let text = String::from_utf8(out).expect("utf-8 output");
        let mut replies = Vec::new();
        let mut rest = text.as_str();
        while let Some(at) = rest.find("Content-Length: ") {
            rest = &rest[at + "Content-Length: ".len()..];
            let (n, after) = rest.split_once("\r\n\r\n").expect("frame header");
            let n: usize = n.trim().parse().expect("length");
            replies.push(serde_json::from_str(&after[..n]).expect("json body"));
            rest = &after[n..];
        }
        (replies, format!("{code:?}"))
    }

    fn request(id: u64, method: &str) -> Value {
        json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": {} })
    }

    /// F.40 phase 2 review F4: a `Content-Length` past the bound, or one
    /// that is no number, is refused before anything is allocated, and
    /// the session exits non-zero once what came before it is handled.
    #[test]
    fn an_oversized_or_unparseable_frame_is_refused_and_the_session_fails() {
        for header in [
            format!("Content-Length: {}\r\n\r\n", MAX_FRAME_BYTES + 1),
            "Content-Length: 18446744073709551615\r\n\r\n".to_string(),
            "Content-Length: 99999999999999999999999\r\n\r\n".to_string(),
        ] {
            let mut input = frame(request(1, "initialize"));
            input.extend_from_slice(header.as_bytes());
            let mut out: Vec<u8> = Vec::new();
            let code = serve(std::io::Cursor::new(input), &mut out);
            let text = String::from_utf8(out).expect("utf-8 output");
            assert!(text.contains("\"capabilities\""), "the request before it is answered: {text}");
            assert!(code == ExitCode::from(1), "the session ends non-zero after {header}");
        }
    }

    /// A reader thread that panics ends the session non-zero, not as
    /// the clean EOF a client hanging up is.
    #[test]
    fn a_reader_thread_that_panics_fails_the_session() {
        struct Panics;
        impl std::io::Read for Panics {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                panic!("injected reader panic")
            }
        }
        let mut out: Vec<u8> = Vec::new();
        let code = serve(std::io::BufReader::new(Panics), &mut out);
        assert!(code == ExitCode::from(1), "the session ends non-zero");
    }

    fn reply_to(replies: &[Value], id: u64) -> &Value {
        replies
            .iter()
            .find(|r| r.get("id") == Some(&json!(id)))
            .unwrap_or_else(|| panic!("no reply to request {id}: {replies:?}"))
    }

    /// A request whose handler panics is answered with a JSON-RPC
    /// internal error carrying the report text, and the server goes on:
    /// the next request is answered and the session ends cleanly.
    #[test]
    fn a_request_handler_that_panics_is_answered_and_the_server_lives() {
        let (replies, code) = run_session(vec![
            request(1, "initialize"),
            request(2, "hale/testPanic"),
            request(3, "initialize"),
            request(4, "shutdown"),
            json!({ "jsonrpc": "2.0", "method": "exit" }),
        ]);
        let err = &reply_to(&replies, 2)["error"];
        assert_eq!(err["code"], -32603, "{err}");
        let message = err["message"].as_str().expect("message");
        assert!(
            message.starts_with("the compiler hit an internal error on this file: injected handler panic"),
            "{message}"
        );
        assert!(message.ends_with("; please report it with the file"), "{message}");
        assert!(
            reply_to(&replies, 3)["result"]["capabilities"].is_object(),
            "the request after the panic is answered: {replies:?}"
        );
        assert_eq!(reply_to(&replies, 4)["result"], Value::Null);
        assert_eq!(code, format!("{:?}", ExitCode::SUCCESS), "the session ends by its own exit");
    }

    /// A document event whose check panics publishes ONE diagnostic on
    /// that file, and the next event is checked normally. A request sits
    /// between the two changes, so a run cannot take both into one pass.
    #[test]
    fn a_check_that_panics_publishes_one_diagnostic_and_the_server_lives() {
        let dir = std::env::temp_dir().join(format!("hale_lsp_panic_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.hl");
        let uri = path_to_uri(&file);
        let change = |text: &str| json!({
            "jsonrpc": "2.0", "method": "textDocument/didChange",
            "params": { "textDocument": { "uri": uri }, "contentChanges": [{ "text": text }] }
        });
        let (replies, code) = run_session(vec![
            change("fn main() { // hale-lsp-test-panic\n"),
            request(8, "hale/testFence"),
            change("fn main() {\n    let x = 1;\n}\n"),
            request(9, "shutdown"),
            json!({ "jsonrpc": "2.0", "method": "exit" }),
        ]);
        let publishes: Vec<&Value> = replies
            .iter()
            .filter(|r| r["method"] == "textDocument/publishDiagnostics")
            .collect();
        assert!(publishes.len() >= 2, "the panic's publish and the next check's: {replies:?}");
        let first = &publishes[0]["params"];
        assert_eq!(first["uri"], json!(uri));
        let diags = first["diagnostics"].as_array().expect("diagnostics");
        assert_eq!(diags.len(), 1, "one diagnostic: {diags:?}");
        assert_eq!(diags[0]["severity"], 1);
        assert_eq!(diags[0]["source"], "hale");
        let message = diags[0]["message"].as_str().expect("message");
        assert!(
            message.starts_with("the compiler hit an internal error on this file: injected checker panic (at "),
            "{message}"
        );
        assert!(message.ends_with("; please report it with the file"), "{message}");
        let next = &publishes[1]["params"];
        assert_eq!(next["uri"], json!(uri));
        assert!(
            !next["diagnostics"].to_string().contains("internal error"),
            "the next check ran normally: {next}"
        );
        assert_eq!(reply_to(&replies, 9)["result"], Value::Null, "the server still answers");
        assert_eq!(code, format!("{:?}", ExitCode::SUCCESS));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- two publications ----------------------------------------------

    /// A program that breaks a law, beside a bare fallible call (a build
    /// rule, in the typing stage) that does not keep it from denoting a
    /// model.
    const LAW_SRC: &str = "locus B { params { n: Int = 0; } fn stop() { self.n = self.n + 1; } }\n\
locus A { params { b: B = B { }; } fn go() { self.b.stop(); } }\n\
group src = { A };\ngroup dst = { B };\n\
main locus App {\n    params { a: A = A { }; }\n    claims { isolation: forbid reaches(src, dst); }\n    run() { self.a.go(); save(); }\n}\n\
fn save() { std::io::fs::write_file(\"/tmp/hale-lsp-two-publications\", \"x\"); }\n\
fn main() { App { }; }\n";

    /// The publications `out` holds, as URI and messages.
    fn publications_in(out: &[u8]) -> Vec<(String, Vec<String>)> {
        let text = std::str::from_utf8(out).expect("utf-8 output");
        let mut pubs = Vec::new();
        let mut rest = text;
        while let Some(at) = rest.find("Content-Length: ") {
            rest = &rest[at + "Content-Length: ".len()..];
            let (n, after) = rest.split_once("\r\n\r\n").expect("frame header");
            let n: usize = n.trim().parse().expect("length");
            let v: Value = serde_json::from_str(&after[..n]).expect("json body");
            rest = &after[n..];
            let msgs = v["params"]["diagnostics"]
                .as_array()
                .map(|d| d.iter().map(|d| d["message"].as_str().unwrap_or("").to_string()).collect())
                .unwrap_or_default();
            pubs.push((v["params"]["uri"].as_str().unwrap_or("").to_string(), msgs));
        }
        pubs
    }

    /// F.40 phase 3, X2: a pass types the seed reusing its last typed
    /// snapshot, and publishes what a pass with nothing to reuse does; a
    /// hole types nothing and keeps the last typed snapshot for the pass
    /// after it.
    #[test]
    fn a_pass_reuses_the_seeds_last_typed_snapshot() {
        use hale_frontend::typing_reuse::TypingReuse;
        let dir = std::env::temp_dir().join(format!("hale_lsp_typing_reuse_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir = dir.canonicalize().unwrap();
        let file = dir.join("main.hl");
        std::fs::write(&file, LAW_SRC).unwrap();
        let seed = seed_key(&file);
        let checked: BTreeSet<PathBuf> = [seed.clone()].into_iter().collect();
        let mut typed = BTreeMap::new();
        let pass = |text: &str, typed: &mut BTreeMap<PathBuf, Snapshot>| {
            let mut out = Vec::new();
            let overlays = BTreeMap::from([(file.clone(), text.to_string())]);
            check_and_publish(&mut out, &file, &overlays, &mut BTreeMap::new(), typed, &checked, &mut || false);
            publications_in(&out)
        };
        let edited = LAW_SRC.replace("fn save() {", "fn save() { let x2: Int = \"probe\";");
        pass(LAW_SRC, &mut typed);
        assert_eq!(typed[&seed].typing_reuse(), Some(&TypingReuse::Fresh));
        let reusing = pass(&edited, &mut typed);
        assert!(matches!(typed[&seed].typing_reuse(), Some(TypingReuse::Reused { reused, .. }) if *reused > 0), "{:?}", typed[&seed].typing_reuse());
        assert_eq!(reusing, pass(&edited, &mut BTreeMap::new()), "what a pass with nothing to reuse publishes");
        pass("fn main( {", &mut typed);
        assert!(typed[&seed].typing_reuse().is_some(), "the hole keeps the last typed snapshot");
        pass(LAW_SRC, &mut typed);
        assert!(matches!(typed[&seed].typing_reuse(), Some(TypingReuse::Reused { .. })), "{:?}", typed[&seed].typing_reuse());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F.40 phase 3, X1: a seed whose law is broken is published twice,
    /// the typing stage and then the whole check, which replaces the
    /// first only on the file the law adds a finding to; the first is a
    /// prefix of the second. A newer document event queued before a
    /// publication discards it and what follows: before the first, both
    /// (and `published` is untouched); before the laws are judged, or
    /// after and before they are sent, the second alone. A superseded
    /// pass leaves its files for the next (`State::pending`).
    #[test]
    fn the_laws_replace_the_typing_stage_unless_a_newer_event_supersedes_them() {
        let dir = std::env::temp_dir().join(format!("hale_lsp_two_pubs_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dir = dir.canonicalize().unwrap();
        let file = dir.join("main.hl");
        std::fs::write(&file, LAW_SRC).unwrap();
        let uri = path_to_uri(&file);
        let checked: BTreeSet<PathBuf> = [seed_key(&file)].into_iter().collect();
        // `superseded` answers true from its `from`th question on.
        let pass = |from: usize| {
            let mut out = Vec::new();
            let mut published = BTreeMap::new();
            let mut asked = 0;
            let mut superseded = || {
                asked += 1;
                asked >= from
            };
            let ended = check_and_publish(&mut out, &file, &BTreeMap::new(), &mut published, &mut BTreeMap::new(), &checked, &mut superseded);
            (matches!(ended, Pass::Superseded), publications_in(&out), published)
        };

        let (superseded, pubs, published) = pass(usize::MAX);
        assert!(!superseded);
        let [(first_uri, first), (last_uri, last)] = pubs.as_slice() else {
            panic!("two publications of the one file: {pubs:?}");
        };
        assert_eq!((first_uri, last_uri), (&uri, &uri));
        assert!(first.iter().any(|m| m.contains("can fail (IoError)")), "the build rule is the typing stage's: {first:?}");
        assert!(first.iter().all(|m| !m.contains("claim `isolation` violated")), "{first:?}");
        assert_eq!(&last[..first.len()], &first[..], "the first publication is a prefix of the final one");
        assert!(
            !last[first.len()..].is_empty() && last[first.len()..].iter().all(|m| m.contains("claim `isolation`")),
            "the rest is the law's: {last:?}"
        );
        assert_eq!(published[&seed_key(&file)], [file.clone()].into_iter().collect());

        let (superseded, pubs, published) = pass(1);
        assert!(superseded && pubs.is_empty() && published.is_empty(), "nothing sent, nothing recorded: {pubs:?}");
        for from in [2, 3] {
            let (superseded, pubs, published) = pass(from);
            assert!(superseded, "from {from}");
            assert_eq!(pubs, vec![(uri.clone(), first.clone())], "from {from}: the first publication alone");
            assert_eq!(published[&seed_key(&file)], [file.clone()].into_iter().collect());
        }

        // A seed with no law: one publication, the laws asked about and
        // adding nothing.
        std::fs::write(&file, "fn main() {\n    let x: Int = \"text\";\n}\n").unwrap();
        let (_, pubs, _) = pass(usize::MAX);
        assert_eq!(pubs.len(), 1, "{pubs:?}");

        let mut state = State::default();
        let mut out = Vec::new();
        check_open_seeds(&mut out, &[file.clone()], &mut state, &mut || true);
        assert!(out.is_empty());
        assert_eq!(state.pending, vec![file.clone()], "the superseded pass's files wait for the next");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

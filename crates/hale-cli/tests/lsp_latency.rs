//! The editor's latency, first and final publication timed apart (F.40
//! phase 3, X1): a measurement, not a gate, so it is ignored by default.
//!
//! ```text
//! cargo test --release -p hale-cli --test tooling_services lsp_latency:: -- --ignored --nocapture
//! ```
//!
//! For each program it starts `hale lsp`, opens the file, then changes
//! it three times — one declaration's body edited, the same body edited
//! again, one newline appended ([`EVENTS`], X2: the first two are the
//! incremental typing stage's case, the edit before each one's previous
//! snapshot) — and times each event to the first
//! `publishDiagnostics` for the file and to the last one before a fence
//! request sent behind the event is answered: the typing stage's
//! publication, and the final one, which differs only for a program
//! whose laws add a finding. Medians of five sessions; with a base
//! binary, its sessions and this one's alternate. Set
//! `HALE_LSP_LATENCY_BASE` to another `hale` binary to time it beside
//! this one (a before-and-after on one machine). The same events taken
//! apart by stage, in process, are [`lsp_latency_by_stage`] (X3).

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Instant;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().expect("the repository")
}

struct Server {
    child: Child,
    out: BufReader<std::process::ChildStdout>,
}

impl Server {
    fn start(bin: &Path) -> Server {
        let mut child = Command::new(bin)
            .arg("lsp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn hale lsp");
        let out = BufReader::new(child.stdout.take().expect("stdout"));
        Server { child, out }
    }

    fn send(&mut self, v: serde_json::Value) {
        let body = v.to_string();
        let stdin = self.child.stdin.as_mut().expect("stdin");
        write!(stdin, "Content-Length: {}\r\n\r\n{}", body.len(), body).expect("write");
        stdin.flush().expect("flush");
    }

    fn recv(&mut self) -> serde_json::Value {
        let mut n = 0usize;
        loop {
            let mut line = String::new();
            self.out.read_line(&mut line).expect("read header");
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            if let Some(v) = line.strip_prefix("Content-Length:") {
                n = v.trim().parse().expect("length");
            }
        }
        let mut body = vec![0u8; n];
        self.out.read_exact(&mut body).expect("read body");
        serde_json::from_slice(&body).expect("json")
    }

    /// Send `event` and a fence behind it: milliseconds to the first and
    /// to the last publication for `uri` before the fence is answered,
    /// and how many there were.
    fn timed(&mut self, uri: &str, event: serde_json::Value, fence: u64) -> (f64, f64, usize) {
        let t0 = Instant::now();
        self.send(event);
        self.send(serde_json::json!({ "jsonrpc": "2.0", "id": fence, "method": "hale/testFence", "params": null }));
        let (mut first, mut last, mut n) = (None, f64::NAN, 0);
        loop {
            let m = self.recv();
            if m.get("id").and_then(|i| i.as_u64()) == Some(fence) {
                return (first.unwrap_or(f64::NAN), last, n);
            }
            if m["method"] == "textDocument/publishDiagnostics" && m["params"]["uri"] == uri {
                let t = t0.elapsed().as_secs_f64() * 1000.0;
                first.get_or_insert(t);
                last = t;
                n += 1;
            }
        }
    }
}

/// The events a session times, after the open: one declaration's body
/// edited (a `let` added to the first fn's body: the declaration's typing
/// changes, its interface does not), the same body edited again (the
/// steady state of typing in one place), and one newline appended (no
/// declaration changes).
const EVENTS: [&str; 4] = ["open", "body edit", "body edit again", "newline"];

/// `text` with `stmt` added at the start of the first fn's body.
fn body_edited(text: &str, stmt: &str) -> String {
    let at = text.find("fn ").expect("a fn");
    let brace = at + text[at..].find('{').expect("its body");
    format!("{}{stmt}{}", &text[..=brace], &text[brace + 1..])
}

/// One session over `file`: each of [`EVENTS`], (first ms, final ms,
/// publications).
fn session(bin: &Path, file: &Path) -> [(f64, f64, usize); 4] {
    let text = std::fs::read_to_string(file).expect("the program");
    let uri = format!("file://{}", file.display());
    let mut s = Server::start(bin);
    s.send(serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "capabilities": {} } }));
    let _ = s.recv();
    s.send(serde_json::json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }));
    let open = s.timed(
        &uri,
        serde_json::json!({ "jsonrpc": "2.0", "method": "textDocument/didOpen",
            "params": { "textDocument": { "uri": uri, "languageId": "hale", "version": 1, "text": text } } }),
        101,
    );
    let mut change = |version: u64, text: String| {
        s.timed(
            &uri,
            serde_json::json!({ "jsonrpc": "2.0", "method": "textDocument/didChange",
                "params": { "textDocument": { "uri": uri, "version": version }, "contentChanges": [{ "text": text }] } }),
            100 + version,
        )
    };
    let edit = change(2, body_edited(&text, " let x2_probe: Int = 1;"));
    let again = change(3, body_edited(&text, " let x2_probe: Int = 2;"));
    let newline = change(4, format!("{text}\n"));
    s.send(serde_json::json!({ "jsonrpc": "2.0", "id": 2, "method": "shutdown", "params": null }));
    while s.recv().get("id").and_then(|i| i.as_u64()) != Some(2) {}
    s.send(serde_json::json!({ "jsonrpc": "2.0", "method": "exit", "params": null }));
    let _ = s.child.wait();
    [open, edit, again, newline]
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.total_cmp(b));
    v[v.len() / 2]
}

#[test]
#[ignore = "a measurement: run with --ignored --nocapture"]
fn lsp_latency_first_and_final_publication() {
    let root = repo_root();
    let programs = [
        "dna/host/main.hl",
        "crates/hale-codegen/tests/fixtures/examples/hello-world/main.hl",
        "dna/tests/law/money_gate_fail/main.hl",
    ];
    let mut bins = vec![("this", PathBuf::from(env!("CARGO_BIN_EXE_hale")))];
    if let Some(base) = std::env::var_os("HALE_LSP_LATENCY_BASE") {
        bins.insert(0, ("base", PathBuf::from(base)));
    }
    println!("| binary | program | {} |", EVENTS.map(|e| format!("{e}: first / final")).join(" | "));
    println!("|---|---|---|---|---|---|");
    for p in programs {
        // The binaries' sessions alternate (X3), so a machine whose load
        // changes over the run loads both alike.
        let mut runs: Vec<Vec<[(f64, f64, usize); 4]>> = vec![Vec::new(); bins.len()];
        for _ in 0..5 {
            for ((_, bin), runs) in bins.iter().zip(&mut runs) {
                runs.push(session(bin, &root.join(p)));
            }
        }
        for ((label, _), runs) in bins.iter().zip(&runs) {
            let cell = |event: usize| {
                let first = median(runs.iter().map(|r| r[event].0).collect());
                let last = median(runs.iter().map(|r| r[event].1).collect());
                format!("{first:.0} / {last:.0} ms ({} pub.)", runs[0][event].2)
            };
            let cells: Vec<String> = (0..EVENTS.len()).map(cell).collect();
            println!("| {label} | {p} | {} |", cells.join(" | "));
        }
    }
}

/// The stages a pass of the editor runs, in order: the load, the families
/// the typing stage reads, each demanded on its own so it is timed on its
/// own, the typing stage itself (the checker, the reuse's plan, the build
/// rules, the advisory), diagnostic demangling (X3), then the laws and
/// final demangling. This excludes diagnostic positioning, advisory
/// filtering and protocol delivery; the server measurement above
/// includes those costs.
const STAGES: [&str; 9] = ["load", "entry", "scope", "forms", "handlers", "alloc summary", "typing", "demangle", "laws"];

/// The same events in process, each pass taken apart into [`STAGES`]
/// (F.40 phase 3, X3): the snapshot loaded through the session's parse
/// cache and offered the event before's, as the server does. A session's
/// process-wide state (the stdlib's analysis copy, parsed once per
/// process) is warm after the first session, so the open reads lower
/// here than from a fresh server. Medians of five sessions.
#[test]
#[ignore = "a measurement: run with --ignored --nocapture"]
fn lsp_latency_by_stage() {
    use hale_frontend::frontend::LoadMode;
    use hale_frontend::parse_cache::ParseCache;
    use hale_frontend::snapshot::{Config, Snapshot};
    use hale_frontend::source::Overlay;
    let root = repo_root();
    println!("| program | event | {} | first | final | reuse |", STAGES.join(" | "));
    println!("|---|---|{}---|---|---|", "---|".repeat(STAGES.len()));
    for p in ["dna/host/main.hl", "crates/hale-codegen/tests/fixtures/examples/hello-world/main.hl"] {
        let file = root.join(p);
        let text = std::fs::read_to_string(&file).expect("the program");
        let texts = [
            text.clone(),
            body_edited(&text, " let x2_probe: Int = 1;"),
            body_edited(&text, " let x2_probe: Int = 2;"),
            format!("{text}\n"),
        ];
        let mut runs: Vec<Vec<([f64; STAGES.len()], String)>> = Vec::new();
        for _ in 0..5 {
            let cache = ParseCache::new();
            let mut previous: Option<Snapshot> = None;
            let mut events = Vec::new();
            for t in &texts {
                let overlays = std::collections::BTreeMap::from([(file.clone(), t.clone())]);
                let mut ms = [0.0; STAGES.len()];
                let mut t0 = Instant::now();
                let mut lap = |stage: usize| {
                    ms[stage] = t0.elapsed().as_secs_f64() * 1000.0;
                    t0 = Instant::now();
                };
                let Ok(Ok(snap)) = Snapshot::load(&file, LoadMode::Editor, &Overlay::new(&overlays).reusing(&cache), Config::editor())
                    .map(Snapshot::linked)
                else {
                    panic!("{p} loads and links")
                };
                let snap = match previous.take() {
                    Some(previous) => snap.reusing_typing(previous),
                    None => snap,
                };
                lap(0);
                let _ = snap.demand_entry();
                lap(1);
                let _ = snap.demand_scope();
                lap(2);
                let _ = snap.demand_forms();
                lap(3);
                let _ = snap.demand_handlers();
                lap(4);
                let _ = snap.demand_alloc_summary();
                lap(5);
                let mut first = snap.demand_typing().expect("typed").diags.clone();
                lap(6);
                snap.demangler().demangle_diags(&mut first);
                lap(7);
                let mut last = snap.demand_check().expect("checked").diags.clone();
                snap.demangler().demangle_diags(&mut last);
                lap(8);
                events.push((ms, format!("{:?}", snap.typing_reuse().expect("typed"))));
                previous = Some(snap);
            }
            runs.push(events);
        }
        for (event, name) in EVENTS.iter().enumerate() {
            let at = |stage: usize| median(runs.iter().map(|r| r[event].0[stage]).collect());
            let first = median(runs.iter().map(|r| r[event].0[..STAGES.len() - 1].iter().sum()).collect());
            let last = median(runs.iter().map(|r| r[event].0.iter().sum()).collect());
            let cells: Vec<String> = (0..STAGES.len()).map(|s| format!("{:.0}", at(s))).collect();
            println!("| {p} | {name} | {} | {first:.0} | {last:.0} | {} |", cells.join(" | "), runs[0][event].1);
        }
    }
}

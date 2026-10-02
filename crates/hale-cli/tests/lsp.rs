//! `hale lsp` v1 — protocol-level integration test. Spawns the real
//! binary, speaks Content-Length-framed JSON-RPC over its stdio, and
//! walks the v1 lifecycle: initialize → didOpen (type error) →
//! didChange (fixed → diags clear) → didChange (warning shapes,
//! severity 2) → didChange (parse error) → shutdown/exit.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

struct Lsp {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Lsp {
    fn start() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_hale"))
            .arg("lsp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn hale lsp");
        let stdin = child.stdin.take().expect("stdin");
        let stdout = BufReader::new(child.stdout.take().expect("stdout"));
        Lsp { child, stdin, stdout }
    }

    fn send(&mut self, v: serde_json::Value) {
        self.send_all(vec![v]);
    }

    /// Several messages in ONE write, so the server finds them queued
    /// together. Only for small messages: this harness reads nothing
    /// while it writes, so a burst past the pipe's buffer could block
    /// on a server blocked on its own publishes.
    fn send_all(&mut self, msgs: Vec<serde_json::Value>) {
        let mut bytes = Vec::new();
        for v in msgs {
            let body = v.to_string();
            write!(bytes, "Content-Length: {}\r\n\r\n{}", body.len(), body).expect("frame");
        }
        self.stdin.write_all(&bytes).expect("write");
        self.stdin.flush().expect("flush");
    }

    fn recv(&mut self) -> serde_json::Value {
        let mut content_length = 0usize;
        loop {
            let mut line = String::new();
            self.stdout.read_line(&mut line).expect("read header");
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            if let Some(v) = line.strip_prefix("Content-Length:") {
                content_length = v.trim().parse().expect("length");
            }
        }
        let mut buf = vec![0u8; content_length];
        self.stdout.read_exact(&mut buf).expect("read body");
        serde_json::from_slice(&buf).expect("json")
    }
}

#[test]
fn lsp_v1_diagnostics_lifecycle() {
    // A private seed dir so sibling files can't interfere.
    let seed = std::env::temp_dir().join(format!(
        "hale_lsp_test_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&seed).expect("mkdir");
    let file = seed.join("main.hl");
    let uri = format!("file://{}", file.display());

    let broken = "fn main() {\n    let x: Int = \"not an int\";\n    println(x);\n}\n";
    let fixed = "fn main() {\n    let x: Int = 42;\n    println(x);\n}\n";
    let warny = "locus L {\n    params { n: Int = 0; }\n    run() {\n        let mut i = 0;\n        while true {\n            let b = std::bytes::BytesBuilder { };\n            i = i + 1;\n        }\n    }\n}\nfn main() { L { }; }\n";
    std::fs::write(&file, broken).expect("write seed file");

    let mut lsp = Lsp::start();

    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "capabilities": {} }
    }));
    let init = lsp.recv();
    assert_eq!(
        init.pointer("/result/capabilities/textDocumentSync/change"),
        Some(&serde_json::json!(1)),
        "full-document sync advertised: {}",
        init
    );

    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "method": "initialized", "params": {}
    }));

    // Open with a type error → one severity-1 diagnostic with a
    // real range on line 1.
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": { "textDocument": {
            "uri": uri, "languageId": "hale", "version": 1, "text": broken
        }}
    }));
    let open = lsp.recv();
    assert_eq!(
        open.get("method").and_then(|m| m.as_str()),
        Some("textDocument/publishDiagnostics")
    );
    let diags = open.pointer("/params/diagnostics").unwrap().as_array().unwrap();
    assert_eq!(diags.len(), 1, "one type error expected: {}", open);
    assert_eq!(diags[0]["severity"], 1);
    assert_eq!(diags[0]["range"]["start"]["line"], 1);
    assert!(
        diags[0]["message"].as_str().unwrap().contains("expected `Int`"),
        "got: {}",
        diags[0]
    );

    // Fix it → diagnostics clear (empty publish).
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": uri, "version": 2 },
            "contentChanges": [{ "text": fixed }]
        }
    }));
    let clear = lsp.recv();
    assert_eq!(
        clear.pointer("/params/diagnostics").unwrap().as_array().unwrap().len(),
        0,
        "stale diagnostics must clear: {}",
        clear
    );

    // Warning shapes → severity 2, hot-path lint present.
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": uri, "version": 3 },
            "contentChanges": [{ "text": warny }]
        }
    }));
    let warn = lsp.recv();
    let wdiags = warn.pointer("/params/diagnostics").unwrap().as_array().unwrap();
    assert!(!wdiags.is_empty(), "warnings expected: {}", warn);
    assert!(
        wdiags.iter().all(|d| d["severity"] == 2),
        "advisories map to severity 2: {}",
        warn
    );
    assert!(
        wdiags.iter().any(|d| d["message"]
            .as_str()
            .unwrap()
            .contains("hot-path allocation")),
        "hot-path lint expected: {}",
        warn
    );

    // Parse error → surfaced with the parse kind.
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": uri, "version": 4 },
            "contentChanges": [{ "text": "fn main( {\n" }]
        }
    }));
    let perr = lsp.recv();
    let pdiags = perr.pointer("/params/diagnostics").unwrap().as_array().unwrap();
    assert!(!pdiags.is_empty(), "parse error expected: {}", perr);
    assert_eq!(pdiags[0]["code"], "parse error");
    assert_eq!(pdiags[0]["severity"], 1);

    // Orderly shutdown.
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "id": 2, "method": "shutdown", "params": null
    }));
    let _ = lsp.recv();
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "method": "exit", "params": null
    }));
    let status = lsp.child.wait().expect("wait");
    assert!(status.success(), "clean exit after shutdown: {:?}", status);

    let _ = std::fs::remove_dir_all(&seed);
}

#[test]
fn lsp_v2_hover_and_bus_graph() {
    let seed = std::env::temp_dir().join(format!(
        "hale_lsp_v2_test_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&seed).expect("mkdir");
    let file = seed.join("main.hl");
    let uri = format!("file://{}", file.display());

    let src = r#"type Msg { room: String; text: String; }
topic Posted { payload: Msg; subject: "posted"; keyed_by room; }

locus Room {
    params { name: String = "lobby"; }
    bus { subscribe Posted as on_post where key == self.name; }
    fn on_post(m: Msg) { println(self.name, m.text); }
}

@hot @budget(alloc_per_call = 0) fn add_range(lo: Int, hi: Int) -> Int {
    let mut i = lo;
    let mut acc = 0;
    while i < hi { acc = acc + i; i = i + 1; }
    return acc;
}

main locus App {
    params { r: Room = Room { }; }
    bus { publish Posted; }
    run() {
        Posted <- Msg { room: "lobby", text: "t" };
        println(add_range(0, 10));
    }
}
fn main() { App { }; }
"#;
    std::fs::write(&file, src).expect("write");

    let mut lsp = Lsp::start();
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "capabilities": {} }
    }));
    let init = lsp.recv();
    assert_eq!(
        init.pointer("/result/capabilities/hoverProvider"),
        Some(&serde_json::json!(true))
    );
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": { "textDocument": {
            "uri": uri, "languageId": "hale", "version": 1, "text": src
        }}
    }));
    let _diags = lsp.recv();

    // Position helper: (line, col) of the first occurrence + 1.
    let pos = |needle: &str| -> (u32, u32) {
        for (ln, line) in src.lines().enumerate() {
            if let Some(col) = line.find(needle) {
                return (ln as u32, col as u32 + 1);
            }
        }
        panic!("needle not found: {}", needle);
    };

    let mut hover_at = |needle: &str, extra: u32| -> String {
        let (line, character) = pos(needle);
        lsp.send(serde_json::json!({
            "jsonrpc": "2.0", "id": 9, "method": "textDocument/hover",
            "params": {
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": character + extra }
            }
        }));
        let r = lsp.recv();
        r.pointer("/result/contents/value")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };

    // @hot @budget fn hover carries signature + enforcement status.
    let h = hover_at("add_range(0, 10)", 0);
    assert!(h.contains("fn add_range(lo: Int, hi: Int) -> Int"), "{}", h);
    assert!(h.contains("`@hot`"), "{}", h);
    assert!(h.contains("@budget(alloc_per_call = 0)"), "{}", h);

    // Keyed topic hover names the routing field.
    let h = hover_at("Posted <- ", 0);
    assert!(h.contains("topic Posted"), "{}", h);
    assert!(h.contains("keyed_by room"), "{}", h);

    // self.<field> hover resolves through the enclosing locus.
    let (line, character) = pos("self.name, m.text");
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "id": 9, "method": "textDocument/hover",
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": character + 6 }
        }
    }));
    let r = lsp.recv();
    let h = r
        .pointer("/result/contents/value")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert!(h.contains("self.name: String"), "{}", h);
    assert!(h.contains("locus Room"), "{}", h);

    // hale/busGraph: both subjects, keyed one honestly ineligible.
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "id": 10, "method": "hale/busGraph",
        "params": { "textDocument": { "uri": uri } }
    }));
    let g = lsp.recv();
    let subjects = g.pointer("/result/subjects").unwrap().as_array().unwrap();
    let posted = subjects
        .iter()
        .find(|s| s["subject"] == "Posted")
        .expect("Posted in graph");
    assert_eq!(posted["publishers"][0]["locus"], "App");
    assert_eq!(posted["subscribers"][0]["handler"], "on_post");
    assert_eq!(posted["subscribers"][0]["locus"], "Room");
    assert_eq!(posted["staticDispatchEligible"], false);
    assert!(
        posted["ineligibleReason"]
            .as_str()
            .unwrap()
            .contains("routing-key"),
        "{}",
        posted
    );

    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "id": 2, "method": "shutdown", "params": null
    }));
    let _ = lsp.recv();
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "method": "exit", "params": null
    }));
    let status = lsp.child.wait().expect("wait");
    assert!(status.success());
    let _ = std::fs::remove_dir_all(&seed);
}

#[test]
fn lsp_v3_definition_references_placement_alloc() {
    let seed = std::env::temp_dir().join(format!(
        "hale_lsp_v3_test_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&seed).expect("mkdir");
    let file = seed.join("main.hl");
    let uri = format!("file://{}", file.display());

    // A daemon shape: Worker churns a struct into self from an
    // unbounded run loop DIRECTLY (no method boundary) — the one
    // store shape the alloc survey still reports post-retirement.
    let src = r#"type Cell { s: String; n: Int; }

locus Worker {
    params { st: Cell = Cell { s: "", n: 0 }; }
    run() {
        let mut i = 0;
        while true {
            self.st = Cell { s: "v" + i, n: i };
            i = i + 1;
        }
    }
}

main locus App {
    params { w: Worker = Worker { }; }
    placement {
        w: pinned;
    }
    run() { }
}
fn main() { App { }; }
"#;
    std::fs::write(&file, src).expect("write");

    let mut lsp = Lsp::start();
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "capabilities": {} }
    }));
    let init = lsp.recv();
    assert_eq!(
        init.pointer("/result/capabilities/definitionProvider"),
        Some(&serde_json::json!(true))
    );
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": { "textDocument": {
            "uri": uri, "languageId": "hale", "version": 1, "text": src
        }}
    }));
    let _diags = lsp.recv();

    let pos = |needle: &str| -> (u32, u32) {
        for (ln, line) in src.lines().enumerate() {
            if let Some(col) = line.find(needle) {
                return (ln as u32, col as u32 + 1);
            }
        }
        panic!("needle not found: {}", needle);
    };

    // definition: `Cell { s: "v" + i` use → the type decl on line 0.
    let (line, character) = pos("Cell { s: \"v\"");
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "id": 2, "method": "textDocument/definition",
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": character }
        }
    }));
    let d = lsp.recv();
    assert_eq!(
        d.pointer("/result/range/start/line"),
        Some(&serde_json::json!(0)),
        "Cell defines on line 0: {}",
        d
    );

    // references: Worker appears at decl, params type, and literal.
    let (line, character) = pos("Worker = ");
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "id": 3, "method": "textDocument/references",
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": character },
            "context": { "includeDeclaration": true }
        }
    }));
    let refs = lsp.recv();
    let n = refs.pointer("/result").unwrap().as_array().unwrap().len();
    assert!(n >= 3, "Worker referenced at >= 3 sites, got {}: {}", n, refs);

    // hale/placement: the explicit pinned entry surfaces.
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "id": 4, "method": "hale/placement",
        "params": { "textDocument": { "uri": uri } }
    }));
    let pl = lsp.recv();
    assert_eq!(pl.pointer("/result/mainLocus"), Some(&serde_json::json!("App")));
    let fields = pl.pointer("/result/fields").unwrap().as_array().unwrap();
    let w = fields.iter().find(|f| f["field"] == "w").expect("w placed");
    assert_eq!(w["locus"], "Worker");
    assert_eq!(w["explicit"], true);
    assert!(
        w["placement"].as_str().unwrap().starts_with("pinned"),
        "{}",
        pl
    );

    // hale/allocSummary: the run-loop-direct churn is a leak site.
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "id": 5, "method": "hale/allocSummary",
        "params": { "textDocument": { "uri": uri } }
    }));
    let al = lsp.recv();
    let sites = al.pointer("/result/leakSites").unwrap().as_array().unwrap();
    assert!(!sites.is_empty(), "run-loop churn must report: {}", al);
    assert!(
        sites[0]["fn"].as_str().unwrap().contains("Worker"),
        "{}",
        al
    );
    assert!(
        al.pointer("/result/text").unwrap().as_str().unwrap().len() > 0
    );

    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "id": 6, "method": "shutdown", "params": null
    }));
    let _ = lsp.recv();
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "method": "exit", "params": null
    }));
    assert!(lsp.child.wait().expect("wait").success());
    let _ = std::fs::remove_dir_all(&seed);
}

#[test]
fn lsp_v4_completion() {
    let seed = std::env::temp_dir().join(format!(
        "hale_lsp_v4_test_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&seed).expect("mkdir");
    let file = seed.join("main.hl");
    let uri = format!("file://{}", file.display());

    // Line 6 (0-based) inside on_post gives three cursor sites:
    //   `self.` member completion, `std::str::` namespace
    //   completion, and a bare partial word. The buffer does NOT
    //   parse at those cursors (mid-keystroke) — context detection
    //   is text-based, so items must still arrive for self./std::
    //   (top-level symbols degrade to keywords-only on parse
    //   failure, which the bare-word probe uses a parseable buffer
    //   for).
    let src = r#"type Msg { room: String; text: String; }

locus Room {
    params { name: String = "lobby"; hits: Int = 0; }
    fn bump(n: Int) -> Int { return n + 1; }
    fn on_post(m: Msg) {
        println(self.name, m.text);
    }
}
fn main() { Room { }; }
"#;
    std::fs::write(&file, src).expect("write");

    let mut lsp = Lsp::start();
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "capabilities": {} }
    }));
    let init = lsp.recv();
    assert!(
        init.pointer("/result/capabilities/completionProvider").is_some(),
        "completionProvider capability missing"
    );
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": { "textDocument": {
            "uri": uri, "languageId": "hale", "version": 1, "text": src
        }}
    }));
    let _diags = lsp.recv();

    let labels = |resp: &serde_json::Value| -> Vec<String> {
        resp.pointer("/result/items")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|i| i.pointer("/label"))
                    .filter_map(|l| l.as_str())
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default()
    };

    // 1. `self.` → params (name, hits) + methods (bump, on_post).
    //    Edit line 6 to end mid-typing: `        self.`
    let edited = src.replace(
        "        println(self.name, m.text);",
        "        self.",
    );
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": uri, "version": 2 },
            "contentChanges": [{ "text": edited }]
        }
    }));
    let _diags = lsp.recv();
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "id": 2, "method": "textDocument/completion",
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": 6, "character": 13 }
        }
    }));
    let resp = lsp.recv();
    let ls = labels(&resp);
    assert!(ls.contains(&"name".to_string()), "self. params: {:?}", ls);
    assert!(ls.contains(&"hits".to_string()), "self. params: {:?}", ls);
    assert!(ls.contains(&"bump".to_string()), "self. methods: {:?}", ls);
    let bump = resp
        .pointer("/result/items")
        .and_then(|v| v.as_array())
        .and_then(|a| {
            a.iter().find(|i| i.pointer("/label")
                == Some(&serde_json::json!("bump")))
        })
        .cloned()
        .expect("bump item");
    assert!(
        bump.pointer("/detail")
            .and_then(|d| d.as_str())
            .is_some_and(|d| d.contains("-> Int")),
        "method detail: {:?}",
        bump
    );

    // 2. `std::str::` → stdlib namespace fns with signatures.
    let edited = src.replace(
        "        println(self.name, m.text);",
        "        std::str::",
    );
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": uri, "version": 3 },
            "contentChanges": [{ "text": edited }]
        }
    }));
    let _diags = lsp.recv();
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "id": 3, "method": "textDocument/completion",
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": 6, "character": 18 }
        }
    }));
    let resp = lsp.recv();
    let ls = labels(&resp);
    assert!(
        ls.contains(&"parse_int".to_string()),
        "std::str:: fns: {:?}",
        ls
    );

    // 3. `std::` → child namespaces.
    let edited = src.replace(
        "        println(self.name, m.text);",
        "        std::",
    );
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": uri, "version": 4 },
            "contentChanges": [{ "text": edited }]
        }
    }));
    let _diags = lsp.recv();
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "id": 4, "method": "textDocument/completion",
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": 6, "character": 13 }
        }
    }));
    let resp = lsp.recv();
    let ls = labels(&resp);
    assert!(ls.contains(&"str".to_string()), "std:: children: {:?}", ls);
    assert!(ls.contains(&"io".to_string()), "std:: children: {:?}", ls);

    // 4. Bare partial on a PARSEABLE buffer → top-level symbols +
    //    keywords. `Ro` should offer the Room locus; `wh` the
    //    while keyword.
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": uri, "version": 5 },
            "contentChanges": [{ "text": src }]
        }
    }));
    let _diags = lsp.recv();
    // Cursor right after `Ro` in `fn main() { Room { }; }` (line 9,
    // "fn main() { Ro|om" → character 14).
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "id": 5, "method": "textDocument/completion",
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": 9, "character": 14 }
        }
    }));
    let resp = lsp.recv();
    let ls = labels(&resp);
    assert!(ls.contains(&"Room".to_string()), "top-level: {:?}", ls);

    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "id": 6, "method": "shutdown", "params": null
    }));
    let _ = lsp.recv();
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "method": "exit", "params": null
    }));
    let _ = lsp.child.wait();
    let _ = std::fs::remove_dir_all(&seed);
}

#[test]
fn lsp_v5_formatting_symbols_enforcement() {
    let seed = std::env::temp_dir().join(format!(
        "hale_lsp_v5_test_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&seed).expect("mkdir");
    let file = seed.join("main.hl");
    let uri = format!("file://{}", file.display());

    // Deliberately messy spacing so formatting has work to do.
    let src = "locus Room {\n    params { name: String = \"lobby\"; }\n    @hot @budget(alloc_per_call = 0) fn bump(n:Int) -> Int { return n+1; }\n    fn fetch(u: String) -> Int fallible(IoError) { return 0; }\n}\nfn main() { Room { }; }\n";
    std::fs::write(&file, src).expect("write");

    let mut lsp = Lsp::start();
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "capabilities": {} }
    }));
    let init = lsp.recv();
    assert_eq!(
        init.pointer("/result/capabilities/documentFormattingProvider"),
        Some(&serde_json::json!(true))
    );
    assert_eq!(
        init.pointer("/result/capabilities/documentSymbolProvider"),
        Some(&serde_json::json!(true))
    );
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": { "textDocument": {
            "uri": uri, "languageId": "hale", "version": 1, "text": src
        }}
    }));
    let _diags = lsp.recv();

    // Formatting: one whole-document edit whose newText is the
    // canonical form (spaces around + and after :).
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "id": 2, "method": "textDocument/formatting",
        "params": {
            "textDocument": { "uri": uri },
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    }));
    let resp = lsp.recv();
    let new_text = resp
        .pointer("/result/0/newText")
        .and_then(|v| v.as_str())
        .expect("one edit");
    assert!(new_text.contains("fn bump(n: Int) -> Int { return n + 1; }"),
        "{}", new_text);

    // Document symbols: Room (class) with params field + methods.
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "id": 3, "method": "textDocument/documentSymbol",
        "params": { "textDocument": { "uri": uri } }
    }));
    let resp = lsp.recv();
    let syms = resp.pointer("/result").and_then(|v| v.as_array()).expect("syms");
    let room = syms
        .iter()
        .find(|s| s["name"] == "Room")
        .expect("Room symbol");
    assert_eq!(room["kind"], 5, "locus = Class");
    let children: Vec<&str> = room["children"]
        .as_array()
        .expect("children")
        .iter()
        .filter_map(|c| c["name"].as_str())
        .collect();
    assert!(children.contains(&"name"), "{:?}", children);
    assert!(children.contains(&"bump"), "{:?}", children);

    // hale/enforcement: bump carries hot + budget, fetch fallible.
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "id": 4, "method": "hale/enforcement",
        "params": { "textDocument": { "uri": uri } }
    }));
    let resp = lsp.recv();
    let fns = resp.pointer("/result/fns").and_then(|v| v.as_array()).expect("fns");
    let bump = fns
        .iter()
        .find(|f| f["name"] == "Room.bump")
        .expect("Room.bump");
    assert_eq!(bump["hot"], true);
    assert_eq!(bump["budget"], 0);
    let fetch = fns
        .iter()
        .find(|f| f["name"] == "Room.fetch")
        .expect("Room.fetch");
    assert_eq!(fetch["fallible"], "IoError");

    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "id": 5, "method": "shutdown", "params": null
    }));
    let _ = lsp.recv();
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "method": "exit", "params": null
    }));
    let _ = lsp.child.wait();
    let _ = std::fs::remove_dir_all(&seed);
}

/// The outline reads the open file's member program, which the editor's
/// load keeps however the rest of the seed fares (outside review of
/// #1295, finding 2): an import that does not resolve blocks the check
/// but not the outline, which answers what the file declares as for a
/// healthy seed and for a seed whose sibling does not parse. A file that
/// does not parse itself has no outline. The diagnostics of the
/// missing-import seed are still the import failure `hale check` reports.
#[test]
fn lsp_outline_survives_a_link_failure() {
    const BODY: &str = "fn helper() -> Int { return 1; }\nfn main() { println(helper()); }\n";
    let root = std::env::temp_dir().join(format!("hale_lsp_outline_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    // (seed, files) — the outline is asked of each seed's main.hl
    let seeds: [(&str, Vec<(&str, String)>); 4] = [
        ("healthy", vec![("main.hl", BODY.to_string())]),
        ("missing_import", vec![("main.hl", format!("import \"missing\" as m;\n{BODY}"))]),
        ("malformed_sibling", vec![("main.hl", BODY.to_string()), ("broken.hl", "fn broken( {\n".to_string())]),
        ("unparseable", vec![("main.hl", format!("fn broken( {{\n{BODY}"))]),
    ];
    for (name, files) in &seeds {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).expect("mkdir");
        for (f, text) in files {
            std::fs::write(dir.join(f), text).expect("write");
        }
    }
    let uri = |name: &str| format!("file://{}", root.join(name).join("main.hl").display());

    let mut lsp = Lsp::start();
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "capabilities": {} }
    }));
    let _ = lsp.recv();
    let mut outline = |id: u64, name: &str| -> Vec<String> {
        lsp.send(serde_json::json!({
            "jsonrpc": "2.0", "id": id, "method": "textDocument/documentSymbol",
            "params": { "textDocument": { "uri": uri(name) } }
        }));
        let resp = lsp.recv();
        assert_eq!(resp["id"], id, "{resp}");
        resp["result"]
            .as_array()
            .unwrap_or_else(|| panic!("{name}: an array: {resp}"))
            .iter()
            .filter_map(|s| s["name"].as_str().map(str::to_string))
            .collect()
    };
    let own = vec!["helper".to_string(), "main".to_string()];
    assert_eq!(outline(2, "healthy"), own, "a healthy seed");
    assert_eq!(outline(3, "missing_import"), own, "an import that does not resolve");
    assert_eq!(outline(4, "malformed_sibling"), own, "a sibling that does not parse");
    assert_eq!(outline(5, "unparseable"), Vec::<String>::new(), "a file that does not parse has no outline");

    // the check still refuses the missing-import seed, as `hale check` does
    let text = &seeds[1].1[0].1;
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": { "textDocument": {
            "uri": uri("missing_import"), "languageId": "hale", "version": 1, "text": text
        }}
    }));
    // one publish per file the load names; the open file's is among them
    let open = loop {
        let msg = lsp.recv();
        if msg.pointer("/params/uri").and_then(|u| u.as_str()) == Some(uri("missing_import").as_str()) {
            break msg;
        }
    };
    let diags = open.pointer("/params/diagnostics").and_then(|d| d.as_array()).expect("diagnostics");
    assert!(
        diags.iter().any(|d| d["severity"] == 1 && d["message"].as_str().is_some_and(|m| m.contains("missing"))),
        "the import failure is published: {open}"
    );

    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "id": 6, "method": "shutdown", "params": null
    }));
    let _ = lsp.recv();
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "method": "exit", "params": null
    }));
    let _ = lsp.child.wait();
    let _ = std::fs::remove_dir_all(&root);
}

/// Downstream handoff (2026-08-11): a diagnostic with a secondary
/// location — duplicate top-level name pointing at the previous
/// declaration — publishes `relatedInformation`, which clients
/// render as a clickable second location. Before, the previous
/// declaration reached editors as a Debug-formatted `Span { .. }`
/// inside the message text.
#[test]
fn lsp_duplicate_name_carries_related_information() {
    let seed = std::env::temp_dir().join(format!(
        "hale_lsp_related_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&seed);
    std::fs::create_dir_all(&seed).expect("mkdir");
    let file = seed.join("main.hl");
    let uri = format!("file://{}", file.display());
    let dup = "type Widget { n: Int; }\ntype Widget { m: Int; }\n\nfn main() { println(\"x\"); }\n";
    std::fs::write(&file, dup).expect("write seed file");

    let mut lsp = Lsp::start();
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "capabilities": {} }
    }));
    let _ = lsp.recv();
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "method": "initialized", "params": {}
    }));
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": { "textDocument": {
            "uri": uri, "languageId": "hale", "version": 1, "text": dup
        }}
    }));
    let open = lsp.recv();
    let diags =
        open.pointer("/params/diagnostics").unwrap().as_array().unwrap();
    let dup_diag = diags
        .iter()
        .find(|d| {
            d["message"]
                .as_str()
                .map_or(false, |m| m.contains("duplicate top-level"))
        })
        .unwrap_or_else(|| panic!("duplicate diagnostic published: {}", open));
    assert!(
        !dup_diag["message"].as_str().unwrap().contains("Span {"),
        "no Debug span in the editor payload: {}",
        dup_diag
    );
    // The primary points at line 2 (0-based 1); the related entry
    // points at line 1 (0-based 0) with a clickable location.
    assert_eq!(dup_diag["range"]["start"]["line"], 1, "{}", dup_diag);
    let rel = &dup_diag["relatedInformation"][0];
    assert_eq!(
        rel["location"]["range"]["start"]["line"], 0,
        "related points at the previous declaration: {}",
        dup_diag
    );
    assert_eq!(rel["message"], "previous declaration", "{}", dup_diag);

    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "id": 9, "method": "shutdown", "params": {}
    }));
    let _ = lsp.recv();
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "method": "exit", "params": {}
    }));
    let _ = lsp.child.wait();
    let _ = std::fs::remove_dir_all(&seed);
}

/// GH #856: an effect assertion whose witness leaf lands in a
/// STDLIB body raises a second diagnostic positioned in the embedded
/// stdlib's own parse space. That space starts at base 0, so in a
/// seed large enough to contain the offset — ~40 KB here, against a
/// leaf ~13 KB into `AP_SOURCE` — the window test could only see a
/// number inside `main.hl` and published a squiggle on a line of
/// filler the author never wrote. A stdlib span is not a seed range,
/// so the editor is not handed one.
#[test]
fn lsp_never_squiggles_a_stdlib_span_in_a_seed_file() {
    let seed = std::env::temp_dir().join(format!(
        "hale_lsp_stdlib_origin_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&seed);
    std::fs::create_dir_all(&seed).expect("mkdir");
    let file = seed.join("main.hl");
    let uri = format!("file://{}", file.display());
    let mut text = String::from(
        "@effects(none: {alloc})\n\
         fn ship(s: std::io::tcp::Stream) {\n\
         \x20   s.send(\"tick\") or discard;\n\
         }\n\
         \n\
         fn main() {\n\
         \x20   let s = std::io::tcp::Stream { conn_fd: 1, owns_fd: false };\n\
         \x20   ship(s);\n\
         }\n",
    );
    for i in 0..1_100 {
        text.push_str(&format!("fn filler_{i}() -> Int {{ return {i}; }}\n"));
    }
    std::fs::write(&file, &text).expect("write seed file");

    let mut lsp = Lsp::start();
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "capabilities": {} }
    }));
    let _ = lsp.recv();
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "method": "initialized", "params": {}
    }));
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": { "textDocument": {
            "uri": uri, "languageId": "hale", "version": 1, "text": text
        }}
    }));
    let open = lsp.recv();
    let diags =
        open.pointer("/params/diagnostics").unwrap().as_array().unwrap();

    // The finding is published, on the asserting fn's own line.
    let violation = diags
        .iter()
        .find(|d| {
            d["message"]
                .as_str()
                .is_some_and(|m| m.contains("effect assertion violated"))
        })
        .unwrap_or_else(|| panic!("the violation is published: {}", open));
    assert_eq!(violation["range"]["start"]["line"], 1, "{}", violation);

    // …and nothing else is. The witness leaf has no range in this
    // document, so the editor is given none rather than one pointing
    // into the filler.
    for d in diags {
        let line = d["range"]["start"]["line"].as_u64().unwrap_or(0);
        assert!(
            line < 9,
            "a diagnostic on line {} is in the filler — no line past \
             the program has anything wrong with it: {}",
            line,
            d
        );
        assert!(
            !d["message"]
                .as_str()
                .unwrap_or("")
                .starts_with("the `alloc` effect happens here"),
            "the stdlib-positioned leaf must not be published as a \
             document diagnostic: {}",
            d
        );
    }

    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "id": 9, "method": "shutdown", "params": {}
    }));
    let _ = lsp.recv();
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "method": "exit", "params": {}
    }));
    let _ = lsp.child.wait();
    let _ = std::fs::remove_dir_all(&seed);
}

/// Downstream handoff (2026-08-11): `textDocument/definition` on a
/// `std::` path jumps INTO the embedded stdlib source. The rename
/// table maps the path to the mangled declaration in AP_SOURCE, and
/// the owning per-domain file is materialized to a versioned
/// read-only cache — so the location is a plain `file://` URI that
/// works in every editor, even when the binary was installed with
/// no stdlib checkout on disk.
#[test]
fn lsp_definition_resolves_std_paths_into_materialized_stdlib() {
    let seed = std::env::temp_dir().join(format!(
        "hale_lsp_stddef_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&seed);
    std::fs::create_dir_all(&seed).expect("mkdir");
    let file = seed.join("main.hl");
    let uri = format!("file://{}", file.display());
    let text = "fn main() {\n    let b = std::bytes::BytesBuilder { };\n    b.append_str(\"x\");\n}\n";
    std::fs::write(&file, text).expect("write seed file");

    let mut lsp = Lsp::start();
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "capabilities": {} }
    }));
    let _ = lsp.recv();
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "method": "initialized", "params": {}
    }));
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": { "textDocument": {
            "uri": uri, "languageId": "hale", "version": 1, "text": text
        }}
    }));
    let _ = lsp.recv(); // publishDiagnostics

    // Cursor on `BytesBuilder` (line 1, inside the last segment).
    let character = text.lines().nth(1).unwrap().find("BytesBuilder").unwrap() + 3;
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "id": 2, "method": "textDocument/definition",
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": 1, "character": character }
        }
    }));
    let d = lsp.recv();
    let def_uri = d
        .pointer("/result/uri")
        .and_then(|u| u.as_str())
        .unwrap_or_else(|| panic!("std:: definition resolves: {}", d));
    assert!(
        def_uri.ends_with("bytes_builder.hl"),
        "lands in the owning per-domain file: {}",
        def_uri
    );
    assert!(
        def_uri.contains("stdlib-"),
        "materialized under the versioned cache: {}",
        def_uri
    );

    // The materialized file exists, is read-only, and the returned
    // range points at the mangled declaration's name.
    let def_path = std::path::PathBuf::from(
        def_uri.strip_prefix("file://").unwrap(),
    );
    let content =
        std::fs::read_to_string(&def_path).expect("materialized file");
    assert!(
        std::fs::metadata(&def_path).unwrap().permissions().readonly(),
        "cache file is read-only"
    );
    let line = d.pointer("/result/range/start/line").unwrap().as_u64().unwrap()
        as usize;
    assert!(
        content.lines().nth(line).unwrap().contains("__StdBytesBytesBuilder"),
        "range points at the declaration: line {} = {:?}",
        line,
        content.lines().nth(line)
    );

    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "id": 9, "method": "shutdown", "params": {}
    }));
    let _ = lsp.recv();
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "method": "exit", "params": {}
    }));
    let _ = lsp.child.wait();
    let _ = std::fs::remove_dir_all(&seed);
}

/// A file inside the materialized stdlib cache
/// (`…/hale/stdlib-<version>/…`) is a definition-jump target, not
/// a seed: analyzed standalone it would spray spurious errors over
/// correct stdlib code (mangled declarations and path-call
/// primitives only resolve in the merged program). The LSP
/// publishes an EMPTY diagnostic set for such files.
#[test]
fn lsp_stdlib_cache_files_get_no_diagnostics() {
    let dir = std::env::temp_dir()
        .join(format!("hale_lsp_cachedq_{}", std::process::id()))
        .join("hale")
        .join("stdlib-0.0.0-test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    let file = dir.join("core.hl");
    // Standalone-invalid content of the kind real stdlib files
    // carry (path-call primitive references, mangled names).
    let text = "fn __StdProbe(n: Int) -> Int {\n    return std::__nonexistent::path(n);\n}\n";
    std::fs::write(&file, text).expect("write");
    let uri = format!("file://{}", file.display());

    let mut lsp = Lsp::start();
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "capabilities": {} }
    }));
    let _ = lsp.recv();
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "method": "initialized", "params": {}
    }));
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": { "textDocument": {
            "uri": uri, "languageId": "hale", "version": 1, "text": text
        }}
    }));
    let open = lsp.recv();
    assert_eq!(
        open.get("method").and_then(|m| m.as_str()),
        Some("textDocument/publishDiagnostics"),
        "{}",
        open
    );
    assert_eq!(
        open.pointer("/params/diagnostics")
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        0,
        "stdlib cache files publish empty diagnostics: {}",
        open
    );

    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "id": 9, "method": "shutdown", "params": {}
    }));
    let _ = lsp.recv();
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "method": "exit", "params": {}
    }));
    let _ = lsp.child.wait();
}

/// GH #476 Change 9 (review round 1): a claim diagnostic must be
/// published against the file the claim is IN.
///
/// Claim verdicts are judged over the canonical model, whose
/// provenance resolves through the bundle's source map — and the
/// LSP was building its bundle with `Bundle::new`, which leaves
/// that map empty, even though the editor already holds every
/// base, path and text. Unplaceable claim spans then collapsed to
/// byte zero, which the LSP resolves to the first line of the first
/// file in the seed. A two-file seed makes that visible: the claim
/// lives in the SECOND file, so a byte-zero regression publishes it
/// against the first.
#[test]
fn lsp_publishes_claim_diagnostics_against_the_right_file() {
    let seed = std::env::temp_dir().join(format!(
        "hale_lsp_claims_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&seed);
    std::fs::create_dir_all(&seed).expect("mkdir");

    // File one sorts first and holds the loci; the claim is in file
    // two, well past byte zero of the seed.
    let a = seed.join("a_domain.hl");
    let b = seed.join("b_app.hl");
    std::fs::write(
        &a,
        "locus B { params { n: Int = 0; } fn stop() { self.n = self.n + 1; } }\n\
         locus A {\n    params { b: B = B { }; }\n    fn go() { self.b.stop(); }\n}\n\
         group src = { A };\ngroup dst = { B };\n",
    )
    .expect("write a");
    let app = "main locus App {\n    params { a: A = A { }; }\n    claims {\n        isolation: forbid reaches(src, dst);\n    }\n    run() { self.a.go(); }\n}\nfn main() { App { }; }\n";
    std::fs::write(&b, app).expect("write b");

    let uri_b = format!("file://{}", b.display());
    let mut lsp = Lsp::start();
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "capabilities": {} }
    }));
    let _ = lsp.recv();
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "method": "initialized", "params": {}
    }));
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": { "textDocument": {
            "uri": uri_b, "languageId": "hale", "version": 1, "text": app
        }}
    }));

    // Collect publishes until we see the claim (the server may
    // publish per-file).
    let mut found: Option<(String, u64)> = None;
    for _ in 0..8 {
        let msg = lsp.recv();
        if msg.get("method").and_then(|m| m.as_str())
            != Some("textDocument/publishDiagnostics")
        {
            continue;
        }
        let uri = msg.pointer("/params/uri").and_then(|u| u.as_str())
            .unwrap_or("")
            .to_string();
        let empty = Vec::new();
        let diags = msg
            .pointer("/params/diagnostics")
            .and_then(|d| d.as_array())
            .unwrap_or(&empty);
        for d in diags {
            let m = d["message"].as_str().unwrap_or("");
            // The PRIMARY violation only. The secondary notes
            // ("the boundary is crossed by this call", "the
            // forbidden destination is declared here") legitimately
            // live in the other file — that they do is the
            // multi-file placement working, not a failure.
            if m.contains("claim `isolation` violated") {
                found = Some((
                    uri.clone(),
                    d["range"]["start"]["line"].as_u64().unwrap_or(u64::MAX),
                ));
            }
        }
        if found.is_some() {
            break;
        }
    }
    let (uri, line) = found.expect(
        "the violated claim must be published as a diagnostic",
    );
    assert!(
        uri.ends_with("b_app.hl"),
        "the claim lives in b_app.hl but was published against {}",
        uri
    );
    // `claims {` is line 2 (0-based) and the clause is line 3.
    assert_eq!(
        line, 3,
        "expected the claim clause's own line, got line {} in {}",
        line, uri
    );
    let _ = std::fs::remove_dir_all(&seed);
}

/// One diagnostic, as all three channels can spell it: the file's
/// NAME (the three runs sit in three directories), 1-based line and
/// column, the message.
type Finding = (String, u64, u64, String);

/// A seed's files: name and text.
type Files = &'static [(&'static str, &'static str)];

/// The plain parity seed (`lsp_overlays_lsp_on_disk_and_check_agree`).
const PLAIN: Files = &[
    ("a.hl", "fn helper(n: Int) -> Int {\n    let s: String = n;\n    return n + 1;\n}\n"),
    ("b.hl", "fn main() {\n    let x: Int = helper(1);\n    let y: Int = \"not an int\";\n    println(x);\n    save();\n    Work { };\n}\n"),
    (
        "c.hl",
        "interface Performer { fn perform(x: Int) -> Int; }\n\
locus Doubler { fn perform(x: Int) -> Int { return x * 2; } }\n\
locus Rt { params { performer: Performer = Doubler { }; } fn go() -> Int { return self.performer.perform(2); } }\n\
locus Work {\n    params { rt: Rt = Rt { }; }\n    fn rewire() { let d = Doubler { }; self.rt = Rt { performer: d }; }\n    run() { self.rewire(); }\n}\n\
fn save() {\n    std::io::fs::write_file(\"/tmp/hale-lsp-parity\", \"x\");\n}\n",
    ),
];

/// The importing parity seed and its library
/// (`lsp_and_check_agree_over_a_seed_that_imports`).
const IMPORTING: Files =
    &[("main.hl", "import \"lib\" as lib;\n\nfn main() {\n    let x: String = lib::helper(1);\n    println(x);\n}\n")];
const IMPORTED: Files = &[("lib.hl", "fn helper(n: Int) -> Int {\n    return n + 1;\n}\n")];

/// The generated-source parity seed
/// (`lsp_and_check_agree_over_a_seed_with_generated_source`).
const GENERATED: Files = &[(
    "main.hl",
    "type Order { id: Int `json:\"id\"`; qty: Int `json:\"qty\"`; }\n\
type Ack { id: Int; }\n\
topic Orders { payload: Order; subject: \"app.order\"; }\n\
locus Desk {\n\
    bus { subscribe Orders as on_order; }\n\
    fn on_order(o: Order) -> Ack {\n\
        let bad: String = o.id;\n\
        return Ack { id: o.id };\n\
    }\n\
}\n\
main locus App {\n\
    params { desk: Desk = Desk { }; }\n\
    bindings { api: unix(\"/tmp/hale-lsp-parity-generated.sock\", bound: 4, on_full: refuse); }\n\
    run() {\n\
        let o = Order::from_json(\"{}\");\n\
        println(o.id);\n\
    }\n\
}\n\
fn main() { App { }; }\n",
)];

/// The author's-leak parity seed
/// (`lsp_and_check_report_an_authors_leak_in_a_handler_the_api_binding_calls`).
const AUTHOR_LEAK_APP: &str = "type Order { id: Int `json:\"id\"`; qty: Int `json:\"qty\"`; }\n\
type Ack { id: Int; }\n\
type Seen { ids: [Int; 2]; }\n\
topic Orders { payload: Order; subject: \"app.order\"; }\n\
locus Desk {\n\
    params { seen: Seen = Seen { ids: [0, 0] }; }\n\
    bus { subscribe Orders as on_order; }\n\
    fn on_order(o: Order) -> Ack {\n\
        self.seen = Seen { ids: [o.id, o.qty] };\n\
        return Ack { id: o.id };\n\
    }\n\
}\n\
main locus App {\n\
    params { desk: Desk = Desk { }; }\n\
    bindings { api: unix(\"/tmp/hale-lsp-parity-author-leak.sock\", bound: 4, on_full: refuse); }\n\
    run() {\n\
        println(\"up\");\n\
    }\n\
}\n\
fn main() { App { }; }\n";
const AUTHOR_LEAK: Files = &[("main.hl", AUTHOR_LEAK_APP)];

/// The laws parity seed (`lsp_and_check_agree_over_a_seed_with_laws`):
/// a law the program breaks, beside a finding of each kind the typing
/// stage carries — a build rule (a bare fallible call) and the
/// allocation advisory (a builder per loop turn) — none of which keeps
/// the program from denoting a model, so the law is judged. The law
/// sits in the second file, past the first file's bytes. The app imports
/// a library with an advisory of its own, which neither `hale check` nor
/// either publication reports (an advisory about an imported seed is
/// that seed's check's).
const LAWS: Files = &[
    (
        "a_domain.hl",
        "locus B {\n    params { n: Int = 0; }\n    fn stop() { self.n = self.n + 1; }\n    run() {\n        let mut i = 0;\n        while true {\n            let b = std::bytes::BytesBuilder { };\n            i = i + 1;\n        }\n    }\n}\n\
locus A {\n    params { b: B = B { }; }\n    fn go() { self.b.stop(); }\n}\n\
group src = { A };\ngroup dst = { B };\n",
    ),
    (
        "b_app.hl",
        "import \"lib\" as lib;\n\
main locus App {\n    params { a: A = A { }; }\n    claims {\n        isolation: forbid reaches(src, dst);\n    }\n    run() { self.a.go(); save(); println(lib::tick()); }\n}\n\
fn save() {\n    std::io::fs::write_file(\"/tmp/hale-lsp-parity-laws\", \"x\");\n}\n\
fn main() { App { }; }\n",
    ),
];
const LAWS_LIB: Files = &[(
    "lib.hl",
    "fn tick() -> Int { return 1; }\n\
locus Spin {\n    run() {\n        let mut i = 0;\n        while true {\n            let b = std::bytes::BytesBuilder { };\n            i = i + 1;\n        }\n    }\n}\n",
)];

/// Every parity seed: its tag, its files and its library's.
const PARITY: &[(&str, Files, Files)] = &[
    ("plain", PLAIN, &[]),
    ("import", IMPORTING, IMPORTED),
    ("generated", GENERATED, &[]),
    ("author-leak", AUTHOR_LEAK, &[]),
    ("laws", LAWS, LAWS_LIB),
];

/// The laws parity fixture (F.40 phase 3, X1): a program that breaks a
/// law and holds a finding of each kind the typing stage carries is
/// checked three ways, one answer, the law among the findings.
#[test]
fn lsp_and_check_agree_over_a_seed_with_laws() {
    let check = agree_three_ways("laws", LAWS, LAWS_LIB);
    assert!(check.iter().all(|(f, ..)| f != "lib.hl"), "the library's advisory is its own check's: {check:?}");
    for (file, want) in [
        ("b_app.hl", "claim `isolation` violated"),
        ("b_app.hl", "can fail (IoError) and this call says nothing about it"),
        ("a_domain.hl", "unbounded allocation"),
    ] {
        assert!(
            check.iter().any(|(f, .., m)| f == file && m.contains(want)),
            "no `{want}` finding in {file}: {check:?}"
        );
    }
}

/// The editor's two publications (F.40 phase 3, X1), over the laws
/// parity seed, on open and on an edit: every file of the seed once with
/// the check's typing stage, then the whole check on the one file the
/// law adds a finding to, and nothing else. Per file, the first
/// publication is a prefix of the final one and the rest is the law's;
/// the first carries the typing stage's findings (the build rule, the
/// advisory) and no law; the library's advisory is in neither, as `hale
/// check` reports none; and each file's final list holds `hale check`'s
/// findings for it.
#[test]
fn lsp_publishes_the_typing_stage_first_and_the_laws_replace_it() {
    let root = scratch_root("two-publications");
    std::fs::create_dir_all(root.join("lib")).expect("mkdir");
    let dir = root.canonicalize().expect("canonical dir");
    for (f, text) in LAWS {
        std::fs::write(dir.join(f), text).expect("write app");
    }
    for (f, text) in LAWS_LIB {
        std::fs::write(dir.join("lib").join(f), text).expect("write lib");
    }
    let (check, _) = check_json(&dir);
    let (domain, app) = (dir.join(LAWS[0].0), dir.join(LAWS[1].0));
    let mut lsp = LspSession::start();
    lsp.lsp.send(open(&app, LAWS[1].1));
    let opened = lsp.publications();
    lsp.lsp.send(change(&app, 2, &format!("{}\n", LAWS[1].1)));
    let edited = lsp.publications();
    lsp.close();
    let _ = std::fs::remove_dir_all(&root);

    let sorted = |mut v: Vec<String>| {
        v.sort();
        v
    };
    let seed = [uri(&domain), uri(&app)];
    for (event, pubs) in [("open", &opened), ("edit", &edited)] {
        let uris: Vec<&String> = pubs.iter().map(|(u, _)| u).collect();
        assert!(pubs.len() > seed.len(), "{event}: two publications: {pubs:?}");
        let (first, second) = pubs.split_at(seed.len());
        assert_eq!(&uris[..seed.len()], &[&seed[0], &seed[1]], "{event}: the first publication, every file once: {pubs:?}");
        for (i, (u, _)) in second.iter().enumerate() {
            assert!(seed.contains(u) && second[..i].iter().all(|(v, _)| v != u), "{event}: the second, a file of the seed at most once: {pubs:?}");
        }
        assert!(first[1].1.iter().any(|m| m.contains("can fail (IoError)")), "{event}: the build rule comes first: {first:?}");
        assert!(first[0].1.iter().any(|m| m.contains("unbounded allocation")), "{event}: the advisory comes first: {first:?}");
        assert!(
            first.iter().flat_map(|(_, m)| m).all(|m| !m.contains("claim `isolation`")),
            "{event}: no law in the first publication: {first:?}"
        );
        for (file, (u, initial)) in LAWS.iter().map(|(f, _)| *f).zip(first) {
            let last = second.iter().find(|(v, _)| v == u).map_or(initial, |(_, l)| l);
            assert_eq!(&last[..initial.len()], &initial[..], "{event}: {file}'s first publication is a prefix of its final one");
            assert!(last[initial.len()..].iter().all(|m| m.contains("claim `isolation`")), "{event}: the rest is the law's: {last:?}");
            assert_eq!(second.iter().any(|(v, _)| v == u), last.len() > initial.len(), "{event}: {file} is published again only when the law adds to it");
            let want: Vec<String> = check.iter().filter(|(f, ..)| f == file).map(|(.., m)| m.clone()).collect();
            assert_eq!(sorted(last.clone()), sorted(want), "{event}: {file}'s final list is hale check's");
        }
        assert!(second.iter().any(|(u, _)| *u == seed[1]), "{event}: the violation itself, on the app: {second:?}");
    }
}

/// A diagnostic as the stage test compares it: kind, span, message.
fn diag_key(d: &hale_syntax::Diag) -> (String, usize, usize, String) {
    (d.kind_str().to_string(), d.span.start.as_usize(), d.span.end.as_usize(), d.message.clone())
}

fn diag_keys<'d>(diags: impl IntoIterator<Item = &'d hale_syntax::Diag>) -> Vec<(String, usize, usize, String)> {
    diags.into_iter().map(diag_key).collect()
}

/// The check's two stages over every parity seed (F.40 phase 3, X1).
///
/// - `demand_check` is `demand_typing`'s diagnostics followed by
///   `demand_laws`', each stage counted once.
/// - The typing stage ends with what the editor's config asks of it,
///   the build rules then the allocation advisory; the laws are the
///   claims judged over the model, finished after the typing's own, and
///   the two stages' own findings repeat nothing.
/// - Today's single pass (`finish_check_diags` over the typing and the
///   claims, then the rules and the advisory) reports the same findings,
///   only with the laws ahead of the rules and the advisory.
/// - `hale check`'s composition (its snapshot's check, which holds no
///   rule and no advisory, then the advisory and the rules it runs
///   itself) reports the same findings as the editor's check.
#[test]
fn the_check_is_its_typing_stage_followed_by_its_laws_stage() {
    use hale_frontend::frontend::LoadMode;
    use hale_frontend::snapshot::{Config, Snapshot};
    use hale_frontend::source::Disk;
    let load = |entry: &std::path::Path, mode: LoadMode, config: Config| match Snapshot::load(entry, mode, &Disk, config) {
        Ok(s) => s,
        Err(_) => panic!("{} loads", entry.display()),
    };
    let mut laws_judged = 0;
    for (tag, app, lib) in PARITY {
        let root = scratch_root(&format!("stages-{tag}"));
        std::fs::create_dir_all(root.join("lib")).expect("mkdir");
        let dir = root.canonicalize().expect("canonical dir");
        for (f, text) in *app {
            std::fs::write(dir.join(f), text).expect("write app");
        }
        for (f, text) in *lib {
            std::fs::write(dir.join("lib").join(f), text).expect("write lib");
        }
        let (last, _) = app.last().expect("an app file");
        let s = load(&dir.join(last), LoadMode::Editor, Config::editor());
        let typing = s.demand_typing().expect("typed").diags.clone();
        let laws = s.demand_laws().expect("judged").diags.clone();
        let check = s.demand_check().expect("checked").diags.clone();
        assert_eq!(diag_keys(&check), diag_keys(typing.iter().chain(&laws)), "{tag}: the check is its two stages");
        let builds = s.builds();
        for stage in ["typing_stage", "laws_stage", "expression_typing"] {
            assert_eq!(builds[stage], 1, "{tag}: `{stage}` once");
        }

        let bundle = s.bundle();
        let tail: Vec<hale_syntax::Diag> = hale_types::build_rule_diags(&bundle)
            .into_iter()
            .chain(hale_types::unbounded_alloc_warnings(&bundle, s.demand_alloc_summary().expect("the summary"), true))
            .collect();
        assert!(typing.len() >= tail.len(), "{tag}: the typing stage holds the rules and the advisory");
        let own = &typing[..typing.len() - tail.len()];
        assert_eq!(diag_keys(&typing[own.len()..]), diag_keys(&tail), "{tag}: the rules, then the advisory");
        let mut claims = Vec::new();
        if hale_types::denotes_a_model(own) && hale_types::judgment::has_claim_surface(&bundle) {
            let model = s.demand_model().expect("a model");
            let effects = s.demand_effect_certificates().expect("the report");
            claims = s.with_env(|| hale_types::judgment::claim_law_diags_over(&bundle, model, effects, s.demand_alloc_summary().expect("the summary")));
            laws_judged += 1;
        }
        let mut finished = claims.clone();
        hale_types::finish_check_diags_after(own, &mut finished);
        assert_eq!(diag_keys(&laws), diag_keys(&finished), "{tag}: the laws, finished after the typing's own");
        let mut seen = std::collections::BTreeSet::new();
        for k in diag_keys(own.iter().chain(&laws)) {
            assert!(seen.insert(k.clone()), "{tag}: {k:?} repeats");
        }
        let sorted = |mut v: Vec<(String, usize, usize, String)>| {
            v.sort();
            v
        };
        let mut today = own.to_vec();
        today.extend(claims);
        hale_types::finish_check_diags(&mut today);
        today.extend(tail);
        assert_eq!(sorted(diag_keys(&check)), sorted(diag_keys(&today)), "{tag}: the findings of today's single pass");

        let c = load(&dir, LoadMode::WholeSeed, Config::check(true, false));
        let (c_typing, c_laws) = (c.demand_typing().expect("typed"), c.demand_laws().expect("judged"));
        let c_check = c.demand_check().expect("checked");
        assert_eq!(diag_keys(&c_check.diags), diag_keys(c_typing.diags.iter().chain(&c_laws.diags)), "{tag}: hale check's too");
        let c_bundle = c.bundle();
        let cli: Vec<hale_syntax::Diag> = c_check
            .diags
            .iter()
            .cloned()
            .chain(hale_types::unbounded_alloc_warnings(&c_bundle, c.demand_alloc_summary().expect("the summary"), true))
            .chain(hale_types::build_rule_diags(&c_bundle))
            .collect();
        assert_eq!(sorted(diag_keys(&cli)), sorted(diag_keys(&check)), "{tag}: hale check's findings are the editor's");
        let _ = std::fs::remove_dir_all(&root);
    }
    assert_eq!(laws_judged, 1, "the laws seed's law is judged, and only there");
}

/// A diagnostic whole, as the incremental stage is held to it: kind,
/// origin, span, message, and every related location.
fn full_key(d: &hale_syntax::Diag) -> String {
    format!("{d:?}")
}

/// The editor's load of `entry` through `overlays`, typed: reusing
/// `previous` when one is given (F.40 phase 3, X2), whole otherwise.
fn typed_snapshot(
    entry: &std::path::Path,
    overlays: &std::collections::BTreeMap<std::path::PathBuf, String>,
    previous: Option<hale_frontend::snapshot::Snapshot>,
) -> (hale_frontend::snapshot::Snapshot, Vec<String>) {
    use hale_frontend::frontend::LoadMode;
    use hale_frontend::snapshot::{Config, Snapshot};
    use hale_frontend::source::Overlay;
    let Ok(snap) = Snapshot::load(entry, LoadMode::Editor, &Overlay::new(overlays), Config::editor()) else {
        panic!("{} loads", entry.display())
    };
    let snap = match previous {
        Some(p) => snap.reusing_typing(p),
        None => snap,
    };
    let keys = snap.demand_typing().expect("typed").diags.iter().map(full_key).collect();
    (snap, keys)
}

/// The edits of one step: per file, the bodies to edit (by the span of
/// the body's opening brace in the file) and the text each gets first.
fn edit_bodies(text: &str, at: &[usize], insert: &str) -> String {
    let mut out = text.to_string();
    let mut at = at.to_vec();
    at.sort_unstable_by(|a, b| b.cmp(a));
    for i in at {
        out.insert_str(i + 1, insert);
    }
    out
}

/// One edit sequence over a seed: from a fresh typing, each step's text
/// is typed reusing the previous step's snapshot (as the editor chains
/// them) and fresh, and the two typing stages must be one, diagnostic
/// for diagnostic. Returns each step's reuse.
fn incremental_equals_full(
    tag: &str,
    entry: &std::path::Path,
    steps: &[std::collections::BTreeMap<std::path::PathBuf, String>],
) -> Vec<hale_frontend::typing_reuse::TypingReuse> {
    let (mut prev, _) = typed_snapshot(entry, &std::collections::BTreeMap::new(), None);
    let mut reuses = Vec::new();
    for (n, overlays) in steps.iter().enumerate() {
        let (snap, incremental) = typed_snapshot(entry, overlays, Some(prev));
        let (_, fresh) = typed_snapshot(entry, overlays, None);
        assert_eq!(incremental, fresh, "{tag}, step {n}: the incremental typing stage is the full one");
        reuses.push(snap.typing_reuse().expect("typed").clone());
        prev = snap;
    }
    reuses
}

/// The bodies an edit sequence edits: for each fn and locus declaration
/// of `snap`'s own `file`, the file offset of its body's opening brace
/// (a locus's first member with a body).
fn bodies_in(snap: &hale_frontend::snapshot::Snapshot, file: &std::path::Path) -> Vec<(String, usize)> {
    use hale_syntax::ast::{LocusMember, TopDecl};
    let (base, _, len) = snap.file_bases().iter().find(|(_, p, _)| p == file).cloned().expect("the file's base");
    let mut out = Vec::new();
    for d in snap.declarations() {
        let body = match &snap.programs()[&d.program].items[d.index] {
            TopDecl::Fn(f) => Some(f.body.span),
            TopDecl::Locus(l) => l.members.iter().find_map(|m| match m {
                LocusMember::Fn(f) => Some(f.body.span),
                LocusMember::Lifecycle(lc) => Some(lc.body.span),
                _ => None,
            }),
            _ => None,
        };
        if let Some(b) = body.filter(|b| base <= b.start.0 && b.end.0 <= base + len) {
            out.push((d.name.clone(), (b.start.0 - base) as usize));
        }
    }
    out
}

/// The editor's incremental typing stage is the full one (F.40 phase 3,
/// X2): over every parity seed, each fn and locus body of the app's files
/// edited (an ill-typed `let` added) and undone in turn, then two edited
/// at once, then a declared surface changed and a newline appended; each
/// step typed reusing the step before and fresh, the two compared
/// diagnostic for diagnostic, whole. A body edit reuses the declarations
/// it does not reach; an edit to what a declaration declares checks the
/// seed whole.
#[test]
fn the_incremental_typing_stage_is_the_full_one() {
    use hale_frontend::typing_reuse::TypingReuse;
    const PROBE: &str = " let x2_probe: Int = \"probe\";";
    let (mut reused, mut whole) = (0, 0);
    for (tag, app, lib) in PARITY {
        let root = scratch_root(&format!("incremental-{tag}"));
        std::fs::create_dir_all(root.join("lib")).expect("mkdir");
        let dir = root.canonicalize().expect("canonical dir");
        for (f, text) in *app {
            std::fs::write(dir.join(f), text).expect("write app");
        }
        for (f, text) in *lib {
            std::fs::write(dir.join("lib").join(f), text).expect("write lib");
        }
        let entry = dir.join(app.last().expect("an app file").0);
        let (s0, _) = typed_snapshot(&entry, &std::collections::BTreeMap::new(), None);
        let mut steps = Vec::new();
        let mut twice = Vec::new();
        for (f, text) in *app {
            let path = dir.join(f);
            let bodies = bodies_in(&s0, &path);
            for (_, at) in &bodies {
                steps.push(std::collections::BTreeMap::from([(path.clone(), edit_bodies(text, &[*at], PROBE))]));
                steps.push(std::collections::BTreeMap::from([(path.clone(), text.to_string())]));
            }
            if bodies.len() >= 2 {
                twice.push((path.clone(), edit_bodies(text, &[bodies[0].1, bodies[1].1], PROBE)));
            }
        }
        steps.extend(twice.into_iter().map(|(p, t)| std::collections::BTreeMap::from([(p, t)])));
        let (f, text) = app[0];
        steps.push(std::collections::BTreeMap::from([(dir.join(f), format!("{text}\nfn x2_added() {{ }}\n"))]));
        steps.push(std::collections::BTreeMap::from([(dir.join(f), format!("{text}\n"))]));
        for reuse in incremental_equals_full(tag, &entry, &steps) {
            match reuse {
                TypingReuse::Reused { reused: n, .. } if n > 0 => reused += 1,
                TypingReuse::Whole(_) => whole += 1,
                _ => {}
            }
        }
        let _ = std::fs::remove_dir_all(&root);
    }
    assert!(reused > 10, "body edits reuse what they do not reach: {reused} steps");
    assert!(whole >= PARITY.len(), "an added declaration checks the seed whole: {whole} steps");
}

/// The same equality over `dna/host` (X2): one declaration's body
/// edited, the edit undone, two edited at once; each step reuses the
/// rest of the seed and types exactly as a fresh check does.
#[test]
fn the_incremental_typing_stage_is_the_full_one_over_the_dna_host() {
    use hale_frontend::typing_reuse::TypingReuse;
    const PROBE: &str = " let x2_probe: Int = \"probe\";";
    let host = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dna/host").canonicalize().expect("dna/host");
    let entry = host.join("main.hl");
    let (s0, _) = typed_snapshot(&entry, &std::collections::BTreeMap::new(), None);
    let pick = |file: &str, n: usize| -> (std::path::PathBuf, String, usize) {
        let path = host.join(file);
        let text = std::fs::read_to_string(&path).expect("the host's file");
        let (_, at) = bodies_in(&s0, &path).into_iter().nth(n).expect("a body to edit");
        (path, text, at)
    };
    let (main, main_text, a) = pick("main.hl", 0);
    let (verbs, verbs_text, b) = pick("verbs.hl", 0);
    let steps = vec![
        std::collections::BTreeMap::from([(main.clone(), edit_bodies(&main_text, &[a], PROBE))]),
        std::collections::BTreeMap::from([(main.clone(), main_text.clone())]),
        std::collections::BTreeMap::from([
            (main.clone(), edit_bodies(&main_text, &[a], PROBE)),
            (verbs.clone(), edit_bodies(&verbs_text, &[b], PROBE)),
        ]),
    ];
    for (n, reuse) in incremental_equals_full("dna/host", &entry, &steps).into_iter().enumerate() {
        assert!(
            matches!(reuse, TypingReuse::Reused { reused, .. } if reused > 1000),
            "step {n}: the rest of the host is reused: {reuse:?}"
        );
    }
}

/// A revealed secret reaches the wire through `enc` in `Api`'s
/// `on_failure` handler, `enc`'s only caller (outside review of #1321).
/// `enc` sits after its caller, so editing it moves nothing in `Api`
/// and only the dependents relation can say `Api` reads it.
const REVEAL_ON_FAILURE: &str = "locus Child { }

locus Api {
    params {
        token: std::secret::Credential =
            std::secret::Credential { vault: \"api\" };
    }
    on_failure(c: Child, err: ClosureViolation) {
        let r = std::http::post(
            \"http://127.0.0.1:1/t\",
            std::bytes::from_string(\"secret=\" + enc(self.token.reveal_text())),
            \"text/plain\"
        ) or std::http::ClientResponse {
            status: 0, headers: \"\", body: b\"\"
        };
    }
}

fn main() { let a = Api { }; }

fn enc(s: String) -> String { return s + \"!\"; }
";

/// The same reveal in a block-valued params initializer, `enc`'s only
/// caller (outside review of #1321).
const REVEAL_INITIALIZER: &str = "locus Api {
    params {
        token: std::secret::Credential =
            std::secret::Credential { vault: \"api\" };
        sent: Int = {
            let r = std::http::post(
                \"http://127.0.0.1:1/t\",
                std::bytes::from_string(\"secret=\" + enc(self.token.reveal_text())),
                \"text/plain\"
            ) or std::http::ClientResponse {
                status: 0, headers: \"\", body: b\"\"
            };
            1
        };
    }
}

fn main() { let a = Api { }; }

fn enc(s: String) -> String { return s + \"!\"; }
";

/// The edit both reveal seeds take: a print in `enc`, which makes it
/// opaque to the reveal rule, so the reveal in `Api` is refused.
fn enc_edited(text: &str) -> String {
    let edited = text.replace("{ return s + \"!\"; }", "{ println(\"changed\"); return s + \"!\"; }");
    assert_ne!(edited, text, "the seed has `enc`");
    edited
}

const ENC_OPAQUE: &str = "here it reaches `enc`, which is not a wire write";

/// The incremental typing stage is the full one when the only caller of
/// an edited helper is an `on_failure` handler or a params initializer,
/// bodies that are no fn's row in the allocation summary (outside review
/// of #1321): the edit is reused around, never over, the declaration
/// that reads it.
#[test]
fn the_incremental_typing_stage_is_the_full_one_through_handler_and_initializer_calls() {
    use hale_frontend::typing_reuse::TypingReuse;
    for (tag, text) in [("on-failure", REVEAL_ON_FAILURE), ("initializer", REVEAL_INITIALIZER)] {
        let root = scratch_root(&format!("x2-reveal-{tag}"));
        let dir = root.canonicalize().expect("canonical dir");
        let entry = dir.join("main.hl");
        std::fs::write(&entry, text).expect("write seed");
        let edited = enc_edited(text);
        let steps = vec![
            std::collections::BTreeMap::from([(entry.clone(), edited.clone())]),
            std::collections::BTreeMap::from([(entry.clone(), text.to_string())]),
        ];
        let reuses = incremental_equals_full(tag, &entry, &steps);
        // Not vacuous: the edit refuses the reveal, and the stage reused
        // what the edit does not reach.
        let (_, fresh) = typed_snapshot(&entry, &steps[0], None);
        assert!(fresh.iter().any(|d| d.contains(ENC_OPAQUE)), "{tag}: the edit refuses the reveal: {fresh:?}");
        for (n, reuse) in reuses.iter().enumerate() {
            assert!(
                matches!(reuse, TypingReuse::Reused { checked, reused } if *checked >= 2 && *reused >= 1),
                "{tag}, step {n}: `enc` and `Api` checked, the rest reused: {reuse:?}"
            );
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}

/// The LSP's `didChange` publication after the helper's body edit is
/// `hale check --json`'s over the same text (outside review of #1321):
/// the reveal reached only through an `on_failure` handler or a params
/// initializer is refused in the editor as at the command line.
#[test]
fn lsp_publishes_what_check_reports_after_a_helper_edit_through_handler_and_initializer_calls() {
    for (tag, text) in [("on-failure", REVEAL_ON_FAILURE), ("initializer", REVEAL_INITIALIZER)] {
        let root = scratch_root(&format!("x2-reveal-lsp-{tag}"));
        let dir = root.canonicalize().expect("canonical dir");
        let main = dir.join("main.hl");
        std::fs::write(&main, text).expect("write seed");
        let edited = enc_edited(text);
        let mut lsp = LspSession::start();
        lsp.lsp.send(open(&main, text));
        let opened = lsp.published();
        lsp.lsp.send(change(&main, 2, &edited));
        let changed = lsp.published();
        lsp.close();
        std::fs::write(&main, &edited).expect("write the edit");
        let (check, _) = check_json(&dir);
        let _ = std::fs::remove_dir_all(&root);

        assert!(opened.is_empty(), "{tag}: the seed checks clean: {opened:?}");
        assert!(check.iter().any(|(.., m)| m.contains(ENC_OPAQUE)), "{tag}: hale check refuses the reveal: {check:?}");
        assert_eq!(changed, check, "{tag}: the didChange publication and `hale check --json` disagree");
    }
}

/// The overlay parity fixture (F.40 phase 2.1a): one seed, checked
/// three ways, one answer.
///
/// 1. `hale check --json` on the directory.
/// 2. The LSP with the files on disk and no buffer open: a `didSave`
///    without text checks from the disk alone.
/// 3. The LSP with every file's text supplied as a buffer, twice: over
///    an EMPTY directory (the files exist only as buffers, so the seed
///    is whatever the server's source provider lists) and over STALE
///    disk copies (every buffer must win over its file).
///
/// The seed is three files in one directory, and `b.hl` calls a `fn`
/// declared in `a.hl`, so a file the server failed to load shows up as
/// an unresolved name, not as silence. `c.hl` holds one of each rule
/// `hale check` runs beside its check — a borrow that does not outlive
/// its holder and a bare fallible call — which the editor's check
/// carries as the build rules (`Config::editor`): asserted here, not
/// trusted.
#[test]
fn lsp_overlays_lsp_on_disk_and_check_agree() {
    let check = agree_three_ways("plain", PLAIN, &[]);
    // Not vacuous: a finding in each file, the cross-file call among
    // none (the whole seed was loaded), and one of each build rule.
    for f in ["a.hl", "b.hl", "c.hl"] {
        assert!(check.iter().any(|(file, ..)| file == f), "hale check found nothing in {f}: {check:?}");
    }
    for rule in [
        "would hold a borrow that does not outlive it",
        "can fail (IoError) and this call says nothing about it",
    ] {
        assert!(check.iter().any(|(.., m)| m.contains(rule)), "no `{rule}` finding: {check:?}");
    }
}

/// The overlay parity fixture over a seed that `import`s a sibling
/// library (F.40 phase 2.3): the editor loads the whole seed as `hale
/// check <dir>` does, every import followed through the buffers, so
/// the two answer alike. The call's result type comes from the library
/// alone: a server that did not follow the import would not see the
/// mismatch.
#[test]
fn lsp_and_check_agree_over_a_seed_that_imports() {
    let check = agree_three_ways("import", IMPORTING, IMPORTED);
    assert!(
        check.iter().any(|(file, line, ..)| file == "main.hl" && *line == 4),
        "hale check found no mismatch at the imported call: {check:?}"
    );
}

/// The overlay parity fixture over a seed with generated source (F.40
/// phase 2.4): a `json:`-tagged type, whose parser the sequence
/// synthesizes, and an api binding, whose envelope types, topics and
/// socket loci it synthesizes. Both channels run the same sequence, so
/// the author's errors beside the generated declarations (a mismatch
/// in the handler the binding calls, a bare fallible `from_json`) land
/// at the same author positions, and nothing is reported at a position
/// inside generated code.
#[test]
fn lsp_and_check_agree_over_a_seed_with_generated_source() {
    let check = agree_three_ways("generated", GENERATED, &[]);
    assert!(
        check.iter().all(|(file, ..)| file == "main.hl"),
        "a finding positioned outside the author's file (inside generated source?): {check:?}"
    );
    for line in [7u64, 15u64] {
        assert!(
            check.iter().any(|(_, l, ..)| *l == line),
            "no finding at main.hl:{line}: {check:?}"
        );
    }
}

/// The generated-source fixture's positive (F.40 phase 2 review F1): a
/// bound-solver finding is dropped only when its site has no author
/// position, so a leak the author wrote in the handler the api binding
/// calls — a whole-value replace of a stored struct, per message — is
/// reported at the author's line by all three channels, and
/// `hale/allocSummary` lists that site and none in the binding's own
/// generated code.
#[test]
fn lsp_and_check_report_an_authors_leak_in_a_handler_the_api_binding_calls() {
    const APP: &str = AUTHOR_LEAK_APP;
    let check = agree_three_ways("author-leak", AUTHOR_LEAK, &[]);
    assert!(
        check.iter().all(|(file, ..)| file == "main.hl"),
        "a finding positioned outside the author's file: {check:?}"
    );
    assert!(
        check.iter().any(|(_, line, _, m)| *line == 9 && m.contains("unbounded allocation")),
        "the author's per-message replace at main.hl:9 is not reported: {check:?}"
    );

    let root = scratch_root("author-leak-summary");
    let main = root.canonicalize().expect("canonical dir").join("main.hl");
    let mut lsp = LspSession::start();
    lsp.lsp.send(open(&main, APP));
    let _ = lsp.published();
    let summary = lsp.request("hale/allocSummary", serde_json::json!({ "textDocument": { "uri": uri(&main) } }));
    lsp.close();
    let _ = std::fs::remove_dir_all(&root);
    let sites = summary["leakSites"].as_array().expect("leakSites");
    assert!(
        sites.iter().any(|s| s["fn"] == "Desk::on_order" && s["location"]["range"]["start"]["line"] == 8),
        "hale/allocSummary does not list the author's site: {summary}"
    );
    assert!(
        sites.iter().all(|s| s["location"]["uri"] == uri(&main)),
        "hale/allocSummary lists a site with no author position: {summary}"
    );
}

/// A seed member that will not read (a dangling symlink here; any
/// unreadable `.hl` member is the same case): `hale check` refuses the
/// load, and the editor says so instead of publishing a clean seed —
/// `seed member <name>: <the OS error>`, against the member and against
/// the file being edited, and no checker diagnostic over the partial
/// seed (outside review of #1282, finding 2).
#[cfg(unix)]
#[test]
fn lsp_reports_an_unreadable_seed_member_as_check_does() {
    const MAIN: &str = "fn main() {\n    let y: Int = \"not an int\";\n    println(y);\n}\n";
    let root = scratch_root("unreadable");
    let dir = root.join("seed");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let dir = dir.canonicalize().expect("canonical dir");
    std::fs::write(dir.join("main.hl"), MAIN).expect("write main");
    std::os::unix::fs::symlink("absent.hl", dir.join("missing.hl")).expect("symlink");

    let (check, status) = check_json(&dir);
    assert!(!status, "hale check must refuse the load: {check:?}");
    let [(file, _, _, os_error)] = check.as_slice() else {
        panic!("hale check reports the unreadable member alone: {check:?}");
    };
    assert_eq!(file, "missing.hl", "{check:?}");
    let want = format!("seed member missing.hl: {os_error}");

    let mut lsp = LspSession::start();
    lsp.lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "method": "textDocument/didSave",
        "params": { "textDocument": { "uri": uri(&dir.join("main.hl")) } }
    }));
    let disk = lsp.published();
    lsp.lsp.send(open(&dir.join("main.hl"), MAIN));
    let buffered = lsp.published();
    lsp.close();
    let _ = std::fs::remove_dir_all(&root);

    let expected: Vec<Finding> = vec![
        ("main.hl".to_string(), 1, 1, want.clone()),
        ("missing.hl".to_string(), 1, 1, want.clone()),
    ];
    assert_eq!(disk, expected, "the LSP on disk: the member's error, and no checker finding");
    assert_eq!(buffered, expected, "the LSP with a buffer: the member's error, and no checker finding");
}

/// A library that leaves the seed's import graph is published EMPTY
/// (outside review of #1286, finding 1). The import-failure pass
/// publishes the library's parse error against the library; once the
/// app stops importing it, the next pass has no reason of its own to
/// name the library, and a client keeps a URI's diagnostics until the
/// URI is published again — so the server clears it from what it sent
/// before.
#[test]
fn lsp_clears_a_library_the_seed_stops_importing() {
    let root = scratch_root("unimported");
    std::fs::create_dir_all(root.join("app").join("lib")).expect("mkdir");
    let dir = root.join("app").canonicalize().expect("canonical dir");
    let (main, lib) = (dir.join("main.hl"), dir.join("lib").join("lib.hl"));
    std::fs::write(&lib, "fn helper( {\n").expect("write lib");

    let mut lsp = LspSession::start();
    lsp.lsp.send(open(&main, "import \"lib\" as lib;\n\nfn main() { }\n"));
    let importing = lsp.publications();
    lsp.lsp.send(change(&main, 2, "fn main() { }\n"));
    let unimported = lsp.publications();
    lsp.close();
    let _ = std::fs::remove_dir_all(&root);

    let [(lib_uri, lib_msgs), (main_uri, main_msgs)] = importing.as_slice() else {
        panic!("the importing pass publishes the library and the app: {importing:?}");
    };
    assert_eq!((lib_uri, main_uri), (&uri(&lib), &uri(&main)), "{importing:?}");
    assert!(
        lib_msgs.len() == 1 && lib_msgs[0].contains("expected parameter name"),
        "the library's parse error, against the library: {importing:?}"
    );
    assert!(main_msgs.is_empty(), "{importing:?}");
    assert_eq!(
        unimported,
        vec![(uri(&lib), vec![]), (uri(&main), vec![])],
        "the pass after the import is removed clears the library"
    );
}

/// The app of the dependent-recheck tests: its one finding comes from
/// the imported `helper`'s result type.
const DEPENDENT_APP: &str =
    "import \"lib\" as lib;\n\nfn main() {\n    let x: String = lib::helper(1);\n    println(x);\n}\n";
const HELPER_INT: &str = "fn helper(n: Int) -> Int {\n    return n + 1;\n}\n";
const HELPER_STRING: &str = "fn helper(n: Int) -> String {\n    return \"ok\";\n}\n";
const MISMATCH: &str = "let `x`: expected `String`, got `Int`";

/// An app and the library it imports, both open, the library's disk
/// copy `lib_on_disk`; the sequences of the two opens are asserted
/// here. Each event checks its file's seed, then every other open seed.
fn open_app_and_library(tag: &str, lib_on_disk: &str) -> (LspSession, std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
    let root = scratch_root(tag);
    std::fs::create_dir_all(root.join("app").join("lib")).expect("mkdir");
    let dir = root.join("app").canonicalize().expect("canonical dir");
    let (main, lib) = (dir.join("main.hl"), dir.join("lib").join("lib.hl"));
    std::fs::write(&lib, lib_on_disk).expect("write lib");

    let mut lsp = LspSession::start();
    lsp.lsp.send(open(&lib, HELPER_INT));
    assert_eq!(lsp.publications(), vec![(uri(&lib), vec![])], "the library alone");
    lsp.lsp.send(open(&main, DEPENDENT_APP));
    assert_eq!(
        lsp.publications(),
        vec![(uri(&main), vec![MISMATCH.to_string()]), (uri(&lib), vec![])],
        "the app's seed, then the library's, the other open seed"
    );
    (lsp, root, main, lib)
}

/// An edit to an imported library's buffer rechecks the open app that
/// imports it (outside review of #1286, finding 2): the snapshot reads
/// the library from the buffer, so the edit changes the app's program.
/// The library's seed publishes first, then the app's.
#[test]
fn lsp_rechecks_an_open_dependent_when_a_library_buffer_changes() {
    let (mut lsp, root, main, lib) = open_app_and_library("dependent_change", HELPER_INT);
    lsp.lsp.send(change(&lib, 2, HELPER_STRING));
    let cleared = lsp.publications();
    lsp.lsp.send(change(&lib, 3, HELPER_INT));
    let reintroduced = lsp.publications();
    lsp.close();
    let _ = std::fs::remove_dir_all(&root);

    assert_eq!(
        cleared,
        vec![(uri(&lib), vec![]), (uri(&main), vec![])],
        "a library edit that clears the app's error publishes the app empty"
    );
    assert_eq!(
        reintroduced,
        vec![(uri(&lib), vec![]), (uri(&main), vec![MISMATCH.to_string()])],
        "a library edit that introduces the error publishes it on the app"
    );
}

/// Closing an imported library's buffer rechecks the open app against
/// the library's disk copy (outside review of #1286, finding 2). The
/// disk copy differs from the buffer — it returns the `String` the app
/// wants — so the recheck is visible: the app's error clears.
#[test]
fn lsp_rechecks_an_open_dependent_when_a_library_buffer_closes() {
    let (mut lsp, root, main, lib) = open_app_and_library("dependent_close", HELPER_STRING);
    lsp.lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "method": "textDocument/didClose",
        "params": { "textDocument": { "uri": uri(&lib) } }
    }));
    let closed = lsp.publications();
    lsp.close();
    let _ = std::fs::remove_dir_all(&root);

    assert_eq!(
        closed,
        vec![(uri(&lib), vec![]), (uri(&main), vec![])],
        "the library's seed from disk, then the app against the disk copy"
    );
}

/// A burst of document events costs one check, not one per event
/// (F.40 phase 2.4): five changes in one write, and the last publish
/// describes the LAST text, every publish is for the seed's file, and
/// the fence behind the burst is answered after that publish. The
/// NUMBER of passes is not asserted here: the reader thread frames the
/// five messages one at a time, and a scheduler may let the main loop
/// check between any two of them, so five passes are a permitted
/// interleaving (outside review of #1291, finding 2); the collapse rule
/// itself is pinned by the deterministic unit test
/// `a_run_of_document_events_costs_one_pass_and_stops_at_a_request`.
#[test]
fn lsp_a_burst_of_changes_is_checked_once() {
    let root = scratch_root("burst");
    let main = root.canonicalize().expect("canonical dir").join("main.hl");
    // Text `v` declares `v` lets; the fifth is a type error, so only
    // the last text of the burst can raise it.
    let text = |v: usize| {
        let mut t = String::from("fn main() {\n");
        for i in 1..=v {
            let value = if i == 5 { "\"five\"".to_string() } else { i.to_string() };
            t.push_str(&format!("    let x{i}: Int = {value};\n"));
        }
        t.push_str("}\n");
        t
    };

    let mut lsp = LspSession::start();
    lsp.lsp.send(open(&main, &text(0)));
    assert_eq!(lsp.publications(), vec![(uri(&main), vec![])], "the clean open");
    lsp.lsp.send_all((1..=5).map(|v| change(&main, 1 + v as u64, &text(v))).collect());
    let burst = lsp.publications();
    lsp.lsp.send(change(&main, 7, &text(4)));
    let fixed = lsp.publications();
    lsp.close();
    let _ = std::fs::remove_dir_all(&root);

    assert!(burst.iter().all(|(u, _)| *u == uri(&main)), "only the seed's file is published: {burst:?}");
    // How many passes the burst costs depends on how far the reader
    // thread has framed it when the loop wakes; fewer than five is what
    // collapsing guarantees. The exact collapse is the unit test's
    // (`a_run_of_document_events_costs_one_pass_and_stops_at_a_request`).
    let (_, last) = burst.last().expect("a publish");
    assert!(
        last.len() == 1 && last[0].contains("expected `Int`"),
        "the last publish carries the last text's error: {burst:?}"
    );
    assert_eq!(fixed, vec![(uri(&main), vec![])], "the change that fixes it clears it");
}

/// F.40 phase 2 review F4: a frame whose `Content-Length` no buffer
/// should hold is refused, not allocated, and the session ends with a
/// non-zero status, the message before it answered. The reader thread
/// used to panic on the allocation, and the server exited 0 as if the
/// client had gone away.
#[test]
fn lsp_refuses_an_absurd_content_length_and_exits_non_zero() {
    let mut lsp = Lsp::start();
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "capabilities": {} }
    }));
    assert!(lsp.recv()["result"]["capabilities"].is_object(), "the message before it is answered");
    lsp.stdin.write_all(b"Content-Length: 18446744073709551615\r\n\r\n").expect("write");
    lsp.stdin.flush().expect("flush");
    let status = lsp.child.wait().expect("wait");
    assert!(!status.success(), "a refused frame ends the session non-zero: {status:?}");
}

/// A scratch root of this test's own, empty.
fn scratch_root(tag: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("hale_lsp_parity_{}_{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("mkdir");
    root
}

/// `hale check --json <dir>`'s records as findings, sorted, and whether
/// it passed.
fn check_json(dir: &std::path::Path) -> (Vec<Finding>, bool) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["check", "--json"])
        .arg(dir)
        .output()
        .expect("run hale check");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut check: Vec<Finding> = stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let r: serde_json::Value = serde_json::from_str(l)
                .unwrap_or_else(|e| panic!("an NDJSON record ({e}): {l}"));
            let file = std::path::Path::new(r["file"].as_str().unwrap_or(""))
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            (
                file,
                r["line"].as_u64().unwrap_or(0),
                r["col"].as_u64().unwrap_or(0),
                r["message"].as_str().unwrap_or("").to_string(),
            )
        })
        .collect();
    check.sort();
    (check, out.status.success())
}

/// One seed, checked three ways, one answer; `hale check`'s findings
/// are handed back for the caller's own assertions.
///
/// 1. `hale check --json` on the directory.
/// 2. The LSP with the files on disk and no buffer open: a `didSave`
///    without text checks from the disk alone.
/// 3. The LSP with every one of `app`'s files supplied as a buffer,
///    twice: over a directory with none of them (the files exist only
///    as buffers, so the seed is whatever the server's source provider
///    lists) and over STALE disk copies (every buffer must win over its
///    file).
///
/// `lib` is a library the app imports as `"lib"`: its files are on disk
/// in `lib/` beside every one of the three copies, and never a buffer.
fn agree_three_ways(tag: &str, app: &[(&str, &str)], lib: &[(&str, &str)]) -> Vec<Finding> {
    const STALE: &str = "fn main() { }\n";
    let root = scratch_root(tag);
    let mkdir = |name: &str| {
        let d = root.join(name);
        std::fs::create_dir_all(d.join("lib")).expect("mkdir");
        for (f, text) in lib {
            std::fs::write(d.join("lib").join(f), text).expect("write lib");
        }
        d.canonicalize().expect("canonical dir")
    };
    let on_disk = mkdir("disk");
    for (f, text) in app {
        std::fs::write(on_disk.join(f), text).expect("write app");
    }
    let empty = mkdir("empty");
    let stale = mkdir("stale");
    for (f, _) in app {
        std::fs::write(stale.join(f), STALE).expect("write stale");
    }

    // 1. `hale check --json`.
    let (check, _) = check_json(&on_disk);

    // 2. The LSP on disk: no buffer, a save without text.
    let mut lsp = LspSession::start();
    let (last, _) = app.last().expect("an app file");
    lsp.lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "method": "textDocument/didSave",
        "params": { "textDocument": { "uri": uri(&on_disk.join(last)) } }
    }));
    let disk = lsp.published();

    // 3. The LSP with every file as a buffer, over an empty directory
    // and over stale copies. Each open checks the seed with the buffers
    // opened so far; the last has them all, and that check is the
    // answer (the opens over the stale copies recheck the empty
    // directory's seed too, still open, so the answer is the files
    // under `dir`).
    let buffered = |lsp: &mut LspSession, dir: &std::path::Path| -> Vec<Finding> {
        let mut answer = Vec::new();
        for (f, text) in app {
            lsp.lsp.send(open(&dir.join(f), text));
            answer = lsp.published_under(dir);
        }
        answer
    };
    let over_empty = buffered(&mut lsp, &empty);
    let over_stale = buffered(&mut lsp, &stale);
    lsp.close();
    let _ = std::fs::remove_dir_all(&root);

    assert_eq!(disk, check, "the LSP on disk and `hale check` disagree");
    assert_eq!(over_empty, check, "the LSP with buffers over an empty directory and `hale check` disagree");
    assert_eq!(over_stale, check, "the LSP with buffers over stale files and `hale check` disagree");
    check
}

/// An initialized `hale lsp`, and the fence ids its reads use.
struct LspSession {
    lsp: Lsp,
    fence: u64,
}

impl LspSession {
    fn start() -> Self {
        let mut lsp = Lsp::start();
        lsp.send(serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": { "capabilities": {} }
        }));
        let _ = lsp.recv();
        lsp.send(serde_json::json!({
            "jsonrpc": "2.0", "method": "initialized", "params": {}
        }));
        LspSession { lsp, fence: 10 }
    }

    /// Every publish of the check the last notification started.
    fn published(&mut self) -> Vec<Finding> {
        self.fence += 1;
        published(&mut self.lsp, self.fence)
    }

    /// Every publish of the check the last notification started on a
    /// file under `dir`: an event also rechecks every other open seed,
    /// whose files are not this read's answer.
    fn published_under(&mut self, dir: &std::path::Path) -> Vec<Finding> {
        self.fence += 1;
        let prefix = format!("{}/", uri(dir));
        let pubs = publications(&mut self.lsp, self.fence);
        findings(pubs.into_iter().filter(|(u, _)| u.starts_with(&prefix)).collect())
    }

    /// Every publish of the check the last notification started, in
    /// the order sent, as URI and messages.
    fn publications(&mut self) -> Vec<(String, Vec<String>)> {
        self.fence += 1;
        messages(publications(&mut self.lsp, self.fence))
    }

    /// A request's answer: the `result` of the reply to it.
    fn request(&mut self, method: &str, params: serde_json::Value) -> serde_json::Value {
        self.fence += 1;
        let id = self.fence;
        self.lsp.send(serde_json::json!({
            "jsonrpc": "2.0", "id": id, "method": method, "params": params
        }));
        loop {
            let msg = self.lsp.recv();
            if msg.get("id").and_then(|i| i.as_u64()) == Some(id) {
                return msg.get("result").cloned().unwrap_or(serde_json::Value::Null);
            }
        }
    }

    fn close(mut self) {
        self.request("shutdown", serde_json::Value::Null);
        self.lsp.send(serde_json::json!({
            "jsonrpc": "2.0", "method": "exit", "params": null
        }));
        let _ = self.lsp.child.wait();
    }
}

fn uri(path: &std::path::Path) -> String {
    format!("file://{}", path.display())
}

fn open(path: &std::path::Path, text: &str) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": { "textDocument": {
            "uri": uri(path), "languageId": "hale", "version": 1, "text": text
        }}
    })
}

fn change(path: &std::path::Path, version: u64, text: &str) -> serde_json::Value {
    serde_json::json!({
        "jsonrpc": "2.0", "method": "textDocument/didChange",
        "params": {
            "textDocument": { "uri": uri(path), "version": version },
            "contentChanges": [{ "text": text }]
        }
    })
}

/// Every publish of the check the last notification started, as
/// findings, in 1-based lines and columns (the server's are 0-based,
/// in UTF-16 units; the fixture is ASCII, so a unit is a character).
///
/// The server handles messages in order, so a request sent now is
/// answered after every publish of that check: reading up to the
/// answer collects the whole check however many files it covered —
/// a file the server failed to load is missing from the result, not a
/// read that never returns.
fn published(lsp: &mut Lsp, fence: u64) -> Vec<Finding> {
    findings(publications(lsp, fence))
}

/// Publications as findings, sorted: each file's LAST publication, the
/// one the client keeps. A seed with a broken law is published twice
/// (F.40 phase 3, X1), the typing stage and then the whole check, which
/// replaces the first per file; the findings are the second's.
fn findings(pubs: Vec<(String, Vec<serde_json::Value>)>) -> Vec<Finding> {
    let last: std::collections::BTreeMap<String, Vec<serde_json::Value>> = pubs.into_iter().collect();
    let mut out = Vec::new();
    for (uri, diags) in last {
        let file = uri.rsplit('/').next().unwrap_or("").to_string();
        for d in diags {
            out.push((
                file.clone(),
                d["range"]["start"]["line"].as_u64().unwrap_or(u64::MAX) + 1,
                d["range"]["start"]["character"].as_u64().unwrap_or(u64::MAX) + 1,
                d["message"].as_str().unwrap_or("").to_string(),
            ));
        }
    }
    out.sort();
    out
}

/// Every `publishDiagnostics` of the check the last notification
/// started, in the order the server sent them: each URI with its list,
/// read up to the fence as `published` reads.
fn publications(lsp: &mut Lsp, fence: u64) -> Vec<(String, Vec<serde_json::Value>)> {
    lsp.send(serde_json::json!({
        "jsonrpc": "2.0", "id": fence, "method": "hale/testFence", "params": null
    }));
    let mut out = Vec::new();
    loop {
        let msg = lsp.recv();
        if msg.get("id").and_then(|i| i.as_u64()) == Some(fence) {
            return out;
        }
        if msg.get("method").and_then(|m| m.as_str()) != Some("textDocument/publishDiagnostics") {
            continue;
        }
        let uri = msg.pointer("/params/uri").and_then(|u| u.as_str()).unwrap_or("").to_string();
        let diags = msg.pointer("/params/diagnostics").and_then(|d| d.as_array()).cloned().unwrap_or_default();
        out.push((uri, diags));
    }
}

/// A sequence of publications as URI and messages, the shape the
/// sequence tests compare.
fn messages(pubs: Vec<(String, Vec<serde_json::Value>)>) -> Vec<(String, Vec<String>)> {
    pubs.into_iter()
        .map(|(uri, diags)| {
            let msgs = diags.iter().map(|d| d["message"].as_str().unwrap_or("").to_string()).collect();
            (uri, msgs)
        })
        .collect()
}

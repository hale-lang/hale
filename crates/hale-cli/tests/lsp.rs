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
        let body = v.to_string();
        write!(self.stdin, "Content-Length: {}\r\n\r\n{}", body.len(), body)
            .expect("write");
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

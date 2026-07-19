//! `hale mcp` — the in-binary Model Context Protocol server
//! (stdio, newline-delimited JSON-RPC). Toolchain tools self-exec
//! this very binary (version-locked by construction); analysis
//! tools call hale-lsp directly; docs search greps the spec
//! embedded at build time.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

struct Mcp {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Mcp {
    fn start() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_hale"))
            .arg("mcp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn hale mcp");
        let stdin = child.stdin.take().expect("stdin");
        let stdout = BufReader::new(child.stdout.take().expect("stdout"));
        Mcp { child, stdin, stdout }
    }

    fn send(&mut self, v: serde_json::Value) {
        writeln!(self.stdin, "{}", v).expect("write");
        self.stdin.flush().expect("flush");
    }

    fn recv(&mut self) -> serde_json::Value {
        let mut line = String::new();
        self.stdout.read_line(&mut line).expect("read");
        serde_json::from_str(&line).expect("json")
    }
}

#[test]
fn handshake_tools_and_calls() {
    let dir = std::env::temp_dir().join(format!(
        "hale_mcp_test_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let good = dir.join("good.hl");
    std::fs::write(&good, "fn main() { println(\"hi\"); }\n")
        .expect("write");
    let bad = dir.join("sub");
    std::fs::create_dir_all(&bad).expect("mkdir sub");
    let bad = bad.join("bad.hl");
    std::fs::write(&bad, "fn main() { let x: Int = \"s\"; }\n")
        .expect("write bad");
    // A seed with a bus topology for the analysis tools.
    let busd = dir.join("busapp");
    std::fs::create_dir_all(&busd).expect("mkdir busapp");
    std::fs::write(
        busd.join("main.hl"),
        r#"type Msg { text: String; }
topic Posted { payload: Msg; subject: "posted"; }
locus Sink {
    bus { subscribe Posted as on_post; }
    fn on_post(m: Msg) { println(m.text); }
}
main locus App {
    params { s: Sink = Sink { }; }
    bus { publish Posted; }
    run() { Posted <- Msg { text: "t" }; }
}
fn main() { App { }; }
"#,
    )
    .expect("write busapp");

    let mut mcp = Mcp::start();

    // initialize → serverInfo is this binary.
    mcp.send(serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "protocolVersion": "2024-11-05" }
    }));
    let init = mcp.recv();
    assert_eq!(init.pointer("/result/serverInfo/name"),
        Some(&serde_json::json!("hale")));
    assert_eq!(
        init.pointer("/result/protocolVersion"),
        Some(&serde_json::json!("2024-11-05"))
    );
    // Notification: consumed silently (no response).
    mcp.send(serde_json::json!({
        "jsonrpc": "2.0", "method": "notifications/initialized"
    }));

    // tools/list — the full surface.
    mcp.send(serde_json::json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/list"
    }));
    let tools = mcp.recv();
    let names: Vec<&str> = tools
        .pointer("/result/tools")
        .and_then(|v| v.as_array())
        .expect("tools")
        .iter()
        .filter_map(|t| t.pointer("/name").and_then(|n| n.as_str()))
        .collect();
    for expect in [
        "hale_check", "hale_verify", "hale_fmt", "hale_test",
        "hale_bench", "hale_doc", "hale_docs_search",
        "hale_bus_graph", "hale_enforcement",
    ] {
        assert!(names.contains(&expect), "missing tool {}", expect);
    }

    // hale_check: clean file ok, broken file isError with the
    // diagnostic text.
    mcp.send(serde_json::json!({
        "jsonrpc": "2.0", "id": 3, "method": "tools/call",
        "params": { "name": "hale_check",
                    "arguments": { "path": good.display().to_string() } }
    }));
    let r = mcp.recv();
    assert_eq!(r.pointer("/result/isError"), Some(&serde_json::json!(false)));
    mcp.send(serde_json::json!({
        "jsonrpc": "2.0", "id": 4, "method": "tools/call",
        "params": { "name": "hale_check",
                    "arguments": { "path": bad.display().to_string() } }
    }));
    let r = mcp.recv();
    assert_eq!(r.pointer("/result/isError"), Some(&serde_json::json!(true)));
    assert!(
        r.pointer("/result/content/0/text")
            .and_then(|t| t.as_str())
            .is_some_and(|t| t.contains("Int")),
        "diagnostic text missing"
    );

    // hale_docs_search: embedded spec answers with file:line hits.
    mcp.send(serde_json::json!({
        "jsonrpc": "2.0", "id": 5, "method": "tools/call",
        "params": { "name": "hale_docs_search",
                    "arguments": { "query": "keyed_by", "max_results": 3 } }
    }));
    let r = mcp.recv();
    let text = r
        .pointer("/result/content/0/text")
        .and_then(|t| t.as_str())
        .expect("text");
    assert!(text.contains(".md:"), "no file:line hits:\n{}", text);

    // hale_bus_graph: library-call analysis over the bus seed.
    mcp.send(serde_json::json!({
        "jsonrpc": "2.0", "id": 6, "method": "tools/call",
        "params": { "name": "hale_bus_graph",
                    "arguments": { "path":
                        busd.join("main.hl").display().to_string() } }
    }));
    let r = mcp.recv();
    let text = r
        .pointer("/result/content/0/text")
        .and_then(|t| t.as_str())
        .expect("text");
    assert!(text.contains("Posted"), "bus graph missing subject:\n{}", text);
    assert!(text.contains("Sink"), "bus graph missing subscriber:\n{}", text);

    // Sandbox: HALE_MCP_ROOT rejection is covered by a separate
    // server instance below.
    drop(mcp.stdin);
    let _ = mcp.child.wait();

    // Sandboxed instance: a path outside the root is rejected.
    let mut child = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("mcp")
        .env("HALE_MCP_ROOT", busd.display().to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn sandboxed");
    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = BufReader::new(child.stdout.take().expect("stdout"));
    writeln!(
        stdin,
        "{}",
        serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": { "name": "hale_check",
                        "arguments": { "path": good.display().to_string() } }
        })
    )
    .expect("write");
    stdin.flush().expect("flush");
    let mut line = String::new();
    stdout.read_line(&mut line).expect("read");
    let r: serde_json::Value = serde_json::from_str(&line).expect("json");
    assert_eq!(r.pointer("/result/isError"), Some(&serde_json::json!(true)));
    assert!(
        r.pointer("/result/content/0/text")
            .and_then(|t| t.as_str())
            .is_some_and(|t| t.contains("HALE_MCP_ROOT")),
        "sandbox rejection missing"
    );
    drop(stdin);
    let _ = child.wait();

    let _ = std::fs::remove_dir_all(&dir);
}

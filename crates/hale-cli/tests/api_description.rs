//! GH #1107: the api binding's description, the model's first wire
//! form (spec/model.md § "The description: the model's first wire
//! form").
//!
//! The fixture program under `fixtures/api_description/` describes
//! itself in the same bytes every time, in its native form and in
//! the OpenAPI 3.1 and MCP forms derived from it; the committed files
//! beside it are the baseline. A running binding serves the same
//! bytes `hale check --dump-api` emits, and `hale mcp --app` lists
//! exactly the fixture's commands as tools. Regenerate deliberately:
//!
//!     HALE_REGEN_API_DESCRIPTION=1 cargo test -p hale-cli --test api_description

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use serde_json::Value;

fn hale() -> Command {
    Command::new(env!("CARGO_BIN_EXE_hale"))
}

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/api_description")
}

fn describe(form: &[&str]) -> String {
    let out = hale()
        .arg("describe")
        .arg(fixture_dir().join("main.hl"))
        .args(form)
        .output()
        .expect("hale describe");
    assert!(
        out.status.success(),
        "hale describe {:?} failed: {}",
        form,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("utf8")
}

fn check_baseline(name: &str, form: &[&str]) {
    let got = describe(form);
    let path = fixture_dir().join(name);
    if std::env::var("HALE_REGEN_API_DESCRIPTION").is_ok() {
        std::fs::write(&path, &got).expect("write baseline");
        return;
    }
    let want = std::fs::read_to_string(&path).expect("baseline present");
    assert!(
        got == want,
        "{} moved from its committed baseline. If that is intended, regenerate with \
         HALE_REGEN_API_DESCRIPTION=1 cargo test -p hale-cli --test api_description\n--- got\n{}",
        name,
        got
    );
    // Byte-reproducible: a second run is the same bytes.
    assert_eq!(describe(form), got, "{} is not reproducible", name);
}

#[test]
fn the_native_description_matches_its_baseline() {
    check_baseline("description.json", &[]);
    let d: Value = serde_json::from_str(&describe(&[])).expect("json");
    let names = |t: &str| -> Vec<String> {
        d[t].as_array()
            .unwrap()
            .iter()
            .map(|x| x["name"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(names("commands"), ["Audits", "Refunds", "Ticks"]);
    assert_eq!(names("reads"), ["billing.ledger", "uptime"]);
    assert_eq!(names("streams"), ["Moved"]);
    assert_eq!(d["commands"][1]["reply"], "RefundResult");
    assert_eq!(d["commands"][0]["reply"], Value::Null);
    assert_eq!(d["commands"][2]["keyed_by"], "order_id");
    assert_eq!(d["schemas"]["Refund"]["properties"]["money"]["$ref"], "#/schemas/Money");
    assert_eq!(d["schemas"]["Money"]["required"], serde_json::json!(["amount"]));
    assert!(d["notes"]["gates"].as_str().unwrap().contains("boundary check"));
    assert!(d["notes"]["reads"].as_str().unwrap().contains("snapshot"));
}

#[test]
fn the_openapi_form_matches_its_baseline_and_validates() {
    check_baseline("openapi.json", &["--openapi"]);
    let d: Value = serde_json::from_str(&describe(&["--openapi"])).expect("json");
    assert_eq!(d["openapi"], "3.1.0");
    let paths = d["paths"].as_object().unwrap();
    for c in ["Audits", "Refunds", "Ticks"] {
        let op = &paths[&format!("/call/{}", c)]["post"];
        assert_eq!(op["operationId"], format!("call.{}", c));
        assert!(op["requestBody"]["content"]["application/json"]["schema"]["$ref"].is_string());
        assert!(op["responses"]["200"].is_object() && op["responses"]["4XX"].is_object());
    }
    for r in ["billing.ledger", "uptime"] {
        assert_eq!(paths[&format!("/read/{}", r)]["get"]["operationId"], format!("read.{}", r));
    }
    assert_eq!(paths["/watch/Moved"]["get"]["operationId"], "watch.Moved");
    // Every $ref resolves under components.schemas.
    let schemas = d["components"]["schemas"].as_object().unwrap();
    fn refs(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::Object(m) => {
                if let Some(s) = m.get("$ref").and_then(Value::as_str) {
                    out.push(s.to_string());
                }
                m.values().for_each(|x| refs(x, out));
            }
            Value::Array(a) => a.iter().for_each(|x| refs(x, out)),
            _ => {}
        }
    }
    let mut all = Vec::new();
    refs(&d, &mut all);
    assert!(!all.is_empty());
    for r in all {
        let name = r.strip_prefix("#/components/schemas/").expect(&r);
        assert!(schemas.contains_key(name), "unresolved {}", r);
    }
    assert!(schemas.contains_key("Refusal") && schemas.contains_key("Accepted"));
    assert!(d["info"]["description"].as_str().unwrap().contains("boundary check"));
}

#[test]
fn the_mcp_form_matches_its_baseline() {
    check_baseline("mcp.json", &["--mcp"]);
    let d: Value = serde_json::from_str(&describe(&["--mcp"])).expect("json");
    let tools: Vec<&str> = d["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(tools, ["Audits", "Refunds", "Ticks"]);
    // A tool's input schema is self-contained: the nested type rides in $defs.
    let refund = &d["tools"][1]["inputSchema"];
    assert_eq!(refund["properties"]["money"]["$ref"], "#/$defs/Money");
    assert!(refund["$defs"]["Money"].is_object());
    let uris: Vec<&str> = d["resources"].as_array().unwrap().iter().map(|r| r["uri"].as_str().unwrap()).collect();
    assert_eq!(uris, ["hale://read/billing.ledger", "hale://read/uptime"]);
    assert_eq!(d["streams"], serde_json::json!(["Moved"]));
}

#[test]
fn a_program_without_the_entry_describes_nothing() {
    let dir = std::env::temp_dir().join(format!("hale_api_desc_none_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("plain.hl");
    std::fs::write(&f, "fn main() { println(\"hi\"); }\n").unwrap();
    let out = hale().arg("describe").arg(&f).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("no `api:` entry"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The fixture, built and running on a socket of this test's own
/// (`LOTUS_API` overrides the path the source names).
struct Running {
    child: Child,
    sock: PathBuf,
    bin: PathBuf,
}

impl Running {
    fn start(tag: &str) -> Running {
        // `hale build` lands the binary beside its source, so build a
        // copy of the fixture under a path of this test's own.
        let dir = std::env::temp_dir();
        let src = dir.join(format!("hale_api_desc_{}_{}.hl", tag, std::process::id()));
        std::fs::copy(fixture_dir().join("main.hl"), &src).expect("copy fixture");
        let out = hale().arg("build").arg(&src).output().expect("hale build");
        assert!(out.status.success(), "build: {}", String::from_utf8_lossy(&out.stderr));
        let _ = std::fs::remove_file(&src);
        let bin = src.with_extension("");
        let sock = dir.join(format!("hale_api_desc_{}_{}.sock", tag, std::process::id()));
        let _ = std::fs::remove_file(&sock);
        let child = Command::new(&bin)
            .env("LOTUS_API", &sock)
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("run fixture");
        // Wait, bounded, for the socket to appear.
        for _ in 0..200 {
            if sock.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        assert!(sock.exists(), "the binding never bound {}", sock.display());
        Running { child, sock, bin }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.sock);
        let _ = std::fs::remove_file(&self.bin);
    }
}

#[test]
fn a_running_binding_serves_the_emitted_description() {
    let app = Running::start("serve");
    let out = hale().arg("describe").arg(&app.sock).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let served: Value = serde_json::from_slice(&out.stdout).unwrap();
    let emitted: Value = serde_json::from_str(&describe(&[])).unwrap();
    assert_eq!(served, emitted, "the binding serves the bytes the compiler emits");
    // And hale call round-trips a command through it.
    let out = hale()
        .arg("call")
        .arg(&app.sock)
        .arg("Refunds")
        .arg(r#"{"order_id": 4, "money": {"amount": 5}}"#)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["note"], "refunded 4");
}

struct Mcp {
    child: Child,
    stdin: std::process::ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
}

impl Mcp {
    fn start(sock: &Path) -> Mcp {
        let mut child = hale()
            .args(["mcp", "--app"])
            .arg(sock)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("hale mcp --app");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Mcp { child, stdin, stdout }
    }
    fn call(&mut self, v: Value) -> Value {
        writeln!(self.stdin, "{}", v).unwrap();
        self.stdin.flush().unwrap();
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        serde_json::from_str(&line).expect("json-rpc line")
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn mcp_app_lists_the_commands_as_tools_and_calls_through() {
    let app = Running::start("mcp");
    let mut m = Mcp::start(&app.sock);
    let init = m.call(serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}));
    assert!(init["result"]["capabilities"]["tools"].is_object());
    assert!(init["result"]["instructions"].as_str().unwrap().contains("snapshot"));
    let tools = m.call(serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}));
    let names: Vec<&str> = tools["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Audits", "Refunds", "Ticks"], "exactly the subscribed topics");
    let called = m.call(serde_json::json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
        "params": {"name": "Refunds", "arguments": {"order_id": 11, "money": {"amount": 1}}}}));
    assert_eq!(called["result"]["isError"], false, "{}", called);
    let text: Value = serde_json::from_str(called["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(text["note"], "refunded 11");
    let refused = m.call(serde_json::json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call",
        "params": {"name": "Refunds", "arguments": {"order_id": "x", "money": {"amount": 1}}}}));
    assert_eq!(refused["result"]["isError"], true);
    assert!(refused["result"]["content"][0]["text"].as_str().unwrap().contains("malformed"));
    let res = m.call(serde_json::json!({"jsonrpc": "2.0", "id": 5, "method": "resources/list"}));
    let uris: Vec<&str> = res["result"]["resources"].as_array().unwrap().iter().map(|r| r["uri"].as_str().unwrap()).collect();
    assert_eq!(uris, ["hale://read/billing.ledger", "hale://read/uptime"]);
    let read = m.call(serde_json::json!({"jsonrpc": "2.0", "id": 6, "method": "resources/read", "params": {"uri": "hale://read/billing.ledger"}}));
    let body: Value = serde_json::from_str(read["result"]["contents"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(body["value"]["entries"], 1);
    assert!(body["as_of"].as_str().unwrap().starts_with("sha256:"));
}

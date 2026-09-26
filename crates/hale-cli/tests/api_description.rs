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
    assert_eq!(names("commands"), ["Audits", "Counts", "Refunds", "Ticks"]);
    // The native form keeps the binding's own key order, never a
    // sorted re-serialization.
    let raw = describe(&[]);
    assert!(raw.starts_with("{\"hale_api\":1,\"app\":\"Shop\",\"notes\":"), "{}", raw);
    assert_eq!(d["commands"][1]["reply"], "Int");
    assert_eq!(names("reads"), ["billing.ledger", "ticks", "uptime"]);
    assert_eq!(names("streams"), ["Moved"]);
    assert_eq!(d["commands"][2]["reply"], "RefundResult");
    assert_eq!(d["commands"][0]["reply"], Value::Null);
    assert_eq!(d["commands"][3]["keyed_by"], "order_id");
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
    // A scalar reply and a scalar read are inlined, never a $ref to a
    // component that does not exist.
    assert_eq!(paths["/call/Counts"]["post"]["responses"]["200"]["content"]["application/json"]["schema"], serde_json::json!({ "type": "integer" }));
    assert_eq!(paths["/read/ticks"]["get"]["responses"]["200"]["content"]["application/json"]["schema"]["properties"]["value"], serde_json::json!({ "type": "integer" }));
    assert!(d["components"]["securitySchemes"]["role"].is_object(), "the role scheme is declared");
    for c in ["Audits", "Counts", "Refunds", "Ticks"] {
        let op = &paths[&format!("/call/{}", c)]["post"];
        assert_eq!(op["operationId"], format!("call.{}", c));
        assert!(op["requestBody"]["content"]["application/json"]["schema"]["$ref"].is_string());
        assert!(op["responses"]["200"].is_object() && op["responses"]["4XX"].is_object());
    }
    for r in ["billing.ledger", "ticks", "uptime"] {
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
    assert_eq!(tools, ["Audits", "Counts", "Refunds", "Ticks"]);
    // A tool's input schema is self-contained: the nested type rides in $defs.
    let refund = &d["tools"][2]["inputSchema"];
    assert_eq!(refund["properties"]["money"]["$ref"], "#/$defs/Money");
    assert!(refund["$defs"]["Money"].is_object());
    let uris: Vec<&str> = d["resources"].as_array().unwrap().iter().map(|r| r["uri"].as_str().unwrap()).collect();
    assert_eq!(uris, ["hale://read/billing.ledger", "hale://read/ticks", "hale://read/uptime"]);
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
    let served = String::from_utf8(out.stdout).unwrap();
    assert_eq!(served, describe(&[]), "the binding serves the bytes the compiler emits, byte for byte");
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
    assert_eq!(names, ["Audits", "Counts", "Refunds", "Ticks"], "exactly the subscribed topics");
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
    assert_eq!(uris, ["hale://read/billing.ledger", "hale://read/ticks", "hale://read/uptime"]);
    let missing = m.call(serde_json::json!({"jsonrpc": "2.0", "id": 7, "method": "resources/read", "params": {"uri": "hale://read/nope"}}));
    assert_eq!(missing["error"]["code"], -32002, "{}", missing);
    let unknown = m.call(serde_json::json!({"jsonrpc": "2.0", "id": 8, "method": "prompts/list"}));
    assert_eq!(unknown["error"]["code"], -32601, "{}", unknown);
    let read = m.call(serde_json::json!({"jsonrpc": "2.0", "id": 6, "method": "resources/read", "params": {"uri": "hale://read/billing.ledger"}}));
    let body: Value = serde_json::from_str(read["result"]["contents"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(body["value"]["entries"], 1);
    assert!(body["as_of"].as_str().unwrap().starts_with("sha256:"));
}

// ---- hale admin: a page for 127.0.0.1 only ------------------------------------

/// `hale admin` on a port the OS picks, with the URL it printed.
struct Admin {
    child: Child,
    port: u16,
    /// From the URL the process printed: the only way in.
    token: String,
}

impl Admin {
    fn start(sock: &Path) -> Admin {
        let mut child = hale()
            .args(["admin"])
            .arg(sock)
            .args(["--port", "0"])
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("hale admin");
        let mut out = BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        out.read_line(&mut line).unwrap();
        let port: u16 = line
            .split("127.0.0.1:")
            .nth(1)
            .and_then(|s| s.split('/').next())
            .and_then(|p| p.parse().ok())
            .unwrap_or_else(|| panic!("no port in {:?}", line));
        let token = line
            .split("?token=")
            .nth(1)
            .and_then(|s| s.split_whitespace().next())
            .unwrap_or_else(|| panic!("no token in the printed URL {:?}", line))
            .to_string();
        Admin { child, port, token }
    }

    /// One raw HTTP exchange: status line, headers, body.
    fn http(&self, req: &str) -> (String, String) {
        use std::io::Read;
        let mut s = std::net::TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        s.write_all(req.as_bytes()).unwrap();
        let mut resp = String::new();
        s.read_to_string(&mut resp).unwrap();
        let (head, body) = resp.split_once("\r\n\r\n").unwrap_or((&resp, ""));
        (head.lines().next().unwrap_or("").to_string(), body.to_string())
    }
}

impl Drop for Admin {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn admin_refuses_other_origins_hosts_and_untokened_calls() {
    let app = Running::start("admin");
    let admin = Admin::start(&app.sock);
    let host = format!("127.0.0.1:{}", admin.port);
    // The page itself is behind the token: any local process can open
    // loopback TCP, so a bare GET / must not hand out the token.
    let (status, page) = admin.http(&format!("GET / HTTP/1.1\r\nHost: {}\r\n\r\n", host));
    assert!(status.contains("403"), "a bare GET / is refused: {}", status);
    assert!(!page.contains("const TOKEN"), "the refusal carries no token");
    let token = admin.token.clone();
    assert_eq!(token.len(), 32);
    let (status, page) = admin.http(&format!("GET /?token={} HTTP/1.1\r\nHost: {}\r\n\r\n", token, host));
    assert!(status.contains("200"), "{}", status);
    assert!(page.contains(&format!("const TOKEN = \"{}\"", token)) && !page.contains("{{SOCKET}}") && !page.contains("undefined"), "the tokened page names the socket and carries the token");

    // A foreign Origin, whatever the body: refused before anything runs.
    let body = r#"{"line": "from elsewhere"}"#;
    let (status, _) = admin.http(&format!(
        "POST /api/call/Audits HTTP/1.1\r\nHost: {}\r\nOrigin: http://evil.example\r\nContent-Type: text/plain\r\nX-Hale-Admin: {}\r\nContent-Length: {}\r\n\r\n{}",
        host, token, body.len(), body
    ));
    assert!(status.contains("403"), "foreign origin: {}", status);
    // A foreign Host (DNS rebinding): refused, reads included.
    let (status, _) = admin.http(&format!("GET /api/read/uptime HTTP/1.1\r\nHost: evil.example:{}\r\nX-Hale-Admin: {}\r\n\r\n", admin.port, token));
    assert!(status.contains("403"), "foreign host: {}", status);
    // No token: refused.
    let (status, _) = admin.http(&format!("GET /api/read/uptime HTTP/1.1\r\nHost: {}\r\n\r\n", host));
    assert!(status.contains("403"), "no token: {}", status);
    // The wrong content type, and a body that is not JSON, never dispatch.
    let (status, _) = admin.http(&format!(
        "POST /api/call/Audits HTTP/1.1\r\nHost: {}\r\nContent-Type: text/plain\r\nX-Hale-Admin: {}\r\nContent-Length: {}\r\n\r\n{}",
        host, token, body.len(), body
    ));
    assert!(status.contains("415"), "text/plain: {}", status);
    let bad = "not json";
    let (status, _) = admin.http(&format!(
        "POST /api/call/Audits HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nX-Hale-Admin: {}\r\nContent-Length: {}\r\n\r\n{}",
        host, token, bad.len(), bad
    ));
    assert!(status.contains("400"), "bad json: {}", status);
    // The page's own shape works: the same origin, the token, JSON.
    let (status, resp) = admin.http(&format!(
        "POST /api/call/Audits HTTP/1.1\r\nHost: {}\r\nOrigin: http://{}\r\nContent-Type: application/json\r\nX-Hale-Admin: {}\r\nContent-Length: {}\r\n\r\n{}",
        host, host, token, body.len(), body
    ));
    assert!(status.contains("200"), "{}", status);
    let v: Value = serde_json::from_str(&resp).unwrap();
    assert_eq!(v["ok"], true, "{}", resp);
    assert!(v["request_id"].is_number(), "the whole receipt comes back: {}", resp);
    // A refused attach closes with the receipt instead of streaming.
    let (status, resp) = admin.http(&format!("GET /api/watch/Audits?token={} HTTP/1.1\r\nHost: {}\r\n\r\n", token, host));
    assert!(status.contains("403"), "{}", status);
    assert!(resp.contains("not_a_stream"), "{}", resp);
}

#[test]
fn call_prints_the_whole_receipt_on_request() {
    let app = Running::start("receipt");
    let out = hale()
        .arg("call")
        .arg(&app.sock)
        .arg("Counts")
        .arg(r#"{"by": 2}"#)
        .arg("--receipt")
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["ok"], true);
    assert_eq!(v["value"], 2);
    assert!(v["request_id"].is_number());
    // A refusal prints the whole receipt on stderr too.
    let out = hale()
        .arg("call")
        .arg(&app.sock)
        .arg("Counts")
        .arg(r#"{"by": "x"}"#)
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("refused: malformed"), "{}", err);
    assert!(err.contains("\"request_id\":"), "{}", err);
}

// ---- a cross-seed topic's tool name ----------------------------------------

const IMPORTED_LIB: &str = r#"
type Order { id: Int; }
topic Orders { payload: Order; subject: "lib.order"; }
"#;

const IMPORTER: &str = r#"
import "lib" as lib;
type Ack { id: Int; }
locus Desk {
    bus { subscribe lib::Orders as on_order; }
    fn on_order(o: lib::Order) -> Ack { return Ack { id: o.id }; }
}
main locus App {
    params { desk: Desk = Desk { }; }
    bindings { api: unix("/tmp/hale-api-import-fixture.sock", bound: 4, on_full: refuse); }
    run() { std::time::sleep(20s); }
}
fn main() { App { }; }
"#;

#[test]
fn a_cross_seed_topic_is_a_tool_with_a_legal_name() {
    let dir = std::env::temp_dir().join(format!("hale_api_import_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    std::fs::write(dir.join("lib").join("main.hl"), IMPORTED_LIB).unwrap();
    std::fs::write(dir.join("main.hl"), IMPORTER).unwrap();
    let out = hale().arg("describe").arg(&dir).arg("--mcp").output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let d: Value = serde_json::from_slice(&out.stdout).unwrap();
    let tools: Vec<&str> = d["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(tools, ["lib__Orders"], "`::` is outside a tool name's characters");
    // And the mapped name calls the imported topic through a running seed.
    let out = hale().arg("build").arg(&dir).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let bin = dir.join(dir.file_name().unwrap());
    let sock = dir.join("api.sock");
    let mut child = Command::new(&bin)
        .env("LOTUS_API", &sock)
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    for _ in 0..200 {
        if sock.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    let mut m = Mcp::start(&sock);
    let called = m.call(serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {"name": "lib__Orders", "arguments": {"id": 5}}}));
    assert_eq!(called["result"]["isError"], false, "{}", called);
    assert!(called["result"]["content"][0]["text"].as_str().unwrap().contains("\"id\":5"));
    drop(m);
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&dir);
}

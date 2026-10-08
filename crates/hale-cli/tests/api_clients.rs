//! GH #1107, #1417 (R4 C): the generic clients read a served exposure's
//! description and speak the R0 wire.
//!
//! The fixture program under `fixtures/api_clients/` serves one surface,
//! `Books`, over a Unix socket, HTTP and MCP, and one stream through a hub.
//! These tests build it once, run it, and drive `hale describe`, `call`,
//! `watch`, `admin` and `mcp --app` against each endpoint, holding what they
//! print to what the exposure served: the description a client reads is the
//! exposure's own bytes, a call names the digest it read, a member the caller
//! may not call is not listed, and a stream is reached through the hub.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

fn hale() -> Command {
    Command::new(env!("CARGO_BIN_EXE_hale"))
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").expect("bind").local_addr().expect("addr").port()
}

fn uid() -> u32 {
    // SAFETY: getuid cannot fail.
    unsafe { libc::getuid() }
}

fn unique_dir(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!("hale_api_clients_{}_{}_{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed), tag));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

/// The fixture program's addresses.
#[derive(Clone)]
struct Addrs {
    sock: PathBuf,
    http: u16,
    mcp: u16,
    hub: u16,
}

/// The fixture built once per process: its binary and the addresses it was
/// built with (they are literals of the serve sites).
fn built() -> &'static (PathBuf, Addrs) {
    static BUILT: OnceLock<(PathBuf, Addrs)> = OnceLock::new();
    BUILT.get_or_init(|| {
        let dir = unique_dir("build");
        // a Unix socket path is at most ~100 bytes: keep it short
        let sock = std::env::temp_dir().join(format!("hc{}.sock", std::process::id()));
        let addrs = Addrs { sock, http: free_port(), mcp: free_port(), hub: free_port() };
        let src = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/api_clients/main.hl")).expect("fixture");
        let src = src
            .replace("@SOCK@", &addrs.sock.display().to_string())
            .replace("@HTTP@", &addrs.http.to_string())
            .replace("@MCP@", &addrs.mcp.to_string())
            .replace("@HUB@", &addrs.hub.to_string())
            .replace("@TRADER@", &format!("uid:{}", uid()));
        let file = dir.join("main.hl");
        std::fs::write(&file, src).expect("write fixture");
        let bin = dir.join("clients_fixture");
        let out = hale().arg("build").arg(&file).arg("-o").arg(&bin).env("HALE_SKIP_STALE_CHECK", "1").output().expect("hale build");
        assert!(out.status.success(), "build: {}", String::from_utf8_lossy(&out.stderr));
        (bin, addrs)
    })
}

/// The fixture, running. Dropping it kills it.
struct Running {
    child: Child,
    a: Addrs,
}

impl Running {
    fn start() -> Running {
        let (bin, a) = built().clone();
        let _ = std::fs::remove_file(&a.sock);
        let child = Command::new(&bin).stdout(Stdio::null()).stderr(Stdio::inherit()).spawn().expect("run the fixture");
        let run = Running { child, a: a.clone() };
        // the socket exists, and every listener accepts: bounded, not a sleep
        let start = Instant::now();
        let ready = || a.sock.exists() && [a.http, a.mcp, a.hub].iter().all(|p| TcpStream::connect(("127.0.0.1", *p)).is_ok());
        while !ready() {
            assert!(start.elapsed() < Duration::from_secs(30), "the fixture never began listening");
            std::thread::sleep(Duration::from_millis(20));
        }
        run
    }
    fn sock(&self) -> String {
        self.a.sock.display().to_string()
    }
    fn http(&self) -> String {
        format!("http://127.0.0.1:{}", self.a.http)
    }
    fn hub(&self) -> String {
        format!("ws://127.0.0.1:{}", self.a.hub)
    }
    fn mcp(&self) -> String {
        format!("mcp://127.0.0.1:{}", self.a.mcp)
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.a.sock);
    }
}

/// The fixture is one process on fixed addresses: a test that runs it holds
/// this for its whole life, and the others wait their turn.
fn exclusive() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

struct Out {
    ok: bool,
    stdout: String,
    stderr: String,
}

fn run(args: &[&str]) -> Out {
    let o = hale().args(args).env("HALE_API_TOKEN", "").output().expect("run hale");
    Out { ok: o.status.success(), stdout: String::from_utf8_lossy(&o.stdout).to_string(), stderr: String::from_utf8_lossy(&o.stderr).to_string() }
}

fn json_of(s: &str) -> Value {
    serde_json::from_str(s.trim()).unwrap_or_else(|e| panic!("{e}: {s}"))
}

fn member_names(doc: &Value) -> Vec<String> {
    doc["members"].as_array().expect("members").iter().map(|m| m["name"].as_str().unwrap().to_string()).collect()
}

/// One line out and one back over the socket, as a client of the wire.
fn unix_line(sock: &str, line: &str) -> String {
    let mut s = std::os::unix::net::UnixStream::connect(sock).expect("connect");
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    writeln!(s, "{line}").unwrap();
    let mut r = BufReader::new(s);
    let mut out = String::new();
    r.read_line(&mut out).unwrap();
    out.trim().to_string()
}

/// `GET /.description` as a raw HTTP client.
fn http_description(port: u16, token: &str) -> String {
    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    write!(s, "GET /.description HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n").unwrap();
    let mut raw = String::new();
    s.read_to_string(&mut raw).unwrap();
    raw.split_once("\r\n\r\n").expect("a response").1.to_string()
}

// ---- describe ---------------------------------------------------------------------------

#[test]
fn describe_prints_the_bytes_each_transport_serves() {
    let _g = exclusive();
    let app = Running::start();
    // over the socket: the reply's `value`, byte for byte
    let served = unix_line(&app.sock(), r#"{"describe":true}"#);
    let at = served.find("\"value\":").expect("a description") + 8;
    let want = &served[at..served.rfind(",\"caller\":").expect("the reply names its caller")];
    let got = run(&["describe", &app.sock()]);
    assert!(got.ok, "{}", got.stderr);
    // (the principal's pid is the connecting process's: each client is its own peer)
    let no_pid = |t: &str| -> String {
        let at = t.find("\"pid\":").expect("a pid") + 6;
        let end = at + t[at..].find(|c: char| !c.is_ascii_digit()).unwrap();
        format!("{}{}", &t[..at], &t[end..])
    };
    assert_eq!(no_pid(&got.stdout), format!("{}\n", no_pid(want)), "hale describe prints the bytes the socket serves");
    let doc = json_of(&got.stdout);
    assert_eq!(doc["description"], 1);
    assert_eq!(doc["listener"], json!({ "transport": "unix", "address": app.sock() }));
    assert_eq!(member_names(&doc), ["Counter::add", "Counter::fail", "Counter::peek"], "the test's own uid holds trader");
    assert_eq!(doc["caller"]["roles"], json!(["trader"]));
    // the same spelling with its scheme
    let got2 = run(&["describe", &format!("unix:{}", app.sock())]);
    assert_eq!(no_pid(&got2.stdout), no_pid(&got.stdout));

    // over HTTP: the body of GET /.description for the caller the token names
    for (token, roles) in [("t-alice", json!(["trader"])), ("t-bob", json!([]))] {
        let got = run(&["describe", &app.http(), "--token", token]);
        assert!(got.ok, "{}", got.stderr);
        assert_eq!(got.stdout, format!("{}\n", http_description(app.a.http, token)), "{token}: the served bytes");
        assert_eq!(json_of(&got.stdout)["caller"]["roles"], roles);
    }
    // a caller the sources name nobody for is refused with the source's reason
    let nobody = run(&["describe", &app.http(), "--token", "t-nobody"]);
    assert!(!nobody.ok);
    assert!(nobody.stderr.contains("401") && nobody.stderr.contains("unauthenticated") && nobody.stderr.contains("no such token"), "{}", nobody.stderr);

    // over a hub: the stream rows the caller may subscribe to
    let alice = json_of(&run(&["describe", &app.hub(), "--token", "t-alice"]).stdout);
    assert_eq!(alice["surface"], Value::Null);
    assert_eq!(alice["streams"][0]["topic"], "Ticks");
    let bob = json_of(&run(&["describe", &app.hub(), "--token", "t-bob"]).stdout);
    assert_eq!(bob["streams"], json!([]));
}

#[test]
fn describe_of_a_program_is_the_check_api_document() {
    // no exposure is running: the description of a program is its rows'
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/api_description/main.hl");
    let via_describe = run(&["describe", fixture.to_str().unwrap(), "--exposure", "books", "--caller", "uid:1000", "--holds", "auditor"]);
    let via_check = run(&["check", "--api", "--exposure", "books", "--caller", "uid:1000", "--holds", "auditor", fixture.to_str().unwrap()]);
    assert!(via_describe.ok, "{}", via_describe.stderr);
    assert_eq!(via_describe.stdout, via_check.stdout);
    // the live forms of the endpoint do not take the program's flags
    let bad = run(&["describe", "/nonexistent.sock", "--openapi"]);
    assert!(!bad.ok && bad.stderr.contains("--openapi is a flag of a program"), "{}", bad.stderr);
}

// ---- call -----------------------------------------------------------------------------------

#[test]
fn call_speaks_each_transports_wire_and_names_the_digest_it_read() {
    let _g = exclusive();
    let app = Running::start();
    let sock = app.sock();

    // the socket: a result, then the same state read back through HTTP
    let r = run(&["call", &sock, "Counter::add", r#"{"n": 2}"#]);
    assert!(r.ok, "{}", r.stderr);
    assert_eq!(json_of(&r.stdout), json!({ "total": 2 }));
    let r = run(&["call", &app.http(), "Counter::peek", r#"{"n": 0}"#, "--token", "t-bob"]);
    assert!(r.ok, "{}", r.stderr);
    assert_eq!(json_of(&r.stdout), json!({ "total": 2 }), "one Counter behind both exposures");

    // `--receipt`: the reply line over the socket, the status and body over HTTP
    let r = run(&["call", &sock, "Counter::peek", r#"{"n": 0}"#, "--receipt"]);
    let line = json_of(&r.stdout);
    assert_eq!(line["ok"], true);
    assert!(line["request_id"].is_i64() && line["caller"]["mode"] == "unix", "{line}");
    let r = run(&["call", &app.http(), "Counter::peek", r#"{"n": 0}"#, "--receipt", "--token", "t-alice"]);
    assert_eq!(json_of(&r.stdout), json!({ "status": 200, "body": { "total": 2 } }));

    // the handler's own error: 422 over HTTP, `{"ok": false, "error": …}` over the socket
    for endpoint in [sock.clone(), app.http()] {
        let r = run(&["call", &endpoint, "Counter::fail", r#"{"n": 1}"#, "--token", "t-alice"]);
        assert!(!r.ok, "{endpoint}");
        assert!(r.stderr.contains("handler error") && r.stderr.contains("\"code\":\"nope\""), "{endpoint}: {}", r.stderr);
    }

    // a payload the member does not decode is the exposure's `malformed`
    let r = run(&["call", &sock, "Counter::add", r#"{"n": "two"}"#]);
    assert!(!r.ok && r.stderr.contains("refused: malformed"), "{}", r.stderr);

    // a member the caller may not call is not listed: the client says so and lists what is
    let r = run(&["call", &app.http(), "Counter::add", r#"{"n": 1}"#, "--token", "t-bob"]);
    assert!(!r.ok);
    assert!(r.stderr.contains("not a member this caller may call") && r.stderr.contains("Counter::peek") && !r.stderr.contains("Counter::add,"), "{}", r.stderr);
    // and the exposure refuses it by name when asked anyway, naming what the row requires
    let body = r#"{"n": 1}"#;
    let mut s = TcpStream::connect(("127.0.0.1", app.a.http)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    write!(s, "POST /call/Counter%3A%3Aadd HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer t-bob\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    let mut raw = String::new();
    s.read_to_string(&mut raw).unwrap();
    assert!(raw.starts_with("HTTP/1.1 403") && raw.contains("\"requires\":[\"trader\"]"), "{raw}");

    // a digest that is not the served one is refused, not run: the line wire, as a client of it
    let line = unix_line(&sock, r#"{"call":"Counter::add","payload":{"n":1},"id":1,"digest":"fnv1a64:0000000000000000"}"#);
    assert!(line.contains("\"kind\":\"digest_mismatch\"") && line.contains("\"served\":\"fnv1a64:"), "{line}");
    let r = run(&["call", &sock, "Counter::peek", r#"{"n": 0}"#]);
    assert_eq!(json_of(&r.stdout), json!({ "total": 2 }), "the refused call did not run, and the digest the client names is accepted");

    // over a hub's connection: the `call` frame, under the hub's sources (alice holds no `trader` there,
    // and `peek` requires none)
    let r = run(&["call", &app.hub(), "Counter::peek", r#"{"n": 0}"#, "--token", "t-alice"]);
    assert!(r.ok, "{}", r.stderr);
    assert_eq!(json_of(&r.stdout), json!({ "total": 2 }));
    let r = run(&["call", &app.hub(), "Counter::add", r#"{"n": 1}"#, "--token", "t-alice"]);
    assert!(!r.ok && r.stderr.contains("refused: unauthorized") && r.stderr.contains("requires trader"), "{}", r.stderr);

    // a socket that is not there, a scheme that is not spoken
    let r = run(&["call", "/nonexistent/hale.sock", "Counter::peek"]);
    assert!(!r.ok && r.stderr.contains("could not connect"), "{}", r.stderr);
    let r = run(&["call", "https://127.0.0.1:1", "Counter::peek"]);
    assert!(!r.ok && r.stderr.contains("TLS"), "{}", r.stderr);
    // gRPC has no client here (spec/api.md, Open points): a grpc:// endpoint says so, for describe too
    for verb in ["call", "describe"] {
        let r = run(&[verb, "grpc://127.0.0.1:1", "Counter::peek"]);
        assert!(!r.ok && r.stderr.contains("do not speak gRPC"), "{verb}: {}", r.stderr);
    }
}

// ---- watch ----------------------------------------------------------------------------------

/// A child's stdout, a line at a time with a bound.
struct Lines {
    child: Child,
    rx: std::sync::mpsc::Receiver<String>,
}

impl Lines {
    fn spawn(mut cmd: Command) -> Lines {
        let mut child = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().expect("spawn");
        let out = child.stdout.take().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for l in BufReader::new(out).lines().map_while(Result::ok) {
                if tx.send(l).is_err() {
                    break;
                }
            }
        });
        Lines { child, rx }
    }
    fn next(&self) -> String {
        self.rx.recv_timeout(Duration::from_secs(15)).expect("a line within 15 s")
    }
}

impl Drop for Lines {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn watch_subscribes_through_the_hub_and_prints_the_frames() {
    let _g = exclusive();
    let app = Running::start();
    let mut cmd = hale();
    cmd.args(["watch", &app.hub(), "Ticks", "--token", "t-alice"]);
    let w = Lines::spawn(cmd);
    assert_eq!(w.next(), r#"{"type":"subscribed","topic":"Ticks"}"#);
    // a call through the socket publishes a Tick; the stream carries it with its seq
    let r = run(&["call", &app.sock(), "Counter::add", r#"{"n": 5}"#]);
    assert!(r.ok, "{}", r.stderr);
    assert_eq!(w.next(), r#"{"type":"event","topic":"Ticks","seq":1,"payload":{"n":5}}"#);
    run(&["call", &app.sock(), "Counter::add", r#"{"n": 1}"#]);
    assert_eq!(w.next(), r#"{"type":"event","topic":"Ticks","seq":2,"payload":{"n":6}}"#);
}

#[test]
fn watch_refuses_what_the_description_does_not_list() {
    let _g = exclusive();
    let app = Running::start();
    // bob holds nothing under the hub's roles: the stream is not in his description
    let r = run(&["watch", &app.hub(), "Ticks", "--token", "t-bob"]);
    assert!(!r.ok);
    assert!(r.stderr.contains("not a stream this caller may subscribe to"), "{}", r.stderr);
    // a stream is served by a hub, not by a socket
    let r = run(&["watch", &app.sock(), "Ticks"]);
    assert!(!r.ok && r.stderr.contains("ws://"), "{}", r.stderr);
}

// ---- mcp --app ---------------------------------------------------------------------------------

struct Mcp {
    child: Child,
    stdin: std::process::ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
}

impl Mcp {
    fn start(args: &[&str]) -> Mcp {
        let mut child = hale().args(["mcp", "--app"]).args(args).env("HALE_API_TOKEN", "").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit()).spawn().expect("hale mcp --app");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Mcp { child, stdin, stdout }
    }
    fn call(&mut self, v: Value) -> Value {
        writeln!(self.stdin, "{}", v).unwrap();
        self.stdin.flush().unwrap();
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap_or_else(|e| panic!("{e}: {line}"))
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn tool_names(v: &Value) -> Vec<String> {
    v["result"]["tools"].as_array().expect("tools").iter().map(|t| t["name"].as_str().unwrap().to_string()).collect()
}

#[test]
fn mcp_app_lists_the_members_as_tools_and_calls_through() {
    let _g = exclusive();
    let app = Running::start();

    // over the socket: every member this uid may call is a tool, its input the request's schema
    let mut m = Mcp::start(&[&app.sock()]);
    let init = m.call(json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-06-18" } }));
    assert!(init["result"]["serverInfo"]["name"].as_str().unwrap().starts_with("hale-app:Books@fnv1a64:"), "{init}");
    let list = m.call(json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }));
    assert_eq!(tool_names(&list), ["Counter__add", "Counter__fail", "Counter__peek"]);
    let add = &list["result"]["tools"][0];
    assert_eq!(add["inputSchema"]["properties"]["n"]["type"], "integer");
    assert_eq!(add["x-hale-requires"], json!(["trader"]));
    let r = m.call(json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": { "name": "Counter__add", "arguments": { "n": 4 } } }));
    assert_eq!(r["result"]["isError"], false, "{r}");
    assert_eq!(json_of(r["result"]["content"][0]["text"].as_str().unwrap()), json!({ "total": 4 }));
    // the handler's error and an unknown tool are errors the host reads
    let r = m.call(json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": { "name": "Counter__fail", "arguments": { "n": 1 } } }));
    assert_eq!(r["result"]["isError"], true);
    assert!(r["result"]["content"][0]["text"].as_str().unwrap().contains("nope"), "{r}");
    let r = m.call(json!({ "jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": { "name": "Counter__nothing", "arguments": {} } }));
    assert_eq!(r["result"]["isError"], true);
    drop(m);

    // over HTTP: bob's tools are the members bob may call
    let mut m = Mcp::start(&[&app.http(), "--token", "t-bob"]);
    let list = m.call(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }));
    assert_eq!(tool_names(&list), ["Counter__fail", "Counter__peek"]);
    drop(m);

    // an mcp::Rpc listener's own tools/list is forwarded: the same names as the exposure serves
    let mut m = Mcp::start(&[&app.mcp(), "--token", "t-alice"]);
    let list = m.call(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }));
    assert_eq!(tool_names(&list), ["Counter__add", "Counter__fail", "Counter__peek"], "{list}");
    let r = m.call(json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": { "name": "Counter__peek", "arguments": { "n": 0 } } }));
    assert_eq!(r["result"]["isError"], false, "{r}");
    assert_eq!(r["result"]["structuredContent"], json!({ "total": 4 }));
}

// ---- admin ----------------------------------------------------------------------------------------

/// `GET`/`POST` against the admin page with the given headers: (status line, body).
fn admin_request(port: u16, method: &str, path: &str, headers: &[(&str, &str)], body: Option<&str>) -> (String, String) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let mut req = format!("{method} {path} HTTP/1.1\r\nConnection: close\r\n");
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    if let Some(b) = body {
        req.push_str(&format!("Content-Length: {}\r\n", b.len()));
    }
    req.push_str("\r\n");
    if let Some(b) = body {
        req.push_str(b);
    }
    s.write_all(req.as_bytes()).unwrap();
    let mut raw = String::new();
    let _ = s.read_to_string(&mut raw);
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
    (head.lines().next().unwrap_or("").to_string(), body.to_string())
}

#[test]
fn admin_serves_the_description_calls_through_and_refuses_other_sites() {
    let _g = exclusive();
    let app = Running::start();
    let port = free_port();
    let mut cmd = hale();
    cmd.args(["admin", &app.sock(), "--port", &port.to_string()]);
    let admin = Lines::spawn(cmd);
    let banner = admin.next();
    let token = banner.split("token=").nth(1).and_then(|t| t.split_whitespace().next()).expect("the banner carries the token").to_string();
    let host = format!("127.0.0.1:{port}");
    let ok = |extra: &[(&'static str, &str)]| -> Vec<(&'static str, String)> {
        let mut h = vec![("Host", host.clone()), ("X-Hale-Admin", token.clone())];
        h.extend(extra.iter().map(|(k, v)| (*k, v.to_string())));
        h
    };
    let call = |method: &str, path: &str, extra: &[(&'static str, &str)], body: Option<&str>| {
        let h = ok(extra);
        let h: Vec<(&str, &str)> = h.iter().map(|(k, v)| (*k, v.as_str())).collect();
        admin_request(port, method, path, &h, body)
    };

    // the page and the description it reads
    let (status, page) = call("GET", "/", &[], None);
    assert!(status.contains("200") && page.contains("hale admin") && page.contains(&format!("const ENDPOINT = \"{}\"", app.sock())), "{status}");
    let (status, d) = call("GET", "/api/describe", &[], None);
    assert!(status.contains("200"), "{status}");
    assert_eq!(member_names(&json_of(&d)), ["Counter::add", "Counter::fail", "Counter::peek"]);
    // a call goes through the socket under the digest it read
    let (status, r) = call("POST", "/api/call/Counter%3A%3Aadd", &[("Content-Type", "application/json")], Some(r#"{"n": 3}"#));
    assert!(status.contains("200"), "{status} {r}");
    assert_eq!(json_of(&r), json!({ "outcome": "result", "status": null, "body": { "total": 3 } }));
    let (_, r) = call("POST", "/api/call/Counter%3A%3Afail", &[("Content-Type", "application/json")], Some(r#"{"n": 3}"#));
    assert_eq!(json_of(&r)["outcome"], "handler_error", "{r}");
    // a call that is not JSON, or not declared JSON, is refused before it reaches the exposure
    let (status, _) = call("POST", "/api/call/Counter%3A%3Aadd", &[("Content-Type", "application/json")], Some("{nope"));
    assert!(status.contains("400"), "{status}");
    let (status, _) = call("POST", "/api/call/Counter%3A%3Aadd", &[("Content-Type", "text/plain")], Some("{}"));
    assert!(status.contains("415"), "{status}");

    // another site reaching for the exposure through this process: a Host or an Origin that is not the page's, no token
    let (status, _) = admin_request(port, "GET", "/api/describe", &[("Host", "evil.example"), ("X-Hale-Admin", &token)], None);
    assert!(status.contains("403"), "{status}");
    let (status, _) = admin_request(port, "GET", "/api/describe", &[("Host", &host), ("Origin", "http://evil.example"), ("X-Hale-Admin", &token)], None);
    assert!(status.contains("403"), "{status}");
    let (status, _) = admin_request(port, "GET", "/api/describe", &[("Host", &host)], None);
    assert!(status.contains("403"), "{status}");
    let (status, _) = admin_request(port, "POST", "/api/call/Counter%3A%3Aadd", &[("Host", &host), ("Content-Type", "application/json")], Some("{}"));
    assert!(status.contains("403"), "{status}");
    let (status, _) = admin_request(port, "GET", "/", &[("Host", &host)], None);
    assert!(status.contains("403"), "the page itself is served to whoever holds the token");
}

#[test]
fn admin_over_a_hub_tails_a_stream() {
    let _g = exclusive();
    let app = Running::start();
    let port = free_port();
    let mut cmd = hale();
    cmd.args(["admin", &app.hub(), "--port", &port.to_string(), "--token", "t-alice"]);
    let admin = Lines::spawn(cmd);
    let banner = admin.next();
    let token = banner.split("token=").nth(1).and_then(|t| t.split_whitespace().next()).unwrap().to_string();
    let host = format!("127.0.0.1:{port}");
    // the tail is server-sent events: the first data line is `subscribed`, then the event
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(15))).unwrap();
    write!(s, "GET /api/watch/Ticks?token={token} HTTP/1.1\r\nHost: {host}\r\n\r\n").unwrap();
    let mut r = BufReader::new(s);
    let mut data = Vec::new();
    let mut fired = false;
    while data.len() < 2 {
        let mut l = String::new();
        r.read_line(&mut l).expect("an SSE line");
        if let Some(d) = l.strip_prefix("data: ") {
            data.push(d.trim().to_string());
            if !fired {
                fired = true;
                let c = run(&["call", &app.sock(), "Counter::add", r#"{"n": 7}"#]);
                assert!(c.ok, "{}", c.stderr);
            }
        }
    }
    assert_eq!(data[0], r#"{"type":"subscribed","topic":"Ticks"}"#);
    assert_eq!(data[1], r#"{"type":"event","topic":"Ticks","seq":1,"payload":{"n":7}}"#);
}

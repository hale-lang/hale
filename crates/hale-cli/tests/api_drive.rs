//! `hale api describe` and `hale api call` drive a served program from the
//! description it serves (spec/api.md § Driving a served program).
//!
//! The witness, `tests/api-contract/program.hl`, is built once per process
//! with the addresses a test cannot fix (the two HTTP binds, the Unix path, the
//! hub's bind, the Unix operator) read from the environment, and run by each
//! test that drives it. Its listeners bind port 0, each on a loopback address
//! of its own, and the test reads back the port the kernel gave each: a port
//! drawn here and handed over is free, not reserved, and under a parallel run
//! another test can bind it first. What the verbs print is held to what the program
//! serves and to `hale api export --surface`, the description the compiler
//! writes from the rows.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde_json::Value;

const PUBLIC_DIGEST: &str = "fnv1a64:a8930d6e7998e986";
const ADMIN_DIGEST: &str = "fnv1a64:40381db6685c9f75";

fn hale() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_hale"));
    c.env("HALE_SKIP_STALE_CHECK", "1").env_remove("HALE_API_BEARER");
    c
}

// The codegen suite's: what a child listens on, read from `/proc`.
#[path = "../../hale-codegen/tests/support/child.rs"]
mod child;

/// Where the witness's `BIND`, `BIND2` and hub listen.
const BIND_HOST: std::net::Ipv4Addr = std::net::Ipv4Addr::new(127, 0, 0, 1);
const BIND2_HOST: std::net::Ipv4Addr = std::net::Ipv4Addr::new(127, 0, 0, 2);
const HUB_BIND: &str = "127.0.0.3:0";

fn uid() -> u32 {
    // SAFETY: getuid cannot fail.
    unsafe { libc::getuid() }
}

fn unique_dir(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!("hale_api_drive_{}_{}_{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed), tag));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn witness_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/api-contract/program.hl")
}

/// The witness with its addresses read from the environment and a run loop
/// that waits for nothing but the end of the process.
fn witness_source() -> String {
    let mut s = std::fs::read_to_string(witness_path()).expect("the witness");
    let mut swap = |from: &str, to: &str| {
        assert!(s.contains(from), "tests/api-contract/program.hl no longer has `{from}`");
        s = s.replacen(from, to, 1);
    };
    swap("bind: \"127.0.0.1:8080\"", "bind: std::env::var(\"BIND\")");
    swap("bind: \"127.0.0.1:8081\"", "bind: std::env::var(\"BIND2\")");
    swap("path: \"/run/desk/admin.sock\"", "path: std::env::var(\"SOCK\")");
    swap("bind: \"127.0.0.1:9000\"", "bind: std::env::var(\"HUB\")");
    swap("operator: \"uid:1000\"", "operator: std::env::var(\"OPERATOR\")");
    swap(
        "        std::api::run_until_stopped(self.public);\n",
        "        while !written(std::env::var(\"TRIGGER\")) { std::time::sleep(10ms); }\n",
    );
    swap("fn main() { Desk { }; }", "fn written(path: String) -> Bool {\n    return (std::io::fs::read_file(path) or \"\") != \"\";\n}\n\nfn main() { Desk { }; }");
    s
}

fn built() -> &'static PathBuf {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| {
        let dir = unique_dir("build");
        let file = dir.join("main.hl");
        std::fs::write(&file, witness_source()).expect("write the witness");
        let bin = dir.join("witness");
        let out = hale().arg("build").arg(&file).arg("-o").arg(&bin).output().expect("hale build");
        assert!(out.status.success(), "build: {}", String::from_utf8_lossy(&out.stderr));
        bin
    })
}

/// The witness, running; dropping it kills it.
struct Witness {
    child: Child,
    dir: PathBuf,
    sock: String,
    public: String,
    partner: String,
}

impl Witness {
    fn start() -> Witness {
        Witness::start_as(&format!("uid:{}", uid()))
    }
    /// The witness with `operator` the Unix peer `admin_roles` grants `operator`.
    fn start_as(operator: &str) -> Witness {
        let bin = built().clone();
        let dir = unique_dir("run");
        let sock = dir.join("a.sock");
        assert!(sock.as_os_str().len() < 100, "the socket path is too long for sockaddr_un");
        let mut child = Command::new(&bin)
            .env("BIND", format!("{BIND_HOST}:0"))
            .env("BIND2", format!("{BIND2_HOST}:0"))
            .env("HUB", HUB_BIND)
            .env("SOCK", &sock)
            .env("OPERATOR", operator)
            .env("TRIGGER", dir.join("stop"))
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("run the witness");
        let p1 = child::tcp_port_on(&mut child, BIND_HOST);
        let p2 = child::tcp_port_on(&mut child, BIND2_HOST);
        let w = Witness { child, dir, sock: sock.display().to_string(), public: format!("{BIND_HOST}:{p1}"), partner: format!("{BIND2_HOST}:{p2}") };
        let start = Instant::now();
        while !(sock.exists() && TcpStream::connect(&w.public).is_ok() && TcpStream::connect(&w.partner).is_ok()) {
            assert!(start.elapsed() < Duration::from_secs(30), "the witness never began listening");
            std::thread::sleep(Duration::from_millis(20));
        }
        w
    }
    fn unix(&self) -> String {
        format!("unix:{}", self.sock)
    }
    fn http(&self) -> String {
        format!("http://{}", self.public)
    }
}

impl Drop for Witness {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

struct Out {
    code: i32,
    stdout: String,
    stderr: String,
}

fn run(args: &[&str]) -> Out {
    run_env(args, &[])
}

fn run_env(args: &[&str], env: &[(&str, &str)]) -> Out {
    let mut c = hale();
    c.args(args);
    for (k, v) in env {
        c.env(k, v);
    }
    let o = c.output().expect("run hale");
    Out { code: o.status.code().expect("exited"), stdout: String::from_utf8_lossy(&o.stdout).into_owned(), stderr: String::from_utf8_lossy(&o.stderr).into_owned() }
}

fn json_of(s: &str) -> Value {
    serde_json::from_str(s.trim()).unwrap_or_else(|e| panic!("{e}: {s}"))
}

/// The members a document lists, sorted.
fn members_of(v: &Value) -> Vec<String> {
    let mut m: Vec<String> = v["members"].as_array().expect("members").iter().map(|m| m["name"].as_str().expect("name").to_string()).collect();
    m.sort();
    m
}

/// The members `hale api export --surface` writes for `surface`, sorted.
fn exported_members(surface: &str) -> Vec<String> {
    let dir = unique_dir("export");
    let out = hale().args(["api", "export", "--surface", surface, "--out"]).arg(&dir).arg(witness_path()).output().expect("export");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let doc = json_of(&std::fs::read_to_string(dir.join(format!("{surface}.description.json"))).expect("exported description"));
    let _ = std::fs::remove_dir_all(&dir);
    let mut m: Vec<String> = doc["surfaces"][0]["members"].as_array().expect("members").iter().map(|m| m["name"].as_str().expect("name").to_string()).collect();
    m.sort();
    m
}

#[test]
#[cfg_attr(not(target_os = "linux"), ignore = "binds 127.0.0.2 and up: macOS's loopback carries only 127.0.0.1")]
fn describe_over_unix_answers_the_surface_digest_and_its_members() {
    let w = Witness::start();
    let o = run(&["api", "describe", &w.unix(), "--json"]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    let doc = json_of(&o.stdout);
    assert_eq!(doc["digest"], ADMIN_DIGEST);
    assert_eq!(doc["exposure"], format!("Admin@{ADMIN_DIGEST}/admin"));
    assert_eq!(members_of(&doc), exported_members("Admin"));
    // verbatim: the line the program served, one line
    assert_eq!(o.stdout.trim().lines().count(), 1);

    let readable = run(&["api", "describe", &w.unix()]);
    assert_eq!(readable.code, 0, "{}", readable.stderr);
    assert!(readable.stdout.starts_with(&format!("Admin@{ADMIN_DIGEST}/admin")), "{}", readable.stdout);
    assert!(readable.stdout.contains("Orders::cancel(order: Int (OrderId)) -> Cancelled, error OrderError  requires: operator"), "{}", readable.stdout);
    assert!(readable.stdout.contains("Ledger::rebalance(book: String) -> Rebalanced, error ClosureViolation"), "{}", readable.stdout);
}

#[test]
#[cfg_attr(not(target_os = "linux"), ignore = "binds 127.0.0.2 and up: macOS's loopback carries only 127.0.0.1")]
fn describe_over_http_answers_the_callers_slice_by_bearer() {
    let w = Witness::start();
    let alice = run(&["api", "describe", &w.http(), "--json", "--bearer", "t-alice"]);
    assert_eq!(alice.code, 0, "{}", alice.stderr);
    let doc = json_of(&alice.stdout);
    assert_eq!(doc["digest"], PUBLIC_DIGEST);
    assert_eq!(members_of(&doc), exported_members("Public"));

    // bob holds nothing under `public_roles`: `Orders::cancel` is absent for him
    // (the bearer from the environment, the flag's other source)
    let bob = run_env(&["api", "describe", &w.http(), "--json"], &[("HALE_API_BEARER", "t-bob")]);
    assert_eq!(bob.code, 0, "{}", bob.stderr);
    assert_eq!(members_of(&json_of(&bob.stdout)), vec!["Orders::place".to_string()]);

    let readable = run(&["api", "describe", &w.http(), "--bearer", "t-alice"]);
    assert!(readable.stdout.contains("caller: bearer alice; roles: trader"), "{}", readable.stdout);
    assert!(readable.stdout.contains("Orders::place(symbol: String, qty: Int, limit: Int (Money, q(cent))) -> OrderReceipt, error ClosureViolation  requires: -"), "{}", readable.stdout);
    let readable_bob = run(&["api", "describe", &w.http(), "--bearer", "t-bob"]);
    assert!(!readable_bob.stdout.contains("Orders::cancel"), "{}", readable_bob.stdout);

    // a caller the bearer source names nobody for is refused, which is an
    // answer (2), not silence (4)
    let nobody = run(&["api", "describe", &w.http()]);
    assert_eq!(nobody.code, 2, "{}", nobody.stderr);
    assert!(nobody.stderr.contains("unauthenticated"), "{}", nobody.stderr);
}

#[test]
fn describe_refuses_the_transports_it_does_not_drive_naming_the_two_it_does() {
    for ep in ["grpc://127.0.0.1:50051", "mcp://127.0.0.1:8090"] {
        let o = run(&["api", "describe", ep]);
        assert_eq!(o.code, 5, "{ep}");
        assert!(o.stdout.is_empty());
        assert!(o.stderr.contains("unix:<path>") && o.stderr.contains("http://host:port"), "{}", o.stderr);
    }
    assert_eq!(run(&["api", "describe"]).code, 5);
    assert_eq!(run(&["api", "describe", "unix:/x", "--nope"]).code, 5);
}

#[test]
fn describe_of_an_endpoint_that_does_not_answer_exits_four_and_says_which() {
    let dir = unique_dir("dead");
    let none = run(&["api", "describe", &format!("unix:{}", dir.join("none.sock").display())]);
    assert_eq!(none.code, 4, "{}", none.stderr);
    assert!(none.stderr.contains("could not connect"), "{}", none.stderr);

    let refusing = child::Refusing::new();
    let closed = run(&["api", "describe", &format!("http://{}", refusing.addr())]);
    assert_eq!(closed.code, 4, "{}", closed.stderr);
    assert!(closed.stderr.contains("could not connect"), "{}", closed.stderr);

    // something answers, and it is not a Hale exposure
    let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = l.local_addr().unwrap().port();
    let t = std::thread::spawn(move || {
        let (mut s, _) = l.accept().expect("accept");
        let mut buf = [0u8; 1024];
        let _ = s.read(&mut buf);
        let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello");
    });
    let other = run(&["api", "describe", &format!("http://127.0.0.1:{port}")]);
    t.join().unwrap();
    assert_eq!(other.code, 4, "{}", other.stderr);
    assert!(other.stderr.contains("not a Hale exposure"), "{}", other.stderr);
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- hale api call ---------------------------------------------------------------

/// `run` with `input` on the verb's stdin.
fn run_stdin(args: &[&str], input: &str) -> Out {
    let mut child = hale().args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().expect("run hale");
    child.stdin.take().expect("stdin").write_all(input.as_bytes()).expect("write stdin");
    let o = child.wait_with_output().expect("wait");
    Out { code: o.status.code().expect("exited"), stdout: String::from_utf8_lossy(&o.stdout).into_owned(), stderr: String::from_utf8_lossy(&o.stderr).into_owned() }
}

/// What the stand-in answers a call with.
#[derive(Clone, Copy)]
enum Answer {
    Result,
    /// a reply whose `id` is not the request's
    OtherId,
    /// the connection ends with no reply
    Hangup,
}

/// A unix::Rpc stand-in: it answers `{"describe": true}` with a description and
/// records every call line it is sent, so a test sees the wire line a verb wrote.
struct Fake {
    dir: PathBuf,
    sock: String,
    calls: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

impl Fake {
    fn start(doc: &Value, answer: Answer) -> Fake {
        use std::io::BufRead;
        let dir = unique_dir("fake");
        let path = dir.join("f.sock");
        let l = std::os::unix::net::UnixListener::bind(&path).expect("bind the stand-in");
        let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let (doc, seen) = (doc.to_string(), calls.clone());
        std::thread::spawn(move || {
            for conn in l.incoming() {
                let Ok(mut c) = conn else { return };
                let mut line = String::new();
                if std::io::BufReader::new(c.try_clone().expect("clone")).read_line(&mut line).unwrap_or(0) == 0 {
                    continue;
                }
                let req: Value = serde_json::from_str(line.trim()).expect("a request line");
                if req.get("describe").is_some() {
                    let _ = writeln!(c, "{}", serde_json::json!({"ok": true, "value": serde_json::from_str::<Value>(&doc).unwrap()}));
                    continue;
                }
                seen.lock().unwrap().push(line.trim().to_string());
                let id = match answer {
                    Answer::Hangup => continue,
                    Answer::OtherId => serde_json::json!("not-yours"),
                    Answer::Result => req["id"].clone(),
                };
                let reply = serde_json::json!({"request_id": 1, "id": id, "ok": true, "value": {"fine": true}, "caller": {"mode": "unix", "name": "uid:1"}});
                let _ = writeln!(c, "{reply}");
            }
        });
        Fake { sock: format!("unix:{}", path.display()), dir, calls }
    }
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A description with one member that takes every kind of field.
fn shop_doc() -> Value {
    serde_json::json!({
        "description": 1, "exposure": "Shop@fnv1a64:0000000000000001/shop", "name": "shop", "surface": "Shop", "digest": "fnv1a64:0000000000000001",
        "listener": {"transport": "unix", "address": "/x"}, "caller": {"principal": {"mode": "unix", "name": "uid:1"}, "roles": []},
        "members": [{"name": "Shop::order", "request": {"$ref": "#/schemas/Order"}, "response": {"$ref": "#/schemas/Order"}, "error": "ClosureViolation", "requires": []}],
        "schemas": {
            "Order": {"type": "object",
                "properties": {
                    "name": {"type": "string"}, "qty": {"type": "integer"}, "price": {"type": "number"}, "rush": {"type": "boolean"},
                    "limit": {"type": "integer", "x-hale-type": "Money", "x-hale-unit": "q(cent)"},
                    "items": {"type": "array", "items": {"type": "integer"}}, "ship": {"$ref": "#/schemas/Address"}, "note": {"type": "string"}},
                "required": ["name", "qty", "price", "rush", "limit", "items", "ship"]},
            "Address": {"type": "object", "properties": {"city": {"type": "string"}}, "required": ["city"]}}
    })
}

/// A call line with the verb's own request id made `#`.
fn mask_id(line: &str) -> String {
    let v: Value = serde_json::from_str(line).expect("a request line");
    let id = v["id"].as_str().expect("the verb's id");
    assert!(id.starts_with("hale-"), "{id}");
    line.replace(&format!("\"id\":\"{id}\""), "\"id\":#")
}

#[test]
fn typed_flags_and_json_write_the_same_wire_line() {
    let fake = Fake::start(&shop_doc(), Answer::Result);
    let flags = run(&[
        "api", "call", &fake.sock, "Shop::order", "--name", "alice", "--qty", "3", "--price", "2.5", "--rush", "--limit", "125", "--items", "[1,2,3]", "--ship",
        "{\"city\":\"Oslo\"}",
    ]);
    assert_eq!(flags.code, 0, "{}", flags.stderr);
    assert_eq!(json_of(&flags.stdout), serde_json::json!({"fine": true}));
    let json = run(&[
        "api", "call", &fake.sock, "Shop::order", "--json",
        "{\"name\":\"alice\",\"qty\":3,\"price\":2.5,\"rush\":true,\"limit\":125,\"items\":[1,2,3],\"ship\":{\"city\":\"Oslo\"}}",
    ]);
    assert_eq!(json.code, 0, "{}", json.stderr);
    let stdin = run_stdin(
        &["api", "call", &fake.sock, "Shop::order", "--json", "-"],
        "{ \"name\": \"alice\", \"qty\": 3, \"price\": 2.5, \"rush\": true, \"limit\": 125, \"items\": [1, 2, 3], \"ship\": {\"city\": \"Oslo\"} }\n",
    );
    assert_eq!(stdin.code, 0, "{}", stdin.stderr);
    let calls = fake.calls();
    assert_eq!(calls.len(), 3);
    assert_eq!(mask_id(&calls[0]), mask_id(&calls[1]));
    assert_eq!(mask_id(&calls[0]), mask_id(&calls[2]));
    let v: Value = serde_json::from_str(&calls[0]).unwrap();
    assert_eq!(v["call"], "Shop::order");
    assert_eq!(v["digest"], "fnv1a64:0000000000000001", "the call names the digest of the description it read");
    assert_eq!(v["payload"]["limit"], 125);
    assert_eq!(v["payload"]["price"], 2.5);
    assert_eq!(v["payload"]["rush"], true);
    assert!(v["payload"].get("note").is_none(), "an optional field not given is not sent");

    // --request-id and --digest are the caller's
    let own = run(&["api", "call", &fake.sock, "Shop::order", "--json", "{}", "--request-id", "c-9", "--digest", "fnv1a64:feed"]);
    assert_eq!(own.code, 0, "{}", own.stderr);
    let v: Value = serde_json::from_str(fake.calls().last().unwrap()).unwrap();
    assert_eq!((v["id"].as_str(), v["digest"].as_str()), (Some("c-9"), Some("fnv1a64:feed")));
}

/// A payload field named `id` is set by `--id`: the verb's own request id is
/// `--request-id`, so the commonest field name is not shadowed (downstream handoff).
#[test]
fn a_payload_field_named_id_is_a_typed_flag_and_the_request_id_is_its_own() {
    let doc = serde_json::json!({
        "description": 1, "exposure": "Items@fnv1a64:0000000000000002/items", "name": "items", "surface": "Items", "digest": "fnv1a64:0000000000000002",
        "listener": {"transport": "unix", "address": "/x"}, "caller": {"principal": {"mode": "unix", "name": "uid:1"}, "roles": []},
        "members": [{"name": "Items::get", "request": {"$ref": "#/schemas/Get"}, "response": {"$ref": "#/schemas/Get"}, "error": "ClosureViolation", "requires": []}],
        "schemas": {"Get": {"type": "object", "properties": {"id": {"type": "integer"}}, "required": ["id"]}}
    });
    let fake = Fake::start(&doc, Answer::Result);
    let o = run(&["api", "call", &fake.sock, "Items::get", "--id", "5", "--request-id", "r-1"]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    let v: Value = serde_json::from_str(fake.calls().last().unwrap()).unwrap();
    assert_eq!(v["payload"], serde_json::json!({"id": 5}), "--id sets the payload field");
    assert_eq!(v["id"], "r-1", "--request-id names the request");
    // the short form agrees
    let o = run(&["call", &fake.sock, "Items::get", "--id", "6"]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    let v: Value = serde_json::from_str(fake.calls().last().unwrap()).unwrap();
    assert_eq!(v["payload"]["id"], 6);
    assert!(v["id"].as_str().unwrap().starts_with("hale-"), "{v}");
}

#[test]
fn a_flag_that_does_not_fit_its_field_is_refused_before_anything_is_sent() {
    let fake = Fake::start(&shop_doc(), Answer::Result);
    let base: [(&str, &str); 7] = [
        ("--name", "a"),
        ("--qty", "3"),
        ("--price", "2.5"),
        ("--rush", "true"),
        ("--limit", "125"),
        ("--items", "[1]"),
        ("--ship", "{\"city\":\"o\"}"),
    ];
    // the flags of a good call with `flag` given `bad` instead
    let with = |flag: &str, bad: &str| -> Out {
        let mut a = vec!["api", "call", fake.sock.as_str(), "Shop::order"];
        for (k, v) in &base {
            a.push(k);
            a.push(if *k == flag { bad } else { v });
        }
        run(&a)
    };
    assert_eq!(with("--qty", "3").code, 0, "the good call is good");
    let sent = fake.calls().len();
    // wrong-typed: names the field and its type
    let o = with("--qty", "three");
    assert_eq!(o.code, 5, "{}", o.stderr);
    assert!(o.stderr.contains("--qty") && o.stderr.contains("Int") && o.stderr.contains("\"type\":\"integer\""), "{}", o.stderr);
    for (flag, bad, ty) in [("--price", "x", "Float"), ("--rush", "maybe", "Bool"), ("--limit", "1.5", "Money"), ("--items", "{}", "[Int]"), ("--ship", "[1]", "Address"), ("--items", "[1", "[Int]")] {
        let o = with(flag, bad);
        assert_eq!(o.code, 5, "{flag} {bad}: {}", o.stderr);
        assert!(o.stderr.contains(flag) && o.stderr.contains(ty), "{flag} {bad}: {}", o.stderr);
    }
    // an unknown flag lists the fields; a missing required field names itself and its schema
    let o = run(&["api", "call", &fake.sock, "Shop::order", "--nope", "1"]);
    assert_eq!(o.code, 5);
    assert!(o.stderr.contains("--nope") && o.stderr.contains("--name <String>"), "{}", o.stderr);
    let o = run(&["api", "call", &fake.sock, "Shop::order", "--name", "a"]);
    assert_eq!(o.code, 5);
    assert!(o.stderr.contains("needs `--qty`") && o.stderr.contains("\"type\":\"integer\""), "{}", o.stderr);
    // flags and --json together; a member the caller does not see, with flags; the rest of usage
    assert_eq!(run(&["api", "call", &fake.sock, "Shop::order", "--json", "{}", "--qty", "1"]).code, 5);
    let o = run(&["api", "call", &fake.sock, "Shop::gone", "--qty", "1"]);
    assert_eq!(o.code, 5);
    assert!(o.stderr.contains("Shop::order"), "{}", o.stderr);
    assert_eq!(run(&["api", "call", &fake.sock, "Shop::order", "--json", "{nope"]).code, 5);
    assert_eq!(run(&["api", "call", "grpc://127.0.0.1:1", "Shop::order", "--json", "{}"]).code, 5);
    assert_eq!(run(&["api", "call", &fake.sock]).code, 5);
    assert_eq!(fake.calls().len(), sent, "nothing more was sent: {:?}", fake.calls());
}

/// `Shop::order` plus a list of records, each holding a record and an optional field.
fn nested_doc() -> Value {
    let mut doc = shop_doc();
    doc["schemas"]["Order"]["properties"]["lines"] = serde_json::json!({"type": "array", "items": {"$ref": "#/schemas/Line"}});
    doc["schemas"]["Line"] = serde_json::json!({"type": "object",
        "properties": {"sku": {"type": "string"}, "qty": {"type": "integer", "x-hale-type": "Money", "x-hale-unit": "q(cent)"}, "to": {"$ref": "#/schemas/Address"}, "gift": {"type": "string"}},
        "required": ["sku", "qty", "to"]});
    doc
}

#[test]
fn record_and_list_flags_are_checked_against_their_schemas_before_anything_is_sent() {
    let fake = Fake::start(&nested_doc(), Answer::Result);
    let call = |items: &str, ship: &str, lines: &str| -> Out {
        run(&["api", "call", &fake.sock, "Shop::order", "--name", "a", "--qty", "1", "--price", "1", "--rush", "--limit", "5", "--items", items, "--ship", ship, "--lines", lines])
    };
    let city = r#"{"city":"o"}"#;
    let good_line = r#"{"sku":"s","qty":2,"to":{"city":"Oslo"}}"#;
    let good_lines = format!("[{good_line}]");
    for (o, path, ty) in [
        (call(r#"["x"]"#, city, &good_lines), "items[0]", "Int"),
        (call("[1]", "{}", &good_lines), "required field `city`", "Address"),
        (call("[1]", r#"{"city":7}"#, &good_lines), "ship.city", "String"),
        (call("[1]", r#"{"city":"o","zip":1}"#, &good_lines), "unknown field `zip`", "Address"),
        (call("[1]", city, r#"[{"sku":"s","qty":2,"to":{"city":3}}]"#), "lines[0].to.city", "String"),
        (call("[1]", city, r#"[{"sku":"s","qty":1.5,"to":{"city":"o"}}]"#), "lines[0].qty", "Money"),
        (call("[1]", city, r#"[{"sku":"s","to":{"city":"o"}}]"#), "required field `qty`", "Line"),
    ] {
        assert_eq!(o.code, 5, "{path}: {}", o.stderr);
        assert!(o.stderr.contains(path) && o.stderr.contains(ty), "{path}: {}", o.stderr);
    }
    assert!(fake.calls().is_empty(), "nothing was sent: {:?}", fake.calls());

    // a valid nested value is sent, and is the --json form byte for byte; the optional `gift` absent is not sent
    let o = call("[1,2]", r#"{"city":"Oslo"}"#, &good_lines);
    assert_eq!(o.code, 0, "{}", o.stderr);
    let body = format!(r#"{{"name":"a","qty":1,"price":1.0,"rush":true,"limit":5,"items":[1,2],"ship":{{"city":"Oslo"}},"lines":{good_lines}}}"#);
    let o = run(&["api", "call", &fake.sock, "Shop::order", "--json", &body]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    let calls = fake.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(mask_id(&calls[0]), mask_id(&calls[1]));
    assert!(!calls[0].contains("gift"), "{}", calls[0]);
}

#[test]
fn a_reply_that_is_not_the_requests_or_never_comes_is_a_transport_failure() {
    let other = Fake::start(&shop_doc(), Answer::OtherId);
    let o = run(&["api", "call", &other.sock, "Shop::order", "--json", "{}"]);
    assert_eq!(o.code, 4, "{}", o.stderr);
    assert!(o.stderr.contains("another request"), "{}", o.stderr);
    let gone = Fake::start(&shop_doc(), Answer::Hangup);
    let o = run(&["api", "call", &gone.sock, "Shop::order", "--json", "{}"]);
    assert_eq!(o.code, 4, "{}", o.stderr);
    // a dead endpoint
    let dir = unique_dir("dead");
    let o = run(&["api", "call", &format!("unix:{}", dir.join("none.sock").display()), "A::b", "--json", "{}"]);
    assert_eq!(o.code, 4, "{}", o.stderr);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[cfg_attr(not(target_os = "linux"), ignore = "binds 127.0.0.2 and up: macOS's loopback carries only 127.0.0.1")]
fn call_over_unix_prints_each_outcome_with_its_exit_code() {
    let w = Witness::start();
    let u = w.unix();

    // result: the value on stdout, exit 0
    let o = run(&["api", "call", &u, "Ledger::rebalance", "--book", "desk"]);
    assert_eq!((o.code, json_of(&o.stdout)), (0, serde_json::json!({"moved": 1500})), "{}", o.stderr);
    assert!(o.stderr.is_empty(), "{}", o.stderr);
    // the payload on stdin
    let o = run_stdin(&["api", "call", &u, "Ledger::rebalance", "--json", "-"], "{\"book\": \"desk\"}");
    assert_eq!((o.code, json_of(&o.stdout)), (0, serde_json::json!({"moved": 1500})), "{}", o.stderr);

    // handler error: the wire outcome on stderr, exit 1
    let o = run(&["api", "call", &u, "Orders::cancel", "--order", "999"]);
    assert_eq!(o.code, 1, "{}", o.stderr);
    assert!(o.stdout.is_empty());
    let wire = json_of(&o.stderr);
    assert_eq!((wire["ok"].as_bool(), wire["error"]["code"].as_str()), (Some(false), Some("unknown_order")));
    // --raw: the whole outcome line on stdout whatever the kind, the exit code unchanged
    let o = run(&["api", "call", &u, "Orders::cancel", "--order", "999", "--raw"]);
    assert_eq!(o.code, 1);
    assert_eq!(json_of(&o.stdout)["error"]["code"], "unknown_order");
    assert!(o.stderr.is_empty(), "{}", o.stderr);
    let o = run(&["api", "call", &u, "Ledger::rebalance", "--book", "desk", "--raw"]);
    assert_eq!(o.code, 0);
    let line = json_of(&o.stdout);
    assert_eq!((line["ok"].as_bool(), line["value"]["moved"].as_i64()), (Some(true), Some(1500)));
    assert!(line["id"].as_str().unwrap().starts_with("hale-") && line["caller"]["mode"] == "unix");
    // --request-id is echoed
    let o = run(&["api", "call", &u, "Ledger::rebalance", "--book", "desk", "--raw", "--request-id", "c-7"]);
    assert_eq!(json_of(&o.stdout)["id"], "c-7");

    // refusals: the kind and reason as the wire says them, exit 2
    let refusal = |args: &[&str]| -> Value {
        let mut a = vec!["api", "call", u.as_str()];
        a.extend_from_slice(args);
        let o = run(&a);
        assert_eq!(o.code, 2, "{}", o.stderr);
        json_of(&o.stderr)["refusal"].clone()
    };
    let malformed = |r: &Value, why: &str| r["kind"] == "malformed" && r["reason"].as_str().unwrap().contains(why);
    let r = refusal(&["Nope::nothing", "--json", "{}"]);
    assert!(malformed(&r, "unknown_member"), "{r}");
    let r = refusal(&["Orders::cancel", "--json", "{\"order\":\"x\"}"]);
    assert!(malformed(&r, "wrong_type"), "{r}");
    let r = refusal(&["Orders::cancel", "--json", "{}"]);
    assert!(malformed(&r, "missing_field"), "{r}");
    let r = refusal(&["Orders::cancel", "--order", "41", "--digest", "fnv1a64:0000000000000000"]);
    assert_eq!((r["kind"].as_str(), r["served"].as_str()), (Some("digest_mismatch"), Some(ADMIN_DIGEST)), "{r}");

    // server error: the handler violated a closure, exit 3, and the wire says nothing more
    let o = run(&["api", "call", &u, "Ledger::rebalance", "--book", ""]);
    assert_eq!(o.code, 3, "{}", o.stderr);
    assert_eq!(json_of(&o.stderr)["refusal"], serde_json::json!({"kind": "server"}));
}

#[test]
#[cfg_attr(not(target_os = "linux"), ignore = "binds 127.0.0.2 and up: macOS's loopback carries only 127.0.0.1")]
fn call_over_unix_by_a_caller_the_roles_do_not_admit_is_unauthorized() {
    let w = Witness::start_as("uid:4242424");
    // the description shows this caller nothing, and says so
    let d = run(&["api", "describe", &w.unix(), "--json"]);
    assert!(members_of(&json_of(&d.stdout)).is_empty());
    let o = run(&["api", "call", &w.unix(), "Orders::cancel", "--json", "{\"order\":41}"]);
    assert_eq!(o.code, 2, "{}", o.stderr);
    let r = json_of(&o.stderr);
    assert_eq!((r["refusal"]["kind"].as_str(), r["refusal"]["requires"].clone()), (Some("unauthorized"), serde_json::json!(["operator"])));
    // typed flags need the member in the description
    assert_eq!(run(&["api", "call", &w.unix(), "Orders::cancel", "--order", "41"]).code, 5);
}

#[test]
#[cfg_attr(not(target_os = "linux"), ignore = "binds 127.0.0.2 and up: macOS's loopback carries only 127.0.0.1")]
fn call_over_http_prints_each_outcome_with_its_exit_code() {
    let w = Witness::start();
    let h = w.http();
    let call = |args: &[&str], token: Option<&str>| -> Out {
        let mut a = vec!["api", "call", h.as_str()];
        a.extend_from_slice(args);
        if let Some(t) = token {
            a.extend_from_slice(&["--bearer", t]);
        }
        run(&a)
    };
    // result: place, then cancel what was placed
    let o = call(&["Orders::place", "--symbol", "ACME", "--qty", "3", "--limit", "125"], Some("t-alice"));
    assert_eq!((o.code, json_of(&o.stdout)), (0, serde_json::json!({"order": 41, "notional": 375})), "{}", o.stderr);
    let o = call(&["Orders::cancel", "--order", "41"], Some("t-alice"));
    assert_eq!((o.code, json_of(&o.stdout)), (0, serde_json::json!({"order": 41, "was_open": true})), "{}", o.stderr);
    // handler error: 422, the error on stderr
    let o = call(&["Orders::cancel", "--order", "999"], Some("t-alice"));
    assert_eq!(o.code, 1, "{}", o.stderr);
    assert_eq!(json_of(&o.stderr)["code"], "unknown_order");
    let o = call(&["Orders::cancel", "--order", "999", "--raw"], Some("t-alice"));
    assert_eq!((o.code, json_of(&o.stdout)["code"].as_str(), o.stderr.is_empty()), (1, Some("unknown_order"), true));
    // refusals
    let refusal = |args: &[&str], token: Option<&str>| -> Value {
        let o = call(args, token);
        assert_eq!(o.code, 2, "{}", o.stderr);
        json_of(&o.stderr)["refusal"].clone()
    };
    let malformed = |r: &Value, why: &str| r["kind"] == "malformed" && r["reason"].as_str().unwrap().contains(why);
    let r = refusal(&["Nope::nothing", "--json", "{}"], Some("t-alice"));
    assert!(malformed(&r, "unknown_member"), "{r}");
    let r = refusal(&["Orders::cancel", "--json", "{\"order\":\"x\"}"], Some("t-alice"));
    assert!(malformed(&r, "wrong_type"), "{r}");
    let r = refusal(&["Orders::cancel", "--json", "{}"], Some("t-alice"));
    assert!(malformed(&r, "missing_field"), "{r}");
    // bob holds no role: cancel is not in his description, and the server refuses it all the same
    let r = refusal(&["Orders::cancel", "--json", "{\"order\":41}"], Some("t-bob"));
    assert_eq!((r["kind"].as_str(), r["requires"].clone()), (Some("unauthorized"), serde_json::json!(["trader"])));
    // no bearer, and a token the source names nobody for
    assert_eq!(refusal(&["Orders::cancel", "--json", "{\"order\":41}"], None)["kind"], "unauthenticated");
    assert_eq!(refusal(&["Orders::cancel", "--json", "{\"order\":41}"], Some("t-nobody"))["kind"], "unauthenticated");
    // typed flags with no bearer: the description is refused, which is the outcome
    let o = call(&["Orders::cancel", "--order", "41"], None);
    assert_eq!(o.code, 2, "{}", o.stderr);
    // a stale digest
    let r = refusal(&["Orders::cancel", "--order", "41", "--digest", "fnv1a64:0000000000000000"], Some("t-alice"));
    assert_eq!((r["kind"].as_str(), r["served"].as_str()), (Some("digest_mismatch"), Some(PUBLIC_DIGEST)));
    // the bearer from the environment
    let o = run_env(&["api", "call", &h, "Orders::place", "--symbol", "ACME", "--qty", "1", "--limit", "5"], &[("HALE_API_BEARER", "t-bob")]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    // server error: the position limit
    let o = call(&["Orders::place", "--symbol", "ACME", "--qty", "20000", "--limit", "1"], Some("t-alice"));
    assert_eq!(o.code, 3, "{}", o.stderr);
    assert_eq!(json_of(&o.stderr)["refusal"], serde_json::json!({"kind": "server"}));
}

/// `hale describe` and `hale call` are the `api` verbs under their old names: a bare path is
/// `unix:<path>`, a bare payload after the member is `--json`, and nothing else differs.
#[test]
#[cfg_attr(not(target_os = "linux"), ignore = "binds 127.0.0.2 and up: macOS's loopback carries only 127.0.0.1")]
fn the_short_forms_are_the_api_verbs_with_two_conveniences() {
    let w = Witness::start();
    let (bare, u) = (w.sock.clone(), w.unix());
    let same = |short: &[&str], long: &[&str]| {
        let (s, l) = (run(short), run(long));
        // (the wire names the connecting process and a request counter: each client is its own peer)
        let mask = |t: &str| {
            let mut t = t.to_string();
            for key in ["\"pid\":", "\"request_id\":", "\"id\":\"hale-"] {
                let mut from = 0;
                while let Some(at) = t[from..].find(key) {
                    let start = from + at + key.len();
                    let end = start + t[start..].find(|c: char| !c.is_ascii_digit()).unwrap_or(t.len() - start);
                    t.replace_range(start..end, "");
                    from = start;
                }
            }
            t
        };
        assert_eq!((s.code, mask(&s.stdout), mask(&s.stderr)), (l.code, mask(&l.stdout), mask(&l.stderr)), "{short:?} against {long:?}");
        s
    };
    let book = r#"{"book": "desk"}"#;

    // describe: the document, and the readable rendering
    let o = same(&["describe", &bare, "--json"], &["api", "describe", &u, "--json"]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert_eq!(json_of(&o.stdout)["digest"], ADMIN_DIGEST);
    same(&["describe", &bare], &["api", "describe", &u]);
    same(&["describe", &u], &["api", "describe", &u]);

    // a result: by typed flag, by a bare payload, by --json
    let o = same(&["call", &bare, "Ledger::rebalance", "--book", "desk"], &["api", "call", &u, "Ledger::rebalance", "--book", "desk"]);
    assert_eq!((o.code, json_of(&o.stdout)), (0, serde_json::json!({"moved": 1500})), "{}", o.stderr);
    let o = same(&["call", &bare, "Ledger::rebalance", book], &["api", "call", &u, "Ledger::rebalance", "--json", book]);
    assert_eq!((o.code, json_of(&o.stdout)), (0, serde_json::json!({"moved": 1500})), "{}", o.stderr);
    // (--raw carries the server's request counter, which differs per call: check its shape)
    let o = run(&["call", &bare, "Ledger::rebalance", book, "--raw", "--request-id", "c-1"]);
    assert_eq!((o.code, json_of(&o.stdout)["id"].clone()), (0, serde_json::json!("c-1")), "{}", o.stderr);

    // a refusal and a handler error
    let bad = r#"{"order":"x"}"#;
    let o = same(&["call", &bare, "Orders::cancel", bad], &["api", "call", &u, "Orders::cancel", "--json", bad]);
    assert_eq!(o.code, 2, "{}", o.stderr);
    let o = same(&["call", &bare, "Orders::cancel", "--order", "999"], &["api", "call", &u, "Orders::cancel", "--order", "999"]);
    assert_eq!(o.code, 1, "{}", o.stderr);
    // usage
    let o = same(&["call", &bare], &["api", "call", &u]);
    assert_eq!(o.code, 5);

    // a dead endpoint, and a transport this client does not drive
    let dead = format!("{}.gone", w.sock);
    let o = same(&["describe", &dead], &["api", "describe", &format!("unix:{dead}")]);
    assert_eq!(o.code, 4, "{}", o.stderr);
    let o = same(&["call", &dead, "Ledger::rebalance", "{}"], &["api", "call", &format!("unix:{dead}"), "Ledger::rebalance", "--json", "{}"]);
    assert_eq!(o.code, 4, "{}", o.stderr);
    let o = same(&["describe", "grpc://127.0.0.1:1"], &["api", "describe", "grpc://127.0.0.1:1"]);
    assert_eq!(o.code, 5, "{}", o.stderr);
}

/// spec/api.md § The contract digest, additive compatibility: a member's
/// digest is its row's alone, so adding a member to a surface moves the
/// surface's digest and leaves the old member's; a generated client names
/// its member's digest on each call; and the exposure admits a request
/// naming the member's digest or the surface's, and refuses the old
/// surface's (`digest_mismatch`), which no longer describes what it serves.
#[test]
fn a_client_of_a_member_is_admitted_after_the_surface_gains_one() {
    let dir = unique_dir("additive");
    let source = |extra: bool| {
        format!(
            r#"type Ask {{ id: Int; }}
api Shop {{ rpc Orders::get;{} }}
locus Tokens {{
    fn principal(token: String) -> std::api::Principal {{ return std::api::Principal {{ mode: "bearer", name: token }}; }}
    fn refused() -> String {{ return "no such token"; }}
}}
locus Orders {{
    fn get(a: Ask) -> Int {{ return a.id; }}
    fn count(a: Ask) -> Int {{ return 1; }}
}}
main locus App {{
    params {{
        bearer: Tokens = Tokens {{ }};
        orders: Orders = Orders {{ }};
        web: api::Exposure = api::serve(Shop, http::Rpc {{ bind: std::env::var("BIND"), codec: json, principals: self.bearer }}, as: "web", receivers: {{ Orders: self.orders }}, bound: 8, on_full: refuse);
    }}
    run() {{
        let trigger = std::env::var("TRIGGER");
        while !self.draining && (std::io::fs::read_file(trigger) or "") == "" {{ std::time::sleep(10ms); }}
        self.web.stop();
    }}
}}
fn main() {{ App {{ }}; }}
"#,
            if extra { " rpc Orders::count;" } else { "" }
        )
    };
    let (v1, v2) = (dir.join("v1.hl"), dir.join("v2.hl"));
    std::fs::write(&v1, source(false)).unwrap();
    std::fs::write(&v2, source(true)).unwrap();
    let inventory = |file: &Path| {
        let out = hale().args(["check", "--api"]).arg(file).output().expect("hale check");
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let inv = json_of(&String::from_utf8_lossy(&out.stdout));
        let s = inv["surfaces"][0].clone();
        let get = s["members"].as_array().unwrap().iter().find(|m| m["name"] == "Orders::get").unwrap()["digest"].as_str().unwrap().to_string();
        (s["digest"].as_str().unwrap().to_string(), get)
    };
    let (old_surface, old_get) = inventory(&v1);
    let (new_surface, new_get) = inventory(&v2);
    assert_ne!(old_surface, new_surface, "a member added moves the surface's digest");
    assert_eq!(old_get, new_get, "and leaves the other member's");

    // the old surface's client names its member's digest on the call
    let client = hale().args(["api", "client", "--surface", "Shop", "--lang", "ts"]).arg(&v1).output().expect("hale api client");
    assert!(client.status.success(), "{}", String::from_utf8_lossy(&client.stderr));
    let ts = String::from_utf8_lossy(&client.stdout);
    assert!(ts.contains(&format!("apiCall(opts, \"Orders::get\", \"{old_get}\"")), "{ts}");

    let bin = dir.join("shop");
    let out = hale().arg("build").arg(&v2).arg("-o").arg(&bin).output().expect("hale build");
    assert!(out.status.success(), "build: {}", String::from_utf8_lossy(&out.stderr));
    struct Running(Child);
    impl Drop for Running {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut child = Running(
        Command::new(&bin)
            .env("BIND", format!("{BIND_HOST}:0"))
            .env("TRIGGER", dir.join("stop"))
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("run the program"),
    );
    let port = child::tcp_port_on(&mut child.0, BIND_HOST);
    let endpoint = format!("http://{BIND_HOST}:{port}");
    let call = |digest: &str| run(&["api", "call", &endpoint, "Orders::get", "--bearer", "x", "--json", "{\"id\":7}", "--digest", digest]);
    let member = call(&old_get);
    assert_eq!((member.code, member.stdout.trim()), (0, "7"), "the member's digest is admitted: {}", member.stderr);
    let current = call(&new_surface);
    assert_eq!(current.code, 0, "the surface's digest is admitted: {}", current.stderr);
    let stale = call(&old_surface);
    assert_eq!(stale.code, 2, "the old surface's digest is refused: {} {}", stale.stdout, stale.stderr);
    assert!(stale.stderr.contains("digest_mismatch"), "{}", stale.stderr);
    let served = run(&["api", "describe", &endpoint, "--json", "--bearer", "x"]);
    let doc = json_of(&served.stdout);
    let listed = doc["members"].as_array().unwrap().iter().find(|m| m["name"] == "Orders::get").unwrap()["digest"].clone();
    assert_eq!(listed, serde_json::json!(old_get), "the served description lists the member's digest");

    drop(child);
    let _ = std::fs::remove_dir_all(&dir);
}

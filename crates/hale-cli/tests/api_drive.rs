//! `hale api describe` and `hale api call` drive a served program from the
//! description it serves (spec/api.md § Driving a served program).
//!
//! The witness, `tests/api-contract/program.hl`, is built once per process
//! with the addresses a test cannot fix (the two HTTP binds, the Unix path, the
//! hub's bind, the Unix operator) read from the environment, and run by each
//! test that drives it. What the verbs print is held to what the program
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
        "        std::api::run_until_stopped(public);\n",
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
        let bin = built().clone();
        let dir = unique_dir("run");
        let sock = dir.join("a.sock");
        assert!(sock.as_os_str().len() < 100, "the socket path is too long for sockaddr_un");
        let (p1, p2) = (free_port(), free_port());
        let child = Command::new(&bin)
            .env("BIND", format!("127.0.0.1:{p1}"))
            .env("BIND2", format!("127.0.0.1:{p2}"))
            .env("HUB", format!("127.0.0.1:{}", free_port()))
            .env("SOCK", &sock)
            .env("OPERATOR", format!("uid:{}", uid()))
            .env("TRIGGER", dir.join("stop"))
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("run the witness");
        let w = Witness { child, dir, sock: sock.display().to_string(), public: format!("127.0.0.1:{p1}"), partner: format!("127.0.0.1:{p2}") };
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

    let closed = run(&["api", "describe", &format!("http://127.0.0.1:{}", free_port())]);
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

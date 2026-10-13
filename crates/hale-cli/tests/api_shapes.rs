//! The shapes a payload carries beyond scalars and records (spec/api.md
//! § Codecs), served over HTTP and driven by the operator's verbs and the
//! generated clients.
//!
//! The fixture under `fixtures/api_shapes/` serves `Shelves`, whose records
//! hold lists (`bounded[Book; 3]`, `bounded[Int; 4]`, `[Int; 2]`), an instant
//! and a `Duration`. It is built
//! once per process and run by each test that drives it; its listener binds
//! port 0 and the test reads back the port it was given.

#[path = "../../hale-codegen/tests/support/child.rs"]
mod child;

use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

fn hale() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_hale"));
    c.env("HALE_SKIP_STALE_CHECK", "1").env_remove("HALE_API_BEARER");
    c
}

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/api_shapes/main.hl")
}

fn unique_dir(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!("hale_api_shapes_{}_{}_{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed), tag));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn built() -> &'static PathBuf {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| {
        let bin = unique_dir("build").join("shelves");
        let out = hale().arg("build").arg(fixture()).arg("-o").arg(&bin).output().expect("hale build");
        assert!(out.status.success(), "build: {}", String::from_utf8_lossy(&out.stderr));
        bin
    })
}

/// The fixture, running; dropping it kills it.
struct Served {
    child: Child,
    endpoint: String,
}

impl Served {
    fn start() -> Served {
        let mut child = Command::new(built()).stdout(Stdio::null()).stderr(Stdio::inherit()).spawn().expect("run the fixture");
        let port = child::tcp_port(&mut child);
        let start = Instant::now();
        while TcpStream::connect(("127.0.0.1", port)).is_err() {
            assert!(start.elapsed() < Duration::from_secs(30), "the fixture never began listening");
            std::thread::sleep(Duration::from_millis(20));
        }
        Served { child, endpoint: format!("http://127.0.0.1:{port}") }
    }
}

impl Drop for Served {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Out {
    code: i32,
    stdout: String,
    stderr: String,
}

fn run(args: &[&str]) -> Out {
    let o = hale().args(args).env("HALE_API_BEARER", "t-reader").output().expect("run hale");
    Out { code: o.status.code().expect("exited"), stdout: String::from_utf8_lossy(&o.stdout).into_owned(), stderr: String::from_utf8_lossy(&o.stderr).into_owned() }
}

fn json_of(s: &str) -> Value {
    serde_json::from_str(s.trim()).unwrap_or_else(|e| panic!("{e}: {s}"))
}

#[test]
fn a_list_is_described_as_an_array_with_its_bound() {
    let s = Served::start();
    let d = run(&["api", "describe", &s.endpoint, "--json"]);
    assert_eq!(d.code, 0, "{}", d.stderr);
    let doc = json_of(&d.stdout);
    assert_eq!(doc["schemas"]["Shelf"]["properties"]["books"], json!({"type": "array", "items": {"$ref": "#/schemas/Book"}, "maxItems": 3}));
    assert_eq!(doc["schemas"]["Corner"]["properties"]["at"], json!({"type": "array", "items": {"type": "integer"}, "minItems": 2, "maxItems": 2}));
    // a bounded list may be left out (it is empty); a fixed array may not
    assert_eq!(doc["schemas"]["Shelf"]["required"], json!(["label"]));
    assert_eq!(doc["schemas"]["Corner"]["required"], json!(["at"]));
    // the served document is the compiler's, as the export writes it
    let dir = unique_dir("export");
    let e = hale().args(["api", "export", "--surface", "Shelves", "--out"]).arg(&dir).arg(fixture()).output().expect("export");
    assert!(e.status.success(), "{}", String::from_utf8_lossy(&e.stderr));
    let exported = json_of(&std::fs::read_to_string(dir.join("Shelves.description.json")).unwrap());
    assert_eq!(exported["schemas"], doc["schemas"], "the served schemas are the exported ones");
    let proto = std::fs::read_to_string(dir.join("Shelves.proto")).unwrap();
    assert!(proto.contains("repeated Book books = 1; // Hale: at most 3"), "{proto}");
    assert!(proto.contains("repeated int64 at = 1; // Hale: exactly 2"), "{proto}");
    let _ = std::fs::remove_dir_all(&dir);
    let r = run(&["api", "describe", &s.endpoint]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(r.stdout.contains("Library::put(label: String, books?: [Book; at most 3]) -> Shelf"), "{}", r.stdout);
    assert!(r.stdout.contains("Library::corner(at: [Int; 2]) -> Corner"), "{}", r.stdout);
    assert!(r.stdout.contains("Library::stamp(at: String (Time), every: Int (Duration, q(ns))) -> Stamp"), "{}", r.stdout);
    assert_eq!(doc["schemas"]["Stamp"]["properties"]["at"], json!({"type": "string", "format": "date-time", "x-hale-type": "Time"}));
    assert_eq!(doc["schemas"]["Stamp"]["properties"]["every"], json!({"type": "integer", "x-hale-type": "Duration", "x-hale-unit": "q(ns)"}));
}

#[test]
fn an_instant_crosses_http_as_its_rfc_3339_text_and_a_duration_as_its_nanoseconds() {
    let s = Served::start();
    let o = run(&["api", "call", &s.endpoint, "Library::stamp", "--at", "2026-10-12T03:04:05.123456789Z", "--every", "1000000001"]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert_eq!(json_of(&o.stdout), json!({"at": "2026-10-12T03:04:06.12345679Z", "every": 2000000002}));
    for (at, what) in [("2026-10-12T03:04:05", "no zone"), ("2026-10-12T03:04:05+01:00", "an offset"), ("2999-01-01T00:00:00Z", "past Time's range")] {
        let o = run(&["api", "call", &s.endpoint, "Library::stamp", "--at", at, "--every", "1"]);
        assert_eq!(o.code, 2, "{what}: {}", o.stderr);
        assert_eq!(json_of(&o.stderr)["refusal"], json!({"kind": "malformed", "reason": "wrong_type: at"}), "{what}");
    }
}

#[test]
fn a_list_crosses_http_both_ways_and_one_too_long_is_refused_not_truncated() {
    let s = Served::start();
    let books = r#"[{"id":1,"title":"Dune"},{"id":2,"title":"Emma"}]"#;
    let put = run(&["api", "call", &s.endpoint, "Library::put", "--books", books, "--label", "front"]);
    assert_eq!(put.code, 0, "{}", put.stderr);
    assert_eq!(json_of(&put.stdout), json!({"books": [{"id": 1, "title": "Dune"}, {"id": 2, "title": "Emma"}], "label": "front"}));
    let found = run(&["api", "call", &s.endpoint, "Library::find", "--ids", "[2,9]"]);
    assert_eq!(found.code, 0, "{}", found.stderr);
    assert_eq!(json_of(&found.stdout), json!({"books": [{"id": 2, "title": "Emma"}], "label": "found"}));
    // left out, a bounded list is empty
    let none = run(&["api", "call", &s.endpoint, "Library::find", "--json", "{}"]);
    assert_eq!(none.code, 0, "{}", none.stderr);
    assert_eq!(json_of(&none.stdout), json!({"books": [], "label": "found"}));
    // the typed flag holds the bound before anything is sent
    let flagged = run(&["api", "call", &s.endpoint, "Library::find", "--ids", "[1,2,3,4,5]"]);
    assert_eq!(flagged.code, 5, "{}", flagged.stderr);
    assert!(flagged.stderr.contains("5 items, more than the 4 the list holds"), "{}", flagged.stderr);
    // the server holds it too: a payload over the bound is a refusal, never a shorter list
    let five = run(&["api", "call", &s.endpoint, "Library::find", "--json", r#"{"ids":[1,2,3,4,5]}"#]);
    assert_eq!(five.code, 2, "{}", five.stderr);
    assert_eq!(json_of(&five.stderr)["refusal"], json!({"kind": "malformed", "reason": "too_long: ids"}));
    let short = run(&["api", "call", &s.endpoint, "Library::corner", "--json", r#"{"at":[1]}"#]);
    assert_eq!(short.code, 2, "{}", short.stderr);
    assert_eq!(json_of(&short.stderr)["refusal"], json!({"kind": "malformed", "reason": "too_short: at"}));
    let corner = run(&["api", "call", &s.endpoint, "Library::corner", "--at", "[3,-4]"]);
    assert_eq!(corner.code, 0, "{}", corner.stderr);
    assert_eq!(json_of(&corner.stdout), json!({"at": [3, -4]}));
}

/// What a program of the generated Hale client does with the served fixture.
const HALE_CALLER: &str = r#"
fn main() {
    let ep = std::env::var("ENDPOINT");
    let mut s = Shelf { label: "back" };
    push(s.books, Book { id: 7, title: "Kim" }) or discard;
    push(s.books, Book { id: 8, title: "Emma" }) or discard;
    match library_put(ep, "t-reader", s) {
        LibraryPutOutcome::Result(v) -> { println(api_enc_Shelf(v)); },
        _ -> { println("put: another outcome"); },
    }
    let mut f = Find { };
    push(f.ids, 8) or discard;
    match library_find(ep, "t-reader", f) {
        LibraryFindOutcome::Result(v) -> { println(to_string(count(v.books)) + " " + api_enc_Shelf(v)); },
        _ -> { println("find: another outcome"); },
    }
    match library_corner(ep, "t-reader", Corner { at: [5, -6] }) {
        LibraryCornerOutcome::Result(v) -> { println(api_enc_Corner(v)); },
        _ -> { println("corner: another outcome"); },
    }
    match library_stamp(ep, "t-reader", Stamp { at: `2026-10-12T03:04:05.5Z`, every: 250 }) {
        LibraryStampOutcome::Result(v) -> { println(std::time::iso8601(v.at) + " " + to_string(v.every)); },
        _ -> { println("stamp: another outcome"); },
    }
}
"#;

/// The generated clients type a list and an instant: the Hale client checks
/// (`hale api client` refuses one that does not) and carries both, both ways,
/// against the served fixture, and the TypeScript one compiles under `tsc
/// --strict` where a compiler is installed.
#[test]
fn the_generated_clients_carry_a_list_and_an_instant() {
    let dir = unique_dir("clients");
    let hl = dir.join("client.hl");
    let o = hale().args(["api", "client", "--surface", "Shelves", "--lang", "hale", "--out"]).arg(&hl).arg(fixture()).output().expect("hale client");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let text = std::fs::read_to_string(&hl).unwrap();
    assert!(text.contains("books: bounded[Book; 3];") && text.contains("at: [Int; 2];"), "{text}");
    let caller = dir.join("caller.hl");
    std::fs::write(&caller, text + HALE_CALLER).unwrap();
    let bin = dir.join("caller");
    let b = hale().arg("build").arg(&caller).arg("-o").arg(&bin).output().expect("build the caller");
    assert!(b.status.success(), "{}", String::from_utf8_lossy(&b.stderr));
    let s = Served::start();
    let r = Command::new(&bin).env("ENDPOINT", &s.endpoint).output().expect("run the caller");
    assert_eq!(
        String::from_utf8_lossy(&r.stdout),
        "{\"books\":[{\"id\":7,\"title\":\"Kim\"},{\"id\":8,\"title\":\"Emma\"}],\"label\":\"back\"}\n\
         1 {\"books\":[{\"id\":8,\"title\":\"Emma\"}],\"label\":\"found\"}\n\
         {\"at\":[5,-6]}\n\
         2026-10-12T03:04:05.50000025Z 500\n",
        "{}",
        String::from_utf8_lossy(&r.stderr)
    );
    let ts = dir.join("client.ts");
    let o = hale().args(["api", "client", "--surface", "Shelves", "--lang", "ts", "--out"]).arg(&ts).arg(fixture()).output().expect("ts client");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let text = std::fs::read_to_string(&ts).unwrap();
    assert!(text.contains("books: __Array<Book>;") && text.contains("at: __Array<number>;"), "{text}");
    assert!(text.contains("export interface Stamp {\n  at: string;\n  every: number;\n}"), "an instant is its text: {text}");
    let tsc = std::env::var("TSC").unwrap_or_else(|_| "tsc".to_string());
    if Command::new(&tsc).arg("--version").output().is_ok_and(|o| o.status.success()) {
        let c = Command::new(&tsc).args(["--strict", "--target", "es2022", "--lib", "es2022,dom", "--noEmit"]).arg(&ts).output().expect("tsc");
        assert!(c.status.success(), "tsc refused the client:\n{}{}", String::from_utf8_lossy(&c.stdout), String::from_utf8_lossy(&c.stderr));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

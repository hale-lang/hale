//! The shapes a payload carries beyond scalars and records (spec/api.md
//! § Codecs), served over HTTP and driven by the operator's verbs and the
//! generated clients.
//!
//! The fixture under `fixtures/api_shapes/` serves `Shelves`, whose records
//! hold lists (`bounded[Book; 3]`, `bounded[Int; 4]`, `[Int; 2]`), an instant
//! and a `Duration`, optional fields, enums and wire maps. It is built
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
    // an optional map: the map's schema or null; its flag a field of its own
    let map = json!({"type": "object", "additionalProperties": {"type": "object", "properties": {"status": {"type": "string"}}, "required": ["status"]}, "maxProperties": 2, "x-hale-key": "name"});
    assert_eq!(doc["schemas"]["Board"]["properties"]["extra"], json!({"anyOf": [map, {"type": "null"}]}));
    assert!(proto.contains("map<string, Check> extra = 5;") && proto.contains("optional bool has_extra = 6;"), "{proto}");
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
        LibraryPutOutcome::Result(v) -> { println(Shelf::api_encode(v)); },
        _ -> { println("put: another outcome"); },
    }
    let mut f = Find { };
    push(f.ids, 8) or discard;
    match library_find(ep, "t-reader", f) {
        LibraryFindOutcome::Result(v) -> { println(to_string(count(v.books)) + " " + Shelf::api_encode(v)); },
        _ -> { println("find: another outcome"); },
    }
    match library_corner(ep, "t-reader", Corner { at: [5, -6] }) {
        LibraryCornerOutcome::Result(v) -> { println(Corner::api_encode(v)); },
        _ -> { println("corner: another outcome"); },
    }
    match library_stamp(ep, "t-reader", Stamp { at: `2026-10-12T03:04:05.5Z`, every: 250 }) {
        LibraryStampOutcome::Result(v) -> { println(std::time::iso8601(v.at) + " " + to_string(v.every)); },
        _ -> { println("stamp: another outcome"); },
    }
    let mut n = Note { };
    push(n.tags, "a") or discard;
    n.has_tags = true;
    match library_note(ep, "t-reader", n) {
        LibraryNoteOutcome::Result(v) -> { println(Note::api_encode(v)); },
        _ -> { println("note: another outcome"); },
    }
    match library_note(ep, "t-reader", Note { text: "kept", has_text: true }) {
        LibraryNoteOutcome::Result(v) -> { println(Note::api_encode(v) + " " + (if v.has_tags { "tags" } else { "no tags" })); },
        _ -> { println("note: another outcome"); },
    }
    let mut b = Board { mood: Mood::Busy, shape: Shape::Rect(4, 5) };
    push(b.moods, Mood::Calm) or discard;
    push(b.checks, Check { name: "db", status: "up" }) or discard;
    push(b.checks, Check { name: "cache", status: "down" }) or discard;
    match library_board(ep, "t-reader", b) {
        LibraryBoardOutcome::Result(v) -> { println(Board::api_encode(v) + " " + to_string(count(v.checks))); },
        _ -> { println("board: another outcome"); },
    }
    let mut spare = Board { mood: Mood::Calm, shape: Shape::Boxed(Book { id: 3, title: "Kim" }), has_extra: true };
    push(spare.extra, Check { name: "x", status: "ok" }) or discard;
    match library_board(ep, "t-reader", spare) {
        LibraryBoardOutcome::Result(v) -> { println(Board::api_encode(v)); },
        _ -> { println("board: another outcome"); },
    }
    match library_flip(ep, "t-reader", Mood::Busy) {
        LibraryFlipOutcome::Result(v) -> { println(Shape::api_encode(v)); },
        _ -> { println("flip: another outcome"); },
    }
}
"#;

/// The generated clients type a list, an instant, an optional field, an enum
/// and a wire map: the Hale client checks (`hale api client` refuses one that
/// does not) and carries each, both ways, against the served fixture, and the
/// TypeScript one compiles under `tsc --strict` where a compiler is installed.
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
         2026-10-12T03:04:05.50000025Z 500\n\
         {\"text\":\"none\",\"tags\":[\"a\"]}\n\
         {\"text\":\"kept\"} no tags\n\
         {\"mood\":\"Busy\",\"shape\":{\"Rect\":[4,5]},\"moods\":[\"Calm\"],\"checks\":{\"db\":{\"status\":\"up\"},\"cache\":{\"status\":\"down\"}},\"extra\":{}} 2\n\
         {\"mood\":\"Calm\",\"shape\":{\"Boxed\":{\"id\":3,\"title\":\"Kim\"}},\"moods\":[],\"checks\":{},\"extra\":{\"x\":{\"status\":\"ok\"}}}\n\
         {\"Rect\":[2,-3]}\n",
        "{}",
        String::from_utf8_lossy(&r.stderr)
    );
    let ts = dir.join("client.ts");
    let o = hale().args(["api", "client", "--surface", "Shelves", "--lang", "ts", "--out"]).arg(&ts).arg(fixture()).output().expect("ts client");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let text = std::fs::read_to_string(&ts).unwrap();
    assert!(text.contains("books: __Array<Book>;") && text.contains("at: __Array<number>;"), "{text}");
    assert!(text.contains("export interface Stamp {\n  at: string;\n  every: number;\n}"), "an instant is its text: {text}");
    assert!(text.contains("export interface Note {\n  text?: string;\n  tags?: __Array<string>;\n}"), "an optional field is an optional property: {text}");
    assert!(
        text.contains("export type Shape = \"Empty\" | { Circle: number } | { Rect: [number, number] } | { Boxed: Book } | { Feel: Mood };"),
        "an enum is the union of its variants: {text}"
    );
    assert!(text.contains("export type Mood = \"Calm\" | \"Busy\";"), "{text}");
    assert!(text.contains("  checks: __Array<Check>;\n  extra?: __Array<Check>;\n"), "a map is its records: {text}");
    let tsc = std::env::var("TSC").unwrap_or_else(|_| "tsc".to_string());
    if Command::new(&tsc).arg("--version").output().is_ok_and(|o| o.status.success()) {
        let c = Command::new(&tsc).args(["--strict", "--target", "es2022", "--lib", "es2022,dom", "--noEmit"]).arg(&ts).output().expect("tsc");
        assert!(c.status.success(), "tsc refused the client:\n{}{}", String::from_utf8_lossy(&c.stdout), String::from_utf8_lossy(&c.stderr));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// What a page does with the served fixture through the JavaScript client.
const JS_CALLER: &str = r#"import * as c from "./client.js";
const opts = { endpoint: process.env.ENDPOINT, bearer: "t-reader" };
const out = [];
out.push(await c.libraryPut(opts, { books: [{ id: 7, title: "Kim" }, { id: 8, title: "Emma" }], label: "back" }));
out.push(await c.libraryFind(opts, { ids: [8] }));
out.push(await c.libraryCorner(opts, { at: [5, -6] }));
out.push(await c.libraryStamp(opts, { at: "2026-10-12T03:04:05.5Z", every: 250 }));
out.push(await c.libraryFind(opts, { ids: [1, 2, 3, 4, 5] }));
out.push(await c.libraryNote(opts, {}));
out.push(await c.libraryNote(opts, { text: "kept", tags: [] }));
out.push(await c.libraryBoard(opts, { mood: "Busy", shape: { Rect: [4, 5] }, moods: ["Calm"], checks: [{ name: "db", status: "up" }, { name: "cache", status: "down" }], extra: [{ name: "x", status: "ok" }] }));
out.push(await c.libraryBoard(opts, { mood: "Calm", shape: "Empty", moods: [], checks: [] }));
out.push(await c.libraryFlip(opts, "Calm"));
out.push(await c.libraryFlip(opts, "Busy"));
for (const o of out) console.log(JSON.stringify(o));
"#;

/// The JavaScript client (`--lang js`) checks under `tsc --checkJs --strict`
/// where a compiler is installed, carries a list, an instant, an enum and a
/// wire map (its records, keyed on the wire) against the
/// served fixture on node's `fetch`, and a list over its bound comes back the
/// server's refusal. It needs `node`; without it the test says so and
/// passes, unless `HALE_REQUIRE_TS_CLIENT=1` (the face-browser job's
/// `run.mjs` runs the JavaScript client against the contract's recordings).
#[test]
fn the_javascript_client_carries_a_list_and_an_instant() {
    if !Command::new("node").arg("--version").output().is_ok_and(|o| o.status.success()) {
        assert!(std::env::var("HALE_REQUIRE_TS_CLIENT").map_or(true, |v| v != "1"), "HALE_REQUIRE_TS_CLIENT=1, and node is not installed");
        eprintln!("skipped: node is needed (the face-browser job runs the JavaScript client)");
        return;
    }
    let dir = unique_dir("js");
    let o = hale().args(["api", "client", "--surface", "Shelves", "--lang", "js", "--out"]).arg(dir.join("client.js")).arg(fixture()).output().expect("js client");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    std::fs::write(dir.join("package.json"), "{\"type\":\"module\"}\n").unwrap();
    std::fs::write(dir.join("caller.mjs"), JS_CALLER).unwrap();
    let tsc = std::env::var("TSC").unwrap_or_else(|_| "tsc".to_string());
    if Command::new(&tsc).arg("--version").output().is_ok_and(|o| o.status.success()) {
        let c = Command::new(&tsc)
            .args(["--allowJs", "--checkJs", "--strict", "--noEmit", "--target", "es2022", "--lib", "es2022,dom", "--module", "es2022", "--moduleResolution", "bundler"])
            .arg(dir.join("client.js"))
            .output()
            .expect("tsc");
        assert!(c.status.success(), "tsc --checkJs refused the client:\n{}{}", String::from_utf8_lossy(&c.stdout), String::from_utf8_lossy(&c.stderr));
    }
    let s = Served::start();
    let r = Command::new("node").arg(dir.join("caller.mjs")).env("ENDPOINT", &s.endpoint).output().expect("run node");
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let lines: Vec<Value> = String::from_utf8_lossy(&r.stdout).lines().map(json_of).collect();
    assert_eq!(
        lines,
        [
            json!({"kind": "result", "value": {"books": [{"id": 7, "title": "Kim"}, {"id": 8, "title": "Emma"}], "label": "back"}}),
            json!({"kind": "result", "value": {"books": [{"id": 8, "title": "Emma"}], "label": "found"}}),
            json!({"kind": "result", "value": {"at": [5, -6]}}),
            json!({"kind": "result", "value": {"at": "2026-10-12T03:04:05.50000025Z", "every": 500}}),
            json!({"kind": "refusal", "refusal": {"kind": "malformed", "reason": "too_long: ids", "requires": []}}),
            json!({"kind": "result", "value": {"text": "none"}}),
            json!({"kind": "result", "value": {"text": "kept", "tags": []}}),
            json!({"kind": "result", "value": {
                "mood": "Busy", "shape": {"Rect": [4, 5]}, "moods": ["Calm"],
                "checks": [{"name": "db", "status": "up"}, {"name": "cache", "status": "down"}],
                "extra": [{"name": "x", "status": "ok"}]
            }}),
            json!({"kind": "result", "value": {"mood": "Calm", "shape": "Empty", "moods": [], "checks": [], "extra": []}}),
            json!({"kind": "result", "value": "Empty"}),
            json!({"kind": "result", "value": {"Rect": [2, -3]}}),
        ]
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The Hale client pairs a presence flag with its field by declaration and
/// names both through one allocator: a flag renamed on the wire (`json:
/// "present"`) still pairs, so an empty value encodes as `{}`; and a flag's
/// derived name (`has_type_` for the key `type`) is reserved before an
/// ordinary field that would take it, which is renamed and tagged instead.
#[test]
fn the_hale_client_pairs_a_flag_by_its_declaration() {
    let dir = unique_dir("flags");
    let src = dir.join("flags.hl");
    std::fs::write(
        &src,
        "type Tagged { data: String `json:\"type\"`; has_data: Bool; has_type_: Int; }\n\
         type Renamed { data: String; has_data: Bool `json:\"present\"`; }\n\
         locus L {\n    fn tagged(t: Tagged) -> Tagged { return t; }\n    fn renamed(r: Renamed) -> Renamed { return r; }\n}\n\
         api Flags { rpc L::tagged; rpc L::renamed; }\n\
         locus Nobody { fn holds(p: std::api::Principal, r: String) -> Bool { return false; } }\n\
         main locus App {\n    params {\n        l: L = L { };\n        h: api::Exposure = api::serve(Flags, unix::Rpc { path: \"/tmp/hale-flags.sock\", roles: Nobody { } }, as: \"flags\", receivers: { L: self.l }, bound: 4, on_full: refuse);\n    }\n    run() { }\n}\n\
         fn main() { App { }; }\n",
    )
    .unwrap();
    let hl = dir.join("client.hl");
    let o = hale().args(["api", "client", "--surface", "Flags", "--lang", "hale", "--out"]).arg(&hl).arg(&src).output().expect("hale client");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let text = std::fs::read_to_string(&hl).unwrap();
    assert!(text.contains("type Renamed {\n    data: String = \"\";\n    has_data: Bool = false;\n}"), "the renamed flag pairs: {text}");
    assert!(text.contains("type_: String = \"\" `json:\"type\"`;") && text.contains("has_type_: Bool = false;"), "the tagged field and its flag: {text}");
    assert!(text.contains("has_type__2: Int `json:\"has_type_\"`;"), "the ordinary field gives way: {text}");
    let caller = dir.join("caller.hl");
    std::fs::write(
        &caller,
        text + "\nfn main() {\n    println(Renamed::api_encode(Renamed { }));\n    println(Renamed::api_encode(Renamed { data: \"x\", has_data: true }));\n    println(Tagged::api_encode(Tagged { type_: \"y\", has_type_: true, has_type__2: 3 }));\n}\n",
    )
    .unwrap();
    let r = hale().arg("run").arg(&caller).output().expect("run the caller");
    assert_eq!(
        String::from_utf8_lossy(&r.stdout),
        "{}\n{\"data\":\"x\"}\n{\"type\":\"y\",\"has_type_\":3}\n",
        "{}",
        String::from_utf8_lossy(&r.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A field with a default is optional on the wire, whatever its type, and
/// the description lists it under no `required` (spec/api.md § Codecs).
/// `tests/hale/api/defaults_test.hl` holds the same surface's codecs to it:
/// a payload that leaves each such field out decodes to its default.
#[test]
fn a_field_with_a_default_is_described_optional() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/hale/api/defaults_test.hl");
    let dir = unique_dir("defaults");
    let e = hale().args(["api", "export", "--surface", "Defaults", "--out"]).arg(&dir).arg(&src).output().expect("export");
    assert!(e.status.success(), "{}", String::from_utf8_lossy(&e.stderr));
    let doc = json_of(&std::fs::read_to_string(dir.join("Defaults.description.json")).unwrap());
    // every field of `Rec` but `name` has a default, or is a list
    assert_eq!(doc["schemas"]["Rec"]["required"], json!(["name"]));
    assert_eq!(doc["schemas"]["AllDefault"]["required"], json!([]));
    assert_eq!(doc["schemas"]["Inner"]["required"], json!(["a"]));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A map of values (`indexed_by name -> status`, spec/api.md § Codecs) is
/// described as an object of the value's schema, naming the value
/// (`x-hale-value`), and `map<string, string>` in the `.proto`. The Hale
/// client declares the same map and its codec reads and writes the server's
/// wire (`{"db":"ok"}`); the TypeScript client keeps the records and puts
/// each one's value alone on the wire.
#[test]
fn a_map_of_values_is_described_and_carried_by_the_clients() {
    let dir = unique_dir("value_map");
    let src = dir.join("levels.hl");
    std::fs::write(
        &src,
        "type Level { name: String; status: String; }\n\
         type Gauge { name: String; depth: Int; }\n\
         type Health { checks: bounded[Level; 3] indexed_by name -> status; gauges: bounded[Gauge; 2] indexed_by name -> depth; }\n\
         locus L {\n    fn echo(h: Health) -> Health { return h; }\n}\n\
         api Levels { rpc L::echo; }\n\
         locus Nobody { fn holds(p: std::api::Principal, r: String) -> Bool { return false; } }\n\
         main locus App {\n    params {\n        l: L = L { };\n        h: api::Exposure = api::serve(Levels, unix::Rpc { path: \"/tmp/hale-levels.sock\", roles: Nobody { } }, as: \"levels\", receivers: { L: self.l }, bound: 4, on_full: refuse);\n    }\n    run() { }\n}\n\
         fn main() { App { }; }\n",
    )
    .unwrap();
    let out = dir.join("out");
    let e = hale().args(["api", "export", "--surface", "Levels", "--out"]).arg(&out).arg(&src).output().expect("export");
    assert!(e.status.success(), "{}", String::from_utf8_lossy(&e.stderr));
    let doc = json_of(&std::fs::read_to_string(out.join("Levels.description.json")).unwrap());
    assert_eq!(
        doc["schemas"]["Health"]["properties"]["checks"],
        json!({"type": "object", "additionalProperties": {"type": "string"}, "maxProperties": 3, "x-hale-key": "name", "x-hale-value": "status"})
    );
    assert_eq!(
        doc["schemas"]["Health"]["properties"]["gauges"],
        json!({"type": "object", "additionalProperties": {"type": "integer"}, "maxProperties": 2, "x-hale-key": "name", "x-hale-value": "depth"})
    );
    let proto = std::fs::read_to_string(out.join("Levels.proto")).unwrap();
    assert!(proto.contains("map<string, string> checks = 1;") && proto.contains("map<string, int64> gauges = 2;"), "{proto}");

    let hl = dir.join("client.hl");
    let o = hale().args(["api", "client", "--surface", "Levels", "--lang", "hale", "--out"]).arg(&hl).arg(&src).output().expect("hale client");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let text = std::fs::read_to_string(&hl).unwrap();
    assert!(text.contains("checks: bounded[Level; 3] indexed_by name -> status;"), "the client declares the map of values: {text}");
    let caller = dir.join("caller.hl");
    std::fs::write(
        &caller,
        text + "\nfn main() {\n    let h = Health::api_decode(\"{\\\"checks\\\":{\\\"db\\\":\\\"ok\\\",\\\"nats\\\":\\\"down\\\"},\\\"gauges\\\":{\\\"q\\\":3}}\") or { println(err.kind); return; };\n    println(to_string(count(h.checks)) + \" \" + Health::api_encode(h));\n}\n",
    )
    .unwrap();
    let r = hale().arg("run").arg(&caller).output().expect("run the caller");
    assert_eq!(
        String::from_utf8_lossy(&r.stdout),
        "2 {\"checks\":{\"db\":\"ok\",\"nats\":\"down\"},\"gauges\":{\"q\":3}}\n",
        "{}",
        String::from_utf8_lossy(&r.stderr)
    );

    let ts = dir.join("client.ts");
    let o = hale().args(["api", "client", "--surface", "Levels", "--lang", "ts", "--out"]).arg(&ts).arg(&src).output().expect("ts client");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let text = std::fs::read_to_string(&ts).unwrap();
    assert!(text.contains("checks: __Array<Level>;"), "a map of values is its records: {text}");
    assert!(text.contains("encode_Level, \"status\")") && text.contains("decode_Level, \"status\")"), "each record's value alone on the wire: {text}");
    let tsc = std::env::var("TSC").unwrap_or_else(|_| "tsc".to_string());
    if Command::new(&tsc).arg("--version").output().is_ok_and(|o| o.status.success()) {
        let c = Command::new(&tsc).args(["--strict", "--target", "es2022", "--lib", "es2022,dom", "--noEmit"]).arg(&ts).output().expect("tsc");
        assert!(c.status.success(), "tsc refused the client:\n{}{}", String::from_utf8_lossy(&c.stdout), String::from_utf8_lossy(&c.stderr));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A presence triple (`note` beside `has_note` and `clear_note`, spec/api.md
/// § Codecs) is described as the field's schema or `null`, optional, with
/// `"x-hale-null": "clear"`; protobuf carries its three fields. The Hale
/// client declares the triple and its codec tells left alone, cleared and
/// set apart on the server's wire; the TypeScript client's property may be
/// left out, `null`, or a value, and sends each as it is.
#[test]
fn a_presence_triple_is_described_and_carried_by_the_clients() {
    let dir = unique_dir("triple");
    let src = dir.join("patch.hl");
    std::fs::write(
        &src,
        "type Patch { note: String = \"\"; has_note: Bool = false; clear_note: Bool = false; }\n\
         locus L {\n    fn edit(p: Patch) -> Patch { return p; }\n}\n\
         api Edits { rpc L::edit; }\n\
         locus Nobody { fn holds(p: std::api::Principal, r: String) -> Bool { return false; } }\n\
         main locus App {\n    params {\n        l: L = L { };\n        h: api::Exposure = api::serve(Edits, unix::Rpc { path: \"/tmp/hale-edits.sock\", roles: Nobody { } }, as: \"edits\", receivers: { L: self.l }, bound: 4, on_full: refuse);\n    }\n    run() { }\n}\n\
         fn main() { App { }; }\n",
    )
    .unwrap();
    let out = dir.join("out");
    let e = hale().args(["api", "export", "--surface", "Edits", "--out"]).arg(&out).arg(&src).output().expect("export");
    assert!(e.status.success(), "{}", String::from_utf8_lossy(&e.stderr));
    let doc = json_of(&std::fs::read_to_string(out.join("Edits.description.json")).unwrap());
    assert_eq!(
        doc["schemas"]["Patch"],
        json!({"type": "object", "properties": {"note": {"anyOf": [{"type": "string"}, {"type": "null"}], "x-hale-null": "clear"}}, "required": []})
    );
    let proto = std::fs::read_to_string(out.join("Edits.proto")).unwrap();
    assert!(
        proto.contains("optional string note = 1;") && proto.contains("optional bool has_note = 2;") && proto.contains("optional bool clear_note = 3;"),
        "{proto}"
    );

    let hl = dir.join("client.hl");
    let o = hale().args(["api", "client", "--surface", "Edits", "--lang", "hale", "--out"]).arg(&hl).arg(&src).output().expect("hale client");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let text = std::fs::read_to_string(&hl).unwrap();
    assert!(text.contains("has_note: Bool = false;") && text.contains("clear_note: Bool = false;"), "the client declares the triple: {text}");
    let caller = dir.join("caller.hl");
    std::fs::write(
        &caller,
        text + "\nfn show(t: String) {\n    let p = Patch::api_decode(t) or { println(err.kind); return; };\n    println(to_string(p.has_note) + \" \" + to_string(p.clear_note) + \" \" + Patch::api_encode(p));\n}\n\
                fn main() {\n    show(\"{}\");\n    show(\"{\\\"note\\\":null}\");\n    show(\"{\\\"note\\\":\\\"x\\\"}\");\n}\n",
    )
    .unwrap();
    let r = hale().arg("run").arg(&caller).output().expect("run the caller");
    assert_eq!(
        String::from_utf8_lossy(&r.stdout),
        "false false {}\nfalse true {\"note\":null}\ntrue false {\"note\":\"x\"}\n",
        "{}",
        String::from_utf8_lossy(&r.stderr)
    );

    let ts = dir.join("client.ts");
    let o = hale().args(["api", "client", "--surface", "Edits", "--lang", "ts", "--out"]).arg(&ts).arg(&src).output().expect("ts client");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let text = std::fs::read_to_string(&ts).unwrap();
    assert!(text.contains("note?: string | null;"), "left out, null, or a value: {text}");
    assert!(text.contains("=== null ? null :"), "null is sent and read as null: {text}");
    let tsc = std::env::var("TSC").unwrap_or_else(|_| "tsc".to_string());
    if Command::new(&tsc).arg("--version").output().is_ok_and(|o| o.status.success()) {
        let c = Command::new(&tsc).args(["--strict", "--target", "es2022", "--lib", "es2022,dom", "--noEmit"]).arg(&ts).output().expect("tsc");
        assert!(c.status.success(), "tsc refused the client:\n{}{}", String::from_utf8_lossy(&c.stdout), String::from_utf8_lossy(&c.stderr));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

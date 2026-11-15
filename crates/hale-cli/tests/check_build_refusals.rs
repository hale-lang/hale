//! Shapes `hale check` accepted and `hale build` refused, from codegen,
//! with a message that named lowering rather than the program and, for
//! most, no file or line (downstream handoff). Each is refused by the
//! check now, as `file:line:col` at the construct, and the build of the
//! same file stops at the check's message rather than in codegen; a
//! shape that is lowered instead checks and builds.
//! `corpus_check_build_agreement` holds the same shapes to the
//! agreement sweep's verdict.

use std::path::{Path, PathBuf};
use std::process::Command;

fn hale(args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale")).args(args).current_dir(Path::new("/")).output().expect("hale");
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

/// `hale check` and `hale build` on one file; the scratch dir is removed.
fn check_and_build(src: &str, tag: &str) -> ((bool, String), (bool, String)) {
    let d: PathBuf = std::env::temp_dir().join(format!("hale_check_build_refusals_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let f = d.join("main.hl");
    std::fs::write(&f, src).unwrap();
    let file = f.to_string_lossy().to_string();
    let checked = hale(&["check", &file]);
    let built = hale(&["build", &file, "-o", &d.join("bin").to_string_lossy()]);
    let _ = std::fs::remove_dir_all(&d);
    (checked, built)
}

/// The check refuses `src` at `main.hl:<at>` with `head`, and the build
/// stops at the same message, never reaching codegen.
fn refused(src: &str, tag: &str, at: &str, head: &str) {
    let ((ok, out), (built, bout)) = check_and_build(src, tag);
    assert!(!ok && out.contains(&format!("main.hl:{at}: type error: {head}")), "check: {out}");
    assert!(!built && bout.contains(head), "build: {bout}");
    assert!(!bout.contains("codegen error"), "the build stops at the check: {bout}");
}

#[test]
fn a_locus_held_by_a_type_is_refused_at_the_field() {
    refused(
        "locus M { params { n: Int = 1; } }\ntype Layer { w: M; }\nfn main() { let l = Layer { w: M { } }; println(l.w.n); }\n",
        "locus_field",
        "2:17",
        "`type Layer`: field `w` holds the locus `M`. A `type` is plain data with no lifecycle",
    );
    refused(
        "type Conn { s: std::io::tcp::Stream; }\nfn main() { println(\"x\"); }\n",
        "stdlib_locus_field",
        "1:16",
        "`type Conn`: field `s` holds the locus `std::io::tcp::Stream`.",
    );
}

#[test]
fn an_uncarried_topic_payload_is_refused_where_it_is_delivered() {
    let src = "topic Chunk { payload: String; }\nlocus Sub {\n    bus { subscribe Chunk as on_chunk; }\n    fn on_chunk(s: String) { println(s); }\n}\nlocus Pub {\n    bus { publish Chunk; }\n    run() { Chunk <- \"hi\"; }\n}\nfn main() { let s = Sub { }; Pub { }; }\n";
    let ((ok, out), (built, bout)) = check_and_build(src, "topic");
    assert!(!ok, "{out}");
    assert!(
        out.contains("main.hl:3:11: type error: subscribe `Chunk`: topic `Chunk` declares `payload: String`, which the bus cannot carry.")
            && out.contains("main.hl:8:13: type error: `Chunk <- ...`: topic `Chunk` declares `payload: String`")
            && out.contains("wrap it in a user `type` and declare the topic's `payload:` as that"),
        "{out}"
    );
    assert!(!built && !bout.contains("codegen error"), "{bout}");
}

#[test]
fn an_unlowered_recovery_modifier_is_refused_at_the_statement() {
    let desk = |stmt: &str| {
        format!(
            "locus Book {{\n    params {{ credits: Int = 0; debits: Int = 0; }}\n    closure balanced {{ self.credits ~~ self.debits within 0; }}\n}}\nlocus Desk {{\n    params {{ n: Int = 0; }}\n    on_failure(c: Book, err: ClosureViolation) {{ {stmt} }}\n    run() {{ Book {{ credits: 100, debits: 93 }}; }}\n}}\nfn main() {{ let d = Desk {{ }}; }}\n"
        )
    };
    refused(
        &desk("quarantine(c) for 5s;"),
        "quarantine_for",
        "7:50",
        "`quarantine(c) for d` (an automatic restart after a duration) is specified but not implemented: `quarantine(c)` itself ships, the `for d` clause does not.",
    );
    refused(&desk("quarantine(c) until self.n > 1;"), "until", "7:50", "`until` (restart while a condition holds) is specified but not implemented");
    refused(&desk("reorganize(c);"), "reorganize", "7:50", "`reorganize(...)` is reserved syntax with no semantics yet");
}

#[test]
fn an_unused_expression_statement_is_refused_at_the_expression() {
    refused("fn main() { let a = 1; a + 2; println(\"x\"); }\n", "binary", "1:24", "this expression's value is unused");
    refused("type T { a: Int = 0; }\nfn main() { let t = T { }; t.a; println(\"x\"); }\n", "field", "2:28", "this expression's value is unused");
}

#[test]
fn a_return_in_an_on_failure_body_checks_and_builds() {
    let src = "locus Book {\n    params { credits: Int = 0; debits: Int = 0; }\n    closure balanced { self.credits ~~ self.debits within 0; }\n}\nlocus Desk {\n    params { seen: Int = 0; }\n    on_failure(c: Book, err: ClosureViolation) {\n        if self.seen > 2 { return; }\n        self.seen = self.seen + 1;\n    }\n    run() { Book { credits: 100, debits: 93 }; }\n}\nfn main() { let d = Desk { }; }\n";
    let ((ok, out), (built, bout)) = check_and_build(src, "on_failure_return");
    assert!(ok, "check: {out}");
    assert!(built, "build: {bout}");
    let valued = src.replace("return;", "return 1;");
    refused(&valued, "on_failure_return_value", "8:35", "`return` in an `on_failure` body carries no value");
}

#[test]
fn a_non_locus_accept_param_is_refused_at_the_param() {
    refused(
        "locus H { params { n: Int = 0; } accept(c: Int) { self.n = c; } }\nfn main() { let h = H { }; println(h.n); }\n",
        "accept_int",
        "1:44",
        "`accept(c: Int)`: `accept()` takes a locus or an interface: it adopts a child",
    );
    refused(
        "type T { a: Int = 0; }\nlocus H { params { n: Int = 0; } release(c: T) { } accept(c: T) { } }\nfn main() { let h = H { }; println(h.n); }\n",
        "release_type",
        "2:45",
        "`release(c: T)`: `release()` takes a locus or an interface",
    );
}

#[test]
fn a_value_returned_from_a_body_that_returns_nothing_is_refused_at_the_value() {
    refused(
        "locus D { params { n: Int = 0; } run() { if self.n == 0 { return 5; } } }\nfn main() { let d = D { }; }\n",
        "run_value",
        "1:66",
        "`return` carries a value, but this body returns nothing",
    );
    refused(
        "fn note(n: Int) { if n > 0 { return n; } println(n); }\nfn main() { note(1); }\n",
        "fn_value",
        "1:37",
        "`return` carries a value, but this body returns nothing",
    );
    let ((ok, out), (built, bout)) = check_and_build("fn main() { println(\"x\"); return 3; }\n", "main_exit");
    assert!(ok && built, "fn main's `return n;` is the exit code: {out}{bout}");
}

#[test]
fn a_raw_frame_topic_off_a_layout_ring_is_refused_where_it_is_delivered() {
    refused(
        "topic Recs { payload: BytesView; }\nlocus Reader {\n    bus { subscribe Recs as on_rec; }\n    fn on_rec(v: BytesView) { println(\"rec\"); }\n}\nfn main() { let r = Reader { }; }\n",
        "raw_frame_unbound",
        "3:11",
        "subscribe `Recs`: topic `Recs` declares `payload: BytesView`, the raw-frame path, which only a ring with a declared record layout carries",
    );
    refused(
        "topic Recs { payload: BytesView; }\nlocus Reader {\n    bus { subscribe Recs as on_rec; }\n    fn on_rec(v: BytesView) { println(\"rec\"); }\n}\nmain locus App {\n    params { r: Reader = Reader { }; }\n    bindings { Recs: shm_ring(\"/hale.cli.recs.native\", on_overflow: drop); }\n}\nfn main() { App { }; }\n",
        "raw_frame_native",
        "8:16",
        "`Recs: shm_ring(...)`: topic `Recs` declares `payload: BytesView`, the raw-frame path, and a ring carries raw frames only with a declared record layout",
    );
}

/// `hale check` and `hale build` on a seed of several files; the scratch
/// dir is removed.
fn check_and_build_seed(files: &[(&str, &str)], tag: &str) -> ((bool, String), (bool, String)) {
    let d: PathBuf = std::env::temp_dir().join(format!("hale_check_build_refusals_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    for (rel, src) in files {
        let f = d.join(rel);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(&f, src).unwrap();
    }
    let main = d.join("main.hl").to_string_lossy().to_string();
    let checked = hale(&["check", &main]);
    let built = hale(&["build", &main, "-o", &d.join("bin").to_string_lossy()]);
    let _ = std::fs::remove_dir_all(&d);
    (checked, built)
}

/// An imported seed's `fallible` fn read as a value (`apply(lib::parse,
/// s)`): a fn value's call reads a plain result, and the fallible ABI
/// returns the failure beside it, so the built call wrote through an
/// indeterminate pointer after the check accepted it. The check refuses
/// the path, and the build stops at the same message; the seed's
/// infallible fn passes as a value and builds.
#[test]
fn an_imported_fallible_fn_read_as_a_value_is_refused_at_the_path() {
    let lib = "type Oops { why: String = \"\"; }\n\
               fn parse(s: String) -> Int fallible(Oops) { if s == \"bad\" { fail Oops { why: s }; } return 7; }\n\
               fn twice(n: Int) -> Int { return n * 2; }\n";
    let refused_main = "import \"./lib\" as lib;\n\nfn apply(f: fn(String) -> Int, s: String) -> Int { return f(s); }\n\nfn main() { println(apply(lib::parse, \"ok\")); }\n";
    let ((ok, out), (built, bout)) = check_and_build_seed(&[("main.hl", refused_main), ("lib/lib.hl", lib)], "imported_fallible_value");
    let head = "type error: `lib::parse` is `fallible(lib::Oops)`, and a fn value has no error channel";
    assert!(!ok && out.contains("main.hl:5:27: ") && out.contains(head), "check: {out}");
    // The build renders the check's finding in the author's spelling
    // too: `lib::Oops`, never the merged `__lib_…` name.
    assert!(!built && bout.contains("main.hl:5:27: ") && bout.contains(head) && !bout.contains("__lib_"), "build: {bout}");
    assert!(!bout.contains("codegen error"), "build: {bout}");
    let ok_main = "import \"./lib\" as lib;\n\nfn apply(f: fn(Int) -> Int, n: Int) -> Int { return f(n); }\n\nfn main() { println(apply(lib::twice, 4)); }\n";
    let ((ok, out), (built, bout)) = check_and_build_seed(&[("main.hl", ok_main), ("lib/lib.hl", lib)], "imported_infallible_value");
    assert!(ok, "check: {out}");
    assert!(built, "build: {bout}");
}

/// GH #1945: `or raise` hands the call's error on as the enclosing fn's
/// own, so the two are one type. A mismatch passed the check and the build
/// refused it in lowering's terms (`TypeRef("IoError")`); the check
/// refuses it at the `or raise`, naming both types and the mapping. The
/// mapping (`or fail CryptoError { … }`, the call's error in scope as
/// `err`) checks and builds, and so does a raise of the declared type
/// across a seed (`fallible(lib::Oops)` raising `lib::parse`'s error).
#[test]
fn an_or_raise_of_another_error_type_is_refused_at_the_or_raise() {
    refused(
        "fn seed() -> Bytes fallible(CryptoError) {\n    let b = std::os::getrandom(4) or raise;\n    return b;\n}\nfn main() { let b = seed() or { return; }; println(len(b)); }\n",
        "or_raise_mismatch",
        "2:13",
        "`or raise` hands on the call's error, of type `IoError`, and the enclosing fn is declared `fallible(CryptoError)`: a raised error is the fn's own type. Map it, `or fail CryptoError { … }` (the call's error is in scope as `err`), or declare the fn `fallible(IoError)`",
    );
    let mapped = "fn seed() -> Bytes fallible(CryptoError) {\n    let b = std::os::getrandom(4) or fail CryptoError { kind: err.kind, detail: err.detail };\n    return b;\n}\nfn main() { let b = seed() or { return; }; println(len(b)); }\n";
    let ((ok, out), (built, bout)) = check_and_build(mapped, "or_raise_mapped");
    assert!(ok, "check: {out}");
    assert!(built, "build: {bout}");
    let lib = "type Oops { why: String = \"\"; }\n\
               fn parse(s: String) -> Int fallible(Oops) { if s == \"bad\" { fail Oops { why: s }; } return 7; }\n";
    let main = "import \"./lib\" as lib;\n\nfn twice(s: String) -> Int fallible(lib::Oops) { let n = lib::parse(s) or raise; return n * 2; }\n\nfn main() { println(twice(\"ok\") or 0); }\n";
    let ((ok, out), (built, bout)) = check_and_build_seed(&[("main.hl", main), ("lib/lib.hl", lib)], "or_raise_cross_seed");
    assert!(ok, "check: {out}");
    assert!(built, "build: {bout}");
}

/// GH #1945: a stdlib error type spelled by its path builds wherever its
/// bare name does. `fallible(std::crypto::CryptoError)` checked clean and
/// the build refused it, unlocated ("not in stdlib path-renames table");
/// each path the shadowing law suggests now names the same builtin, so a
/// raise of the stdlib call's error through it is the same type too.
#[test]
fn a_qualified_stdlib_error_type_builds_where_its_bare_name_does() {
    let src = "fn sign(k: Bytes) -> Bytes fallible(std::crypto::CryptoError) {\n    let s = std::crypto::ed25519_sign(k, std::bytes::from_string(\"m\")) or raise;\n    return s;\n}\n\
               fn read(p: String) -> String fallible(std::io::IoError) { let t = std::io::fs::read_file(p) or raise; return t; }\n\
               fn parse(s: String) -> Int fallible(std::str::ParseError) { let n = std::str::parse_int(s) or raise; return n; }\n\
               fn main() {\n    let s = sign(std::bytes::from_string(\"short\")) or { println(err.kind); return; };\n    println(len(s));\n    println(read(\"/nonexistent/x\") or \"none\");\n    println(parse(\"12\") or 0);\n}\n";
    let ((ok, out), (built, bout)) = check_and_build(src, "qualified_errors");
    assert!(ok, "check: {out}");
    assert!(built && !bout.contains("path-renames"), "build: {bout}");
}

/// GH #1944: an `indexed_by` field of a `json:`-tagged record had codecs
/// that called helpers nothing declared, so the check refused the program
/// naming them (`__json_parse_Pair__by_k`) with no location, and warned of
/// a hot-path copy inside them. The record's `from_json` / `to_json` now
/// carry the map, and no finding is raised inside the generated codec.
#[test]
fn an_indexed_by_field_of_a_json_tagged_record_checks_and_builds() {
    let src = "type Pair { k: String `json:\"k\"`; v: Int `json:\"v\"`; }\n\
               type Doc { ps: bounded[Pair; 4] indexed_by k `json:\"ps\"`; }\n\
               fn main() {\n    let d = Doc::from_json(\"{\\\"ps\\\":{\\\"a\\\":{\\\"v\\\":1}}}\") or { println(err.kind); return; };\n    println(Doc::to_json(d));\n}\n";
    let ((ok, out), (built, bout)) = check_and_build(src, "indexed_by_json_tag");
    assert!(ok && !out.contains("__json_") && !out.contains("hot-path"), "check: {out}");
    assert!(built && !bout.contains("hot-path"), "build: {bout}");
}

/// GH #1946: an exposure's receivers are slots of its runtime, numbered
/// within its key. The slots were 16, and a serve site with a 17th
/// receiver type passed the check, was left unexpanded, and failed the
/// build ("path call `api::serve` in expression position") — the check
/// skipping its validation on the way. 17 builds now, and past the
/// slots (100) the check refuses the site at its `receivers:`.
#[test]
fn api_serve_has_no_seventeen_receiver_cliff() {
    let serve = |n: usize| {
        let mut s = String::from("type Ping { n: Int; }\napi Public {\n");
        for i in 0..n {
            s.push_str(&format!("    rpc R{i}::ping;\n"));
        }
        s.push_str("}\nlocus Grants { fn holds(p: std::api::Principal, r: String) -> Bool { return true; } }\n");
        for i in 0..n {
            s.push_str(&format!("locus R{i} {{ fn ping(p: Ping) -> Ping {{ return p; }} }}\n"));
        }
        s.push_str("main locus Desk {\n    params {\n        g: Grants = Grants { };\n");
        for i in 0..n {
            s.push_str(&format!("        r{i}: R{i} = R{i} {{ }};\n"));
        }
        let receivers: Vec<String> = (0..n).map(|i| format!("R{i}: self.r{i}")).collect();
        s.push_str("        fixture: std::api::test::Rpc = std::api::test::Rpc { roles: self.g };\n");
        s.push_str(&format!(
            "        public: api::Exposure = api::serve(Public, self.fixture, as: \"public\", receivers: {{ {} }}, bound: 8, on_full: refuse);\n",
            receivers.join(", ")
        ));
        s.push_str("    }\n    run() { self.public.stop(); }\n}\nfn main() { Desk { }; }\n");
        s
    };
    let ((ok, out), (built, bout)) = check_and_build(&serve(17), "serve_17");
    assert!(ok, "check: {out}");
    assert!(built, "build: {bout}");
    let ((ok, out), (built, bout)) = check_and_build(&serve(101), "serve_101");
    assert!(
        !ok && out.contains("main.hl:")
            && out.contains("type error: serve of `Public` as `public`: its rows name 101 receiver types, and an exposure dispatches to at most 100"),
        "check: {out}"
    );
    assert!(!built && !bout.contains("codegen error"), "build: {bout}");
}

/// GH #1943, STABILITY.md § Deprecation: `std::json::Builder.field` and
/// `.value` write their argument raw, as they always have, and the spec
/// says they take a string; v0.34 makes them quote. Until then a call
/// warns `deprecated:` at the call, naming the raw and the string
/// variants, and still checks, builds and writes what it wrote before.
#[test]
fn a_builder_field_or_value_call_warns_deprecated_and_writes_raw() {
    let src = "fn main() {\n    let b = std::json::Builder { };\n    b.begin_object();\n    b.field(\"n\", \"42\");\n    b.begin_array_field(\"xs\");\n    b.value(\"1\");\n    b.end_array();\n    b.end_object();\n    println(b.result());\n}\n";
    let ((ok, out), (built, bout)) = check_and_build(src, "builder_deprecated");
    assert!(ok, "check: {out}");
    assert!(
        out.contains("main.hl:4:5: warning: deprecated: `std::json::Builder.field(name, value)` writes `value` raw")
            && out.contains("write `raw_field(name, json)` for encoded JSON or `string_field(name, s)` for a string")
            && out.contains("main.hl:6:5: warning: deprecated: `std::json::Builder.value(v)` writes `v` raw")
            && out.matches("deprecated:").count() == 2,
        "check: {out}"
    );
    assert!(built, "build: {bout}");
    // the raw and string variants raise nothing
    let quiet = "fn main() {\n    let b = std::json::Builder { };\n    b.begin_object();\n    b.raw_field(\"n\", \"42\");\n    b.string_field(\"s\", \"x\");\n    b.begin_array_field(\"xs\");\n    b.raw_value(\"1\");\n    b.string_value(\"y\");\n    b.end_array();\n    b.end_object();\n    println(b.result());\n}\n";
    let ((ok, out), _) = check_and_build(quiet, "builder_quiet");
    assert!(ok && !out.contains("deprecated"), "check: {out}");
}

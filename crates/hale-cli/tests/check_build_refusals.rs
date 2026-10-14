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

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

/// GH #1986: an imported seed's generic type and enum variant through
/// the qualified alias path are held to their declarations as the bare
/// spelling is: a template's parameter count, a variant's value count and
/// each value's type, a scale-less Decimal literal taking the payload's
/// scale. Each passed the check and the build refused it, in lowering's
/// terms. The well-formed program builds.
#[test]
fn an_imported_template_or_variant_through_its_alias_is_held_to_its_declaration() {
    let lib = "type Box<T> { value: T = 0; }\ntype Wrap = enum { Held(Int), Empty };\n\
               type Side = enum { Buy, Limit(Decimal<2>), Stop(Int, Decimal<4>) };\n";
    let refused_at = |body: &str, tag: &str, at: &str, head: &str| {
        let main = format!("import \"./lib\" as lib;\n\nfn main() {{\n{body}\n}}\n");
        let ((ok, out), (built, bout)) = check_and_build_seed(&[("main.hl", &main), ("lib/lib.hl", lib)], tag);
        assert!(!ok && out.contains(&format!("main.hl:{at}: type error: {head}")), "check: {out}");
        assert!(!built && bout.contains(head) && !bout.contains("codegen error"), "build: {bout}");
    };
    refused_at(
        "    let b: lib::Box<Int, String> = lib::Box { value: 1 };",
        "alias_arity",
        "4:12",
        "generic type `lib::Box` takes 1 type argument, not 2",
    );
    refused_at("    let w = lib::Wrap::Held(1, 2);", "alias_variant_arity", "4:13", "`lib::Wrap::Held` carries 1 value, and this passes 2");
    refused_at("    let w = lib::Wrap::Held(\"s\");", "alias_variant_value", "4:29", "`lib::Wrap::Held` value 0: expected `Int`, got `String`");
    let ok_main = "import \"./lib\" as lib;\n\n\
                   fn make(px: Decimal<2>) -> lib::Side { return lib::Side::Limit(px); }\n\n\
                   fn main() {\n    let b: lib::Box<Int> = lib::Box { value: 1 };\n    let t = lib::Side::Limit(2.5d);\n\
                   \x20   match lib::Side::Stop(7, 0.0001d) {\n        lib::Side::Stop(n, px) -> { println(f\"{n} {px}\"); },\n        _ -> { },\n    }\n\
                   \x20   match make(1.25d) {\n        lib::Side::Limit(px) -> { println(f\"{px} {b.value}\"); },\n        _ -> { },\n    }\n}\n";
    let ((ok, out), (built, bout)) = check_and_build_seed(&[("main.hl", ok_main), ("lib/lib.hl", lib)], "alias_ok");
    assert!(ok, "check: {out}");
    assert!(built, "build: {bout}");
}

/// GH #1978: a generic fn whose result applies a generic type to its
/// parameter (`-> Box<T>`) is typed at its call as the monomorph its
/// arguments name, as the build lowers it. The checker took it as
/// `Unknown`, so a record no f-string renders, read out of the result,
/// checked clean and the build refused it in lowering's terms (``to_string`
/// not supported for type Bytes``).
#[test]
fn a_generic_fn_returning_a_generic_type_is_typed_as_its_monomorph() {
    let src = "type Blob { data: Bytes; }\ntype Box<T> { v: T; }\nfn box_of<T>(x: T) -> Box<T> { return Box { v: x }; }\n\
               fn main() {\n    let b = box_of(Blob { data: b\"x\" }).v;\n    println(f\"blob = {b}\");\n}\n";
    refused(src, "generic_return_fstring", "6:23", "`to_string` cannot render a value of type `Blob`");
    refused(
        "type Box<T> { v: T; }\nfn box_of<T>(x: T) -> Box<T> { return Box { v: x }; }\nfn main() {\n    let z: Int = box_of(\"s\");\n    println(z);\n}\n",
        "generic_return_let",
        "4:18",
        "let `z`: expected `Int`, got `Box_String`",
    );
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
        "`or raise` hands on the call's error, of type `IoError`, and the enclosing fn is declared `fallible(CryptoError)`: a raised error is the fn's own type, or one it converts from, and nothing converts `IoError` into `CryptoError`. Declare the conversion, `@raise fn …(e: IoError) -> CryptoError { … }`; make `CryptoError` a set holding it (`type CryptoError = IoError | …;`); or translate here, `or fail CryptoError { … }` (the call's error is in scope as `err`)",
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

/// GH #1951 (item 2): an `or raise` whose error two `@raise` fns convert
/// into the enclosing fn's is refused at the `raise`, naming both. Each
/// alone builds.
#[test]
fn an_or_raise_two_conversions_apply_to_is_refused_naming_both() {
    let src = |second: &str| {
        format!(
            "type Oops {{ why: String; }}\ntype ApiError {{ code: Int; }}\n\
             fn parse(s: String) -> Int fallible(Oops) {{ if s == \"bad\" {{ fail Oops {{ why: s }}; }} return 7; }}\n\
             @raise fn api_a(e: Oops) -> ApiError {{ return ApiError {{ code: 1 }}; }}\n{second}\
             fn twice(s: String) -> Int fallible(ApiError) {{\n    let n = parse(s) or raise;\n    return n * 2;\n}}\n\
             fn main() {{ println(twice(\"bad\") or 0); }}\n"
        )
    };
    refused(
        &src("@raise fn api_b(e: Oops) -> ApiError { return ApiError { code: 2 }; }\n"),
        "raise_ambiguous",
        "7:25",
        "`or raise` converts the call's error, of type `Oops`, into the enclosing fn's `ApiError`, and two `@raise` fns convert it: `api_a` and `api_b`. Keep one, or translate here, `or fail ApiError { … }`",
    );
    let ((ok, out), (built, bout)) = check_and_build(&src(""), "raise_one");
    assert!(ok, "check: {out}");
    assert!(built, "build: {bout}");
}

/// GH #1951 (item 2): a `@raise` fn's shape. It takes one parameter, is
/// neither fallible nor generic, and returns another type; and it is
/// declared in the seed of one of its two types (the orphan rule, where a
/// builtin error type belongs to no seed). Each is refused at the fn.
#[test]
fn a_raise_fn_of_the_wrong_shape_is_refused_at_the_fn() {
    let with = |decl: &str| format!("type Oops {{ why: String; }}\ntype ApiError {{ code: Int; }}\n{decl}\nfn main() {{ println(1); }}\n");
    refused(
        &with("@raise fn conv(e: Oops, n: Int) -> ApiError { return ApiError { code: n }; }"),
        "raise_arity",
        "3:11",
        "a `@raise` fn takes one parameter, the error it converts, and `conv` takes 2: `@raise fn conv(e: P) -> E { … }`",
    );
    refused(
        &with("@raise fn conv(e: Oops) -> ApiError fallible(Oops) { return ApiError { code: 1 }; }"),
        "raise_fallible",
        "3:46",
        "a `@raise` fn cannot fail: it runs where an `or raise` hands an error on, and a second error there has nowhere to go. Drop `fallible(…)` from `conv`",
    );
    refused(
        &with("@raise fn conv<T>(e: T) -> ApiError { return ApiError { code: 1 }; }"),
        "raise_generic",
        "3:16",
        "a `@raise` fn converts one error type into another, and `conv` is generic",
    );
    refused(
        &with("@raise fn conv(e: Oops) -> Oops { return e; }"),
        "raise_itself",
        "3:1",
        "`conv` converts `Oops` into itself",
    );
    refused(
        &with("locus L { @raise fn conv(e: Oops) -> ApiError { return ApiError { code: 1 }; } }"),
        "raise_method",
        "3:11",
        "`@raise` declares a conversion, a free fn: `L.conv` is a method of a locus",
    );
    // The orphan rule: the entry seed declares neither the builtin
    // `IoError` nor the library's `ApiError`.
    let lib = "type ApiError { code: Int; }\n";
    let main = "import \"./lib\" as lib;\n\n@raise fn conv(e: IoError) -> lib::ApiError { return lib::ApiError { code: e.errno }; }\n\nfn main() { println(1); }\n";
    let ((ok, out), (built, bout)) = check_and_build_seed(&[("main.hl", main), ("lib/lib.hl", lib)], "raise_orphan");
    let head = "`conv` converts `IoError` into `lib::ApiError`, and a `@raise` fn is declared in the seed of one of its two types: `IoError` is a builtin type, which no seed declares, and `lib::ApiError` is declared in another seed.";
    assert!(!ok && out.contains("main.hl:3:1: type error: ") && out.contains(head), "check: {out}");
    assert!(!built && bout.contains(head) && !bout.contains("codegen error"), "build: {bout}");
}

/// GH #1951 (item 2): a `@raise` fn runs where an `or raise` hands an
/// error on, so it makes no syscall, publishes, spawns or blocks. One that
/// does is refused as its implied effect assertion is, saying why.
#[test]
fn a_raise_fn_that_reaches_a_forbidden_effect_is_refused() {
    let src = "type Oops { why: String; }\ntype ApiError { code: Int; }\n\
               @raise fn conv(e: Oops) -> ApiError {\n    println(e.why);\n    return ApiError { code: 1 };\n}\n\
               fn main() { println(1); }\n";
    refused(
        src,
        "raise_effect",
        "3:11",
        "effect assertion violated: `conv` is a `@raise` fn, which runs where an `or raise` hands an error on, so it makes no syscall, publishes nothing, spawns nothing and never blocks; it must not reach `syscall`",
    );
}

/// GH #1982: a converting `or raise` is the call `conv(err)` it lowers
/// to, for the raising fn's analyses: an effect class the conversion
/// reaches (one a `@raise` fn may: `time`, a program's own) is the raising
/// fn's, with the conversion on the witness path; the conversion's
/// allocations count against the raising fn's `@budget`; and `@no_panic`
/// holds the raise to be a trap, as any `or raise` is.
#[test]
fn a_raise_conversion_is_a_call_of_the_fn_that_raises() {
    let src = |raiser: &str| {
        format!(
            "effect audit;\ntype Oops {{ why: String; }}\ntype ApiError {{ code: Int; detail: String; }}\n\
             @effects(is: {{audit}})\nfn note(s: String) -> Int {{ return len(s); }}\n\
             @raise fn conv(e: Oops) -> ApiError {{\n    let n = note(e.why);\n    return ApiError {{ code: n, detail: \"no \" + e.why }};\n}}\n\
             fn parse(s: String) -> Int fallible(Oops) {{ if s == \"bad\" {{ fail Oops {{ why: s }}; }} return 7; }}\n\
             {raiser}\nfn main() {{ println(twice(\"bad\") or 0); }}\n"
        )
    };
    refused(
        &src("@effects(none: {audit})\nfn twice(s: String) -> Int fallible(ApiError) {\n    return parse(s) or raise;\n}"),
        "raise_conv_effect",
        "12:4",
        "effect assertion violated: `twice` must not reach `audit`, but reaches twice -> conv [`note` declares it carries this effect class]",
    );
    refused(
        &src("@budget(alloc_per_call = 0)\nfn twice(s: String) -> Int fallible(ApiError) {\n    return parse(s) or raise;\n}"),
        "raise_conv_budget",
        "12:4",
        "hot-path budget exceeded: `twice` declares `@budget(alloc_per_call = 0)`",
    );
    let ((ok, out), _) = check_and_build(
        &src("@budget(alloc_per_call = 0)\nfn twice(s: String) -> Int fallible(ApiError) {\n    return parse(s) or raise;\n}"),
        "raise_conv_budget_sites",
    );
    assert!(!ok && out.contains("main.hl:8:12: type error: counts against `twice`'s alloc budget: a `ApiError` instantiation"), "check: {out}");
    // A fallible handler's implicit `or raise` converts, and is a call,
    // the same way.
    refused(
        &src("fn retry(e: Oops) -> Int fallible(Oops) { fail e; }\n@effects(none: {audit})\nfn twice(s: String) -> Int fallible(ApiError) {\n    return parse(s) or retry(err);\n}"),
        "raise_conv_handler",
        "13:4",
        "effect assertion violated: `twice` must not reach `audit`, but reaches twice -> conv",
    );
    refused(
        &src("@no_panic\nfn twice(s: String) -> Int fallible(ApiError) {\n    return parse(s) or raise;\n}"),
        "raise_conv_no_panic",
        "12:4",
        "`@no_panic` violated: `twice` can trap — a fallible call dispositioned `or raise`",
    );
    let ((ok, out), (built, bout)) =
        check_and_build(&src("fn twice(s: String) -> Int fallible(ApiError) {\n    return parse(s) or raise;\n}"), "raise_conv_plain");
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
    // And with the hot-path advisory asked for (opt-in since GH #1948):
    // still nothing inside the generated codec.
    let d: PathBuf = std::env::temp_dir().join(format!("hale_check_build_refusals_{}_indexed_by_warn", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let f = d.join("main.hl");
    std::fs::write(&f, src).unwrap();
    let (ok, out) = hale(&["check", "--warn-hot-path", &f.to_string_lossy()]);
    let _ = std::fs::remove_dir_all(&d);
    assert!(ok && !out.contains("hot-path"), "check --warn-hot-path: {out}");
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

/// GH #1943: v0.35 makes `std::json::Builder.field`
/// and `.value` quote and escape their argument as a string, as the spec
/// always said they take one (v0.33 and v0.34 warned `deprecated:` at each
/// call, which wrote it raw). A call now checks with no warning and writes
/// a JSON string; the raw variants still write their JSON as given.
#[test]
fn a_builder_field_or_value_call_quotes_its_argument() {
    let src = "fn main() {\n    let b = std::json::Builder { };\n    b.begin_object();\n    b.field(\"n\", \"42\");\n    b.field(\"q\", \"say \\\"hi\\\"\");\n    b.begin_array_field(\"xs\");\n    b.value(\"1\");\n    b.raw_value(\"2\");\n    b.end_array();\n    b.raw_field(\"r\", \"[3]\");\n    b.end_object();\n    println(b.result());\n}\n";
    let ((ok, out), (built, bout)) = check_and_build(src, "builder_quotes");
    assert!(ok && !out.contains("deprecated"), "check: {out}");
    assert!(built, "build: {bout}");
    let d: PathBuf = std::env::temp_dir().join(format!("hale_check_build_refusals_{}_builder_quotes_run", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let f = d.join("main.hl");
    std::fs::write(&f, src).unwrap();
    let (ran, rout) = hale(&["run", &f.to_string_lossy()]);
    let _ = std::fs::remove_dir_all(&d);
    assert!(ran, "run: {rout}");
    assert!(
        rout.lines().any(|l| l == "{\"n\": \"42\", \"q\": \"say \\\"hi\\\"\", \"xs\": [\"1\", 2], \"r\": [3]}"),
        "run: {rout}"
    );
}

/// GH #1967: a call through a fn value is held to the value's type. A
/// short call is refused at the callee (a fn type has no defaults), for
/// a local and for a fn-typed locus param; an `Int` argument to a
/// `Float` parameter widens, as at a direct call, and builds.
#[test]
fn a_call_through_a_fn_value_is_held_to_its_type() {
    refused(
        "fn add(a: Int, b: Int) -> Int { return a + b; }\nfn main() {\n    let add2: fn(Int, Int) -> Int = add;\n    println(add2(1));\n}\n",
        "fn_value_arity",
        "4:13",
        "`add2` is a fn value of type `fn(Int, Int) -> Int`, which takes 2 arguments, got 1",
    );
    refused(
        "fn add(a: Int, b: Int) -> Int { return a + b; }\nlocus S {\n    params { f: fn(Int, Int) -> Int = add; }\n    fn go() -> Int { return self.f(1); }\n}\nfn main() { println(S { }.go()); }\n",
        "fn_field_arity",
        "4:29",
        "`f` is a fn value of type `fn(Int, Int) -> Int`, which takes 2 arguments, got 1",
    );
    let widened = "fn half(x: Float) -> Float { return x / 2.0; }\nfn main() {\n    let h: fn(Float) -> Float = half;\n    let n = 8;\n    println(h(n));\n}\n";
    let ((ok, out), (built, bout)) = check_and_build(widened, "fn_value_widen");
    assert!(ok, "check: {out}");
    assert!(built, "build: {bout}");
}

/// GH #1969: `return <a call with no value>;` in a `() fallible(E)` fn
/// is a value returned from a body that returns nothing, as in any fn
/// with no `-> T`.
#[test]
fn a_unit_call_returned_from_a_unit_fallible_fn_is_refused_at_the_value() {
    refused(
        "fn touch() -> () fallible(String) {\n}\n\nfn fwd() -> () fallible(String) {\n    return touch() or fail err;\n}\n\nfn main() {\n    fwd() or discard;\n}\n",
        "unit_fallible_return",
        "5:12",
        "`return` carries a value, but this body returns nothing",
    );
}

/// GH #1971: a pattern on an enum names its enum; a bare `A(x)` is a
/// set member's spelling only.
#[test]
fn a_bare_variant_pattern_on_an_enum_is_refused_at_the_pattern() {
    refused(
        "type E = enum { A(Int), B };\n\nfn f(e: E) -> Int {\n    return match e {\n        A(x) -> x,\n        E::B -> 0,\n    };\n}\n\nfn main() {\n    println(f(E::A(3)));\n}\n",
        "bare_variant",
        "5:9",
        "`A(…)` names a variant of the enum `E` without its enum: a pattern on an enum names it, `E::A(…)`",
    );
    // Structural, whatever the scrutinee is: under a type parameter the
    // check cannot see a set, and lowering refused it unlocated.
    refused(
        "type Opt = enum { Has(Int), Empty };\nfn unwrap<T>(o: T, d: Int) -> Int {\n    return match o {\n        Has(x) -> x,\n    };\n}\nfn main() {\n    let o = Opt::Has(7);\n    println(unwrap(o, 7));\n}\n",
        "bare_variant_generic",
        "4:9",
        "`Has(…)` names a variant without its enum, against a value whose type is not known here",
    );
    // A value that is no enum or set.
    refused(
        "fn f(n: Int) -> Int {\n    return match n {\n        Has(x) -> x,\n        _ -> 0,\n    };\n}\nfn main() { println(f(1)); }\n",
        "bare_variant_int",
        "3:9",
        "`Has(…)` is a constructor pattern without its enum against a value of `Int`, which is no enum or set",
    );
    // A set's member keeps its bare name, an imported set's included.
    let set = "type Book { id: Int; }\ntype Trade { qty: Int; }\ntype Inbound = Book | Trade;\nfn d(m: Inbound) -> Int {\n    return match m {\n        Book(b) -> b.id,\n        Trade(t) -> t.qty,\n    };\n}\nfn main() { println(d(Trade { qty: 4 })); }\n";
    let ((ok, out), (built, bout)) = check_and_build(set, "bare_set_member");
    assert!(ok, "check: {out}");
    assert!(built, "build: {bout}");
}

/// GH #2030: a native `shm_ring` copies the payload's bytes into the slot,
/// so a payload that is not flat is refused at the binding whether or not
/// it says `where zero_copy` (a `String` crossed as a pointer into the
/// publisher's arena: a use-after-free in process). A ring name over 31
/// bytes is refused too: macOS's shm_open fails it at startup.
#[test]
fn a_native_shm_ring_is_refused_a_payload_that_is_not_flat_and_a_long_name() {
    let prog = |fields: &str, name: &str| {
        format!(
            "type Note {{ {fields} }}\n\ntopic Notes {{\n    payload: Note;\n}}\n\nlocus Sub {{\n    params {{ seen: Int = 0; }}\n    bus {{ subscribe Notes as on_note; }}\n    fn on_note(n: Note) {{ self.seen = self.seen + 1; }}\n}}\n\nlocus Pub {{\n    bus {{ publish Notes; }}\n    run() {{ Notes <- Note {{ id: 1 }}; }}\n}}\n\nmain locus App {{\n    params {{\n        sub: Sub = Sub {{ }};\n        p: Pub = Pub {{ }};\n    }}\n    bindings {{ Notes: shm_ring(\"{name}\", slot_count: 64, on_overflow: drop); }}\n    run() {{ println(self.sub.seen); }}\n}}\n\nfn main() {{ App {{ }}; }}\n"
        )
    };
    refused(
        &prog("id: Int; text: String = \"\";", "/hale-2030-s"),
        "shm_string",
        "23:16",
        "`Notes: shm_ring(...)`: topic `Notes`'s payload `Note` is not flat-shapeable",
    );
    refused(
        &prog("id: Int; v: Int = 0;", "/hale-triage-shm-ring-with-a-long-name-09e"),
        "shm_long_name",
        "23:23",
        "`Notes: shm_ring(\"/hale-triage-shm-ring-with-a-long-name-09e\", ...)`: the ring's name is 42 bytes, and a POSIX shm name is at most 31 bytes on macOS",
    );
    // A `codec(...)` clause does not serialize a native ring's slot (the
    // native publish never calls its encoder), so it changes nothing.
    let codec = prog("id: Int; text: String = \"\";", "/hale-2030-c")
        .replace(
            "on_overflow: drop); }",
            "on_overflow: drop) codec(C { }); }",
        )
        .replace(
            "\n\nmain locus App",
            "\n\ntype EncErr { kind: String = \"\"; }\ntype DecErr { kind: String = \"\"; }\nlocus C {\n    fn encode(v: Note) -> Bytes fallible(EncErr) { return std::bytes::from_string(v.text); }\n    fn decode(b: Bytes) -> Note fallible(DecErr) { return Note { id: 0 }; }\n}\n\nmain locus App",
        );
    let ((ok, out), (built, bout)) = check_and_build(&codec, "shm_codec");
    let head = "`Notes: shm_ring(...)`: topic `Notes`'s payload `Note` is not flat-shapeable";
    assert!(!ok && out.contains(head), "check: {out}");
    assert!(!built && bout.contains(head) && !bout.contains("codegen error"), "build: {bout}");
    // A flat payload and a 31-byte name still check and build.
    let ((ok, out), (built, bout)) = check_and_build(&prog("id: Int; v: Int = 0;", "/hale-2030-abcdefghijklmnopqrst"), "shm_flat");
    assert!(ok, "check: {out}");
    assert!(built, "build: {bout}");
}

/// GH #1970: a body's tail is its return, checked as one: a member of
/// the return type's set builds as that set (in a fn, a fallible fn and
/// a method), and a tail of another type is refused at the tail.
#[test]
fn a_tail_is_checked_and_lowered_as_its_return() {
    let src = "type Book { id: Int; }\ntype Trade { qty: Int; }\ntype Inbound = Book | Trade;\nfn decode() -> Inbound {\n    Trade { qty: 1 }\n}\nfn maybe(n: Int) -> Inbound fallible(String) {\n    if n < 0 { fail \"neg\"; }\n    Book { id: n }\n}\nlocus D {\n    fn next() -> Inbound {\n        Book { id: 2 }\n    }\n}\nfn main() {\n    let x = decode();\n    let y = maybe(1) or Trade { qty: 0 };\n    let z = D { }.next();\n}\n";
    let ((ok, out), (built, bout)) = check_and_build(src, "set_tail");
    assert!(ok, "check: {out}");
    assert!(built, "build: {bout}");
    refused("fn f() -> Int {\n    \"s\"\n}\nfn main() { println(f()); }\n", "tail_mismatch", "2:5", "this is the body's value (its tail): expected `Int`, got `String`");
}

/// GH #1968: a fallible locus method's tail is its return.
#[test]
fn a_fallible_method_ending_in_a_tail_builds() {
    let src = "locus Lv {\n    fn go(x: Int) -> Int fallible(String) {\n        x\n    }\n}\n\nfn main() {\n    println(Lv { }.go(1) or 0);\n}\n";
    let ((ok, out), (built, bout)) = check_and_build(src, "fallible_method_tail");
    assert!(ok, "check: {out}");
    assert!(built, "build: {bout}");
}

/// GH #1972: a locus literal used as a value where its enclosing locus
/// runs both on the owner's thread and off it is refused by the check,
/// in the words lowering used, with the instances it runs in.
#[test]
fn a_mixed_edge_literal_used_as_a_value_is_refused_at_the_literal() {
    refused(
        "locus I {\n    params {\n        x: Int = 0;\n    }\n}\n\nfn id<T>(x: T) -> T {\n    return x;\n}\n\nlocus Worker {\n    fn spawn() {\n        id(I { });\n    }\n}\n\nlocus Owner {\n    params {\n        w: Worker = Worker { };\n    }\n}\n\nmain locus App {\n    params {\n        a: Worker = Worker { };\n        o: Owner = Owner { };\n    }\n    placement {\n        o: pinned;\n    }\n    accept(c: I) { }\n}\n\nfn main() {\n    App { };\n}\n",
        "mixed_value",
        "13:12",
        "`I { }` is born to `App`, but `Worker` runs both on `App`'s thread and off it (App.a on main; App.o.w on the pinned thread of App.o; the owner App on main), so its birth is a same-tower allocation in some instances and a cross-pool post in others; a cross-pool birth is fire-and-forget, so the literal cannot be used as a value.",
    );
}

/// GH #2023: a path to a variant the enum does not declare is refused by
/// the check at the path, with the nearest variant, in every position: a
/// value, a comparison operand, a match pattern (nested ones too) and a
/// payload constructor's callee. Each checked clean and the build
/// refused it (`enum `Place` has no variant `Wrok``).
#[test]
fn a_variant_the_enum_does_not_declare_is_refused_at_the_path() {
    let head = "enum `Place` has no variant `Wrok` — did you mean `Work`?";
    refused(
        "type Place = enum { Work, Home };\n\nfn label(s: Place) -> String {\n    return match s {\n        Place::Work -> \"work\",\n        Place::Home -> \"home\",\n    };\n}\n\nfn main() {\n    let s = Place::Wrok;\n    println(label(s));\n}\n",
        "variant_value",
        "11:13",
        head,
    );
    refused(
        "type Place = enum { Work, Home };\nfn at_work(s: Place) -> Bool {\n    return s == Place::Wrok;\n}\nfn main() { println(at_work(Place::Home)); }\n",
        "variant_operand",
        "3:17",
        head,
    );
    refused(
        "type Place = enum { Work, Home };\nfn label(s: Place) -> String {\n    return match s {\n        Place::Wrok -> \"w\",\n        _ -> \"o\",\n    };\n}\nfn main() { println(label(Place::Home)); }\n",
        "variant_pattern",
        "4:9",
        head,
    );
    refused(
        "type Place = enum { Work(Int), Home };\nfn main() {\n    let s = Place::Wrok(1);\n    match s {\n        Place::Work(n) -> println(n),\n        Place::Home -> println(\"home\"),\n    }\n}\n",
        "variant_ctor",
        "3:13",
        head,
    );
    // Nested in a variant's values.
    refused(
        "type Place = enum { Work, Home };\ntype Trip = enum { To(Place), Stay };\nfn f(t: Trip) -> Int {\n    return match t {\n        Trip::To(Place::Wrok) -> 1,\n        _ -> 0,\n    };\n}\nfn main() { println(f(Trip::Stay)); }\n",
        "variant_nested_pattern",
        "5:18",
        head,
    );
    // No near spelling: the variants are listed.
    refused(
        "type Place = enum { Work, Home };\nfn main() {\n    let s = Place::Elsewhere;\n    println(s == Place::Home);\n}\n",
        "variant_far",
        "3:13",
        "enum `Place` has no variant `Elsewhere`; its variants are Work, Home",
    );
    // The enum's own names (RFC #1995 E1) are not variants and still work.
    let ok_src = "type Place = enum { Work, Home };\nfn main() {\n    println(Place::name(Place::Work));\n    println(Place::index(Place::Home));\n    for p in Place::variants { println(Place::name(p)); }\n}\n";
    let ((ok, out), (built, bout)) = check_and_build(ok_src, "variant_enum_names");
    assert!(ok, "check: {out}");
    assert!(built, "build: {bout}");
}

/// GH #2023, on review: a concrete alias of a generic enum (`type I =
/// E<Int>;`) is neither the template nor a declared enum, so the lookup
/// passed a typo through it in a constructor call, a unit variant and a
/// match pattern, and the build refused each (`enum `E_Int` has no
/// variant `Wrok``). The alias resolves to the monomorph, whose template
/// declares the variants; the right spellings still build.
#[test]
fn a_variant_a_generic_enums_alias_does_not_declare_is_refused_at_the_path() {
    let decl = "type E<T> = enum { Work(T), Home };\ntype I = E<Int>;\n";
    refused(
        &format!("{decl}fn main() {{ let x: I = I::Wrok(1); }}\n"),
        "variant_alias_ctor",
        "3:24",
        "enum `E<Int>` has no variant `Wrok` — did you mean `Work`?",
    );
    refused(
        &format!("{decl}fn main() {{ let x: I = I::Hme; }}\n"),
        "variant_alias_unit",
        "3:24",
        "enum `E<Int>` has no variant `Hme` — did you mean `Home`?",
    );
    refused(
        &format!(
            "{decl}fn f(x: I) -> Int {{\n    return match x {{\n        I::Wrok(n) -> n,\n        _ -> 0,\n    }};\n}}\nfn main() {{ println(f(I::Home)); }}\n"
        ),
        "variant_alias_pattern",
        "5:9",
        "enum `E<Int>` has no variant `Wrok` — did you mean `Work`?",
    );
    let ok_src = format!(
        "{decl}fn f(x: I) -> Int {{\n    return match x {{\n        I::Work(n) -> n,\n        I::Home -> 0,\n    }};\n}}\nfn main() {{ let a: I = I::Work(3); let b: I = I::Home; println(f(a) + f(b)); }}\n"
    );
    let ((ok, out), (built, bout)) = check_and_build(&ok_src, "variant_alias_ok");
    assert!(ok, "check: {out}");
    assert!(built, "build: {bout}");
}

/// GH #2069 (F.58): a `.Variant` pattern names a variant of the value's
/// enum, so one that names no variant of it is refused by the check where
/// it is written, with the nearest variant (as `E::Wrok` is, GH #2023),
/// in every pattern form: an arm, an or-pattern's alternative, `if let`,
/// `let … else`; through an alias of a generic enum's instantiation the
/// enum is shown as written. The build stops at the same message.
#[test]
fn a_dot_variant_the_enum_does_not_declare_is_refused_where_it_is_written() {
    let decl = "type Place = enum { Work(Int), Home };\n";
    refused(
        &format!("{decl}fn f(s: Place) -> Int {{\n    return match s {{\n        .Wrok(n) -> n,\n        _ -> 0,\n    }};\n}}\nfn main() {{ println(f(Place::Home)); }}\n"),
        "dot_variant_arm",
        "4:9",
        "`.Wrok(…)`: enum `Place` has no variant `Wrok` — did you mean `.Work`?",
    );
    refused(
        &format!("{decl}fn f(s: Place) -> Int {{\n    return match s {{\n        .Work(n) | .Hme -> 1,\n        _ -> 0,\n    }};\n}}\nfn main() {{ println(f(Place::Home)); }}\n"),
        "dot_variant_alternative",
        "4:20",
        "`.Hme`: enum `Place` has no variant `Hme` — did you mean `.Home`?",
    );
    refused(
        &format!("{decl}fn f(s: Place) -> Int {{\n    if let .Wrk(n) = s {{ n }} else {{ 0 }}\n}}\nfn main() {{ println(f(Place::Home)); }}\n"),
        "dot_variant_if_let",
        "3:12",
        "`.Wrk(…)`: enum `Place` has no variant `Wrk` — did you mean `.Work`?",
    );
    refused(
        &format!("{decl}fn f(s: Place) -> Int {{\n    let .Wrok(n) = s else {{\n        return 0;\n    }};\n    return n;\n}}\nfn main() {{ println(f(Place::Home)); }}\n"),
        "dot_variant_let_else",
        "3:9",
        "`.Wrok(…)`: enum `Place` has no variant `Wrok` — did you mean `.Work`?",
    );
    refused(
        &format!("{decl}fn f(s: Place) -> Int {{\n    return match s {{\n        .Elsewhere -> 1,\n        _ -> 0,\n    }};\n}}\nfn main() {{ println(f(Place::Home)); }}\n"),
        "dot_variant_far",
        "4:9",
        "`.Elsewhere`: enum `Place` has no variant `Elsewhere`; its variants are Work, Home",
    );
    refused(
        "type E<T> = enum { Work(T), Home };\ntype I = E<Int>;\nfn f(x: I) -> Int {\n    return match x {\n        .Wrok(n) -> n,\n        _ -> 0,\n    };\n}\nfn main() { println(f(I::Home)); }\n",
        "dot_variant_generic_alias",
        "5:9",
        "`.Wrok(…)`: enum `E<Int>` has no variant `Wrok` — did you mean `.Work`?",
    );
}

/// GH #2069 (F.58): against a value whose type is no enum, a `.Variant`
/// names nothing, and the check refuses it at the pattern: an `Int`, a
/// set (whose members are matched by their types' names), and a generic
/// body's type parameter, once a call specializes the body.
#[test]
fn a_dot_variant_against_a_value_of_no_enum_is_refused_at_the_pattern() {
    refused(
        "fn f(n: Int) -> Int {\n    return match n {\n        .Work(x) -> x,\n        _ -> 0,\n    };\n}\nfn main() { println(f(1)); }\n",
        "dot_variant_int",
        "3:9",
        "`.Work(…)` names a variant of the value's enum, and the value matched is of type `Int`, which is no enum",
    );
    refused(
        "type Book { title: String; }\ntype Film { title: String; }\ntype Item = Book | Film;\nfn f(i: Item) -> Int {\n    return match i {\n        .Book(b) -> 1,\n        _ -> 0,\n    };\n}\nfn main() { println(f(Book { title: \"t\" })); }\n",
        "dot_variant_set",
        "6:9",
        "`.Book(…)` names a variant of an enum, and the value matched is of the set `Item`: a set's member is matched by its type's name alone, `Book(…)`",
    );
    refused(
        "type Job = enum { Work(Int), Rest };\nfn f<T>(t: T) -> Int {\n    return match t {\n        .Work(x) -> 1,\n        _ -> 0,\n    };\n}\nfn main() { println(f(3)); }\n",
        "dot_variant_type_param",
        "4:9",
        "`.Work(…)` names a variant of the value's enum, and the value matched is of type `Int`, which is no enum (in this generic body, specialized for `Int`)",
    );
}

/// GH #2069: a variant's pattern that names its values names each of
/// them. The check passed `Job::Work(1, 2)` against `Work(Int)` and the
/// build refused it (`has 2 arg(s); variant declares 1`); `.Work(1, 2)`
/// and `.Rest(x)` are the same pattern, refused the same way.
#[test]
fn a_variant_pattern_naming_the_wrong_number_of_values_is_refused_at_the_pattern() {
    let decl = "type Job = enum { Work(Int), Rest };\n";
    for (pat, tag) in [("Job::Work(1, 2)", "arity_qualified"), (".Work(1, 2)", "arity_dot")] {
        refused(
            &format!("{decl}fn f(j: Job) -> Int {{\n    return match j {{\n        {pat} -> 2,\n        _ -> 0,\n    }};\n}}\nfn main() {{ println(f(Job::Rest)); }}\n"),
            tag,
            "4:9",
            "the variant `Work` carries 1 value, and this pattern names 2",
        );
    }
    refused(
        &format!("{decl}fn f(j: Job) -> Int {{\n    return match j {{\n        .Rest(x) -> 2,\n        _ -> 0,\n    }};\n}}\nfn main() {{ println(f(Job::Rest)); }}\n"),
        "arity_none",
        "4:9",
        "the variant `Rest` carries no value, so its pattern takes no parentheses, and this pattern names 1",
    );
}

/// GH #2069, on review: the shadowing desugar runs before the check, so
/// against a value whose enum the match does not show (a field, a call,
/// an element) and several enums declaring `Use`, it cannot tell whether
/// `.Use(f)`'s `f` is a fn value. The check can: a call of `f` is the call
/// through the binding where `f` is the payload's fn, and the program's fn
/// where `f` is an `Int`, exactly as `Apply::Use(f)` and `Count::Use(f)`.
#[test]
fn a_dot_variant_binding_of_a_fns_name_calls_what_the_spelled_pattern_does() {
    let decl = "type Apply = enum { Use(fn(Int) -> Int), Skip };\ntype Count = enum { Use(Int), Skip };\ntype Holder { a: Apply; c: Count; }\nfn f(n: Int) -> Int { return n + 1; }\nfn tenfold(n: Int) -> Int { return n * 10; }\n";
    let main = "fn main() {\n    let h = Holder { a: Apply::Use(tenfold), c: Count::Use(3) };\n    println(viaa(h), \" \", viac(h));\n}\n";
    for (pa, pc, tag) in [
        ((".Use(f)", ".Skip"), (".Use(f)", ".Skip"), "dot_shadow_dotted"),
        (("Apply::Use(f)", "Apply::Skip"), ("Count::Use(f)", "Count::Skip"), "dot_shadow_spelled"),
    ] {
        let src = format!(
            "{decl}fn viaa(h: Holder) -> Int {{\n    return match h.a {{\n        {} -> f(2),\n        {} -> 0,\n    }};\n}}\nfn viac(h: Holder) -> Int {{\n    return match h.c {{\n        {} -> f(1) + f,\n        {} -> 0,\n    }};\n}}\n{main}",
            pa.0, pa.1, pc.0, pc.1
        );
        let d: PathBuf = std::env::temp_dir().join(format!("hale_check_build_refusals_{}_{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join("main.hl");
        std::fs::write(&f, &src).unwrap();
        let bin = d.join("bin");
        let (built, bout) = hale(&["build", &f.to_string_lossy(), "-o", &bin.to_string_lossy()]);
        assert!(built, "{tag}: {bout}");
        let run = Command::new(&bin).output().expect("run");
        let _ = std::fs::remove_dir_all(&d);
        assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "20 5", "{tag}");
    }
}

/// GH #2068 (F.59): a receiver call on a text value names a `std::str` /
/// `std::bytes` fn that takes the value first, or nothing; what names
/// nothing is refused by the check where it is written, and the build
/// stops at the same message. An unknown name gets the nearest fns; a
/// fn of the program's is called with the value as an argument; `len`
/// is the builtin; a value of no text type (a record, an `Int`) has no
/// receiver calls; a generic body's type parameter calls nothing.
#[test]
fn a_receiver_call_naming_no_text_fn_is_refused_where_it_is_written() {
    refused(
        "fn main() {\n    let s = \"abc\";\n    println(s.trimm());\n}\n",
        "recv_near",
        "3:13",
        "no fn `trimm` in `std::str` takes a `String` first, so `.trimm` names nothing on it — the nearest are `.trim`, `.trim_end`, `.trim_start`",
    );
    refused(
        "fn main() {\n    let s = \"abc\";\n    println(s.frobnicate());\n}\n",
        "recv_far",
        "3:13",
        "no fn `frobnicate` in `std::str` takes a `String` first, so `.frobnicate` names nothing on it",
    );
    refused(
        "fn shout(s: String) -> String { return s; }\nfn main() {\n    let s = \"abc\";\n    println(s.shout());\n}\n",
        "recv_user_fn",
        "4:13",
        "no fn `shout` in `std::str` takes a `String` first, so `.shout` names nothing on it — `shout` is a fn of the program's: call it as `shout(x, …)`",
    );
    refused(
        "fn main() {\n    let s = \"abc\";\n    println(s.len());\n}\n",
        "recv_len",
        "3:13",
        "no field `len` on `String` — the length of a String is the builtin `len(s)`",
    );
    refused(
        "fn main() {\n    let b = std::bytes::from_string(\"abc\");\n    println(b.lower());\n}\n",
        "recv_bytes",
        "3:13",
        "no fn `lower` in `std::bytes` takes a `Bytes` first, so `.lower` names nothing on it",
    );
    refused(
        "type Point { x: Int; }\nfn main() {\n    let p = Point { x: 1 };\n    println(p.trim());\n}\n",
        "recv_record",
        "4:13",
        "no field `trim` on `Point`",
    );
    refused(
        "fn main() {\n    let n = 5;\n    println(n.trim());\n}\n",
        "recv_int",
        "3:13",
        "no field `trim` on `Int`",
    );
    refused(
        "fn main() {\n    let s = \"abc\";\n    let f = s.trim;\n    println(\"x\");\n}\n",
        "recv_value",
        "3:13",
        "no field `trim` on `String` — `trim` is the fn `std::str::trim`, called on the value as `.trim(…)`; it is not a field",
    );
    refused(
        "fn g<T>(x: T) -> String {\n    return x.trim();\n}\nfn main() {\n    println(g(\"x\"));\n}\n",
        "recv_type_param",
        "2:12",
        "`.trim(…)` on a value of a type parameter calls nothing",
    );
}

/// GH #2068 (F.59): a receiver call is the path call it means to every
/// pass after the check, so a program spelled with receiver calls and
/// the same program spelled with the paths have one allocation summary
/// (positions aside), meet one `@effects` contract, and build and run to
/// one output.
#[test]
fn a_receiver_call_is_summarized_and_built_as_its_path_call() {
    let receiver = "@effects(none: { syscall, block })\nfn norm(s: String) -> String {\n    return s.trim().lower().replace(\"a\", \"b\");\n}\n\nfn total(s: String) -> Int {\n    let mut t = 0;\n    for p in s.split(\",\") {\n        t = t + (p.trim().parse_int() or 0);\n    }\n    return t + s.words().filter(len(it) > 1).count();\n}\n\nfn main() {\n    println(norm(\" A \"), total(\"1,22\"));\n}\n";
    let path = "@effects(none: { syscall, block })\nfn norm(s: String) -> String {\n    return std::str::replace(std::str::lower(std::str::trim(s)), \"a\", \"b\");\n}\n\nfn total(s: String) -> Int {\n    let mut t = 0;\n    for p in std::str::split(s, \",\") {\n        t = t + (std::str::parse_int(std::str::trim(p)) or 0);\n    }\n    return t + std::str::words(s).filter(len(it) > 1).count();\n}\n\nfn main() {\n    println(norm(\" A \"), total(\"1,22\"));\n}\n";
    let summary = |src: &str, tag: &str| -> String {
        let d: PathBuf = std::env::temp_dir().join(format!("hale_check_build_refusals_{}_{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join("main.hl");
        std::fs::write(&f, src).unwrap();
        let (ok, out) = hale(&["check", "--dump-alloc-summary", &f.to_string_lossy()]);
        let bin = d.join("bin");
        let (built, bout) = hale(&["build", &f.to_string_lossy(), "-o", &bin.to_string_lossy()]);
        assert!(ok && built, "{tag}: {out}{bout}");
        let run = Command::new(&bin).output().expect("run");
        let _ = std::fs::remove_dir_all(&d);
        // positions are the spelling's; what the summary says is not
        let mut s = String::new();
        for line in out.lines() {
            let mut l = String::new();
            let mut rest = line;
            while let Some(at) = rest.find(" @") {
                l.push_str(&rest[..at]);
                rest = rest[at + 2..].trim_start_matches(|c: char| c.is_ascii_digit() || c == '.');
            }
            l.push_str(rest);
            s.push_str(&l);
            s.push('\n');
        }
        s + "run: " + &String::from_utf8_lossy(&run.stdout)
    };
    let (a, b) = (summary(receiver, "recv_summary"), summary(path, "path_summary"));
    assert!(a.contains("<unresolved: std::str::trim>") && a.contains("run: b24"), "{a}");
    assert_eq!(a, b);
}

/// GH #2068, on review: a text source over a part of another is typed only
/// once the outer loop is desugared, so the snapshot settles in rounds
/// until a walk resolves nothing, however deep the loops nest: eight
/// `for vN in v(N-1).words()` build and run as the path spelling does.
#[test]
fn receiver_text_sources_nested_eight_deep_build() {
    let mut src = String::from("fn main() {\n    let v = \"ab\";\n");
    let mut prev = "v".to_string();
    for k in 0..8 {
        src.push_str(&format!("{}for v{k} in {prev}.words() {{\n", "    ".repeat(k + 1)));
        prev = format!("v{k}");
    }
    src.push_str(&format!("{}println({prev}.upper());\n", "    ".repeat(9)));
    for k in (0..8).rev() {
        src.push_str(&format!("{}}}\n", "    ".repeat(k + 1)));
    }
    src.push_str("}\n");
    let d: PathBuf = std::env::temp_dir().join(format!("hale_check_build_refusals_{}_nested8", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let f = d.join("main.hl");
    std::fs::write(&f, &src).unwrap();
    let bin = d.join("bin");
    let (built, bout) = hale(&["build", &f.to_string_lossy(), "-o", &bin.to_string_lossy()]);
    assert!(built, "{src}\n{bout}");
    let run = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_dir_all(&d);
    assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "AB");
}

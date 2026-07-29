//! m78: std::str — minimal string parsing primitives.
//!
//! The parse_int / parse_decimal behaviour tests moved to
//! `tests/hale/str_parse_*_test.hl`, written in Hale and run by
//! `hale test`. Two reasons, both concrete rather than stylistic:
//!
//!  * the expectation stops being transcribed. It was
//!    `stdout.contains("a=42")` here; it is
//!    `assert_eq_int(parse_int("42"), 42)` there — which is also
//!    STRICTER, since `contains("a=42")` passes on `a=421`.
//!  * `build_executable` (what these tests call) does NOT run the
//!    typechecker, so the programs here were compiled and run but
//!    never checked. `hale test` checks them. That gap is not
//!    hypothetical: the `err.kind` shape these tests exercise did
//!    not typecheck at all, and moving them found it.
//!
//! What stays here is what genuinely needs the Rust side: the two
//! DIAGNOSTIC tests, which assert on compiler output rather than
//! program behaviour.

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn build_and_run(name: &str, source: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(source).expect("parse");
    let bin = harness::unique_bin(&format!("hale_test_stdlib_str_{}", name));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (String::from_utf8_lossy(&output.stdout).to_string(), output.status)
}

#[test]
fn std_str_parse_error_qualified_path_resolves() {
    // v1.x polish (2026-05-20): `std::str::ParseError` resolves
    // to the same struct the stdlib's parse_* fns inject. Lets
    // users disambiguate explicitly in fn signatures and `as e`
    // bindings — useful when a project also has its own local
    // error types.
    let src = r#"
        fn handle(e: std::str::ParseError) -> Int {
            println("kind=", e.kind);
            println("input=", e.input);
            return -1;
        }
        fn main() {
            let v = std::str::parse_int("nope") or handle(err);
            println("v=", v);
        }
    "#;
    let (stdout, status) = build_and_run("qualified_path", src);
    assert!(status.success(), "build/run failed: {:?}", stdout);
    assert!(
        stdout.contains("kind=parse_int") && stdout.contains("input=nope"),
        "expected qualified-path handler to see stdlib ParseError fields, \
         got stdout: {:?}",
        stdout
    );
}

#[test]
fn std_str_parse_user_parse_error_collision_diagnoses_cleanly() {
    // v1.x polish (2026-05-20): when a user declares their own
    // `type ParseError` with non-stdlib-compatible fields, the
    // codegen previously panicked with `ParseError.kind field`.
    // Now it returns a clean diagnostic naming the fix paths.
    let src = r#"
        type ParseError { msg: String; venue: String; }
        fn handle(e: ParseError) -> Int { let _ = e; return -1; }
        fn main() {
            let v = std::str::parse_int("nope") or handle(err);
            let _ = v;
        }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin("hale_test_stdlib_str_collision");
    let result = build_executable(&program, &bin);
    let _ = std::fs::remove_file(&bin);
    let err = match result {
        Err(e) => e,
        Ok(()) => panic!("expected codegen failure, but build succeeded"),
    };
    let msg = format!("{:?}", err);
    assert!(
        msg.contains("user-declared `type ParseError`")
            && msg.contains("std::str::ParseError"),
        "expected clean diag naming the qualified-path fix, got: {}",
        msg
    );
}

//! m87: std::test::* assertion primitives.
//!
//! Each test compiles a small .hl program that exercises one
//! of the assertion primitives, runs it, and verifies:
//!
//! - Success path: program exits 0 with no diagnostic on stdout.
//! - Failure path: program exits non-zero, stdout contains the
//!   "ASSERTION FAILED" prefix + the user-supplied message.
//!
//! These pin the contract Phase 2 ships. Real Hale-language
//! tests built ON TOP of this layer live in
//! `tests/hale_self_test.rs`.

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn build_and_run(name: &str, source: &str) -> std::process::Output {
    let program = hale_syntax::parse_source(source).expect("parse");
    let bin = harness::unique_bin(&format!("hale_test_assert_{}", name));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    output
}

#[test]
fn assert_true_is_silent_and_exits_zero() {
    // Pass-path discipline: a test program with all assertions
    // passing should produce NO output and exit 0. That's the
    // signal a test runner consumes.
    let src = r#"
        fn main() {
            std::test::assert(true, "trivially true");
            std::test::assert(1 + 1 == 2, "arithmetic still works");
        }
    "#;
    let out = build_and_run("assert_true", src);
    assert!(
        out.status.success(),
        "expected exit 0; got {:?}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.is_empty(),
        "passing assertions must be silent; got stdout: {:?}",
        stdout
    );
}

#[test]
fn assert_false_emits_diagnostic_and_exits_nonzero() {
    let src = r#"
        fn main() {
            std::test::assert(false, "this should fail");
            println("after-assert");  // never runs
        }
    "#;
    let out = build_and_run("assert_false", src);
    assert!(
        !out.status.success(),
        "expected non-zero exit; got {:?}",
        out.status
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("ASSERTION FAILED: this should fail"),
        "missing diagnostic; got: {:?}",
        stdout
    );
    assert!(
        !stdout.contains("after-assert"),
        "execution must stop on assertion failure; got: {:?}",
        stdout
    );
}

#[test]
fn assert_eq_int_silent_when_equal() {
    let src = r#"
        fn main() {
            std::test::assert_eq_int(42, 42, "answer is 42");
            std::test::assert_eq_int(0 - 5, 0 - 5, "negatives match");
        }
    "#;
    let out = build_and_run("assert_eq_int_pass", src);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.is_empty(), "got: {:?}", stdout);
}

#[test]
fn assert_eq_int_diagnostic_includes_actual_and_expected() {
    let src = r#"
        fn main() {
            std::test::assert_eq_int(3, 7, "should match");
        }
    "#;
    let out = build_and_run("assert_eq_int_fail", src);
    assert!(!out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("ASSERTION FAILED: should match"),
        "got: {:?}",
        stdout
    );
    assert!(
        stdout.contains("expected: 7"),
        "got: {:?}",
        stdout
    );
    assert!(
        stdout.contains("actual:   3"),
        "got: {:?}",
        stdout
    );
}

#[test]
fn assert_eq_str_silent_when_equal() {
    let src = r#"
        fn main() {
            std::test::assert_eq_str("hello", "hello", "literal eq");
            let a = "concat" + "enated";
            std::test::assert_eq_str(a, "concatenated", "concat eq");
        }
    "#;
    let out = build_and_run("assert_eq_str_pass", src);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.is_empty(), "got: {:?}", stdout);
}

#[test]
fn assert_eq_str_diagnostic_includes_both_values() {
    let src = r#"
        fn main() {
            std::test::assert_eq_str("got", "expected", "string mismatch");
        }
    "#;
    let out = build_and_run("assert_eq_str_fail", src);
    assert!(!out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("ASSERTION FAILED: string mismatch"),
        "got: {:?}",
        stdout
    );
    assert!(
        stdout.contains("expected: \"expected\""),
        "got: {:?}",
        stdout
    );
    assert!(
        stdout.contains("actual:   \"got\""),
        "got: {:?}",
        stdout
    );
}

#[test]
fn first_failing_assertion_short_circuits() {
    // Multi-assertion test: only the first failure runs. The
    // second assertion (which would also fail) shouldn't
    // appear in the diagnostic, because std::process::exit
    // terminates immediately.
    let src = r#"
        fn main() {
            std::test::assert(true, "first passes");
            std::test::assert(false, "second fails");
            std::test::assert(false, "third never runs");
        }
    "#;
    let out = build_and_run("short_circuit", src);
    assert!(!out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("ASSERTION FAILED: second fails"),
        "got: {:?}",
        stdout
    );
    assert!(
        !stdout.contains("third never runs"),
        "later assertions must not run; got: {:?}",
        stdout
    );
}

//! v1.x followups: `std::str::lower` / `std::str::upper` ASCII
//! case folding primitives, plus a hardening of f-string
//! interpolation to handle string literals nested inside `{...}`.
//!
//! Case folding mirrors Rust's `to_ascii_lowercase` / `_uppercase`:
//! only the 26 ASCII letters flip; non-ASCII bytes pass through.
//! The C runtime version (`lotus_str_lower`/`_upper`) allocates the
//! result in the bus payload arena so it lives for the program.
//!
//! The f-string change: the inner interpolation-capture loop now
//! tracks quote state and processes `\"` escapes, so an inline
//! `f"... {std::str::upper(\"abc\")} ..."` parses cleanly. Empty
//! interpolation `{}` still rejects with the original diagnostic;
//! mismatched braces still error per the existing rules.

use std::process::Command;

use aperio_codegen::build_executable;

fn build_and_run(name: &str, source: &str) -> (String, std::process::ExitStatus) {
    let program = aperio_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("aperio_test_case_fstring_{}", name));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (String::from_utf8_lossy(&output.stdout).to_string(), output.status)
}

#[test]
fn lower_flips_ascii_uppercase_only() {
    let src = r#"
        fn main() {
            println(std::str::lower("Hello, World!"));
        }
    "#;
    let (stdout, status) = build_and_run("lower_basic", src);
    assert!(status.success());
    assert!(stdout.contains("hello, world!"), "got: {:?}", stdout);
}

#[test]
fn upper_flips_ascii_lowercase_only() {
    let src = r#"
        fn main() {
            println(std::str::upper("Hello, World!"));
        }
    "#;
    let (stdout, status) = build_and_run("upper_basic", src);
    assert!(status.success());
    assert!(stdout.contains("HELLO, WORLD!"), "got: {:?}", stdout);
}

#[test]
fn case_fold_passes_through_digits_and_punctuation() {
    let src = r#"
        fn main() {
            println(std::str::upper("abc123XYZ!?_"));
        }
    "#;
    let (stdout, status) = build_and_run("mixed", src);
    assert!(status.success());
    assert!(stdout.contains("ABC123XYZ!?_"), "got: {:?}", stdout);
}

#[test]
fn case_fold_round_trip() {
    let src = r#"
        fn main() {
            let s = "MixedCase";
            let r = std::str::upper(std::str::lower(s));
            println(r);
        }
    "#;
    let (stdout, status) = build_and_run("round_trip", src);
    assert!(status.success());
    assert!(stdout.contains("MIXEDCASE"), "got: {:?}", stdout);
}

#[test]
fn fstring_interp_accepts_nested_string_literal() {
    let src = r#"
        fn main() {
            println(f"upper: {std::str::upper(\"abc\")}");
        }
    "#;
    let (stdout, status) = build_and_run("nested_str", src);
    assert!(status.success());
    assert!(
        stdout.contains("upper: ABC"),
        "f-string interpolation with inline string literal failed; got: {:?}",
        stdout
    );
}

#[test]
fn fstring_interp_accepts_nested_string_with_brace_inside() {
    // The `}` inside the string literal must NOT close the
    // interpolation. Quote-state tracking in lex_fstring's
    // inner loop is the load-bearing piece here.
    let src = r#"
        fn main() {
            let n = 42;
            println(f"got: {std::str::upper(\"a}b\")} n={n}");
        }
    "#;
    let (stdout, status) = build_and_run("nested_brace", src);
    assert!(status.success());
    assert!(
        stdout.contains("got: A}B n=42"),
        "got: {:?}", stdout
    );
}

//! `\xNN` ASCII-byte escape — fills the gap surfaced by the
//! token-efficiency experiment (trial 2 used `"\x01"` as an
//! in-string separator and hit a lex error).
//!
//! Scope is intentionally narrow: ASCII bytes only (0x00..=0x7f).
//! High bytes would UTF-8-encode as 2 bytes under Rust's String
//! invariant and surprise the caller; the lexer rejects them with
//! a message pointing at the std::bytes::* API.

use std::process::Command;

use aperio_codegen::build_executable;

fn build_and_run(name: &str, src: &str) -> (String, std::process::ExitStatus) {
    let program = aperio_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!(
        "aperio_test_str_esc_{}_{}",
        name,
        std::process::id()
    ));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        output.status,
    )
}

#[test]
fn hex_escape_ascii_separator_round_trip() {
    let src = r#"
        fn main() {
            let s = "a\x01b\x01c";
            println("len=", len(s));
            let bytes = std::bytes::from_string(s);
            let mid = std::bytes::at(bytes, 1) or 0;
            println("mid=", mid);
        }
    "#;
    let (stdout, status) = build_and_run("ascii_sep", src);
    assert!(status.success(), "non-zero exit: {:?}", status);
    assert!(stdout.contains("len=5"), "got: {:?}", stdout);
    assert!(stdout.contains("mid=1"), "got: {:?}", stdout);
}

#[test]
fn hex_escape_printable_ascii() {
    let src = r#"
        fn main() {
            let s = "\x48\x69\x21";
            println(s);
            println("len=", len(s));
        }
    "#;
    let (stdout, status) = build_and_run("printable", src);
    assert!(status.success(), "non-zero exit: {:?}", status);
    assert!(stdout.contains("Hi!"), "got: {:?}", stdout);
    assert!(stdout.contains("len=3"), "got: {:?}", stdout);
}

#[test]
fn hex_escape_works_in_fstring() {
    let src = r#"
        fn main() {
            let n = 42;
            let s = f"v\x01{n}\x01end";
            println("len=", len(s));
        }
    "#;
    let (stdout, status) = build_and_run("fstring", src);
    assert!(status.success(), "non-zero exit: {:?}", status);
    // "v" + 0x01 + "42" + 0x01 + "end" = 1 + 1 + 2 + 1 + 3 = 8
    assert!(stdout.contains("len=8"), "got: {:?}", stdout);
}

#[test]
fn hex_escape_high_byte_rejected_with_pointer_to_bytes_api() {
    let src = r#"
        fn main() {
            let s = "\xff";
            println(s);
        }
    "#;
    let program = aperio_syntax::parse_source(src);
    let err = program.err().expect("expected lex error");
    let msg = format!("{:?}", err);
    assert!(msg.contains("ASCII"), "got: {}", msg);
    assert!(msg.contains("std::bytes"), "got: {}", msg);
}

#[test]
fn hex_escape_one_digit_rejected() {
    let src = r#"
        fn main() {
            let s = "\x1";
        }
    "#;
    let program = aperio_syntax::parse_source(src);
    let err = program.err().expect("expected lex error");
    let msg = format!("{:?}", err);
    assert!(msg.contains("two hex digits"), "got: {}", msg);
}

#[test]
fn unresolved_callee_ident_suggests_close_match() {
    // The trial-2 agent renamed `fallback` to `key_fallback` and
    // left dangling references — the Discriminant(N) error left
    // them stranded. The diagnostic now names the unresolved
    // ident and (via substring match on the rename pattern)
    // suggests the right name.
    let src = r#"
        fn key_fallback(e: KeyError) -> Int { return 0; }
        fn main() {
            let _ = key_fallback;
            let x = fallback(1);
            println(x);
        }
    "#;
    let program = aperio_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!(
        "aperio_test_typo_diag_{}",
        std::process::id()
    ));
    let err = aperio_codegen::build_executable(&program, &bin).expect_err("should reject");
    let _ = std::fs::remove_file(&bin);
    let msg = format!("{:?}", err);
    assert!(msg.contains("`fallback`"), "got: {}", msg);
    assert!(msg.contains("did you mean"), "got: {}", msg);
    assert!(msg.contains("key_fallback"), "got: {}", msg);
}

//! v1.x followups: `std::str::repeat`, `std::str::pad_left`,
//! `std::str::pad_right`. Common formatting primitives — repeat
//! for separators / indentation; pad_left / pad_right for table
//! columns and right-aligned numbers.

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn build_and_run(name: &str, source: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(source).expect("parse");
    let bin = harness::unique_bin(&format!("hale_test_str_repeat_pad_{}", name));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (String::from_utf8_lossy(&output.stdout).to_string(), output.status)
}

#[test]
fn repeat_concatenates_n_copies() {
    let src = r#"
        fn main() {
            println(std::str::repeat("ab", 3));
        }
    "#;
    let (stdout, status) = build_and_run("basic", src);
    assert!(status.success());
    assert!(stdout.contains("ababab"), "got: {:?}", stdout);
}

#[test]
fn repeat_zero_returns_empty() {
    let src = r#"
        fn main() {
            println(f"[{std::str::repeat(\"x\", 0)}]");
        }
    "#;
    let (stdout, status) = build_and_run("zero", src);
    assert!(status.success());
    assert!(stdout.contains("[]"), "got: {:?}", stdout);
}

#[test]
fn repeat_negative_returns_empty() {
    let src = r#"
        fn main() {
            println(f"[{std::str::repeat(\"x\", 0 - 5)}]");
        }
    "#;
    let (stdout, status) = build_and_run("neg", src);
    assert!(status.success());
    assert!(stdout.contains("[]"), "got: {:?}", stdout);
}

#[test]
fn repeat_for_separator_lines() {
    // The canonical "draw a horizontal rule" use case.
    let src = r#"
        fn main() {
            println(std::str::repeat("=", 5));
        }
    "#;
    let (stdout, status) = build_and_run("sep", src);
    assert!(status.success());
    assert!(stdout.contains("====="), "got: {:?}", stdout);
}

#[test]
fn pad_left_aligns_to_width() {
    let src = r#"
        fn main() {
            println(f"[{std::str::pad_left(\"42\", 6, \" \")}]");
        }
    "#;
    let (stdout, status) = build_and_run("pad_left_basic", src);
    assert!(status.success());
    assert!(stdout.contains("[    42]"), "got: {:?}", stdout);
}

#[test]
fn pad_right_aligns_to_width() {
    let src = r#"
        fn main() {
            println(f"[{std::str::pad_right(\"hello\", 10, \".\")}]");
        }
    "#;
    let (stdout, status) = build_and_run("pad_right_basic", src);
    assert!(status.success());
    assert!(stdout.contains("[hello.....]"), "got: {:?}", stdout);
}

#[test]
fn pad_no_op_when_already_wide() {
    // No truncation — string returned unchanged when it's already
    // at-or-over the requested width.
    let src = r#"
        fn main() {
            println(f"[{std::str::pad_left(\"already-too-long\", 4, \" \")}]");
        }
    "#;
    let (stdout, status) = build_and_run("no_truncate", src);
    assert!(status.success());
    assert!(stdout.contains("[already-too-long]"), "got: {:?}", stdout);
}

#[test]
fn pad_default_space_when_pad_is_empty() {
    // Empty pad string falls back to space (matches the C side).
    let src = r#"
        fn main() {
            println(f"[{std::str::pad_left(\"x\", 4, \"\")}]");
        }
    "#;
    let (stdout, status) = build_and_run("empty_pad", src);
    assert!(status.success());
    assert!(stdout.contains("[   x]"), "got: {:?}", stdout);
}

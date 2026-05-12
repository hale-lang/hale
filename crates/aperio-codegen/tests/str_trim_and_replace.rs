//! v1.x followups: `std::str::trim` and `std::str::replace`.
//!
//! Both are small ergonomic string primitives that resolve common
//! patterns hand-written across apps: trim is the standard
//! whitespace strip (space / tab / CR / LF), replace is greedy
//! non-overlapping substring replacement. Both anchor results in
//! the bus payload arena (program-lifetime).

use std::process::Command;

use aperio_codegen::build_executable;

fn build_and_run(name: &str, source: &str) -> (String, std::process::ExitStatus) {
    let program = aperio_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("aperio_test_str_trim_replace_{}", name));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (String::from_utf8_lossy(&output.stdout).to_string(), output.status)
}

#[test]
fn trim_strips_leading_and_trailing_whitespace() {
    let src = r#"
        fn main() {
            let s = "   hello world   ";
            println(f"[{std::str::trim(s)}]");
        }
    "#;
    let (stdout, status) = build_and_run("basic", src);
    assert!(status.success());
    assert!(stdout.contains("[hello world]"), "got: {:?}", stdout);
}

#[test]
fn trim_handles_all_ascii_whitespace_kinds() {
    let src = r#"
        fn main() {
            let s = "\t\r\n abc \n\r\t";
            println(f"[{std::str::trim(s)}]");
        }
    "#;
    let (stdout, status) = build_and_run("ws_kinds", src);
    assert!(status.success());
    assert!(stdout.contains("[abc]"), "got: {:?}", stdout);
}

#[test]
fn trim_no_op_for_already_trimmed() {
    let src = r#"
        fn main() {
            println(f"[{std::str::trim(\"unchanged\")}]");
        }
    "#;
    let (stdout, status) = build_and_run("no_op", src);
    assert!(status.success());
    assert!(stdout.contains("[unchanged]"), "got: {:?}", stdout);
}

#[test]
fn trim_returns_empty_for_all_whitespace_input() {
    let src = r#"
        fn main() {
            println(f"[{std::str::trim(\"   \")}]");
        }
    "#;
    let (stdout, status) = build_and_run("all_ws", src);
    assert!(status.success());
    assert!(stdout.contains("[]"), "got: {:?}", stdout);
}

#[test]
fn replace_substitutes_all_occurrences_greedy_forward() {
    let src = r#"
        fn main() {
            let r = std::str::replace("foo bar foo baz", "foo", "FOO");
            println(r);
        }
    "#;
    let (stdout, status) = build_and_run("greedy", src);
    assert!(status.success());
    assert!(stdout.contains("FOO bar FOO baz"), "got: {:?}", stdout);
}

#[test]
fn replace_handles_different_length_replacement() {
    // Shrinking and growing both — the out-length precomputation
    // must handle both directions.
    let src = r#"
        fn main() {
            // shrink: "abcabc" → "12c12c" (2 -> 2, same)
            //         "aaaa" with a -> "" → ""
            println(std::str::replace("aaaa", "a", ""));
            // grow: "ab" with a -> "ZZZ" → "ZZZb"
            println(std::str::replace("ab", "a", "ZZZ"));
        }
    "#;
    let (stdout, status) = build_and_run("size_change", src);
    assert!(status.success());
    let lines: Vec<&str> = stdout.lines().collect();
    assert!(lines.iter().any(|l| l.trim().is_empty() || *l == ""),
        "shrink-to-empty line missing; got: {:?}", stdout);
    assert!(stdout.contains("ZZZb"), "got: {:?}", stdout);
}

#[test]
fn replace_empty_needle_is_no_op() {
    let src = r#"
        fn main() {
            println(std::str::replace("hello", "", "X"));
        }
    "#;
    let (stdout, status) = build_and_run("empty_needle", src);
    assert!(status.success());
    assert!(stdout.contains("hello"), "got: {:?}", stdout);
    assert!(!stdout.contains("X"), "empty needle should be no-op; got: {:?}", stdout);
}

#[test]
fn replace_no_match_returns_input_unchanged() {
    let src = r#"
        fn main() {
            println(std::str::replace("hello", "xyz", "ABC"));
        }
    "#;
    let (stdout, status) = build_and_run("no_match", src);
    assert!(status.success());
    assert!(stdout.contains("hello"), "got: {:?}", stdout);
    assert!(!stdout.contains("ABC"), "got: {:?}", stdout);
}

//! std::iter::Lines — cursor-shape iteration over a
//! newline-separated String.
//!
//! Validates the surface contract before any app migrations
//! land: `next_idx` / `line_at` / `is_skippable` behave per the
//! styleguide and the proposal's use-site sketch.

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn build_and_run(name: &str, source: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(source).expect("parse");
    let bin = harness::unique_bin(&format!("hale_test_stdlib_iter_{}", name));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (String::from_utf8_lossy(&output.stdout).to_string(), output.status)
}

#[test]
fn lines_walks_three_line_string_with_trailing_newline() {
    let src = r#"
        fn main() {
            let it = std::iter::Lines { };
            let s = "alpha\nbeta\ngamma\n";
            let mut from = 0;
            while from >= 0 {
                let line = it.line_at(s, from);
                from = it.next_idx(s, from);
                if it.is_skippable(line) { continue; }
                println("L=", line);
            }
        }
    "#;
    let (stdout, status) = build_and_run("trailing_nl", src);
    assert!(status.success());
    assert!(stdout.contains("L=alpha"), "got: {:?}", stdout);
    assert!(stdout.contains("L=beta"),  "got: {:?}", stdout);
    assert!(stdout.contains("L=gamma"), "got: {:?}", stdout);
}

#[test]
fn lines_handles_missing_trailing_newline() {
    let src = r#"
        fn main() {
            let it = std::iter::Lines { };
            let s = "first\nsecond\nthird";
            let mut from = 0;
            while from >= 0 {
                let line = it.line_at(s, from);
                from = it.next_idx(s, from);
                if it.is_skippable(line) { continue; }
                println("L=", line);
            }
        }
    "#;
    let (stdout, status) = build_and_run("no_trailing_nl", src);
    assert!(status.success());
    assert!(stdout.contains("L=first"),  "got: {:?}", stdout);
    assert!(stdout.contains("L=second"), "got: {:?}", stdout);
    assert!(stdout.contains("L=third"),  "got: {:?}", stdout);
}

#[test]
fn lines_skips_blank_and_comment_lines_via_is_skippable() {
    let src = r#"
        fn main() {
            let it = std::iter::Lines { };
            let s = "alpha\n\n# comment\nbeta\n";
            let mut from = 0;
            while from >= 0 {
                let line = it.line_at(s, from);
                from = it.next_idx(s, from);
                if it.is_skippable(line) { continue; }
                println("L=", line);
            }
        }
    "#;
    let (stdout, status) = build_and_run("skippable", src);
    assert!(status.success());
    assert!(stdout.contains("L=alpha"), "got: {:?}", stdout);
    assert!(stdout.contains("L=beta"),  "got: {:?}", stdout);
    assert!(!stdout.contains("L=#"),    "comment lines must skip; got: {:?}", stdout);
    // Blank lines must not produce an "L=" line of their own:
    assert!(!stdout.contains("L=\n\nL="), "blank lines must skip; got: {:?}", stdout);
}

#[test]
fn lines_empty_string_exits_loop_immediately() {
    let src = r#"
        fn main() {
            let it = std::iter::Lines { };
            let s = "";
            let mut from = 0;
            let mut iters = 0;
            while from >= 0 {
                let line = it.line_at(s, from);
                from = it.next_idx(s, from);
                iters = iters + 1;
                if iters > 5 { break; }
                if it.is_skippable(line) { continue; }
                println("L=", line);
            }
            println("iters=", iters);
        }
    "#;
    let (stdout, status) = build_and_run("empty", src);
    assert!(status.success());
    // One iteration: line_at returns "", next_idx returns -1, loop exits.
    assert!(stdout.contains("iters=1"), "expected single iteration on empty input; got: {:?}", stdout);
}

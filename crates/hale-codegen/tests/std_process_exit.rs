//! `std::process::exit(code)` — end-to-end build+run test.
//! Verifies the spec entry in spec/stdlib.md line 343 is real
//! rather than aspirational.

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn build_and_run(name: &str, source: &str) -> std::process::ExitStatus {
    let program = hale_syntax::parse_source(source).expect("parse");
    let bin = harness::unique_bin(&format!("lotus_test_{}", name));
    build_executable(&program, &bin).expect("build");
    let status = Command::new(&bin).status().expect("run");
    let _ = std::fs::remove_file(&bin);
    status
}

#[test]
fn exit_zero() {
    let src = r#"
fn main() {
    std::process::exit(0);
    println("unreachable");
}
"#;
    let status = build_and_run("exit_zero", src);
    assert_eq!(status.code(), Some(0));
}

#[test]
fn exit_seven() {
    let src = r#"
fn main() {
    std::process::exit(7);
}
"#;
    let status = build_and_run("exit_seven", src);
    assert_eq!(status.code(), Some(7));
}

#[test]
fn exit_diverges_subsequent_statements_dont_run() {
    let src = r#"
fn main() {
    std::process::exit(3);
    std::process::exit(99);
}
"#;
    let status = build_and_run("exit_diverges", src);
    assert_eq!(status.code(), Some(3));
}

#[test]
fn exit_from_helper_fn() {
    let src = r#"
fn die(code: Int) {
    std::process::exit(code);
}

fn main() {
    die(42);
    println("unreachable");
}
"#;
    let status = build_and_run("exit_from_helper", src);
    assert_eq!(status.code(), Some(42));
}

#[test]
fn exit_inside_conditional() {
    let src = r#"
fn main() {
    let n = 5;
    if n > 0 {
        std::process::exit(11);
    }
    println("unreachable");
}
"#;
    let status = build_and_run("exit_in_if", src);
    assert_eq!(status.code(), Some(11));
}

//! `std::io::stdin::read_line` end-to-end build+run test. Pipes
//! input on stdin, verifies the read.

use std::io::Write;
use std::process::{Command, Stdio};

use aperio_codegen::build_executable;

fn build_for_stdin(name: &str, source: &str) -> std::path::PathBuf {
    let program = aperio_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("lotus_test_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

fn run_with_stdin(bin: &std::path::Path, input: &str) -> (String, std::process::ExitStatus) {
    let mut child = Command::new(bin)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let output = child.wait_with_output().expect("wait");
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        output.status,
    )
}

#[test]
fn read_line_strips_trailing_newline() {
    let src = r#"
fn main() {
    let line = std::io::stdin::read_line();
    println("got=[", line, "]");
}
"#;
    let bin = build_for_stdin("stdin_strip_newline", src);
    let (stdout, status) = run_with_stdin(&bin, "hello world\n");
    let _ = std::fs::remove_file(&bin);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("got=[hello world]"), "got: {:?}", stdout);
}

#[test]
fn read_line_returns_empty_on_eof() {
    let src = r#"
fn main() {
    let line = std::io::stdin::read_line();
    let status = std::io::stdin::read_line_status();
    if status == -1 {
        println("eof");
    } else {
        println("got=[", line, "] status=", status);
    }
}
"#;
    let bin = build_for_stdin("stdin_eof", src);
    let (stdout, status) = run_with_stdin(&bin, "");
    let _ = std::fs::remove_file(&bin);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("eof"), "got: {:?}", stdout);
}

#[test]
fn read_line_distinguishes_empty_line_from_eof_via_status() {
    let src = r#"
fn main() {
    let line = std::io::stdin::read_line();
    let s = std::io::stdin::read_line_status();
    println("len=", len(line), " status=", s);
}
"#;
    let bin = build_for_stdin("stdin_empty_line", src);
    // Empty line followed by EOF: status should be 0, len 0.
    let (stdout, _status) = run_with_stdin(&bin, "\n");
    let _ = std::fs::remove_file(&bin);
    assert!(
        stdout.contains("len=0 status=0"),
        "expected empty-line status 0; got: {:?}",
        stdout
    );
}

#[test]
fn read_line_handles_crlf() {
    let src = r#"
fn main() {
    let line = std::io::stdin::read_line();
    println("got=[", line, "]");
}
"#;
    let bin = build_for_stdin("stdin_crlf", src);
    let (stdout, _) = run_with_stdin(&bin, "hello\r\n");
    let _ = std::fs::remove_file(&bin);
    assert!(stdout.contains("got=[hello]"), "got: {:?}", stdout);
}

#[test]
fn read_line_multiple_calls_each_read_one_line() {
    let src = r#"
fn main() {
    let a = std::io::stdin::read_line();
    let b = std::io::stdin::read_line();
    let c = std::io::stdin::read_line();
    println("[", a, "][", b, "][", c, "]");
}
"#;
    let bin = build_for_stdin("stdin_multi", src);
    let (stdout, _) = run_with_stdin(&bin, "one\ntwo\nthree\n");
    let _ = std::fs::remove_file(&bin);
    assert!(stdout.contains("[one][two][three]"), "got: {:?}", stdout);
}

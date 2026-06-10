//! pond P4 stage 1: std::term::is_tty + std::io::stdout::write_bytes.

use std::process::Command;

use hale_codegen::build_executable;

fn build_and_run(name: &str, source: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_test_term_{}", name));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (String::from_utf8_lossy(&output.stdout).to_string(), output.status)
}

#[test]
fn is_tty_false_when_stdout_is_piped() {
    // The test harness captures stdout via a pipe, so is_tty(1) is false.
    let src = r#"
        fn main() {
            if std::term::is_tty(1) { println("tty"); } else { println("not-tty"); }
        }
    "#;
    let (stdout, status) = build_and_run("is_tty", src);
    assert!(status.success(), "exit: {:?}", status);
    assert!(stdout.contains("not-tty"), "piped stdout should not be a tty; got: {:?}", stdout);
}

#[test]
fn write_bytes_writes_and_returns_count() {
    let src = r#"
        fn main() {
            let n = std::io::stdout::write_bytes("hello\n");
            print("n=");
            println(n);
        }
    "#;
    let (stdout, status) = build_and_run("write_count", src);
    assert!(status.success(), "exit: {:?}", status);
    assert!(stdout.contains("hello\n"), "missing the written bytes; got: {:?}", stdout);
    assert!(stdout.contains("n=6"), "expected 6 bytes written; got: {:?}", stdout);
}

#[test]
fn write_bytes_flushes_so_ordering_is_consistent_with_println() {
    // The prelude line-buffers stdout (_IOLBF). write_bytes does a raw
    // write(2) — without the fflush, "B" would land before the buffered
    // "A". The fflush keeps them in source order.
    let src = r#"
        fn main() {
            print("A-");
            let _ = std::io::stdout::write_bytes("B-");
            print("C-");
            let _ = std::io::stdout::write_bytes("D");
            println("");
        }
    "#;
    let (stdout, status) = build_and_run("ordering", src);
    assert!(status.success(), "exit: {:?}", status);
    assert!(
        stdout.contains("A-B-C-D"),
        "write_bytes must fflush so it doesn't reorder past buffered output; got: {:?}",
        stdout
    );
}

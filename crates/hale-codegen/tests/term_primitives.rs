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

#[test]
fn raw_mode_guard_births_and_dissolves_cleanly() {
    // pond P4 stage 3: the RawMode guard locus. With stdin piped (not a
    // tty) raw_enable soft-fails — the program runs unstyled and the
    // dissolve no-ops; the point is the guard instantiates, runs birth +
    // dissolve, and exits clean (the wiring is sound). On a real tty
    // raw_enable activates + registers the atexit restore (verified
    // separately: is_tty(0) is true under a pty), which composes with the
    // exit()-on-panic path (P2) to restore the terminal.
    let src = r#"
        fn main() {
            let raw = std::term::RawMode { };
            println("in-raw-scope");
        }
    "#;
    let (stdout, status) = build_and_run("raw_mode", src);
    assert!(status.success(), "RawMode guard should exit clean on a non-tty; {:?}", status);
    assert!(stdout.contains("in-raw-scope"), "guard body didn't run; got: {:?}", stdout);
}

#[test]
fn size_returns_zero_when_stdout_not_a_tty() {
    // pond P4 stage 2: std::term::size() -> TermSize. Piped stdout isn't a
    // tty, so the ioctl path yields the {0,0} sentinel. (Real dims need a
    // sized terminal; the unpack into the record is what's exercised here.)
    let src = r#"
        fn main() {
            let sz = std::term::size();
            print("cols="); print(sz.cols); print(" rows="); println(sz.rows);
        }
    "#;
    let (stdout, status) = build_and_run("size", src);
    assert!(status.success(), "exit: {:?}", status);
    assert!(stdout.contains("cols=0 rows=0"), "expected {{0,0}} on a non-tty; got: {:?}", stdout);
}

fn run_with_stdin(name: &str, source: &str, stdin: std::process::Stdio) -> String {
    use std::io::Read;
    let program = hale_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_test_term_{}", name));
    build_executable(&program, &bin).expect("build");
    let mut child = Command::new(&bin)
        .stdin(stdin)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("spawn");
    let mut out = String::new();
    child.stdout.take().unwrap().read_to_string(&mut out).unwrap();
    let _ = child.wait();
    let _ = std::fs::remove_file(&bin);
    out
}

#[test]
fn read_byte_returns_the_byte_then_eof() {
    // pond P4 stage 4: std::io::stdin::read_byte(timeout). With a closed
    // (EOF) stdin the first read is -2; with a byte available it's that
    // byte. Use /dev/null (immediate EOF) for the -2 case.
    let src = r#"
        fn main() {
            let b = std::io::stdin::read_byte(0);
            print("b="); println(b);
        }
    "#;
    let eof = run_with_stdin("read_eof", src, std::process::Stdio::null());
    assert!(eof.contains("b=-2"), "closed stdin should read EOF (-2); got: {:?}", eof);
}

//! m75: Aperio surface for filesystem ops.
//!
//! `std::io::fs::*` exposes one-shot file operations as path-
//! call functions (read_file, write_file, file_size,
//! file_exists). No locus-wrapped FileL — Phase-1 file I/O is
//! one-shot enough that locus lifetime would be ceremony around
//! the call. A future milestone that needs streaming reads
//! adds a separate FileL family alongside these.
//!
//! These tests build a tiny Aperio program per case, run it,
//! and assert on stdout / disk state — exercising the full
//! parse → bundle-merge → lower → run path through the
//! `lower_std_io_fs_*` codegen surface.

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use aperio_codegen::build_executable;

fn build_and_run(name: &str, source: &str) -> (String, std::process::ExitStatus) {
    let program = aperio_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("aperio_test_stdlib_fs_{}", name));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (String::from_utf8_lossy(&output.stdout).to_string(), output.status)
}

fn unique_tempfile(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "aperio_fs_test_{}_{}_{}.tmp",
        tag,
        std::process::id(),
        nanos
    ));
    p
}

#[test]
fn aperio_write_file_then_read_it_back_via_std_fs() {
    // Use std::fs from Rust to verify what Aperio wrote — proves
    // the bytes hit the disk through the right path.
    let tmp = unique_tempfile("write_then_read");
    let source = format!(
        r#"
        fn main() {{
            std::io::fs::write_file("{}", "hello from aperio");
        }}
        "#,
        tmp.to_str().unwrap()
    );
    let (_, status) = build_and_run("write_only", &source);
    assert!(status.success());
    let on_disk = std::fs::read_to_string(&tmp).expect("read written file");
    assert_eq!(on_disk, "hello from aperio");
    let _ = std::fs::remove_file(&tmp);
}

#[test]
fn aperio_read_file_returns_full_contents() {
    let tmp = unique_tempfile("read_full");
    std::fs::write(&tmp, "first line\nsecond line\nthird")
        .expect("seed file");

    let source = format!(
        r#"
        fn main() {{
            let s = std::io::fs::read_file("{}");
            println("got=", s);
        }}
        "#,
        tmp.to_str().unwrap()
    );
    let (stdout, status) = build_and_run("read_full", &source);
    let _ = std::fs::remove_file(&tmp);
    assert!(status.success());
    assert!(
        stdout.contains("got=first line\nsecond line\nthird"),
        "expected full file contents in stdout; got: {:?}",
        stdout
    );
}

#[test]
fn aperio_round_trip_write_then_read() {
    // Aperio writes, then reads back the same file in the same
    // program. Confirms that write_file's effect is visible
    // through read_file without any external help.
    let tmp = unique_tempfile("round_trip");
    let source = format!(
        r#"
        fn main() {{
            std::io::fs::write_file("{}", "round-trip payload\nwith newline");
            let s = std::io::fs::read_file("{}");
            println("read=", s);
        }}
        "#,
        tmp.to_str().unwrap(),
        tmp.to_str().unwrap()
    );
    let (stdout, status) = build_and_run("round_trip", &source);
    let _ = std::fs::remove_file(&tmp);
    assert!(status.success());
    assert!(
        stdout.contains("read=round-trip payload\nwith newline"),
        "expected round-tripped payload; got: {:?}",
        stdout
    );
}

#[test]
fn aperio_file_size_returns_byte_count() {
    let tmp = unique_tempfile("size");
    std::fs::write(&tmp, "exactly twenty four bytes").expect("seed");
    // "exactly twenty four bytes" is 25 bytes — we'll assert the
    // number rather than recompute.
    let actual = std::fs::metadata(&tmp).expect("stat").len();

    let source = format!(
        r#"
        fn main() {{
            let s = std::io::fs::file_size("{}");
            println("size=", s);
        }}
        "#,
        tmp.to_str().unwrap()
    );
    let (stdout, status) = build_and_run("size", &source);
    let _ = std::fs::remove_file(&tmp);
    assert!(status.success());
    assert!(
        stdout.contains(&format!("size={}", actual)),
        "expected size={}; got: {:?}",
        actual,
        stdout
    );
}

#[test]
fn aperio_file_exists_distinguishes_present_from_absent() {
    let absent = unique_tempfile("absent");
    let present = unique_tempfile("present");
    std::fs::write(&present, "x").expect("seed present file");
    // Don't create `absent`.

    let source = format!(
        r#"
        fn main() {{
            let a = std::io::fs::file_exists("{}");
            let b = std::io::fs::file_exists("{}");
            println("absent=", a);
            println("present=", b);
        }}
        "#,
        absent.to_str().unwrap(),
        present.to_str().unwrap()
    );
    let (stdout, status) = build_and_run("exists", &source);
    let _ = std::fs::remove_file(&present);
    assert!(status.success());
    assert!(
        stdout.contains("absent=false"),
        "absent file should report false; got: {:?}",
        stdout
    );
    assert!(
        stdout.contains("present=true"),
        "present file should report true; got: {:?}",
        stdout
    );
}

#[test]
fn aperio_read_file_on_missing_path_returns_empty_string() {
    // The clamp-on-negative behavior: read_file on a missing
    // path returns "" rather than aborting. Callers that need
    // to distinguish empty-file from missing-file probe with
    // file_exists first.
    let absent = unique_tempfile("missing");
    let source = format!(
        r#"
        fn main() {{
            let s = std::io::fs::read_file("{}");
            let n = std::io::fs::file_size("{}");
            println("s=", s, " n=", n);
        }}
        "#,
        absent.to_str().unwrap(),
        absent.to_str().unwrap()
    );
    let (stdout, status) = build_and_run("missing", &source);
    assert!(status.success());
    // s should be empty; n should be -1 (file_size error).
    assert!(
        stdout.contains("s= n=-1"),
        "expected empty string + size=-1 for missing file; got: {:?}",
        stdout
    );
}

#[test]
fn aperio_extension_isolates_basename_last_dot() {
    // Covers every shape the proposal called out:
    //   - "main.go" → ".go" (the common case)
    //   - "archive.tar.gz" → ".gz" (multiple dots; last wins)
    //   - "Makefile" → "" (no dot)
    //   - "a.b/c" → "" (dot in dir segment, not basename)
    //   - "src/.config" → "" (leading-dot basename is not an ext)
    //   - "src/.config.toml" → ".toml" (leading dot allowed once)
    let source = r#"
        fn main() {
            println("go=", std::io::fs::extension("main.go"));
            println("gz=", std::io::fs::extension("archive.tar.gz"));
            println("make=", std::io::fs::extension("Makefile"));
            println("dirdot=", std::io::fs::extension("a.b/c"));
            println("hidden=", std::io::fs::extension("src/.config"));
            println("toml=", std::io::fs::extension("src/.config.toml"));
        }
    "#;
    let (stdout, status) = build_and_run("extension", source);
    assert!(status.success());
    assert!(stdout.contains("go=.go"),       "main.go → .go; got: {:?}", stdout);
    assert!(stdout.contains("gz=.gz"),       "archive.tar.gz → .gz; got: {:?}", stdout);
    assert!(stdout.contains("make="),        "Makefile → ''; got: {:?}", stdout);
    assert!(!stdout.contains("make=."),      "Makefile must not report a dot; got: {:?}", stdout);
    assert!(stdout.contains("dirdot="),      "a.b/c → ''; got: {:?}", stdout);
    assert!(!stdout.contains("dirdot=."),    "a.b/c must not report a dot; got: {:?}", stdout);
    assert!(stdout.contains("hidden="),      "/.config → ''; got: {:?}", stdout);
    assert!(!stdout.contains("hidden=."),    "/.config must not report a dot; got: {:?}", stdout);
    assert!(stdout.contains("toml=.toml"),   "/.config.toml → .toml; got: {:?}", stdout);
}

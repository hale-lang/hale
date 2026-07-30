//! Phase 2e — list_dir index API.
//!
//! 2e closes `apps/ssg/FRICTION.md` 2026-05-10 list_dir-newline-string:
//! the older newline-joined `list_dir(path) -> String` was removed
//! 2026-05-16. `list_dir_count` + `list_dir_at` route through the
//! same global-arena cache, so iteration becomes a clean
//! `while i < n` bounded by count.

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn build_and_run(name: &str, source: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(source).expect("parse");
    let bin = harness::unique_bin(&format!("hale_test_fsindex_{}", name));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (String::from_utf8_lossy(&output.stdout).to_string(), output.status)
}

fn unique_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let p = harness::unique_bin(&format!(
        "hale_fsindex_{}_{}_{}.d",
        tag,
        std::process::id(),
        nanos
    ));
    std::fs::create_dir_all(&p).expect("mkdir");
    p
}

#[test]
fn list_dir_count_returns_entry_count() {
    let dir = unique_dir("count");
    for name in &["alpha.txt", "beta.md", "gamma.bin"] {
        std::fs::write(dir.join(name), b"x").expect("write");
    }
    let src = format!(
        r#"
        fn main() {{
            let n = std::io::fs::list_dir_count("{}");
            println("count=", n);
        }}
        "#,
        dir.display()
    );
    let (stdout, status) = build_and_run("count_three", &src);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(status.success(), "exit: {:?}", status);
    assert!(stdout.contains("count=3"), "got: {:?}", stdout);
}

#[test]
fn list_dir_at_walks_entries_in_order() {
    // The order of readdir entries isn't specified by POSIX, but
    // for any given directory the count + at pair must agree:
    // every index 0..count returns a valid entry name, and the
    // names collected by at(i) match what list_dir's newline form
    // produces.
    let dir = unique_dir("walk");
    for name in &["a.txt", "b.txt", "c.txt"] {
        std::fs::write(dir.join(name), b"y").expect("write");
    }
    let src = format!(
        r#"
        fn main() {{
            let p = "{}";
            let n = std::io::fs::list_dir_count(p);
            let mut i = 0;
            while i < n {{
                let name = std::io::fs::list_dir_at(p, i);
                println("e", i, "=", name);
                i = i + 1;
            }}
        }}
        "#,
        dir.display()
    );
    let (stdout, status) = build_and_run("walk_three", &src);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(status.success(), "exit: {:?}", status);
    // The test asserts presence, not order — readdir's order is
    // implementation-defined.
    assert!(stdout.contains("e0="), "expected e0= line; got: {:?}", stdout);
    assert!(stdout.contains("e1="), "got: {:?}", stdout);
    assert!(stdout.contains("e2="), "got: {:?}", stdout);
    for name in &["a.txt", "b.txt", "c.txt"] {
        assert!(
            stdout.contains(name),
            "missing {} in: {:?}",
            name,
            stdout
        );
    }
}

#[test]
fn list_dir_count_on_missing_dir_returns_zero() {
    let src = r#"
        fn main() {
            let n = std::io::fs::list_dir_count("/tmp/hale_definitely_missing_xyz123_dir");
            println("count=", n);
        }
    "#;
    let (stdout, status) = build_and_run("missing", src);
    assert!(status.success(), "exit: {:?}", status);
    assert!(stdout.contains("count=0"), "got: {:?}", stdout);
}

#[test]
fn list_dir_at_out_of_range_returns_empty_string() {
    let dir = unique_dir("oob");
    std::fs::write(dir.join("only.txt"), b"z").expect("write");
    let src = format!(
        r#"
        fn main() {{
            let p = "{}";
            let n = std::io::fs::list_dir_count(p);
            println("n=", n);
            let valid = std::io::fs::list_dir_at(p, 0);
            println("valid_len=", len(valid));
            let oob = std::io::fs::list_dir_at(p, 5);
            println("oob_len=", len(oob));
            let neg = std::io::fs::list_dir_at(p, -1);
            println("neg_len=", len(neg));
        }}
        "#,
        dir.display()
    );
    let (stdout, status) = build_and_run("oob", &src);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(status.success(), "exit: {:?}", status);
    assert!(stdout.contains("n=1"), "got: {:?}", stdout);
    assert!(stdout.contains("valid_len=8"), "len('only.txt')=8; got: {:?}", stdout);
    assert!(stdout.contains("oob_len=0"), "got: {:?}", stdout);
    assert!(stdout.contains("neg_len=0"), "got: {:?}", stdout);
}

// read_file_status was removed 2026-05-16 as a backwards-compat
// shim; the modern shape is `read_file(path) -> String fallible(IoError)`
// which carries errno + kind tag on the err path. Use the
// fs_io_error_path.rs tests for the canonical shape.

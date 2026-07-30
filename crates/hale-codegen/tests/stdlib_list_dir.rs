//! std::io::fs::list_dir_count + list_dir_at — directory enumeration.
//!
//! The newline-joined `list_dir(path) -> String` shape was removed
//! 2026-05-16. Iteration is now `for i in 0..count { let name =
//! list_dir_at(path, i); ... }`. Both wrappers walk the same
//! global-arena cache, so iteration is one stat + readdir per path
//! regardless of count.

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn build_hale(name: &str, source: &str) -> std::path::PathBuf {
    let program = hale_syntax::parse_source(source).expect("parse");
    let bin = harness::unique_bin(&format!("hale_test_list_dir_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

fn unique_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let p = harness::unique_bin(&format!(
        "hale_list_dir_{}_{}_{}",
        tag,
        std::process::id(),
        nanos
    ));
    std::fs::create_dir(&p).expect("create dir");
    p
}

#[test]
fn list_dir_returns_each_filename_via_index_api() {
    let dir = unique_dir("three");
    for name in &["alpha.md", "beta.md", "gamma.md"] {
        std::fs::write(dir.join(name), "x").expect("write");
    }

    let src = format!(
        r#"
        fn main() {{
            let n = std::io::fs::list_dir_count("{0}") or raise;
            println("n=", n);
            let mut i = 0;
            while i < n {{
                let name = std::io::fs::list_dir_at("{0}", i) or "";
                println("entry=", name);
                i = i + 1;
            }}
        }}
        "#,
        dir.display()
    );
    let bin = build_hale("three_files", &src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_dir_all(&dir);

    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("n=3"), "got: {:?}", stdout);
    for name in &["alpha.md", "beta.md", "gamma.md"] {
        assert!(
            stdout.contains(name),
            "missing {}; got: {:?}",
            name,
            stdout
        );
    }
}

#[test]
fn list_dir_skips_dot_and_dotdot() {
    let dir = unique_dir("just_dots");
    std::fs::write(dir.join("only_real_entry.txt"), "x").expect("write");

    let src = format!(
        r#"
        fn main() {{
            let n = std::io::fs::list_dir_count("{0}") or raise;
            println("n=", n);
            let mut i = 0;
            while i < n {{
                let name = std::io::fs::list_dir_at("{0}", i) or "";
                println("entry=[", name, "]");
                i = i + 1;
            }}
        }}
        "#,
        dir.display()
    );
    let bin = build_hale("dot_filter", &src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_dir_all(&dir);

    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("n=1"), "got: {:?}", stdout);
    assert!(stdout.contains("only_real_entry.txt"), "got: {:?}", stdout);
    assert!(!stdout.contains("entry=[.]"), "leaked `.`; got: {:?}", stdout);
    assert!(!stdout.contains("entry=[..]"), "leaked `..`; got: {:?}", stdout);
}

#[test]
fn list_dir_on_missing_path_diverges_via_or_raise() {
    // list_dir_count returns fallible(IoError); the `or substitute -1`
    // arm sees the err, lets us report a sentinel.
    let src = r#"
        fn main() {
            let n = std::io::fs::list_dir_count("/tmp/hale_definitely_missing_xyz_91011") or -1;
            println("n=", n);
        }
    "#;
    let bin = build_hale("missing", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("n=-1"), "got: {:?}", stdout);
}

#[test]
fn list_dir_on_empty_dir_returns_zero_count() {
    let dir = unique_dir("empty");
    let src = format!(
        r#"
        fn main() {{
            let n = std::io::fs::list_dir_count("{}") or raise;
            println("n=", n);
        }}
        "#,
        dir.display()
    );
    let bin = build_hale("empty_dir", &src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("n=0"), "got: {:?}", stdout);
}

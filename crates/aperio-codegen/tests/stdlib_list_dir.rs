//! m90: std::io::fs::list_dir — directory enumeration.
//!
//! Returns a single newline-separated String of entry
//! names (skipping `.` and `..`). Phase 5's doc server
//! uses this to enumerate `.md` files in `docs/`. v0
//! limit: filenames containing `\n` corrupt the format —
//! POSIX-legal but extremely rare; documented in stdlib.

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use aperio_codegen::build_executable;

fn build_aperio(name: &str, source: &str) -> std::path::PathBuf {
    let program = aperio_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("aperio_test_list_dir_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

fn unique_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "aperio_list_dir_{}_{}_{}",
        tag,
        std::process::id(),
        nanos
    ));
    std::fs::create_dir(&p).expect("create dir");
    p
}

#[test]
fn list_dir_returns_newline_separated_filenames() {
    // Three known files in a fresh temp dir. The exact
    // order of readdir depends on the filesystem, so the
    // test asserts on presence of each name + on the
    // newline-separator shape.
    let dir = unique_dir("three");
    for name in &["alpha.md", "beta.md", "gamma.md"] {
        std::fs::write(dir.join(name), "x").expect("write");
    }

    let src = format!(
        r#"
        fn main() {{
            let s = std::io::fs::list_dir("{}");
            println("==", s, "==");
            println("len=", len(s));
        }}
        "#,
        dir.display()
    );
    let bin = build_aperio("three_files", &src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_dir_all(&dir);

    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    for name in &["alpha.md", "beta.md", "gamma.md"] {
        assert!(
            stdout.contains(name),
            "missing {}; got: {:?}",
            name,
            stdout
        );
    }
    // Each entry has a trailing `\n`; total length =
    // sum(strlen + 1). 8 + 7 + 8 = 23, plus 3 newlines = 26.
    assert!(
        stdout.contains("len=26"),
        "expected newline-separated length 26; got: {:?}",
        stdout
    );
}

#[test]
fn list_dir_skips_dot_and_dotdot() {
    // Every directory has `.` and `..`. The C primitive
    // filters them so user code doesn't have to.
    let dir = unique_dir("just_dots");
    std::fs::write(dir.join("only_real_entry.txt"), "x").expect("write");

    let src = format!(
        r#"
        fn main() {{
            let s = std::io::fs::list_dir("{}");
            println("==", s, "==");
        }}
        "#,
        dir.display()
    );
    let bin = build_aperio("dot_filter", &src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_dir_all(&dir);

    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("only_real_entry.txt"),
        "got: {:?}",
        stdout
    );
    // Output should be `==only_real_entry.txt\n==\n` — neither
    // a literal "." nor ".." should be present as a separated
    // entry.
    assert!(
        !stdout.contains("==.\n"),
        "leaked `.` entry; got: {:?}",
        stdout
    );
    assert!(
        !stdout.contains("==..\n"),
        "leaked `..` entry; got: {:?}",
        stdout
    );
}

#[test]
fn list_dir_on_missing_path_returns_empty() {
    // Soft-fail like read_file / read_bytes — empty String,
    // user checks via len().
    let src = r#"
        fn main() {
            let s = std::io::fs::list_dir("/tmp/aperio_definitely_missing_xyz_91011");
            println("len=", len(s));
        }
    "#;
    let bin = build_aperio("missing", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("len=0"), "got: {:?}", stdout);
}

#[test]
fn list_dir_on_empty_dir_returns_empty_string() {
    let dir = unique_dir("empty");
    let src = format!(
        r#"
        fn main() {{
            let s = std::io::fs::list_dir("{}");
            println("len=", len(s));
        }}
        "#,
        dir.display()
    );
    let bin = build_aperio("empty_dir", &src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("len=0"), "got: {:?}", stdout);
}

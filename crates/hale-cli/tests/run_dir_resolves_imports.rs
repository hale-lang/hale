//! WS3.3 — `hale run <dir>` resolves cross-seed imports.
//!
//! A directory-seed app that `import`s a vendored library used to
//! work under `hale build <dir>` but NOT under `hale run <dir>`:
//! the run path bundled the directory's files yet silently dropped
//! every `import "..."`, so qualified `alias::Name` references
//! failed codegen ("qualified-name struct literal ... in expression
//! position" / "missing payload type"). This was the pond/fathom
//! "qualified type not in path-renames table" friction on the run
//! path, and the reason a topic decl appeared to need to live in
//! the same file as its publisher.
//!
//! This fixture also splits the topic decl (`topics.hl`) from its
//! publisher (`emitter.hl`) inside the library, so a pass exercises
//! both `hale run`'s import resolution AND cross-file topic
//! resolution end to end.

use std::path::PathBuf;
use std::process::Command;

fn hale_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hale"))
}

fn fixtures_dir() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests");
    p.push("fixtures");
    p
}

#[test]
fn run_dir_resolves_imports_and_cross_file_topic() {
    let app_dir = fixtures_dir().join("ws33-run-dir-app");

    let out = Command::new(hale_bin())
        .arg("run")
        .arg(&app_dir)
        .output()
        .expect("invoke hale run <dir>");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "hale run <dir> failed (import resolution on the run path \
         regressed): status={:?}\nstdout={}\nstderr={}",
        out.status,
        stdout,
        stderr
    );
    // The publisher (emitter.hl) sends on a topic declared in a
    // different file (topics.hl); the subscriber receives both.
    assert!(stdout.contains("got 7"), "missing first publish: {:?}", stdout);
    assert!(stdout.contains("got 11"), "missing second publish: {:?}", stdout);
    assert!(stdout.contains("done"), "missing done sentinel: {:?}", stdout);
}

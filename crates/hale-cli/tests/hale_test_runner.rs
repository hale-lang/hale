//! `hale test` CLI runner — the discovery→compile→run→report
//! driver specified in `spec/testing.md`.
//!
//! These pin the runner's user-visible contract:
//!  - an all-passing directory exits 0 with an "N passed, 0 failed"
//!    summary and an `ok` line per file;
//!  - a directory containing a failing test exits 1, surfaces the
//!    `ASSERTION FAILED` diagnostic, and recurses into subdirs;
//!  - `-run <substr>` filters by path;
//!  - `--json` emits a well-formed array with the expected shape.
//!
//! The `.hl` fixtures live under `tests/fixtures/hale-test-*`.

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
fn all_passing_dir_exits_zero_with_summary() {
    let dir = fixtures_dir().join("hale-test-pass");
    let out = Command::new(hale_bin())
        .arg("test")
        .arg(&dir)
        .output()
        .expect("invoke hale test <dir>");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "all-passing dir must exit 0; status={:?}\nstdout={}\nstderr={}",
        out.status,
        stdout,
        stderr
    );
    assert!(
        stdout.contains("2 passed, 0 failed"),
        "missing summary; stdout={:?}",
        stdout
    );
    assert!(
        stdout.contains("ok   ") && stdout.contains("arith_test.hl"),
        "missing per-file ok line; stdout={:?}",
        stdout
    );
    assert!(
        !stdout.contains("FAIL"),
        "no test should fail here; stdout={:?}",
        stdout
    );
}

#[test]
fn failing_test_exits_one_and_surfaces_diagnostic() {
    let dir = fixtures_dir().join("hale-test-mixed");
    let out = Command::new(hale_bin())
        .arg("test")
        .arg(&dir)
        .output()
        .expect("invoke hale test <dir>");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !out.status.success(),
        "a failing test must make the runner exit non-zero; stdout={:?}",
        stdout
    );
    assert_eq!(out.status.code(), Some(1), "expected exit code 1");
    assert!(
        stdout.contains("1 passed, 1 failed"),
        "missing summary; stdout={:?}",
        stdout
    );
    // The failure lives one directory deep — recursion must find it.
    assert!(
        stdout.contains("FAIL") && stdout.contains("fail_test.hl"),
        "missing FAIL line for nested test; stdout={:?}",
        stdout
    );
    assert!(
        stdout.contains("ASSERTION FAILED: math is broken"),
        "assertion diagnostic must be surfaced; stdout={:?}",
        stdout
    );
    // The non-`_test.hl` sibling (notes.hl) must be ignored.
    assert!(
        !stdout.contains("notes.hl"),
        "non-_test.hl files must not be discovered; stdout={:?}",
        stdout
    );
}

#[test]
fn run_filter_selects_by_substring() {
    let dir = fixtures_dir().join("hale-test-mixed");
    let out = Command::new(hale_bin())
        .arg("test")
        .arg(&dir)
        .arg("-run")
        .arg("pass")
        .output()
        .expect("invoke hale test -run");
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Only pass_test.hl matches "pass"; the failing nested test is
    // filtered out, so the run is all-green.
    assert!(
        out.status.success(),
        "filtered-to-passing run must exit 0; stdout={:?}",
        stdout
    );
    assert!(
        stdout.contains("1 passed, 0 failed"),
        "filter should select exactly one test; stdout={:?}",
        stdout
    );
    assert!(
        !stdout.contains("fail_test.hl"),
        "filtered-out test must not appear; stdout={:?}",
        stdout
    );
}

#[test]
fn json_output_is_well_formed() {
    let dir = fixtures_dir().join("hale-test-mixed");
    let out = Command::new(hale_bin())
        .arg("test")
        .arg(&dir)
        .arg("--json")
        .output()
        .expect("invoke hale test --json");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stdout = stdout.trim();
    assert!(
        stdout.starts_with('[') && stdout.ends_with(']'),
        "json must be an array; got {:?}",
        stdout
    );
    // Shape: {file, status, [message], elapsed_ms} per entry.
    assert!(stdout.contains("\"status\":\"pass\""), "got {:?}", stdout);
    assert!(stdout.contains("\"status\":\"fail\""), "got {:?}", stdout);
    assert!(stdout.contains("\"file\":\""), "got {:?}", stdout);
    assert!(stdout.contains("\"elapsed_ms\":"), "got {:?}", stdout);
    assert!(
        stdout.contains("\"message\":\"ASSERTION FAILED: math is broken"),
        "failure message must be embedded; got {:?}",
        stdout
    );
    // A failing test still means exit 1, even in --json mode.
    assert_eq!(out.status.code(), Some(1));
}

#[test]
fn single_file_target_runs_that_file() {
    let file = fixtures_dir()
        .join("hale-test-pass")
        .join("arith_test.hl");
    let out = Command::new(hale_bin())
        .arg("test")
        .arg(&file)
        .output()
        .expect("invoke hale test <file>");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "stdout={:?}", stdout);
    assert!(
        stdout.contains("1 passed, 0 failed"),
        "single-file run summary; stdout={:?}",
        stdout
    );
}

#[test]
fn no_tests_found_is_not_an_error() {
    // A directory with no `_test.hl` files: "nothing to run" exits
    // 0 with a clear message, not a failure.
    let dir = std::env::temp_dir().join(format!(
        "hale_test_empty_{}_{}",
        std::process::id(),
        "notests"
    ));
    let _ = std::fs::create_dir_all(&dir);
    let out = Command::new(hale_bin())
        .arg("test")
        .arg(&dir)
        .output()
        .expect("invoke hale test <empty dir>");
    let _ = std::fs::remove_dir_all(&dir);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "no tests must exit 0; status={:?} stdout={:?}",
        out.status,
        stdout
    );
    assert!(
        stdout.contains("no `_test.hl` files found"),
        "expected a clear no-tests message; stdout={:?}",
        stdout
    );
}

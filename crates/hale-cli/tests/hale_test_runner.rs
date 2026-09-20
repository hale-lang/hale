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
//!  - GH #717: a failed assertion runs the test's own locus teardown
//!    before the process exits, so a fixture that started a child and
//!    created scratch it owns leaves neither behind.
//!
//! The `.hl` fixtures live under `tests/fixtures/hale-test-*`; the
//! GH #717 pair is generated into a per-run temp directory because the
//! fixture has to name absolute paths it owns.

use std::path::{Path, PathBuf};
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

// ---------------------------------------------------------------
// GH #717 — deterministic fixture cleanup on assertion failure.
// ---------------------------------------------------------------

/// A `_test.hl` program that owns two real resources — a `sleep 30`
/// child it spawned and a scratch directory it created — and whose
/// `dissolve()` releases exactly those two, nothing else. `fail` picks
/// whether its single assertion fails.
///
/// Braces are doubled because this is a `format!` template. That also
/// keeps `hale-corpus`'s embedded-program harvester off it: a literal
/// with a live `{}` placeholder is a template awaiting values, not a
/// corpus program.
fn cleanup_fixture(scratch: &Path, pidfile: &Path, fail: bool) -> String {
    format!(
        r#"// GH #717: a fixture that owns a child process and a scratch dir.
locus Fixture {{
    params {{
        pid: Int = -1;
        dir: String = "";
    }}
    dissolve() {{
        // The only cleanup this program is entitled to: the child it
        // started and the scratch directory it created.
        let _stop = std::process::run(
            f"sh\n-c\nkill -9 {{self.pid}} 2>/dev/null; rm -rf {{self.dir}}"
        ) or std::process::ProcessOutput {{ code: -1, signal: 0, stdout: "", stderr: "" }};
    }}
}}

fn main() {{
    let dir = "{scratch}";
    std::io::fs::mkdir(dir) or discard;
    std::io::fs::write_file(dir + "/owned.txt", "scratch\n") or discard;
    let c = std::process::spawn("sleep\n30") or raise;
    let f = Fixture {{ pid: c.pid, dir: dir }};
    std::io::fs::write_file("{pidfile}", f"{{c.pid}}") or discard;
    std::test::assert(1 == {rhs}, "deliberate failure after spawning a child");
    {tail}
}}
"#,
        scratch = scratch.display(),
        pidfile = pidfile.display(),
        rhs = if fail { 2 } else { 1 },
        // On the failing variant this line must NOT run — it is the
        // "the run continued past the first failure" detector. On the
        // passing variant it must run and stay silent (a passing test
        // that writes to stdout is itself a runner failure).
        tail = if fail {
            "println(\"UNREACHABLE: execution continued past a failed assertion\");"
        } else {
            "std::test::assert(true, \"reached the end\");"
        },
    )
}

/// A per-run directory to generate the fixture into. Unique by pid so
/// parallel shards never share one.
fn gh717_root(tag: &str) -> PathBuf {
    let p = std::env::temp_dir()
        .join(format!("hale_test_717_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).expect("create the GH #717 fixture root");
    p
}

/// A pid counts as alive while `/proc/<pid>/cmdline` is readable AND
/// still names the command the fixture spawned. The cmdline check makes
/// the poll immune to pid reuse — a recycled pid is some other program,
/// not our orphan.
fn gh717_child_alive(pid: u32) -> bool {
    match std::fs::read(format!("/proc/{}/cmdline", pid)) {
        Ok(raw) => String::from_utf8_lossy(&raw).contains("sleep"),
        Err(_) => false,
    }
}

/// Poll until the pid is gone or the deadline passes.
fn gh717_wait_for_exit(pid: u32) -> bool {
    let deadline =
        std::time::Instant::now() + std::time::Duration::from_secs(5);
    while gh717_child_alive(pid) && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    !gh717_child_alive(pid)
}

#[test]
fn failed_assertion_dissolves_the_fixtures_loci_before_exit() {
    // GH #717: the assertion used to call `std::process::exit(1)` from
    // inside `std::test::assert`, which jumped over `fn main`'s
    // teardown — the `sleep 30` child stayed alive and the scratch
    // directory survived into the next run. The assertion now records
    // the failure and main leaves through its ordinary teardown.
    let root = gh717_root("fail_cleanup");
    let scratch = root.join("scratch");
    let pidfile = root.join("child.pid");
    std::fs::write(
        root.join("cleanup_test.hl"),
        cleanup_fixture(&scratch, &pidfile, true),
    )
    .expect("write the failing fixture");

    let out = Command::new(hale_bin())
        .arg("test")
        .arg(&root)
        .output()
        .expect("invoke hale test <generated dir>");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();

    // Collect the evidence, then clean up, then assert — a failing
    // assertion below must not leave an orphan on a shared machine.
    let pid: Option<u32> = std::fs::read_to_string(&pidfile)
        .ok()
        .and_then(|s| s.trim().parse().ok());
    let child_gone = pid.map(gh717_wait_for_exit);
    let scratch_gone = !scratch.exists();
    if let Some(p) = pid {
        if gh717_child_alive(p) {
            let _ =
                Command::new("kill").arg("-9").arg(p.to_string()).status();
        }
    }
    let _ = std::fs::remove_dir_all(&root);

    // 1. The run still fails, with the status the runner always gave.
    assert_eq!(
        out.status.code(),
        Some(1),
        "a failing fixture must still exit 1; stdout={:?} stderr={:?}",
        stdout,
        stderr
    );
    assert!(
        stdout.contains("0 passed, 1 failed"),
        "missing summary; stdout={:?}",
        stdout
    );
    // 2. The failure's FIRST line is unchanged.
    let first = stdout
        .lines()
        .skip_while(|l| !l.starts_with("FAIL "))
        .nth(1)
        .unwrap_or("")
        .trim()
        .to_string();
    assert_eq!(
        first, "ASSERTION FAILED: deliberate failure after spawning a child",
        "the first line of a failure must be unchanged; stdout={:?}",
        stdout
    );
    // 3. The first failure still stops the run — nothing after the
    //    failed assertion executed.
    assert!(
        !stdout.contains("UNREACHABLE"),
        "execution continued past the first failure; stdout={:?}",
        stdout
    );
    // 4. The child the fixture spawned is gone: main's teardown ran,
    //    so Fixture.dissolve() stopped it.
    assert_eq!(
        child_gone,
        Some(true),
        "the fixture's child survived the failed assertion (pid={:?}) — \
         main's teardown did not run; stdout={:?}",
        pid,
        stdout
    );
    // 5. And so is the scratch it owned.
    assert!(
        scratch_gone,
        "scratch dir {} survived the failed assertion — it would \
         contaminate the next run",
        scratch.display()
    );
}

#[test]
fn passing_fixture_with_a_child_and_scratch_still_passes_silently() {
    // The control for GH #717: the same fixture with a passing
    // assertion is untouched — exit 0, no stdout, and the fall-through
    // teardown it always had still releases what it owns.
    let root = gh717_root("pass_cleanup");
    let scratch = root.join("scratch");
    let pidfile = root.join("child.pid");
    std::fs::write(
        root.join("cleanup_test.hl"),
        cleanup_fixture(&scratch, &pidfile, false),
    )
    .expect("write the passing fixture");

    let out = Command::new(hale_bin())
        .arg("test")
        .arg(&root)
        .output()
        .expect("invoke hale test <generated dir>");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();

    let pid: Option<u32> = std::fs::read_to_string(&pidfile)
        .ok()
        .and_then(|s| s.trim().parse().ok());
    let child_gone = pid.map(gh717_wait_for_exit);
    let scratch_gone = !scratch.exists();
    if let Some(p) = pid {
        if gh717_child_alive(p) {
            let _ =
                Command::new("kill").arg("-9").arg(p.to_string()).status();
        }
    }
    let _ = std::fs::remove_dir_all(&root);

    assert!(
        out.status.success(),
        "a passing fixture must still exit 0; stdout={:?} stderr={:?}",
        stdout,
        stderr
    );
    assert!(
        stdout.contains("1 passed, 0 failed"),
        "missing summary; stdout={:?}",
        stdout
    );
    assert!(
        !stdout.contains("FAIL"),
        "nothing should fail here; stdout={:?}",
        stdout
    );
    assert_eq!(
        child_gone,
        Some(true),
        "the fixture's child survived a PASSING run (pid={:?})",
        pid
    );
    assert!(
        scratch_gone,
        "scratch dir {} survived a passing run",
        scratch.display()
    );
}

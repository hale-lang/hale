//! GH #18 item 1 — the memory-bound-proof warning is OPT-IN.
//!
//! "Bounded per epoch" only means something for a long-lived process
//! (a daemon, a bus handler, a persistent locus). A script that
//! allocates and exits owes the proof nothing, so it pays nothing by
//! default — the same descent-curve stance as the `@locality`
//! cache-tier budgets (annotation/flag-gated, never automatic). This
//! pins that contract so a future refactor can't silently re-enable
//! the former default-on behavior:
//!
//!   - default: silent (no warning), build succeeds
//!   - `--warn-unbounded-alloc`: emits the advisory warning, build
//!     still succeeds (a warning, not an error)
//!   - `--no-warn-unbounded-alloc`: accepted-and-ignored for
//!     back-compat with the former default-on flag
//!
//! The fixture (`fixtures/unbounded-alloc-opt-in/app.hl`) is the
//! canonical unbounded-accumulation shape: a per-message handler that
//! whole-value-replaces a struct into `self` each message.

use std::path::PathBuf;
use std::process::Command;

fn hale_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hale"))
}

fn app() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests");
    p.push("fixtures");
    p.push("unbounded-alloc-opt-in");
    p.push("app.hl");
    p
}

fn check(extra: &[&str]) -> (bool, String) {
    let out = Command::new(hale_bin())
        .arg("check")
        .arg(app())
        .args(extra)
        .output()
        .expect("invoke hale check");
    (out.status.success(), String::from_utf8_lossy(&out.stderr).into_owned())
}

const WARN_NEEDLE: &str = "unbounded allocation";

#[test]
fn default_is_silent() {
    let (ok, stderr) = check(&[]);
    assert!(ok, "default check should succeed: {stderr}");
    assert!(
        !stderr.contains(WARN_NEEDLE),
        "memory-bound warning must be OPT-IN — default run leaked it:\n{stderr}"
    );
}

#[test]
fn warn_flag_emits_advisory_but_succeeds() {
    let (ok, stderr) = check(&["--warn-unbounded-alloc"]);
    assert!(
        stderr.contains(WARN_NEEDLE),
        "--warn-unbounded-alloc should emit the advisory warning:\n{stderr}"
    );
    assert!(
        ok,
        "the warning is advisory — it must not fail the build:\n{stderr}"
    );
}

#[test]
fn no_warn_flag_is_accepted_noop() {
    let (ok, stderr) = check(&["--no-warn-unbounded-alloc"]);
    assert!(ok, "--no-warn-unbounded-alloc should be accepted: {stderr}");
    assert!(
        !stderr.contains(WARN_NEEDLE),
        "back-compat opt-out must stay silent:\n{stderr}"
    );
}

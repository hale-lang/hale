//! M3 stage 5 (2026-07-02) — the memory-bound-proof warning is
//! DEFAULT-ON (Riley's flip after the 402-warning audit; see
//! notes/unbounded-alloc-audit-2026-07-02.md). Run-to-exit programs
//! (a `main`, no run loop, no bus handler) still warn nothing —
//! the analysis itself spares them, so scripts pay nothing.
//!
//!   - default: emits the advisory warning, build still succeeds
//!     (a warning, not an error)
//!   - `--no-warn-unbounded-alloc`: the opt-OUT, silent
//!   - `--warn-unbounded-alloc`: accepted-and-ignored (the former
//!     opt-in spelling)
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
fn default_emits_advisory_but_succeeds() {
    let (ok, stderr) = check(&[]);
    assert!(
        stderr.contains(WARN_NEEDLE),
        "default-on: the advisory warning must print without a flag:\n{stderr}"
    );
    assert!(
        ok,
        "the warning is advisory — it must not fail the build:\n{stderr}"
    );
}

#[test]
fn warn_flag_is_accepted_redundant() {
    let (ok, stderr) = check(&["--warn-unbounded-alloc"]);
    assert!(
        stderr.contains(WARN_NEEDLE),
        "the former opt-in spelling stays accepted:\n{stderr}"
    );
    assert!(ok, "advisory only: {stderr}");
}

#[test]
fn no_warn_flag_opts_out() {
    let (ok, stderr) = check(&["--no-warn-unbounded-alloc"]);
    assert!(ok, "--no-warn-unbounded-alloc should be accepted: {stderr}");
    assert!(
        !stderr.contains(WARN_NEEDLE),
        "the opt-out must silence the survey:\n{stderr}"
    );
}

// #8 (2026-07-02): `--json` NDJSON diagnostics — the LSP-groundwork
// contract. Reuses this fixture (it has a default-on warning).
#[test]
fn json_mode_emits_ndjson_on_stdout() {
    let out = Command::new(hale_bin())
        .arg("check")
        .arg(app())
        .arg("--json")
        .output()
        .expect("invoke hale check --json");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.lines().count() >= 1,
        "at least one diagnostic line: {stdout}"
    );
    for line in stdout.lines() {
        assert!(
            line.starts_with('{') && line.ends_with('}'),
            "NDJSON object per line: {line}"
        );
        for key in ["\"file\":", "\"line\":", "\"col\":", "\"severity\":", "\"message\":"] {
            assert!(line.contains(key), "missing {key} in {line}");
        }
    }
    assert!(
        out.status.success(),
        "warnings are advisory in json mode too"
    );
}

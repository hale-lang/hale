//! GH #18 item 1, Phase B — `@bounded` opts a locus into the
//! memory-bound proof in-source; `@unbounded` carves a fn/hook out.
//!
//! Phase A made the proof opt-in (silent unless `--warn-unbounded-alloc`).
//! Phase B adds the annotation surface so a long-lived locus can request
//! the proof for itself — the descent-curve dual of the whole-program
//! flag — without making scripts pay. This pins the end-to-end contract:
//!
//!   - a `@bounded` locus emits its leak warnings on a plain `hale check`,
//!     no flag needed
//!   - `@unbounded` on a method (or lifecycle hook) suppresses that body's
//!     sites, with or without the survey flag
//!   - a program with no `@bounded` locus stays silent by default
//!     (Phase A behavior preserved)
//!   - warnings remain advisory — they never fail the build

use std::path::PathBuf;
use std::process::Command;

fn hale_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hale"))
}

fn fixture(name: &str) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests");
    p.push("fixtures");
    p.push("bounded-annotation");
    p.push(name);
    p
}

fn check(file: &str, extra: &[&str]) -> (bool, String) {
    let out = Command::new(hale_bin())
        .arg("check")
        .arg(fixture(file))
        .args(extra)
        .output()
        .expect("invoke hale check");
    (out.status.success(), String::from_utf8_lossy(&out.stderr).into_owned())
}

const NEEDLE: &str = "unbounded allocation";

#[test]
fn bounded_locus_warns_by_default_carved_method_silent() {
    // `app.hl`: a `@bounded` locus with two handlers — `on_sample`
    // (flagged) and `@unbounded on_other` (carved out).
    let (ok, stderr) = check("app.hl", &[]);
    assert!(ok, "warnings are advisory, build must succeed:\n{stderr}");
    assert!(
        stderr.contains(NEEDLE),
        "@bounded locus should warn on a plain check:\n{stderr}"
    );
    assert!(
        stderr.contains("on_sample"),
        "the flagged handler should be named:\n{stderr}"
    );
    assert!(
        !stderr.contains("on_other"),
        "@unbounded handler must be carved out:\n{stderr}"
    );
}

#[test]
fn carve_out_holds_under_survey_flag() {
    let (_ok, stderr) = check("app.hl", &["--warn-unbounded-alloc"]);
    assert!(
        !stderr.contains("on_other"),
        "@unbounded suppresses even under the survey flag:\n{stderr}"
    );
}

#[test]
fn no_bounded_locus_is_silent_by_default() {
    // M3 stage 5 flip (2026-07-02): the survey is DEFAULT-ON — a
    // plain locus with a real accumulation warns without any flag
    // or `@bounded` annotation. The opt-OUT silences it.
    let (ok, stderr) = check("plain.hl", &[]);
    assert!(ok, "{stderr}");
    assert!(
        stderr.contains(NEEDLE),
        "default-on: the leak shape must warn without @bounded:\n{stderr}"
    );
    let (ok, stderr) = check("plain.hl", &["--no-warn-unbounded-alloc"]);
    assert!(ok, "{stderr}");
    assert!(
        !stderr.contains(NEEDLE),
        "--no-warn-unbounded-alloc must silence the survey:\n{stderr}"
    );
}

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

// ---- GH #1490: `--report-unbounded-use` --------------------------------
//
// One stdout line per `@unbounded` fn or hook: where the decorator is,
// the fn, the survey sites and the hot-path warnings it silences. The
// counts are what a plain check would warn in that fn were the decorator
// removed, which `report_counts_what_removal_would_warn` holds them to.

/// `hale check <fixture> <extra>`, run from the fixture directory so the
/// report names the file as `<file>:<line>`: the exit status, stdout and
/// stderr.
fn run_in_fixtures(file: &str, extra: &[&str]) -> (Option<i32>, String, String) {
    let dir = fixture(file).parent().unwrap().to_path_buf();
    let out = Command::new(hale_bin())
        .current_dir(&dir)
        .arg("check")
        .arg(file)
        .args(extra)
        .output()
        .expect("invoke hale check");
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn report(file: &str) -> (Option<i32>, String, String) {
    run_in_fixtures(file, &["--report-unbounded-use"])
}

#[test]
fn report_names_what_each_decorator_silences() {
    let (code, stdout, _) = report("unbounded-use.hl");
    assert_eq!(code, Some(0), "the report changes no exit code");
    assert_eq!(
        stdout,
        "unbounded-use\tunbounded-use.hl:17\tAcc::on_sample\t2\t0\n\
         unbounded-use\tunbounded-use.hl:22\tAcc::on_quiet\t0\t0\n\
         unbounded-use\tunbounded-use.hl:29\tspawn_many\t1\t1\n\
         unbounded-use\tunbounded-use.hl:41\tFan::run\t0\t1\n",
        "survey sites silenced (on_sample), nothing silenced (on_quiet), a \
         hot-path warning silenced (spawn_many), and a hook's, at its \
         decorator's line (Fan::run)"
    );
}

#[test]
fn report_in_a_run_to_exit_program_silences_no_survey_site() {
    // The same whole-value replace that `on_sample` silences, in a
    // program that runs to exit: the survey reports nothing there.
    let (code, stdout, _) = report("unbounded-use-script.hl");
    assert_eq!(code, Some(0));
    assert_eq!(stdout, "unbounded-use\tunbounded-use-script.hl:7\tAcc::put\t0\t0\n");
}

#[test]
fn report_changes_nothing_the_check_prints() {
    let (code, _, with) = report("unbounded-use.hl");
    let (code_without, stdout_without, without) = run_in_fixtures("unbounded-use.hl", &[]);
    assert_eq!((code, with), (code_without, without), "the diagnostics and the exit code are the check's");
    assert_eq!(stdout_without, "", "without the flag, no report");
}

#[test]
fn report_counts_what_removal_would_warn() {
    // The fixture with every `@unbounded` removed warns exactly what the
    // report says the decorators silence: 3 survey sites, 2 hot-path
    // warnings.
    let src = std::fs::read_to_string(fixture("unbounded-use.hl")).unwrap();
    let bare: String = src
        .lines()
        .map(|l| if l.trim() == "@unbounded" { "" } else { l })
        .collect::<Vec<_>>()
        .join("\n");
    let dir = std::env::temp_dir().join(format!("hale_1490_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("bare.hl");
    std::fs::write(&path, bare).unwrap();
    let out = Command::new(hale_bin()).arg("check").arg(&path).output().expect("invoke hale check");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let _ = std::fs::remove_dir_all(&dir);
    let count = |needle: &str| stderr.lines().filter(|l| l.contains(needle)).count();
    assert_eq!(count("warning: unbounded allocation"), 3, "{stderr}");
    assert_eq!(count("warning: hot-path allocation"), 2, "{stderr}");
}

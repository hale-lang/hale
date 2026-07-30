//! The committed effect baseline must match what the compiler
//! currently infers.
//!
//! GH #265 shipped `--dump-effects-manifest` / `--check-effects-
//! manifest` as a CI gate against effect regressions. It was
//! exercised on two toy inputs in `effects_manifest.rs`, and this
//! repo had **no committed baseline** — so the gate guarded nothing
//! here. A regression gate pointed at nothing is a gate in name.
//!
//! `.effects-baseline/corpus.effects` is that baseline: the inferred
//! effect set of every function in every in-tree example. This test
//! is the gate, and it runs in the normal suite rather than only in
//! CI so the diff shows up while you still have the context to judge
//! it.
//!
//! When it fails, read the diff before regenerating. It says: some
//! function's effects changed. Either that was the point — in which
//! case `scripts/effects-baseline.sh` and commit, and the diff is
//! your review artifact — or something started doing I/O that
//! didn't before, which is precisely the regression annotations
//! cannot catch, because nothing in the annotated source changed.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p
}

fn hale_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hale"))
}

fn manifest_for(main_hl: &Path) -> String {
    let out = Command::new(hale_bin())
        .arg("check")
        .arg(main_hl)
        .arg("--dump-effects-manifest")
        .output()
        .expect("invoke hale check --dump-effects-manifest");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.starts_with("ok:") && !l.starts_with("# .hale.effects"))
        .map(|l| format!("{}\n", l))
        .collect()
}

fn current() -> String {
    let root = repo_root();
    let examples = root.join("crates/hale-codegen/tests/fixtures/examples");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&examples)
        .expect("read examples dir")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("main.hl").is_file())
        .collect();
    dirs.sort();
    let mut out = String::new();
    for d in dirs {
        let name = d.file_name().unwrap().to_string_lossy().to_string();
        out.push_str(&format!("### {}\n", name));
        out.push_str(&manifest_for(&d.join("main.hl")));
    }
    out
}

fn baseline_body() -> String {
    let p = repo_root().join(".effects-baseline/corpus.effects");
    let text = std::fs::read_to_string(&p).unwrap_or_else(|e| {
        panic!("missing baseline at {}: {} — run scripts/effects-baseline.sh", p.display(), e)
    });
    text.lines()
        // `###` marks a program section and IS content; only `# `
        // comment lines are stripped.
        .filter(|l| !(l.starts_with('#') && !l.starts_with("###")))
        .filter(|l| !l.trim().is_empty())
        .map(|l| format!("{}\n", l))
        .collect()
}

#[test]
fn corpus_effects_match_the_committed_baseline() {
    let have: Vec<String> =
        current().lines().map(|s| s.to_string()).filter(|l| !l.trim().is_empty()).collect();
    let want: Vec<String> =
        baseline_body().lines().map(|s| s.to_string()).collect();

    if have == want {
        return;
    }
    // Report as a line diff — the whole value of the artifact is that
    // a behavioural change is a small, readable diff.
    let mut added = Vec::new();
    let mut removed = Vec::new();
    for l in &have {
        if !want.contains(l) {
            added.push(l.clone());
        }
    }
    for l in &want {
        if !have.contains(l) {
            removed.push(l.clone());
        }
    }
    panic!(
        "the corpus effect fingerprint changed.\n\n\
         + now:      {:#?}\n\
         - baseline: {:#?}\n\n\
         If intended, regenerate with `scripts/effects-baseline.sh` and \
         commit — the diff is the review artifact. If not, some function \
         gained an effect it should not have.",
        &added[..added.len().min(30)],
        &removed[..removed.len().min(30)],
    );
}

/// A baseline that had drifted to empty would make the gate pass on
/// anything.
#[test]
fn baseline_is_substantial() {
    let body = baseline_body();
    let rows = body.lines().filter(|l| l.contains("does={")).count();
    assert!(
        rows > 100,
        "baseline carries only {} effect rows — it is not fingerprinting \
         the corpus",
        rows
    );
    let programs = body.lines().filter(|l| l.starts_with("### ")).count();
    assert!(
        programs > 80,
        "baseline covers only {} programs",
        programs
    );
}

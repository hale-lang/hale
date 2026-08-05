//! #392 thread 2 — library-tier claims: a top-level `claims { }`
//! block in a library seed travels with the import and re-evaluates
//! in every closing build.
//!
//! The fixture is the motivating shape: the payments seed swears
//! `count subscribers(topic Charges) <= 1` about its own boundary.
//! Checked alone, it holds (its `Wire` is the one settler). The app
//! then quietly wires a second subscriber — legal bus plumbing —
//! and the library's own law refuses the closing build, attributed
//! (`pay::single_settle`) and pointing at the library's source.

use std::path::PathBuf;
use std::process::Command;

fn check(rel: &str) -> String {
    let target = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/xseed-library-claims")
        .join(rel);
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("check")
        .arg(&target)
        .output()
        .expect("run hale check");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// The library alone satisfies its own law.
#[test]
fn the_library_holds_in_its_own_world() {
    let out = check("lib/pay");
    assert!(
        !out.contains("violated") && out.contains("ok"),
        "the seed's own world satisfies its claims:\n{}",
        out
    );
}

/// CANARY — the closing build re-evaluates the traveled claim over
/// the merged world and refuses the app's second subscriber, with
/// seed attribution and demangled names.
#[test]
fn the_traveled_claim_refuses_the_closing_build() {
    let out = check("app");
    assert!(
        out.contains("claim `pay::single_settle` violated"),
        "the library's law must fire at close, attributed:\n{}",
        out
    );
    assert!(
        out.contains("`Audit`") && out.contains("`pay::Wire`"),
        "the countermodel must name both subscribers in author \
         spelling:\n{}",
        out
    );
    assert!(
        out.contains("p.hl"),
        "the violation must point at the library's own claim line:\n{}",
        out
    );
    assert!(
        !out.contains("__lib_"),
        "no mangled symbol may appear in a law diagnostic:\n{}",
        out
    );
    // The sibling claim that stays satisfied at close reports
    // nothing — traveling law is still ordinary law.
    assert!(
        !out.contains("`pay::wired`"),
        "a satisfied traveled claim must stay silent:\n{}",
        out
    );
}

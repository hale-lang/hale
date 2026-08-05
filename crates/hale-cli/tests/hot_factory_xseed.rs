//! GH #402: the hot-path advisory across a seed boundary.
//!
//! This lives at the CLI layer because the behavior only exists once
//! seeds are merged. At a cross-seed call the callee keeps its author
//! spelling (`mat::zeros`) while the symbol is merged under a mangled
//! one (`__lib_<id>_<stem>_<fn>`), so neither the joined path nor the
//! bare tail resolves and the lint falls back to scanning for the
//! mangled spelling. A single-source unit test never reaches that
//! path — resolution there is exact — so it cannot pin any of this.
//!
//! Two properties, and the second is the one worth the fixture:
//!
//!  - a **unique** tail resolves, so the finding survives the seed
//!    boundary. Without this the lint would be silent on exactly the
//!    codebases it was written for (#402's reference workload calls
//!    its factories cross-seed).
//!  - an **ambiguous** tail — two seeds exporting the same name, one
//!    returning a locus and one not — is dropped rather than guessed.
//!    This layer has no alias table to tell them apart (the same
//!    limitation `topic_tail` documents), and a lint must not invent
//!    a finding out of an ambiguity.
//!
//! The ambiguity verdict is also order-independent: both kinds of
//! candidate are counted before deciding, rather than short-circuiting
//! mid-scan, so a non-factory encountered *before* the factory poisons
//! the tail exactly as one encountered after it does. An earlier draft
//! short-circuited and silently depended on `symbols` iteration order.

use std::path::PathBuf;
use std::process::Command;

fn check_app() -> String {
    let app = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/hot-factory-app");
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("check")
        .arg(&app)
        .output()
        .expect("run hale check");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn a_cross_seed_factory_call_in_a_loop_is_flagged() {
    let out = check_app();
    assert!(
        out.contains("`mat::zeros` returns the locus"),
        "a factory call must resolve across the seed boundary, in \
         author spelling on both halves:\n{}",
        out
    );
    assert!(
        !out.contains("__lib_"),
        "no mangled symbol may reach the author:\n{}",
        out
    );
}

#[test]
fn an_ambiguous_tail_name_is_never_guessed() {
    let out = check_app();
    assert!(
        !out.contains("`mat::build` returns the locus"),
        "`build` is exported by two seeds with different return \
         types — the call cannot be resolved, so it must produce no \
         finding rather than a guessed one:\n{}",
        out
    );
    // Guard against the assertion above passing because the lint
    // stopped working altogether.
    assert!(
        out.contains("returns the locus"),
        "the unambiguous call in the same file must still fire:\n{}",
        out
    );
}

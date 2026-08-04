//! GH #382 phase 1 — claims across a seed boundary.
//!
//! The motivating shape is exactly this fixture: two wings imported
//! as seeds, an isolation law declared once in the app's main locus,
//! and a shared literal subject as the boundary-crossing temptation.
//!
//! What travels through the mangle stage and must survive it:
//!   - `delta::*` — the glob expands the imported seed's decls via
//!     the same rename table codegen resolves `alias::Name` through;
//!   - `gamma::Research` — a qualified member canonicalized to the
//!     mangled decl at the mangle stage (#334's path — never by
//!     name-suffix matching);
//!   - the witness, demangled back to author spelling
//!     (`delta::Triage`, not `__lib_lib_delta_d_Triage`).

use std::path::PathBuf;
use std::process::Command;

fn check() -> String {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/xseed-claims/app");
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("check")
        .arg(&fixture)
        .output()
        .expect("run hale check");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn an_isolation_claim_fires_across_seed_boundaries() {
    let out = check();
    assert!(
        out.contains("claim `iso_dg` violated"),
        "the cross-wing publish must violate the isolation claim:\n{}",
        out
    );
    assert!(
        out.contains("org.metrics"),
        "the witness must name the crossing subject:\n{}",
        out
    );
}

/// The witness names what the author wrote, not merged symbols. A
/// mangled name in a diagnostic points at something that appears
/// nowhere in their source and cannot be searched for.
#[test]
fn the_witness_is_demangled_to_author_spelling() {
    let out = check();
    assert!(
        out.contains("delta::Triage::on_task")
            && out.contains("gamma::Research::on_metric"),
        "the path must be in author spelling:\n{}",
        out
    );
    assert!(
        !out.contains("__lib_"),
        "no mangled symbol may leak into the witness:\n{}",
        out
    );
}

/// Group resolution across the boundary: neither the glob nor the
/// qualified member may produce an unknown-name or vacuity error —
/// the ONLY finding is the violation itself.
#[test]
fn cross_seed_members_resolve_without_noise() {
    let out = check();
    assert!(
        !out.contains("names no") && !out.contains("vacuously"),
        "cross-seed members must resolve cleanly:\n{}",
        out
    );
}

//! Effect assertions must survive a seed boundary.
//!
//! Reported by fathom (FRICTION.md P0), and the highest-cost item on
//! their list: `@no_syscall` / `@budget` / `@deterministic` enforced
//! only within the asserting fn's own seed, and were **silently
//! vacuous** through a cross-seed call. Their probe showed all three
//! contracts violated one seed away with `hale check` reporting
//! nothing, then the binary printing proof every effect had run.
//!
//! That is worse than no annotation: it reads as verified. Fathom's
//! hot paths are almost entirely cross-seed — every venue parse, every
//! domain helper, every topic lives in `lib/` and is imported under an
//! alias — so their certificates certified only the thin app-seed
//! portion, and their venue-tier rollout was blocked on it.
//!
//! ## Why the compiler's own corpus could not see this
//!
//! Every in-tree effect test declares its types, topics and loci
//! **inline in one seed**. The one shape the substrate never
//! exercised is the only shape a real multi-seed codebase has. This
//! fixture is that shape, in-tree, so it cannot regress.
//!
//! ## Root cause
//!
//! `hale check` collected only the target directory's own `.hl`
//! files — it never followed `import`. So the imported bodies were
//! not in the program the callgraph walked. Separately, a call
//! written `alias::name` reaches the graph as a qualified path while
//! the imported decl was merged under a mangled symbol, so even with
//! the bodies present the two never met. Codegen had the rename table
//! all along; the analysis phases did not.

use std::path::PathBuf;
use std::process::Command;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/xseed-effects/app")
}

fn check() -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("check")
        .arg(fixture())
        .output()
        .expect("invoke hale check");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn no_syscall_bites_across_a_seed_boundary() {
    let out = check();
    assert!(
        out.contains("`certified_no_syscall` must not reach `syscall`"),
        "a syscall one seed away must violate @no_syscall:\n{}",
        out
    );
}

#[test]
fn budget_counts_an_allocation_one_seed_away() {
    let out = check();
    assert!(
        out.contains("certified_zero_alloc")
            && out.contains("budget"),
        "an allocation one seed away must count against the ceiling:\n{}",
        out
    );
}

#[test]
fn deterministic_bites_across_a_seed_boundary() {
    let out = check();
    assert!(
        out.contains("`certified_deterministic` must not reach `time`"),
        "a clock read one seed away must violate @deterministic:\n{}",
        out
    );
}

/// The witness must name the call as the author wrote it. A merged
/// symbol (`__lib_lib_probe_p_far_syscall`) appears nowhere in their
/// source and cannot even be searched for.
#[test]
fn the_witness_path_is_demangled() {
    let out = check();
    assert!(
        out.contains("p::far_syscall"),
        "cross-seed witness should read in the alias spelling:\n{}",
        out
    );
    assert!(
        !out.contains("__lib_"),
        "no mangled symbol may reach a user-facing diagnostic:\n{}",
        out
    );
}

/// The negative control: a genuinely clean fn in the same file must
/// stay silent, or the tests above would pass on a checker that
/// simply rejects everything.
#[test]
fn a_clean_in_seed_fn_is_not_flagged() {
    let out = check();
    assert!(
        !out.contains("control_clean"),
        "the clean control must not be reported:\n{}",
        out
    );
}

/// Resolving imports is what makes cross-seed ERRORS visible — and it
/// also drags every advisory lint in every imported seed into the
/// target's output. Checking one fathom app began reporting 47
/// hot-path warnings from `lib/` and `pond/`, and because
/// `hale verify` gates on ANY finding, 10 of 12 apps that passed it
/// started failing.
///
/// A gate that goes red for library internals you cannot edit from
/// here is a gate people switch off. Advisories are therefore
/// reported where they are actionable — when that seed is checked —
/// while errors are never filtered, wherever they originate.
#[test]
fn advisories_from_an_imported_seed_are_not_reported_on_the_app() {
    let out = check();
    assert!(
        !out.contains("warning:"),
        "checking the app must not report advisories about the seed it \
         imports:\n{}",
        out
    );
    // …and the errors that motivated resolving imports still fire.
    assert!(
        out.contains("must not reach `syscall`"),
        "cross-seed errors must survive the advisory filter:\n{}",
        out
    );
}

/// The other half, and what makes the filter honest rather than a
/// blanket suppression: checking the SEED reports its own advisories.
/// Nothing is lost, it just lands where someone can act on it.
#[test]
fn the_seed_still_reports_its_own_advisories_when_checked_directly() {
    let seed = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/xseed-effects/lib/probe");
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("check")
        .arg(&seed)
        .output()
        .expect("invoke hale check on the seed");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        text.contains("warning:"),
        "the seed's own advisories must appear when IT is the target — \
         otherwise the filter is hiding them, not relocating them:\n{}",
        text
    );
}

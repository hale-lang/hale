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

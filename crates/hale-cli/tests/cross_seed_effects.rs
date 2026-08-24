//! Effect assertions must survive a seed boundary.
//!
//! Reported in a downstream handoff, and the highest-cost item on
//! their list: `@no_syscall` / `@budget` / `@deterministic` enforced
//! only within the asserting fn's own seed, and were **silently
//! vacuous** through a cross-seed call. Their probe showed all three
//! contracts violated one seed away with `hale check` reporting
//! nothing, then the binary printing proof every effect had run.
//!
//! That is worse than no annotation: it reads as verified. The
//! hot paths are almost entirely cross-seed — every venue parse, every
//! domain helper, every topic lives in `lib/` and is imported under an
//! alias — so their certificates certified only the thin app-seed
//! portion, and the rollout of real certificates was blocked on it.
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
/// target's output. Checking one downstream app began reporting 47
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

/// The filter must not swallow the TARGET's own advisories.
///
/// Its first cut did exactly that: `file_bases` carries paths as they
/// were passed in (usually relative) while the owned-file set is
/// canonicalized, so a plain set lookup reported every file as
/// foreign and the app's own warnings vanished along with the
/// imported ones. Suppressing findings the author can act on is a
/// worse failure than the noise the filter exists to remove, and it
/// is silent — the output just looks clean.
#[test]
fn the_targets_own_advisories_survive_the_filter() {
    // The app fixture's own file carries an unbounded accumulation.
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/xseed-own-warning");
    std::fs::create_dir_all(&dir).ok();
    std::fs::write(
        dir.join("hale.toml"),
        "name = \"xseed-own-warning\"\n",
    )
    .ok();
    std::fs::create_dir_all(dir.join("lib/quiet")).ok();
    std::fs::write(
        dir.join("lib/quiet/q.hl"),
        "fn helper(n: Int) -> Int { return n + 1; }\n",
    )
    .ok();
    std::fs::create_dir_all(dir.join("app")).ok();
    std::fs::write(
        dir.join("app/main.hl"),
        "import \"lib/quiet\" as q;\n\
         type Slot { key: String; n: Int; }\n\
         @form(hashmap)\n\
         locus Acc { capacity { pool slots of Slot indexed_by key; } }\n\
         locus Grow {\n\
         \x20   params { acc: Acc = Acc { }; }\n\
         \x20   run() {\n\
         \x20       let mut i = 0;\n\
         \x20       while true {\n\
         \x20           self.acc.set(Slot { key: \"k\" + to_string(i), n: q::helper(i) });\n\
         \x20           i = i + 1;\n\
         \x20       }\n\
         \x20   }\n\
         }\n\
         main locus App { params { g: Grow = Grow { }; } }\n\
         fn main() { App { }; }\n",
    )
    .ok();

    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("check")
        .arg(dir.join("app"))
        .output()
        .expect("invoke hale check");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        text.contains("warning:"),
        "the target's OWN advisory must survive — the filter drops \
         foreign findings, not local ones:\n{}",
        text
    );
    assert!(
        text.contains("app/main.hl"),
        "and it must be attributed to the app's own file:\n{}",
        text
    );
}

/// An app's effects manifest describes the APP, not the libraries it
/// imports.
///
/// Once `check` resolved imports, every imported fn emitted its own
/// row under its merged symbol: one downstream fleet's committed
/// baseline went from 1,319 rows to 8,021, and 131 of one app's 151
/// rows were mangled names. That defeats the artifact's purpose — an
/// effect regression is meant to be a one-line diff in review, and a
/// mangled name is unreadable and encodes an internal scheme that
/// churns.
///
/// Nothing is lost: a library's rows come from checking the library,
/// and what an imported fn contributes here is already folded into
/// the caller's inferred `does={…}`.
#[test]
fn the_manifest_carries_no_merged_symbols() {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("check")
        .arg(fixture())
        .arg("--dump-effects-manifest")
        .output()
        .expect("invoke hale check --dump-effects-manifest");
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        !text.contains("__lib_"),
        "no merged cross-seed symbol may appear in a manifest:\n{}",
        text
    );
    // Non-vacuous: the app's OWN annotated fns must still be listed.
    assert!(
        text.contains("certified_no_syscall"),
        "the app's own rows must survive the filter:\n{}",
        text
    );
}

/// Every diagnostic renders in the spelling the author wrote — not
/// only effect witnesses. The no-locus-return rule was naming
/// `__lib_lib_a_b_OrderBook.query_bulk`, a symbol appearing nowhere
/// in the user's program and impossible to search for.
#[test]
fn non_effect_diagnostics_are_demangled_too() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/xseed-cqrs");
    std::fs::create_dir_all(dir.join("lib/mat")).ok();
    std::fs::create_dir_all(dir.join("app")).ok();
    std::fs::write(dir.join("hale.toml"), "name = \"xseed-cqrs\"\n").ok();
    std::fs::write(
        dir.join("lib/mat/m.hl"),
        "locus Matrix { params { n: Int = 0; } }\n",
    )
    .ok();
    std::fs::write(
        dir.join("app/main.hl"),
        "import \"lib/mat\" as mx;\n\
         locus Book {\n\
         \x20   params { n: Int = 0; }\n\
         \x20   fn query_bulk() -> mx::Matrix { return mx::Matrix { n: 1 }; }\n\
         }\n\
         main locus App { params { b: Book = Book { }; } }\n\
         fn main() { App { }; }\n",
    )
    .ok();
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("check")
        .arg(dir.join("app"))
        .output()
        .expect("invoke hale check");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        text.contains("mx::Matrix") && !text.contains("__lib_"),
        "the no-locus-return diagnostic must name the alias spelling:\n{}",
        text
    );
}

/// Review round 4 of GH #476 Change 5h: the QUANTITATIVE dimensions
/// cross the seed boundary too.
///
/// The bundle carries the rename table precisely because analysis
/// without it loses cross-seed calls, and the budgets are named as
/// an affected analysis. The migrated evidence path called
/// `quantitative::certificate_groups` without it, so
/// `summarize_programs` left `p::far_block()` an unresolved
/// qualified free call — and the quantity traversal, which treats
/// only indirect and opaque-receiver calls as unbounded, counted it
/// as ZERO. Every dimension had the defect.
#[test]
fn block_points_bites_across_a_seed_boundary() {
    let out = check();
    assert!(
        out.contains("`certified_no_block` declares \
                      `@budget(block_points = 0)`"),
        "a blocking call one seed away must bust a zero \
         block-point budget:\n{}",
        out
    );
}

#[test]
fn stack_bytes_bites_across_a_seed_boundary() {
    let out = check();
    assert!(
        out.contains("`certified_thin_stack` declares \
                      `@budget(stack_bytes = 40)`"),
        "an imported frame must count toward the stack bound:\n{}",
        out
    );
}

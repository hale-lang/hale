//! GH #265 step 7 — the CORPUS-WIDE conformance sweep.
//!
//! `effects_conformance.rs` checks hand-written programs against the
//! runtime oracle. This is the scaled version the issue describes:
//! walk the example `main.hl` files, compute each source body's
//! INFERRED effect sets, and assert the analysis is internally
//! coherent everywhere real code lives — not just on fixtures
//! written to exercise it.
//!
//! What it checks per program:
//!   1. **Total classification** — no reachable stdlib call lands on
//!      an UNCLASSIFIED registry row. An unknown execution horizon
//!      also saturates the upper bound, so it must remain distinct
//!      from an unclassified leaf. Neither can certify absence.
//!   2. **Inference terminates and is deterministic** — the same
//!      program inferred twice yields identical sets (no
//!      iteration-order or memo-poisoning nondeterminism, the class
//!      of bug that makes a manifest diff-noisy).
//!   3. **No known contract violation** — a fixture that violates
//!      its own assertion fails here. An uncertified contract remains
//!      an error of the normal check; this sweep does not certify it.

#[path = "../../hale-types/tests/support/entries.rs"]
mod entries;
use std::collections::BTreeSet;
use std::path::PathBuf;

use hale_types::alloc_summary::{self, AllocSummary, FnKey};
use hale_types::{callgraph, frontier, stdlib_surface};
use hale_types::stdlib_surface::EffectSet;

fn corpus_files() -> Vec<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/examples");
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(&root) else {
        return out;
    };
    for e in entries.flatten() {
        let main = e.path().join("main.hl");
        if main.is_file() {
            out.push(main);
        }
    }
    out.sort();
    out
}

// An unknown execution horizon saturates the inferred upper bound too.
// Ask the shared witness walk specifically for an unclassified registry
// leaf, rather than treating that upper bound as the leaf's provenance.
fn unclassified_registry_path(
    summary: &AllocSummary,
    key: &FnKey,
    lookup: &dyn Fn(&[&str]) -> Option<EffectSet>,
) -> Option<Vec<callgraph::WitnessStep>> {
    callgraph::witness_path(summary, key, &mut |probe| match probe {
        callgraph::Probe::Unresolved(name, edge) if !edge.indirect && !edge.opaque_method_call() => {
            lookup(&name.split("::").collect::<Vec<_>>())
                .filter(|effects| effects.is_unclassified()).map(|_| name.to_string())
        }
        _ => None,
    })
}

#[test]
fn corpus_effect_inference_is_total_and_deterministic() {
    let files = corpus_files();
    assert!(
        files.len() > 10,
        "corpus should be substantial; found {}",
        files.len()
    );
    let mut unclassified_hits: Vec<String> = Vec::new();
    let mut nondeterministic: Vec<String> = Vec::new();
    let mut checked = 0usize;

    for f in &files {
        let Ok(src) = std::fs::read_to_string(f) else { continue };
        let Ok(mut program) = hale_syntax::parse_source(&src) else {
            // Parse failures are other suites' business.
            continue;
        };
        let ids = hale_types::snapshot::mint([("app.hl", &mut program)], &[]);
        let summary = alloc_summary::summarize_identified(&[(&program, &ids)], &[]);
        let ffi: BTreeSet<String> = BTreeSet::new();
        for key in summary.fns.keys() {
            let a = frontier::infer_effect_bounds(&summary, key, &ffi);
            let b = frontier::infer_effect_bounds(&summary, key, &ffi);
            checked += 1;
            assert_eq!(a.unknown, a.effects.is_unclassified(), "{}: unknown work must retain a conservative upper bound", key.display());
            assert!(!a.known.is_unclassified(), "{}: unknown work cannot become a known effect", key.display());
            if a != b {
                nondeterministic.push(format!(
                    "{}::{}",
                    f.file_name().unwrap().to_string_lossy(),
                    key.display()
                ));
            }
            if let Some(path) = unclassified_registry_path(&summary, key, &stdlib_surface::effects_for) {
                unclassified_hits.push(format!(
                    "{} :: {}",
                    f.parent()
                        .and_then(|p| p.file_name())
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default(),
                    callgraph::render_witness(key, &path)
                ));
            }
        }
    }

    assert!(checked > 50, "expected to check many fns, got {}", checked);
    assert!(
        nondeterministic.is_empty(),
        "effect inference is nondeterministic for: {:?} — a manifest built \
         from this would diff spuriously",
        nondeterministic
    );
    assert!(
        unclassified_hits.is_empty(),
        "these corpus fns reach an UNCLASSIFIED stdlib row, which silently \
         weakens every assertion over them: {:?}",
        unclassified_hits
    );
}

#[test]
fn the_registry_oracle_detects_an_unclassified_leaf() {
    // Perturb only the registry query used by this oracle. The producer
    // and its real call sites stay unchanged; a missed leaf must fail.
    for (source, unknown) in [
        ("fn main() { println(1); }", false),
        ("locus Worker { run() { println(1); } } fn main() { Worker { }; }", true),
    ] {
        let mut program = hale_syntax::parse_source(source).unwrap();
        let ids = hale_types::snapshot::mint([("probe.hl", &mut program)], &[]);
        let summary = alloc_summary::summarize_identified(&[(&program, &ids)], &[]);
        let main = summary.resolve(None, "main").unwrap();
        assert_eq!(frontier::infer_effect_bounds(&summary, main, &BTreeSet::new()).unknown, unknown);
        assert!(unclassified_registry_path(&summary, main, &stdlib_surface::effects_for).is_none());
        let missing_classification = |path: &[&str]| {
            if path == ["println"] { Some(EffectSet::UNCLASSIFIED) }
            else { stdlib_surface::effects_for(path) }
        };
        let path = unclassified_registry_path(&summary, main, &missing_classification).expect("even an unknown execution horizon cannot hide the unclassified leaf");
        assert_eq!(path.last().unwrap().label, "println");
    }
}

/// Known violations of a fixture's own effect contracts fail this
/// sweep. Uncertified contracts retain their normal compiler errors;
/// absence of a violation here does not certify them.
#[test]
fn corpus_declared_contracts_hold() {
    let mut violations: Vec<String> = Vec::new();
    for f in corpus_files() {
        let Ok(src) = std::fs::read_to_string(&f) else { continue };
        if !(src.contains("@no_")
            || src.contains("@effects(")
            || src.contains("@deterministic")
            || src.contains("@phase_effects")
            || src.contains("@supervised"))
        {
            continue;
        }
        let Ok(program) = hale_syntax::parse_source(&src) else { continue };
        for d in entries::check_program(&program) {
            let m = &d.message;
            if m.contains("effect assertion violated")
                || m.contains("phase contract violated")
                || m.contains("causal set violated")
                || m.contains("publish set violated")
                || m.contains("@supervised` violated")
                || m.contains("@no_panic` violated")
            {
                violations.push(format!(
                    "{}: {}",
                    f.parent()
                        .and_then(|p| p.file_name())
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default(),
                    m
                ));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "corpus programs violate their own declared effect contracts: {:#?}",
        violations
    );
}

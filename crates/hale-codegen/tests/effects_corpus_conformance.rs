//! GH #265 step 7 — the CORPUS-WIDE conformance sweep.
//!
//! `effects_conformance.rs` checks hand-written programs against the
//! runtime oracle. This is the scaled version the issue describes:
//! walk the whole in-tree `.hl` corpus, compute each program's
//! INFERRED effect sets, and assert the analysis is internally
//! coherent everywhere real code lives — not just on fixtures
//! written to exercise it.
//!
//! What it checks per program:
//!   1. **Total classification** — no reachable stdlib call lands on
//!      an UNCLASSIFIED registry row. An unclassified leaf silently
//!      weakens every assertion, so the frontier staying complete as
//!      the corpus grows is the property that must not rot.
//!   2. **Inference terminates and is deterministic** — the same
//!      program inferred twice yields identical sets (no
//!      iteration-order or memo-poisoning nondeterminism, the class
//!      of bug that makes a manifest diff-noisy).
//!   3. **Declared contracts hold** — any corpus program that
//!      carries an assertion must satisfy it. A corpus fixture that
//!      violates its own contract is either a compiler bug or a bad
//!      fixture; both want to fail loudly here.

use std::collections::BTreeSet;
use std::path::PathBuf;

use hale_types::alloc_summary::{self, FnKey};
use hale_types::frontier;

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

fn fn_keys(program: &hale_syntax::ast::Program) -> Vec<FnKey> {
    use hale_syntax::ast::*;
    let mut keys = Vec::new();
    for item in &program.items {
        match item {
            TopDecl::Fn(fd) => keys.push(FnKey::free_fn(fd.name.name.clone())),
            TopDecl::Locus(l) => {
                for m in &l.members {
                    if let LocusMember::Fn(fd) = m {
                        keys.push(FnKey::method(
                            l.name.name.clone(),
                            fd.name.name.clone(),
                        ));
                    }
                }
            }
            _ => {}
        }
    }
    keys
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
        let Ok(program) = hale_syntax::parse_source(&src) else {
            // Parse failures are other suites' business.
            continue;
        };
        let summary = alloc_summary::summarize_programs(&[&program]);
        let ffi: BTreeSet<String> = BTreeSet::new();
        for key in fn_keys(&program) {
            let a = frontier::infer_effects(&summary, &key, &ffi);
            let b = frontier::infer_effects(&summary, &key, &ffi);
            checked += 1;
            if a != b {
                nondeterministic.push(format!(
                    "{}::{}",
                    f.file_name().unwrap().to_string_lossy(),
                    key.display()
                ));
            }
            if a.is_unclassified() {
                unclassified_hits.push(format!(
                    "{} :: {}",
                    f.parent()
                        .and_then(|p| p.file_name())
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default(),
                    key.display()
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

/// Every corpus program that carries an effect assertion must
/// satisfy it. A fixture violating its own contract is either a
/// compiler bug or a bad fixture — both should fail here rather than
/// rot silently.
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
        for d in hale_types::check_program(&program) {
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

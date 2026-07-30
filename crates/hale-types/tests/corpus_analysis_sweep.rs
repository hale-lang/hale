//! Whole-corpus analysis properties: the compiler must not crash on
//! any program it can parse, and the effect frontier must cover
//! everything the corpus actually calls.
//!
//! Both run over the **full** corpus — on-disk fixtures plus the
//! ~1.2k programs embedded in test string literals. The frontier
//! property in particular is the one that pays for the provider: run
//! against fixtures alone it was clean, and the first time it saw the
//! embedded corpus it found `std::cli`, `std::log` and `std::source`
//! reaching effectful calls with no registry row — a soundness hole
//! in shipped effect assertions.
//!
//! Note what these check. They are *properties*, not transcribed
//! expectations: nothing here says what a program should print. That
//! is the point — a property costs one assertion and scales to every
//! program in the repo, where an expected-output test costs one
//! transcription per program and covers exactly one.

use std::collections::BTreeSet;

fn corpus() -> Vec<hale_corpus::Program> {
    hale_corpus::parseable(|s| hale_syntax::parse_source(s).is_ok())
}

/// No parseable program may panic the analysis passes.
///
/// A panic here is an internal compiler error: whatever the program
/// does wrong, the answer is a diagnostic, never a crash. Running it
/// across the whole corpus is cheap and it is the single broadest
/// net the type layer has.
#[test]
fn analysis_never_panics_on_the_corpus() {
    let mut ices = Vec::new();
    for p in corpus() {
        let Ok(program) = hale_syntax::parse_source(&p.source) else {
            continue;
        };
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
            || {
                let _ = hale_types::check_program(&program);
            },
        ));
        if caught.is_err() {
            ices.push(p.origin);
        }
    }
    assert!(
        ices.is_empty(),
        "the analysis panicked on {} corpus programs — a malformed \
         program earns a diagnostic, never a crash:\n{:#?}",
        ices.len(),
        &ices[..ices.len().min(20)]
    );
}

/// Inference must be deterministic across the whole corpus.
///
/// Non-determinism here is what makes an effects manifest diff-noisy,
/// which in turn makes the CI gate unusable — a gate that reports
/// spurious changes gets switched off.
#[test]
fn analysis_is_deterministic_on_the_corpus() {
    let mut unstable = Vec::new();
    for p in corpus() {
        let Ok(program) = hale_syntax::parse_source(&p.source) else {
            continue;
        };
        let a: Vec<String> = hale_types::check_program(&program)
            .into_iter()
            .map(|d| d.message)
            .collect();
        let b: Vec<String> = hale_types::check_program(&program)
            .into_iter()
            .map(|d| d.message)
            .collect();
        if a != b {
            unstable.push(p.origin);
        }
    }
    assert!(
        unstable.is_empty(),
        "analysis output differs between two runs of the same program \
         ({} cases) — iteration-order or memo nondeterminism:\n{:#?}",
        unstable.len(),
        &unstable[..unstable.len().min(20)]
    );
}

/// Every `std::` path the corpus CALLS must be known to the effect
/// registry.
///
/// This is the property that found the Tier 0 hole. Frontier
/// completeness was previously asserted as "no reachable stdlib call
/// is UNCLASSIFIED", which could only see paths that had a row —
/// absent namespaces were invisible to the very check meant to
/// guarantee coverage. Asking the question from the *corpus* side
/// instead of the registry side is what closes that gap.
#[test]
fn every_std_path_the_corpus_calls_is_classified() {
    // Namespaces that are types/loci rather than call surfaces, or
    // are deliberately-bogus paths in negative fixtures.
    const NEGATIVE_FIXTURE_NAMESPACES: &[&str] = &["nowhere", "nonexistent"];

    let mut unknown: BTreeSet<String> = BTreeSet::new();
    for p in corpus() {
        for path in std_paths_in(&p.source) {
            let segs: Vec<&str> = path.split("::").collect();
            if segs.len() < 3 {
                continue;
            }
            if NEGATIVE_FIXTURE_NAMESPACES.contains(&segs[1]) {
                continue;
            }
            // A capitalized leaf is a type/locus, tracked by
            // LOCUS_PATHS rather than the fn registry.
            if segs
                .last()
                .and_then(|s| s.chars().next())
                .map(|c| c.is_uppercase())
                .unwrap_or(false)
            {
                continue;
            }
            // Ask at NAMESPACE granularity, which is where the
            // soundness hole actually lives. An unknown *leaf* in a
            // known namespace (`std::str::parse_itn`) already fails
            // typecheck with a did-you-mean, so it cannot escape
            // silently — the suite even has a fixture for it. An
            // unknown *namespace* is the dangerous case: nothing
            // rejects it, it types as `Ty::Unknown`, and every path
            // under it slips past classification.
            if !namespace_is_registered(&segs) {
                unknown.insert(path);
            }
        }
    }
    assert!(
        unknown.is_empty(),
        "these `std::` paths are called by the corpus but have no \
         effect-registry row, so an assertion over them cannot be \
         certified ({} found):\n{:#?}",
        unknown.len(),
        unknown
    );
}

/// Is `std::<ns…>` a namespace the registry knows?
fn namespace_is_registered(segs: &[&str]) -> bool {
    let ns = &segs[1..segs.len() - 1];
    hale_types::stdlib_surface::SURFACES.iter().any(|s| {
        s.ns.len() == ns.len() && s.ns.iter().zip(ns).all(|(a, b)| a == b)
    })
}

/// Extract `std::…` paths that appear in CALL position, with comments
/// stripped. Comments matter: several fixtures describe prospective
/// surfaces (`std::geom::segment`) in prose, and counting those would
/// report defects that do not exist.
fn std_paths_in(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in src.lines() {
        let code = match line.find("//") {
            Some(i) => &line[..i],
            None => line,
        };
        let bytes = code.as_bytes();
        let mut i = 0;
        while let Some(rel) = code[i..].find("std::") {
            let start = i + rel;
            let mut end = start;
            while end < bytes.len() {
                let c = bytes[end] as char;
                if c.is_alphanumeric() || c == '_' {
                    end += 1;
                } else if c == ':' && end + 1 < bytes.len() && bytes[end + 1] == b':' {
                    end += 2;
                } else {
                    break;
                }
            }
            let path = code[start..end].trim_end_matches(':').to_string();
            // Call position only — a bare path is a type annotation.
            if code[end..].trim_start().starts_with('(') {
                out.push(path);
            }
            i = end.max(start + 5);
        }
    }
    out
}

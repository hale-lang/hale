//! `hale doc --stdlib` publishes each function's effect class.
//!
//! The registry has carried an `EffectSet` per stdlib fn since #265,
//! and the doc generator walked those very entries to print
//! signatures while ignoring the column beside them. So the
//! classification the checker enforces was invisible to anyone
//! reading the docs — you could not find out whether
//! `std::time::monotonic_ns` is callable from a `@deterministic` fn
//! without reading `stdlib_surface.rs`.
//!
//! Deriving the catalogue from the registry rather than writing it
//! down is the whole point: a hand-maintained table of 300+ rows
//! would drift from the checker, and this repo has now been bitten
//! by exactly that failure mode three times.

use std::process::Command;

fn stdlib_doc() -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["doc", "--stdlib"])
        .output()
        .expect("invoke hale doc --stdlib");
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// Split the markdown into `### <path>` sections.
fn sections(md: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for chunk in md.split("\n### ").skip(1) {
        let name = chunk.lines().next().unwrap_or("").trim().to_string();
        out.push((name, chunk.to_string()));
    }
    out
}

/// Every documented FUNCTION carries its effect classification.
/// Locus and type paths legitimately have no row, and are excluded
/// by case — the surface capitalizes them.
#[test]
fn every_documented_fn_publishes_its_effects() {
    let secs = sections(&stdlib_doc());
    assert!(
        secs.len() > 300,
        "expected the full stdlib surface, saw {} entries",
        secs.len()
    );
    let missing: Vec<String> = secs
        .iter()
        .filter(|(name, _)| {
            !name
                .rsplit("::")
                .next()
                .and_then(|leaf| leaf.chars().next())
                .map(|c| c.is_uppercase())
                .unwrap_or(false)
        })
        .filter(|(_, body)| !body.contains("**Effects:**"))
        .map(|(name, _)| name.clone())
        .collect();
    assert!(
        missing.is_empty(),
        "these documented functions do not publish an effect class \
         ({} of {}) — they are missing a registry row, which also \
         means an effect assertion over them cannot be certified:\n{:#?}",
        missing.len(),
        secs.len(),
        &missing[..missing.len().min(20)]
    );
}

/// The catalogue must reflect the registry, not a copy of it. Spot
/// checks in both directions: an effectful fn and a pure one, chosen
/// because their classification is load-bearing for the `@no_syscall`
/// and `@deterministic` contracts respectively.
#[test]
fn published_classes_match_the_registry() {
    let md = stdlib_doc();
    let secs = sections(&md);
    let find = |p: &str| -> String {
        secs.iter()
            .find(|(n, _)| n == p)
            .unwrap_or_else(|| panic!("{} missing from the stdlib docs", p))
            .1
            .clone()
    };
    assert!(
        find("std::io::fs::read_file").contains("**Effects:** `syscall`"),
        "reading a file is a syscall"
    );
    assert!(
        find("std::str::parse_int").contains("**Effects:** none"),
        "parsing a supplied string is pure — this is what makes it \
         callable inside a @no_syscall fn"
    );
    // The distinction the classification exists to draw: operating on
    // a SUPPLIED value is deterministic; READING the clock is not.
    assert!(
        find("std::time::monotonic_ns").contains("`time`"),
        "reading the clock is a time effect"
    );
}

/// A generator that silently stopped emitting the line would make
/// the coverage test above pass vacuously if the case filter ever
/// over-matched.
#[test]
fn catalogue_is_not_vacuous() {
    let md = stdlib_doc();
    let n = md.matches("**Effects:**").count();
    assert!(
        n > 250,
        "only {} effect lines in the stdlib docs — the catalogue is \
         not being generated",
        n
    );
}

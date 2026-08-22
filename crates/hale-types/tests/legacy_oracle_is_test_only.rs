//! GH #476 Change 9 — the legacy claim evaluator is a TEST ORACLE,
//! and this test is what keeps it one.
//!
//! Change 9's mandate is removing duplicate AUTHORITIES: something
//! that answers a question for a consumer. The evaluator in
//! `claims.rs` no longer answers anything for anyone shipping —
//! `hale check` and the artifact both read the judgment engines over
//! the canonical model. What remains of it is the comparison arm of
//! three corpus differentials, which is the strongest evidence the
//! migrated engines have: a baseline can only lock in whatever the
//! engine does today, bugs included, while an independent
//! implementation disagreeing is a real signal.
//!
//! The risk in keeping it is that it quietly becomes an authority
//! again — one convenient call from a product path and there are two
//! answers to one question. So the boundary is enforced instead of
//! documented: no `src/` file in the workspace may call the
//! evaluation entry points. If you are here because this test
//! failed, the fix is to read the model, not to add an exception.
//!
//! What `claims.rs` DOES still own in production is law SELECTION
//! (`selection_diags`, `constitution_identities`, `enumerate_clauses`)
//! and the vocabulary helpers the model builder calls. Those are not
//! evaluation and are deliberately not listed here.

use std::path::{Path, PathBuf};

/// The evaluation entry points — the ones that judge claims.
const EVALUATION_ENTRY_POINTS: &[&str] = &[
    "claims_report_with_identities",
    "claims_report(",
    "claims_diags(",
];

fn crates_dir() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

#[test]
fn no_production_source_calls_the_legacy_evaluator() {
    let root = crates_dir();
    let mut files = Vec::new();
    for entry in std::fs::read_dir(&root).expect("crates/").flatten() {
        let src = entry.path().join("src");
        if src.is_dir() {
            walk(&src, &mut files);
        }
    }
    files.sort();
    assert!(
        files.len() > 20,
        "the scan found almost no sources ({}) — it would pass \
         vacuously",
        files.len()
    );

    // `claims.rs` defines them; everything else must not call them.
    let definer = root.join("hale-types/src/claims.rs");
    let mut callers: Vec<String> = Vec::new();
    for f in &files {
        if *f == definer {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(f) else { continue };
        for (i, line) in text.lines().enumerate() {
            let code = line.trim_start();
            if code.starts_with("//") || code.starts_with("///") {
                continue;
            }
            for needle in EVALUATION_ENTRY_POINTS {
                if line.contains(needle) {
                    callers.push(format!(
                        "  {}:{}: {}",
                        f.strip_prefix(&root).unwrap_or(f).display(),
                        i + 1,
                        code.trim()
                    ));
                }
            }
        }
    }
    assert!(
        callers.is_empty(),
        "the legacy claim evaluator has {} production caller(s) — it \
         is a test oracle, and a second answer to a question the \
         judgment engines already answer:\n{}",
        callers.len(),
        callers.join("\n")
    );

    // …and the premise: the oracle still exists to be compared
    // against. If it is deleted, the differentials must have been
    // converted first, and this test should go with them.
    let text = std::fs::read_to_string(&definer).expect("claims.rs");
    assert!(
        text.contains("fn claims_report_with_identities"),
        "the oracle is gone but its canary is still here"
    );
}

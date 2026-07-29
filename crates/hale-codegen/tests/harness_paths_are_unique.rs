//! Every test that builds an executable must get its path from
//! `harness::unique_bin`.
//!
//! The lesson from the stdlib-registry refactor applies verbatim: a
//! shared helper is not a guarantee until something enforces that it
//! is *the* one used. `support/harness.rs` existing does not stop the
//! next test from writing `temp_dir().push("lotus_test_basic")`, and
//! the failure mode is nasty — not a clean error but an intermittent
//! `ETXTBSY`, or (worse, and observed in this suite before) a test
//! silently executing a *different* test's binary.
//!
//! At least three files independently rediscovered the fix and left a
//! comment about it. That is the signature of a missing invariant,
//! not a missing convention.
//!
//! Scope note: this checks *executable* paths. Shared temp files are
//! sometimes deliberate — `hale_bubble_suite.lock` is a cross-test
//! mutex whose whole purpose is to be the same path in every
//! process — so a blanket ban on `temp_dir()` would be wrong.

use std::collections::BTreeSet;
use std::path::PathBuf;

/// Files that call `build_executable` but legitimately don't need
/// `unique_bin`, each with the reason.
fn exemptions() -> BTreeSet<&'static str> {
    // Populated only with a stated reason. An empty set is the goal
    // state and the current one.
    BTreeSet::new()
}

fn tests_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests")
}

fn test_sources() -> Vec<(String, String)> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(tests_dir()) else {
        return out;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.extension().map(|x| x != "rs").unwrap_or(true) {
            continue;
        }
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        if let Ok(t) = std::fs::read_to_string(&p) {
            out.push((name, t));
        }
    }
    out.sort();
    out
}

#[test]
fn every_builder_uses_unique_bin() {
    let exempt = exemptions();
    let offenders: Vec<String> = test_sources()
        .into_iter()
        .filter(|(name, text)| {
            text.contains("build_executable")
                && !text.contains("unique_bin")
                && !exempt.contains(name.as_str())
        })
        .map(|(name, _)| name)
        .collect();
    assert!(
        offenders.is_empty(),
        "these tests build an executable without `harness::unique_bin`, \
         so two of them can race on one path under any parallel runner \
         ({} found):\n{:#?}\n\n\
         Use `harness::unique_bin(name)` (add \
         `#[path = \"support/harness.rs\"] mod harness;`), or add the \
         file to `exemptions()` with the reason.",
        offenders.len(),
        offenders
    );
}

/// The hazard in its original form: two files whose binary-path
/// template is textually identical are one duplicated `name`
/// argument away from colliding. `unique_bin` makes the template
/// irrelevant, so what this really guards is that nobody
/// reintroduces a hand-rolled one.
#[test]
fn no_hand_rolled_binary_temp_paths() {
    let offenders: Vec<String> = test_sources()
        .into_iter()
        .filter(|(_, text)| text.contains("build_executable"))
        .filter(|(_, text)| {
            // Trace it properly: a variable bound from a raw
            // `temp_dir()` that is later handed to
            // `build_executable`. Matching on artifact-*shaped names*
            // instead flags config files and scratch dirs that are
            // legitimately temp-rooted — the suite has 14 of those.
            let temp_vars: BTreeSet<&str> = text
                .match_indices("= std::env::temp_dir()")
                .filter_map(|(i, _)| {
                    let head = &text[..i];
                    let decl = head.rsplit_once("let ")?.1;
                    Some(decl.trim().trim_start_matches("mut ").trim())
                })
                .filter(|v| !v.is_empty() && v.chars().all(|c| c.is_alphanumeric() || c == '_'))
                .collect();
            temp_vars.iter().any(|v| {
                text.contains(&format!("build_executable(&program, &{})", v))
                    || text.contains(&format!("build_executable(&prog, &{})", v))
            })
        })
        .map(|(name, _)| name)
        .collect();
    assert!(
        offenders.is_empty(),
        "these tests derive a build-artifact path from a raw \
         `std::env::temp_dir()` ({} found):\n{:#?}\n\n\
         Route it through `harness::unique_bin`. (Deliberately shared \
         temp paths — a cross-test lock file, say — are fine; this only \
         fires on files that also call `build_executable` and use an \
         artifact-shaped name.)",
        offenders.len(),
        offenders
    );
}

/// A scraper that matched nothing would make both checks above pass
/// vacuously, which is exactly how the first registry-parity test
/// shipped with a hole.
#[test]
fn scan_is_not_vacuous() {
    let srcs = test_sources();
    assert!(
        srcs.len() > 200,
        "expected to scan the whole codegen test suite, saw {} files",
        srcs.len()
    );
    let builders = srcs
        .iter()
        .filter(|(_, t)| t.contains("build_executable"))
        .count();
    assert!(
        builders > 150,
        "only {} files call build_executable — the scan is not seeing \
         the suite it thinks it is",
        builders
    );
    let using = srcs.iter().filter(|(_, t)| t.contains("unique_bin")).count();
    assert!(
        using > 150,
        "only {} files reference unique_bin — the sweep did not land",
        using
    );
}

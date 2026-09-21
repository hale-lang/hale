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

/// No test mutates this process's environment (GH #843).
///
/// The environment is one global table shared by every thread, and
/// `cargo test` runs a file's tests as *threads* in one process —
/// so a `set_var` in a test is (a) undefined behavior against the
/// concurrent `getenv` every other in-flight `build_executable` is
/// doing, and (b) visibly wrong even where it survives: the build
/// knobs these calls set (`LOTUS_DUMP_IR`, `LOTUS_ASAN`,
/// `LOTUS_NO_BUS_DEVIRT`, `LOTUS_NO_OWNERSHIP_BUBBLE`, `LOTUS_LTO`)
/// change what a *neighbouring* test compiles, and the paired
/// `remove_var` switches the knob back off underneath a build that
/// is still running. Only nextest's process-per-test isolation hid
/// it, and the repo supports both runners.
///
/// Every one of those knobs is a `BuildOptions` field now, so the
/// request travels with the build that wants it. `harness`'s
/// `set_build_env_var` is the sole exception and the sole
/// allow-listed caller — see its doc-comment for when a knob
/// genuinely cannot reach `BuildOptions` (today: `HALE_NO_TS_SHIM`,
/// read by a free function with no options in scope).
///
/// Note the scan covers `tests/*.rs` and not `tests/support/`, which
/// is exactly the split wanted: the helper lives in `support/`.
#[test]
fn no_test_mutates_the_process_environment() {
    // This file names the calls it bans, so it cannot scan itself.
    const SELF: &str = "harness_paths_are_unique.rs";
    // The open paren matters: `stdlib_env.rs` has a test named
    // `var_returns_empty_for_unset_variable`, and "un-SET_VAR-iable"
    // matches a bare substring. Every spelling of the call —
    // `std::env::set_var(`, `env::set_var(`, an imported bare
    // `set_var(` — keeps its paren.
    let offenders: Vec<String> = test_sources()
        .into_iter()
        .filter(|(name, _)| name != SELF)
        .filter(|(_, text)| {
            text.contains("set_var(") || text.contains("remove_var(")
        })
        .map(|(name, _)| name)
        .collect();
    assert!(
        offenders.is_empty(),
        "these tests mutate the process environment ({} found):\n{:#?}\n\n\
         The environment is global to the process and `cargo test` runs \
         tests as threads, so this is UB against every concurrent \
         `build_executable` — and it changes what a neighbouring test \
         compiles. Pass the knob through `hale_codegen::BuildOptions` \
         instead (`dump_ir`, `asan`, `no_bus_devirt`, \
         `no_ownership_bubble`, `lto`), or, for a child process, \
         through `std::process::Command::env`. If the knob truly \
         cannot reach `BuildOptions`, route it through \
         `harness::set_build_env_var` and say in the call site why.",
        offenders.len(),
        offenders
    );
}

/// Nor does any PRODUCTION source (GH #887).
///
/// The undefined behaviour is the same one the rule above removed
/// from the suite, and it was still in shipped code: `hale model
/// dump` planted `HALE_DUMP_MODEL`, `hale run --observe` planted
/// `LOTUS_OBS`, and `hale node` planted `LOTUS_BUS_CONFIG`, each so
/// that something downstream would read it back. `std::env::set_var`
/// is UB in a process that has threads — the environment is one
/// table with no lock and every concurrent `getenv` races it — and
/// this CLI starts them: the language server, the observation
/// reader, an iris session spawned on the very next line.
///
/// Each of the three had a real destination, and each now goes
/// there: to the CHILD on its own `Command::env`, or to the
/// in-process consumer as a process-global bit that is not the
/// environment.
///
/// The scan covers `crates/*/src` — the shipped crates. `build.rs`
/// and `tests/` are elsewhere (the latter has its own rule above).
#[test]
fn no_production_source_mutates_the_process_environment() {
    // Populated only with a reason saying why the site is provably
    // single-threaded at that point. An empty list is the goal state
    // and the current one: the three sites GH #887 named each had a
    // destination, so none of them needed the environment at all.
    let exempt: BTreeSet<&'static str> = BTreeSet::new();

    let mut offenders: Vec<String> = Vec::new();
    let sources = crate_sources();
    assert!(
        sources.len() > 100,
        "expected to scan every crate's src/, saw {} files",
        sources.len()
    );
    for (rel, text) in &sources {
        if exempt.contains(rel.as_str()) {
            continue;
        }
        // The open paren, for the reason the test above gives: a
        // bare `set_var` substring matches `unset_variable` too.
        for (i, line) in text.lines().enumerate() {
            if line.contains("set_var(") || line.contains("remove_var(") {
                offenders.push(format!("{}:{}: {}", rel, i + 1, line.trim()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "these production sources mutate the process environment ({} \
         found):\n{:#?}\n\n\
         `std::env::set_var` is undefined behaviour once the process \
         has threads, and these binaries start them. Pass the value \
         to the CHILD it is meant for on its own \
         `std::process::Command::env`, or to the in-process consumer \
         as a parameter or a process-global that is not the \
         environment. If a site is provably single-threaded, add it \
         to this test's `exempt` set and say there why.",
        offenders.len(),
        offenders
    );
}

/// Every `.rs` under `crates/*/src`, as (repo-relative path, text).
fn crate_sources() -> Vec<(String, String)> {
    let mut root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    root.pop(); // crates/
    root.pop(); // repo root
    let mut out = Vec::new();
    let Ok(crates) = std::fs::read_dir(root.join("crates")) else {
        return out;
    };
    let mut roots: Vec<PathBuf> = crates
        .flatten()
        .map(|e| e.path().join("src"))
        .filter(|p| p.is_dir())
        .collect();
    roots.sort();
    for r in roots {
        collect_rs(&r, &root, &mut out);
    }
    out.sort();
    out
}

fn collect_rs(
    dir: &std::path::Path,
    root: &std::path::Path,
    out: &mut Vec<(String, String)>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut paths: Vec<PathBuf> =
        entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for p in paths {
        if p.is_dir() {
            collect_rs(&p, root, out);
        } else if p.extension().map(|x| x == "rs").unwrap_or(false) {
            if let Ok(t) = std::fs::read_to_string(&p) {
                let rel = p
                    .strip_prefix(root)
                    .unwrap_or(&p)
                    .to_string_lossy()
                    .to_string();
                out.push((rel, t));
            }
        }
    }
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
    // GH #843: the environment ban is only meaningful over a suite
    // that actually asks for its build knobs through the API. If
    // nobody did, the ban would be passing on an empty premise —
    // and a rewrite that quietly dropped the knobs would look like
    // a clean sweep.
    let via_options = srcs
        .iter()
        .filter(|(_, t)| {
            t.contains("build_ir_text") || t.contains("BuildOptions")
        })
        .count();
    assert!(
        via_options > 20,
        "only {} files route a build knob through BuildOptions — the \
         env-var sweep did not land",
        via_options
    );
}

//! The registry guard (F.40 phase 0): the tree cannot grow a
//! derivation the registry does not name.
//!
//! GH #476's canary ("artifact and fleet code cannot walk source for
//! a modeled fact") never reached codegen or the CLI, and #1208's
//! `compute_scratch_local_free_fns` — a fixpoint over the call graph,
//! written inside codegen — landed without tripping anything. Until a
//! guard fails the build, every fix keeps landing in the layer it was
//! found in. `harness_paths_are_unique` is the precedent: the rule
//! became true the day the guard existed.
//!
//! A scanner cannot prove that arbitrary code never reimplements a
//! semantic decision. It can identify the legitimate path and put
//! friction on the wrong one, in three ways:
//!
//! 1. **Derivation-shaped definitions are registered.** A function
//!    in a semantic crate whose name starts with `compute_`,
//!    `derive_`, `infer_`, `summarize_` or `classify_` is a
//!    derivation until proven otherwise, and must appear in the
//!    registry as a producer, a legacy producer or a consumer site.
//! 2. **Seams have allowlists.** A family's guarded entry symbols
//!    (`build_bus_graph(`, `resolve_owners(`, ...) may be referenced
//!    only from the files the registry lists. A new caller is a new
//!    consumer or a new re-derivation, and either way the registry
//!    entry changes first.
//! 3. **Debug-string scans are frozen.** `format!("{:?}", ..)` over
//!    an AST value, read back as text to decide a fact, is the
//!    cheapest way to derive without saying so. Every occurrence in
//!    the semantic crates is listed with a verdict; a new one fails.
//!
//! Each check has a vacuity assertion, so a broken scanner cannot
//! pass by seeing nothing.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p
}

/// The crates whose `src/` holds semantic derivations.
const SEMANTIC_CRATES: &[&str] = &["hale-types", "hale-codegen", "hale-cli", "hale-lsp"];

/// Every crate a seam symbol may be referenced from (the scan covers
/// all of them; the allowlist decides).
const ALL_CRATES: &[&str] = &[
    "hale-syntax",
    "hale-types",
    "hale-model",
    "hale-codegen",
    "hale-cli",
    "hale-lsp",
    "hale-iris",
    "hale-dna",
    "hale-stdlib",
    "hale-corpus",
];

fn rust_sources(root: &Path, crate_name: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let dir = root.join("crates").join(crate_name).join("src");
    let mut stack = vec![dir];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().map(|x| x == "rs").unwrap_or(false) {
                let rel = p
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                if let Ok(t) = std::fs::read_to_string(&p) {
                    out.push((rel, t));
                }
            }
        }
    }
    out.sort();
    out
}

/// Every symbol the registry names anywhere.
fn registered_symbols() -> BTreeSet<String> {
    let mut s = BTreeSet::new();
    for f in hale_graph::families() {
        if let Some(p) = &f.producer {
            s.insert(p.symbol.to_string());
        }
        for l in f.legacy {
            s.insert(l.site.symbol.to_string());
        }
        for c in f.consumers {
            if let Some(site) = &c.site {
                s.insert(site.symbol.to_string());
            }
        }
        for seam in f.seams {
            s.insert(seam.symbol.trim_end_matches('(').to_string());
        }
    }
    for r in hale_graph::rules() {
        if let Some(e) = &r.evaluator {
            s.insert(e.symbol.to_string());
        }
    }
    s
}

const DERIVATION_PREFIXES: &[&str] = &["compute_", "derive_", "infer_", "summarize_", "classify_"];

#[test]
fn derivation_shaped_definitions_are_registered() {
    let root = workspace_root();
    let registered = registered_symbols();
    let mut unregistered = Vec::new();
    let mut seen = 0usize;
    for c in SEMANTIC_CRATES {
        for (rel, text) in rust_sources(&root, c) {
            for (i, line) in text.lines().enumerate() {
                let t = line.trim_start();
                let Some(rest) = t
                    .strip_prefix("pub fn ")
                    .or_else(|| t.strip_prefix("pub(crate) fn "))
                    .or_else(|| t.strip_prefix("pub(super) fn "))
                    .or_else(|| t.strip_prefix("fn "))
                else {
                    continue;
                };
                let name: String = rest
                    .chars()
                    .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
                    .collect();
                if !DERIVATION_PREFIXES.iter().any(|p| name.starts_with(p)) {
                    continue;
                }
                seen += 1;
                if !registered.contains(&name) {
                    unregistered.push(format!("{rel}:{} fn {name}", i + 1));
                }
            }
        }
    }
    assert!(
        seen >= 20,
        "the definition scan is vacuous ({seen} derivation-shaped fns)"
    );
    assert!(
        unregistered.is_empty(),
        "{} derivation-shaped function(s) are not in the registry:\n{}\n\n\
         A function named compute_/derive_/infer_/summarize_/classify_ \
         derives a fact about the program. Register it in \
         crates/hale-graph/src/registry.rs under the family it belongs to \
         (as the producer, or as a legacy producer with its removal \
         condition, or as a consumer site), then regenerate spec/registry.md. \
         If it is not a semantic derivation, rename it.",
        unregistered.len(),
        unregistered.join("\n")
    );
}

#[test]
fn seam_symbols_are_referenced_only_from_allowlisted_files() {
    let root = workspace_root();
    let mut sources = Vec::new();
    for c in ALL_CRATES {
        sources.extend(rust_sources(&root, c));
    }
    let mut violations = Vec::new();
    let mut seen_seams = 0usize;
    let mut seen_refs = 0usize;
    for f in hale_graph::families() {
        for seam in f.seams {
            seen_seams += 1;
            let allowed: BTreeSet<&str> = seam.allowed.iter().copied().collect();
            for (rel, text) in &sources {
                if !text.contains(seam.symbol) {
                    continue;
                }
                seen_refs += 1;
                if !allowed.contains(rel.as_str()) {
                    violations.push(format!(
                        "family `{}`: `{}` is referenced from {rel}, which the registry does not list",
                        f.name, seam.symbol
                    ));
                }
            }
        }
    }
    assert!(
        seen_seams >= 10 && seen_refs >= 30,
        "the seam scan is vacuous ({seen_seams} seams, {seen_refs} references)"
    );
    assert!(
        violations.is_empty(),
        "{} seam reference(s) outside the registry's allowlist:\n{}\n\n\
         A new reference to a family's producer is a new consumer or a new \
         re-derivation. Add the file to the seam's `allowed` list in \
         crates/hale-graph/src/registry.rs with the family's consumer entry \
         (or remove the re-derivation), then regenerate spec/registry.md.",
        violations.len(),
        violations.join("\n")
    );
}

#[test]
fn debug_string_scans_are_frozen() {
    let root = workspace_root();
    let frozen: BTreeMap<&str, Vec<&hale_graph::DebugScan>> =
        hale_graph::DEBUG_SCANS
            .iter()
            .fold(BTreeMap::new(), |mut m, d| {
                m.entry(d.path).or_default().push(d);
                m
            });
    let mut unlisted = Vec::new();
    let mut seen = 0usize;
    for c in ["hale-types", "hale-codegen"] {
        for (rel, text) in rust_sources(&root, c) {
            for (i, line) in text.lines().enumerate() {
                if !line.contains("format!(\"{:?}\"") {
                    continue;
                }
                seen += 1;
                let listed = frozen
                    .get(rel.as_str())
                    .map(|v| v.iter().any(|d| line.contains(d.fragment)))
                    .unwrap_or(false);
                if !listed {
                    unlisted.push(format!("{rel}:{}: {}", i + 1, line.trim()));
                }
            }
        }
    }
    assert!(
        seen >= 10,
        "the Debug-string scan is vacuous ({seen} occurrences)"
    );
    assert!(
        unlisted.is_empty(),
        "{} Debug-string formatting site(s) are not in the frozen list:\n{}\n\n\
         `format!(\"{{:?}}\", ..)` over an AST value that is then searched as text \
         is a derivation with no name. Decide a fact from the family's table \
         instead; if the site only renders a label, list it in DEBUG_SCANS with \
         `ScanVerdict::Renders` and a distinctive fragment of the line.",
        unlisted.len(),
        unlisted.join("\n")
    );
}

#[test]
fn registry_is_well_formed() {
    use hale_graph::State;
    let mut names = BTreeSet::new();
    let mut problems = Vec::new();
    for f in hale_graph::families() {
        if !names.insert(f.name) {
            problems.push(format!("duplicate family `{}`", f.name));
        }
        match f.state {
            State::Canonical => {
                if f.producer.is_none() {
                    problems.push(format!("`{}` is Canonical without a producer", f.name));
                }
                if !f.legacy.is_empty() {
                    problems.push(format!(
                        "`{}` is Canonical but lists legacy producers",
                        f.name
                    ));
                }
            }
            State::Migrating => {
                if f.legacy.is_empty() {
                    problems.push(format!(
                        "`{}` is Migrating without a legacy inventory",
                        f.name
                    ));
                }
                for l in f.legacy {
                    if l.removal.trim().is_empty() {
                        problems.push(format!(
                            "`{}` legacy `{}` has no removal condition",
                            f.name, l.site.symbol
                        ));
                    }
                }
            }
            State::Reserved => {
                if f.producer.is_some() || !f.legacy.is_empty() {
                    problems.push(format!("`{}` is Reserved but names a producer", f.name));
                }
            }
        }
        if !matches!(f.state, State::Reserved) && f.tests.is_empty() {
            problems.push(format!("`{}` has no focused tests", f.name));
        }
        if f.answers.trim().is_empty() {
            problems.push(format!("`{}` answers nothing", f.name));
        }
        for seam in f.seams {
            if seam.allowed.is_empty() {
                problems.push(format!("`{}` seam `{}` allows nobody", f.name, seam.symbol));
            }
        }
    }
    for r in hale_graph::rules() {
        if !names.contains(r.family) {
            problems.push(format!(
                "rule `{}` names an unknown family `{}`",
                r.id, r.family
            ));
        }
        if r.evaluator.is_none() && !matches!(r.state, State::Reserved) {
            problems.push(format!(
                "rule `{}` is registered without an evaluator: a registered rule with no evaluator fails the build",
                r.id
            ));
        }
    }
    assert!(
        problems.is_empty(),
        "registry problems:\n{}",
        problems.join("\n")
    );
}

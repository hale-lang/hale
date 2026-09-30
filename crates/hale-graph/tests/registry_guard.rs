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
//! 1. **Derivation-shaped definitions are registered, by path.** A
//!    function in a semantic crate whose name starts with `compute_`,
//!    `derive_`, `infer_`, `summarize_` or `classify_` is a
//!    derivation until proven otherwise, and must be the producer, a
//!    legacy producer or an owned helper of some family **in that
//!    file**. A consumer entry registers nothing: a mirror is born by
//!    reusing a producer's name in another crate, and a name alone
//!    would let it through.
//! 2. **Seams carry per-file reference counts.** A family's guarded
//!    entry symbols (`build_bus_graph(`, `resolve_owners(`, ...) may
//!    be referenced only from the files the registry lists, and only
//!    as many times as it lists. A new reference in an allowlisted
//!    file is a new consumer or a new re-derivation, and either way
//!    the registry entry changes first; a cutover shows up as a
//!    decrement.
//! 3. **Debug-string formatting is frozen.** A value rendered with
//!    `{:?}` and read back as text is the cheapest way to derive
//!    without saying so. Every such site in the semantic crates is
//!    listed with a verdict and a count; a new one fails.
//!
//! Each check has a vacuity assertion, so a broken scanner cannot
//! pass by seeing nothing. What the guard does not cover, by design:
//! a derivation named outside the five prefixes, and a call through
//! an alias; both are review's job, and widening the prefix list is a
//! registry change like any other.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p
}

/// The crates whose `src/` may hold semantic derivations.
const SEMANTIC_CRATES: &[&str] = &[
    "hale-syntax",
    "hale-types",
    "hale-model",
    "hale-codegen",
    "hale-cli",
    "hale-lsp",
];

/// Every crate a seam symbol may be referenced from (the scan covers
/// all of them; the allowlist decides). `hale-graph` is the registry
/// itself, not a consumer, and is not scanned.
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

/// Every (path, symbol) the registry names as a producer, a legacy
/// producer, an owned helper or a rule evaluator. Consumers and seams
/// do not register definitions.
fn registered_definitions() -> BTreeSet<(String, String)> {
    let mut s = BTreeSet::new();
    let mut add = |site: &hale_graph::Site| {
        s.insert((site.path.to_string(), site.symbol.to_string()));
    };
    for f in hale_graph::families() {
        if let Some(p) = &f.producer {
            add(p);
        }
        for l in f.legacy {
            add(&l.site);
        }
        for o in f.owned {
            add(o);
        }
    }
    for r in hale_graph::rules() {
        if let Some(e) = &r.evaluator {
            add(e);
        }
    }
    s
}

const DERIVATION_PREFIXES: &[&str] = &["compute_", "derive_", "infer_", "summarize_", "classify_"];

/// Derivation-shaped names that derive nothing about the program,
/// each with the reason. Populated only with a stated reason; the
/// goal state is empty.
const NOT_SEMANTIC: &[(&str, &str, &str)] = &[(
    "crates/hale-syntax/src/fmt.rs",
    "classify_generic_angles",
    "the formatter's lexical classification of `<` and `>`, no program fact",
)];

#[test]
fn derivation_shaped_definitions_are_registered_in_their_file() {
    let root = workspace_root();
    let registered = registered_definitions();
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
                let exempt = NOT_SEMANTIC.iter().any(|(p, n, _)| *p == rel && *n == name);
                if !exempt && !registered.contains(&(rel.clone(), name.clone())) {
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
        "{} derivation-shaped function(s) are not registered in their file:\n{}\n\n\
         A function named compute_/derive_/infer_/summarize_/classify_ \
         derives a fact about the program. Register it in \
         crates/hale-graph/src/registry.rs under the family it belongs to, \
         at this path: as the producer, as a legacy producer with its \
         removal condition, or as an owned helper of the producer; then \
         regenerate spec/registry.md. If it is not a semantic derivation, \
         rename it, or list it in NOT_SEMANTIC with the reason.",
        unregistered.len(),
        unregistered.join("\n")
    );
}

#[test]
fn seam_symbols_are_referenced_only_as_the_registry_counts() {
    let root = workspace_root();
    let mut sources = BTreeMap::new();
    for c in ALL_CRATES {
        for (rel, text) in rust_sources(&root, c) {
            sources.insert(rel, text);
        }
    }
    let mut violations = Vec::new();
    let mut seen_seams = 0usize;
    let mut seen_refs = 0usize;
    for f in hale_graph::families() {
        for seam in f.seams {
            seen_seams += 1;
            let allowed: BTreeMap<&str, usize> = seam.allowed.iter().copied().collect();
            for (rel, text) in &sources {
                let n = text.matches(seam.symbol).count();
                if n == 0 {
                    continue;
                }
                seen_refs += n;
                match allowed.get(rel.as_str()) {
                    Some(&k) if k == n => {}
                    Some(&k) => violations.push(format!(
                        "family `{}`: `{}` is referenced {n} time(s) from {rel}; the registry allows {k}",
                        f.name, seam.symbol
                    )),
                    None => violations.push(format!(
                        "family `{}`: `{}` is referenced from {rel}, which the registry does not list",
                        f.name, seam.symbol
                    )),
                }
            }
            for (rel, k) in seam.allowed {
                if !sources.contains_key(*rel) {
                    violations.push(format!(
                        "family `{}`: seam `{}` allows {rel} ×{k}, which does not exist",
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
        "{} seam reference(s) differ from the registry's counts:\n{}\n\n\
         A new reference to a family's producer is a new consumer or a new \
         re-derivation; a missing one is a cutover. Either way, change the \
         seam's per-file count in crates/hale-graph/src/registry.rs (and the \
         family's consumer entry), then regenerate spec/registry.md.",
        violations.len(),
        violations.join("\n")
    );
}

/// A line that formats a value with `{:?}` (or `{x:?}`, `{:#?}`)
/// inside a formatting macro.
fn is_debug_format_line(line: &str) -> bool {
    line.contains("?}")
        && [
            "format!(",
            "write!(",
            "writeln!(",
            "println!(",
            "eprintln!(",
        ]
        .iter()
        .any(|m| line.contains(m))
}

const DEBUG_SCAN_CRATES: &[&str] = &["hale-types", "hale-codegen", "hale-cli", "hale-lsp"];

#[test]
fn debug_string_formatting_is_frozen() {
    let root = workspace_root();
    let mut frozen: BTreeMap<&str, Vec<(&hale_graph::DebugScan, usize)>> = BTreeMap::new();
    for d in hale_graph::DEBUG_SCANS {
        frozen.entry(d.path).or_default().push((d, 0));
    }
    let mut unlisted = Vec::new();
    let mut seen = 0usize;
    for c in DEBUG_SCAN_CRATES {
        for (rel, text) in rust_sources(&root, c) {
            for (i, line) in text.lines().enumerate() {
                if !is_debug_format_line(line) {
                    continue;
                }
                seen += 1;
                let hit = frozen
                    .get_mut(rel.as_str())
                    .and_then(|v| v.iter_mut().find(|(d, _)| line.contains(d.fragment)));
                match hit {
                    Some((_, n)) => *n += 1,
                    None => unlisted.push(format!("{rel}:{}: {}", i + 1, line.trim())),
                }
            }
        }
    }
    let mut miscounted = Vec::new();
    for (path, v) in &frozen {
        for (d, n) in v {
            if *n != d.count {
                miscounted.push(format!(
                    "{path}: `{}` matches {n} line(s); the registry freezes {}",
                    d.fragment, d.count
                ));
            }
        }
    }
    assert!(
        seen >= 20,
        "the Debug-format scan is vacuous ({seen} lines)"
    );
    assert!(
        unlisted.is_empty() && miscounted.is_empty(),
        "{} Debug-format line(s) are not in the frozen list, {} frozen fragment(s) match a different number of lines:\n{}\n{}\n\n\
         A value rendered with {{:?}} and searched as text is a derivation \
         with no name. Decide the fact from the family's table instead; if \
         the site only renders a label or a message, list it in DEBUG_SCANS \
         with ScanVerdict::Renders, a distinctive fragment of the line, and \
         the number of lines it matches.",
        unlisted.len(),
        miscounted.len(),
        unlisted.join("\n"),
        miscounted.join("\n")
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
                if f.producer.is_some() || !f.legacy.is_empty() || !f.owned.is_empty() {
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

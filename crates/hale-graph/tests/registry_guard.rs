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
//!    file**. So is a function named exactly like one of the producers
//!    `REGISTERED_PRODUCER_NAMES` lists (the ones whose names carry no
//!    such prefix). A consumer entry registers nothing: a mirror is born by
//!    reusing a producer's name in another crate, and a name alone
//!    would let it through.
//! 2. **Seams carry per-file reference counts.** A family's guarded
//!    entry symbols (`build_bus_graph(`, `resolve_owners(`, ...) may
//!    be referenced only from the files the registry lists, and only
//!    as many times as it lists. A reference is the symbol as a whole
//!    name (the character before it is not an identifier character),
//!    so `fresh_factories(` does not count `extend_fresh_factories(`. A new reference in an allowlisted
//!    file is a new consumer or a new re-derivation, and either way
//!    the registry entry changes first; a cutover shows up as a
//!    decrement.
//! 3. **Debug renderings are frozen.** A value rendered with `{:?}`
//!    and read back as text is the cheapest way to derive without
//!    saying so. Every formatting-macro invocation whose template is a
//!    bare rendering (a `?}` placeholder and no prose) in the seven
//!    scanned crates is listed with a verdict and a count, multi-line
//!    invocations included; a new one fails. A message with prose
//!    around its `{:?}` is read by a person and is not a derivation.
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
    "hale-frontend",
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
    "hale-frontend",
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

/// Producers whose names carry none of the prefixes, listed exactly
/// (F.40 phase 1's: widening the prefixes to `resolve_` would take in
/// half the tree). A definition with one of these names is checked like
/// a prefixed one: it is registered in its file, or it is a second
/// producer. `DeclaredNames::of` is a method named `of`, which no name
/// scan can single out; its seam counts its callers instead.
const REGISTERED_PRODUCER_NAMES: &[&str] = &[
    "resolve_binding_facts",
    "child_locus_name",
    "bubble_plans",
    "dispatch_gates",
    "recovery_ops",
    "mint",
    "desugar_intra_locus_topics",
    "desugar_before_check",
    "handler_rows",
];

/// A name the definition scan checks: a derivation prefix, or one of
/// the producers listed by name.
fn derivation_shaped(name: &str) -> bool {
    DERIVATION_PREFIXES.iter().any(|p| name.starts_with(p))
        || REGISTERED_PRODUCER_NAMES.contains(&name)
}

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
                if !derivation_shaped(&name) {
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
    let expected = registered
        .iter()
        .filter(|(path, name)| {
            derivation_shaped(name)
                && SEMANTIC_CRATES
                    .iter()
                    .any(|c| path.starts_with(&format!("crates/{c}/src/")))
        })
        .count();
    assert!(
        seen >= expected,
        "the definition scan saw {seen} derivation-shaped fns but the registry names {expected} in the scanned crates: the scanner is missing definitions"
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

/// How many times `symbol` occurs in `line` as a whole name: the
/// character before it is not an identifier character, so
/// `fresh_factories(` does not also count `extend_fresh_factories(`.
/// A qualified reference (`snapshot::mint(`, `self.bubble_plans(`)
/// still counts.
fn word_bounded_count(line: &str, symbol: &str) -> usize {
    line.match_indices(symbol)
        .filter(|(i, _)| {
            line[..*i]
                .chars()
                .next_back()
                .is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_'))
        })
        .count()
}

#[test]
fn seam_matching_is_word_bounded() {
    assert_eq!(word_bounded_count("fresh_factories(&p)", "fresh_factories("), 1);
    assert_eq!(word_bounded_count("extend_fresh_factories(&p)", "fresh_factories("), 0);
    assert_eq!(word_bounded_count("t.extended_fresh_factories()", "fresh_factories("), 0);
    assert_eq!(word_bounded_count("crate::snapshot::mint(x)", "mint("), 1);
    assert_eq!(word_bounded_count("g.bubble_plans(); g.bubble_plans()", "bubble_plans("), 2);
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
                let n: usize = text
                    .lines()
                    .filter(|l| !l.trim_start().starts_with("//"))
                    .map(|l| word_bounded_count(l.split("//").next().unwrap_or(l), seam.symbol))
                    .sum();
                seen_refs += n;
                match allowed.get(rel.as_str()) {
                    Some(&k) if k == n => {}
                    // No reference where the registry allows some: a
                    // cutover the registry did not record.
                    Some(&k) if n == 0 => violations.push(format!(
                        "family `{}`: `{}` is no longer referenced from {rel}; the registry allows {k}",
                        f.name, seam.symbol
                    )),
                    None if n == 0 => {}
                    Some(&k) => violations.push(format!(
                        "family `{}`: `{}` is referenced {n} time(s) from {rel}; the registry allows {k}",
                        f.name, seam.symbol
                    )),
                    None => violations.push(format!(
                        "family `{}`: `{}` is referenced {n} time(s) from {rel}, which the registry does not list",
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

/// How many times `text` references the shadow facility: the name
/// `shadow` as a whole path segment (`hale_graph::shadow`,
/// `shadow::Report`, `crate::shadow`), or anywhere in a `use hale_graph`
/// tree (`use hale_graph::{shadow, Site}`). Comments do not count, nor
/// does a source file's own `#[cfg(test)] mod`, which is a test. A string
/// literal spelling the path does: the scan errs toward a reference.
fn shadow_references(text: &str) -> usize {
    let lines: Vec<&str> = text.lines().collect();
    let mut n = 0;
    let mut in_use = false;
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim_start();
        if t == "#[cfg(test)]" && lines.get(i + 1).is_some_and(|l| l.trim_start().starts_with("mod ")) {
            break;
        }
        if t.starts_with("//") {
            continue;
        }
        let code = line.split("//").next().unwrap_or(line);
        if code.trim_start().starts_with("use hale_graph") || code.trim_start().starts_with("pub use hale_graph") {
            in_use = true;
        }
        for (k, _) in code.match_indices("shadow") {
            let ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
            let before = code[..k].chars().next_back();
            let after = code[k + "shadow".len()..].chars().next();
            if before.is_some_and(ident) || after.is_some_and(ident) {
                continue;
            }
            if in_use || code[..k].ends_with("::") || code[k..].starts_with("shadow::") {
                n += 1;
            }
        }
        if in_use && code.contains(';') {
            in_use = false;
        }
    }
    n
}

#[test]
fn shadow_references_are_path_segments() {
    assert_eq!(shadow_references("use hale_graph::shadow::{gate_message, Report};\n"), 1);
    assert_eq!(shadow_references("let r = hale_graph::shadow::Report::new(\"f\");\n"), 1);
    assert_eq!(shadow_references("use hale_graph::{\n    shadow,\n    Site,\n};\nshadow::program_id(o, s);\n"), 2);
    assert_eq!(shadow_references("let shadowed = 1; // hale_graph::shadow\nlet shadow = 2;\nfn shadow_return_binding() {}\n"), 0);
    assert_eq!(shadow_references("fn f() {}\n#[cfg(test)]\nmod tests {\n    use hale_graph::shadow::Report;\n}\n"), 0);
}

/// The shadow facility's call sites outside tests are the registry's
/// allowance only (F.40 §5). Every source file under `crates/*/src` is
/// scanned, `hale-graph`'s own included, except the facility's module;
/// the tests are scanned too, as the vacuity check: they are the
/// facility's users today, so a scanner that sees none of them is broken.
#[test]
fn the_shadow_facility_is_called_only_where_the_registry_allows() {
    const FACILITY: &str = "crates/hale-graph/src/shadow.rs";
    let root = workspace_root();
    let allowed: BTreeMap<&str, usize> = hale_graph::SHADOW_CALL_SITES.iter().copied().collect();
    let mut crates: Vec<String> = std::fs::read_dir(root.join("crates"))
        .expect("crates/")
        .flatten()
        .filter(|e| e.path().join("src").is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    crates.sort();
    let mut violations = Vec::new();
    let mut scanned = 0usize;
    for c in &crates {
        for (rel, text) in rust_sources(&root, c) {
            scanned += 1;
            if rel == FACILITY {
                continue;
            }
            let n = shadow_references(&text);
            match allowed.get(rel.as_str()) {
                Some(&k) if k == n => {}
                Some(&k) => violations.push(format!("{rel} references the facility {n} time(s); the registry allows {k}")),
                None if n == 0 => {}
                None => violations.push(format!("{rel} references the facility {n} time(s), which the registry does not allow")),
            }
        }
    }
    for (rel, _) in hale_graph::SHADOW_CALL_SITES {
        if !root.join(rel).is_file() {
            violations.push(format!("the registry allows {rel}, which does not exist"));
        }
    }
    let mut users = 0usize;
    for c in &crates {
        let Ok(entries) = std::fs::read_dir(root.join("crates").join(c).join("tests")) else {
            continue;
        };
        for e in entries.flatten() {
            if let Ok(text) = std::fs::read_to_string(e.path()) {
                users += usize::from(e.path().extension().is_some_and(|x| x == "rs") && shadow_references(&text) > 0);
            }
        }
    }
    assert!(
        scanned >= 100 && crates.iter().any(|c| c == "hale-codegen") && users >= 4,
        "the shadow scan is vacuous ({} crates, {scanned} source files, {users} test files using the facility)",
        crates.len()
    );
    assert!(
        violations.is_empty(),
        "{} file(s) call the shadow facility outside the registry's allowance:\n{}\n\n\
         The shadow facility runs a derivation beside the one it replaces while a family \
         migrates, and phase 3 deleted every such run. A non-test call site is listed in \
         SHADOW_CALL_SITES (crates/hale-graph/src/registry.rs) with its count, and its family's \
         entry says what deletes it; then regenerate spec/registry.md.",
        violations.len(),
        violations.join("\n")
    );
}

/// How many `["std",` path literals `text` holds outside comments: a line
/// that starts `//` (a doc comment too) does not count, nor does what
/// follows `//` on a line.
fn std_path_literals(text: &str) -> usize {
    text.lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .map(|l| l.split("//").next().unwrap_or(l).matches("[\"std\",").count())
        .sum()
}

#[test]
fn std_path_literals_are_counted_outside_comments() {
    assert_eq!(std_path_literals("match segs {\n    [\"std\", \"io\", \"fs\", \"mkdir\"] => 1,\n"), 1);
    assert_eq!(std_path_literals("f(&[\"std\", \"io\"]); g(&[\"std\", \"os\"]);\n"), 2);
    assert_eq!(std_path_literals("/// `[\"std\", ..]` in a doc\n// [\"std\", \"x\"]\nlet a = 1; // [\"std\", \"y\"]\n"), 0);
    assert_eq!(std_path_literals("let p = [\"std\"];\n"), 0);
}

/// No stdlib path literal in codegen outside the registry's allowance
/// (F.40 phase 4's exit): every stdlib call lowers from its row's id, at
/// statement, value and `or` position, so a `["std",` literal in
/// `crates/hale-codegen/src` is a lowering deciding on a path. A seam
/// cannot hold this: a seam is counted in every crate, and the rows
/// themselves, `PATH_RENAMES` and the per-function lists of hale-types
/// spell stdlib paths by design.
#[test]
fn std_path_literals_in_codegen_are_the_registry_allowance() {
    let root = workspace_root();
    let allowed: BTreeMap<&str, usize> =
        hale_graph::CODEGEN_STD_PATH_LITERALS.iter().map(|(p, n, _)| (*p, *n)).collect();
    let sources = rust_sources(&root, "hale-codegen");
    let mut violations = Vec::new();
    let mut found = 0usize;
    for (rel, text) in &sources {
        let n = std_path_literals(text);
        found += n;
        match allowed.get(rel.as_str()) {
            Some(&k) if k == n => {}
            Some(&k) => violations.push(format!("{rel} holds {n} `[\"std\",` literal(s); the registry allows {k}")),
            None if n == 0 => {}
            None => violations.push(format!("{rel} holds {n} `[\"std\",` literal(s), which the registry does not allow")),
        }
    }
    for (rel, _, why) in hale_graph::CODEGEN_STD_PATH_LITERALS {
        if !root.join(rel).is_file() {
            violations.push(format!("the registry allows {rel}, which does not exist"));
        }
        assert!(!why.is_empty(), "{rel}: an allowance carries its reason");
    }
    assert!(
        sources.len() >= 40 && sources.iter().any(|(r, _)| r == "crates/hale-codegen/src/codegen.rs") && found >= 1,
        "the stdlib-literal scan is vacuous ({} codegen source files, {found} literals)",
        sources.len()
    );
    assert!(
        violations.is_empty(),
        "{} file(s) of crates/hale-codegen/src differ from the registry's allowance of stdlib path literals:\n{}\n\n\
         A stdlib call lowers from its row (`hale_types::stdlib_surface::row`): an intrinsic's id \
         picks its arm in `lower_std_intrinsic` or `lower_std_intrinsic_fallible`, and a refusal is \
         an id list. Matching on a `[\"std\", ..]` path decides in lowering what the row should say. \
         A literal that is not a dispatch is listed in CODEGEN_STD_PATH_LITERALS \
         (crates/hale-graph/src/registry.rs) with its count and reason; then regenerate spec/registry.md.",
        violations.len(),
        violations.join("\n")
    );
}

/// Every formatting-macro invocation in `text` whose template holds a
/// `?}` placeholder and no space, collapsed to one line (its first 90
/// characters), nested invocations included. Parentheses inside string
/// literals do not count.
fn debug_renderings(text: &str) -> Vec<String> {
    const MACROS: &[&str] = &[
        "format!(",
        "write!(",
        "writeln!(",
        "println!(",
        "eprintln!(",
    ];
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < text.len() {
        let next = MACROS
            .iter()
            .filter_map(|m| text[i..].find(m).map(|k| (i + k, *m)))
            .min_by_key(|(k, _)| *k);
        let Some((start, m)) = next else { break };
        let mut depth = 0i32;
        let mut in_str = false;
        let mut j = start + m.len() - 1;
        let mut end = None;
        while j < bytes.len() {
            let c = bytes[j];
            if in_str {
                if c == b'\\' {
                    j += 1;
                } else if c == b'"' {
                    in_str = false;
                }
            } else if c == b'"' {
                in_str = true;
            } else if c == b'(' {
                depth += 1;
            } else if c == b')' {
                depth -= 1;
                if depth == 0 {
                    end = Some(j);
                    break;
                }
            }
            j += 1;
        }
        let Some(end) = end else { break };
        let inv = &text[start..=end];
        let template = inv
            .find('"')
            .and_then(|q| inv[q + 1..].find('"').map(|e| &inv[q + 1..q + 1 + e]));
        if let Some(t) = template {
            if t.contains("?}") && !t.contains(' ') {
                let collapsed: String = inv.split_whitespace().collect::<Vec<_>>().join(" ");
                out.push(collapsed.chars().take(90).collect());
            }
        }
        // Continue just past the macro name, not past its closing
        // parenthesis: a bare rendering nested inside another
        // invocation's arguments is an invocation too.
        i = start + m.len();
    }
    out
}

const DEBUG_SCAN_CRATES: &[&str] = &[
    "hale-syntax",
    "hale-types",
    "hale-model",
    "hale-codegen",
    "hale-frontend",
    "hale-cli",
    "hale-lsp",
];

#[test]
fn debug_renderings_are_frozen() {
    let root = workspace_root();
    let mut frozen: BTreeMap<&str, Vec<(&hale_graph::DebugScan, usize)>> = BTreeMap::new();
    for d in hale_graph::DEBUG_SCANS {
        frozen.entry(d.path).or_default().push((d, 0));
    }
    let mut unlisted = Vec::new();
    let mut seen = 0usize;
    for c in DEBUG_SCAN_CRATES {
        for (rel, text) in rust_sources(&root, c) {
            for sig in debug_renderings(&text) {
                seen += 1;
                let hit = frozen
                    .get_mut(rel.as_str())
                    .and_then(|v| v.iter_mut().find(|(d, _)| d.fragment == sig));
                match hit {
                    Some((_, n)) => *n += 1,
                    None => unlisted.push(format!("{rel}: {sig}")),
                }
            }
        }
    }
    let mut miscounted = Vec::new();
    for (path, v) in &frozen {
        for (d, n) in v {
            if *n != d.count {
                miscounted.push(format!(
                    "{path}: `{}` has {n} invocation(s); the registry freezes {}",
                    d.fragment, d.count
                ));
            }
        }
    }
    assert!(
        seen >= 40,
        "the Debug-rendering scan is vacuous ({seen} invocations)"
    );
    assert!(
        unlisted.is_empty() && miscounted.is_empty(),
        "{} Debug rendering(s) are not in the frozen list, {} frozen entries have a different count:\n{}\n{}\n\n\
         A value rendered with {{:?}} and then compared, searched or hashed is a derivation \
         with no name. Decide the fact from the family's table instead; if the rendering only \
         labels a value for a person, list it in DEBUG_SCANS with ScanVerdict::Renders, the \
         invocation collapsed to one line (90 characters), and its count.",
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
        // A seam may allow nobody: the symbol is then one no scanned
        // source may reference (`seam_symbols_are_referenced_only_as_the_registry_counts`
        // fails on the first reference), a tests-only entry point.
    }
    for r in hale_graph::rules() {
        if !names.contains(r.family) {
            problems.push(format!(
                "rule `{}` names an unknown family `{}`",
                r.id, r.family
            ));
        }
        if let hale_graph::Reads::Rows(families) = r.reads {
            if families.is_empty() {
                problems.push(format!("rule `{}` reads no family: say `Declaration`", r.id));
            }
            for f in families {
                if !names.contains(f) {
                    problems.push(format!("rule `{}` reads an unregistered family `{f}`", r.id));
                }
            }
        }
        if r.title.trim().is_empty() {
            problems.push(format!("rule `{}` has no spec title", r.id));
        }
        if !hale_graph::RULE_LISTS
            .iter()
            .any(|l| r.id.starts_with(&format!("{}/", l.key)))
        {
            problems.push(format!("rule `{}` belongs to no list in `RULE_LISTS`", r.id));
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

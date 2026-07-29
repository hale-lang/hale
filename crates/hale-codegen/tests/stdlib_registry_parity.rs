//! R2 completion — the stdlib registry and the codegen dispatch
//! must not drift.
//!
//! The R2 refactor made `hale-types::stdlib_surface` the single
//! table for the stdlib fn surface (name, effect class, and — via
//! `signature_for` — types). But the *lowering* still lives in
//! hand-written `["std", ns, fn] =>` match arms across
//! `crates/hale-codegen/src/`, and nothing forced the two to agree.
//! That is exactly the four-parallel-structures drift R2 set out to
//! kill, only half-killed: adding a fn to the registry and
//! forgetting the arm yields "unknown stdlib function" at lowering
//! time; adding an arm and forgetting the registry yields a
//! path that typechecks as `Ty::Unknown` and silently escapes
//! effect classification (the class of hole that made
//! `std::crypto` invisible downstream — Crumb batch-4 item 3).
//!
//! Generating the arms from the table is the eventual fix; it needs
//! a lowering-shape column the table does not yet carry. Until
//! then this test is the enforcement: **the two lists must cover
//! each other**, and any deliberate exception must be named here
//! with a reason, so drift is a failing build rather than a
//! downstream mystery.
//!
//! **There are THREE lowering structures, not two.** The first cut of
//! this test scraped only `match` arms — and passed, because
//! `hale_stdlib::PATH_RENAMES` rows are *also* `["std", …]` literals
//! and the scraper counted them by accident. Moving that table into
//! its own crate exposed the conflation. The three are:
//!
//!   1. codegen `["std", ns, fn] =>` match arms — native lowering;
//!   2. `PATH_RENAMES` — the path is rewritten to a **Hale-source**
//!      fn/locus declared in `hale_stdlib::AP_SOURCE`;
//!   3. prefix-pattern arms (`bytes::read_*`) covering a family.
//!
//! Counting (2) as coverage is correct — it IS a lowering — but only
//! if the target it names actually exists. `rename_targets_exist`
//! checks that, which the accidental version could not: a row
//! pointing at a deleted Hale fn used to "cover" a registry entry
//! while failing at codegen.

use std::collections::BTreeSet;
use std::path::PathBuf;

/// Paths the codegen lowers but which are deliberately absent from
/// the typecheck surface, each with the reason it is exempt.
fn arm_only_exemptions() -> BTreeSet<String> {
    // Locus / type paths (`std::io::file::File { ... }`) appear in
    // path position but are struct-literal lowering, not path
    // calls — the surface tracks them in its own LOCUS_PATHS list.
    hale_types::stdlib_surface::LOCUS_PATHS
        .iter()
        .map(|p| p.join("::"))
        .collect()
}

/// Registry entries with no dispatch arm, each with its reason.
fn registry_only_exemptions() -> BTreeSet<String> {
    BTreeSet::new()
}

/// Prefix-pattern dispatch arms — `["std", "bytes", n] if
/// n.starts_with("read_")` covers a whole family with one arm, which
/// the literal scraper cannot see. Returns (namespace, prefix)
/// pairs scraped from the source so the family counts as covered
/// without hand-listing 34 names.
fn prefix_pattern_covers() -> Vec<(String, String)> {
    let src_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut out = Vec::new();
    let mut stack = vec![src_dir];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            if p.extension().map(|x| x != "rs").unwrap_or(true) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&p) else { continue };
            for line in text.lines() {
                let Some(ns_start) = line.find("[\"std\", \"") else { continue };
                if !line.contains("starts_with(") {
                    continue;
                }
                let after = &line[ns_start + 9..];
                let Some(q) = after.find('"') else { continue };
                let ns = after[..q].to_string();
                let Some(sw) = line.find("starts_with(\"") else { continue };
                let rest = &line[sw + 13..];
                let Some(q2) = rest.find('"') else { continue };
                out.push((ns, rest[..q2].to_string()));
            }
        }
    }
    out
}

/// Whole-namespace dispatch arms: `["std", "io", "mirror", op] =>`
/// binds the leaf and handles every fn in that namespace with one
/// arm. The literal scraper cannot see those (the last segment is an
/// identifier, not a string), so without this a fully-dispatched
/// namespace reads as entirely uncovered.
fn namespace_wildcard_arms() -> Vec<Vec<String>> {
    let src_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut out = Vec::new();
    let mut stack = vec![src_dir];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            if p.extension().map(|x| x != "rs").unwrap_or(true) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&p) else { continue };
            for line in text.lines() {
                let Some(s) = line.find("[\"std\", \"") else { continue };
                let Some(e_rel) = line[s..].find(']') else { continue };
                let inner = &line[s + 1..s + e_rel];
                let parts: Vec<&str> =
                    inner.split(',').map(|x| x.trim()).collect();
                // literal segments then exactly one bare identifier
                let (last, head) = match parts.split_last() {
                    Some(x) => x,
                    None => continue,
                };
                let head_literal = head
                    .iter()
                    .all(|x| x.len() >= 2 && x.starts_with('"') && x.ends_with('"'));
                let last_is_ident = !last.starts_with('"')
                    && last.chars().all(|c| c.is_alphanumeric() || c == '_')
                    && !last.is_empty();
                if head_literal && last_is_ident && head.len() >= 2 {
                    out.push(
                        head.iter()
                            .map(|x| x.trim_matches('"').to_string())
                            .collect(),
                    );
                }
            }
        }
    }
    out
}

fn dispatch_arm_paths() -> BTreeSet<String> {
    let src_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut out = BTreeSet::new();
    let mut stack = vec![src_dir];
    let re_start = "[\"std\", \"";
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            if p.extension().map(|x| x != "rs").unwrap_or(true) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&p) else { continue };
            let mut idx = 0;
            while let Some(found) = text[idx..].find(re_start) {
                let start = idx + found;
                let Some(end_rel) = text[start..].find(']') else { break };
                let raw = &text[start..start + end_rel + 1];
                idx = start + end_rel + 1;
                // Parse ["std", "a", "b"] -> std::a::b
                // Only LITERAL segments count: `["std", "bytes", n]`
                // is a match arm binding a variable, not a path.
                let inner = raw.trim_matches(|c| c == '[' || c == ']');
                let parts: Vec<&str> = inner.split(',').map(|s| s.trim()).collect();
                let all_literal = parts
                    .iter()
                    .all(|s| s.len() >= 2 && s.starts_with('"') && s.ends_with('"'));
                if !all_literal {
                    continue;
                }
                let segs: Vec<String> = parts
                    .iter()
                    .map(|s| s.trim_matches('"').to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                if segs.len() >= 2 && segs[0] == "std" {
                    out.insert(segs.join("::"));
                }
            }
        }
    }
    out
}

/// Structure (2): paths lowered by rewriting to a Hale-source
/// fn/locus. Just as much a lowering as a match arm.
fn rename_paths() -> BTreeSet<String> {
    hale_stdlib::PATH_RENAMES
        .iter()
        .map(|(path, _)| path.join("::"))
        .collect()
}

/// Every name a rename row points at must actually be declared in
/// the Hale-source stdlib. Without this, a stale row silently
/// "covers" a registry entry that cannot lower.
#[test]
fn rename_targets_exist() {
    let src = hale_stdlib::AP_SOURCE;
    let declared: BTreeSet<&str> = src
        .lines()
        .filter_map(|l| {
            let l = l.trim_start();
            for kw in ["fn ", "locus ", "type ", "interface ", "perspective "] {
                if let Some(rest) = l.strip_prefix(kw) {
                    let name: &str = rest
                        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                        .next()
                        .unwrap_or("");
                    if !name.is_empty() {
                        return Some(name);
                    }
                }
            }
            None
        })
        .collect();
    // Compiler-SYNTHESIZED types: declared by codegen at lowering
    // time (`declare_builtin_parse_error_type`), not by Hale source,
    // so they are legitimately absent from AP_SOURCE.
    const SYNTHESIZED: &[&str] = &["ParseError"];
    let missing: Vec<String> = hale_stdlib::PATH_RENAMES
        .iter()
        .filter(|(_, target)| {
            !declared.contains(target) && !SYNTHESIZED.contains(target)
        })
        .map(|(path, target)| format!("{} -> {}", path.join("::"), target))
        .collect();
    assert!(
        missing.is_empty(),
        "these `PATH_RENAMES` rows point at names that are NOT declared \
         anywhere in `hale_stdlib::AP_SOURCE` ({} of {}):\n{:#?}\n\n\
         A stale rename row makes the parity check think the path is \
         lowered while codegen will fail on it.",
        missing.len(),
        hale_stdlib::PATH_RENAMES.len(),
        missing
    );
}

fn registry_paths() -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for s in hale_types::stdlib_surface::SURFACES {
        for e in s.fns {
            let mut segs = vec!["std".to_string()];
            segs.extend(s.ns.iter().map(|x| x.to_string()));
            segs.push(e.name.to_string());
            out.insert(segs.join("::"));
        }
    }
    out
}

/// Every registry entry must have a lowering. A name the
/// typechecker accepts but codegen cannot lower is a build failure
/// deferred to the worst possible moment.
#[test]
fn every_registry_entry_has_a_dispatch_arm() {
    let arms = dispatch_arm_paths();
    let registry = registry_paths();
    let exempt = registry_only_exemptions();
    let prefixes = prefix_pattern_covers();
    let wildcards = namespace_wildcard_arms();
    let covered_by_wildcard = |path: &str| -> bool {
        wildcards.iter().any(|ns| {
            path.starts_with(&format!("{}::", ns.join("::")))
                && path.split("::").count() == ns.len() + 1
        })
    };
    let covered_by_prefix = |path: &str| -> bool {
        prefixes.iter().any(|(ns, pre)| {
            path.strip_prefix(&format!("std::{}::", ns))
                .map(|leaf| leaf.starts_with(pre.as_str()))
                .unwrap_or(false)
        })
    };
    let renames = rename_paths();
    let missing: Vec<&String> = registry
        .iter()
        .filter(|p| {
            !arms.contains(*p)
                && !renames.contains(*p)
                && !exempt.contains(*p)
                && !covered_by_prefix(p)
                && !covered_by_wildcard(p)
        })
        .collect();
    assert!(
        missing.is_empty(),
        "these stdlib registry entries have no codegen dispatch arm — \
         they typecheck but cannot lower ({} of {}):\n{:#?}\n\n\
         Either add the lowering, or add the path to \
         `registry_only_exemptions()` with the reason it is not a \
         path-call.",
        missing.len(),
        registry.len(),
        missing
    );
}

/// Every lowered path must be in the registry. A path codegen
/// handles but the surface does not know types as `Ty::Unknown`,
/// which silently disables fallibility/arity checking AND exempts
/// it from effect classification — the hole that let `std::crypto`
/// look nonexistent from outside.
#[test]
fn every_dispatch_arm_is_in_the_registry() {
    let arms = dispatch_arm_paths();
    let registry = registry_paths();
    let exempt = arm_only_exemptions();
    let orphans: Vec<&String> = arms
        .iter()
        .filter(|p| !registry.contains(*p) && !exempt.contains(*p))
        // Internal primitives (`__name`) are intentionally hidden
        // from the user-facing surface.
        .filter(|p| {
            !p.rsplit("::").next().map(|n| n.starts_with("__")).unwrap_or(false)
        })
        .collect();
    assert!(
        orphans.is_empty(),
        "these paths are lowered by codegen but absent from the stdlib \
         registry — they type as `Ty::Unknown` (no fallibility/arity \
         checking) and escape effect classification ({} found):\n{:#?}\n\n\
         Add them to `stdlib_surface::SURFACES` with an effect class, or \
         to `arm_only_exemptions()` with a reason.",
        orphans.len(),
        orphans
    );
}

/// The parity check is only meaningful if it is actually seeing
/// both lists — a scraper that silently matched nothing would make
/// both tests above pass vacuously.
#[test]
fn parity_check_is_not_vacuous() {
    let arms = dispatch_arm_paths();
    let registry = registry_paths();
    assert!(
        arms.len() > 100,
        "dispatch-arm scraper found only {} paths — it is not reading the \
         source it thinks it is",
        arms.len()
    );
    assert!(
        registry.len() > 200,
        "registry has only {} entries — unexpected",
        registry.len()
    );
    // And they must genuinely overlap, not just both be non-empty.
    let overlap = arms.intersection(&registry).count();
    assert!(
        overlap > 100,
        "only {} paths appear in BOTH lists — the two scrapers are \
         producing different shapes, so the parity assertions are \
         comparing apples to oranges",
        overlap
    );
}

//! R2 completion — the stdlib registry and the codegen dispatch
//! must not drift.
//!
//! `hale_types::stdlib_surface::SURFACES` holds one row per stdlib
//! function, and since F.40 phase 4 (S2) the row says how the function
//! lowers (`Lower`): natively (`Intrinsic(id)`), by a Hale body of the
//! stdlib seeds called by name (`HaleBody`), through
//! `hale_stdlib::PATH_RENAMES` (`Renamed`), or not at all (`Unlowered`).
//!
//! Since S3 the statement and value positions dispatch from the row
//! (`lower_std_call`), and since S4 the `or` position does too
//! (`lower_std_fallible_call`), so the compiler holds what this test
//! used to scrape: an `Intrinsic` row's id has an arm in
//! `lower_std_intrinsic`'s and in `lower_std_intrinsic_fallible`'s
//! exhaustive `match id`, a `HaleBody` row is called by the body its row
//! names, and a `Renamed` or `Unlowered` row reaches the fallback (under
//! `or`, the caller's resolution as a function: no id, so no fallible
//! arm can match it first). What is left here:
//!
//!   - every `Intrinsic` row is lowered at some position (an id whose
//!     arms only answer "not implemented" at both bare positions and
//!     "not a fallible call" under `or` would compile);
//!   - every `HaleBody` row names a function the stdlib declares;
//!   - a `Renamed` row is in `PATH_RENAMES`; an `Unlowered` row has
//!     neither an arm nor a rename, and is named in [`UNLOWERED`];
//!   - an all-literal `["std", ..]` path that codegen's source still
//!     spells names a row or a locus path (none does today).
//!
//! Before the column, this file could only check that the scraped
//! literals and the registry's names covered each other: adding a fn to
//! the registry and forgetting the arm yielded "unknown stdlib function"
//! at lowering time; adding an arm and forgetting the registry yielded a
//! path that typechecked as `Ty::Unknown` and escaped effect
//! classification (the class of hole that made `std::crypto` invisible
//! downstream).
//!
//! `rename_targets_exist` checks the third structure's other end: a
//! `PATH_RENAMES` row pointing at a deleted Hale fn would "lower" a
//! registry row while failing at codegen.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use hale_types::stdlib_surface::{self as surf, Lower};

use crate::stdlib_dispatch_coverage::{scrape, ArmCall, ArmKind, Position};

/// The rows no dispatcher lowers and no rename reaches, each with its
/// reason. A new one is a public name that cannot be lowered, so it is
/// named here deliberately.
const UNLOWERED: &[(&str, &str)] = &[(
    "std::io::file::close",
    "a signature row the surface never listed, with no arm and no rename (the descriptor close the seeds call is `__close`)",
)];

/// What the arms of the given positions do with each path: for every
/// arm that lowers it, the call that arm makes; and whether some arm
/// refuses it.
struct Dispatched {
    lowering: Vec<ArmCall>,
    refused: bool,
}

fn dispatched(positions: &[Position]) -> BTreeMap<String, Dispatched> {
    let mut out: BTreeMap<String, Dispatched> = BTreeMap::new();
    for s in scrape().into_iter().filter(|s| positions.contains(&s.position)) {
        for (path, (kind, _)) in &s.paths {
            let d = out.entry(path.clone()).or_insert(Dispatched { lowering: Vec::new(), refused: false });
            match kind {
                ArmKind::Lowers => d.lowering.push(s.calls[path].clone()),
                ArmKind::Refuses => d.refused = true,
            }
        }
    }
    out
}

const ALL: &[Position] = &[Position::Statement, Position::Expression, Position::Fallible];

fn renames() -> BTreeSet<String> {
    hale_stdlib::PATH_RENAMES.iter().map(|(path, _)| path.join("::")).collect()
}

fn rows() -> BTreeMap<String, Lower> {
    surf::rows().map(|(s, f)| (format!("std::{}::{}", s.ns.join("::"), f.name), f.lower)).collect()
}

/// Every `Intrinsic` row is lowered at some position, every `HaleBody`
/// row names a declared body, every `Renamed` row is a rename, and the
/// `Unlowered` rows are named.
#[test]
fn every_rows_lowering_is_where_the_column_says() {
    let lowered = dispatched(ALL);
    let renames = renames();
    let declared = declared_names();
    let mut wrong = Vec::new();
    let mut unlowered = Vec::new();
    for (path, lower) in rows() {
        let armed = lowered.get(&path).is_some_and(|d| !d.lowering.is_empty());
        match lower {
            Lower::Intrinsic(_) if !armed => {
                wrong.push(format!("{path}: {lower:?}, but no position lowers it"))
            }
            Lower::HaleBody(body) if !declared.contains(body) => {
                wrong.push(format!("{path}: HaleBody({body:?}), but the stdlib declares no `{body}`"))
            }
            Lower::Renamed if !renames.contains(&path) => {
                wrong.push(format!("{path}: Renamed, but not in PATH_RENAMES"))
            }
            Lower::Unlowered if lowered.contains_key(&path) || renames.contains(&path) => {
                wrong.push(format!("{path}: Unlowered, but an arm or a rename reaches it"))
            }
            Lower::Unlowered => unlowered.push(path),
            _ => {}
        }
    }
    assert!(wrong.is_empty(), "rows whose lowering is not where they say ({}):\n{:#?}", wrong.len(), wrong);
    let named: Vec<String> = UNLOWERED.iter().map(|(p, _)| p.to_string()).collect();
    assert_eq!(
        unlowered, named,
        "the rows nothing lowers are named in `UNLOWERED` with their reason: a public name \
         the checker accepts and lowering cannot lower fails at the worst possible moment"
    );
}

/// The `["std", ..]` literals anywhere in codegen's source — the three
/// dispatchers and every helper — name a row or a locus path. A literal
/// naming neither is a path lowering knows and the checker does not.
#[test]
fn every_std_literal_in_codegen_names_a_row_or_a_locus() {
    let rows = rows();
    let locus: BTreeSet<String> = surf::LOCUS_PATHS.iter().map(|p| p.join("::")).collect();
    let orphans: Vec<String> =
        std_literals().into_iter().filter(|p| !rows.contains_key(p) && !locus.contains(p)).collect();
    assert!(
        orphans.is_empty(),
        "these paths are spelled in codegen but have no row in `stdlib_surface::SURFACES` — \
         they type as `Ty::Unknown` and escape effect classification ({}):\n{:#?}",
        orphans.len(),
        orphans
    );
}

/// `std::io::tcp::__connect` -> `IoTcpConnectRaw`: the segments after
/// `std` in CamelCase, a leading `__` as a trailing `Raw`.
fn intrinsic_name(path: &str) -> String {
    let camel = |seg: &str| -> String {
        seg.split('_')
            .filter(|p| !p.is_empty())
            .map(|p| {
                let p = p.to_ascii_lowercase();
                p[..1].to_ascii_uppercase() + &p[1..]
            })
            .collect()
    };
    let segs: Vec<&str> = path.split("::").skip(1).collect();
    let (last, ns) = segs.split_last().expect("a function segment");
    let mut out: String = ns.iter().map(|s| camel(s)).collect();
    match last.strip_prefix("__") {
        Some(rest) => out.push_str(&(camel(rest) + "Raw")),
        None => out.push_str(&camel(last)),
    }
    out
}

/// An intrinsic's id is its path's mechanical name, which is injective
/// on paths, so the id names its row and no other.
#[test]
fn every_intrinsic_id_is_its_paths_name() {
    let mut wrong = Vec::new();
    for (path, lower) in rows() {
        if let Lower::Intrinsic(id) = lower {
            let want = intrinsic_name(&path);
            if format!("{id:?}") != want {
                wrong.push(format!("{path}: {id:?}, want {want}"));
            }
        }
    }
    assert!(wrong.is_empty(), "intrinsic ids that are not their path's name:\n{wrong:#?}");
}

/// Every all-literal `["std", ..]` slice in codegen's source.
fn std_literals() -> BTreeSet<String> {
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
                // Only LITERAL segments count: `["std", "bytes", n]`
                // is a match arm binding a variable, not a path.
                let inner = raw.trim_matches(|c| c == '[' || c == ']');
                let parts: Vec<&str> = inner.split(',').map(|s| s.trim()).collect();
                if !parts.iter().all(|s| s.len() >= 2 && s.starts_with('"') && s.ends_with('"')) {
                    continue;
                }
                let segs: Vec<&str> = parts.iter().map(|s| s.trim_matches('"')).filter(|s| !s.is_empty()).collect();
                if segs.len() >= 3 && segs[0] == "std" {
                    out.insert(segs.join("::"));
                }
            }
        }
    }
    out
}

/// Every name the Hale-source stdlib declares (a fn, locus, type,
/// interface or perspective).
fn declared_names() -> BTreeSet<&'static str> {
    hale_stdlib::AP_SOURCE
        .lines()
        .filter_map(|l| {
            // GH #436: skip leading decorators. A declaration may be
            // annotated (`@sealed locus X`, `@form(vec) locus Y`), and
            // matching on a bare `locus ` prefix made every annotated
            // stdlib decl invisible here — so a rename row pointing at
            // one read as stale. Caught when `std::secret::Signer`
            // landed; it would equally have hidden a `@form` locus.
            let mut l = l.trim_start();
            while let Some(rest) = l.strip_prefix('@') {
                let after_name =
                    rest.trim_start_matches(|c: char| {
                        c.is_alphanumeric() || c == '_'
                    });
                // An arg list, if there is one: `@form(vec, cap = 8)`.
                let after_args = match after_name.strip_prefix('(') {
                    Some(args) => match args.find(')') {
                        Some(i) => &args[i + 1..],
                        None => break,
                    },
                    None => after_name,
                };
                l = after_args.trim_start();
            }
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
        .collect()
}

/// Every name a rename row points at must actually be declared in
/// the Hale-source stdlib. Without this, a stale row silently
/// "lowers" a `Renamed` registry row that cannot lower.
#[test]
fn rename_targets_exist() {
    let declared = declared_names();
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

/// The checks above are only meaningful if they see both sides: a
/// scraper that silently matched nothing, or a column that said one
/// thing everywhere, would make them pass vacuously.
#[test]
fn parity_check_is_not_vacuous() {
    let dispatched = dispatched(ALL);
    let rows = rows();
    assert!(
        dispatched.len() > 300,
        "the dispatcher scrape found only {} paths — it is not reading the source it thinks it is",
        dispatched.len()
    );
    let fallible = self::dispatched(&[Position::Fallible]);
    assert!(fallible.len() > 150, "the fallible dispatcher scrape found only {} paths", fallible.len());
    assert!(
        fallible.values().filter(|d| !d.lowering.is_empty()).count() > 100,
        "the fallible dispatcher scrape found few lowering arms"
    );
    assert!(rows.len() > 400, "the table has only {} rows — unexpected", rows.len());
    let both = dispatched.keys().filter(|p| rows.contains_key(*p)).count();
    assert!(both > 300, "only {both} paths are both dispatched and rows");
    let count = |f: fn(&Lower) -> bool| rows.values().filter(|l| f(l)).count();
    let intrinsic = count(|l| matches!(l, Lower::Intrinsic(_)));
    let body = count(|l| matches!(l, Lower::HaleBody(_)));
    let renamed = count(|l| matches!(l, Lower::Renamed));
    assert!(intrinsic > 300, "only {intrinsic} Intrinsic rows");
    assert!(body > 30, "only {body} HaleBody rows");
    assert!(renamed > 30, "only {renamed} Renamed rows");
    // And the arm classification sees both kinds of arm.
    let hale_arms = dispatched.values().flat_map(|d| &d.lowering).filter(|c| matches!(c, ArmCall::HaleBody(_))).count();
    assert!(hale_arms > 40, "only {hale_arms} arms call a Hale body by name");
    // Every position dispatches from the row (S3, S4), so codegen spells
    // no all-literal `["std", ..]` path and `std_literals` finds none: the
    // literal test above has nothing to check until one returns.
}

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
//!     arms only refuse, at both bare positions and under `or`, would
//!     compile);
//!   - which calls lowering refuses is the rows' fallibility (S5);
//!   - every `HaleBody` row names a function the stdlib declares;
//!   - a `Renamed` row is in `PATH_RENAMES`; an `Unlowered` row has
//!     neither an arm nor a rename, and is named in [`UNLOWERED`];
//!   - an all-literal `["std", ..]` path that codegen's source still
//!     spells names a row or a locus path (none does today; the registry
//!     guard holds codegen's `["std",` literals to its allowance).
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

/// F.40 phase 4, S5: which stdlib calls lowering refuses is the rows'
/// fallibility, not a list of its own. Under `or`, every id whose row is
/// fallible has a lowering arm in `lower_std_intrinsic_fallible` and no
/// id whose row is not fallible has one: the rest are its last arm, which
/// refuses (`or_over_an_infallible_row`). At a bare position the arm
/// that refuses (`bare_call_of_a_fallible_row`) is every id whose row is
/// fallible, and no other. The exceptions are the arms the rulings after
/// this one remove, named in `stdlib_dispatch_coverage`; an id outside
/// them that disagrees is a finding, not a repair.
#[test]
fn which_calls_lowering_refuses_is_the_rows_fallibility() {
    use crate::stdlib_dispatch_coverage::{
        fallible_id_arms, id_arms, Branch, DEAD_BARE_OF_FALLIBLE_ROWS, OR_LOWERS_AN_INFALLIBLE_ROW,
    };
    let mut fallible = BTreeSet::new();
    let mut path_of = BTreeMap::new();
    for (s, f) in surf::rows() {
        if let Lower::Intrinsic(id) = f.lower {
            let id = format!("{id:?}");
            path_of.insert(id.clone(), format!("std::{}::{}", s.ns.join("::"), f.name));
            if f.sig.is_some_and(|s| s.fallible.is_some()) {
                fallible.insert(id);
            }
        }
    }
    let excepted = |list: &[&str], id: &String| list.contains(&path_of[id].as_str());

    let mut or_lowers = BTreeSet::new();
    let mut or_refuses = BTreeSet::new();
    for (_, ids, lowers) in fallible_id_arms() {
        (if lowers { &mut or_lowers } else { &mut or_refuses }).extend(ids);
    }
    let fallible_unarmed: Vec<&String> = fallible.difference(&or_lowers).collect();
    assert!(fallible_unarmed.is_empty(), "fallible rows `or` does not lower: {fallible_unarmed:?}");
    let infallible_armed: Vec<&String> =
        or_lowers.difference(&fallible).filter(|id| !excepted(OR_LOWERS_AN_INFALLIBLE_ROW, id)).collect();
    assert!(infallible_armed.is_empty(), "rows that cannot fail, lowered under `or`: {infallible_armed:?}");
    assert!(or_refuses.is_disjoint(&fallible), "a fallible row refused under `or`");
    assert_eq!(or_lowers.len() + or_refuses.len(), path_of.len(), "every id has one arm under `or`");

    let mut bare_refuses = BTreeSet::new();
    let mut bare_lowers = BTreeSet::new();
    for a in id_arms() {
        for id in a.ids {
            match (a.value, a.statement) {
                (Branch::Refuses, None) => bare_refuses.insert(id),
                (Branch::Refuses, Some(_)) => panic!("`Id::{id}`: a bare refusal branches on the position"),
                (Branch::Lowers, _) | (_, Some(Branch::Lowers)) => bare_lowers.insert(id),
                _ => false,
            };
        }
    }
    let unrefused: Vec<&String> = fallible
        .difference(&bare_refuses)
        .filter(|id| !(excepted(DEAD_BARE_OF_FALLIBLE_ROWS, id) && bare_lowers.contains(*id)))
        .collect();
    assert!(unrefused.is_empty(), "fallible rows a bare call does not refuse: {unrefused:?}");
    let wrongly: Vec<&String> = bare_refuses.difference(&fallible).collect();
    assert!(wrongly.is_empty(), "rows that cannot fail, refused bare as fallible: {wrongly:?}");
    assert!(bare_refuses.len() > 100 && or_refuses.len() > 200, "the scrape found the refusal arms");
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
    // literal test above has nothing to check until one returns, and the
    // registry guard (`std_path_literals_in_codegen_are_the_registry_allowance`)
    // refuses one outside its allowance first.
}

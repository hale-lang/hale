//! The typing stage's reuse (F.40 phase 3, X2): what of a previous
//! snapshot's typing a new snapshot of the same seed may keep.
//!
//! Opt-in (`Snapshot::reusing_typing`), and only the editor opts in. The
//! snapshot key stays the whole seed's (every source unit's text): the
//! reuse is named by a second key, the load's entry, mode, target, config
//! and import renames ([`ReuseKey`]), and a snapshot whose second key
//! differs from its previous one's checks whole.
//!
//! The rule, per top-level declaration of the seed's programs, the two
//! snapshots' declarations corresponding by program, kind, name and
//! order:
//!
//! - **unchanged**: the previous declaration moved by the distance its
//!   start moved is the new one, position for position. The check's
//!   per-declaration passes (the checker's walk, the reveal rule;
//!   `hale_types::check::DeclChecked`) report for it what they reported
//!   before, moved the same distance, so their result is reused — unless
//!   one of its diagnostics has a position outside the declaration,
//!   which the move would not carry, or the declaration is a dependent;
//! - **changed in a body**: the two are one declaration once every
//!   position and every body (a fn's, a method's, a hook's, a mode's, an
//!   `on_failure`'s) is set aside. It is checked again, and so is every
//!   declaration the families name as its dependent
//!   (`Snapshot::declaration_dependents`), in the previous snapshot (an
//!   edge the edit removed) and in the new one (an edge it added);
//! - **anything else** checks the seed whole: a declaration added,
//!   removed, renamed or reordered, an edit to what a declaration
//!   declares (read through the scope, which no family records), a
//!   changed declaration the families do not place
//!   (`Dependents::Whole`), a seed whose files changed. A seed with a
//!   hole is not typed at all.
//!
//! Everything else the typing stage runs (the scope, the bundle-wide
//! rules, the build rules, the advisory) runs whole every time, so the
//! incremental stage equals the full one by construction wherever the
//! per-declaration passes of an unchanged declaration that is no
//! dependent report what they reported; `crates/hale-cli/tests/lsp.rs`
//! (`the_incremental_typing_stage_is_the_full_one`) holds the two equal
//! over every parity seed and an edit sequence over `dna/host`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use hale_graph::ids::SiteId;
use hale_syntax::ast::{LocusMember, Program, TopDecl};
use hale_syntax::shift::{erase_positions, shift_item};
use hale_syntax::{Diag, SpanOrigin};
use hale_types::check::{ByDeclaration, DeclChecked};

use crate::dependents::{kind_and_name, Declaration, Dependents};

/// What names a reuse: two snapshots whose reuse keys are equal load one
/// seed under one config, whatever text they read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReuseKey {
    pub entry: PathBuf,
    pub mode: Option<crate::frontend::LoadMode>,
    pub target: String,
    pub config_digest: u64,
    /// The import renames, as a set.
    pub import_renames: u64,
}

/// What a snapshot's typing stage reused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypingReuse {
    /// No previous snapshot was offered: the whole check (every entry
    /// point but the editor's).
    Fresh,
    /// A previous snapshot was offered and the seed was checked whole,
    /// for this reason.
    Whole(&'static str),
    /// The per-declaration passes ran for `checked` declarations and were
    /// reused for `reused`.
    Reused { checked: usize, reused: usize },
}

/// One side of a reuse: a snapshot's programs, declarations, and its
/// dependents relation.
pub(crate) struct Side<'a> {
    pub programs: &'a BTreeMap<PathBuf, Program>,
    pub decls: &'a [Declaration],
    pub dependents: &'a dyn Fn(SiteId) -> Option<Dependents>,
}

/// Per program key (the bundle's spelling of the path), per item, the
/// previous result to stand in for the passes, or `None` to run them.
pub(crate) type Plan = BTreeMap<String, Vec<Option<DeclChecked>>>;

/// The reuse of `prev`'s per-declaration results (`checked`) in `next`,
/// or why the seed is checked whole.
pub(crate) fn plan(prev: &Side<'_>, checked: &ByDeclaration, next: &Side<'_>) -> Result<(Plan, TypingReuse), &'static str> {
    if !prev.programs.keys().eq(next.programs.keys()) {
        return Err("the seed's programs changed");
    }
    if prev.decls.len() != next.decls.len()
        || prev
            .decls
            .iter()
            .zip(next.decls)
            .any(|(a, b)| (&a.program, a.index, a.kind, &a.name) != (&b.program, b.index, b.kind, &b.name))
    {
        return Err("a declaration was added, removed, renamed or reordered");
    }
    // Per declaration (an index into both lists): the distance it moved,
    // when it is unchanged.
    let mut moved: Vec<Option<u32>> = Vec::with_capacity(next.decls.len());
    let mut changed: Vec<usize> = Vec::new();
    for (i, (a, b)) in prev.decls.iter().zip(next.decls).enumerate() {
        let old = &prev.programs[&a.program].items[a.index];
        let new = &next.programs[&b.program].items[b.index];
        let delta = b.span.start.0.wrapping_sub(a.span.start.0);
        let same = match delta {
            0 => old == new,
            _ => {
                let mut old = old.clone();
                shift_item(&mut old, delta);
                old == *new
            }
        };
        if same {
            moved.push(Some(delta));
        } else if same_declared_surface(old, new) {
            moved.push(None);
            changed.push(i);
        } else {
            return Err("an edit changed what a declaration declares");
        }
    }
    let mut recheck: BTreeSet<usize> = changed.iter().copied().collect();
    for side in [prev, next] {
        let index: BTreeMap<SiteId, usize> =
            side.decls.iter().enumerate().filter_map(|(i, d)| Some((d.site?, i))).collect();
        for &c in &changed {
            let site = side.decls[c].site.ok_or("a changed declaration has no site")?;
            match (side.dependents)(site) {
                None => return Err("the families are blocked"),
                Some(Dependents::Whole(why)) => return Err(why),
                Some(Dependents::Decls(sites)) => recheck.extend(sites.iter().filter_map(|s| index.get(s))),
            }
        }
    }
    let mut plan: Plan = BTreeMap::new();
    let mut reused = 0;
    for (i, (a, b)) in prev.decls.iter().zip(next.decls).enumerate() {
        let key = b.program.display().to_string();
        let row = plan.entry(key.clone()).or_default();
        let kept = match (recheck.contains(&i), moved[i]) {
            (false, Some(delta)) => checked
                .get(&key)
                .and_then(|rows| rows.get(a.index))
                .filter(|done| done.typing.iter().chain(&done.reveal).all(|d| inside(d, a)))
                .map(|done| DeclChecked {
                    typing: done.typing.iter().map(|d| d.clone().shifted(delta)).collect(),
                    reveal: done.reveal.iter().map(|d| d.clone().shifted(delta)).collect(),
                }),
            _ => None,
        };
        reused += kept.is_some() as usize;
        debug_assert_eq!(row.len(), b.index);
        row.push(kept);
    }
    let total = next.decls.len();
    Ok((plan, TypingReuse::Reused { checked: total - reused, reused }))
}

/// Every position of the diagnostic in the seed's space lies inside the
/// declaration, so moving the declaration moves it.
fn inside(d: &Diag, decl: &Declaration) -> bool {
    let (start, end) = (decl.span.start.0, decl.span.end.0);
    let within = |origin: SpanOrigin, s: hale_syntax::Span| origin != SpanOrigin::Seed || (start <= s.start.0 && s.end.0 <= end);
    within(d.origin, d.span) && d.related.iter().all(|r| within(r.origin, r.span))
}

/// The two declarations declare the same thing: equal once every
/// position is erased and every body is set aside.
fn same_declared_surface(old: &TopDecl, new: &TopDecl) -> bool {
    if kind_and_name(old) != kind_and_name(new) || !matches!(old, TopDecl::Fn(_) | TopDecl::Locus(_)) {
        return false;
    }
    let surface = |d: &TopDecl| {
        let mut d = d.clone();
        set_bodies_aside(&mut d);
        erase_positions(&mut d);
        d
    };
    surface(old) == surface(new)
}

fn set_bodies_aside(d: &mut TopDecl) {
    let empty = |b: &mut hale_syntax::ast::Block| {
        b.stmts.clear();
        b.tail = None;
    };
    match d {
        TopDecl::Fn(f) => empty(&mut f.body),
        TopDecl::Locus(l) => {
            for m in &mut l.members {
                match m {
                    LocusMember::Fn(f) => empty(&mut f.body),
                    LocusMember::Lifecycle(lc) => empty(&mut lc.body),
                    LocusMember::Mode(md) => empty(&mut md.body),
                    LocusMember::Failure(fd) => empty(&mut fd.body),
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

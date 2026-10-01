//! The `effects` family's rows (F.40 phase 3, E1): one effects
//! fixpoint per snapshot, demanded and counted
//! (`hale_frontend::snapshot::Snapshot::demand_effects`).
//!
//! [`derive_effect_rows`] walks the checked programs with the
//! stdlib's minted analysis copy beside them, as the allocation
//! summary does ([`crate::stdlib_bodies::summarize_with_stdlib_and_renames`]),
//! once, and answers per fn what every consumer used to re-derive for
//! itself: the resolved call targets, the effect set the walk proves,
//! how much of the walk it could see, and the fn's purity.
//!
//! An unresolved edge is coverage, never a violation: the walk keeps
//! two answers apart. [`EffectRow::effects`] is the saturating answer
//! to "what might this do" (`UNCLASSIFIED` once the walk reaches
//! something it cannot name); [`EffectRow::known`] is the lower bound,
//! "what does this definitely do", which an unresolved edge never
//! erases — a fn that publishes and makes an indirect call still
//! publishes (GH #476 Change 5f review).

use std::collections::{BTreeMap, BTreeSet};

use hale_syntax::ast::Program;

use crate::alloc_summary::{AllocSummary, Callee, EffectSiteKind, FnKey};
use crate::purity::Purity;
use crate::resolve::TopScope;
use crate::stdlib_surface::EffectSet;
use crate::symbol::Bundle;

/// The effect rows of one snapshot.
pub struct EffectRows {
    /// One row per fn: every fn the summary holds (the checked
    /// programs' and the stdlib analysis copy's), and every fn the
    /// purity walk keys that the summary does not.
    pub rows: BTreeMap<FnKey, EffectRow>,
    /// The summary the walk read: the checked programs and the stdlib
    /// analysis copy, cross-seed calls resolved through the bundle's
    /// import renames.
    pub summary: AllocSummary,
    /// The program's FFI fn names: a resolved call to one is a syscall.
    pub ffi: BTreeSet<String>,
    /// The load's one user effect-class table, the one every analysis
    /// reads: a class's name by `User(i)` index, and a composed class's
    /// expansion.
    pub classes: crate::effect_classes::EffectClassTable,
}

/// One fn's effects.
#[derive(Debug, Clone, PartialEq)]
pub struct EffectRow {
    /// The fn by name: (locus, fn). The row's identity goes with the
    /// `snapshot_identity` family's declaration rows; until then the
    /// name is the key.
    pub key: FnKey,
    /// Whether the summary holds a body for it. A row without one is
    /// known only to the purity walk: it has no targets and no effects.
    pub summarized: bool,
    /// The resolved call targets, in body order, each once.
    pub targets: Vec<FnKey>,
    /// The callees the summary could not resolve, as written, each
    /// once: a stdlib leaf, an indirect call through a parameter, a
    /// method of a receiver it could not type.
    pub unresolved: Vec<String>,
    /// The saturating answer: everything the fn may do, transitively;
    /// `UNCLASSIFIED` when the walk reached what it cannot name.
    pub effects: EffectSet,
    /// The lower bound: the classes the walk proves, transitively,
    /// whatever else it reached.
    pub known: EffectSet,
    /// Whether the walk reached an edge it cannot name (an indirect
    /// call, an untypeable receiver, an unclassified leaf, the step
    /// bound): `known` is then a lower bound, not the whole set.
    pub unknown: bool,
    /// The fn's own contribution, no recursion: its declared classes,
    /// its allocation and publish sites, its stdlib leaves and FFI
    /// calls.
    pub direct: EffectSet,
    /// Whether the body observably mutates or reaches what does
    /// (F.36); `None` for a fn the purity walk does not key (a
    /// lifecycle hook, a module-nested fn, a stdlib body).
    pub purity: Option<Purity>,
}

impl EffectRows {
    pub fn row(&self, key: &FnKey) -> Option<&EffectRow> {
        self.rows.get(key)
    }

    /// The fn's purity, when the purity walk keys it.
    pub fn purity(&self, key: &FnKey) -> Option<&Purity> {
        self.rows.get(key).and_then(|r| r.purity.as_ref())
    }

    /// The fn's own contribution, for any key: its row's `direct` when
    /// the summary holds its body, else only what it declares it
    /// carries (a fn with no body has no sites and no calls). The
    /// reachability judgment's `effects(C)` destination test and the
    /// model's absorbed paths read it.
    pub fn direct(&self, key: &FnKey) -> EffectSet {
        match self.rows.get(key) {
            Some(r) if r.summarized => r.direct,
            _ => self.summary.carries.get(key).copied().unwrap_or(EffectSet::PURE),
        }
    }
}

/// A fn's DIRECT effect contribution — its own body only, no
/// recursion (a walk over the rows supplies transitivity). Mirrors
/// the per-node fold of [`crate::frontier::infer_effect_bounds`].
fn direct_effects(summary: &AllocSummary, key: &FnKey, ffi: &BTreeSet<String>) -> EffectSet {
    let mut acc = EffectSet::PURE;
    if let Some(c) = summary.carries.get(key) {
        acc = acc.union(*c);
    }
    let Some(fs) = summary.fns.get(key) else { return acc };
    if !fs.sites.is_empty() {
        acc = acc.union(EffectSet::ALLOC);
    }
    for site in &fs.effect_sites {
        acc = acc.union(match site.kind {
            EffectSiteKind::Publish(_) => EffectSet::PUBLISH,
            EffectSiteKind::Spawn(_) => EffectSet::ALLOC,
        });
    }
    for edge in &fs.calls {
        match &edge.callee {
            Callee::Resolved(k) => {
                // The callee's own contribution is its own row's; a
                // leaf's declared `is:` is on the callee's `carries`
                // entry.
                if k.locus.is_none() && ffi.contains(&k.fn_name) {
                    acc = acc.union(EffectSet::SYSCALL);
                }
            }
            Callee::Unresolved(name) => {
                let segs: Vec<&str> = name.split("::").collect();
                if let Some(e) = crate::stdlib_surface::effects_for(&segs) {
                    if !e.is_unclassified() {
                        acc = acc.union(e);
                    }
                }
            }
        }
    }
    acc
}

/// The `effects` family's producer: one walk per fn of the summary,
/// over the bundle's checked programs and the stdlib analysis copy.
pub fn derive_effect_rows(bundle: &Bundle<'_>, top: &TopScope) -> EffectRows {
    let programs: Vec<&Program> = bundle.programs.values().copied().collect();
    let summary = crate::stdlib_bodies::summarize_with_stdlib_and_renames(
        &programs,
        &bundle.snapshot,
        &bundle.import_renames,
    );
    let ffi = crate::effects::ffi_names(&programs);
    let classes = crate::effect_classes::EffectClassTable::of(&programs);
    let mut rows: BTreeMap<FnKey, EffectRow> = BTreeMap::new();
    for (key, fs) in &summary.fns {
        let mut targets: Vec<FnKey> = Vec::new();
        let mut unresolved: Vec<String> = Vec::new();
        for edge in &fs.calls {
            match &edge.callee {
                Callee::Resolved(k) => {
                    if !targets.contains(k) {
                        targets.push(k.clone());
                    }
                }
                Callee::Unresolved(name) => {
                    if !unresolved.contains(name) {
                        unresolved.push(name.clone());
                    }
                }
            }
        }
        // One walk, two results: the saturating set and the lower bound.
        let bounds = crate::frontier::infer_effect_bounds(&summary, key, &ffi);
        rows.insert(
            key.clone(),
            EffectRow {
                key: key.clone(),
                summarized: true,
                targets,
                unresolved,
                effects: bounds.effects,
                known: bounds.known,
                unknown: bounds.unknown,
                direct: direct_effects(&summary, key, &ffi),
                purity: None,
            },
        );
    }
    for (key, purity) in crate::purity::infer_purity_for_bundle(&programs, top) {
        rows.entry(key.clone())
            .or_insert_with(|| EffectRow {
                key,
                summarized: false,
                targets: Vec::new(),
                unresolved: Vec::new(),
                effects: EffectSet::PURE,
                known: EffectSet::PURE,
                unknown: false,
                direct: EffectSet::PURE,
                purity: None,
            })
            .purity = Some(purity);
    }
    EffectRows { rows, summary, ffi, classes }
}

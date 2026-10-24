//! The `effects` family's rows (F.40 phase 3, E1): one effects
//! fixpoint per snapshot, demanded and counted
//! (`hale_frontend::snapshot::Snapshot::demand_effects`).
//!
//! [`derive_effect_rows`] walks the checked programs with the
//! stdlib's minted analysis copy beside them, as the allocation
//! summary does ([`crate::alloc_summary::derive_alloc_summary`]),
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
use std::sync::Arc;

use hale_syntax::ast::Program;

use crate::alloc_summary::{AllocSummary, Callee, EffectSiteKind, FnKey};
use crate::purity::Purity;
use crate::resolve::TopScope;
use crate::stdlib_surface::EffectSet;
use crate::symbol::Bundle;

/// The effect rows of one snapshot.
pub struct EffectRows {
    /// One row per executable body in the shared summary, including
    /// the checked programs and the stdlib analysis copy.
    pub rows: BTreeMap<FnKey, EffectRow>,
    /// The shared contextual execution summary, extending base allocation
    /// facts with caller-sensitive parent protocol work. Certificates and
    /// model consumers read these same source rows.
    pub summary: Arc<AllocSummary>,
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
    /// The fn: its declaration's identity, the (locus, fn) pair its
    /// display name.
    pub key: FnKey,
    /// Whether the summary holds an executable body for this row.
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
    /// Whether shared execution rows establish purity, known impurity,
    /// or incomplete execution/confinement coverage (F.36).
    pub purity: Option<Purity>,
}

impl EffectRows {
    pub fn row(&self, key: &FnKey) -> Option<&EffectRow> {
        self.rows.get(key)
    }

    /// The body's purity from the shared execution account.
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
            // Unknown implicit work contributes no *known* direct class.
            EffectSiteKind::Recovery(_) | EffectSiteKind::AutomaticClosure { .. } | EffectSiteKind::BirthCheckInvocation { .. } | EffectSiteKind::LifecycleInvocation { .. } | EffectSiteKind::ProtocolInvocation { .. } => EffectSet::PURE,
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
/// The summary is the `alloc_summary` family's
/// ([`crate::alloc_summary::derive_alloc_summary`]), shared, not
/// rebuilt.
pub fn derive_effect_rows(
    bundle: &Bundle<'_>,
    _top: &TopScope,
    summary: Arc<AllocSummary>,
) -> EffectRows {
    let programs: Vec<&Program> = bundle.programs.values().copied().collect();
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
        for edge in fs.effect_evaluations() {
            if !targets.contains(&edge.target) { targets.push(edge.target.clone()); }
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
    for (key, purity) in crate::purity::infer_from_summary(&summary, &ffi) {
        rows.get_mut(&key).expect("purity uses the shared summary keys").purity = Some(purity);
    }
    EffectRows { rows, summary, ffi, classes }
}

//! The plan as the emitters read it (F.40 phase 3, L4).
//!
//! Emission discharges a plan's obligations spine by spine: the
//! instantiation, the pinned thread's function, the posted run, the
//! teardown spines. [`LifecyclePlan::spine`] hands an emitter one
//! spine's obligations for one instance template, in the order the plan
//! places them; [`LifecyclePlan::birth_spine`] the template's birth
//! spine, params settle to run start, whichever spine holds each step;
//! and [`LifecyclePlan::birth_order`] the order a declaration's
//! templates place its birth kinds in, which is what an emitter lowering
//! one literal of the declaration reads.
//!
//! **The order.** An obligation comes after every obligation its entry
//! edges reach, through any row of the plan (a completion edge orders a
//! completion, not an entry, and is not read); two the edges leave
//! unordered come in the producer's order, which is the order the
//! instance's domain performs them in ([`super::derive`]). Only the
//! path no failure takes is read ([`PathGuard::Normal`]): a failure's
//! rows are the settle's, the queue drain's or the restart's.
//!
//! **The law.** The steps an emitter emits for a spine are exactly the
//! plan's ordered obligations for it: the trace build (L2) records each
//! step with its spine, and `lifecycle_spines.rs` holds the sequence a
//! run of every fixture program shows, per instance and spine, to
//! [`LifecyclePlan::spine`]'s.

use std::collections::{BTreeMap, BTreeSet};

use super::{LifecyclePlan, ObligationId, ObligationKind, PathGuard, SourceSite, Spine};

/// One obligation a spine discharges for a template.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpineStep {
    pub obligation: ObligationId,
    pub kind: ObligationKind,
}

/// The kinds of the birth spine, the instantiation's obligations from
/// params settle to run start, in the order the producer derives every
/// instance's rows ([`super::derive`]): each template's birth spine is
/// a subsequence of it (`lifecycle_plan.rs` holds that over the corpus).
pub const BIRTH_KINDS: &[ObligationKind] = &[
    ObligationKind::ParamsSettle,
    ObligationKind::Accept,
    ObligationKind::Subscribe,
    ObligationKind::Birth,
    ObligationKind::Readiness,
    ObligationKind::RunAdmission,
    ObligationKind::Run,
];

impl LifecyclePlan {
    /// The source sites of a declaration's instance templates, by its
    /// lowered name, in the order the producer visited them.
    pub fn templates<'a>(&'a self, lowered: &'a str) -> impl Iterator<Item = &'a SourceSite> + 'a {
        self.instances.iter().map(|i| &i.site).filter(move |s| s.decl.lowered == lowered)
    }

    /// The obligations `site` owes on `spine` on the path no failure
    /// takes, in the order the plan places them.
    pub fn spine(&self, site: &SourceSite, spine: Spine) -> Vec<SpineStep> {
        self.ordered(|o| o.holder.spine == spine && o.site.as_ref() == Some(site))
    }

    /// The template's birth spine: its rows of [`BIRTH_KINDS`] on the path
    /// no failure takes, whichever spine holds each (the instantiation,
    /// a pinned locus's thread, the posted run), in the plan's order.
    pub fn birth_spine(&self, site: &SourceSite) -> Vec<SpineStep> {
        self.ordered(|o| BIRTH_KINDS.contains(&o.kind) && o.site.as_ref() == Some(site))
    }

    /// The order the plan places `kinds` in on the birth spine of the
    /// declaration lowered as `lowered`: read from its templates; for
    /// two kinds none of them owes together (a declaration only the
    /// stdlib or an imported seed builds has none in the plan), from
    /// every template of the plan; and for two no template of the plan
    /// owes together, in the producer's order ([`BIRTH_KINDS`]). An error
    /// names two kinds the templates order both ways.
    pub fn birth_order(&self, lowered: &str, kinds: &[ObligationKind]) -> Result<Vec<ObligationKind>, String> {
        let pairs = |sites: &mut dyn Iterator<Item = &SourceSite>| -> BTreeSet<(ObligationKind, ObligationKind)> {
            let mut out = BTreeSet::new();
            for site in sites {
                let seq: Vec<ObligationKind> = self.birth_spine(site).iter().map(|s| s.kind).collect();
                for (i, a) in seq.iter().enumerate() {
                    for b in &seq[i + 1..] {
                        out.insert((*a, *b));
                    }
                }
            }
            out
        };
        let own = pairs(&mut self.templates(lowered));
        let every = pairs(&mut self.instances.iter().map(|i| &i.site));
        let before = |a: ObligationKind, b: ObligationKind| -> Result<bool, String> {
            for set in [&own, &every] {
                match (set.contains(&(a, b)), set.contains(&(b, a))) {
                    (true, true) => {
                        return Err(format!("{lowered}: the plan orders {} and {} both ways", a.name(), b.name()))
                    }
                    (true, false) => return Ok(true),
                    (false, true) => return Ok(false),
                    (false, false) => {}
                }
            }
            let at = |k: ObligationKind| BIRTH_KINDS.iter().position(|&x| x == k);
            match (at(a), at(b)) {
                (Some(i), Some(j)) => Ok(i < j),
                _ => Err(format!("{lowered}: no template of the plan orders {} and {}", a.name(), b.name())),
            }
        };
        let mut out: Vec<ObligationKind> = Vec::new();
        let mut rest: Vec<ObligationKind> = kinds.to_vec();
        rest.dedup();
        while !rest.is_empty() {
            // The kind nothing left precedes.
            let mut next = None;
            for (i, &k) in rest.iter().enumerate() {
                let mut first = true;
                for &j in &rest {
                    if j != k && before(j, k)? {
                        first = false;
                        break;
                    }
                }
                if first {
                    next = Some(i);
                    break;
                }
            }
            let i = next.ok_or_else(|| format!("{lowered}: the birth kinds {kinds:?} have no order"))?;
            out.push(rest.remove(i));
        }
        Ok(out)
    }

    /// The rows `keep` selects on the path no failure takes, each after
    /// every row its entry edges reach, ties in the producer's order.
    fn ordered(&self, keep: impl Fn(&super::Obligation) -> bool) -> Vec<SpineStep> {
        let chosen: Vec<ObligationId> = self
            .iter()
            .filter(|(_, o)| o.guard == PathGuard::Normal && o.source.is_none() && keep(o))
            .map(|(id, _)| id)
            .collect();
        // What each chosen row's entry edges reach, transitively.
        let mut reach: BTreeMap<ObligationId, BTreeSet<ObligationId>> = BTreeMap::new();
        for &id in &chosen {
            let mut seen = BTreeSet::new();
            let mut stack = vec![id];
            while let Some(at) = stack.pop() {
                let Some(o) = self.get(at) else { continue };
                for p in &o.edges.entry {
                    if seen.insert(p.event.obligation) {
                        stack.push(p.event.obligation);
                    }
                }
            }
            reach.insert(id, seen);
        }
        let mut out = Vec::new();
        let mut left: Vec<ObligationId> = chosen;
        while !left.is_empty() {
            // The earliest row no row left precedes; the edges are
            // acyclic (the plan's law), so one always exists.
            let i = left
                .iter()
                .position(|id| !left.iter().any(|other| other != id && reach[id].contains(other)))
                .unwrap_or(0);
            let id = left.remove(i);
            out.push(SpineStep { obligation: id, kind: self.obligations[id.0 as usize].kind });
        }
        out
    }
}

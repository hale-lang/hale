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
//! one literal of the declaration reads. [`LifecyclePlan::reclaim_order`]
//! is the same for the steps inside one instance's reclaim
//! ([`ReclaimStep`]), which the plan states as one `Reclaim` row with
//! the edges and retentions around it.
//!
//! **The order.** An obligation comes after every obligation its entry
//! edges reach, through any row of the plan (a completion edge orders a
//! completion, not an entry, and is not read); two the edges leave
//! unordered come in the producer's order, which is the order the
//! instance's domain performs them in ([`super::derive`]). Only the
//! path no failure takes is read ([`PathGuard::Normal`]): a failure's
//! rows are the settle's, the queue drain's or the restart's.
//! [`LifecyclePlan::shutdown_spine`] reads the same rows with the ones a
//! shutdown adds ([`PathGuard::DrainInFlight`]: a queued run's
//! cancellation inside its child's reclaim, a parked run abandoned at
//! the pool join), so a run that took that path is compared too.
//!
//! **The law.** The steps an emitter emits for a spine are exactly the
//! plan's ordered obligations for it: the trace build (L2) records each
//! step with its spine, and `lifecycle_spines.rs` holds the sequence a
//! run of every fixture program shows, per instance and spine, to
//! [`LifecyclePlan::spine`]'s.

use std::collections::{BTreeMap, BTreeSet};

use super::{Event, LifecyclePlan, ObligationId, ObligationKind, PathGuard, Point, Resource, SourceSite, Spine};

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

/// A step inside one instance's reclaim. The plan states the reclaim as
/// one row ([`ObligationKind::Reclaim`], exactly once per instance) and
/// orders what happens around and inside it with edges and retentions:
/// the owned children's reclaims complete before it is entered (line
/// 14); a run still queued for the instance is canceled after it is
/// entered (line 19's cancellation); the run holds the instance until it
/// ends, and the reclaim completes only once it has (line 19's
/// retention). [`LifecyclePlan::reclaim_order`] reads those into the
/// order of these steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ReclaimStep {
    /// The owned children's reclaims (the accepted children's; a
    /// field's is its cascade's, before its owner's).
    Children,
    /// The latch: the reclaim entered, once per instance (`__arena`).
    Latch,
    /// The runs still queued for the instance, on any pool, canceled.
    CancelQueuedRuns,
    /// The instance's arena released, with what it holds (its bus
    /// registrations, its children's buffer, its capacity slots).
    ReleaseArena,
    /// The instance's struct released to its owner.
    ReleaseStruct,
}

/// The producer's order of the reclaim's steps, for a pair no row of the
/// plan orders.
pub const RECLAIM_STEPS: &[ReclaimStep] = &[
    ReclaimStep::Children,
    ReclaimStep::Latch,
    ReclaimStep::CancelQueuedRuns,
    ReclaimStep::ReleaseArena,
    ReclaimStep::ReleaseStruct,
];

impl ReclaimStep {
    pub fn name(self) -> &'static str {
        match self {
            ReclaimStep::Children => "Children",
            ReclaimStep::Latch => "Latch",
            ReclaimStep::CancelQueuedRuns => "CancelQueuedRuns",
            ReclaimStep::ReleaseArena => "ReleaseArena",
            ReclaimStep::ReleaseStruct => "ReleaseStruct",
        }
    }
}

impl LifecyclePlan {
    /// The source sites of a declaration's instance templates, by its
    /// lowered name, in the order the producer visited them.
    pub fn templates<'a>(&'a self, lowered: &'a str) -> impl Iterator<Item = &'a SourceSite> + 'a {
        self.instances.iter().map(|i| &i.site).filter(move |s| s.decl.lowered == lowered)
    }

    /// The obligations `site` owes on `spine` on the path no failure
    /// takes, in the order the plan places them.
    pub fn spine(&self, site: &SourceSite, spine: Spine) -> Vec<SpineStep> {
        self.ordered(&[PathGuard::Normal], |o| o.holder.spine == spine && o.site.as_ref() == Some(site))
    }

    /// The obligations `site` owes on `spine` on the path a shutdown
    /// takes: [`LifecyclePlan::spine`]'s, with the rows only that path
    /// owes (a run still queued when its child is reclaimed, a parked run
    /// the pool join abandons), in the order the plan places them. Which
    /// of the two paths a run takes is the scheduler's.
    pub fn shutdown_spine(&self, site: &SourceSite, spine: Spine) -> Vec<SpineStep> {
        self.ordered(&[PathGuard::Normal, PathGuard::DrainInFlight], |o| {
            o.holder.spine == spine && o.site.as_ref() == Some(site)
        })
    }

    /// The template's birth spine: its rows of [`BIRTH_KINDS`] on the path
    /// no failure takes, whichever spine holds each (the instantiation,
    /// a pinned locus's thread, the posted run), in the plan's order.
    pub fn birth_spine(&self, site: &SourceSite) -> Vec<SpineStep> {
        self.ordered(&[PathGuard::Normal], |o| BIRTH_KINDS.contains(&o.kind) && o.site.as_ref() == Some(site))
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
        order_by(lowered, kinds, &own, &every, BIRTH_KINDS, |k| k.name())
    }

    /// The order the plan places the steps of a reclaim of the
    /// declaration lowered as `lowered` in ([`ReclaimStep`]): read from its
    /// templates' rows; for two steps none of them orders (a declaration
    /// whose run is never queued behind its own reclaim has no
    /// cancellation row), from every template of the plan; and for two no
    /// template orders, in the producer's order ([`RECLAIM_STEPS`]). An
    /// error names two steps the templates order both ways.
    pub fn reclaim_order(&self, lowered: &str) -> Result<Vec<ReclaimStep>, String> {
        let pairs = |sites: &mut dyn Iterator<Item = &SourceSite>| -> BTreeSet<(ReclaimStep, ReclaimStep)> {
            let mut out = BTreeSet::new();
            for site in sites {
                out.extend(self.reclaim_pairs(site));
            }
            out
        };
        let own = pairs(&mut self.templates(lowered));
        let every = pairs(&mut self.instances.iter().map(|i| &i.site));
        order_by(lowered, RECLAIM_STEPS, &own, &every, RECLAIM_STEPS, |s| s.name())
    }

    /// The pairs of reclaim steps one template's rows order (the rest
    /// [`LifecyclePlan::reclaim_order`] takes from other templates or the
    /// producer's order).
    pub fn reclaim_pairs(&self, site: &SourceSite) -> BTreeSet<(ReclaimStep, ReclaimStep)> {
        use ReclaimStep as R;
        let mut out = BTreeSet::new();
        let mine = |kind: ObligationKind| {
            self.iter()
                .filter(move |(_, o)| o.kind == kind && o.site.as_ref() == Some(site) && o.source.is_none())
                .filter(|(_, o)| matches!(o.guard, PathGuard::Normal | PathGuard::DrainInFlight))
        };
        let Some((reclaim, row)) = mine(ObligationKind::Reclaim).next() else { return out };
        // Line 14: an owned child's reclaim completes before this one is
        // entered.
        let child_first = row.edges.entry.iter().any(|p| {
            p.event.point.satisfies(Point::Completed)
                && self.get(p.event.obligation).is_some_and(|o| o.kind == ObligationKind::Reclaim && o.site.as_ref() != Some(site))
        });
        if child_first {
            out.insert((R::Children, R::Latch));
        }
        // Line 14: the reclaim is the arena's release, exactly once: the
        // arena is retained until the reclaim completes, and what the
        // reclaim releases is released past its entry (the latch).
        let arena_held = row
            .lifetime
            .iter()
            .any(|t| t.resource == Resource::Arena && t.until == Event { obligation: reclaim, point: Point::Completed });
        if arena_held {
            out.insert((R::Latch, R::ReleaseArena));
            out.insert((R::Latch, R::ReleaseStruct));
        }
        // Line 19: a queued run's cancellation is entered once the reclaim
        // is; the run holds the instance until it ends, and the reclaim
        // completes only once the run has ended and the cancellation
        // completed, so nothing of the instance is released before it.
        let entered = Event { obligation: reclaim, point: Point::Entered };
        for (cancel, c) in mine(ObligationKind::Cancellation) {
            if !c.edges.entry.iter().any(|p| p.event == entered) {
                continue;
            }
            out.insert((R::Latch, R::CancelQueuedRuns));
            let completes_after = |id: ObligationId| row.edges.completion.iter().any(|p| p.event.obligation == id);
            let held_run = mine(ObligationKind::Run).any(|(run, r)| {
                completes_after(run)
                    && r.lifetime.iter().any(|t| t.resource == Resource::Instance && t.until.obligation == run)
            });
            if held_run && completes_after(cancel) {
                out.insert((R::CancelQueuedRuns, R::ReleaseArena));
                out.insert((R::CancelQueuedRuns, R::ReleaseStruct));
            }
        }
        out
    }

}

/// `items` in the order `own`'s pairs place them, then `every`'s for a
/// pair `own` leaves unordered, then `fallback`'s: the item nothing left
/// precedes, repeatedly.
fn order_by<T: Copy + Ord + std::fmt::Debug>(
    lowered: &str,
    items: &[T],
    own: &BTreeSet<(T, T)>,
    every: &BTreeSet<(T, T)>,
    fallback: &[T],
    name: impl Fn(T) -> &'static str,
) -> Result<Vec<T>, String> {
    let before = |a: T, b: T| -> Result<bool, String> {
        for set in [own, every] {
            match (set.contains(&(a, b)), set.contains(&(b, a))) {
                (true, true) => return Err(format!("{lowered}: the plan orders {} and {} both ways", name(a), name(b))),
                (true, false) => return Ok(true),
                (false, true) => return Ok(false),
                (false, false) => {}
            }
        }
        let at = |k: T| fallback.iter().position(|&x| x == k);
        match (at(a), at(b)) {
            (Some(i), Some(j)) => Ok(i < j),
            _ => Err(format!("{lowered}: no template of the plan orders {} and {}", name(a), name(b))),
        }
    };
    let mut out: Vec<T> = Vec::new();
    let mut rest: Vec<T> = items.to_vec();
    rest.dedup();
    while !rest.is_empty() {
        // The item nothing left precedes.
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
        let i = next.ok_or_else(|| format!("{lowered}: {items:?} have no order"))?;
        out.push(rest.remove(i));
    }
    Ok(out)
}

impl LifecyclePlan {
    /// The rows `keep` selects on the paths `guards` names (no failure
    /// on any), each after every row its entry edges reach, ties in the
    /// producer's order.
    fn ordered(&self, guards: &[PathGuard], keep: impl Fn(&super::Obligation) -> bool) -> Vec<SpineStep> {
        let chosen: Vec<ObligationId> = self
            .iter()
            .filter(|(_, o)| guards.contains(&o.guard) && o.source.is_none() && keep(o))
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

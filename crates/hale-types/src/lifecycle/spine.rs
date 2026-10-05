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
//! [`LifecyclePlan::recovery_spine`] reads a failure's recovery rows (the
//! resume and the restart, [`RECOVERY_KINDS`]) on the path one failure
//! takes, held or not, its restart performed or not.
//! [`LifecyclePlan::recovery_order`] is what an emitter of a restart
//! reads: the decision, the restart, and the incarnation it begins
//! ([`RecoveryStep`]). The lowering reads those orders from a
//! [`SpineIndex`], which computes each once per plan.
//!
//! **The law.** The steps an emitter emits for a spine are exactly the
//! plan's ordered obligations for it: the trace build (L2) records each
//! step with its spine, and `lifecycle_spines.rs` holds the sequence a
//! run of every fixture program shows, per instance and spine, to
//! [`LifecyclePlan::spine`]'s.

use std::collections::{BTreeMap, BTreeSet};

use super::{
    Event, FailureSource, LifecyclePlan, ObligationId, ObligationKind, PathGuard, Point, Resource, SourceSite, Spine,
};

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

/// The kinds a failure's recovery is emitted as, on whichever spine the
/// decision is carried out: the resume of a held failure at its owner's
/// settle, and the restart ([`LifecyclePlan::recovery_spine`]).
pub const RECOVERY_KINDS: &[ObligationKind] = &[ObligationKind::Resume, ObligationKind::Restart];

/// A step inside one instance's reclaim. The plan states the reclaim as
/// one row ([`ObligationKind::Reclaim`], exactly once per instance) and
/// orders what happens around and inside it with edges and retentions:
/// the owned children's reclaims complete before its storage is released
/// (line 14); a run still queued for the instance is canceled after it is
/// entered (line 19's cancellation); the run holds the instance until it
/// ends, and the reclaim completes only once it has (line 19's
/// retention). [`LifecyclePlan::reclaim_order`] reads those into the
/// order of these steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ReclaimStep {
    /// The owned children's reclaims (the accepted children's; a
    /// field's is its cascade's, before its owner's).
    Children,
    /// The guarded logical reclaim entry; the instance claim precedes it.
    Latch,
    /// The runs still queued for the instance, on any pool, canceled.
    CancelQueuedRuns,
    /// All admitted runs have ended and deferred descendants are released.
    /// The runtime may retain this continuation until their holds clear.
    WaitForRuns,
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
    ReclaimStep::WaitForRuns,
    ReclaimStep::ReleaseArena,
    ReclaimStep::ReleaseStruct,
];

/// A step of an owner's teardown, as the dissolve cascade over its
/// instance tree places its fields' around its own: the fields' drains
/// before its drain (line 12), its dissolve after its drain, its fields'
/// dissolves after its dissolve (line 10), its reclaim after theirs (line
/// 14). [`LifecyclePlan::cascade_order`] reads it from the edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CascadeStep {
    /// The owned fields' drains, each field's own fields first.
    FieldDrains,
    /// The owner's `drain()`.
    Drain,
    /// The owner's dissolve-epoch closures and `dissolve()`.
    Dissolve,
    /// The owned fields' dissolves, each with its own fields' and its
    /// reclaim.
    FieldDissolves,
    /// The owner's reclaim.
    Reclaim,
}

/// The producer's order of an owner's cascade steps, for a pair no row
/// orders.
pub const CASCADE_STEPS: &[CascadeStep] = &[
    CascadeStep::FieldDrains,
    CascadeStep::Drain,
    CascadeStep::Dissolve,
    CascadeStep::FieldDissolves,
    CascadeStep::Reclaim,
];

/// A step of a main-locus entry's teardown, as a teardown spine places
/// its process head around the pinned threads it joins and its cascade:
/// the head first (line 7: a wait is aborted before the join it would
/// block), then the joins, then the cascade.
/// [`LifecyclePlan::entry_order`] reads it from the edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EntryStep {
    /// The spine's process head: the ingress quiesce, the wait-abort, the
    /// pool join.
    Head,
    /// The joins of the pinned threads the entry's teardown joins.
    PinnedJoins,
    /// The entry's dissolve cascade, its fields' drains first.
    Cascade,
}

/// The producer's order of an entry's teardown steps, for a pair no row
/// orders.
pub const ENTRY_STEPS: &[EntryStep] = &[EntryStep::Head, EntryStep::PinnedJoins, EntryStep::Cascade];

impl EntryStep {
    pub fn name(self) -> &'static str {
        match self {
            EntryStep::Head => "Head",
            EntryStep::PinnedJoins => "PinnedJoins",
            EntryStep::Cascade => "Cascade",
        }
    }
}

impl CascadeStep {
    pub fn name(self) -> &'static str {
        match self {
            CascadeStep::FieldDrains => "FieldDrains",
            CascadeStep::Drain => "Drain",
            CascadeStep::Dissolve => "Dissolve",
            CascadeStep::FieldDissolves => "FieldDissolves",
            CascadeStep::Reclaim => "Reclaim",
        }
    }
}

impl ReclaimStep {
    pub fn name(self) -> &'static str {
        match self {
            ReclaimStep::Children => "Children",
            ReclaimStep::Latch => "Latch",
            ReclaimStep::CancelQueuedRuns => "CancelQueuedRuns",
            ReclaimStep::WaitForRuns => "WaitForRuns",
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

    /// The process obligations a teardown spine owes (the ingress
    /// quiesce, the wait-abort, the pool join, the frame's pre-drain:
    /// the rows with no source site), one step per kind, in the order the
    /// plan's edges place them among the spine's rows; two kinds the
    /// edges leave unordered come in the producer's order. Each step names
    /// the first of its kind's rows, whose status is the kind's on the
    /// spine (a known-open row is one the code does not emit). An error
    /// names two kinds the edges order both ways.
    pub fn process_order(&self, spine: Spine) -> Result<Vec<SpineStep>, String> {
        count_pass();
        let rows: Vec<ObligationId> = self
            .iter()
            .filter(|(_, o)| o.site.is_none() && o.holder.spine == spine && o.guard == PathGuard::Normal && o.source.is_none())
            .map(|(id, _)| id)
            .collect();
        let mut kinds: Vec<ObligationKind> = Vec::new();
        let mut first: BTreeMap<ObligationKind, ObligationId> = BTreeMap::new();
        for &id in &rows {
            let k = self.obligations[id.0 as usize].kind;
            if !kinds.contains(&k) {
                kinds.push(k);
                first.insert(k, id);
            }
        }
        // A row's kind after every kind of the spine its entry edges reach.
        let mut pairs: BTreeSet<(ObligationKind, ObligationKind)> = BTreeSet::new();
        for &id in &rows {
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
            let k = self.obligations[id.0 as usize].kind;
            for &b in &rows {
                let kb = self.obligations[b.0 as usize].kind;
                if kb != k && seen.contains(&b) {
                    pairs.insert((kb, k));
                }
            }
        }
        let ordered = order_by(spine.name(), &kinds, &pairs, &BTreeSet::new(), &kinds, |k| k.name())?;
        Ok(ordered.into_iter().map(|kind| SpineStep { obligation: first[&kind], kind }).collect())
    }

    /// The order the plan places a main-locus entry's teardown in on
    /// `spine` ([`EntryStep`]): its head (the spine's process rows before
    /// the frame's pre-drain), the joins of the pinned threads its teardown
    /// joins, and its cascade. A pair is ordered where some row of one
    /// has an entry edge from some row of the other; a pair the
    /// plan leaves unordered (the pinned joins against the cascade, line
    /// 17) comes in the producer's order ([`ENTRY_STEPS`]). An error names
    /// two steps the plan orders both ways.
    pub fn entry_order(&self, spine: Spine) -> Result<Vec<EntryStep>, String> {
        use EntryStep as E;
        count_pass();
        let head: BTreeSet<ObligationId> = self
            .iter()
            .filter(|(_, o)| {
                o.site.is_none()
                    && o.holder.spine == spine
                    && o.guard == PathGuard::Normal
                    && o.source.is_none()
                    && matches!(o.kind, ObligationKind::IngressQuiesce | ObligationKind::WaitAbort | ObligationKind::PoolJoin)
            })
            .map(|(id, _)| id)
            .collect();
        let rows = |step: EntryStep| -> Vec<ObligationId> {
            match step {
                E::Head => head.iter().copied().collect(),
                E::PinnedJoins => self
                    .iter()
                    .filter(|(_, o)| o.kind == ObligationKind::PinnedJoin && o.guard == PathGuard::Normal && o.source.is_none())
                    .map(|(id, _)| id)
                    .collect(),
                E::Cascade => self
                    .iter()
                    .filter(|(_, o)| {
                        o.kind == ObligationKind::Drain
                            && o.holder.spine == Spine::Cascade
                            && o.guard == PathGuard::Normal
                            && o.source.is_none()
                    })
                    .map(|(id, _)| id)
                    .collect(),
            }
        };
        // Whether some row of `b` has an entry edge from some row of `a`.
        // Only a direct edge is read: the head's rows of a spine held by
        // `fn main`'s frame wait, through the exit's pre-drain, for every
        // statement literal's teardown, its pinned join included, which is
        // not a join this entry's teardown performs.
        let reaches = |a: &[ObligationId], b: &[ObligationId]| -> bool {
            b.iter().any(|&id| self.get(id).is_some_and(|o| o.edges.entry.iter().any(|p| a.contains(&p.event.obligation))))
        };
        let mut pairs = BTreeSet::new();
        for (i, &a) in ENTRY_STEPS.iter().enumerate() {
            for &b in &ENTRY_STEPS[i + 1..] {
                let (ra, rb) = (rows(a), rows(b));
                if reaches(&ra, &rb) {
                    pairs.insert((a, b));
                }
                if reaches(&rb, &ra) {
                    pairs.insert((b, a));
                }
            }
        }
        order_by(spine.name(), ENTRY_STEPS, &pairs, &BTreeSet::new(), ENTRY_STEPS, |s| s.name())
    }

    /// The template's birth spine: its rows of [`BIRTH_KINDS`] on the path
    /// no failure takes, whichever spine holds each (the instantiation,
    /// a pinned locus's thread, the posted run), in the plan's order.
    pub fn birth_spine(&self, site: &SourceSite) -> Vec<SpineStep> {
        self.birth_spine_of(&self.site_rows(site))
    }

    /// [`LifecyclePlan::birth_spine`] over `rows`, its site's rows.
    fn birth_spine_of(&self, rows: &[ObligationId]) -> Vec<SpineStep> {
        let chosen = self
            .rows(rows)
            .filter(|(_, o)| o.guard == PathGuard::Normal && o.source.is_none() && BIRTH_KINDS.contains(&o.kind))
            .map(|(id, _)| id)
            .collect();
        self.in_order(chosen)
    }

    /// `site`'s rows in the producer's order: a pass over the plan, which
    /// [`SpineIndex`] makes once for every site.
    fn site_rows(&self, site: &SourceSite) -> Vec<ObligationId> {
        count_pass();
        self.iter().filter(|(_, o)| o.site.as_ref() == Some(site)).map(|(id, _)| id).collect()
    }

    /// `ids` with their rows.
    fn rows<'a>(&'a self, ids: &'a [ObligationId]) -> impl Iterator<Item = (ObligationId, &'a super::Obligation)> + 'a {
        ids.iter().map(move |&id| (id, &self.obligations[id.0 as usize]))
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
        count_pass();
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
        count_pass();
        let every = pairs(&mut self.instances.iter().map(|i| &i.site));
        order_by(lowered, RECLAIM_STEPS, &own, &every, RECLAIM_STEPS, |s| s.name())
    }

    /// The order the plan places an owner's cascade steps in
    /// ([`CascadeStep`]) for the declaration lowered as `lowered`: read
    /// from its templates' edges; for two none of them orders (a
    /// declaration with no field in the plan), from every template; and
    /// for two no template orders, in the producer's order
    /// ([`CASCADE_STEPS`]).
    pub fn cascade_order(&self, lowered: &str) -> Result<Vec<CascadeStep>, String> {
        let pairs = |sites: &mut dyn Iterator<Item = &SourceSite>| -> BTreeSet<(CascadeStep, CascadeStep)> {
            let mut out = BTreeSet::new();
            for site in sites {
                out.extend(self.cascade_pairs(site));
            }
            out
        };
        let own = pairs(&mut self.templates(lowered));
        count_pass();
        let every = pairs(&mut self.instances.iter().map(|i| &i.site));
        order_by(lowered, CASCADE_STEPS, &own, &every, CASCADE_STEPS, |s| s.name())
    }

    /// The pairs of cascade steps one owner template's rows order.
    pub fn cascade_pairs(&self, site: &SourceSite) -> BTreeSet<(CascadeStep, CascadeStep)> {
        // Line 10's test: a field's dissolve waits for the owner's to
        // complete, read by a pass over the plan.
        let fields_after = |dissolve: ObligationId| {
            let after_mine = Event { obligation: dissolve, point: Point::Completed };
            self.obligations.iter().any(|o| {
                o.kind == ObligationKind::Dissolve
                    && o.site.as_ref() != Some(site)
                    && o.edges.entry.iter().any(|p| p.event == after_mine)
            })
        };
        self.cascade_pairs_of(site, &self.site_rows(site), fields_after)
    }

    /// [`LifecyclePlan::cascade_pairs`] over `rows`, `site`'s rows;
    /// `fields_after` answers whether a dissolve row of another site waits
    /// for the owner's dissolve (that id) to complete.
    fn cascade_pairs_of(
        &self,
        site: &SourceSite,
        rows: &[ObligationId],
        fields_after: impl Fn(ObligationId) -> bool,
    ) -> BTreeSet<(CascadeStep, CascadeStep)> {
        use CascadeStep as C;
        let mut out = BTreeSet::new();
        let row = |kind: ObligationKind| {
            self.rows(rows)
                .find(|(_, o)| o.kind == kind && o.guard == PathGuard::Normal && o.source.is_none())
        };
        let (Some((drain, d)), Some((dissolve, ds))) = (row(ObligationKind::Drain), row(ObligationKind::Dissolve)) else {
            return out;
        };
        // A row of one of the owner's children: a static template one
        // field below the owner's, or a literal site (a body's, an
        // accepted child's).
        let other = |id: ObligationId, kind: ObligationKind| {
            self.get(id).is_some_and(|o| {
                o.kind == kind
                    && o.site.as_ref().is_some_and(|s| {
                        s != site
                            && match (&site.template, &s.template) {
                                (super::Template::Static(a), super::Template::Static(b)) => {
                                    b.origin == a.origin && b.path.len() == a.path.len() + 1 && b.path.starts_with(&a.path)
                                }
                                _ => true,
                            }
                    })
            })
        };
        let completed = |p: &super::Prerequisite| p.event.point.satisfies(Point::Completed);
        // Line 12: a field's drain completes before its owner's is entered.
        if d.edges.entry.iter().any(|p| completed(p) && other(p.event.obligation, ObligationKind::Drain)) {
            out.insert((C::FieldDrains, C::Drain));
        }
        if ds.edges.entry.iter().any(|p| completed(p) && p.event.obligation == drain) {
            out.insert((C::Drain, C::Dissolve));
        }
        // Line 10: a field is dissolved once its owner's dissolve completes.
        if fields_after(dissolve) {
            out.insert((C::Dissolve, C::FieldDissolves));
        }
        // Line 14: a field's dissolve completes before its owner enters
        // reclaim. Its retained storage can finish releasing later.
        if let Some((_, r)) = row(ObligationKind::Reclaim) {
            if r.edges.entry.iter().any(|p| completed(p) && other(p.event.obligation, ObligationKind::Dissolve)) {
                out.insert((C::FieldDissolves, C::Reclaim));
            }
        }
        out
    }

    /// The owned fields of the declaration lowered as `lowered`, by name,
    /// in the order the plan's edges chain their drains (line 12: the
    /// owner's declaration order), read from its templates' fields. A
    /// field no edge places (a pinned one, one with several templates) is
    /// not named.
    pub fn cascade_fields(&self, lowered: &str) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        let mut before: BTreeSet<(String, String)> = BTreeSet::new();
        for owner in self.templates(lowered) {
            let super::Template::Static(key) = &owner.template else { continue };
            // The owner's field templates, by field name, and their drains.
            let mut drains: BTreeMap<ObligationId, String> = BTreeMap::new();
            for (id, o) in self.iter() {
                let Some(super::Template::Static(k)) = o.site.as_ref().map(|s| &s.template) else { continue };
                if o.kind == ObligationKind::Drain
                    && o.guard == PathGuard::Normal
                    && o.source.is_none()
                    && k.origin == key.origin
                    && k.path.len() == key.path.len() + 1
                    && k.path.starts_with(&key.path)
                {
                    drains.insert(id, k.path.last().expect("a field step").field.clone());
                }
            }
            for (&id, name) in &drains {
                let o = self.get(id).expect("a row");
                for p in &o.edges.entry {
                    if let Some(prev) = drains.get(&p.event.obligation) {
                        before.insert((prev.clone(), name.clone()));
                        for n in [prev, name] {
                            if !names.contains(n) {
                                names.push(n.clone());
                            }
                        }
                    }
                }
            }
        }
        let mut out: Vec<String> = Vec::new();
        while !names.is_empty() {
            let i = names
                .iter()
                .position(|n| !names.iter().any(|m| m != n && before.contains(&(m.clone(), n.clone()))))
                .unwrap_or(0);
            out.push(names.remove(i));
        }
        out
    }

    /// The pairs of reclaim steps one template's rows order (the rest
    /// [`LifecyclePlan::reclaim_order`] takes from other templates or the
    /// producer's order).
    pub fn reclaim_pairs(&self, site: &SourceSite) -> BTreeSet<(ReclaimStep, ReclaimStep)> {
        self.reclaim_pairs_of(site, &self.site_rows(site))
    }

    /// [`LifecyclePlan::reclaim_pairs`] over `rows`, `site`'s rows.
    fn reclaim_pairs_of(&self, site: &SourceSite, rows: &[ObligationId]) -> BTreeSet<(ReclaimStep, ReclaimStep)> {
        use ReclaimStep as R;
        let mut out = BTreeSet::new();
        let mine = |kind: ObligationKind| {
            self.rows(rows)
                .filter(move |(_, o)| o.kind == kind && o.source.is_none())
                .filter(|(_, o)| matches!(o.guard, PathGuard::Normal | PathGuard::DrainInFlight))
        };
        let Some((reclaim, row)) = mine(ObligationKind::Reclaim).next() else { return out };
        // Line 14: initiate child reclaim before waiting for retained
        // descendants; all child storage is released before this arena.
        let child_first = row.edges.completion.iter().any(|p| {
            p.event.point.satisfies(Point::Completed)
                && self.get(p.event.obligation).is_some_and(|o| o.kind == ObligationKind::Reclaim && o.site.as_ref() != Some(site))
        });
        if child_first {
            out.insert((R::Children, R::WaitForRuns));
            out.insert((R::Children, R::ReleaseArena));
            out.insert((R::Children, R::ReleaseStruct));
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
        let held_run = mine(ObligationKind::Run).any(|(run, r)| {
            row.edges.completion.iter().any(|p| p.event.obligation == run)
                && r.lifetime.iter().any(|t| t.resource == Resource::Instance && t.until.obligation == run)
        });
        if held_run {
            out.insert((R::WaitForRuns, R::ReleaseArena));
            out.insert((R::WaitForRuns, R::ReleaseStruct));
        }
        let entered = Event { obligation: reclaim, point: Point::Entered };
        for (cancel, c) in mine(ObligationKind::Cancellation) {
            if !c.edges.entry.iter().any(|p| p.event == entered) {
                continue;
            }
            out.insert((R::Latch, R::CancelQueuedRuns));
            out.insert((R::CancelQueuedRuns, R::WaitForRuns));
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

/// A step of a restart, as the emitters carry it out wherever the
/// decision is read (the posted run's loop, a pinned locus's thread, the
/// run gate of an instantiation, the resume at settle). The plan states
/// the recovery decision and the restart as rows (line RD) and the
/// incarnation the restart begins as the template's rows owed once per
/// incarnation; [`LifecyclePlan::recovery_order`] reads their order. A
/// restart tears nothing down: it re-runs the same instance (C42).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RecoveryStep {
    /// The decision, read once the handler has returned: a restart asked
    /// for, within its bound, the child not quarantined, the process not
    /// draining; or the resume's at settle (line 13).
    Decision,
    /// The restart's entry: the params as built put back for a
    /// `restart_in_place`, the latch the failure raised lowered. It
    /// begins the next incarnation.
    Restart,
    /// The next incarnation's `birth()`, with its birth-epoch closures.
    Birth,
    /// The next incarnation's `run()`, owed only by a template that
    /// declares one (line 13, C48).
    Run,
}

/// The producer's order of a restart's steps, for a pair no row orders.
pub const RECOVERY_STEPS: &[RecoveryStep] =
    &[RecoveryStep::Decision, RecoveryStep::Restart, RecoveryStep::Birth, RecoveryStep::Run];

impl RecoveryStep {
    pub fn name(self) -> &'static str {
        match self {
            RecoveryStep::Decision => "Decision",
            RecoveryStep::Restart => "Restart",
            RecoveryStep::Birth => "Birth",
            RecoveryStep::Run => "Run",
        }
    }
}

impl LifecyclePlan {
    /// The steps of a restart of the declaration lowered as `lowered`,
    /// in the order the plan places them ([`RecoveryStep`]): read from its
    /// templates' rows; for two none of them orders, from every template
    /// of the plan; and for two no template orders, in the producer's
    /// order ([`RECOVERY_STEPS`]). `Run` is a step where a template owes
    /// its incarnations a `run()`, or where the plan has no template of
    /// the declaration (one nothing builds, whose restart never runs).
    pub fn recovery_order(&self, lowered: &str) -> Result<Vec<RecoveryStep>, String> {
        let pairs = |sites: &mut dyn Iterator<Item = &SourceSite>| -> BTreeSet<(RecoveryStep, RecoveryStep)> {
            let mut out = BTreeSet::new();
            for site in sites {
                out.extend(self.recovery_pairs(site));
            }
            out
        };
        let own = pairs(&mut self.templates(lowered));
        count_pass();
        let every = pairs(&mut self.instances.iter().map(|i| &i.site));
        let mut templates = self.templates(lowered).peekable();
        let run = templates.peek().is_none()
            || templates.any(|s| self.incarnation_row(&self.site_rows(s), ObligationKind::Run).is_some());
        order_by(lowered, &recovery_steps(run), &own, &every, RECOVERY_STEPS, |s| s.name())
    }

    /// The pairs of restart steps one template's rows order: the restart
    /// after the decision completes (line RD); the restart's next
    /// incarnation after it, its birth first; its run after its birth.
    pub fn recovery_pairs(&self, site: &SourceSite) -> BTreeSet<(RecoveryStep, RecoveryStep)> {
        self.recovery_pairs_of(&self.site_rows(site))
    }

    /// [`LifecyclePlan::recovery_pairs`] over `rows`, its site's rows.
    fn recovery_pairs_of(&self, rows: &[ObligationId]) -> BTreeSet<(RecoveryStep, RecoveryStep)> {
        use RecoveryStep as R;
        let mut out = BTreeSet::new();
        let restarts: Vec<&super::Obligation> = self
            .rows(rows)
            .filter(|(_, o)| o.kind == ObligationKind::Restart && o.guard == PathGuard::Restart)
            .map(|(_, o)| o)
            .collect();
        let decided = |o: &super::Obligation| {
            o.edges.entry.iter().any(|p| {
                p.event.point.satisfies(Point::Completed)
                    && self.get(p.event.obligation).is_some_and(|d| d.kind == ObligationKind::RecoveryDecision)
            })
        };
        if restarts.iter().any(|o| decided(o)) {
            out.insert((R::Decision, R::Restart));
        }
        let Some((birth, _)) = self.incarnation_row(rows, ObligationKind::Birth) else { return out };
        // A restart begins the next incarnation, which owes again every
        // row owed once per incarnation.
        if !restarts.is_empty() {
            out.insert((R::Restart, R::Birth));
        }
        if let Some((_, run)) = self.incarnation_row(rows, ObligationKind::Run) {
            if run.edges.entry.iter().any(|p| p.event == Event { obligation: birth, point: Point::Completed }) {
                out.insert((R::Birth, R::Run));
            }
        }
        out
    }

    /// The row of `kind` among a site's `rows` owed once per incarnation,
    /// on the path no failure takes.
    fn incarnation_row<'a>(&'a self, rows: &'a [ObligationId], kind: ObligationKind) -> Option<(ObligationId, &'a super::Obligation)> {
        self.rows(rows).find(|(_, o)| {
            o.kind == kind
                && o.guard == PathGuard::Normal
                && o.source.is_none()
                && o.multiplicity == super::Multiplicity::OncePerIncarnation
        })
    }

    /// The recovery steps ([`RECOVERY_KINDS`]) `site` owes on `spine`
    /// after one failure of `source`: on the path where the failure was
    /// held at its owner's settle (`held`) or delivered in place, and its
    /// restart performed (`performed`) or not. A restart refused under
    /// teardown owes no step: its every terminal is a not-started one.
    /// In the plan's order, once each; how many restarts a run performs
    /// is the run's.
    pub fn recovery_spine(
        &self,
        site: &SourceSite,
        spine: Spine,
        source: FailureSource,
        held: bool,
        performed: bool,
    ) -> Vec<SpineStep> {
        // The path a restart row is on is its decision's: the decision
        // after the held delivery, or after the one in place.
        let decided_held = |o: &super::Obligation| {
            o.edges.entry.iter().any(|p| {
                self.get(p.event.obligation).is_some_and(|d| {
                    d.kind == ObligationKind::RecoveryDecision && (d.guard == PathGuard::FailedAtSettle) == held
                })
            })
        };
        let chosen: Vec<ObligationId> = self
            .iter()
            .filter(|(_, o)| {
                RECOVERY_KINDS.contains(&o.kind)
                    && o.holder.spine == spine
                    && o.source == Some(source)
                    && o.site.as_ref() == Some(site)
                    && match (o.kind, o.guard) {
                        (ObligationKind::Resume, PathGuard::FailedAtSettle) => held,
                        (ObligationKind::Restart, PathGuard::Restart) => performed && decided_held(o),
                        _ => false,
                    }
            })
            .map(|(id, _)| id)
            .collect();
        self.in_order(chosen)
    }

    /// The rows `keep` selects on the paths `guards` names (no failure
    /// on any), each after every row its entry edges reach, ties in the
    /// producer's order.
    fn ordered(&self, guards: &[PathGuard], keep: impl Fn(&super::Obligation) -> bool) -> Vec<SpineStep> {
        let chosen: Vec<ObligationId> = self
            .iter()
            .filter(|(_, o)| guards.contains(&o.guard) && o.source.is_none() && keep(o))
            .map(|(id, _)| id)
            .collect();
        self.in_order(chosen)
    }

    /// `chosen`, each after every row its entry edges reach, ties in the
    /// producer's order.
    fn in_order(&self, chosen: Vec<ObligationId>) -> Vec<SpineStep> {
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

/// A restart's steps: `Run` among them where `run`.
fn recovery_steps(run: bool) -> Vec<RecoveryStep> {
    let mut steps = vec![RecoveryStep::Decision, RecoveryStep::Restart, RecoveryStep::Birth];
    if run {
        steps.push(RecoveryStep::Run);
    }
    steps
}

thread_local! {
    static PASSES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

fn count_pass() {
    PASSES.with(|n| n.set(n.get() + 1));
}

/// How many passes over a plan's obligations the spine readers have made
/// on this thread: one per site a [`LifecyclePlan`] reader scans for and
/// one per plan-wide pair set it computes, and one per
/// [`SpineIndex::new`]. What a guard reads to hold that an emitter asks
/// the index, whose cost does not grow with the literals it lowers.
pub fn plan_passes() -> u64 {
    PASSES.with(|n| n.get())
}

/// Pairs of steps of one order: per declaration, by lowered name, its
/// templates'; and every template's.
#[derive(Debug)]
struct Pairs<T> {
    own: BTreeMap<String, BTreeSet<(T, T)>>,
    every: BTreeSet<(T, T)>,
    /// A declaration with no template's own pairs.
    none: BTreeSet<(T, T)>,
}

impl<T: Ord + Copy> Pairs<T> {
    fn new() -> Self {
        Pairs { own: BTreeMap::new(), every: BTreeSet::new(), none: BTreeSet::new() }
    }

    fn add(&mut self, lowered: &str, pairs: impl IntoIterator<Item = (T, T)>) {
        let own = self.own.entry(lowered.to_string()).or_default();
        for p in pairs {
            own.insert(p);
            self.every.insert(p);
        }
    }

    fn own(&self, lowered: &str) -> &BTreeSet<(T, T)> {
        self.own.get(lowered).unwrap_or(&self.none)
    }

    fn of(&self, lowered: &str) -> (&BTreeSet<(T, T)>, &BTreeSet<(T, T)>) {
        (self.own(lowered), &self.every)
    }
}

/// The plan's orders as the emitters read them, computed once per plan
/// (F.40 phase 3, close). [`LifecyclePlan::birth_order`],
/// [`LifecyclePlan::reclaim_order`], [`LifecyclePlan::cascade_order`] and
/// [`LifecyclePlan::recovery_order`] compute, on every call, the pair set
/// over every template of the plan, each template's spine a scan of its
/// obligations; an emitter asks once per literal or declaration. The
/// index makes one pass: each site's rows found once, each template's
/// pairs read once, each declaration's and the plan's sets formed once.
/// Its answers are the plan's readers', for every input
/// (`lifecycle_plan.rs` holds that over the corpus). It is not part of
/// the plan's value: the lowering builds it from the plan it reads.
#[derive(Debug)]
pub struct SpineIndex {
    birth: Pairs<ObligationKind>,
    reclaim: Pairs<ReclaimStep>,
    cascade: Pairs<CascadeStep>,
    recovery: Pairs<RecoveryStep>,
    /// The declarations some template of which owes readiness.
    readiness: BTreeSet<String>,
    /// The declarations some template of which owes a `run()` per
    /// incarnation.
    incarnation_run: BTreeSet<String>,
    /// Per spine, [`LifecyclePlan::process_order`] and
    /// [`LifecyclePlan::entry_order`]: a frame's teardown asks on every
    /// exit it lowers.
    process: BTreeMap<Spine, Result<Vec<SpineStep>, String>>,
    entry: BTreeMap<Spine, Result<Vec<EntryStep>, String>>,
}

impl SpineIndex {
    pub fn new(plan: &LifecyclePlan) -> SpineIndex {
        count_pass();
        // Each site's rows, in the producer's order, and each row's
        // dissolve rows waiting on it (line 10's test).
        let mut at: BTreeMap<(&super::DeclRef, &super::Template), Vec<ObligationId>> = BTreeMap::new();
        let mut dissolve_waiters: BTreeMap<ObligationId, Vec<ObligationId>> = BTreeMap::new();
        for (id, o) in plan.iter() {
            if let Some(s) = &o.site {
                at.entry((&s.decl, &s.template)).or_default().push(id);
            }
            if o.kind == ObligationKind::Dissolve {
                for p in &o.edges.entry {
                    dissolve_waiters.entry(p.event.obligation).or_default().push(id);
                }
            }
        }
        let mut ix = SpineIndex {
            birth: Pairs::new(),
            reclaim: Pairs::new(),
            cascade: Pairs::new(),
            recovery: Pairs::new(),
            readiness: BTreeSet::new(),
            incarnation_run: BTreeSet::new(),
            process: Spine::ALL.iter().map(|&s| (s, plan.process_order(s))).collect(),
            entry: Spine::ALL.iter().map(|&s| (s, plan.entry_order(s))).collect(),
        };
        for inst in &plan.instances {
            let site = &inst.site;
            let lowered = site.decl.lowered.as_str();
            let rows: &[ObligationId] = at.get(&(&site.decl, &site.template)).map_or(&[], Vec::as_slice);
            let seq: Vec<ObligationKind> = plan.birth_spine_of(rows).iter().map(|s| s.kind).collect();
            if seq.contains(&ObligationKind::Readiness) {
                ix.readiness.insert(lowered.to_string());
            }
            ix.birth.add(lowered, seq.iter().enumerate().flat_map(|(i, &a)| seq[i + 1..].iter().map(move |&b| (a, b))));
            ix.reclaim.add(lowered, plan.reclaim_pairs_of(site, rows));
            let fields_after = |dissolve: ObligationId| {
                let after_mine = Event { obligation: dissolve, point: Point::Completed };
                dissolve_waiters.get(&dissolve).is_some_and(|ws| {
                    ws.iter().any(|&w| {
                        let o = &plan.obligations[w.0 as usize];
                        o.site.as_ref() != Some(site) && o.edges.entry.iter().any(|p| p.event == after_mine)
                    })
                })
            };
            ix.cascade.add(lowered, plan.cascade_pairs_of(site, rows, fields_after));
            ix.recovery.add(lowered, plan.recovery_pairs_of(rows));
            if plan.incarnation_row(rows, ObligationKind::Run).is_some() {
                ix.incarnation_run.insert(lowered.to_string());
            }
        }
        ix
    }

    /// [`LifecyclePlan::birth_order`].
    pub fn birth_order(&self, lowered: &str, kinds: &[ObligationKind]) -> Result<Vec<ObligationKind>, String> {
        order_by(lowered, kinds, self.birth.own(lowered), &self.birth.every, BIRTH_KINDS, |k| k.name())
    }

    /// [`LifecyclePlan::reclaim_order`].
    pub fn reclaim_order(&self, lowered: &str) -> Result<Vec<ReclaimStep>, String> {
        order_by(lowered, RECLAIM_STEPS, self.reclaim.own(lowered), &self.reclaim.every, RECLAIM_STEPS, |s| s.name())
    }

    /// [`LifecyclePlan::cascade_order`].
    pub fn cascade_order(&self, lowered: &str) -> Result<Vec<CascadeStep>, String> {
        order_by(lowered, CASCADE_STEPS, self.cascade.own(lowered), &self.cascade.every, CASCADE_STEPS, |s| s.name())
    }

    /// [`LifecyclePlan::recovery_order`].
    pub fn recovery_order(&self, lowered: &str) -> Result<Vec<RecoveryStep>, String> {
        let run = !self.recovery.own.contains_key(lowered) || self.incarnation_run.contains(lowered);
        order_by(lowered, &recovery_steps(run), self.recovery.own(lowered), &self.recovery.every, RECOVERY_STEPS, |s| {
            s.name()
        })
    }

    /// Whether some template of the declaration lowered as `lowered`
    /// owes readiness on its birth spine ([`LifecyclePlan::birth_spine`]).
    pub fn owes_readiness(&self, lowered: &str) -> bool {
        self.readiness.contains(lowered)
    }

    /// The pairs each order reads for the declaration lowered as
    /// `lowered`: its templates' (empty where it has none), then every
    /// template's. Birth pairs are those of each template's
    /// [`LifecyclePlan::birth_spine`]; the rest, each template's
    /// [`LifecyclePlan::reclaim_pairs`], [`LifecyclePlan::cascade_pairs`]
    /// and [`LifecyclePlan::recovery_pairs`].
    pub fn birth_pairs(
        &self,
        lowered: &str,
    ) -> (&BTreeSet<(ObligationKind, ObligationKind)>, &BTreeSet<(ObligationKind, ObligationKind)>) {
        self.birth.of(lowered)
    }

    pub fn reclaim_pairs(&self, lowered: &str) -> (&BTreeSet<(ReclaimStep, ReclaimStep)>, &BTreeSet<(ReclaimStep, ReclaimStep)>) {
        self.reclaim.of(lowered)
    }

    pub fn cascade_pairs(&self, lowered: &str) -> (&BTreeSet<(CascadeStep, CascadeStep)>, &BTreeSet<(CascadeStep, CascadeStep)>) {
        self.cascade.of(lowered)
    }

    pub fn recovery_pairs(
        &self,
        lowered: &str,
    ) -> (&BTreeSet<(RecoveryStep, RecoveryStep)>, &BTreeSet<(RecoveryStep, RecoveryStep)>) {
        self.recovery.of(lowered)
    }

    /// [`LifecyclePlan::process_order`].
    pub fn process_order(&self, spine: Spine) -> Result<Vec<SpineStep>, String> {
        self.process[&spine].clone()
    }

    /// [`LifecyclePlan::entry_order`].
    pub fn entry_order(&self, spine: Spine) -> Result<Vec<EntryStep>, String> {
        self.entry[&spine].clone()
    }
}

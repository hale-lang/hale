//! `ClaimIr` — every law surface, lowered to one typed vocabulary
//! (GH #476 Change 4).
//!
//! The epic's law: `Source --check/derive--> Model --judge(ClaimIr)
//! --> Evidence`. A judgment engine (Changes 5a–5e) evaluates laws
//! against the [`ApplicationModel`]; this module is the laws' typed
//! form — **one variant per law form**, across every surface the
//! language has grown:
//!
//! * `claims { }` block forms (#382): reachability, boundary
//!   grants, endpoint existence, sealing, attribution, coverage,
//!   cardinality, path bounds;
//! * constitution clauses (#409) — the same forms, arriving through
//!   adoption, with their origin recorded;
//! * library-tier claims (#392 thread 2) — a seed swearing about
//!   itself, alias-attributed;
//! * fn/locus effect assertions (#265/#330/#345/#354):
//!   `@effects(none/publish/causes/only)`, `@no_panic`,
//!   `@effects(depends: …)`, `@phase_effects`, `@budget`;
//! * fleet plan rows (#408) — deployment claims over instances,
//!   name-level until `FleetModel` (Change 7) gives them typed
//!   targets.
//!
//! **Change 4 is lowering only.** The old evaluators stay active
//! and authoritative; nothing consumes these rows yet. What this
//! buys now: the clause universe is enumerated ONCE (the lowering
//! shares the evaluator's own collection walk), every law is
//! representable in model vocabulary, and the Change-5 migration
//! can proceed family-by-family against a stable target.
//!
//! ## Reference doctrine
//!
//! A lowered reference carries three things: the **raw** canonical
//! spelling (post-merge symbol — the model's identity space), the
//! **display** spelling (what the author wrote / what diagnostics
//! print), and — when the model resolves it — the typed **id**.
//! `id: None` is not an error at this layer: the lowering is total
//! over parseable programs, and an unresolvable name is exactly
//! what the judgment vocabulary's `invalid` verdict exists for
//! (Change 5). Fleet rows always carry `None` ids at Change 4 —
//! their targets live in a plan, not in this application's model.
//!
//! ## Ordering
//!
//! Rows keep AUTHORED order via `ordinal` — the evaluator reports
//! outcomes in authored order and Change 5's diagnostics-parity
//! differential needs to walk both lists in lockstep. `validate`
//! enforces ordinal contiguity from zero, which makes the table
//! canonically ordered without imposing a lexical sort that would
//! scramble evaluation order.

use crate::ids::{
    FunctionId, GroupId, LocusDeclId, PhaseId, ProvenanceId, SeedId,
    SubjectId, TopicId,
};
use crate::provenance::ProvenanceTable;
use crate::ApplicationModel;

/// A name in claim position: raw canonical spelling + author
/// display. Two spellings, same doctrine as every entity table.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct NameRef {
    pub raw: String,
    pub display: String,
}

/// A group reference, resolved against `entities.groups` when the
/// model knows it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct GroupRef {
    pub group: Option<GroupId>,
    pub name: NameRef,
}

/// A topic reference (`T` / `alias::T`, canonicalized at mangle).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TopicIrRef {
    pub topic: Option<TopicId>,
    pub name: NameRef,
}

/// An effect class by canonical name — the model's label/effects
/// vocabulary (`syscall`, `alloc`, …, or a declared user class).
/// String-keyed on purpose: the class universe is program-declared
/// and the model's labels/effects sections already speak it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct EffectClassRef {
    pub name: String,
}

/// A set expression in claim-argument position.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SetIr {
    Group(GroupRef),
    /// `effects(money)` — declared carriers of the class.
    EffectCarriers(EffectClassRef),
}

/// One granted edge inside `only edges { … }`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct GrantIr {
    /// `publish T` when true, `subscribe T` when false — both admit
    /// the same edge; the verb names the reviewable declaration.
    pub publish: bool,
    pub topic: TopicIrRef,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CountCmpIr {
    Eq,
    Le,
    Ge,
}

/// A quantitative budget dimension (`@budget(<dim> = N)`).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum QuantDimIr {
    StackBytes,
    BlockPoints,
    Publish,
    Fanout,
    /// `@budget(<user class> = N)` — carrier-call bound.
    UserClass(EffectClassRef),
}

/// Where a law came from. Origin is provenance at the law grain —
/// the same clause text means different lifetimes depending on
/// whether it was authored here, adopted, or shipped with a seed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ClaimOrigin {
    /// Authored in this main's `claims { }` block.
    Main,
    /// Adopted from a constitution (#409); carries its name as the
    /// evaluator reports it (`ClaimOutcome::source`).
    Constitution { name: String },
    /// A library-tier top-level block (#392 thread 2); the alias the
    /// evaluator attributes rows to, when one resolved.
    Library { alias: Option<String> },
    /// An `@effects` / `@budget` / `@phase_effects` annotation — the
    /// law rides on a declaration rather than a claims block.
    Annotation,
    /// A deployment-plan claim row (#408). Targets are plan-level
    /// names until Change 7's `FleetModel`.
    FleetPlan,
}

/// One law form. ONE variant per form, across every surface.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ClaimIr {
    // ---------- claims-block forms (#382) ----------
    /// `forbid reaches(SRC, DST) [via { … }] [during P] [avoiding G]`.
    ForbidReaches {
        src: SetIr,
        dst: SetIr,
        via_calls: bool,
        via_bus: bool,
        during: Option<(Option<PhaseId>, String)>,
        avoiding: Option<GroupRef>,
    },
    /// `only edges SRC -> DST { grants }`.
    OnlyEdges {
        src: GroupRef,
        dst: GroupRef,
        grants: Vec<GrantIr>,
    },
    /// `bound C <= N on paths from G`.
    Bound {
        class: EffectClassRef,
        limit: u64,
        from: GroupRef,
    },
    /// `require publishes/subscribes(some G, topic T)`.
    RequireEndpoint {
        publishers: bool,
        group: GroupRef,
        topic: TopicIrRef,
    },
    /// `require sealed(all G)` (GH #436).
    RequireSealed { group: GroupRef },
    /// `require attributed(all <class>)` (GH #436).
    RequireAttributed { class: EffectClassRef },
    /// `cover topic in seed(a): subscribed_by(some G)`.
    Cover {
        seed: (Option<SeedId>, String),
        group: GroupRef,
    },
    /// `count publishers/subscribers(topic T) <cmp> N`.
    Count {
        publishers: bool,
        topic: TopicIrRef,
        cmp: CountCmpIr,
        n: u64,
    },

    // ---------- fn/locus effect assertions (#265) ----------
    /// `@effects(none: {…})` — forbidden classes.
    EffectForbid {
        at: (Option<FunctionId>, NameRef),
        classes: Vec<EffectClassRef>,
    },
    /// `@effects(publish: {…})` — the closed allowed publish set.
    EffectPublishSet {
        at: (Option<FunctionId>, NameRef),
        subjects: Vec<(Option<SubjectId>, NameRef)>,
    },
    /// `@effects(causes: {…})` — transitive through bus edges.
    EffectCauses {
        at: (Option<FunctionId>, NameRef),
        classes: Vec<EffectClassRef>,
    },
    /// `@effects(only: {…})` — the closed subset form (#354).
    EffectOnly {
        at: (Option<FunctionId>, NameRef),
        classes: Vec<EffectClassRef>,
    },
    /// `@no_panic`.
    NoPanic { at: (Option<FunctionId>, NameRef) },
    /// `@effects(depends: {…})` on a locus (RFC #330) — the
    /// COMPLETE backward-reachable subject set.
    DependsSet {
        locus: (Option<LocusDeclId>, NameRef),
        subjects: Vec<(Option<SubjectId>, NameRef)>,
    },
    /// `@phase_effects(birth: {alloc}, run: {})` on a locus.
    PhaseEffects {
        locus: (Option<LocusDeclId>, NameRef),
        /// (phase name, allowed classes); absent phase =
        /// unconstrained, present-empty = forbids all.
        phases: Vec<(String, Vec<EffectClassRef>)>,
    },
    /// `@budget(alloc_per_call = N)`.
    AllocBudget {
        at: (Option<FunctionId>, NameRef),
        per_call: u32,
    },
    /// `@budget(<dim> = N)` beyond `alloc_per_call`.
    QuantBudget {
        at: (Option<FunctionId>, NameRef),
        dim: QuantDimIr,
        limit: u64,
    },

    // ---------- fleet plan rows (#408) ----------
    /// Plan `forbid_reaches` — instance/group names are plan-level.
    FleetForbidReaches {
        from: String,
        to: String,
        avoiding: Option<String>,
    },
    /// Plan `require_subscribes` / `require_publishes`.
    FleetRequireEndpoint {
        publishers: bool,
        target: String,
        topic: String,
    },
    /// Plan `count_publisher_instances` / `count_subscriber_instances`.
    FleetCountInstances {
        publishers: bool,
        topic: String,
        cmp: CountCmpIr,
        n: u64,
    },
    /// Plan `only_edges`.
    FleetOnlyEdges {
        src: String,
        dst: String,
        grants: Vec<String>,
    },
}

/// One lowered law row.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ClaimRow {
    /// Authored position (see module docs) — the canonical order.
    pub ordinal: u32,
    /// The claim's name where the surface has one (claims-block
    /// forms; library rows arrive alias-prefixed exactly as the
    /// evaluator attributes them). Annotations and plan rows carry
    /// the annotated declaration's / plan row's name.
    pub name: String,
    pub origin: ClaimOrigin,
    pub law: ClaimIr,
    pub provenance: ProvenanceId,
}

/// The lowered law table for one application (or one plan).
#[derive(Clone, Debug, Default)]
pub struct ClaimIrTable {
    pub rows: Vec<ClaimRow>,
    /// Row provenance — its OWN table (the laws are judged WITH the
    /// model, not stored in it; sharing the model's interner would
    /// force the lowering to mutate a finished model).
    pub provenance: ProvenanceTable,
}

/// A violated `ClaimIr` law.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ClaimIrError {
    /// `rows[i].ordinal != i` — authored order is the canonical
    /// order and must be contiguous from zero.
    NonContiguousOrdinal { index: usize },
    /// A resolved id points outside the model's table.
    DanglingId { index: usize, what: &'static str },
    /// A row's provenance id is out of range.
    DanglingProvenance { index: usize },
    /// An empty name — every row must be addressable in diagnostics.
    UnnamedRow { index: usize },
}

impl ClaimIrTable {
    /// Structural laws, checked against the model the rows were
    /// lowered from. Resolution is OPTIONAL (`None` ids are lawful
    /// residue for Change 5's `invalid` verdict); a PRESENT id must
    /// be in range.
    pub fn validate(
        &self,
        model: &ApplicationModel,
    ) -> Result<(), ClaimIrError> {
        let e = &model.entities;
        for (i, row) in self.rows.iter().enumerate() {
            if row.ordinal as usize != i {
                return Err(ClaimIrError::NonContiguousOrdinal {
                    index: i,
                });
            }
            if row.name.is_empty() {
                return Err(ClaimIrError::UnnamedRow { index: i });
            }
            if row.provenance.index() >= self.provenance.records.len()
            {
                return Err(ClaimIrError::DanglingProvenance {
                    index: i,
                });
            }
            let bad =
                |what: &'static str| ClaimIrError::DanglingId {
                    index: i,
                    what,
                };
            let group_ok = |g: &GroupRef| match g.group {
                Some(id) => id.index() < e.groups.len(),
                None => true,
            };
            let topic_ok = |t: &TopicIrRef| match t.topic {
                Some(id) => id.index() < e.topics.len(),
                None => true,
            };
            let set_ok = |s: &SetIr| match s {
                SetIr::Group(g) => group_ok(g),
                SetIr::EffectCarriers(_) => true,
            };
            let fn_ok = |f: &(Option<FunctionId>, NameRef)| match f.0 {
                Some(id) => id.index() < e.functions.len(),
                None => true,
            };
            let locus_ok =
                |l: &(Option<LocusDeclId>, NameRef)| match l.0 {
                    Some(id) => id.index() < e.loci.len(),
                    None => true,
                };
            let subj_ok =
                |s: &(Option<SubjectId>, NameRef)| match s.0 {
                    Some(id) => id.index() < e.subjects.len(),
                    None => true,
                };
            match &row.law {
                ClaimIr::ForbidReaches {
                    src,
                    dst,
                    during,
                    avoiding,
                    ..
                } => {
                    if !set_ok(src) || !set_ok(dst) {
                        return Err(bad("set"));
                    }
                    if let Some((Some(p), _)) = during {
                        if p.index() >= e.phases.len() {
                            return Err(bad("phase"));
                        }
                    }
                    if let Some(a) = avoiding {
                        if !group_ok(a) {
                            return Err(bad("avoiding"));
                        }
                    }
                }
                ClaimIr::OnlyEdges { src, dst, grants } => {
                    if !group_ok(src) || !group_ok(dst) {
                        return Err(bad("group"));
                    }
                    if grants.iter().any(|g| !topic_ok(&g.topic)) {
                        return Err(bad("grant topic"));
                    }
                }
                ClaimIr::Bound { from, .. } => {
                    if !group_ok(from) {
                        return Err(bad("group"));
                    }
                }
                ClaimIr::RequireEndpoint { group, topic, .. } => {
                    if !group_ok(group) || !topic_ok(topic) {
                        return Err(bad("endpoint ref"));
                    }
                }
                ClaimIr::RequireSealed { group } => {
                    if !group_ok(group) {
                        return Err(bad("group"));
                    }
                }
                ClaimIr::RequireAttributed { .. } => {}
                ClaimIr::Cover { seed, group } => {
                    if let Some(s) = seed.0 {
                        if s.index() >= e.seeds.len() {
                            return Err(bad("seed"));
                        }
                    }
                    if !group_ok(group) {
                        return Err(bad("group"));
                    }
                }
                ClaimIr::Count { topic, .. } => {
                    if !topic_ok(topic) {
                        return Err(bad("topic"));
                    }
                }
                ClaimIr::EffectForbid { at, .. }
                | ClaimIr::EffectCauses { at, .. }
                | ClaimIr::EffectOnly { at, .. }
                | ClaimIr::NoPanic { at }
                | ClaimIr::AllocBudget { at, .. }
                | ClaimIr::QuantBudget { at, .. } => {
                    if !fn_ok(at) {
                        return Err(bad("function"));
                    }
                }
                ClaimIr::EffectPublishSet { at, subjects } => {
                    if !fn_ok(at) {
                        return Err(bad("function"));
                    }
                    if subjects.iter().any(|s| !subj_ok(s)) {
                        return Err(bad("subject"));
                    }
                }
                ClaimIr::DependsSet { locus, subjects } => {
                    if !locus_ok(locus) {
                        return Err(bad("locus"));
                    }
                    if subjects.iter().any(|s| !subj_ok(s)) {
                        return Err(bad("subject"));
                    }
                }
                ClaimIr::PhaseEffects { locus, .. } => {
                    if !locus_ok(locus) {
                        return Err(bad("locus"));
                    }
                }
                ClaimIr::FleetForbidReaches { .. }
                | ClaimIr::FleetRequireEndpoint { .. }
                | ClaimIr::FleetCountInstances { .. }
                | ClaimIr::FleetOnlyEdges { .. } => {}
            }
        }
        Ok(())
    }
}

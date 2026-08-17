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
    EffectClassId, FunctionId, GroupId, LocusDeclId, PhaseId,
    ProvenanceId, SeedId, SubjectId, TopicId,
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
/// model knows it. Every source-bearing reference carries its OWN
/// provenance — the evaluator anchors "unknown group" at the
/// reference, not at the clause (review round 15), and Change 5
/// must preserve those primary spans without reopening the AST.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct GroupRef {
    pub group: Option<GroupId>,
    pub name: NameRef,
    pub provenance: ProvenanceId,
}

/// A topic reference (`T` / `alias::T`, canonicalized at mangle).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TopicIrRef {
    pub topic: Option<TopicId>,
    pub name: NameRef,
    pub provenance: ProvenanceId,
}

/// A phase reference (`during P`).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PhaseIrRef {
    pub phase: Option<PhaseId>,
    pub name: String,
    pub provenance: ProvenanceId,
}

/// A seed reference (`in seed(a)`).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SeedIrRef {
    pub seed: Option<SeedId>,
    pub name: String,
    pub provenance: ProvenanceId,
}


/// A bus selector for annotation surfaces (`@effects(publish:)`,
/// `@effects(depends:)`) — review rounds 16/17. The authoritative
/// evaluator compares each entry against RESOLVED bus subjects with
/// `topic_ref_matches` (exact string, or trailing-name), and the
/// entry list admits identifiers AND string literals — so one
/// spelling can legitimately denote a literal wire subject
/// (`"audit.log"`), a declared topic, or an unqualified cross-seed
/// name matching several same-tailed merged topics ("the
/// permissiveness the author asked for"). The selector represents
/// all three; only an explicit alias path is a single exact
/// reference.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum BusSelector {
    /// A qualified alias path — the author named one topic.
    Exact(TopicIrRef),
    /// Any other spelling: matched exact-or-trailing-name against
    /// declared TOPICS and wire SUBJECTS — both candidate sets,
    /// sorted. A literal wire subject lands in `subjects`; both
    /// empty = resolves to nothing (Change 5's residue).
    Match {
        name: String,
        topics: Vec<TopicId>,
        subjects: Vec<SubjectId>,
        provenance: ProvenanceId,
    },
}

/// An effect class reference. The class universe is TYPED
/// (review round 15): a built-in is language-fixed and always
/// valid; a user class resolves into `entities.effect_classes`,
/// whose row records declaration status and the normalized
/// composition — so an IR consumer can distinguish a declared
/// class from an interned typo and expand `effect io = {…}`
/// without the AST.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct EffectClassRef {
    /// `Some` iff the name resolves to a user-class table row.
    pub class: Option<EffectClassId>,
    /// `true` iff the name is a language built-in (`syscall`, …).
    pub builtin: bool,
    pub name: String,
    pub provenance: ProvenanceId,
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
        during: Option<PhaseIrRef>,
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
    Cover { seed: SeedIrRef, group: GroupRef },
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
    /// TOPIC references, not wire subjects: the source names topics
    /// and the evaluator matches topic identity across local /
    /// alias-qualified / mangled spellings (review round 15) — a
    /// declared topic must never masquerade as its wire subject.
    EffectPublishSet {
        at: (Option<FunctionId>, NameRef),
        entries: Vec<BusSelector>,
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
        entries: Vec<BusSelector>,
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
    /// Plan `count_publisher_instances` / `count_subscriber_instances`
    /// — ONE law: the fleet evaluator judges the eq/max/min bounds
    /// conjunctively under one claim name and one verdict (review
    /// round 15), so the IR must not split them into separate laws.
    FleetCountInstances {
        publishers: bool,
        topic: String,
        eq: Option<u64>,
        max: Option<u64>,
        min: Option<u64>,
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

/// A structured lowering issue: law-SELECTION state that produced
/// no row — an unknown or cyclic constitution, an illegal
/// library-tier `adopt`, a duplicate declaration, a name collision,
/// a malformed plan claim. Preserved so an IR-only evaluator
/// (Change 5) observes the invalidity instead of "no law"
/// (review round 15) — the lowering never silently drops law-shaped
/// source.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LoweringIssue {
    pub message: String,
    pub provenance: ProvenanceId,
}

/// The lowered law table for one application (or one plan).
#[derive(Clone, Debug, Default)]
pub struct ClaimIrTable {
    pub rows: Vec<ClaimRow>,
    /// Law-selection invalidity (see [`LoweringIssue`]).
    pub issues: Vec<LoweringIssue>,
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
    /// A resolved id's entity disagrees with the reference's
    /// duplicated name/display fields — an evaluator using the id
    /// and a renderer using the name would describe different laws.
    NameDisagreement { index: usize, what: &'static str },
}

impl ClaimIrTable {
    /// Structural laws, checked against the model the rows were
    /// lowered from. Resolution is OPTIONAL (`None` ids are lawful
    /// residue for Change 5's `invalid` verdict); a PRESENT id must
    /// be in range AND agree with the reference's duplicated
    /// name/display fields — an evaluator using the id and a
    /// renderer using the name must describe the same law
    /// (review round 15).
    pub fn validate(
        &self,
        model: &ApplicationModel,
    ) -> Result<(), ClaimIrError> {
        let e = &model.entities;
        let prov_len = self.provenance.records.len();
        for (i, issue) in self.issues.iter().enumerate() {
            if issue.provenance.index() >= prov_len {
                return Err(ClaimIrError::DanglingProvenance {
                    index: i,
                });
            }
        }
        for (i, row) in self.rows.iter().enumerate() {
            if row.ordinal as usize != i {
                return Err(ClaimIrError::NonContiguousOrdinal {
                    index: i,
                });
            }
            if row.name.is_empty() {
                return Err(ClaimIrError::UnnamedRow { index: i });
            }
            let pr = |p: ProvenanceId| p.index() < prov_len;
            if !pr(row.provenance) {
                return Err(ClaimIrError::DanglingProvenance {
                    index: i,
                });
            }
            let bad = |what: &'static str| ClaimIrError::DanglingId {
                index: i,
                what,
            };
            let dis = |what: &'static str| {
                ClaimIrError::NameDisagreement { index: i, what }
            };
            let group_ok =
                |g: &GroupRef| -> Result<(), ClaimIrError> {
                    if !pr(g.provenance) {
                        return Err(ClaimIrError::DanglingProvenance {
                            index: i,
                        });
                    }
                    match g.group {
                        None => Ok(()),
                        Some(id) => {
                            let Some(row) = e.groups.get(id.index())
                            else {
                                return Err(bad("group"));
                            };
                            if row.name != g.name.raw
                                || row.display != g.name.display
                            {
                                return Err(dis("group"));
                            }
                            Ok(())
                        }
                    }
                };
            let topic_ok =
                |t: &TopicIrRef| -> Result<(), ClaimIrError> {
                    if !pr(t.provenance) {
                        return Err(ClaimIrError::DanglingProvenance {
                            index: i,
                        });
                    }
                    match t.topic {
                        None => Ok(()),
                        Some(id) => {
                            let Some(row) = e.topics.get(id.index())
                            else {
                                return Err(bad("topic"));
                            };
                            if row.name != t.name.raw
                                || row.display != t.name.display
                            {
                                return Err(dis("topic"));
                            }
                            Ok(())
                        }
                    }
                };
            let class_ok =
                |c: &EffectClassRef| -> Result<(), ClaimIrError> {
                    if !pr(c.provenance) {
                        return Err(ClaimIrError::DanglingProvenance {
                            index: i,
                        });
                    }
                    match c.class {
                        None => Ok(()),
                        Some(id) => {
                            if c.builtin {
                                // A built-in never has a table row.
                                return Err(dis("effect class"));
                            }
                            let Some(row) =
                                e.effect_classes.get(id.index())
                            else {
                                return Err(bad("effect class"));
                            };
                            if row.name != c.name {
                                return Err(dis("effect class"));
                            }
                            Ok(())
                        }
                    }
                };
            let set_ok = |s: &SetIr| -> Result<(), ClaimIrError> {
                match s {
                    SetIr::Group(g) => group_ok(g),
                    SetIr::EffectCarriers(c) => class_ok(c),
                }
            };
            let fn_ok = |f: &(Option<FunctionId>, NameRef)| -> Result<(), ClaimIrError> {
                match f.0 {
                    None => Ok(()),
                    Some(id) => {
                        let Some(row) = e.functions.get(id.index())
                        else {
                            return Err(bad("function"));
                        };
                        if row.name != f.1.raw
                            || row.display != f.1.display
                        {
                            return Err(dis("function"));
                        }
                        Ok(())
                    }
                }
            };
            let locus_ok = |l: &(Option<LocusDeclId>, NameRef)| -> Result<(), ClaimIrError> {
                match l.0 {
                    None => Ok(()),
                    Some(id) => {
                        let Some(row) = e.loci.get(id.index()) else {
                            return Err(bad("locus"));
                        };
                        if row.name != l.1.raw
                            || row.display != l.1.display
                        {
                            return Err(dis("locus"));
                        }
                        Ok(())
                    }
                }
            };
            let sel_ok =
                |sel: &BusSelector| -> Result<(), ClaimIrError> {
                    match sel {
                        BusSelector::Exact(t) => topic_ok(t),
                        BusSelector::Match {
                            topics,
                            subjects,
                            provenance,
                            ..
                        } => {
                            if !pr(*provenance) {
                                return Err(ClaimIrError::DanglingProvenance { index: i });
                            }
                            if topics
                                .windows(2)
                                .any(|w| w[0] >= w[1])
                                || subjects
                                    .windows(2)
                                    .any(|w| w[0] >= w[1])
                            {
                                return Err(bad("selector candidates"));
                            }
                            if topics.iter().any(|c| {
                                c.index() >= e.topics.len()
                            }) || subjects.iter().any(|c| {
                                c.index() >= e.subjects.len()
                            }) {
                                return Err(bad("selector candidate"));
                            }
                            Ok(())
                        }
                    }
                };
            match &row.law {
                ClaimIr::ForbidReaches {
                    src,
                    dst,
                    during,
                    avoiding,
                    ..
                } => {
                    set_ok(src)?;
                    set_ok(dst)?;
                    if let Some(d) = during {
                        if !pr(d.provenance) {
                            return Err(
                                ClaimIrError::DanglingProvenance {
                                    index: i,
                                },
                            );
                        }
                        if let Some(pid) = d.phase {
                            let Some(row) =
                                e.phases.get(pid.index())
                            else {
                                return Err(bad("phase"));
                            };
                            if row.name != d.name {
                                return Err(dis("phase"));
                            }
                        }
                    }
                    if let Some(a) = avoiding {
                        group_ok(a)?;
                    }
                }
                ClaimIr::OnlyEdges { src, dst, grants } => {
                    group_ok(src)?;
                    group_ok(dst)?;
                    for g in grants {
                        topic_ok(&g.topic)?;
                    }
                }
                ClaimIr::Bound { class, from, .. } => {
                    class_ok(class)?;
                    group_ok(from)?;
                }
                ClaimIr::RequireEndpoint { group, topic, .. } => {
                    group_ok(group)?;
                    topic_ok(topic)?;
                }
                ClaimIr::RequireSealed { group } => group_ok(group)?,
                ClaimIr::RequireAttributed { class } => {
                    class_ok(class)?
                }
                ClaimIr::Cover { seed, group } => {
                    if !pr(seed.provenance) {
                        return Err(ClaimIrError::DanglingProvenance {
                            index: i,
                        });
                    }
                    if let Some(id) = seed.seed {
                        let Some(row) = e.seeds.get(id.index())
                        else {
                            return Err(bad("seed"));
                        };
                        if row.name != seed.name {
                            return Err(dis("seed"));
                        }
                    }
                    group_ok(group)?;
                }
                ClaimIr::Count { topic, .. } => topic_ok(topic)?,
                ClaimIr::EffectForbid { at, classes }
                | ClaimIr::EffectCauses { at, classes }
                | ClaimIr::EffectOnly { at, classes } => {
                    fn_ok(at)?;
                    for c in classes {
                        class_ok(c)?;
                    }
                }
                ClaimIr::EffectPublishSet { at, entries } => {
                    fn_ok(at)?;
                    for t in entries {
                        sel_ok(t)?;
                    }
                }
                ClaimIr::NoPanic { at } => fn_ok(at)?,
                ClaimIr::AllocBudget { at, .. } => fn_ok(at)?,
                ClaimIr::QuantBudget { at, dim, .. } => {
                    fn_ok(at)?;
                    if let QuantDimIr::UserClass(c) = dim {
                        class_ok(c)?;
                    }
                }
                ClaimIr::DependsSet { locus, entries } => {
                    locus_ok(locus)?;
                    for t in entries {
                        sel_ok(t)?;
                    }
                }
                ClaimIr::PhaseEffects { locus, phases } => {
                    locus_ok(locus)?;
                    for (_, cs) in phases {
                        for c in cs {
                            class_ok(c)?;
                        }
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

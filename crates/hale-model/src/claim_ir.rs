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

/// The CANONICAL bus-set matching rule (review round 19): exact
/// string equality first, trailing-name equality second — the same
/// `topic_ref_matches` the authoritative evaluator applies
/// (`hale-types::effects` delegates HERE, so there is exactly one
/// definition and the dependency-free schema can validate candidate
/// sets against it).
pub fn bus_ref_matches(declared: &str, resolved: &str) -> bool {
    if declared == resolved {
        return true;
    }
    bus_topic_tail(declared) == bus_topic_tail(resolved)
}

/// The bare topic name, whichever spelling reached us: the last
/// `::` segment; for a merged `__lib_…` symbol, the last `_` token.
pub fn bus_topic_tail(s: &str) -> &str {
    let s = s.rsplit("::").next().unwrap_or(s);
    match s.strip_prefix("__lib_") {
        Some(rest) => rest.rsplit('_').next().unwrap_or(rest),
        None => s,
    }
}

/// The language's built-in effect classes — the CANONICAL
/// vocabulary (review round 20), mirrored from the compiler's
/// `EffectClass` (a conformance test in `hale-types` pins the two
/// lists to each other, since this crate cannot depend on the
/// AST). `validate` holds every [`EffectClassRef::builtin`] flag to
/// exactly this list.
pub const BUILTIN_EFFECT_CLASSES: [&str; 11] = [
    "syscall",
    "block",
    "time",
    "entropy",
    "env",
    "ffi",
    "publish",
    "spawn",
    "recursion",
    "alloc",
    "secret_use",
];

/// Is `name` a language built-in effect class?
pub fn is_builtin_effect_class(name: &str) -> bool {
    BUILTIN_EFFECT_CLASSES.contains(&name)
}

/// A name in claim position: raw canonical spelling + author
/// display. Two spellings, same doctrine as every entity table.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct NameRef {
    pub raw: String,
    pub display: String,
}

/// A group reference, resolved against `entities.groups` when the
/// model knows it. Every source-bearing reference carries its OWN
/// provenance — the evaluator anchors "unknown group" at the
/// reference, not at the clause (review round 15), and Change 5
/// must preserve those primary spans without reopening the AST.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct GroupRef {
    pub group: Option<GroupId>,
    pub name: NameRef,
    pub provenance: ProvenanceId,
}

/// A topic reference (`T` / `alias::T`, canonicalized at mangle).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct TopicIrRef {
    pub topic: Option<TopicId>,
    pub name: NameRef,
    pub provenance: ProvenanceId,
}

/// A phase reference (`during P`).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct PhaseIrRef {
    pub phase: Option<PhaseId>,
    pub name: String,
    pub provenance: ProvenanceId,
}

/// A seed reference (`in seed(a)`).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct SeedIrRef {
    pub seed: Option<SeedId>,
    pub name: String,
    pub provenance: ProvenanceId,
}


/// A bus selector for annotation surfaces (`@effects(publish:)`,
/// `@effects(depends:)`) — review rounds 16/17/18. The entry list
/// admits identifiers AND string literals, and the parser collapses
/// both to a plain string — so a spelling containing `::` can be an
/// alias path OR a literal wire subject (`"audit::log"`), and the
/// syntax cannot be recovered from the AST. The authoritative
/// evaluator never needs to: `topic_ref_matches` tests exact string
/// equality first and trailing-name equality second for EVERY
/// spelling, qualified or not. The selector therefore has exactly
/// one shape — the candidate sets that rule produces over declared
/// TOPICS and wire SUBJECTS, sorted. A literal wire subject lands
/// in `subjects`; two same-tailed imports both land in `topics`
/// ("the permissiveness the author asked for"); both empty =
/// resolves to nothing (Change 5's residue).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct BusSelector {
    pub name: String,
    pub topics: Vec<TopicId>,
    pub subjects: Vec<SubjectId>,
    pub provenance: ProvenanceId,
}

/// An effect class reference. The class universe is TYPED
/// (review round 15): a built-in is language-fixed and always
/// valid; a user class resolves into `entities.effect_classes`,
/// whose row records declaration status and the normalized
/// composition — so an IR consumer can distinguish a declared
/// class from an interned typo and expand `effect io = {…}`
/// without the AST.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct EffectClassRef {
    /// `Some` iff the name resolves to a user-class table row.
    pub class: Option<EffectClassId>,
    /// `true` iff the name is a language built-in (`syscall`, …).
    pub builtin: bool,
    pub name: String,
    pub provenance: ProvenanceId,
}

/// A set expression in claim-argument position.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum SetIr {
    Group(GroupRef),
    /// `effects(money)` — declared carriers of the class.
    EffectCarriers(EffectClassRef),
}

/// One granted edge inside `only edges { … }`.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct GrantIr {
    /// `publish T` when true, `subscribe T` when false — both admit
    /// the same edge; the verb names the reviewable declaration.
    pub publish: bool,
    pub topic: TopicIrRef,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum CountCmpIr {
    Eq,
    Le,
    Ge,
}

/// A quantitative budget dimension (`@budget(<dim> = N)`).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
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
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
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
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
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
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
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
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct LoweringIssue {
    pub message: String,
    pub provenance: ProvenanceId,
    /// Which family's row this issue prevented, when one owns it —
    /// `None` for table-level LAW SELECTION (an unknown or cyclic
    /// constitution, a name declared twice, a group that resolves
    /// to nothing), which belongs to no single family.
    ///
    /// GH #476 Change 9: the two consumers of this table report
    /// different subsets, and they must not guess. The artifact
    /// carries every issue; the CHECK path reports only the ones no
    /// other engine already owns — law selection is its own
    /// authority, while an annotation-surface issue (an undeclared
    /// effect class, say) is reported by the effects engine, and
    /// emitting it here too would put the same message on screen
    /// twice.
    pub family: Option<JudgmentFamily>,
}

/// What LAW SELECTION concluded about one group declaration.
///
/// GH #476 Change 9, review round 2: this is carried, not inferred.
/// The judgment previously reconstructed "did selection accept this
/// group?" from the model's member count, and that predicate is not
/// the same question. A group whose member is misspelled resolves to
/// nothing, so `{ MissingWorker } may_be_empty` looked
/// intentionally empty; `{ Worker, MissingWorker }` looked resolved
/// because one member survived; and a name declared twice looked
/// fine because the model keeps the LAST declaration while selection
/// keeps the first. Each of those judges a law against a domain
/// selection refused — the same two-answers defect the change set
/// out to close, one layer down.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum GroupSelection {
    /// Every selector resolved and the group has members.
    Resolved,
    /// Every selector resolved, the group is empty, and it declared
    /// `may_be_empty` — the author's stated intent, and the ONLY
    /// case where a law over an empty domain may hold vacuously.
    IntentionallyEmpty,
    /// At least one selector named something that does not exist.
    /// `may_be_empty` does not rescue this: it authorizes an
    /// intentionally empty group, not a misspelled member.
    SelectorFailed,
    /// The declaration itself was refused — declared more than once,
    /// or empty without declaring it may be.
    Refused,
}

impl GroupSelection {
    /// May a law quantifying over this group be judged at all?
    pub fn is_judgable(self) -> bool {
        matches!(
            self,
            GroupSelection::Resolved
                | GroupSelection::IntentionallyEmpty
        )
    }

    pub fn as_str(self) -> &'static str {
        match self {
            GroupSelection::Resolved => "resolved",
            GroupSelection::IntentionallyEmpty => "intentionally_empty",
            GroupSelection::SelectorFailed => "selector_failed",
            GroupSelection::Refused => "refused",
        }
    }
}

/// The lowered law table for one application (or one plan).
#[derive(Clone, Debug, Default)]
pub struct ClaimIrTable {
    pub rows: Vec<ClaimRow>,
    /// Law-selection invalidity (see [`LoweringIssue`]).
    pub issues: Vec<LoweringIssue>,
    /// What selection concluded about each declared group, by RAW
    /// group name — the identity selection itself keys on. A law
    /// referencing a group that is not judgable is `Invalid`; see
    /// [`GroupSelection`].
    pub group_selection:
        std::collections::BTreeMap<String, GroupSelection>,
    /// Row provenance — its OWN table (the laws are judged WITH the
    /// model, not stored in it; sharing the model's interner would
    /// force the lowering to mutate a finished model).
    pub provenance: ProvenanceTable,
}

/// The judgment family that owns a lowered row — the unit at which
/// Change-5 migration, artifact adequacy, and the corpus
/// differentials are organized (GH #476 Change 6).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum JudgmentFamily {
    /// `forbid reaches` (Change 5a).
    Reachability,
    /// `only edges` (Change 5b).
    Boundary,
    /// `require` / `cover` / `count` / `sealed` / `attributed`
    /// (Change 5c).
    Endpoint,
    /// `bound` (Change 5d).
    Bound,
    /// The pointwise `@effects` / `@no_panic` / `@phase_effects`
    /// certificates (Change 5e).
    Certificate,
    /// `@effects(causes: …)` — the cross-actor causal surface
    /// (Change 5f).
    Causes,
    /// Lowered but its engine has not migrated (`depends:`,
    /// `@budget`) — judged at minimum `uncertified`.
    Unmigrated,
    /// Fleet plan rows — Change 7's `FleetModel`.
    Fleet,
}

impl JudgmentFamily {
    pub fn as_str(self) -> &'static str {
        match self {
            JudgmentFamily::Reachability => "reachability",
            JudgmentFamily::Boundary => "boundary",
            JudgmentFamily::Endpoint => "endpoint",
            JudgmentFamily::Bound => "bound",
            JudgmentFamily::Certificate => "certificate",
            JudgmentFamily::Causes => "causes",
            JudgmentFamily::Unmigrated => "unmigrated",
            JudgmentFamily::Fleet => "fleet",
        }
    }

    /// The relation families this judgment's projection consumes —
    /// the adequacy question "can this model support family X
    /// exactly?" is `capabilities` vouching for every one of these
    /// (GH #476 Change 6). Mirrors what the Change-5 engines
    /// actually read; extending an engine's reads extends this row
    /// in the same change.
    pub fn required_relations(self) -> crate::hole::RelationSet {
        use crate::hole::RelationSet as R;
        match self {
            JudgmentFamily::Reachability
            | JudgmentFamily::Boundary
            | JudgmentFamily::Bound => R::CALLS
                .union(R::PUBLISHES)
                .union(R::SUBSCRIBES)
                .union(R::EFFECTS),
            JudgmentFamily::Endpoint => R::PUBLISHES
                .union(R::SUBSCRIBES)
                .union(R::CARDINALITY)
                .union(R::EFFECTS),
            // PUBLISHES joins CALLS + EFFECTS (round 1): the
            // `@effects(publish: {…})` certificate walks publish
            // sites, and a computed subject is exactly the
            // unresolved knowledge that makes the family inexact.
            JudgmentFamily::Certificate => R::CALLS
                .union(R::EFFECTS)
                .union(R::PUBLISHES),
            // `causes` walks CALLS for publish sites, PUBLISHES and
            // DELIVERY to reach handlers, EFFECTS to say what they do.
            JudgmentFamily::Causes => R::CALLS
                .union(R::PUBLISHES)
                .union(R::DELIVERY)
                .union(R::EFFECTS),
            JudgmentFamily::Unmigrated | JudgmentFamily::Fleet => {
                crate::hole::RelationSet(0)
            }
        }
    }
}

/// Minimal JSON string escaping for the dep-free payload renderer.
fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32))
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

impl ClaimRow {
    /// The TYPED law payload (GH #476 Change 6): one tagged JSON
    /// object per `ClaimIr` variant carrying the law's operands —
    /// what a consumer reads instead of parsing the rendered form
    /// string. References render as `{"name": raw, "display",
    /// "resolved": bool}`; class references as `{"class",
    /// "builtin", "resolved"}`; bus SELECTORS render their
    /// resolved CANDIDATE sets — the canonical topic names and
    /// wire-subject patterns the selector matched (round 2: the
    /// candidate sets ARE the selector's meaning; a consumer must
    /// never re-derive the compiler's matching rule) — plus the
    /// selector's own source location. One authority beside
    /// [`ClaimRow::claims_form`] — the artifact serializes this
    /// verbatim, and Track A's claim view consumes it.
    pub fn law_payload_json(
        &self,
        e: &crate::application::Entities,
        prov: &crate::provenance::ProvenanceTable,
    ) -> String {
        let loc0 = |pid: ProvenanceId| -> String {
            match prov.records.get(pid.index()) {
                Some(crate::provenance::Provenance::Source {
                    source,
                    span,
                }) => match prov.sources.get(source.index()) {
                    Some(su) => format!(
                        ", \"file\": {}, \"span\": [{}, {}]",
                        json_str(&su.path),
                        span.0,
                        span.1
                    ),
                    None => String::new(),
                },
                _ => String::new(),
            }
        };
        // EVERY operand reference is lossless (round 4): canonical
        // identity, author spelling, resolution status, AND its own
        // provenance — a consumer can reconstruct the typed law the
        // semantic digest hashed.
        let name_ref = |n: &NameRef,
                        resolved: bool,
                        pid: ProvenanceId|
         -> String {
            format!(
                "{{\"name\": {}, \"display\": {}, \"resolved\": {}{}}}",
                json_str(&n.raw),
                json_str(&n.display),
                resolved,
                loc0(pid)
            )
        };
        let gref = |g: &GroupRef| -> String {
            name_ref(&g.name, g.group.is_some(), g.provenance)
        };
        let tref = |t: &TopicIrRef| -> String {
            name_ref(&t.name, t.topic.is_some(), t.provenance)
        };
        let phref = |p2: &PhaseIrRef| -> String {
            name_ref(
                &NameRef {
                    raw: p2.name.clone(),
                    display: p2.name.clone(),
                },
                p2.phase.is_some(),
                p2.provenance,
            )
        };
        let sref = |sd: &SeedIrRef| -> String {
            name_ref(
                &NameRef {
                    raw: sd.name.clone(),
                    display: sd.name.clone(),
                },
                sd.seed.is_some(),
                sd.provenance,
            )
        };
        let cref = |c: &EffectClassRef| -> String {
            format!(
                "{{\"class\": {}, \"builtin\": {}, \"resolved\": {}{}}}",
                json_str(&c.name),
                c.builtin,
                c.builtin || c.class.is_some(),
                loc0(c.provenance)
            )
        };
        let crefs = |cs: &[EffectClassRef]| -> String {
            let v: Vec<String> = cs.iter().map(&cref).collect();
            format!("[{}]", v.join(", "))
        };
        let fnref = |at: &(Option<FunctionId>, NameRef)| -> String {
            // Annotation subjects anchor at the row's own
            // provenance (the lowering interns the decl name span
            // on the row).
            format!(
                "{{\"name\": {}, \"display\": {}, \"resolved\": {}}}",
                json_str(&at.1.raw),
                json_str(&at.1.display),
                at.0.is_some()
            )
        };
        let locusref =
            |at: &(Option<LocusDeclId>, NameRef)| -> String {
                format!(
                    "{{\"name\": {}, \"display\": {}, \"resolved\": {}}}",
                    json_str(&at.1.raw),
                    json_str(&at.1.display),
                    at.0.is_some()
                )
            };
        let set = |s: &SetIr| -> String {
            match s {
                SetIr::Group(g) => {
                    format!("{{\"group\": {}}}", gref(g))
                }
                SetIr::EffectCarriers(c) => {
                    format!("{{\"effects\": {}}}", cref(c))
                }
            }
        };
        let loc = |pid: ProvenanceId| -> String {
            match prov.records.get(pid.index()) {
                Some(crate::provenance::Provenance::Source {
                    source,
                    span,
                }) => match prov.sources.get(source.index()) {
                    Some(su) => format!(
                        ", \"file\": {}, \"span\": [{}, {}]",
                        json_str(&su.path),
                        span.0,
                        span.1
                    ),
                    None => String::new(),
                },
                _ => String::new(),
            }
        };
        let selector = |b: &BusSelector| -> String {
            let topics: Vec<String> = b
                .topics
                .iter()
                .filter_map(|t| e.topics.get(t.index()))
                .map(|t| {
                    format!(
                        "{{\"name\": {}, \"display\": {}}}",
                        json_str(&t.name),
                        json_str(&t.display)
                    )
                })
                .collect();
            let subjects: Vec<String> = b
                .subjects
                .iter()
                .filter_map(|sid| e.subjects.get(sid.index()))
                .map(|su| json_str(&su.pattern))
                .collect();
            format!(
                "{{\"name\": {}, \"topics\": [{}], \
                 \"subjects\": [{}]{}}}",
                json_str(&b.name),
                topics.join(", "),
                subjects.join(", "),
                loc(b.provenance)
            )
        };
        match &self.law {
            ClaimIr::ForbidReaches {
                src,
                dst,
                via_calls,
                via_bus,
                during,
                avoiding,
            } => {
                let mut via: Vec<&str> = Vec::new();
                if *via_calls {
                    via.push("\"calls\"");
                }
                if *via_bus {
                    via.push("\"bus\"");
                }
                let mut out = format!(
                    "{{\"kind\": \"forbid_reaches\", \"src\": {}, \
                     \"dst\": {}, \"via\": [{}]",
                    set(src),
                    set(dst),
                    via.join(", ")
                );
                if let Some(p) = during {
                    out.push_str(&format!(
                        ", \"during\": {}",
                        phref(p)
                    ));
                }
                if let Some(a) = avoiding {
                    out.push_str(&format!(
                        ", \"avoiding\": {}",
                        gref(a)
                    ));
                }
                out.push('}');
                out
            }
            ClaimIr::OnlyEdges { src, dst, grants } => {
                let gs: Vec<String> = grants
                    .iter()
                    .map(|g| {
                        format!(
                            "{{\"verb\": {}, \"topic\": {}}}",
                            json_str(if g.publish {
                                "publish"
                            } else {
                                "subscribe"
                            }),
                            tref(&g.topic)
                        )
                    })
                    .collect();
                format!(
                    "{{\"kind\": \"only_edges\", \"src\": {}, \
                     \"dst\": {}, \"grants\": [{}]}}",
                    gref(src),
                    gref(dst),
                    gs.join(", ")
                )
            }
            ClaimIr::Bound { class, limit, from } => format!(
                "{{\"kind\": \"bound\", \"class\": {}, \
                 \"limit\": {}, \"from\": {}}}",
                cref(class),
                limit,
                gref(from)
            ),
            ClaimIr::RequireEndpoint {
                publishers,
                group,
                topic,
            } => format!(
                "{{\"kind\": \"require_endpoint\", \
                 \"publishers\": {}, \"group\": {}, \
                 \"topic\": {}}}",
                publishers,
                gref(group),
                tref(topic)
            ),
            ClaimIr::RequireSealed { group } => format!(
                "{{\"kind\": \"require_sealed\", \"group\": {}}}",
                gref(group)
            ),
            ClaimIr::RequireAttributed { class } => format!(
                "{{\"kind\": \"require_attributed\", \
                 \"class\": {}}}",
                cref(class)
            ),
            ClaimIr::Cover { seed, group } => format!(
                "{{\"kind\": \"cover\", \"seed\": {}, \
                 \"group\": {}}}",
                sref(seed),
                gref(group)
            ),
            ClaimIr::Count {
                publishers,
                topic,
                cmp,
                n,
            } => format!(
                "{{\"kind\": \"count\", \"publishers\": {}, \
                 \"topic\": {}, \"cmp\": {}, \"n\": {}}}",
                publishers,
                tref(topic),
                json_str(match cmp {
                    CountCmpIr::Eq => "==",
                    CountCmpIr::Le => "<=",
                    CountCmpIr::Ge => ">=",
                }),
                n
            ),
            ClaimIr::EffectForbid { at, classes } => format!(
                "{{\"kind\": \"effect_forbid\", \"at\": {}, \
                 \"classes\": {}}}",
                fnref(at),
                crefs(classes)
            ),
            ClaimIr::EffectOnly { at, classes } => format!(
                "{{\"kind\": \"effect_only\", \"at\": {}, \
                 \"classes\": {}}}",
                fnref(at),
                crefs(classes)
            ),
            ClaimIr::EffectPublishSet { at, entries } => {
                let es: Vec<String> =
                    entries.iter().map(&selector).collect();
                format!(
                    "{{\"kind\": \"effect_publish_set\", \
                     \"at\": {}, \"entries\": [{}]}}",
                    fnref(at),
                    es.join(", ")
                )
            }
            ClaimIr::EffectCauses { at, classes } => format!(
                "{{\"kind\": \"effect_causes\", \"at\": {}, \
                 \"classes\": {}}}",
                fnref(at),
                crefs(classes)
            ),
            ClaimIr::NoPanic { at } => format!(
                "{{\"kind\": \"no_panic\", \"at\": {}}}",
                fnref(at)
            ),
            ClaimIr::DependsSet { locus, entries } => {
                let es: Vec<String> =
                    entries.iter().map(&selector).collect();
                format!(
                    "{{\"kind\": \"depends_set\", \
                     \"locus\": {}, \"entries\": [{}]}}",
                    locusref(locus),
                    es.join(", ")
                )
            }
            ClaimIr::PhaseEffects { locus, phases } => {
                let ps: Vec<String> = phases
                    .iter()
                    .map(|(ph, allowed)| {
                        format!(
                            "{{\"phase\": {}, \"allowed\": {}}}",
                            json_str(ph),
                            crefs(allowed)
                        )
                    })
                    .collect();
                format!(
                    "{{\"kind\": \"phase_effects\", \
                     \"locus\": {}, \"phases\": [{}]}}",
                    locusref(locus),
                    ps.join(", ")
                )
            }
            ClaimIr::AllocBudget { at, per_call } => format!(
                "{{\"kind\": \"alloc_budget\", \"at\": {}, \
                 \"per_call\": {}}}",
                fnref(at),
                per_call
            ),
            ClaimIr::QuantBudget { at, dim, limit } => {
                // The dimension is TYPED (round 4): a builtin is a
                // closed tag; a user class is a full class
                // reference with its resolution state.
                let dim_json = match dim {
                    QuantDimIr::StackBytes => {
                        "{\"builtin\": \"stack_bytes\"}".to_string()
                    }
                    QuantDimIr::BlockPoints => {
                        "{\"builtin\": \"block_points\"}".to_string()
                    }
                    QuantDimIr::Publish => {
                        "{\"builtin\": \"publish\"}".to_string()
                    }
                    QuantDimIr::Fanout => {
                        "{\"builtin\": \"fanout\"}".to_string()
                    }
                    QuantDimIr::UserClass(c) => {
                        format!("{{\"user_class\": {}}}", cref(c))
                    }
                };
                format!(
                    "{{\"kind\": \"quant_budget\", \"at\": {}, \
                     \"dim\": {}, \"limit\": {}}}",
                    fnref(at),
                    dim_json,
                    limit
                )
            }
            ClaimIr::FleetForbidReaches { from, to, avoiding } => {
                let av = match avoiding {
                    Some(a) => format!(
                        ", \"avoiding\": {}",
                        json_str(a)
                    ),
                    None => String::new(),
                };
                format!(
                    "{{\"kind\": \"fleet_forbid_reaches\", \
                     \"from\": {}, \"to\": {}{}}}",
                    json_str(from),
                    json_str(to),
                    av
                )
            }
            ClaimIr::FleetOnlyEdges { src, dst, grants } => {
                let gs: Vec<String> =
                    grants.iter().map(|g| json_str(g)).collect();
                format!(
                    "{{\"kind\": \"fleet_only_edges\", \
                     \"src\": {}, \"dst\": {}, \
                     \"grants\": [{}]}}",
                    json_str(src),
                    json_str(dst),
                    gs.join(", ")
                )
            }
            ClaimIr::FleetRequireEndpoint {
                publishers,
                target,
                topic,
            } => format!(
                "{{\"kind\": \"fleet_require_endpoint\", \
                 \"publishers\": {}, \"target\": {}, \
                 \"topic\": {}}}",
                publishers,
                json_str(target),
                json_str(topic)
            ),
            ClaimIr::FleetCountInstances {
                publishers,
                topic,
                eq,
                max,
                min,
            } => {
                let mut out = format!(
                    "{{\"kind\": \"fleet_count_instances\", \
                     \"publishers\": {}, \"topic\": {}",
                    publishers,
                    json_str(topic)
                );
                if let Some(n) = eq {
                    out.push_str(&format!(", \"eq\": {}", n));
                }
                if let Some(n) = max {
                    out.push_str(&format!(", \"max\": {}", n));
                }
                if let Some(n) = min {
                    out.push_str(&format!(", \"min\": {}", n));
                }
                out.push('}');
                out
            }
        }
    }

    /// Which judgment family owns this row.
    pub fn family(&self) -> JudgmentFamily {
        match &self.law {
            ClaimIr::ForbidReaches { .. } => {
                JudgmentFamily::Reachability
            }
            ClaimIr::OnlyEdges { .. } => JudgmentFamily::Boundary,
            ClaimIr::RequireEndpoint { .. }
            | ClaimIr::RequireSealed { .. }
            | ClaimIr::RequireAttributed { .. }
            | ClaimIr::Cover { .. }
            | ClaimIr::Count { .. } => JudgmentFamily::Endpoint,
            ClaimIr::Bound { .. } => JudgmentFamily::Bound,
            ClaimIr::EffectForbid { .. }
            | ClaimIr::EffectOnly { .. }
            | ClaimIr::EffectPublishSet { .. }
            | ClaimIr::NoPanic { .. }
            | ClaimIr::PhaseEffects { .. } => {
                JudgmentFamily::Certificate
            }
            ClaimIr::EffectCauses { .. } => JudgmentFamily::Causes,
            ClaimIr::DependsSet { .. }
            | ClaimIr::AllocBudget { .. }
            | ClaimIr::QuantBudget { .. } => {
                JudgmentFamily::Unmigrated
            }
            ClaimIr::FleetForbidReaches { .. }
            | ClaimIr::FleetOnlyEdges { .. }
            | ClaimIr::FleetRequireEndpoint { .. }
            | ClaimIr::FleetCountInstances { .. } => JudgmentFamily::Fleet,
        }
    }

    /// The claims-block form rendering — display voice, matching
    /// the evaluator's `render_form` spelling byte-for-byte (the
    /// artifact projection differential is the gate). `None` for
    /// rows that are not claims-block forms (annotations render via
    /// [`ClaimRow::certificate_forms`]; fleet rows via Change 7).
    pub fn claims_form(&self) -> Option<String> {
        let set = |s: &SetIr| -> String {
            match s {
                SetIr::Group(g) => g.name.display.clone(),
                SetIr::EffectCarriers(c) => {
                    format!("effects({})", c.name)
                }
            }
        };
        let cmp = |c: &CountCmpIr| match c {
            CountCmpIr::Eq => "==",
            CountCmpIr::Le => "<=",
            CountCmpIr::Ge => ">=",
        };
        Some(match &self.law {
            ClaimIr::RequireSealed { group } => format!(
                "require sealed(all {})",
                group.name.display
            ),
            ClaimIr::RequireAttributed { class } => format!(
                "require attributed(all {})",
                class.name
            ),
            ClaimIr::ForbidReaches {
                src,
                dst,
                via_calls,
                via_bus,
                during,
                avoiding,
            } => {
                let mut out = format!(
                    "forbid reaches({}, {})",
                    set(src),
                    set(dst)
                );
                match (via_calls, via_bus) {
                    (true, true) => {}
                    (true, false) => {
                        out.push_str(" via { calls }")
                    }
                    (false, true) => out.push_str(" via { bus }"),
                    (false, false) => {}
                }
                if let Some(p) = during {
                    out.push_str(&format!(
                        " during {}",
                        p.name
                    ));
                }
                if let Some(a) = avoiding {
                    out.push_str(&format!(
                        " avoiding {}",
                        a.name.display
                    ));
                }
                out
            }
            ClaimIr::OnlyEdges { src, dst, grants } => {
                let gs: Vec<String> = grants
                    .iter()
                    .map(|g| {
                        format!(
                            "{} {}",
                            if g.publish {
                                "publish"
                            } else {
                                "subscribe"
                            },
                            g.topic.name.display
                        )
                    })
                    .collect();
                format!(
                    "only edges {} -> {} {{ {} }}",
                    src.name.display,
                    dst.name.display,
                    gs.join("; ")
                )
            }
            ClaimIr::Bound { class, limit, from } => format!(
                "bound {} <= {} on paths from {}",
                class.name, limit, from.name.display
            ),
            ClaimIr::RequireEndpoint {
                publishers,
                group,
                topic,
            } => format!(
                "require {}(some {}, topic {})",
                if *publishers {
                    "publishes"
                } else {
                    "subscribes"
                },
                group.name.display,
                topic.name.display
            ),
            ClaimIr::Cover { seed, group } => format!(
                "cover topic in seed({}): subscribed_by(some {})",
                seed.name, group.name.display
            ),
            ClaimIr::Count {
                publishers,
                topic,
                cmp: c,
                n,
            } => format!(
                "count {}(topic {}) {} {}",
                if *publishers {
                    "publishers"
                } else {
                    "subscribers"
                },
                topic.name.display,
                cmp(c),
                n
            ),
            _ => return None,
        })
    }

    /// The certificate forms this row's engines produce, in
    /// generation order — `(subject display, form display)` pairs.
    /// One authority for three consumers: the evidence PRODUCER
    /// matches engine rows against these strings, `EvidenceTable::
    /// validate` requires each evidence row's certificates to agree
    /// with them (binding the certificate payload to the exact law
    /// it answers — review round 3: a table-wide digest cannot see
    /// two same-subject rows exchanging their certs), and a
    /// renderer may display them.
    /// The compatibility `lowered` form a BUDGET row generates —
    /// the SAME spelling the legacy producers emit, so the emitter
    /// can key each legacy evidence row to its law ordinal
    /// (round 6) and admission can re-render it from the typed
    /// operands.
    pub fn budget_lowered_form(&self) -> Option<String> {
        match &self.law {
            ClaimIr::AllocBudget { at, per_call } => Some(format!(
                "bound alloc <= {} on paths from {{{}}}",
                per_call, at.1.display
            )),
            ClaimIr::QuantBudget { at, dim, limit } => {
                let d = match dim {
                    QuantDimIr::StackBytes => "stack_bytes".to_string(),
                    QuantDimIr::BlockPoints => "block_points".to_string(),
                    QuantDimIr::Publish => "publish".to_string(),
                    QuantDimIr::Fanout => "fanout".to_string(),
                    QuantDimIr::UserClass(c) => c.name.clone(),
                };
                Some(format!(
                    "bound {} <= {} on paths from {{{}}}",
                    d, limit, at.1.display
                ))
            }
            _ => None,
        }
    }

    /// Round 6: the typed LEGACY-report fingerprint for the
    /// unmigrated non-budget families (`causes:` / `depends:`) —
    /// rendered from the typed operands, so the `law.legacy` entry
    /// binds the imported old-engine verdict to the EXACT law in
    /// the row (an operand mutation changes the fingerprint and
    /// orphans the report entry).
    pub fn legacy_form(&self) -> Option<String> {
        match &self.law {
            ClaimIr::EffectCauses { at, classes } => {
                let cs: Vec<&str> = classes
                    .iter()
                    .map(|c| c.name.as_str())
                    .collect();
                Some(format!(
                    "causes {{{}}} from {{{}}}",
                    cs.join(", "),
                    at.1.display
                ))
            }
            ClaimIr::DependsSet { locus, entries } => {
                let es: Vec<&str> = entries
                    .iter()
                    .map(|b| b.name.as_str())
                    .collect();
                Some(format!(
                    "depends {{{}}} on {{{}}}",
                    es.join(", "),
                    locus.1.display
                ))
            }
            _ => None,
        }
    }

    pub fn certificate_forms(&self) -> Vec<(String, String)> {
        let subject_disp =
            |at: &(Option<FunctionId>, NameRef)| at.1.display.clone();
        match &self.law {
            ClaimIr::EffectForbid { at, classes } => classes
                .iter()
                .map(|c| {
                    (
                        subject_disp(at),
                        format!(
                            "forbid reaches({{{}}}, effects({}))",
                            at.1.display, c.name
                        ),
                    )
                })
                .collect(),
            ClaimIr::EffectOnly { at, classes } => {
                let set = classes
                    .iter()
                    .map(|c| c.name.clone())
                    .collect::<Vec<_>>()
                    .join(", ");
                vec![(
                    subject_disp(at),
                    format!(
                        "only effects {{{}}} on {{{}}}",
                        set, at.1.display
                    ),
                )]
            }
            ClaimIr::EffectPublishSet { at, entries } => {
                let allowed = entries
                    .iter()
                    .map(|s| s.name.clone())
                    .collect::<Vec<_>>()
                    .join(", ");
                vec![(
                    subject_disp(at),
                    format!(
                        "only publishes {{{}}} from {{{}}}",
                        allowed, at.1.display
                    ),
                )]
            }
            ClaimIr::NoPanic { at } => vec![(
                subject_disp(at),
                format!(
                    "forbid reaches({{{}}}, panic)",
                    at.1.display
                ),
            )],
            ClaimIr::PhaseEffects { locus, phases } => phases
                .iter()
                .map(|(phase, allowed)| {
                    let set = allowed
                        .iter()
                        .map(|c| c.name.clone())
                        .collect::<Vec<_>>()
                        .join(", ");
                    (
                        locus.1.display.clone(),
                        format!(
                            "only effects {{{}}} on {{{}}} during {}",
                            set, locus.1.display, phase
                        ),
                    )
                })
                .collect(),
            _ => Vec::new(),
        }
    }
}

impl ClaimIrTable {
    /// A canonical semantic digest of the complete law table —
    /// FNV-1a/64 over every row's `Hash` image (ordinal, name,
    /// origin, and the full typed law with its references). The
    /// evidence sidecar records it at derivation and a judgment
    /// refuses evidence whose digest disagrees with the table it is
    /// asked to judge: `TopologyShapeV1` hashes the topology model
    /// half, NOT annotation laws, so two programs with identical
    /// topology but different `@effects` classes would otherwise
    /// accept each other's evidence (review round 2).
    ///
    /// Stability: this uses the crate's own FNV-1a hasher, not
    /// `DefaultHasher` (whose algorithm is unspecified and may
    /// change between Rust releases).
    pub fn semantic_digest(&self) -> u64 {
        use core::hash::{Hash, Hasher};
        struct Fnv64(u64);
        impl Hasher for Fnv64 {
            fn finish(&self) -> u64 {
                self.0
            }
            fn write(&mut self, bytes: &[u8]) {
                for b in bytes {
                    self.0 ^= u64::from(*b);
                    self.0 = self.0.wrapping_mul(0x100_0000_01b3);
                }
            }
        }
        let mut h = Fnv64(0xcbf2_9ce4_8422_2325);
        (self.rows.len() as u64).hash(&mut h);
        for row in &self.rows {
            row.ordinal.hash(&mut h);
            row.name.hash(&mut h);
            row.origin.hash(&mut h);
            row.law.hash(&mut h);
            row.provenance.hash(&mut h);
        }
        // The law's identity includes its provenance STORE (review
        // round 4): rows reference records by numeric id, so a
        // re-lowered table whose spans moved would otherwise share
        // a digest with the original and render stale offsets
        // against the new bases.
        (self.provenance.sources.len() as u64).hash(&mut h);
        for su in &self.provenance.sources {
            su.hash(&mut h);
        }
        (self.provenance.records.len() as u64).hash(&mut h);
        for rec in &self.provenance.records {
            rec.hash(&mut h);
        }
        h.finish()
    }
}

/// A violated `ClaimIr` law.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
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
    /// A provenance record's contents are unresolvable: a dangling
    /// `SourceId` or an inverted span.
    InvalidProvenanceRecord { index: usize },
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
        // The records themselves must resolve — the same boundary
        // ApplicationModel::validate draws (review round 19): a
        // dangling SourceId or inverted span is a primary
        // diagnostic location Change 5 could not resolve without
        // reopening the AST.
        let src_len = self.provenance.sources.len();
        for (i, r) in self.provenance.records.iter().enumerate() {
            match r {
                crate::provenance::Provenance::Source {
                    source,
                    span,
                } => {
                    if source.index() >= src_len
                        || span.0 > span.1
                    {
                        return Err(
                            ClaimIrError::InvalidProvenanceRecord {
                                index: i,
                            },
                        );
                    }
                }
                crate::provenance::Provenance::ForeignSpan {
                    span,
                } => {
                    if span.0 > span.1 {
                        return Err(
                            ClaimIrError::InvalidProvenanceRecord {
                                index: i,
                            },
                        );
                    }
                }
                crate::provenance::Provenance::Synthetic {
                    ..
                } => {}
            }
        }
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
                    // The discriminator is DERIVED from the name:
                    // `builtin` must agree with the canonical
                    // vocabulary exactly (review round 20) — a
                    // judgment branching on the flag and a renderer
                    // reading the name must describe one law.
                    if c.builtin != is_builtin_effect_class(&c.name) {
                        return Err(dis("effect class builtin"));
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
                    if !pr(sel.provenance) {
                        return Err(ClaimIrError::DanglingProvenance {
                            index: i,
                        });
                    }
                    // The candidate sets are DERIVED data: exactly
                    // what `bus_ref_matches(name, …)` produces over
                    // the model's topic and subject tables — an
                    // unrelated candidate widens the law a renderer
                    // reads from `name`, an omitted same-tailed
                    // candidate silently narrows it (review
                    // round 19; the same machine/display-agreement
                    // doctrine as NameDisagreement).
                    let want_topics: Vec<TopicId> = e
                        .topics
                        .iter()
                        .enumerate()
                        .filter(|(_, t)| {
                            bus_ref_matches(&sel.name, &t.name)
                        })
                        .map(|(k, _)| TopicId(k as u32))
                        .collect();
                    let want_subjects: Vec<SubjectId> = e
                        .subjects
                        .iter()
                        .enumerate()
                        .filter(|(_, su)| {
                            bus_ref_matches(&sel.name, &su.pattern)
                        })
                        .map(|(k, _)| SubjectId(k as u32))
                        .collect();
                    if sel.topics != want_topics
                        || sel.subjects != want_subjects
                    {
                        return Err(dis("bus selector"));
                    }
                    Ok(())
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
                        // The variant specifically means a USER
                        // class budget (built-ins have their own
                        // dimensions).
                        if c.builtin {
                            return Err(dis(
                                "quant budget user class",
                            ));
                        }
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

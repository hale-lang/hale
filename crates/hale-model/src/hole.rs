//! Typed unresolved residue — a model is known facts PLUS an
//! explicit account of what it does not know.
//!
//! The verdict law these feed: a judgment whose relevant projection
//! can reach a hole that hides a relation family it needs returns
//! `uncertified`, never `holds`. Holes are rows, not caller policy —
//! one evaluator cannot treat an unknown as fatal while another
//! forgets it.
//!
//! Composition law (fleet, Change 7): holes never disappear when
//! models compose.

use crate::ids::{EntityRef, ProvenanceId};

/// The relation families a hole can hide. A tiny fixed bitset —
/// adding a family is a schema change with review, provenance rules,
/// and a canary, exactly like adding a relation table.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RelationSet(pub u32);

impl RelationSet {
    pub const CALLS: RelationSet = RelationSet(1 << 0);
    pub const PUBLISHES: RelationSet = RelationSet(1 << 1);
    pub const SUBSCRIBES: RelationSet = RelationSet(1 << 2);
    pub const OWNS: RelationSet = RelationSet(1 << 3);
    pub const PLACED: RelationSet = RelationSet(1 << 4);
    pub const BINDS: RelationSet = RelationSet(1 << 5);
    pub const SUPERVISES: RelationSet = RelationSet(1 << 6);
    pub const EFFECTS: RelationSet = RelationSet(1 << 7);
    pub const ROUTES: RelationSet = RelationSet(1 << 8);
    /// Key-filter knowledge: hidden by unknown key domains or
    /// predicates. Every inline `KeyDomain::Unknown` /
    /// `KeyPredicate::Unknown` REQUIRES a hole hiding this family
    /// (validated) — an unknown may not hide solely inside an
    /// otherwise resolved row.
    pub const KEY_FILTERS: RelationSet = RelationSet(1 << 9);
    /// Cardinality knowledge (instance/publisher/subscriber counts).
    pub const CARDINALITY: RelationSet = RelationSet(1 << 10);
    /// Delivery-guarantee knowledge (the must-deliver side).
    pub const DELIVERY: RelationSet = RelationSet(1 << 11);

    /// Every DEFINED family bit. A hides mask outside this set
    /// names a family no judgment or capability knows — accepting
    /// it would create a valid-but-invisible hole and defeat the
    /// rule that adding a family is a reviewed schema change
    /// (round 7).
    pub const ALL_KNOWN: RelationSet = RelationSet((1 << 12) - 1);

    pub const fn union(self, other: RelationSet) -> RelationSet {
        RelationSet(self.0 | other.0)
    }
    pub const fn contains(self, other: RelationSet) -> bool {
        (self.0 & other.0) == other.0
    }
    pub const fn intersects(self, other: RelationSet) -> bool {
        (self.0 & other.0) != 0
    }
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

/// Why the model does not know.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum HoleKind {
    /// A call through a fn pointer / indirect site.
    IndirectCall,
    /// A method call on a receiver the summarizer cannot type. The
    /// dispatched method name is MACHINE data (the legacy artifact's
    /// `untyped_receiver_call:<callee>` row requires it), so it
    /// lives here in the kind — never parsed back out of the
    /// human-readable `reason` text (review round 11).
    UntypedReceiver { callee: String },
    /// A publish whose subject is computed at runtime.
    ComputedSubject,
    /// A keyed publish/subscription whose key values are unknown
    /// (the filter is preserved; its coverage is not provable).
    UnknownKeyDomain,
    /// An interface dispatch whose conformer set is open.
    OpenInterface,
    /// Placement inherited at runtime rather than declared.
    RuntimeInheritedPlacement,
    /// An endpoint that appears or disappears dynamically.
    DynamicEndpoint,
    /// A declared external boundary with opaque internals.
    ExternalOpaque,
    /// An artifact carried semantics this consumer does not
    /// implement (decode-side honesty: refuse to pretend).
    UnsupportedArtifactSemantics,
    /// A declared executable body the behavior analysis did not
    /// walk (module-scoped bodies, `on_failure` handlers at
    /// Change 2). The declaration EXISTS as an entity; its calls,
    /// publishes, and effects are unknown — this hole is what
    /// keeps `exact_calls`/`exact_effects` honest until the
    /// summary covers the body family.
    UnanalyzedBody,
}

/// One hole: where, why, at which authored position, and —
/// critically — which relation families
/// it hides from judgments.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Hole {
    pub at: EntityRef,
    pub kind: HoleKind,
    /// Never empty — a hole that hides nothing is not a hole
    /// (validated; see [`ApplicationModel::validate`]).
    ///
    /// [`ApplicationModel::validate`]: crate::application::ApplicationModel::validate
    pub hides: RelationSet,
    /// The authored site ordinal of the hole's FIRST occurrence, in
    /// its site space (call-site ordinals for call holes,
    /// publish-site ordinals for computed subjects). Judgments
    /// interleave holes with known edges by this position — the
    /// evaluator walks bodies in source order and halts at the
    /// first unfollowable edge, so position is verdict-relevant.
    pub authored_site: Option<u32>,
    /// Human-readable reason for witnesses ("call through fn param
    /// `f`").
    pub reason: String,
    pub provenance: ProvenanceId,
}

/// The CLOSED anchor/kind/family matrix (review rounds 7–9): for
/// each (anchor grain, kind) pair, the `(required, allowed)` family
/// masks — validation demands `required ⊆ hides ⊆ allowed`.
/// `None` means the combination is not a defined hole shape and
/// validation rejects it — a hole every judgment silently ignores
/// is worse than no hole, because the model then CLAIMS its
/// unknowns are accounted for. The REQUIRED mask exists because
/// judgments SELECT holes by family: an IndirectCall hiding only
/// EFFECTS would be invisible to call traversal while still
/// occupying its call site (round 9). For every fn-grain kind the
/// two masks are equal — Hale has no mechanism for proving the
/// effects of an unresolved call, so a call hole hides EFFECTS
/// too, always. Extending this matrix is a reviewed schema change,
/// made together with the judgment/BusHoles support that consumes
/// the new shape.
///
/// Site rule (validated beside this): only Function-anchored holes
/// carry `authored_site` — a set-level (subject/topic-grain) hole
/// has no authored position relative to known rows, which is
/// exactly why judgments DEFER on it instead of halting (round 6).
/// Site shape per fn-anchored kind (round 8): a SITE-SHAPED hole
/// stands for one authored call/publish expression and REQUIRES
/// its ordinal (judgments interleave it with known events — a
/// missing ordinal would invent an order); a whole-body /
/// declaration-grain hole has no single position and must not
/// carry one.
pub fn hole_site_shaped(kind: &HoleKind) -> bool {
    matches!(
        kind,
        HoleKind::IndirectCall
            | HoleKind::UntypedReceiver { .. }
            | HoleKind::OpenInterface
            | HoleKind::ComputedSubject
    )
}

pub fn allowed_hole_families(
    at: &EntityRef,
    kind: &HoleKind,
) -> Option<(RelationSet, RelationSet)> {
    use HoleKind as K;
    let c = RelationSet::CALLS;
    let p = RelationSet::PUBLISHES;
    let sub = RelationSet::SUBSCRIBES;
    let e = RelationSet::EFFECTS;
    match (at, kind) {
        // An unfollowable call also hides the EFFECTS beyond it —
        // the builder emits CALLS ∪ EFFECTS for the call-hole
        // kinds, the call walks select by CALLS, and the
        // effects(C)-destination scan consumes EFFECTS: both bits
        // are REQUIRED (rounds 2–3, 9).
        (EntityRef::Function(_), K::IndirectCall)
        | (EntityRef::Function(_), K::UntypedReceiver { .. })
        | (EntityRef::Function(_), K::OpenInterface) => {
            Some((c.union(e), c.union(e)))
        }
        (EntityRef::Function(_), K::ComputedSubject) => {
            Some((p, p))
        }
        (EntityRef::Function(_), K::UnknownKeyDomain) => Some((
            RelationSet::KEY_FILTERS,
            RelationSet::KEY_FILTERS,
        )),
        // No judgment consults fn-grain SUBSCRIBES holes (round
        // 8): subscription incompleteness is SET-level knowledge
        // (the subject's subscriber set), so the fn-grain shape
        // stops at what the engines consume — and an unanalyzed
        // body hides ALL of what it could contain (round 9).
        (EntityRef::Function(_), K::UnanalyzedBody) => {
            Some((c.union(p).union(e), c.union(p).union(e)))
        }
        // Set-level endpoint knowledge: each bit is independently
        // meaningful (publisher-incomplete vs
        // subscriber-incomplete are distinct facts), so nothing
        // beyond non-emptiness is required.
        (
            EntityRef::Subject(_) | EntityRef::Topic(_),
            K::DynamicEndpoint,
        ) => Some((
            RelationSet(0),
            p.union(sub)
                .union(RelationSet::CARDINALITY)
                .union(RelationSet::DELIVERY),
        )),
        (
            EntityRef::Subject(_) | EntityRef::Topic(_),
            K::UnknownKeyDomain,
        ) => Some((
            RelationSet(0),
            RelationSet::KEY_FILTERS.union(RelationSet::ROUTES),
        )),
        _ => None,
    }
}

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
    /// A method call on a receiver the summarizer cannot type.
    UntypedReceiver,
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
}

/// One hole: where, why, and — critically — which relation families
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
    /// Human-readable reason for witnesses ("call through fn param
    /// `f`").
    pub reason: String,
    pub provenance: ProvenanceId,
}

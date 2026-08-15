//! Relation tables — typed rows, each with its own fields, proof
//! obligations, and (later) witness rendering.
//!
//! Two laws:
//!
//! 1. **Resolved rows only.** An unresolved callee, computed
//!    subject, or dynamic endpoint is a [`Hole`], never a row with a
//!    stringly placeholder — the fail-closed vocabulary depends on
//!    unknowns being typed residue, not fake edges.
//! 2. **Facts, not conclusions.** Direct-call eligibility, static
//!    dispatch buckets, and placement specialization are lowering
//!    plans derived FROM these rows (Change 8); they never appear
//!    here as if authored.
//!
//! [`Hole`]: crate::hole::Hole

use crate::ids::{
    BindingId, FunctionId, LocusDeclId, LocusInstanceId, PhaseId,
    ProvenanceId, SeedId, ThreadDomainId, TopicId,
};
use crate::keys::{Capacity, KeyDomain, KeyPredicate, PublishDisposition, ShedPolicy};

/// `member_of(function, locus)`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MemberOf {
    pub function: FunctionId,
    pub locus: LocusDeclId,
    pub provenance: ProvenanceId,
}

/// `phase_of(function, phase)`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PhaseOf {
    pub function: FunctionId,
    pub phase: PhaseId,
    pub provenance: ProvenanceId,
}

/// `declared_in(entity, seed)`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DeclaredIn {
    pub entity: crate::ids::EntityRef,
    pub seed: SeedId,
    pub provenance: ProvenanceId,
}

/// `realizes(instance, locus_decl)`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Realizes {
    pub instance: LocusInstanceId,
    pub decl: LocusDeclId,
    pub provenance: ProvenanceId,
}

/// `owns(parent_instance, child_instance)` — the lifecycle tree.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Owns {
    pub parent: LocusInstanceId,
    pub child: LocusInstanceId,
    pub provenance: ProvenanceId,
}

/// How a resolved call dispatches. The *fact* of the mechanism —
/// which lowering flavor it gets (direct call, static bucket, local
/// queue) is a `DispatchPlan` conclusion, not a model row.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum DispatchKind {
    Direct,
    /// Dispatch through an interface, fanned to a conformer in the
    /// closed world. `interface` is the contract's canonical name;
    /// one authored site yields one row per conformer.
    Interface { interface: String },
    /// A contracted through-stdlib path: the artifact deliberately
    /// hides stdlib interiors, but the endpoints are exact.
    ViaStdlib,
}

/// `calls(caller, callee)` — resolved calls only (law 1 above).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Call {
    pub from: FunctionId,
    pub to: FunctionId,
    pub dispatch: DispatchKind,
    /// The site sits inside a loop (quantitative judgments weight
    /// it unbounded-per-activation unless bounded elsewhere).
    pub in_loop: bool,
    pub provenance: ProvenanceId,
}

/// `publishes(function, topic)` with the site's key domain and
/// declared loss disposition.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Publish {
    pub function: FunctionId,
    pub topic: TopicId,
    pub key_domain: KeyDomain,
    pub disposition: PublishDisposition,
    pub provenance: ProvenanceId,
}

/// `subscribes(topic, handler)` with the subscription's key
/// predicate and bounds — the raw facts `may_deliver`/`must_deliver`
/// derive from.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Subscribe {
    pub topic: TopicId,
    pub handler: FunctionId,
    pub key_predicate: KeyPredicate,
    pub capacity: Capacity,
    pub shed: ShedPolicy,
    pub provenance: ProvenanceId,
}

/// `placed_in(instance, thread_domain)`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PlacedIn {
    pub instance: LocusInstanceId,
    pub domain: ThreadDomainId,
    pub provenance: ProvenanceId,
}

/// A resolved CPU set (sorted, deduplicated core indices).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CoreSet(pub Vec<u32>);

/// `affined_to(thread_domain, cores)`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AffinedTo {
    pub domain: ThreadDomainId,
    pub cores: CoreSet,
    pub provenance: ProvenanceId,
}

/// `binds(topic, binding)` — the topic crosses a process boundary
/// through this transport.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TopicBinding {
    pub topic: TopicId,
    pub binding: BindingId,
    pub provenance: ProvenanceId,
}

/// A supervision policy as declared (ops vocabulary + retry bound),
/// kept as authored strings at Change 1 — the artifact already
/// serializes this row shape (schema 1.10).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SupervisionPolicy {
    pub ops: Vec<String>,
    pub retry_bound: Option<u32>,
}

/// `supervises(parent, child, policy)`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Supervises {
    pub parent: LocusDeclId,
    pub child: LocusDeclId,
    pub policy: SupervisionPolicy,
    pub provenance: ProvenanceId,
}

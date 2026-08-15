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
    BindingId, FunctionId, GroupId, LocusDeclId, LocusInstanceId, PayloadContractId, PhaseId,
    ProvenanceId, SeedId, SubjectId, ThreadDomainId, TopicId,
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
    Interface {
        interface: String,
    },
    /// A contracted through-stdlib path: the artifact deliberately
    /// hides stdlib interiors, but the endpoints are exact.
    ViaStdlib,
}

/// `calls(caller, callee)` at SITE grain — resolved calls only
/// (law 1 above). One row per call site: two sites sharing
/// endpoints but differing in loop/boundedness facts are two rows,
/// each with its own provenance span (witnesses point at sites).
/// `site` is the 0-based source-order ordinal of the site within
/// its caller — stable under source motion as long as relative
/// order holds, which is the same stability class as the shape
/// hash. Endpoint-grained projections (the artifact's merged
/// `calls` relation) are DERIVED by conservative merge: `in_loop`
/// and `unbounded` OR together, never dropped.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Call {
    pub from: FunctionId,
    pub to: FunctionId,
    pub dispatch: DispatchKind,
    /// Source-order site ordinal within `from` (see above).
    pub site: u32,
    /// The site sits inside a loop.
    pub in_loop: bool,
    /// The site's per-activation repetition is not statically
    /// bounded (the topology model keeps loop and unbounded as
    /// separate facts; so does this schema).
    pub unbounded: bool,
    pub provenance: ProvenanceId,
}

/// `publishes(function, subject)` at SITE grain, with the site's
/// key domain and declared loss disposition. The endpoint identity
/// is the WIRE SUBJECT — literal (`publish "orders.created"`) and
/// declared-topic publishes are both real endpoints; when a
/// declared topic exists, `declared_topic` links it (and its
/// subject must agree — validated). Synthesizing a fake Topic for
/// a literal subject would corrupt the declared-topic sort and the
/// legacy hash. One function may publish one subject from several
/// sites with DIFFERENT dispositions — each its own row.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Publish {
    pub function: FunctionId,
    pub subject: SubjectId,
    /// `Some` when a declared topic covers this endpoint; its
    /// subject must equal `subject`.
    pub declared_topic: Option<TopicId>,
    /// The endpoint's payload contract. A literal endpoint carries
    /// its `of type T` here (the checked fact BusGraph keeps); a
    /// declared endpoint's payload must agree with its topic's
    /// (validated).
    pub payload: PayloadContractId,
    /// Source-order site ordinal within `function`.
    pub site: u32,
    /// `Some` iff the declared topic is keyed (validated both
    /// ways) — an undeclared or unkeyed endpoint has no key domain
    /// to invent.
    pub key_domain: Option<KeyDomain>,
    pub disposition: PublishDisposition,
    pub provenance: ProvenanceId,
}

/// `subscribes(topic, handler)` with the subscription's key
/// predicate and bounds — the raw facts `may_deliver`/`must_deliver`
/// derive from.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Subscribe {
    /// The wire subject or pattern — wildcard subscriptions
    /// (`subscribe "orders.**"`) are subject-only endpoints.
    pub subject: SubjectId,
    /// `Some` when a declared topic covers this endpoint; subjects
    /// must agree (validated).
    pub declared_topic: Option<TopicId>,
    /// The endpoint's payload contract (`of type T` on literal /
    /// wildcard subscriptions; must agree with a declared topic's
    /// payload).
    pub payload: PayloadContractId,
    pub handler: FunctionId,
    /// Source-order ordinal of the subscription declaration within
    /// its locus's bus block — two subscriptions of one topic by
    /// one handler with different filters are two rows.
    pub site: u32,
    pub key_predicate: KeyPredicate,
    pub capacity: Capacity,
    pub shed: ShedPolicy,
    pub provenance: ProvenanceId,
}

/// `member_of_group(group, member)` — one row per RESOLVED group
/// member (loci and free fns; globs enumerated). This is the
/// normalized grain judgments quantify over. It is deliberately
/// NOT the authored grain: the legacy artifact hashes the selector
/// list AS WRITTEN (`lib::*` stays unexpanded), so projection
/// reads [`GroupSelector`] — `{ lib::* }` and `{ lib::A, lib::B }`
/// may resolve to identical membership yet MUST keep distinct
/// shapes.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct GroupMember {
    pub group: GroupId,
    pub member: crate::ids::EntityRef,
    pub provenance: ProvenanceId,
}

/// One authored selector of a group declaration, in authored order
/// (`ordinal`). The authored spelling is the fact the legacy hash
/// covers; `GroupMember` carries the resolution.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SelectorForm {
    /// A single named member — canonical reference plus the
    /// authored display spelling (`DeltaStore`, `lib::A`).
    Named {
        member: crate::ids::EntityRef,
        display: String,
    },
    /// A trailing glob over a seed's exports, kept UNEXPANDED as
    /// authored (`lib::*`) — a zero-member glob is still a
    /// selector row even though it contributes no members.
    SeedGlob { seed: SeedId, display: String },
}

/// `group_selector(group, ordinal)` — the authored selector list.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct GroupSelector {
    pub group: GroupId,
    /// 0-based authored position — the artifact hashes the list
    /// ordered, so order is a semantic fact.
    pub ordinal: u32,
    pub selector: SelectorForm,
    pub provenance: ProvenanceId,
}

/// A known-DEAD interface dispatch: a call through an interface no
/// locus in the closed world conforms to. NOT a hole — an
/// uninhabited interface has no values, so the site contributes no
/// edge and does not prevent certification (`exact_calls` stays
/// claimable). Retained as a row because a conformer appearing
/// later must change the model shape — exactly why the topology
/// artifact records it inside the hashed half.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DeadInterfaceCall {
    pub from: FunctionId,
    /// Source-order site ordinal within `from`.
    pub site: u32,
    /// The uninhabited interface's canonical name.
    pub interface: String,
    /// The dispatched method name.
    pub method: String,
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

/// `supervises(parent, child, error_type, policy)` — one row per
/// `on_failure` handler. Two handlers supervising the same child
/// for DIFFERENT error types are distinct policies (the schema-1.10
/// supervision section is per-handler), so the error type is part
/// of the row and its canonical key.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Supervises {
    pub parent: LocusDeclId,
    pub child: LocusDeclId,
    /// The handled error/violation type's canonical name.
    pub error_type: String,
    pub policy: SupervisionPolicy,
    pub provenance: ProvenanceId,
}

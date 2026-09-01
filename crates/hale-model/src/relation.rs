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
    /// The member.
    pub function: FunctionId,
    /// The locus declaring it.
    pub locus: LocusDeclId,
    /// The member's declaration site.
    pub provenance: ProvenanceId,
}

/// `phase_of(function, phase)`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PhaseOf {
    /// The hook, mode, or handler.
    pub function: FunctionId,
    /// The lifecycle phase it runs in.
    pub phase: PhaseId,
    /// The hook's declaration site.
    pub provenance: ProvenanceId,
}

/// `declared_in(entity, seed)`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DeclaredIn {
    /// The declared entity, in whichever sort it belongs to.
    pub entity: crate::ids::EntityRef,
    /// The seed that declares it.
    pub seed: SeedId,
    /// The declaration site, in that seed's file.
    pub provenance: ProvenanceId,
}

/// `realizes(instance, locus_decl)`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Realizes {
    /// The statically exact instance.
    pub instance: LocusInstanceId,
    /// The declaration it realizes.
    pub decl: LocusDeclId,
    /// The param or literal that gives the instance birth.
    pub provenance: ProvenanceId,
}

/// `owns(parent_instance, child_instance)` — the lifecycle tree.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Owns {
    /// The owning instance.
    pub parent: LocusInstanceId,
    /// The owned instance. One child has exactly one parent.
    pub child: LocusInstanceId,
    /// The param or birth expression establishing ownership.
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
    /// The caller.
    pub from: FunctionId,
    /// The resolved callee. Unresolved callees are holes, never rows.
    pub to: FunctionId,
    /// How this site reaches the callee. An interface site yields one
    /// row per conformer, all sharing `site`.
    pub dispatch: DispatchKind,
    /// Source-order site ordinal within `from` (see above).
    pub site: u32,
    /// The site sits inside a loop.
    pub in_loop: bool,
    /// The site's per-activation repetition is not statically
    /// bounded (the topology model keeps loop and unbounded as
    /// separate facts; so does this schema).
    pub unbounded: bool,
    /// The call expression — what a witness points at.
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
    /// The function containing the send.
    pub function: FunctionId,
    /// The wire subject sent to. This, not `declared_topic`, is what
    /// delivery joins on.
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
    /// The send is loop-nested (GH #476 Change 5d — a bus carrier
    /// reached from inside a loop repeats per iteration).
    pub in_loop: bool,
    /// `Some` iff the declared topic is keyed (validated both
    /// ways) — an undeclared or unkeyed endpoint has no key domain
    /// to invent.
    pub key_domain: Option<KeyDomain>,
    /// What the send declared should happen if the cell cannot be
    /// delivered.
    pub disposition: PublishDisposition,
    /// The send expression (`T <- v`).
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
    /// The method the delivered cell invokes.
    pub handler: FunctionId,
    /// Source-order ordinal of the subscription declaration within
    /// its locus's bus block — two subscriptions of one topic by
    /// one handler with different filters are two rows.
    pub site: u32,
    /// The filter half of delivery — which keys this subscription
    /// accepts.
    pub key_predicate: KeyPredicate,
    /// The subscription's declared queue bound, if any.
    pub capacity: Capacity,
    /// What to drop when that bound is reached.
    pub shed: ShedPolicy,
    /// The `subscribe` clause in the locus's bus block.
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
    /// The group.
    pub group: GroupId,
    /// One resolved member. Globs arrive here already enumerated.
    pub member: crate::ids::EntityRef,
    /// The selector that admitted this member.
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
    /// The group this selector belongs to.
    pub group: GroupId,
    /// 0-based authored position — the artifact hashes the list
    /// ordered, so order is a semantic fact.
    pub ordinal: u32,
    /// The selector as authored — named member or unexpanded glob.
    pub selector: SelectorForm,
    /// The selector's position in the group declaration.
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
    /// The caller holding the uninhabited dispatch.
    pub from: FunctionId,
    /// Source-order site ordinal within `from`.
    pub site: u32,
    /// The uninhabited interface's canonical name.
    pub interface: String,
    /// The dispatched method name.
    pub method: String,
    /// The call expression, retained so a later conformer has a site to
    /// attach to.
    pub provenance: ProvenanceId,
}

/// `declares_publish(locus, subject)` — a DECLARED publisher end
/// (`bus { publish Orders; }`), distinct from [`Publish`]'s
/// site-grained send expressions. The `require publishes(...)`
/// claim semantics quantify over declared ends: a locus that
/// declares the end but never sends still publishes in the
/// endpoint sense, and dropping this row while claiming
/// `exact_bus_endpoints` was review round 7's catch.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DeclaresPublish {
    /// The locus declaring the end.
    pub locus: LocusDeclId,
    /// The declared wire subject.
    pub subject: SubjectId,
    /// `Some` when a declared topic covers the end; subject and
    /// payload agreement validated exactly as for `Publish`.
    pub declared_topic: Option<TopicId>,
    /// The end's payload contract.
    pub payload: PayloadContractId,
    /// The `publish` clause in the bus block.
    pub provenance: ProvenanceId,
}

/// `placed_in(instance, thread_domain)`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PlacedIn {
    /// The placed instance.
    pub instance: LocusInstanceId,
    /// The thread domain it runs on.
    pub domain: ThreadDomainId,
    /// The placement clause, or the inheriting parent's.
    pub provenance: ProvenanceId,
}

/// A resolved CPU set (sorted, deduplicated core indices).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CoreSet(pub Vec<u32>);

/// `affined_to(thread_domain, cores)`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AffinedTo {
    /// The thread domain being pinned.
    pub domain: ThreadDomainId,
    /// The resolved cores, sorted and deduplicated.
    pub cores: CoreSet,
    /// The affinity clause.
    pub provenance: ProvenanceId,
}

/// `binds(topic, binding)` — the topic crosses a process boundary
/// through this transport.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TopicBinding {
    /// The topic that crosses the boundary.
    pub topic: TopicId,
    /// The transport it crosses through.
    pub binding: BindingId,
    /// The binding declaration.
    pub provenance: ProvenanceId,
}

/// A supervision policy as declared (ops vocabulary + retry bound),
/// kept as authored strings at Change 1 — the artifact already
/// serializes this row shape (schema 1.10).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SupervisionPolicy {
    /// The recovery operations the handler body performs, as authored
    /// (`restart`, `quarantine`, ...), in source order.
    pub ops: Vec<String>,
    /// The literal as WRITTEN — i64 like the legacy artifact, which
    /// serializes any int literal the parser accepted (`for
    /// 4294967296` is check-clean; a narrower type here silently
    /// truncated it — review round 13).
    pub retry_bound: Option<i64>,
}

/// What an `on_failure` handler supervises. Usually a declared
/// locus — but transport supervision is shipped surface (GH #233:
/// `on_failure(t: std::bus::UnixTransport, err: ClosureViolation)`),
/// and a substrate type is not a locus declaration. External
/// carries the canonical type name rather than pretending.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum SupervisedRef {
    Locus(LocusDeclId),
    /// A supervised non-locus (substrate transport, external
    /// boundary), by canonical type name.
    External(String),
}

/// `supervises(parent, child, error_type, policy)` — one row per
/// `on_failure` handler. Two handlers supervising the same child
/// for DIFFERENT error types are distinct policies (the schema-1.10
/// supervision section is per-handler), so the error type is part
/// of the row and its canonical key.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Supervises {
    /// The supervising locus — the one declaring `on_failure`.
    pub parent: LocusDeclId,
    /// What is supervised. Usually a locus declaration; a supervised
    /// transport is `External`.
    pub child: SupervisedRef,
    /// The handled error/violation type's canonical name.
    pub error_type: String,
    /// The recovery vocabulary and retry bound this handler declares.
    pub policy: SupervisionPolicy,
    /// Authored declaration order across the bundle walk. Handler
    /// order is an authored fact the legacy artifact depends on:
    /// its encoder collects handlers in source order and
    /// stable-sorts by (locus, child) only, so handlers sharing
    /// both serialize in authored order, not error-type order
    /// (review round 11). PART OF THE CANONICAL KEY: two handlers
    /// with identical (parent, child, error_type) signatures are
    /// check-clean and both serialize in the legacy artifact, so
    /// the model must hold both rows (review round 14).
    pub authored_ordinal: u32,
    /// The `on_failure` declaration.
    pub provenance: ProvenanceId,
}

/// One per-call COST the model can see inside a function body
/// (GH #476 Change 5h) — what the quantitative `@budget` laws
/// count over.
///
/// Site-grained on purpose, exactly like `Publish`: a per-call
/// budget is a statement about ONE invocation, so whether the site
/// sits inside a loop is the difference between a finite count and
/// an unbounded one. Collapsing sites to a per-function total
/// before the judgment throws away the site that saturates.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CostSite {
    /// The function the cost is attributed to.
    pub function: FunctionId,
    /// Which cost this is — and therefore its grain: `Alloc`/`Block`
    /// are occurrences, `FrameBytes` is one row per function.
    pub dimension: CostDimension,
    /// How much this site costs, in the dimension's own unit.
    pub amount: u64,
    /// Inside a loop — a per-call bound cannot survive it.
    pub in_loop: bool,
    /// The allocation or blocking site. For `FrameBytes`, the
    /// function's own declaration. Part of the row's canonical key,
    /// since cost rows carry no site ordinal.
    pub provenance: ProvenanceId,
}

/// The cost dimensions the model records DIRECTLY. Deliberately
/// short: `publish` and `fanout` are NOT here, because
/// `relations.publishes` plus the delivery join already answer
/// them, and a second copy would be a second authority.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum CostDimension {
    /// One arena allocation the summary can see.
    Alloc,
    /// One blocking operation.
    Block,
    /// This function's own stack frame, in bytes — an estimate from
    /// declared shapes, not measured from codegen. Recorded once
    /// per function, never inside a loop.
    FrameBytes,
}

impl CostDimension {
    pub fn as_str(self) -> &'static str {
        match self {
            CostDimension::Alloc => "alloc",
            CostDimension::Block => "block",
            CostDimension::FrameBytes => "frame_bytes",
        }
    }
}

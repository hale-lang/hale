//! # The canonical semantic model (GH #476, Change 1)
//!
//! One typed, provenance-bearing description of a checked Hale
//! application — the value every structural consumer will read
//! instead of re-deriving its own partial graph. This crate is the
//! **schema and its laws**; derivation from a checked bundle
//! (Change 2), the legacy-artifact projection and hash compatibility
//! (Change 3), `ClaimIr` (Change 4), and the judgment engine
//! (Changes 5a–5e) land separately per the epic's build order.
//!
//! ## The architectural law
//!
//! ```text
//! Source --check/derive--> Model --judge(ClaimIr)--> Evidence
//!                              |
//!                              +--project--> Artifact
//!
//! Plan --admit/elaborate-------------------------------> Model
//! Model --derive---------------------------------------> LoweringPlan
//! ```
//!
//! Derive a modeled semantic fact **once**. Downstream consumers
//! project or query it; they do not walk the AST, generic artifact
//! JSON, or codegen state to rediscover it. Eight concepts stay
//! distinct and un-conflated: source, plan, **model** (this crate),
//! `ClaimIr`, evidence, artifact, lowering plan, and execution
//! evidence.
//!
//! ## What a model is
//!
//! A model is **known facts plus an explicit account of what it does
//! not know**, closed at a horizon:
//!
//! - typed entity tables ([`Entities`]) — not one homogeneous node
//!   kind;
//! - typed relation tables ([`Relations`]) — `calls`, `owns`,
//!   `publishes` are different rows with different fields and
//!   witness renderings, never interchangeable string edges;
//! - typed **holes** ([`Hole`]) — unresolved residue as data, each
//!   naming the relation families it hides;
//! - positive **capabilities** ([`Capabilities`]) — what is exact,
//!   stated, so a judgment can ask "is this model adequate?" without
//!   reverse-engineering the absence of strings;
//! - **provenance on every row** ([`Provenance`]) — source-neutral
//!   (`SourceId` + byte span, or a named synthetic origin); this
//!   crate never sees the AST.
//!
//! The judgment vocabulary this feeds (Changes 5a–5e): `holds` only
//! over a complete relevant projection; `violated` with a concrete
//! countermodel; `uncertified` when relevant reachable residue
//! remains; `invalid` for a malformed law. A concrete path beats a
//! hole; a hole beats a false proof of absence.
//!
//! ## Keyed delivery (first-class, two-sided)
//!
//! A keyed topic is not a broadcast edge. The schema records the
//! authored facts — [`TopicKey`], per-publish [`KeyDomain`],
//! per-subscription [`KeyPredicate`], replica indices — and
//! judgments derive **two different relations** from them:
//!
//! - `may_deliver`: a subscriber is included when its predicate is
//!   satisfiable **or unknown** — unknowns add possibilities, which
//!   keeps negative claims (`forbid reaches`) conservative;
//! - `must_deliver`: available only when key coverage and routing
//!   are exact — a positive delivery guarantee never rests on an
//!   unknown.
//!
//! `must_deliver` is **structural**: "dispatched ⇒ delivered, the
//! process persisting." Temporal process-edge behavior (boot-window
//! buffering, exit quiesce — GH #468) is runtime contract, not a
//! static relation.
//!
//! ## Bounds and loss policy (recorded before claims need them)
//!
//! [`Capacity`], [`ShedPolicy`], [`PublishDisposition`], and
//! [`BindingLossBehavior`] are schema rows now so a future
//! must-arrive/no-loss claim does not retroactively change the
//! meaning of existing delivery edges. Four query concepts stay
//! distinct: a structural endpoint **exists**; a delivery is
//! **possible**; a delivery is **guaranteed** under the declared
//! contract; a path has **capacity**. A shedding policy never
//! removes a possible-reachability edge; it invalidates an
//! unqualified must-arrive guarantee.
//!
//! ## Identity and replay compatibility
//!
//! [`ModelHashKind`] names hash algorithms explicitly. The first
//! model-backed artifact encoder must reproduce the legacy
//! `TopologyShapeV1` hash exactly over the corpus before any cutover
//! (Change 3); a richer identity is a **versioned transition** with
//! an exact recording diagnostic, never a silent reinterpretation of
//! the existing `u64`. Existing `.halerec` admission behavior is an
//! acceptance criterion, not a casualty.
//!
//! ## Demand and the LSP contract
//!
//! Nothing in this crate runs unless demanded. The Change-2 builder
//! is lazy per checked bundle, and a no-claims diagnostics-only
//! check must provably never construct these tables ("cached" must
//! not become "always built").
//!
//! ## Named consumers
//!
//! - **#464 (placement-aware devirtualization)** is Change 8's first
//!   customer: its corpus domain survey becomes a model query and
//!   `DispatchPlan::derive(&ApplicationModel)` owns the decision,
//!   recorded in the execution digest — never as an authored
//!   semantic row.
//! - **iris**: obs registration/events gain canonical model entity
//!   IDs stamped at codegen time, so runtime events join model rows
//!   by ID instead of heuristic string matching, and holes give iris
//!   an "expected-unpaired" category.
//!
//! ## Architecture canaries (enforced in `tests/architecture.rs`)
//!
//! - this crate depends on **nothing** — in particular never on
//!   `hale-syntax` or `hale-types`;
//! - every row carries provenance or a named synthetic origin (by
//!   construction: the field is not optional);
//! - a hole must hide at least one relation family;
//! - a capability cannot claim exactness while a hole hides that
//!   family ([`ApplicationModel::validate`]);
//! - tables are canonically sorted and deduplicated — deterministic
//!   iteration is a law, not a convention.

pub mod application;
pub mod capability;
pub mod entity;
pub mod hole;
pub mod ids;
pub mod keys;
pub mod provenance;
pub mod relation;

pub use application::{
    ApplicationModel, Entities, LabelRow, ModelError, ModelHashKind,
    ModelHeader, Relations, WeightRow, MODEL_SEMANTICS_V1,
};
pub use capability::Capabilities;
pub use entity::{
    Binding, BindingRole, Function, FunctionKind, LocusDecl, LocusInstance,
    PayloadContract, Phase, Seed, Subject, ThreadDomain, Topic,
    TransportKind,
};
pub use hole::{Hole, HoleKind, RelationSet};
pub use ids::{
    BindingId, EntityRef, FunctionId, LocusDeclId, LocusInstanceId,
    PayloadContractId, PhaseId, ProvenanceId, SeedId, SourceId, SubjectId,
    ThreadDomainId, TopicId,
};
pub use keys::{
    BindingLossBehavior, Capacity, KeyDomain, KeyPredicate, KeyValue,
    PublishDisposition, ShedPolicy, TopicBound, TopicKey, TopicOnFull,
};
pub use provenance::{Provenance, ProvenanceTable};
pub use relation::{
    AffinedTo, Call, CoreSet, DeclaredIn, DispatchKind, MemberOf, Owns,
    PhaseOf, PlacedIn, Publish, Realizes, Subscribe, Supervises,
    SupervisionPolicy, TopicBinding,
};

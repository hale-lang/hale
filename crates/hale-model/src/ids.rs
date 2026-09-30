//! Dense typed IDs — indices into their owning table.
//!
//! Two identity laws from the epic:
//!
//! 1. **No semantic string joins.** Canonical declaration identity,
//!    author-facing display spelling, wire subject/address identity,
//!    payload-shape identity, and deployed-instance identity are
//!    separate fields. The model never decides two things are the
//!    same because display strings happen to match.
//! 2. **Internal density, external stability.** These IDs are dense
//!    table indices for in-memory speed. Serialized identities
//!    (Change 3+) are stable canonical names, never these numbers —
//!    an ID is meaningless outside the model value that minted it.

// The macro, the seed id and the provenance ids are `hale-graph`'s
// (F.40 phase 1.1a): the model's identity mechanics are the graph
// core's, and its sorts stay here.
use hale_graph::table_id;
pub use hale_graph::ids::SeedId;
pub use hale_graph::provenance::{ProvenanceId, SourceId};

table_id!(
    /// A function, method, lifecycle hook, or mode body.
    FunctionId
);
table_id!(
    /// A declared USER effect class (GH #476 Change 4) — built-in
    /// classes are language-fixed and never table rows.
    EffectClassId
);
table_id!(
    /// A locus declaration (the type, not a running instance).
    LocusDeclId
);
table_id!(
    /// A statically exact locus instance in the main arrangement.
    LocusInstanceId
);
table_id!(
    /// A declared topic (name + subject + payload contract).
    TopicId
);
table_id!(
    /// A wire subject or subject pattern.
    SubjectId
);
table_id!(
    /// A payload shape contract.
    PayloadContractId
);
table_id!(
    /// A lifecycle phase (birth, run, handler, dissolve, method…).
    PhaseId
);
table_id!(
    /// A thread domain: a pinned thread, a cooperative pool's
    /// worker, the main thread, an async-I/O pool.
    ThreadDomainId
);
table_id!(
    /// A transport binding declared on the main locus (or admitted
    /// from deploy-time config).
    BindingId
);
table_id!(
    /// A declared claim-vocabulary group (`group g = { ... }`).
    GroupId
);
table_id!(
    /// A declared value type (`type T { ... }`, enums included).
    TypeDeclId
);
table_id!(
    /// A declared interface.
    InterfaceDeclId
);
table_id!(
    /// A seed-membership-only declaration (perspective, const,
    /// ring layout, target) — see [`Declaration`].
    ///
    /// [`Declaration`]: crate::entity::Declaration
    DeclarationId
);

/// A reference to any entity sort — the anchor vocabulary shared by
/// holes, labels, weights, and (later) evidence steps.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum EntityRef {
    Function(FunctionId),
    LocusDecl(LocusDeclId),
    LocusInstance(LocusInstanceId),
    Topic(TopicId),
    Subject(SubjectId),
    Binding(BindingId),
    ThreadDomain(ThreadDomainId),
    Phase(PhaseId),
    Seed(SeedId),
    /// A declared group — seed membership (`declared_in`) covers
    /// groups, since the seed sort hashes the full rename table.
    Group(GroupId),
    /// A declared value type — a seed member even though types are
    /// not path vertices.
    Type(TypeDeclId),
    /// A declared interface.
    Interface(InterfaceDeclId),
    /// A seed-membership-only declaration (perspective, const,
    /// ring layout, target).
    Declaration(DeclarationId),
}

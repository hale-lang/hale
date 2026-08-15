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

macro_rules! table_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
        pub struct $name(pub u32);

        impl $name {
            pub fn index(self) -> usize {
                self.0 as usize
            }
        }
    };
}

table_id!(
    /// A function, method, lifecycle hook, or mode body.
    FunctionId
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
    /// A source seed (compilation unit set).
    SeedId
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
    /// A source-neutral origin record in the [`ProvenanceTable`].
    ///
    /// [`ProvenanceTable`]: crate::provenance::ProvenanceTable
    ProvenanceId
);
table_id!(
    /// A source unit (path + content digest) provenance points into.
    SourceId
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
}

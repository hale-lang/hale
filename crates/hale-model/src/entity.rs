//! Entity tables — typed sorts, not one homogeneous node kind.
//!
//! Sorts live at two strata: the code stratum (functions, locus
//! *declarations*) and the system stratum (locus *instances*,
//! bindings, thread domains). The declaration/instance split is what
//! lets application claims count declarations while fleet claims
//! count deployed instances without punning — the epic's typed
//! `CountDomain` distinction starts here.
//!
//! Every entity separates **canonical name** (identity) from
//! **display** spelling (what diagnostics render). Effects are
//! recorded as label strings at Change 1 — the classification
//! vocabulary (the #265 lattice) lives upstream and would otherwise
//! drag a dependency into this crate; the label set is validated
//! against the upstream lattice at derivation time (Change 2).

use crate::ids::{LocusDeclId, PayloadContractId, ProvenanceId, SubjectId};
use crate::keys::TopicKey;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FunctionKind {
    /// A lifecycle hook (birth, run, dissolve, on_failure…).
    Hook,
    /// A locus method (including bus handlers).
    Method,
    /// A free function.
    Free,
    /// A mode body.
    Mode,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Function {
    /// Canonical identity, e.g. `Worker::on_r` or `describe`.
    pub name: String,
    /// Author-facing spelling when it differs (stdlib publics render
    /// their `std::…` path, never the mangled name).
    pub display: String,
    pub kind: FunctionKind,
    /// Effect labels in declaration order (order is semantic in the
    /// existing artifact and is preserved).
    pub effects: Vec<String>,
    pub provenance: ProvenanceId,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LocusDecl {
    pub name: String,
    pub display: String,
    /// `@sealed` confinement (GH #436).
    pub sealed: bool,
    pub provenance: ProvenanceId,
}

/// A statically exact instance in the main arrangement, e.g. the
/// `App.w` born from `params { w: Worker = Worker { }; }`. Replica
/// fan-outs contribute one instance per index.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LocusInstance {
    /// Canonical instance path, e.g. `App.w` or `App.workers[3]`.
    pub path: String,
    pub decl: LocusDeclId,
    /// `Some(k)` for a `replicas = K` member; feeds `EqReplica`
    /// coverage in keyed-delivery judgments.
    pub replica: Option<u32>,
    pub provenance: ProvenanceId,
}

/// A wire subject or pattern. Address identity — deliberately a
/// DIFFERENT sort from the payload contract, even though the current
/// runtime keeps a fused hash for compatibility (the model derives
/// that fusion; it does not make it the schema).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Subject {
    pub pattern: String,
    /// False when the pattern contains wildcards.
    pub exact: bool,
    pub provenance: ProvenanceId,
}

/// A payload shape contract.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PayloadContract {
    /// The canonical shape string (field:kind;… — as the artifact
    /// serializes today).
    pub shape: String,
    pub hash: u64,
    pub provenance: ProvenanceId,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Topic {
    /// Canonical identity: the RAW post-merge declaration name.
    pub name: String,
    /// Author-facing spelling (the alias-qualified form for
    /// imports) — what the artifact's topic sort renders.
    pub display: String,
    pub subject: SubjectId,
    pub payload: PayloadContractId,
    /// `Some` for `keyed_by` topics.
    pub key: Option<TopicKey>,
    /// `Some` for `bounded(N)` topics — the publisher-facing
    /// capacity + refusal contract (GH #255's topic-level knob).
    pub bound: Option<crate::keys::TopicBound>,
    pub provenance: ProvenanceId,
}

/// A declared claim-vocabulary group. Groups are
/// verification-relevant and shape-hashed in the existing artifact
/// (claims resolve their selectors through them), so they are model
/// rows, not a side channel. Membership lives in the
/// [`GroupMember`] relation.
///
/// [`GroupMember`]: crate::relation::GroupMember
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Group {
    /// Canonical identity: the RAW post-merge declaration name.
    pub name: String,
    /// Author-facing spelling.
    pub display: String,
    /// Declared `may_be_empty` — an empty group without it is a
    /// checker error (vacuity fail-closed), so the declared intent
    /// is a semantic fact selectors need.
    pub may_be_empty: bool,
    pub provenance: ProvenanceId,
}

/// A declared value type. Types are not path vertices — they exist
/// in the model because the declaration universe must cover the
/// seed sort exactly (the topology hash covers the full rename
/// table: loci, fns, types, interfaces, topics, groups), and
/// because payload contracts and key types name them.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TypeDecl {
    pub name: String,
    pub display: String,
    pub provenance: ProvenanceId,
}

/// A declared interface (the F.20 structural contract).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct InterfaceDecl {
    pub name: String,
    pub display: String,
    pub provenance: ProvenanceId,
}

/// A declaration kind that participates in seed membership but has
/// no structural role elsewhere in the model (yet). The
/// declaration-universe law: every NAMEABLE top-level declaration
/// (`top_decl_name` in the compiler: locus, perspective, type,
/// const, fn, interface, topic, ring layout, target, group) is
/// representable — as a specialized sort where the model needs its
/// structure, or as an opaque [`Declaration`] row where seed
/// membership is the only fact. `Module`, `Claims`, and
/// `Constitution` are deliberately nameless there and are NOT
/// declarations here either. A new nameable TopDecl variant must
/// extend one side or the other — the universe canary in
/// tests/architecture.rs mirrors the compiler's list (this crate
/// cannot depend on the AST, so the mirror is by test, not type).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum DeclKind {
    Perspective,
    Const,
    RingLayout,
    Target,
}

/// An opaque seed-membership-only declaration (see [`DeclKind`]).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Declaration {
    pub kind: DeclKind,
    pub name: String,
    pub display: String,
    pub provenance: ProvenanceId,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Phase {
    pub name: String,
    pub provenance: ProvenanceId,
}

/// A declared USER effect class (`effect NAME;` /
/// `effect io = { syscall, block };`) — the vocabulary
/// `@effects` contracts and `bound`/`effects(...)` claims speak
/// (GH #476 Change 4). The interner also creates entries for BARE
/// references in `@effects(...)` clauses, and the evaluators
/// distinguish a declared class from an interned typo — so the
/// model must too: `declared: false` is exactly "referenced,
/// never declared".
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct EffectClassDecl {
    pub name: String,
    /// `effect NAME;` exists (false = bare reference only).
    pub declared: bool,
    /// The NORMALIZED atomic expansion for a composed class
    /// (`effect io = { syscall, block }` → `["block", "syscall"]`,
    /// sorted) — a composed class owns no bit of its own and means
    /// its expansion. Empty for atomic classes.
    pub composition: Vec<String>,
    pub provenance: ProvenanceId,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Seed {
    pub name: String,
    pub provenance: ProvenanceId,
}

/// A thread domain: where code actually runs. Main, a pinned
/// thread, one cooperative pool's worker, an async-I/O pool — and
/// (post-#468) a binding's reader thread, which is a real domain
/// that enqueues cross-thread.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ThreadDomain {
    pub name: String,
    pub provenance: ProvenanceId,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum TransportKind {
    Unix,
    Udp,
    ShmRing,
    /// User-supplied protocol adapter locus (by declaration name).
    Adapter(String),
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum BindingRole {
    Listen,
    Connect,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Binding {
    pub subject: SubjectId,
    pub transport: TransportKind,
    pub role: BindingRole,
    pub loss: crate::keys::BindingLossBehavior,
    pub provenance: ProvenanceId,
}

//! The capability matrix (F.40 phase 3, P3; `notes/f40-capability-matrix.md`):
//! what a target can do, stated once, as three typed tables.
//!
//! - **Behaviours** answer "may a program do this on this target, and
//!   how is it lowered": a [`Behaviour`] cell is `Lower` or `Reject`,
//!   and its [`Origin`] says whether a program construct requests it
//!   (`Source`: a refusal is a located diagnostic) or nothing in a
//!   program does (`Environment`: a signal arriving from outside, whose
//!   `Reject` produces no diagnostic and exists only as a premise).
//! - **Invocations** answer how the artifact may be invoked (`hale run`,
//!   `hale replay`, a recording run): an [`InvocationCell`] is `Allowed`
//!   or `Refused`.
//! - **Obligations** answer whether a spine or prelude emits a runtime
//!   call: an [`ObligationCell`] is `Emit`, or `Omit` with the
//!   [`Premise`] that justifies the omission, which names a behaviour
//!   that is `Reject` or an invocation that is `Refused` on the same
//!   target. An obligation never is a behaviour's verdict; it refers to
//!   one.
//!
//! The columns are the [`TargetClass`]es, derived from a
//! [`TargetSpec`]'s `(arch, os, env)`, never from a triple's name. Every
//! row writes all three cells: there is no default arm, and the musl
//! column is written out like the others, so where musl's answer equals
//! glibc's that equality is the reviewed answer, not a fallback. A new
//! target is a new field of [`Columns`], which every row then has to
//! write.
//!
//! **Today's verdicts.** This table states what the compiler does
//! today, spread across `check.rs`'s stdlib table and `async_io` gate,
//! `link_wasm`, the `is_wasm` sites in codegen and the CLI's
//! `run`/`replay` refusal; `crates/hale-types/tests/shadow_capability.rs`
//! and `crates/hale-codegen/tests/shadow_capability_lowering.rs` hold
//! each of those legacy answers to its cell. Cells the design flips (T2:
//! pool threads, `async_io` and transport bindings on wasm32; T3: the
//! known stubs; T5: `@ffi("js")` on a native target) are written as they
//! are today and named in [`KNOWN_OPEN`], which the laws assert is still
//! today's answer; so are the two wasm32 refusals that exist today only
//! as a link failure (a `pinned` placement and an adapter binding, T2
//! locates them), and the three wasm32 obligations whose `Omit` is
//! today's but whose premise only holds once T2 lands.
//!
//! What is not a cell: target-specific emission choices (the triple,
//! CPU, optimization level, LTO, pass pipeline, DWARF, pointer width),
//! a target's support tier, its file naming and the local toolchain.
//! Those stay [`TargetSpec`] queries; the Final direction of #1212 does
//! not count them as decisions.

use hale_syntax::ast::{PrimType, TopDecl};
use hale_syntax::Span;

use crate::target::{TargetArch, TargetEnv, TargetOs, TargetSpec};
use crate::ty::Ty;

// ------------------------------------------------------------- columns

/// A column of the matrix: what a target can do is decided by its
/// class, and every target the compiler can build belongs to one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TargetClass {
    /// glibc Linux and macOS: the lotus runtime over POSIX threads,
    /// with the `async_io` backend (epoll, kqueue).
    PosixAsync,
    /// musl Linux: POSIX threads, and no `async_io` backend (musl
    /// declares `<ucontext.h>` and implements none of it).
    PosixNoAsync,
    /// `wasm32-unknown-unknown`: a freestanding module, driven by its
    /// host through the loader.
    Wasm32,
}

impl TargetClass {
    pub const ALL: [TargetClass; 3] =
        [TargetClass::PosixAsync, TargetClass::PosixNoAsync, TargetClass::Wasm32];

    /// The class of a target, from its `(arch, os, env)`. `None` for
    /// Windows: a `Planned` tier, refused at argument parsing, which
    /// never reaches the matrix because a tier is not a capability.
    pub fn of(spec: &TargetSpec) -> Option<TargetClass> {
        match spec.os {
            TargetOs::Windows => None,
            TargetOs::None => (spec.arch == TargetArch::Wasm32).then_some(TargetClass::Wasm32),
            TargetOs::Linux if spec.env == TargetEnv::Musl => Some(TargetClass::PosixNoAsync),
            TargetOs::Linux | TargetOs::MacOs => Some(TargetClass::PosixAsync),
        }
    }

    /// The column as a document names it.
    pub fn name(self) -> &'static str {
        match self {
            TargetClass::PosixAsync => "glibc Linux, macOS",
            TargetClass::PosixNoAsync => "musl Linux",
            TargetClass::Wasm32 => "wasm32",
        }
    }
}

/// One cell per [`TargetClass`], each written by its row.
#[derive(Debug, Clone, Copy)]
pub struct Columns<T: 'static> {
    pub posix_async: T,
    pub posix_no_async: T,
    pub wasm32: T,
}

impl<T> Columns<T> {
    pub fn get(&self, class: TargetClass) -> &T {
        match class {
            TargetClass::PosixAsync => &self.posix_async,
            TargetClass::PosixNoAsync => &self.posix_no_async,
            TargetClass::Wasm32 => &self.wasm32,
        }
    }
}

// ---------------------------------------------------------------- keys

/// A behaviour a program may ask of its target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Capability {
    /// A `std::` namespace, keyed by its path after `std` (`io::tcp`).
    /// Every namespace the stdlib defines has a row.
    StdNamespace(&'static str),
    /// `[ffi] link` in the manifest, or `--link`.
    LinkLibrary,
    /// `@export fn`, `@export locus`, and the module's fixed exports.
    ExportSurface,
    /// A program whose entry the host drives.
    EntryInversion(Inversion),
    /// `@ffi("c")`, `@ffi("js")`.
    ForeignAbi(Abi),
    /// A type in an `@ffi` or `@export` signature, by its class, at a
    /// boundary of the given ABI (`@export fn` is a C-ABI symbol).
    FfiType(FfiTypeClass, Abi),
    /// `where async_io` on a placement entry.
    AsyncIoPool,
    /// A thread a locus owns: a `pinned` placement, and an adapter
    /// binding (an adapter's instance runs on its own thread). Main
    /// joins it at dissolve (the pinned-child join).
    PinnedThreads,
    /// A cooperative pool other than `main`: worker threads the pools
    /// share, joined by the teardown spines' pool join.
    PoolThreads,
    /// A `bindings { }` entry.
    RemoteTransport(Transport),
    /// `or wait` on a topic with `on_full: fail` capacity (GH #255
    /// phase 2), bound or not.
    BoundedWait,
    /// A process signal reaching the program: SIGINT's drain, SIGPIPE.
    /// Nothing in a program requests it; its origin is the environment.
    ProcessSignals,
}

/// The forms of an inverted entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Inversion {
    /// A program with `@export`s and no `fn main`: an export-only
    /// module.
    ExportOnly,
    /// `--wrap-main`: a bare `fn main` wrapped as the module's
    /// `@export` entry.
    WrapMain,
    /// An `@export locus` that defines `run()`: a host-driven singleton
    /// asked to run its own loop.
    ExportedLocusRun,
}

impl Inversion {
    pub const ALL: [Inversion; 3] = [Inversion::ExportOnly, Inversion::WrapMain, Inversion::ExportedLocusRun];

    pub fn name(self) -> &'static str {
        match self {
            Inversion::ExportOnly => "ExportOnly",
            Inversion::WrapMain => "WrapMain",
            Inversion::ExportedLocusRun => "ExportedLocusRun",
        }
    }
}

/// A foreign ABI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Abi {
    C,
    Js,
}

impl Abi {
    pub const ALL: [Abi; 2] = [Abi::C, Abi::Js];

    pub fn name(self) -> &'static str {
        match self {
            Abi::C => "C",
            Abi::Js => "Js",
        }
    }

    /// The ABI an `@ffi("…")` attribute names; `None` for any other.
    pub fn of(name: &str) -> Option<Abi> {
        match name {
            "c" => Some(Abi::C),
            "js" => Some(Abi::Js),
            _ => None,
        }
    }
}

/// The transport of a `bindings { }` entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Transport {
    /// `unix("/path")`, either role.
    Unix,
    /// `shm_ring(...)`.
    ShmRing,
    /// A user adapter locus (`MyAdapter { url: ... }`).
    Adapter,
}

impl Transport {
    pub const ALL: [Transport; 3] = [Transport::Unix, Transport::ShmRing, Transport::Adapter];

    pub fn name(self) -> &'static str {
        match self {
            Transport::Unix => "Unix",
            Transport::ShmRing => "ShmRing",
            Transport::Adapter => "Adapter",
        }
    }
}

/// The class of a type at an FFI boundary: one per arm of the
/// FFI-portable predicate, so a type's class decides its verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FfiTypeClass {
    Int,
    Float,
    Bool,
    String,
    Bytes,
    BytesView,
    StringView,
    BytesMut,
    Time,
    Duration,
    Decimal,
    Uint,
    Unit,
    /// A named user type: its C struct, kept layout-compatible by the
    /// library author.
    Named,
    Bounded,
    Projection,
    Array,
    Tuple,
    Function,
    Fallible,
    /// An unresolved type name.
    Unknown,
}

impl FfiTypeClass {
    pub const ALL: [FfiTypeClass; 21] = [
        FfiTypeClass::Int,
        FfiTypeClass::Float,
        FfiTypeClass::Bool,
        FfiTypeClass::String,
        FfiTypeClass::Bytes,
        FfiTypeClass::BytesView,
        FfiTypeClass::StringView,
        FfiTypeClass::BytesMut,
        FfiTypeClass::Time,
        FfiTypeClass::Duration,
        FfiTypeClass::Decimal,
        FfiTypeClass::Uint,
        FfiTypeClass::Unit,
        FfiTypeClass::Named,
        FfiTypeClass::Bounded,
        FfiTypeClass::Projection,
        FfiTypeClass::Array,
        FfiTypeClass::Tuple,
        FfiTypeClass::Function,
        FfiTypeClass::Fallible,
        FfiTypeClass::Unknown,
    ];

    pub fn name(self) -> &'static str {
        match self {
            FfiTypeClass::Int => "Int",
            FfiTypeClass::Float => "Float",
            FfiTypeClass::Bool => "Bool",
            FfiTypeClass::String => "String",
            FfiTypeClass::Bytes => "Bytes",
            FfiTypeClass::BytesView => "BytesView",
            FfiTypeClass::StringView => "StringView",
            FfiTypeClass::BytesMut => "BytesMut",
            FfiTypeClass::Time => "Time",
            FfiTypeClass::Duration => "Duration",
            FfiTypeClass::Decimal => "Decimal",
            FfiTypeClass::Uint => "Uint",
            FfiTypeClass::Unit => "Unit",
            FfiTypeClass::Named => "Named",
            FfiTypeClass::Bounded => "Bounded",
            FfiTypeClass::Projection => "Projection",
            FfiTypeClass::Array => "Array",
            FfiTypeClass::Tuple => "Tuple",
            FfiTypeClass::Function => "Function",
            FfiTypeClass::Fallible => "Fallible",
            FfiTypeClass::Unknown => "Unknown",
        }
    }

    /// The class of a resolved type.
    pub fn of(ty: &Ty) -> FfiTypeClass {
        match ty {
            Ty::Prim(p) => match p {
                PrimType::Int => FfiTypeClass::Int,
                PrimType::Float => FfiTypeClass::Float,
                PrimType::Bool => FfiTypeClass::Bool,
                PrimType::String => FfiTypeClass::String,
                PrimType::Bytes => FfiTypeClass::Bytes,
                PrimType::BytesView => FfiTypeClass::BytesView,
                PrimType::StringView => FfiTypeClass::StringView,
                PrimType::BytesMut => FfiTypeClass::BytesMut,
                PrimType::Time => FfiTypeClass::Time,
                PrimType::Duration => FfiTypeClass::Duration,
                PrimType::Decimal => FfiTypeClass::Decimal,
                PrimType::Uint => FfiTypeClass::Uint,
            },
            Ty::Unit => FfiTypeClass::Unit,
            Ty::Named(_) => FfiTypeClass::Named,
            Ty::Bounded(_, _) => FfiTypeClass::Bounded,
            Ty::Projection(_, _) => FfiTypeClass::Projection,
            Ty::Array(_, _) => FfiTypeClass::Array,
            Ty::Tuple(_) => FfiTypeClass::Tuple,
            Ty::Function { .. } => FfiTypeClass::Function,
            Ty::Fallible { .. } => FfiTypeClass::Fallible,
            Ty::Unknown => FfiTypeClass::Unknown,
        }
    }
}

impl Capability {
    /// The semantic layer the capability belongs to (RFC #1212's
    /// numbering): an approximating lowering is legitimate only on
    /// layers 5 and 7.
    pub fn layer(self) -> u8 {
        match self {
            Capability::FfiType(..) => 2,
            Capability::AsyncIoPool
            | Capability::PinnedThreads
            | Capability::PoolThreads
            | Capability::RemoteTransport(_)
            | Capability::BoundedWait => 5,
            Capability::ProcessSignals => 6,
            Capability::StdNamespace(_)
            | Capability::LinkLibrary
            | Capability::ExportSurface
            | Capability::EntryInversion(_)
            | Capability::ForeignAbi(_) => 8,
        }
    }

    /// Who asks for it: a program construct, or the environment.
    pub fn origin(self) -> Origin {
        match self {
            Capability::ProcessSignals => Origin::Environment,
            Capability::StdNamespace(_)
            | Capability::LinkLibrary
            | Capability::ExportSurface
            | Capability::EntryInversion(_)
            | Capability::ForeignAbi(_)
            | Capability::FfiType(..)
            | Capability::AsyncIoPool
            | Capability::PinnedThreads
            | Capability::PoolThreads
            | Capability::RemoteTransport(_)
            | Capability::BoundedWait => Origin::Source,
        }
    }

    /// The capability as a document names it.
    pub fn label(self) -> String {
        match self {
            Capability::StdNamespace(ns) => format!("std::{ns}"),
            Capability::LinkLibrary => "LinkLibrary".to_string(),
            Capability::ExportSurface => "ExportSurface".to_string(),
            Capability::EntryInversion(i) => format!("EntryInversion({})", i.name()),
            Capability::ForeignAbi(a) => format!("ForeignAbi({})", a.name()),
            Capability::FfiType(t, a) => format!("FfiType({}, {})", t.name(), a.name()),
            Capability::AsyncIoPool => "AsyncIoPool".to_string(),
            Capability::PinnedThreads => "PinnedThreads".to_string(),
            Capability::PoolThreads => "PoolThreads".to_string(),
            Capability::RemoteTransport(t) => format!("RemoteTransport({})", t.name()),
            Capability::BoundedWait => "BoundedWait".to_string(),
            Capability::ProcessSignals => "ProcessSignals".to_string(),
        }
    }
}

/// How the artifact may be invoked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Invocation {
    /// `hale run`.
    Run,
    /// `hale replay`.
    Replay,
    /// A recording run (`hale run` under the recording mode).
    Record,
}

impl Invocation {
    pub const ALL: [Invocation; 3] = [Invocation::Run, Invocation::Replay, Invocation::Record];

    pub fn name(self) -> &'static str {
        match self {
            Invocation::Run => "Run",
            Invocation::Replay => "Replay",
            Invocation::Record => "Record",
        }
    }
}

/// A runtime call a spine or prelude emits, or omits, per target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Obligation {
    /// `lotus_replay_start_ingress` on the main locus.
    ReplayIngress,
    /// The observation identity (`lotus_obs_topic_shape`,
    /// `lotus_obs_exec_digest_set`) and its eager init
    /// (`lotus_obs_eager_init`).
    ObservationIdentity,
    /// `lotus_io_init` (SIGPIPE) and `lotus_drain_signals_install`.
    SignalInstall,
    /// The drain observer (`lotus.observes_drain.<locus>`).
    DrainObserver,
    /// The process-draining term: `self.draining`, the restart gate and
    /// `sleep`'s cut-short read `lotus_process_draining_flag`.
    DrainTerm,
    /// `lotus_bus_load_config` and the listen bindings' key extractors.
    BindingConfig,
    /// `lotus_coop_pool_shutdown_all` (R20) in a teardown spine.
    PoolJoin,
    /// `lotus_bus_wait_abort_all` (R34) in a teardown spine.
    WaitAbort,
    /// `lotus_bus_ingress_quiesce` (R35) in a teardown spine.
    IngressQuiesce,
}

impl Obligation {
    pub const ALL: [Obligation; 9] = [
        Obligation::ReplayIngress,
        Obligation::ObservationIdentity,
        Obligation::SignalInstall,
        Obligation::DrainObserver,
        Obligation::DrainTerm,
        Obligation::BindingConfig,
        Obligation::PoolJoin,
        Obligation::WaitAbort,
        Obligation::IngressQuiesce,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Obligation::ReplayIngress => "ReplayIngress",
            Obligation::ObservationIdentity => "ObservationIdentity",
            Obligation::SignalInstall => "SignalInstall",
            Obligation::DrainObserver => "DrainObserver",
            Obligation::DrainTerm => "DrainTerm",
            Obligation::BindingConfig => "BindingConfig",
            Obligation::PoolJoin => "PoolJoin",
            Obligation::WaitAbort => "WaitAbort",
            Obligation::IngressQuiesce => "IngressQuiesce",
        }
    }
}

// --------------------------------------------------------------- cells

/// Why a cell says what it says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Witness {
    /// The site that decides it today, as `path::symbol` (a symbol or a
    /// text fragment the file contains: the registry's convention).
    pub site: &'static str,
    /// The one sentence a diagnostic or a document renders.
    pub reason: &'static str,
    /// The spec section whose sentence the cell implements, as
    /// `spec/<file>.md § <heading>`.
    pub spec: &'static str,
}

/// A refusal's text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Refusal {
    /// The diagnostic, today's text verbatim, with its holes named:
    /// `{reason}` and `{guidance}` are the cell's own, every other hole
    /// is the use's (`{path}`, `{field}`, `{libs}`, `{locus}`, `{cmd}`).
    pub wording: &'static str,
    /// What a program does instead, or why nothing can stand in.
    pub guidance: Option<&'static str>,
}

impl Refusal {
    /// The diagnostic for one use: the cell's reason and guidance, and
    /// the use's holes.
    pub fn render(&self, witness: &Witness, holes: &[(&str, &str)]) -> String {
        let mut out = self.wording.replace("{reason}", witness.reason);
        if let Some(g) = self.guidance {
            out = out.replace("{guidance}", g);
        }
        for (k, v) in holes {
            out = out.replace(&format!("{{{k}}}"), v);
        }
        out
    }
}

/// A module export the lowering emits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Export {
    pub name: &'static str,
    /// Exported only when the module defines it.
    pub if_defined: bool,
}

/// What a `Lower` verdict carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lowering {
    /// Lowered as the program states it.
    Direct,
    /// The module's fixed exports, beside the `@export` set.
    Exports(&'static [Export]),
    /// The `js` ABI's marshalling: an `Int` crosses as an f64.
    IntAsF64,
    /// A collapse that changes no observable answer (F.38: placement is
    /// semantics-free). Legitimate on layers 5 and 7 only; no cell uses
    /// one today.
    Approximate { collapse: &'static str },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// A program construct requests it; a refusal is a located
    /// diagnostic.
    Source,
    /// Nothing in a program requests it; a `Reject` produces no
    /// diagnostic and exists only as a premise.
    Environment,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BehaviourVerdict {
    Lower(Lowering),
    Reject(Refusal),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Behaviour {
    pub verdict: BehaviourVerdict,
    pub origin: Origin,
    /// The behaviours it stands on: a `Lower` cell requires only
    /// behaviours that are `Lower` on the same target.
    pub requires: &'static [Capability],
    pub witness: Witness,
}

impl Behaviour {
    pub fn is_lower(&self) -> bool {
        matches!(self.verdict, BehaviourVerdict::Lower(_))
    }
    pub fn refusal(&self) -> Option<&Refusal> {
        match &self.verdict {
            BehaviourVerdict::Reject(r) => Some(r),
            BehaviourVerdict::Lower(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvocationVerdict {
    Allowed,
    Refused(Refusal),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvocationCell {
    pub verdict: InvocationVerdict,
    pub witness: Witness,
}

/// A stated proof an omission may rest on, registered with its tests in
/// [`PROOFS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProofId(pub &'static str);

/// A registered proof.
#[derive(Debug, Clone, Copy)]
pub struct Proof {
    pub id: ProofId,
    pub tests: &'static [&'static str],
}

/// The proofs an `Omit` may cite. None is registered yet: the
/// single-thread proof that would omit `WaitAbort` on wasm32 (§3.2 of
/// the design) is a future gate.
pub const PROOFS: &[Proof] = &[];

/// Why an obligation may be omitted on a target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Premise {
    /// That target's behaviour cell is `Reject`.
    Rejects(Capability),
    /// That target's invocation cell is `Refused`.
    Refuses(Invocation),
    /// A registered proof.
    Proven(ProofId),
    All(&'static [Premise]),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObligationVerdict {
    Emit,
    Omit(Premise),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObligationCell {
    pub verdict: ObligationVerdict,
    pub witness: Witness,
}

impl ObligationCell {
    pub fn emits(&self) -> bool {
        self.verdict == ObligationVerdict::Emit
    }
}

#[derive(Debug, Clone, Copy)]
pub struct BehaviourRow {
    pub capability: Capability,
    pub cells: Columns<Behaviour>,
}

#[derive(Debug, Clone, Copy)]
pub struct InvocationRow {
    pub invocation: Invocation,
    pub cells: Columns<InvocationCell>,
}

#[derive(Debug, Clone, Copy)]
pub struct ObligationRow {
    pub obligation: Obligation,
    pub cells: Columns<ObligationCell>,
}

// -------------------------------------------------------------- matrix

/// The three tables.
#[derive(Debug, Clone, Copy)]
pub struct CapabilityMatrix {
    pub behaviours: &'static [BehaviourRow],
    pub invocations: &'static [InvocationRow],
    pub obligations: &'static [ObligationRow],
}

/// The `target_capability` family's producer: the matrix, as data.
pub fn derive_capability_matrix() -> CapabilityMatrix {
    CapabilityMatrix { behaviours: BEHAVIOURS, invocations: INVOCATIONS, obligations: OBLIGATIONS }
}

impl CapabilityMatrix {
    /// The cell, or `None` when the capability has no row: a missing
    /// required row is an error for the caller to report, never a
    /// guess.
    pub fn behaviour(&self, class: TargetClass, cap: Capability) -> Option<&'static Behaviour> {
        self.behaviours.iter().find(|r| r.capability == cap).map(|r| r.cells.get(class))
    }

    pub fn invocation(&self, class: TargetClass, inv: Invocation) -> Option<&'static InvocationCell> {
        self.invocations.iter().find(|r| r.invocation == inv).map(|r| r.cells.get(class))
    }

    pub fn obligation(&self, class: TargetClass, ob: Obligation) -> Option<&'static ObligationCell> {
        self.obligations.iter().find(|r| r.obligation == ob).map(|r| r.cells.get(class))
    }

    /// Whether a premise holds on a target: each `Rejects` names a
    /// behaviour that is `Reject` there, each `Refuses` an invocation
    /// that is `Refused` there, each `Proven` a registered proof.
    pub fn premise_holds(&self, class: TargetClass, premise: &Premise) -> bool {
        match premise {
            Premise::Rejects(c) => self.behaviour(class, *c).is_some_and(|b| !b.is_lower()),
            Premise::Refuses(i) => self
                .invocation(class, *i)
                .is_some_and(|c| matches!(c.verdict, InvocationVerdict::Refused(_))),
            Premise::Proven(p) => PROOFS.iter().any(|q| q.id == *p),
            Premise::All(ps) => ps.iter().all(|p| self.premise_holds(class, p)),
        }
    }
}

// ----------------------------------------------------- effective target

/// The target the configuration names: `--target`, or the host when
/// nothing names one. A snapshot's config carries it, and its bundle
/// hands it to the effective-target row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfiguredTarget {
    /// `host`, or the triple `--target` names.
    pub name: String,
    pub spec: TargetSpec,
    /// Whether `--target` named it. An explicit target is the effective
    /// target; the host a configuration falls back to is not, and a
    /// source declaration overrides it.
    pub explicit: bool,
}

impl ConfiguredTarget {
    /// The machine the compiler runs on, named by nothing: the target
    /// of every check and build that passes no `--target`.
    pub fn host() -> Self {
        ConfiguredTarget { name: "host".to_string(), spec: TargetSpec::host(), explicit: false }
    }
}

/// A source `target` declaration: a top-level `target wasm { }` or
/// `target browser_js { }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetDeclaration {
    /// The program that declares it, as the bundle keys it.
    pub file: String,
    /// `wasm` or `browser_js`.
    pub name: String,
    pub span: Span,
    /// Injected by `--wrap-main`, not written: a consequence of the
    /// configured target, which never selects one.
    pub synthesized: bool,
}

/// What selected the effective target (design §1.3, T1(b)).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selection {
    /// `--target` named it.
    Configured,
    /// A written `target wasm`/`browser_js` declaration, with no
    /// `--target`: wasm32.
    Declared,
    /// Neither: the host.
    Host,
}

/// The refusal of an explicit `--target` that a written declaration
/// contradicts, located at the declaration (design §1.3): `{name}` is
/// the declaration's, `{triple}` the configured target's.
pub const TARGET_CONFLICT_WORDING: &str = "this program declares `target {name}`, and is being checked for \
     `{triple}`: build it with `--target wasm32`, or drop the declaration";

/// The effective-target row of a snapshot (the `target_capability`
/// family's first row): the one target check, build and the editor
/// act on, for analysis and emission alike (T1(b)). An explicit
/// `--target` is the effective target; with none, a written
/// `target wasm`/`browser_js` declaration selects wasm32; with neither,
/// the host. An explicit `--target` of another class than a written
/// declaration's is refused at the declaration.
#[derive(Debug, Clone, PartialEq)]
pub struct TargetRow {
    pub configured: ConfiguredTarget,
    /// The first declaring program's, in the bundle's order.
    pub declaration: Option<TargetDeclaration>,
    /// The target every reader acts on.
    pub effective: TargetSpec,
    /// Its column; `None` only for a Windows triple, which argument
    /// parsing refuses before any snapshot exists.
    pub class: Option<TargetClass>,
    pub selected_by: Selection,
    /// The row's own refusals: the conflict between `--target` and a
    /// written declaration.
    pub refusals: Vec<hale_syntax::Diag>,
}

impl TargetRow {
    pub fn is_wasm32(&self) -> bool {
        self.class == Some(TargetClass::Wasm32)
    }

    /// How a stdlib refusal names what put the program under wasm32:
    /// the declaration when the program holds one (written or injected
    /// by `--wrap-main`), else the configuration.
    pub fn wasm32_selector(&self) -> &'static str {
        if self.declaration.is_some() {
            "`target wasm`"
        } else {
            "`--target wasm32`"
        }
    }
}

/// The wasm32 target, as a declaration selects it.
pub fn wasm32_spec() -> TargetSpec {
    TargetSpec::parse("wasm32-unknown-unknown").expect("wasm32 is a known triple")
}

/// The effective-target row for the programs a bundle holds, under the
/// configured target the bundle carries.
pub fn target_row(bundle: &crate::Bundle<'_>) -> TargetRow {
    let configured = bundle.target.clone();
    let declaration = bundle.programs.iter().find_map(|(file, p)| {
        p.items.iter().find_map(|it| match it {
            TopDecl::Target(t) if matches!(t.name.name.as_str(), "wasm" | "browser_js") => Some(TargetDeclaration {
                file: file.clone(),
                name: t.name.name.clone(),
                span: t.span,
                synthesized: t.synthesized,
            }),
            _ => None,
        })
    });
    let written = declaration.as_ref().filter(|d| !d.synthesized);
    let (effective, selected_by) = if configured.explicit {
        (configured.spec, Selection::Configured)
    } else if written.is_some() {
        (wasm32_spec(), Selection::Declared)
    } else {
        (configured.spec, Selection::Host)
    };
    let class = TargetClass::of(&effective);
    let mut refusals = Vec::new();
    if let Some(d) = written {
        if configured.explicit && class != Some(TargetClass::Wasm32) {
            refusals.push(hale_syntax::Diag::ty(
                d.span,
                TARGET_CONFLICT_WORDING.replace("{name}", &d.name).replace("{triple}", configured.spec.triple),
            ));
        }
    }
    TargetRow { configured, declaration, effective, class, selected_by, refusals }
}

// ------------------------------------------------------------ known open

/// A cell written as it is today that the design changes.
#[derive(Debug, Clone, Copy)]
pub struct KnownOpen {
    pub class: TargetClass,
    pub cell: OpenCell,
    /// The decision that changes it (`notes/f40-capability-matrix.md`
    /// §7) and what it becomes.
    pub decision: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum OpenCell {
    /// `Lower` today, `Reject` once the decision lands.
    Behaviour(Capability),
    /// `Reject` today, but refused late: unlocated, by the linker, after
    /// clang has run. The decision makes it a located diagnostic at the
    /// use.
    LateRefusal(Capability),
    /// `Omit` today, as every spine does, on a premise that holds only
    /// once the decision lands.
    Premise(Obligation),
}

/// The cells written as today's answer that a decision changes. Empty
/// since P3 2 of 3 landed T2, T3 and T5 (design §7): each entry went
/// with the flip that closed it, and the laws hold a new one to the
/// same rules.
pub const KNOWN_OPEN: &[KnownOpen] = &[];

// -------------------------------------------------------------- sites

const SPEC_WASM_GATE: &str = "spec/ffi.md § The `target` declaration + stdlib gating";
const SPEC_STDLIB: &str = "spec/stdlib.md § Module surface";
const SPEC_WASM_FFI: &str = "spec/ffi.md § Package `[ffi]` under wasm32";
const SPEC_EXPORT: &str = "spec/ffi.md § `@export` — exports (Hale → callable by the host)";
const SPEC_ENTRY: &str = "spec/ffi.md § Entry-inversion run-model";
const SPEC_JS: &str = "spec/ffi.md § `@ffi(\"js\")` — host imports (host → into Hale's callees)";
const SPEC_FFI: &str = "spec/ffi.md § Syntax";
const SPEC_FFI_TYPES: &str = "spec/ffi.md § Type marshalling";
const SPEC_ASYNC_IO: &str = "spec/runtime.md § `where async_io` — green-I/O cooperative pools (F.35)";
const SPEC_PLACEMENT: &str = "spec/semantics.md § Placement block (F.31)";
const SPEC_BINDINGS: &str = "spec/semantics.md § Phase 2: hierarchy, subjects, bindings, closed-world optimization";
const SPEC_BUS: &str = "spec/runtime.md § Bus message router";
const SPEC_DRAIN: &str = "spec/semantics.md § Drain cascade (whole-process)";
const SPEC_RECORDING: &str = "spec/runtime.md § Lossless recording mode (GH #296 Phase 1)";
const SPEC_OBSERVATION: &str = "spec/runtime.md § Native observation emission (iris P4, 2026-07-27)";
const SPEC_OBLIGATIONS: &str = "spec/runtime.md § Lifecycle obligations";

const fn w(site: &'static str, reason: &'static str, spec: &'static str) -> Witness {
    Witness { site, reason, spec }
}

const fn lower(witness: Witness) -> Behaviour {
    Behaviour {
        verdict: BehaviourVerdict::Lower(Lowering::Direct),
        origin: Origin::Source,
        requires: &[],
        witness,
    }
}

const fn reject(wording: &'static str, guidance: Option<&'static str>, witness: Witness) -> Behaviour {
    Behaviour {
        verdict: BehaviourVerdict::Reject(Refusal { wording, guidance }),
        origin: Origin::Source,
        requires: &[],
        witness,
    }
}

// --------------------------------------------------------- behaviours

/// The stdlib refusal on wasm32, the stdlib gate's wording verbatim:
/// `{path}` is the use's call path after `std` (its namespace when the
/// use reaches it through a chain), `{selector}` what put the program
/// under wasm32 (`` `target wasm` `` or `` `--target wasm32` ``).
pub const STD_WASM_WORDING: &str = "`std::{path}` is unavailable under {selector}: {reason}";

const STD_NATIVE: Behaviour = lower(w(
    "crates/hale-types/src/stdlib_surface.rs::SURFACES",
    "lowered over the target's libc",
    SPEC_STDLIB,
));
const STD_WASM: Behaviour = lower(w(
    ADMISSION,
    "admitted: outside the browser-unavailable set, lowered through the wasm shim",
    SPEC_WASM_GATE,
));
const fn std_wasm_reject(reason: &'static str, guidance: &'static str) -> Behaviour {
    reject(
        STD_WASM_WORDING,
        Some(guidance),
        w(ADMISSION, reason, SPEC_WASM_GATE),
    )
}
const fn std_ns(
    ns: &'static str,
    posix_async: Behaviour,
    posix_no_async: Behaviour,
    wasm32: Behaviour,
) -> BehaviourRow {
    BehaviourRow { capability: Capability::StdNamespace(ns), cells: Columns { posix_async, posix_no_async, wasm32 } }
}

const WASM_TCP: Behaviour = std_wasm_reject(
    "raw TCP sockets don't exist in the browser; use a WebSocket \
     bus adapter (`ws://`) for networking",
    "a WebSocket bus adapter (`ws://`)",
);
const WASM_UDP: Behaviour =
    std_wasm_reject("raw UDP sockets don't exist in the browser", "(no raw UDP in the browser)");
const WASM_TLS: Behaviour = std_wasm_reject(
    "raw TLS isn't available; the browser performs TLS transparently \
     for `wss://` / `https://`",
    "the browser does TLS transparently for `wss://` / `https://`",
);
const WASM_FS: Behaviour = std_wasm_reject(
    "filesystem access isn't available in the browser sandbox; use \
     `fetch` (via an `@ffi(\"js\")` host import) or a bus message",
    "`fetch` via an `@ffi(\"js\")` host import, or a bus message",
);
const WASM_STDIO: Behaviour = std_wasm_reject(
    "raw terminal I/O isn't available; use `println(...)` (the loader \
     routes it to the host console)",
    "`println(...)` (the loader routes it to the host console)",
);
const WASM_TERM: Behaviour = std_wasm_reject(
    "terminal control (`std::term`) isn't available in the browser",
    "(no terminal in the browser)",
);
const WASM_PROCESS: Behaviour = std_wasm_reject(
    "OS process control (`std::process`) isn't available in the browser",
    "(no OS process control)",
);
const WASM_HTTP: Behaviour = std_wasm_reject(
    "the `std::http` server is built on raw TCP and isn't available \
     in the browser",
    "(server is built on raw TCP)",
);
// T3: the known stubs, refused directly; what each did under wasm32
// is its reason (the shim says it), and an `@ffi("js")` host import is
// the substitute.
const WASM_TIME: Behaviour = std_wasm_reject(
    "the browser module has no clock of its own: the shim's inline `clock_gettime` \
     writes zero and `sleep`'s `clock_nanosleep` is an import stubbed to 0, so a read \
     is always the epoch and a sleep never waits",
    "a host clock (`performance.now`, `Date.now`) or timer through an `@ffi(\"js\")` host import",
);
const WASM_ENV: Behaviour = std_wasm_reject(
    "the browser has no process environment: the shim's inline `getenv` returns NULL",
    "configuration handed in through an `@ffi(\"js\")` host import or an `@export` fn's arguments",
);
const WASM_TS: Behaviour = std_wasm_reject(
    "the tree-sitter parser is a native static library the wasm link never reaches",
    "parsing on the host, through an `@ffi(\"js\")` host import",
);
// T3, the unclassified syscall-backed namespaces: refused whole, no
// lowering contract having been written for them.
const WASM_UNIX: Behaviour = std_wasm_reject(
    "AF_UNIX sockets are syscalls the browser sandbox does not have",
    "a WebSocket bus adapter (`ws://`), or an `@ffi(\"js\")` host import",
);
const WASM_SOCKOPT: Behaviour = std_wasm_reject(
    "socket options are syscalls the browser sandbox does not have",
    "(no sockets in the browser)",
);
const WASM_SHM: Behaviour = std_wasm_reject(
    "a shared-memory ring is mapped with `shm_open` and `mmap`, which the browser sandbox does not have",
    "(no shared memory in the browser)",
);

/// The behaviours. The stdlib rows come first: the rejected
/// namespaces in the order the documents list them, then every other
/// namespace the stdlib defines (`stdlib_surface::SURFACES` and the
/// namespaces of `LOCUS_PATHS`), alphabetically.
pub const BEHAVIOURS: &[BehaviourRow] = &[
    std_ns("io::tcp", STD_NATIVE, STD_NATIVE, WASM_TCP),
    std_ns("io::udp", STD_NATIVE, STD_NATIVE, WASM_UDP),
    std_ns("io::tls", STD_NATIVE, STD_NATIVE, WASM_TLS),
    std_ns("io::fs", STD_NATIVE, STD_NATIVE, WASM_FS),
    std_ns("io::file", STD_NATIVE, STD_NATIVE, WASM_FS),
    std_ns("io::stdin", STD_NATIVE, STD_NATIVE, WASM_STDIO),
    std_ns("io::stdout", STD_NATIVE, STD_NATIVE, WASM_STDIO),
    std_ns("term", STD_NATIVE, STD_NATIVE, WASM_TERM),
    std_ns("process", STD_NATIVE, STD_NATIVE, WASM_PROCESS),
    std_ns("http", STD_NATIVE, STD_NATIVE, WASM_HTTP),
    std_ns("api", STD_NATIVE, STD_NATIVE, STD_WASM),
    std_ns("bus", STD_NATIVE, STD_NATIVE, STD_WASM),
    std_ns("bytes", STD_NATIVE, STD_NATIVE, STD_WASM),
    std_ns("bytes::builder", STD_NATIVE, STD_NATIVE, STD_WASM),
    std_ns("cli", STD_NATIVE, STD_NATIVE, STD_WASM),
    std_ns("compress", STD_NATIVE, STD_NATIVE, STD_WASM),
    std_ns("crypto", STD_NATIVE, STD_NATIVE, STD_WASM),
    std_ns("decimal", STD_NATIVE, STD_NATIVE, STD_WASM),
    std_ns("diag", STD_NATIVE, STD_NATIVE, STD_WASM),
    std_ns("env", STD_NATIVE, STD_NATIVE, WASM_ENV),
    std_ns("io", STD_NATIVE, STD_NATIVE, STD_WASM),
    std_ns("io::mirror", STD_NATIVE, STD_NATIVE, WASM_SHM),
    std_ns("io::sockopt", STD_NATIVE, STD_NATIVE, WASM_SOCKOPT),
    std_ns("io::unix", STD_NATIVE, STD_NATIVE, WASM_UNIX),
    std_ns("iter", STD_NATIVE, STD_NATIVE, STD_WASM),
    std_ns("json", STD_NATIVE, STD_NATIVE, STD_WASM),
    std_ns("lang", STD_NATIVE, STD_NATIVE, STD_WASM),
    std_ns("log", STD_NATIVE, STD_NATIVE, STD_WASM),
    std_ns("math", STD_NATIVE, STD_NATIVE, STD_WASM),
    std_ns("metrics", STD_NATIVE, STD_NATIVE, STD_WASM),
    std_ns("name", STD_NATIVE, STD_NATIVE, STD_WASM),
    std_ns("os", STD_NATIVE, STD_NATIVE, STD_WASM),
    std_ns("rand", STD_NATIVE, STD_NATIVE, STD_WASM),
    std_ns("regex", STD_NATIVE, STD_NATIVE, STD_WASM),
    std_ns("ring", STD_NATIVE, STD_NATIVE, WASM_SHM),
    std_ns("secret", STD_NATIVE, STD_NATIVE, STD_WASM),
    std_ns("shm", STD_NATIVE, STD_NATIVE, STD_WASM),
    std_ns("source", STD_NATIVE, STD_NATIVE, STD_WASM),
    std_ns("str", STD_NATIVE, STD_NATIVE, STD_WASM),
    std_ns("tagged", STD_NATIVE, STD_NATIVE, STD_WASM),
    std_ns("tar", STD_NATIVE, STD_NATIVE, STD_WASM),
    std_ns("test", STD_NATIVE, STD_NATIVE, STD_WASM),
    std_ns("text", STD_NATIVE, STD_NATIVE, STD_WASM),
    std_ns("text::base64", STD_NATIVE, STD_NATIVE, STD_WASM),
    std_ns("time", STD_NATIVE, STD_NATIVE, WASM_TIME),
    std_ns("ts", STD_NATIVE, STD_NATIVE, WASM_TS),
    std_ns("yaml", STD_NATIVE, STD_NATIVE, STD_WASM),
    // ---- link, exports, entry
    BehaviourRow {
        capability: Capability::LinkLibrary,
        cells: Columns {
            posix_async: lower(w(
                "crates/hale-codegen/src/codegen.rs::link_libs",
                "a system library is linked by the native linker",
                SPEC_WASM_FFI,
            )),
            posix_no_async: lower(w(
                "crates/hale-codegen/src/codegen.rs::link_libs",
                "a system library is linked by the native linker (zig cc, static)",
                SPEC_WASM_FFI,
            )),
            wasm32: reject(
                "`[ffi] link = {libs}` cannot be satisfied on wasm32 — {reason}. {guidance}",
                Some(
                    "Provide the code as `[ffi] csrc` so it can be compiled into the module, \
                     or gate the dependency out of the wasm build.",
                ),
                w(
                    "crates/hale-codegen/src/codegen.rs::link_wasm",
                    "there are no system dynamic libraries to link against",
                    SPEC_WASM_FFI,
                ),
            ),
        },
    },
    BehaviourRow {
        capability: Capability::ExportSurface,
        cells: Columns {
            posix_async: lower(w(
                "crates/hale-codegen/src/codegen.rs::synthesize_native_export_wrappers",
                "an `@export fn` is an unmangled C-ABI symbol, and an `@export locus` an ordinary locus",
                SPEC_EXPORT,
            )),
            posix_no_async: lower(w(
                "crates/hale-codegen/src/codegen.rs::synthesize_native_export_wrappers",
                "an `@export fn` is an unmangled C-ABI symbol, and an `@export locus` an ordinary locus",
                SPEC_EXPORT,
            )),
            wasm32: Behaviour {
                verdict: BehaviourVerdict::Lower(Lowering::Exports(WASM_FIXED_EXPORTS)),
                origin: Origin::Source,
                requires: &[],
                witness: w(
                    "crates/hale-codegen/src/codegen.rs::synthesize_wasm_export_wrappers",
                    "the module exports a wrapper per `@export fn`, and for an `@export locus` `_hale_start` and a wrapper per method that is not fallible, beside its fixed exports",
                    SPEC_EXPORT,
                ),
            },
        },
    },
    BehaviourRow {
        capability: Capability::EntryInversion(Inversion::ExportOnly),
        cells: Columns {
            posix_async: reject(EXPORT_ONLY_WORDING, Some(EXPORT_ONLY_GUIDANCE), w(ADMISSION, EXPORT_ONLY_REASON, SPEC_ENTRY)),
            posix_no_async: reject(EXPORT_ONLY_WORDING, Some(EXPORT_ONLY_GUIDANCE), w(ADMISSION, EXPORT_ONLY_REASON, SPEC_ENTRY)),
            wasm32: lower(w(
                CG_HAS_EXPORTS,
                "an export-only module: the host drives it through its exports",
                SPEC_ENTRY,
            )),
        },
    },
    BehaviourRow {
        capability: Capability::EntryInversion(Inversion::WrapMain),
        cells: Columns {
            posix_async: reject(
                WRAP_MAIN_WORDING,
                None,
                w(V_BUILD_WRAP, "there is no native entry-inversion to wrap", SPEC_ENTRY),
            ),
            posix_no_async: reject(
                WRAP_MAIN_WORDING,
                None,
                w(V_BUILD_WRAP, "there is no native entry-inversion to wrap", SPEC_ENTRY),
            ),
            wasm32: lower(w(
                V_BUILD_WRAP,
                "a bare `fn main` is wrapped as the module's `@export` entry",
                SPEC_ENTRY,
            )),
        },
    },
    BehaviourRow {
        capability: Capability::EntryInversion(Inversion::ExportedLocusRun),
        cells: Columns {
            posix_async: lower(w(CG_EXPORT_RUN, "an `@export locus` is an ordinary locus, `run()` and all", SPEC_EXPORT)),
            posix_no_async: lower(w(CG_EXPORT_RUN, "an `@export locus` is an ordinary locus, `run()` and all", SPEC_EXPORT)),
            wasm32: reject(
                "@export locus `{locus}` must not define `run()` — {reason}",
                None,
                w(
                    CG_EXPORT_RUN,
                    "a wasm singleton is host-driven via its `@export` methods, not a cooperative run loop",
                    SPEC_ENTRY,
                ),
            ),
        },
    },
    // ---- foreign ABIs
    BehaviourRow {
        capability: Capability::ForeignAbi(Abi::C),
        cells: Columns {
            posix_async: lower(w(CG_FFI_DECL, "an `@ffi(\"c\")` fn is an external C symbol", SPEC_FFI)),
            posix_no_async: lower(w(CG_FFI_DECL, "an `@ffi(\"c\")` fn is an external C symbol", SPEC_FFI)),
            wasm32: lower(w(
                CG_FFI_DECL,
                "an `@ffi(\"c\")` fn is an `env` import, supplied by a package's `csrc` or the loader",
                SPEC_WASM_FFI,
            )),
        },
    },
    BehaviourRow {
        capability: Capability::ForeignAbi(Abi::Js),
        cells: Columns {
            posix_async: reject(JS_NATIVE_WORDING, Some(JS_NATIVE_GUIDANCE), w(ADMISSION, JS_NATIVE_REASON, SPEC_JS)),
            posix_no_async: reject(JS_NATIVE_WORDING, Some(JS_NATIVE_GUIDANCE), w(ADMISSION, JS_NATIVE_REASON, SPEC_JS)),
            wasm32: Behaviour {
                verdict: BehaviourVerdict::Lower(Lowering::IntAsF64),
                origin: Origin::Source,
                requires: &[],
                witness: w(
                    "crates/hale-codegen/src/codegen.rs::a.abi == \"js\"",
                    "an `@ffi(\"js\")` fn is a loader import; an `Int` crosses as an f64",
                    SPEC_JS,
                ),
            },
        },
    },
    // ---- FFI types: target-independent (design §2.6), one row per
    // class and ABI.
    ffi_row(FfiTypeClass::Int, Abi::C, FFI_PORTABLE),
    ffi_row(FfiTypeClass::Int, Abi::Js, FFI_PORTABLE),
    ffi_row(FfiTypeClass::Float, Abi::C, FFI_PORTABLE),
    ffi_row(FfiTypeClass::Float, Abi::Js, FFI_PORTABLE),
    ffi_row(FfiTypeClass::Bool, Abi::C, FFI_PORTABLE),
    ffi_row(FfiTypeClass::Bool, Abi::Js, FFI_PORTABLE),
    ffi_row(FfiTypeClass::String, Abi::C, FFI_PORTABLE),
    ffi_row(FfiTypeClass::String, Abi::Js, FFI_PORTABLE),
    ffi_row(FfiTypeClass::Bytes, Abi::C, FFI_PORTABLE),
    ffi_row(FfiTypeClass::Bytes, Abi::Js, FFI_PORTABLE),
    ffi_row(FfiTypeClass::BytesView, Abi::C, FFI_PORTABLE),
    ffi_row(FfiTypeClass::BytesView, Abi::Js, FFI_PORTABLE),
    ffi_row(FfiTypeClass::StringView, Abi::C, FFI_PORTABLE),
    ffi_row(FfiTypeClass::StringView, Abi::Js, FFI_PORTABLE),
    ffi_row(FfiTypeClass::BytesMut, Abi::C, FFI_PORTABLE),
    ffi_row(FfiTypeClass::BytesMut, Abi::Js, FFI_PORTABLE),
    ffi_row(FfiTypeClass::Time, Abi::C, FFI_PORTABLE),
    ffi_row(FfiTypeClass::Time, Abi::Js, FFI_PORTABLE),
    ffi_row(FfiTypeClass::Duration, Abi::C, FFI_PORTABLE),
    ffi_row(FfiTypeClass::Duration, Abi::Js, FFI_PORTABLE),
    ffi_row(FfiTypeClass::Decimal, Abi::C, FFI_DECIMAL),
    ffi_row(FfiTypeClass::Decimal, Abi::Js, FFI_DECIMAL),
    ffi_row(FfiTypeClass::Uint, Abi::C, FFI_UINT),
    ffi_row(FfiTypeClass::Uint, Abi::Js, FFI_UINT),
    ffi_row(FfiTypeClass::Unit, Abi::C, FFI_UNIT),
    ffi_row(FfiTypeClass::Unit, Abi::Js, FFI_UNIT),
    ffi_row(FfiTypeClass::Named, Abi::C, FFI_NAMED),
    ffi_row(FfiTypeClass::Named, Abi::Js, FFI_NAMED),
    ffi_row(FfiTypeClass::Bounded, Abi::C, FFI_BOUNDED),
    ffi_row(FfiTypeClass::Bounded, Abi::Js, FFI_BOUNDED),
    ffi_row(FfiTypeClass::Projection, Abi::C, FFI_PROJECTION),
    ffi_row(FfiTypeClass::Projection, Abi::Js, FFI_PROJECTION),
    ffi_row(FfiTypeClass::Array, Abi::C, FFI_ARRAY),
    ffi_row(FfiTypeClass::Array, Abi::Js, FFI_ARRAY),
    ffi_row(FfiTypeClass::Tuple, Abi::C, FFI_TUPLE),
    ffi_row(FfiTypeClass::Tuple, Abi::Js, FFI_TUPLE),
    ffi_row(FfiTypeClass::Function, Abi::C, FFI_FUNCTION),
    ffi_row(FfiTypeClass::Function, Abi::Js, FFI_FUNCTION),
    ffi_row(FfiTypeClass::Fallible, Abi::C, FFI_FALLIBLE),
    ffi_row(FfiTypeClass::Fallible, Abi::Js, FFI_FALLIBLE),
    ffi_row(FfiTypeClass::Unknown, Abi::C, FFI_UNKNOWN),
    ffi_row(FfiTypeClass::Unknown, Abi::Js, FFI_UNKNOWN),
    // ---- placement and the bus
    BehaviourRow {
        capability: Capability::AsyncIoPool,
        cells: Columns {
            posix_async: Behaviour {
                requires: &[Capability::PoolThreads],
                ..lower(w(CHECK_ASYNC_IO, "the async_io backend is epoll (glibc Linux) or kqueue (macOS) over ucontext coroutines", SPEC_ASYNC_IO))
            },
            posix_no_async: Behaviour {
                requires: &[Capability::PoolThreads],
                ..reject(
                    "placement entry `{field}`: `async_io` pools \
                     aren't supported on musl Linux yet — use a \
                     cooperative pool (drop `where async_io`), \
                     or build for glibc Linux or macOS. (The \
                     backend is epoll or kqueue over ucontext \
                     coroutines; {reason}.)",
                    Some("a cooperative pool (drop `where async_io`), or a glibc Linux or macOS build"),
                    w(CHECK_ASYNC_IO, "this target's libc has no ucontext", SPEC_ASYNC_IO),
                )
            },
            wasm32: Behaviour {
                requires: &[Capability::PoolThreads],
                ..reject(
                    WASM_ASYNC_IO_WORDING,
                    Some("use a cooperative pool on `main` (drop `where async_io` and the pool)"),
                    w(
                        CHECK_ASYNC_IO,
                        "the runtime's LOTUS_HAVE_ASYNC_IO is 1 over epoll and eventfd imports the loader stubs with `() => 0`, and the pool's workers are threads the module does not have",
                        SPEC_ASYNC_IO,
                    ),
                )
            },
        },
    },
    BehaviourRow {
        capability: Capability::PinnedThreads,
        cells: Columns {
            posix_async: lower(w(CG_PINNED_JOIN, "a pinned locus owns a POSIX thread, which main joins at dissolve", SPEC_PLACEMENT)),
            posix_no_async: lower(w(CG_PINNED_JOIN, "a pinned locus owns a POSIX thread, which main joins at dissolve", SPEC_PLACEMENT)),
            wasm32: reject(WASM_PINNED_WORDING, Some(WASM_PLACE_ON_MAIN), w(ADMISSION, WASM_ONE_THREAD, SPEC_PLACEMENT)),
        },
    },
    BehaviourRow {
        capability: Capability::PoolThreads,
        cells: Columns {
            posix_async: lower(w(CG_POOL_START, "a pool's workers are POSIX threads", SPEC_PLACEMENT)),
            posix_no_async: lower(w(CG_POOL_START, "a pool's workers are POSIX threads", SPEC_PLACEMENT)),
            wasm32: reject(WASM_POOL_WORDING, Some(WASM_PLACE_ON_MAIN), w(ADMISSION, WASM_ONE_THREAD, SPEC_PLACEMENT)),
        },
    },
    transport_row(
        Transport::Unix,
        "an AF_UNIX socket served by the runtime's transport threads",
        "an AF_UNIX socket served by the runtime's transport threads",
        "the browser sandbox has no AF_UNIX sockets",
    ),
    transport_row(
        Transport::ShmRing,
        "a POSIX shared-memory ring",
        "a POSIX shared-memory ring",
        "the browser sandbox has no shared memory to map",
    ),
    BehaviourRow {
        capability: Capability::RemoteTransport(Transport::Adapter),
        cells: Columns {
            posix_async: Behaviour { requires: &[Capability::PinnedThreads], ..lower(w(CG_BINDINGS, ADAPTER_NATIVE, SPEC_BINDINGS)) },
            posix_no_async: Behaviour { requires: &[Capability::PinnedThreads], ..lower(w(CG_BINDINGS, ADAPTER_NATIVE, SPEC_BINDINGS)) },
            wasm32: Behaviour {
                requires: &[Capability::PinnedThreads],
                ..reject(
                    WASM_TRANSPORT_WORDING,
                    Some(WASM_TRANSPORT_GUIDANCE),
                    w(ADMISSION, "an adapter's instance runs on a thread of its own, which the wasm32 module does not have", SPEC_BINDINGS),
                )
            },
        },
    },
    BehaviourRow {
        capability: Capability::BoundedWait,
        cells: Columns {
            posix_async: lower(w(RT_WAIT_SPACE, "the publisher parks until a consumer frees room, or wait-abort ends the wait", SPEC_BUS)),
            posix_no_async: lower(w(RT_WAIT_SPACE, "the publisher parks until a consumer frees room, or wait-abort ends the wait", SPEC_BUS)),
            wasm32: lower(w(
                RT_WAIT_SPACE,
                "the publisher spins on its own pump (the 1 ms nap is the shim's inline no-op) until the queue empties, or wait-abort ends the wait",
                SPEC_BUS,
            )),
        },
    },
    BehaviourRow {
        capability: Capability::ProcessSignals,
        cells: Columns {
            posix_async: Behaviour {
                origin: Origin::Environment,
                ..lower(w(CG_SIGNALS, "SIGINT and SIGTERM start the drain, and SIGPIPE is ignored", SPEC_DRAIN))
            },
            posix_no_async: Behaviour {
                origin: Origin::Environment,
                ..lower(w(CG_SIGNALS, "SIGINT and SIGTERM start the drain, and SIGPIPE is ignored", SPEC_DRAIN))
            },
            wasm32: Behaviour {
                origin: Origin::Environment,
                ..reject(
                    "{reason}",
                    None,
                    w(
                        "crates/hale-codegen/runtime/lotus_arena.c::lotus_drain_signals_install",
                        "wasm32 has no signal source: no drain signal and no SIGPIPE reach the module, and the drain flag stays 0",
                        SPEC_DRAIN,
                    ),
                )
            },
        },
    },
];

const CG_HAS_EXPORTS: &str = "crates/hale-codegen/src/codegen.rs::has_exports";

/// T5: an `@ffi("js")` declaration on a native target, refused at the
/// declaration whether or not anything calls it: the declaration is the
/// use.
const JS_NATIVE_WORDING: &str = "`@ffi(\"js\")` fn `{fn}` is a host import of the wasm32 loader, and this \
     program is built for {selector}: {reason}; {guidance}";
const JS_NATIVE_GUIDANCE: &str = "build it for wasm32 (`target wasm { }` or `--target wasm32`), or bind a C \
     library with `@ffi(\"c\")`";
const JS_NATIVE_REASON: &str = "a native build has no loader to supply it, so it would be an undefined symbol at the link";

/// T2: placement and transports on wasm32, refused at the entry or the
/// binding until each has a real lowering. The module is driven by its
/// host on one thread.
const WASM_ONE_THREAD: &str = "the wasm32 module runs on its host's one thread (the loader stubs \
     `pthread_create` with `() => 0`)";
const WASM_PINNED_WORDING: &str = "placement entry `{field}`: `pinned` is not available under {selector} — a \
     pinned locus owns a thread of its own, and {reason}; {guidance}";
const WASM_POOL_WORDING: &str = "placement entry `{field}`: a cooperative pool other than `main` is not \
     available under {selector} — its workers are threads, and {reason}; {guidance}";
const WASM_ASYNC_IO_WORDING: &str = "placement entry `{field}`: `async_io` pools aren't supported on wasm32 \
     — {guidance}. ({reason}.)";
const WASM_PLACE_ON_MAIN: &str = "place it `cooperative` (pool `main`)";
const WASM_TRANSPORT_WORDING: &str = "bindings entry `{topic}`: this transport is not available under \
     {selector} — {reason}; {guidance}";
const WASM_TRANSPORT_GUIDANCE: &str = "keep the topic in-process, or reach the host through an `@ffi(\"js\")` \
     host import";

/// An `@export`-only program on a native target (design §1.3): located
/// at its first `@export`. It replaces codegen's late, unlocated
/// "program has no `fn main()`".
const EXPORT_ONLY_WORDING: &str = "a program with no `fn main` is an export-only module, which needs wasm32: {guidance}";
const EXPORT_ONLY_GUIDANCE: &str = "declare `target wasm { }` or build with `--target wasm32`";
const EXPORT_ONLY_REASON: &str = "a native program's entry is its `fn main`";
const CG_EXPORT_RUN: &str = "crates/hale-codegen/src/codegen.rs::must not define `run()`";
const CG_FFI_DECL: &str = "crates/hale-codegen/src/codegen.rs::f.ffi.is_some()";
const CG_POOL_START: &str = "crates/hale-codegen/src/codegen.rs::lotus_coop_pool_start_all";
const CG_PINNED_JOIN: &str = "crates/hale-codegen/src/codegen.rs::pinned.tid";
const CG_BINDINGS: &str = "crates/hale-codegen/src/codegen.rs::emit_bindings_prelude";

/// A late refusal's text, `link_wasm`'s verbatim: wasm-ld's own error
/// goes to stderr, and the build reports its exit. A `pinned` placement
/// and an adapter binding met it on wasm32 (codegen's `pthread_join(i64,
/// ptr)` against the shim's i32 `pthread_t`) until T2 located them; a
/// known-open late refusal is held to it.
pub const WASM_LD_WORDING: &str = "wasm-ld failed: exit status: 1";

const ADAPTER_NATIVE: &str = "a user adapter locus on its own thread, its `send` handed to the bus runtime";
const CG_SIGNALS: &str = "crates/hale-codegen/src/codegen.rs::lotus_drain_signals_install";
const CHECK_ASYNC_IO: &str = ADMISSION;
/// The admission law, which refuses every use whose cell is `Reject`.
const ADMISSION: &str = "crates/hale-types/src/capability/uses.rs::admission_diags";
const RT_WAIT_SPACE: &str = "crates/hale-codegen/runtime/lotus_arena.c::lotus_bus_subject_wait_space";
const V_BUILD_WRAP: &str = "crates/hale-cli/src/verbs/build.rs::WRAP_MAIN_WORDING";

/// The `--wrap-main` refusal, `hale build`'s wording verbatim.
pub const WRAP_MAIN_WORDING: &str = "--wrap-main requires --target wasm32 — it \
     synthesizes the wasm @export entry from `fn main`, and \
     there is no native entry-inversion to wrap";

/// The module's fixed exports, beside the `@export` set: `link_wasm`'s
/// list.
pub const WASM_FIXED_EXPORTS: &[Export] = &[
    Export { name: "main", if_defined: true },
    Export { name: "__heap_base", if_defined: false },
    Export { name: "memory", if_defined: true },
    Export { name: "lotus_wasm_alloc", if_defined: false },
    Export { name: "lotus_wasm_set_inbox", if_defined: false },
];

/// A transport's row: lowered over the target's sockets or shared
/// memory on both POSIX columns, each written out, and refused on
/// wasm32 (T2) with the reason the sandbox gives.
const fn transport_row(
    t: Transport,
    posix_async: &'static str,
    posix_no_async: &'static str,
    wasm32: &'static str,
) -> BehaviourRow {
    BehaviourRow {
        capability: Capability::RemoteTransport(t),
        cells: Columns {
            posix_async: lower(w(CG_BINDINGS, posix_async, SPEC_BINDINGS)),
            posix_no_async: lower(w(CG_BINDINGS, posix_no_async, SPEC_BINDINGS)),
            wasm32: reject(WASM_TRANSPORT_WORDING, Some(WASM_TRANSPORT_GUIDANCE), w(ADMISSION, wasm32, SPEC_BINDINGS)),
        },
    }
}

/// An FFI type's row. The predicate does not vary by target (design
/// §2.6), so the one cell is written into each column: this is the
/// only row shape that does so, and only because the design states the
/// equality.
const fn ffi_row(class: FfiTypeClass, abi: Abi, cell: Behaviour) -> BehaviourRow {
    BehaviourRow {
        capability: Capability::FfiType(class, abi),
        cells: Columns { posix_async: cell, posix_no_async: cell, wasm32: cell },
    }
}

/// An FFI type's refusal is its reason; the checker frames it at the
/// declaration's parameter or return.
const FFI_WORDING: &str = "{reason}";
const CHECK_FFI: &str = "crates/hale-types/src/check.rs::ffi_type_unportable";

const FFI_PORTABLE: Behaviour = lower(w(CHECK_FFI, "in the FFI-portable set", SPEC_FFI_TYPES));
const FFI_NAMED: Behaviour = lower(w(
    CHECK_FFI,
    "a named user type crosses as its C struct; the library author keeps the layouts compatible",
    SPEC_FFI_TYPES,
));
const FFI_UNKNOWN: Behaviour = lower(w(
    CHECK_FFI,
    "an unresolved type name is admitted; codegen catches a broken signature at declaration",
    SPEC_FFI_TYPES,
));
const fn ffi_reject(reason: &'static str) -> Behaviour {
    reject(FFI_WORDING, None, w(CHECK_FFI, reason, SPEC_FFI_TYPES))
}
const FFI_DECIMAL: Behaviour = ffi_reject(
    "Decimal (i128) has platform-variable ABI; marshal as \
     Int/Float at the Hale side instead",
);
const FFI_UINT: Behaviour = ffi_reject(
    "Uint is Hale-internal; declare as Int in the @ffi \
     signature",
);
const FFI_UNIT: Behaviour = ffi_reject("() (unit) is not a meaningful FFI parameter type");
const FFI_BOUNDED: Behaviour = ffi_reject(
    "bounded[T; N] has no portable C mapping — pass the \
     element pointer + count separately",
);
const FFI_PROJECTION: Behaviour = ffi_reject(
    "projection-typed values (Rich / Chunked / Recognition) \
     carry per-locus metadata and don't cross the C-ABI \
     boundary",
);
const FFI_ARRAY: Behaviour = ffi_reject(
    "fixed-size arrays don't cross the C-ABI boundary at \
     Stage 1; pass Bytes / a wrapper struct instead",
);
const FFI_TUPLE: Behaviour = ffi_reject(
    "tuples have no portable C struct layout; declare a named \
     type instead",
);
const FFI_FUNCTION: Behaviour = ffi_reject(
    "function-pointer types are not yet FFI-portable; declare \
     the wrapper at the C side and pass a struct/handle",
);
const FFI_FALLIBLE: Behaviour = ffi_reject(
    "fallible(E) is an Hale internal channel; C functions \
     must return an error sentinel and the Hale wrapper \
     above translates",
);

// -------------------------------------------------------- invocations

const OPT_EXEC: &str = "crates/hale-cli/src/shared/options.rs::parse_exec_build_options";
const OPT_WASM_REFUSAL: &str = "crates/hale-cli/src/shared/options.rs::emits an artifact this host";

/// `hale run` / `hale replay` under wasm32, the CLI's wording
/// verbatim: `{cmd}` is the verb.
pub const RUN_WASM_WORDING: &str = "hale {cmd}: --target wasm32 emits an artifact this host \
     cannot execute — build it with `hale build --target \
     wasm32` and run it in a host that can";

const fn allowed(reason: &'static str, spec: &'static str) -> InvocationCell {
    InvocationCell { verdict: InvocationVerdict::Allowed, witness: w(OPT_EXEC, reason, spec) }
}
const fn refused_on_wasm(spec: &'static str) -> InvocationCell {
    InvocationCell {
        verdict: InvocationVerdict::Refused(Refusal {
            wording: RUN_WASM_WORDING,
            guidance: Some("build it with `hale build --target wasm32` and run it in a host that can"),
        }),
        witness: w(
            OPT_WASM_REFUSAL,
            "the host cannot execute the artifact, and the loader passes it no mode",
            spec,
        ),
    }
}

/// `hale run` / `hale replay` for a native target that is not the host,
/// the CLI's wording verbatim: `{triple}` is the target's.
pub const RUN_FOREIGN_WORDING: &str = "hale {cmd}: --target {triple} is not this host's platform, so \
     nothing it builds can run here — build it with `hale build \
     --target {triple}` and run it where it belongs";

/// The musl column's invocations. The CLI refuses to run any native
/// target that is not the host, and no host the compiler runs on is a
/// musl one (`TargetSpec::host`), so for musl that refusal is constant:
/// today's answer, and so the cell. (For a glibc triple the same
/// refusal depends on which machine runs the compiler, which is no
/// target's fact; the PosixAsync column states what the host's own
/// triple does.)
const fn refused_on_musl(spec: &'static str) -> InvocationCell {
    InvocationCell {
        verdict: InvocationVerdict::Refused(Refusal {
            wording: RUN_FOREIGN_WORDING,
            guidance: Some("build it with `hale build --target <musl triple>` and run it on the Linux it is for"),
        }),
        witness: w(
            "crates/hale-cli/src/shared/options.rs::is not this host's platform",
            "no host the compiler runs on is a musl one, so a musl artifact is always a cross build this host does not run",
            spec,
        ),
    }
}

const SPEC_RUN: &str = "spec/projects.md § `hale run` interaction";

pub const INVOCATIONS: &[InvocationRow] = &[
    InvocationRow {
        invocation: Invocation::Run,
        cells: Columns {
            posix_async: allowed("the host runs an artifact built for itself", SPEC_RUN),
            posix_no_async: refused_on_musl(SPEC_RUN),
            wasm32: refused_on_wasm(SPEC_RUN),
        },
    },
    InvocationRow {
        invocation: Invocation::Replay,
        cells: Columns {
            posix_async: allowed("the host replays a recording of an artifact built for itself", SPEC_RECORDING),
            posix_no_async: refused_on_musl(SPEC_RECORDING),
            wasm32: refused_on_wasm(SPEC_RECORDING),
        },
    },
    InvocationRow {
        invocation: Invocation::Record,
        cells: Columns {
            posix_async: allowed("`hale run` under LOTUS_OBS_RECORD records", SPEC_RECORDING),
            posix_no_async: refused_on_musl(SPEC_RECORDING),
            wasm32: refused_on_wasm(SPEC_RECORDING),
        },
    },
];

// -------------------------------------------------------- obligations

const fn emit(site: &'static str, reason: &'static str, spec: &'static str) -> ObligationCell {
    ObligationCell { verdict: ObligationVerdict::Emit, witness: w(site, reason, spec) }
}
const fn omit(premise: Premise, site: &'static str, reason: &'static str, spec: &'static str) -> ObligationCell {
    ObligationCell { verdict: ObligationVerdict::Omit(premise), witness: w(site, reason, spec) }
}
const fn ob_row(
    obligation: Obligation,
    posix_async: ObligationCell,
    posix_no_async: ObligationCell,
    wasm32: ObligationCell,
) -> ObligationRow {
    ObligationRow { obligation, cells: Columns { posix_async, posix_no_async, wasm32 } }
}

// The POSIX cells. A musl build emits every one of them as a glibc
// build does (the runtime has each call on both); each row names the
// cell in both columns, so that equality is written, not inherited.
const REPLAY_INGRESS: ObligationCell = emit(
    "crates/hale-codegen/src/locus/instantiation.rs::lotus_replay_start_ingress",
    "the main locus starts replay ingress at its boot/run boundary",
    SPEC_RECORDING,
);
const OBSERVATION_IDENTITY: ObligationCell = emit(
    "crates/hale-codegen/src/codegen.rs::lotus_obs_eager_init",
    "main registers the topic shapes and the execution digest, and initializes observation eagerly",
    SPEC_OBSERVATION,
);
const SIGNAL_INSTALL: ObligationCell =
    emit(CG_SIGNALS, "main ignores SIGPIPE and installs the drain signals", SPEC_DRAIN);
const DRAIN_OBSERVER: ObligationCell = emit(
    "crates/hale-codegen/src/codegen.rs::lotus.observes_drain.",
    "a locus that reads the drain is counted",
    SPEC_DRAIN,
);
const DRAIN_TERM: ObligationCell = emit(
    "crates/hale-codegen/src/codegen.rs::lotus_process_draining_flag",
    "`self.draining`, the restart gate and `sleep` read the process flag",
    SPEC_DRAIN,
);
const BINDING_CONFIG: ObligationCell = emit(
    "crates/hale-codegen/src/codegen.rs::lotus_bus_load_config",
    "main loads the transport configuration and registers the listen bindings' key extractors",
    SPEC_BINDINGS,
);
const POOL_JOIN: ObligationCell = emit(
    "crates/hale-codegen/src/codegen.rs::emit_coop_pool_shutdown_all",
    "a teardown spine joins the cooperative pools' workers",
    SPEC_OBLIGATIONS,
);
const WAIT_ABORT: ObligationCell = emit(
    "crates/hale-codegen/src/codegen.rs::emit_bus_wait_abort_all",
    "a teardown spine aborts the parked `or wait` publishers",
    SPEC_OBLIGATIONS,
);
const INGRESS_QUIESCE: ObligationCell = emit(
    "crates/hale-codegen/src/codegen.rs::emit_bus_ingress_quiesce",
    "a teardown spine drains kernel-accepted listen ingress first",
    SPEC_OBLIGATIONS,
);

const SIGNALS: Premise = Premise::Rejects(Capability::ProcessSignals);
const LISTEN_TRANSPORTS: &[Premise] = &[
    Premise::Rejects(Capability::RemoteTransport(Transport::Unix)),
    Premise::Rejects(Capability::RemoteTransport(Transport::ShmRing)),
];

pub const OBLIGATIONS: &[ObligationRow] = &[
    ob_row(
        Obligation::ReplayIngress,
        REPLAY_INGRESS,
        REPLAY_INGRESS,
        omit(
            Premise::Refuses(Invocation::Replay),
            "crates/hale-codegen/src/locus/instantiation.rs::lotus_replay_start_ingress",
            "the artifact is never replayed",
            SPEC_RECORDING,
        ),
    ),
    ob_row(
        Obligation::ObservationIdentity,
        OBSERVATION_IDENTITY,
        OBSERVATION_IDENTITY,
        omit(
            Premise::All(&[Premise::Refuses(Invocation::Replay), Premise::Refuses(Invocation::Record)]),
            "crates/hale-codegen/src/codegen.rs::lotus_obs_eager_init",
            "the artifact is neither recorded nor replayed",
            SPEC_OBSERVATION,
        ),
    ),
    ob_row(
        Obligation::SignalInstall,
        SIGNAL_INSTALL,
        SIGNAL_INSTALL,
        omit(SIGNALS, CG_SIGNALS, "no signal reaches the module", SPEC_DRAIN),
    ),
    ob_row(
        Obligation::DrainObserver,
        DRAIN_OBSERVER,
        DRAIN_OBSERVER,
        omit(SIGNALS, "crates/hale-codegen/src/codegen.rs::lotus.observes_drain.", "no drain signal arrives", SPEC_DRAIN),
    ),
    ob_row(
        Obligation::DrainTerm,
        DRAIN_TERM,
        DRAIN_TERM,
        omit(
            SIGNALS,
            "crates/hale-codegen/src/codegen.rs::lotus_process_draining_flag",
            "the process flag is always 0",
            SPEC_DRAIN,
        ),
    ),
    ob_row(
        Obligation::BindingConfig,
        BINDING_CONFIG,
        BINDING_CONFIG,
        omit(
            Premise::All(LISTEN_TRANSPORTS),
            "crates/hale-codegen/src/codegen.rs::lotus_bus_load_config",
            "no transport binding can serve the module",
            SPEC_BINDINGS,
        ),
    ),
    ob_row(
        Obligation::PoolJoin,
        POOL_JOIN,
        POOL_JOIN,
        omit(
            Premise::All(&[Premise::Rejects(Capability::PoolThreads), Premise::Rejects(Capability::AsyncIoPool)]),
            "crates/hale-codegen/src/codegen.rs::emit_coop_pool_shutdown_all",
            "no pool worker exists to join",
            SPEC_OBLIGATIONS,
        ),
    ),
    ob_row(
        Obligation::WaitAbort,
        WAIT_ABORT,
        WAIT_ABORT,
        emit(
            "crates/hale-codegen/src/codegen.rs::emit_bus_wait_abort_all",
            "the local capacity wait is admitted, and no proof yet shows no waiter is live at teardown",
            SPEC_OBLIGATIONS,
        ),
    ),
    ob_row(
        Obligation::IngressQuiesce,
        INGRESS_QUIESCE,
        INGRESS_QUIESCE,
        omit(
            Premise::All(LISTEN_TRANSPORTS),
            "crates/hale-codegen/src/codegen.rs::emit_bus_ingress_quiesce",
            "no listen binding exists to quiesce",
            SPEC_OBLIGATIONS,
        ),
    ),
];

// ----------------------------------------------------------- documents

/// A document region rendered from the matrix (design §4, T6): the
/// lines between its markers are [`render_markdown`]'s, never edited by
/// hand, and `crates/hale-types/tests/capability_doc_matches.rs` holds
/// each file to its rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocRegion {
    /// The book's statement of what wasm32 admits and refuses.
    Wasm32Statement,
    /// The spec's table of the namespaces wasm32 refuses.
    Wasm32StdlibTable,
}

impl DocRegion {
    pub const ALL: [DocRegion; 2] = [DocRegion::Wasm32Statement, DocRegion::Wasm32StdlibTable];

    /// The file that holds the region, from the repository root.
    pub fn file(self) -> &'static str {
        match self {
            DocRegion::Wasm32Statement => "docs/src/systems/webassembly.md",
            DocRegion::Wasm32StdlibTable => "spec/ffi.md",
        }
    }

    /// The line that opens the region.
    pub fn begin(self) -> String {
        let name = match self {
            DocRegion::Wasm32Statement => "wasm32",
            DocRegion::Wasm32StdlibTable => "wasm32 stdlib",
        };
        format!("<!-- capability-matrix: {name} (generated by hale_types::capability::render_markdown; do not edit) -->")
    }
}

/// The line that closes a region.
pub const DOC_REGION_END: &str = "<!-- /capability-matrix -->";

/// The constructs a program writes for the placement and bus rows, in
/// the order the book lists them.
const PLACEMENT_CONSTRUCTS: &[(Capability, &str)] = &[
    (Capability::PinnedThreads, "a `pinned` placement"),
    (Capability::PoolThreads, "`cooperative(pool = X)`, X other than `main`"),
    (Capability::AsyncIoPool, "`where async_io`"),
    (Capability::RemoteTransport(Transport::Unix), "a `unix(...)` binding"),
    (Capability::RemoteTransport(Transport::ShmRing), "a `shm_ring(...)` binding"),
    (Capability::RemoteTransport(Transport::Adapter), "an adapter binding (`T: MyAdapter { ... }`)"),
    (Capability::BoundedWait, "`or wait` on an `on_full: fail` topic"),
];

/// The entry-inversion forms as a program writes them.
fn inversion_construct(i: Inversion) -> &'static str {
    match i {
        Inversion::ExportOnly => "A program with `@export`s and no `fn main`",
        Inversion::WrapMain => "`--wrap-main` on a bare `fn main`",
        Inversion::ExportedLocusRun => "An `@export locus` that defines `run()`",
    }
}

fn sentence(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// The namespaces wasm32 refuses, in the table's order, a run of rows
/// with one cell sharing a line (`io::fs` and `io::file`).
fn refused_namespaces(m: &CapabilityMatrix) -> Vec<(Vec<&'static str>, &'static Behaviour)> {
    let mut out: Vec<(Vec<&'static str>, &'static Behaviour)> = Vec::new();
    for r in m.behaviours {
        let Capability::StdNamespace(ns) = r.capability else { continue };
        let cell = r.cells.get(TargetClass::Wasm32);
        if cell.is_lower() {
            continue;
        }
        match out.last_mut() {
            Some((names, last)) if *last == cell => names.push(ns),
            _ => out.push((vec![ns], cell)),
        }
    }
    out
}

fn std_names(names: &[&str]) -> String {
    names.iter().map(|n| format!("`std::{n}`")).collect::<Vec<_>>().join(", ")
}

/// The region's lines, each ending in a newline: what goes between its
/// markers.
pub fn render_markdown(region: DocRegion) -> String {
    let m = derive_capability_matrix();
    let mut out = String::from("\n");
    let refused = refused_namespaces(&m);
    match region {
        DocRegion::Wasm32StdlibTable => {
            out.push_str("| Rejected path | Browser substitute |\n|---|---|\n");
            for (names, cell) in &refused {
                let substitute = cell.refusal().and_then(|r| r.guidance).unwrap_or("");
                out.push_str(&format!("| {} | {substitute} |\n", std_names(names)));
            }
        }
        DocRegion::Wasm32Statement => {
            let diag = STD_WASM_WORDING
                .replace("{path}", "...")
                .replace("{selector}", "`target wasm`")
                .replace("{reason}", "<reason>");
            out.push_str(&format!(
                "**The standard library.** These namespaces are refused at typecheck, with \
                 ``error: {diag}``:\n\n| Refused | Why | Instead |\n|---|---|---|\n"
            ));
            for (names, cell) in &refused {
                let instead = cell.refusal().and_then(|r| r.guidance).unwrap_or("");
                out.push_str(&format!("| {} | {} | {instead} |\n", std_names(names), cell.witness.reason));
            }
            let stubs: Vec<&'static str> = m
                .behaviours
                .iter()
                .filter_map(|r| match r.capability {
                    Capability::StdNamespace(ns)
                        if KNOWN_OPEN.iter().any(|k| {
                            k.class == TargetClass::Wasm32 && k.cell == OpenCell::Behaviour(r.capability)
                        }) =>
                    {
                        Some(ns)
                    }
                    _ => None,
                })
                .collect();
            if !stubs.is_empty() {
                out.push_str(
                    "\nThese namespaces type-check and build under wasm32, and what they do there is a stub:\n\n\
                     | Namespace | Under wasm32 |\n|---|---|\n",
                );
                for ns in &stubs {
                    let cell = m.behaviour(TargetClass::Wasm32, Capability::StdNamespace(ns)).expect("a row");
                    out.push_str(&format!("| `std::{ns}` | {} |\n", cell.witness.reason));
                }
            }
            let available: Vec<&'static str> = m
                .behaviours
                .iter()
                .filter_map(|r| match r.capability {
                    Capability::StdNamespace(ns) if r.cells.wasm32.is_lower() && !stubs.contains(&ns) => Some(ns),
                    _ => None,
                })
                .collect();
            out.push_str(&format!("\nEvery other namespace is available: {}.\n", std_names(&available)));

            out.push_str("\n**Placement and the bus.**\n\n| Construct | Under wasm32 | Why |\n|---|---|---|\n");
            for (cap, construct) in PLACEMENT_CONSTRUCTS {
                let cell = m.behaviour(TargetClass::Wasm32, *cap).expect("a row");
                let holes = [("field", "<field>"), ("topic", "<Topic>"), ("selector", "`target wasm`")];
                let verdict = match cell.refusal() {
                    Some(r) => format!("refused: ``{}``", r.render(&cell.witness, &holes)),
                    None => "admitted".to_string(),
                };
                out.push_str(&format!("| {construct} | {verdict} | {} |\n", cell.witness.reason));
            }

            let link = m.behaviour(TargetClass::Wasm32, Capability::LinkLibrary).expect("a row");
            let wording = link.refusal().map(|r| r.render(&link.witness, &[("libs", "[...]")])).unwrap_or_default();
            out.push_str(&format!("\n**Linking.** `[ffi] link` is refused:\n\n> {wording}\n"));

            let exports = m.behaviour(TargetClass::Wasm32, Capability::ExportSurface).expect("a row");
            let fixed: Vec<String> = match exports.verdict {
                BehaviourVerdict::Lower(Lowering::Exports(list)) => list
                    .iter()
                    .map(|e| if e.if_defined { format!("`{}` (when defined)", e.name) } else { format!("`{}`", e.name) })
                    .collect(),
                _ => Vec::new(),
            };
            out.push_str(&format!(
                "\n**Exports and the entry.** {}: {}.\n\n",
                sentence(exports.witness.reason),
                fixed.join(", ")
            ));
            for i in Inversion::ALL {
                let cell = m.behaviour(TargetClass::Wasm32, Capability::EntryInversion(i)).expect("a row");
                let verdict = match cell.refusal() {
                    Some(r) => format!("refused, with ``{}``", r.render(&cell.witness, &[("locus", "L")])),
                    None => format!("admitted — {}", cell.witness.reason),
                };
                out.push_str(&format!("- {}: {verdict}.\n", inversion_construct(i)));
            }
        }
    }
    out.push('\n');
    out
}

#[cfg(test)]
mod laws;
pub mod uses;

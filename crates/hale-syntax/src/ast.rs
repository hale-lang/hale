//! Abstract syntax tree types.
//!
//! Each node carries a [`Span`] so diagnostics can locate it in
//! source. Nodes are owned values (no arenas, no string interning
//! at this layer) — Phase 1 prioritizes simplicity over allocator
//! optimization. Compile-time perf is secondary per F.1.

use crate::span::Span;

#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub imports: Vec<Import>,
    pub items: Vec<TopDecl>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Import {
    pub path: String,
    pub alias: Option<String>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TopDecl {
    Locus(LocusDecl),
    Perspective(PerspectiveDecl),
    Type(TypeDecl),
    Const(ConstDecl),
    Fn(FnDecl),
    Module(ModuleDecl),
    Interface(InterfaceDecl),
    Topic(TopicDecl),
    /// shm-ring-interop Proposal B (2026-06-06): `ring_layout Name
    /// { ... }` — declares the byte layout of an externally-defined
    /// SHM broadcast ring so a `shm_ring(..., layout: Name)` binding
    /// can read it. Pure declaration; lowers to a bounded accessor.
    RingLayout(RingLayoutDecl),
    /// FUv0.8.2 #7 (2026-05-25): `target <name> { cap.path,
    /// cap.path, ... }` — names a substrate (e.g.
    /// `browser-js`, `native`) plus its capability profile.
    /// A program reaches only what its target offers; reaching
    /// further fails at the translation boundary with
    /// CAP-MISSING. v0.2 surface lands here as a parser /
    /// AST commit; the capability-enforcement pass is v0.3.
    Target(TargetDecl),
}

impl TopDecl {
    pub fn span(&self) -> Span {
        match self {
            TopDecl::Locus(l) => l.span,
            TopDecl::Perspective(p) => p.span,
            TopDecl::Type(t) => t.span,
            TopDecl::Const(c) => c.span,
            TopDecl::Fn(f) => f.span,
            TopDecl::Module(m) => m.span,
            TopDecl::Interface(i) => i.span,
            TopDecl::Topic(t) => t.span,
            TopDecl::RingLayout(r) => r.span,
            TopDecl::Target(t) => t.span,
        }
    }
}

/// `topic Foo [: Parent] { payload: T; subject: "..."; }` — names
/// a typed bus channel.
///
/// Phase 1 carried only `payload`. Phase 2 adds:
///   - **`subject:`** — explicit wire-format string. When omitted,
///     defaults to the topic's local name (composed with parent's
///     subject when nested).
///   - **`: Parent`** — declarative parent topic, builds the
///     hierarchical tree the closed-world topology analysis
///     consumes (per The Design / vertical-only-flow). Wire
///     subject derives from the parent chain.
///
/// Phase 3 will add `transport:` classification once stdlib's
/// `interface Transport` is in place; for now external-vs-intra
/// is derived purely from `main.bindings`.
#[derive(Debug, Clone, PartialEq)]
pub struct TopicDecl {
    pub name: Ident,
    /// Optional declarative parent — `topic Login : Events { ... }`.
    /// `None` means this topic is at the root of its tree. Resolution
    /// looks the parent up by name; cycles are rejected.
    pub parent: Option<Ident>,
    /// Required: the payload type carried by this topic. Every
    /// subscriber's handler must take a single param of this type;
    /// every publisher's `Foo <- expr` must produce this type.
    pub payload: TypeExpr,
    /// Optional explicit wire subject. When `None`, defaults to
    /// the topic's local name (joined with parent's subject path
    /// at desugar time).
    pub subject: Option<String>,
    /// Routing-key field (Phase 3 — `spec/semantics.md` §
    /// "Phase 3: routing keys"). When `Some`, the bus shards
    /// dispatch by the named payload field's value at the
    /// `(subject, key)` granularity. Field must exist on
    /// `payload` and resolve to an int-shaped scalar at
    /// typecheck. When `None`, the topic is unkeyed and behaves
    /// as it has since Phase 1.
    pub keyed_by: Option<Ident>,
    /// Behavior when a keyed publish finds no subscriber whose
    /// `where key == X` filter matches. `None` is equivalent to
    /// `Some(Swallow)` (the default; matches today's no-subscriber
    /// semantics on unkeyed topics). Has no meaning on unkeyed
    /// topics — typecheck rejects.
    pub on_unmatched: Option<UnmatchedPolicy>,
    pub span: Span,
}

/// shm-ring-interop Proposal B: a declared byte layout for an
/// externally-defined SHM broadcast ring. Members are parsed
/// loosely (idents + ints); the layout *contract* — known reprs /
/// orderings / framing kinds, sane offsets — is enforced in
/// `hale-types::check`. See `notes/binary-shm-ring-interop.md`.
#[derive(Debug, Clone, PartialEq)]
pub struct RingLayoutDecl {
    pub name: Ident,
    /// `magic 0x...;` — expected header magic (None = unchecked).
    pub magic: Option<i64>,
    /// `data_at OFF;` — first-record byte offset.
    pub data_at: Option<i64>,
    /// Header scalar fields: `name [expect] at OFF : repr;`.
    pub scalars: Vec<RingScalarField>,
    /// `cursor [name] { attr value; ... }` (one or more).
    pub cursors: Vec<RingCursorBlock>,
    /// `framing KIND { attr value; ... }`.
    pub framing: Option<RingFramingBlock>,
    /// `overflow KIND;`.
    pub overflow: Option<Ident>,
    pub span: Span,
}

/// A header scalar: `version 1 at 8 : u32;` — name `version`, an
/// optional validated `expect` value (1), byte offset (8), and a
/// width/repr token (`u32`). The repr is a layout token, not a Hale
/// `TypeExpr`, so it's an `Ident` checked against a known set.
#[derive(Debug, Clone, PartialEq)]
pub struct RingScalarField {
    pub name: Ident,
    pub expect: Option<i64>,
    pub at: i64,
    pub repr: Ident,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RingCursorBlock {
    /// `cursor published { ... }` → Some("published"); bare
    /// `cursor { ... }` → None.
    pub name: Option<Ident>,
    pub attrs: Vec<RingAttr>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RingFramingBlock {
    /// `byte_records` | `slots`.
    pub kind: Ident,
    pub attrs: Vec<RingAttr>,
    pub span: Span,
}

/// A `key value;` pair inside a `cursor`/`framing` block, e.g.
/// `at 64`, `repr atomic_u64`, `len_prefix u32`, `align 8`.
#[derive(Debug, Clone, PartialEq)]
pub struct RingAttr {
    pub key: Ident,
    pub value: RingAttrValue,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RingAttrValue {
    Ident(Ident),
    Int(i64),
}

/// Routing-key `on_unmatched` policy. See `spec/semantics.md`
/// § "Phase 3: routing keys".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnmatchedPolicy {
    /// Drop the message silently. Diag visible only with
    /// `LOTUS_BUS_LOG_UNMATCHED=1`. Default when omitted.
    Swallow,
    /// Publish becomes a fallible expression. Caller must use
    /// `or raise` / `or handler(err)` / `or discard`.
    Fail,
    /// At least one `where key == _` subscriber on the subject
    /// catches the unmatched message. Resolve-time check
    /// enforces.
    Fallback,
}

/// FUv0.8.2 #7 (2026-05-25): `target <name> { <cap>, ... }`.
///
/// `name` is a substrate identifier — `browser-js`, `native`,
/// `wasm`, `embedded`, etc. The compiler doesn't enforce a
/// closed set; downstream tooling (codegen, the LSP) maps
/// recognized names to runtime backends.
///
/// `capabilities` is a list of dotted-path capability names:
/// `time.monotonic`, `gfx.canvas2d`, `arenas.epoch_view`. The
/// dot-separated form is structural — the resolver can map
/// each capability to a set of stdlib paths it gates
/// (e.g. `time.monotonic` → `std::time::monotonic`). v0.2
/// ships the AST + parser surface; the actual gate
/// (rejecting programs that reach beyond the target's
/// capability profile) is a v0.3 typecheck pass.
#[derive(Debug, Clone, PartialEq)]
pub struct TargetDecl {
    pub name: Ident,
    pub capabilities: Vec<Capability>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Capability {
    /// Dot-separated path segments, e.g. `["time", "monotonic"]`.
    pub segments: Vec<Ident>,
    pub span: Span,
}

/// Structural interface — a named set of method signatures. Any
/// locus that declares the same set of methods (by name + arity
/// + types) implicitly satisfies the interface; no `impl I for L`
/// declaration. Cross-locus polymorphism uses fat-pointer
/// dispatch (data + vtable). Interfaces are the v0 answer to
/// the Sink-as-tagged-locus friction. Per spec/design-rationale.md
/// F.20.
#[derive(Debug, Clone, PartialEq)]
pub struct InterfaceDecl {
    pub name: Ident,
    /// Method signatures the interface requires. Order is
    /// significant — vtable layout follows declaration order
    /// so that `vtable[i]` corresponds to `methods[i]`. Method
    /// bodies are not allowed (no default methods at v0).
    pub methods: Vec<InterfaceMethodSig>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InterfaceMethodSig {
    pub name: Ident,
    pub params: Vec<Param>,
    pub ret: Option<TypeExpr>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LocusDecl {
    pub name: Ident,
    /// Phase 2: when set, this locus is the binary's entry point —
    /// `main locus App { ... }`. Carries `bindings { }`
    /// configuration for cross-process topics. Exactly one
    /// `main locus` per binary (validated downstream); a
    /// non-main locus carrying a `bindings { }` block is a
    /// parse error.
    pub is_main: bool,
    /// m63: optional generic param list on the locus
    /// declaration. `locus Cache<K, V> { ... }` parses to a
    /// non-empty Vec; non-generic loci leave this empty.
    /// Codegen monomorphizes on use sites — generic templates
    /// emit no LLVM IR directly.
    pub generics: Vec<GenericParam>,
    pub annotations: Vec<LocusAnnotation>,
    /// v1.x-FORM-1: optional `@form(<name>, <args>...)` annotation
    /// that sits above the `locus` keyword. Picks an efficient
    /// lowering and synthesizes a standard method set. One form
    /// per locus in v1.
    pub form: Option<FormAnnotation>,
    /// F.32-2 v0.2 (2026-05-25): optional `@locality(L1|L2|L3|any)`
    /// annotation. When present, the F.32-2 working-set
    /// estimator evaluates this locus against the named cache
    /// tier's budget (or, for `any`, explicitly opts out of the
    /// global `--target-cache` gate).
    pub locality: Option<LocalityAnnotation>,
    pub members: Vec<LocusMember>,
    pub span: Span,
}

/// v1.x-FORM-1: `@form(<name>, <args>...)` annotation.
///
/// Decorates a locus declaration above the `locus` keyword. The
/// `name` identifies one of the compiler-recognized forms
/// (`vec`, `hashmap`, `ring_buffer` in v1). The optional `args`
/// are keyword-style configuration the form's runtime consults
/// (e.g. `cap = 64` for `@form(ring_buffer)`); storage-discipline
/// configuration goes on capacity slot clauses instead (e.g.
/// `indexed_by name` for `@form(hashmap)`).
#[derive(Debug, Clone, PartialEq)]
pub struct FormAnnotation {
    pub name: Ident,
    pub args: Vec<FormArg>,
    pub span: Span,
}

/// F.32-2 v0.2 (2026-05-25): `@locality(L1|L2|L3|any)`
/// annotation on a locus declaration. Pins a per-locus cache-
/// tier budget that the working-set estimator evaluates
/// against. `Any` is the explicit opt-out from any global
/// `--target-cache` gate (no budget for this locus).
#[derive(Debug, Clone, PartialEq)]
pub struct LocalityAnnotation {
    pub tier: LocalityTier,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalityTier {
    L1,
    L2,
    L3,
    Any,
}

/// `@ffi("c")` annotation on a free fn declaration. Marks the fn
/// as an external C-ABI binding: parser accepts no body
/// (terminator is `;`), typechecker validates the parameter and
/// return types against the FFI-portable subset (see
/// `spec/ffi.md`), codegen emits an LLVM `declare` rather than a
/// `define`. Call sites lower as direct calls to the symbol named
/// by the fn — no `__std_*` mangling.
///
/// `abi` is currently always `"c"`; future ABIs (e.g. `"system"`
/// for Windows stdcall) would extend the set. Annotation only
/// valid on top-level free fns at Stage 1; locus methods,
/// perspective methods, and interface signatures may not carry it.
#[derive(Debug, Clone, PartialEq)]
pub struct FfiAnnotation {
    pub abi: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FormArg {
    pub name: Ident,
    pub value: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LocusAnnotation {
    Tier(i64),
    Projection(ProjectionClass),
    // F.31 (2026-05-23): schedule annotation removed from the
    // locus declaration site. Placement is a deployment seam;
    // see PlacementBlock + LocusMember::Placement.
}

#[derive(Debug, Clone, PartialEq, Copy)]
pub enum ProjectionClass {
    Rich,
    Chunked,
    /// Recognition class. As a locus *annotation* the user MUST
    /// commit to a sub-mode at the declaration site
    /// (`: projection recognition(cap=N, fixed_cell)` and
    /// friends), so the variant carries Some(params). As a
    /// *type expression* (`Recognition<T>` in a signature) no
    /// allocator commitment exists at the use site, so the
    /// variant carries None. Locked 2026-05-12 per v1.x-3
    /// handoff: no default sub-mode at locus declarations; bare
    /// `: projection recognition` is a parse error.
    Recognition(Option<RecognitionParams>),
}

/// Parameters attached to a `: projection recognition(...)`
/// locus annotation. `cap` is the child-count cap; `sub_mode`
/// picks the allocator strategy. The cell stride is *not* a
/// user knob — it's derived at codegen time from the union of
/// accept-method param types on this locus. The forcing-function
/// at the surface is still: user names cap and sub-mode at the
/// declaration site (same shape as the two-channel rule).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RecognitionParams {
    pub cap: u64,
    pub sub_mode: RecognitionSubMode,
}

/// Storage discipline picked by the user inside
/// `recognition(cap=N, <sub_mode>)`. v1 ships `FixedCell` and
/// `SharedSlab`; `Spillover` and `SummaryOnly` parse + typecheck
/// but reject at codegen with a "v1.x pending" diagnostic
/// (mirrors the v1.x-4 / v1.x-4b surface-then-runtime split).
/// Cell stride for FixedCell / SharedSlab / Spillover is derived
/// at codegen time from accept-method param types — not a
/// user-supplied byte budget.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RecognitionSubMode {
    FixedCell,
    Spillover,
    SummaryOnly,
    SharedSlab,
}

/// Per-locus execution strategy. Same source, two runtime
/// shapes — substrate-invariance applied to time, kept honestly
/// bimodal: either you share a scheduler thread or you own one.
/// Anything between (the temptation called "greedy" — sharing
/// but refusing to yield) is a bimodality violation: cooperative
/// already gives handler-level atomicity (no preemption within a
/// substrate cell), so the only thing "greedy" added was "don't
/// yield BETWEEN cells either" — which means leaving the shared
/// scheduler entirely. The place you go when you leave is your
/// own thread. That's pinned. Two classes; no third position.
///
/// - `Cooperative`: shared scheduler thread; yields between
///   substrate cells (handler exits, lifecycle transitions, bus
///   dispatches, `time::sleep`). Handler bodies are atomic.
///   Default.
/// - `Pinned`: owns its own OS thread, optionally pinned to a
///   CPU core. Bus events to/from cross thread boundaries via
///   formal mailbox post. For latency-critical paths or work
///   that genuinely belongs in a deeper layer of the lotus.
///
/// m25 wires the annotation through parse / resolve. m26 ships
/// cooperative semantics (deferred bus dispatch + scheduler
/// loop). m27 ships pinned threads. m28a lifts pinned to full
/// lifecycle. m28b adds cross-thread bus mailboxes. m28c adds
/// optional `pinned(core=N)` for explicit CPU-core affinity.
#[derive(Debug, Clone, PartialEq, Copy)]
pub enum ScheduleClass {
    Cooperative,
    /// Pinned to its own thread. Optional CPU core: `Some(n)`
    /// asks the runtime to `pthread_setaffinity_np` the spawned
    /// thread to logical CPU `n`; `None` lets the OS scheduler
    /// pick. Spec/runtime.md::Schedule classes.
    Pinned(Option<i64>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum LocusMember {
    Params(ParamsBlock),
    Contract(ContractBlock),
    Bus(BusBlock),
    Lifecycle(LifecycleDecl),
    Mode(ModeDecl),
    Failure(FailureDecl),
    Closure(ClosureDecl),
    Fn(FnDecl),
    Const(ConstDecl),
    Type(TypeDecl),
    /// F.22 capacity-tuple: zero or more named storage slots
    /// (`pool X of T;` / `heap Y of T;`) declared inside a
    /// `capacity { ... }` block. Slot 0 (the locus's own Arena)
    /// stays implicit — capacity declarations cover slots 1..N.
    Capacity(CapacityBlock),
    /// Phase 2: `bindings { Topic: <transport>; }` block. Valid
    /// only inside `main locus`. Each entry binds one declared
    /// topic to a concrete transport, marking that topic as
    /// external for the closed-world topology classification.
    Bindings(BindingsBlock),
    /// F.31 (2026-05-23): `placement { field: spec; }` block.
    /// Valid only inside `main locus`. Each entry picks the
    /// thread placement for a main-locus `params` field. Same
    /// deployment-seam shape as `bindings { }` — schedule was
    /// moved from a per-locus annotation to a main-only block.
    Placement(PlacementBlock),
    /// F.27 v2 (2026-05-20): `birth_check { EXPR } -> violate
    /// NAME;` synthesis hook. After birth() completes (and birth-
    /// epoch closures fire), each declared birth_check expression
    /// is evaluated; if it returns true, the named closure
    /// violates with the locus's fully-constructed state. The
    /// alternative — calling `violate NAME;` inside birth() —
    /// works but leaves the locus partially constructed (some
    /// fields set, others at defaults) when the violation's
    /// payload-capture reads happen. `birth_check` runs at a
    /// well-defined point where every field has its declared
    /// post-birth value, so the parent's `on_failure` handler
    /// sees coherent state. Multiple birth_check clauses on a
    /// locus are evaluated in declaration order; the first to
    /// fire short-circuits the rest.
    BirthCheck(BirthCheckDecl),
}

/// F.27 v2: declarative birth-time invariant check. The locus
/// has a healthy birth iff `cond` evaluates to false; a true
/// result violates `closure_name` with the locus's full post-
/// birth state.
#[derive(Debug, Clone, PartialEq)]
pub struct BirthCheckDecl {
    /// The boolean predicate evaluated after birth(). Reads
    /// `self.X` like any other locus body. `true` means "this
    /// locus is in an inconsistent post-birth state."
    pub cond: Expr,
    /// The closure name to violate. Must be a declared epoch-
    /// inline closure on the same locus (same constraint as
    /// regular `violate`).
    pub closure_name: Ident,
    /// Optional payload expression. Same shape as the
    /// `violate NAME(payload)` syntax in fn bodies.
    pub payload: Option<Expr>,
    pub span: Span,
}

/// F.31 (2026-05-23): per-locus thread placement inside `main
/// locus`. Parallel to `BindingsBlock` for bus topology —
/// both are deployment seams. Each entry picks the placement
/// for one main-locus `params` field.
#[derive(Debug, Clone, PartialEq)]
pub struct PlacementBlock {
    pub entries: Vec<PlacementEntry>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlacementEntry {
    /// Main-locus `params` field name being placed. Must be a
    /// snake_case Ident matching a declared params field of the
    /// enclosing main locus. Per F.31 the placement keys on
    /// field name (instance-level), not on locus type name,
    /// so two siblings of the same locus type can take distinct
    /// placements.
    pub field: Ident,
    pub spec: PlacementSpec,
    /// F.35 (2026-05-28): optional `where <constraint>, ...`
    /// suffix on a placement entry. Empty when no `where` clause
    /// was written. Each constraint is a bare identifier matched
    /// against `PlacementConstraint::from_ident`. Validity rules
    /// (e.g. `async_io` rejected on pinned, pool consistency
    /// across entries) live at typecheck.
    pub constraints: Vec<SpannedPlacementConstraint>,
    pub span: Span,
}

/// F.31: per-entry thread placement specification.
#[derive(Debug, Clone, PartialEq)]
pub enum PlacementSpec {
    /// `cooperative` or `cooperative(pool = X)`. The locus shares
    /// pool `X`'s OS thread with other cooperative loci placed
    /// on the same pool. `None` means pool `main` (the program's
    /// main OS thread) — equivalent to writing
    /// `cooperative(pool = main)`.
    Cooperative { pool: Option<Ident> },
    /// `pinned` or `pinned(core = N)`. The locus owns its own
    /// OS thread; `Some(n)` asks the runtime to
    /// `pthread_setaffinity_np` the spawned thread to logical
    /// CPU `n`.
    Pinned { core: Option<i64> },
}

/// F.35 (2026-05-28): operational-constraint keyword on a
/// `placement { }` entry. Same surface pattern as
/// `BindingConstraint` (Form K) — user assertions about how the
/// deployment route should run, validated by the typechecker,
/// consumed by codegen to pick a lowering strategy.
///
/// Surface: `field: cooperative(pool = X) where async_io;`.
/// `where` is the reserved keyword token; constraint names are
/// contextual idents matched against `from_ident`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlacementConstraint {
    /// `async_io` — the pool's worker drain loop integrates an
    /// epoll instance; blocking I/O syscalls inside locus methods
    /// on this pool park-and-resume instead of blocking the OS
    /// thread. Rejected on pinned entries (pinned owns its
    /// thread — no shared drain to park on) and on pool `main`
    /// (no dedicated worker thread). Requires every entry on the
    /// same pool to declare it (mixed-mode-on-same-pool is a
    /// typecheck error).
    AsyncIo,
}

impl PlacementConstraint {
    pub fn from_ident(name: &str) -> Option<Self> {
        match name {
            "async_io" => Some(Self::AsyncIo),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::AsyncIo => "async_io",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpannedPlacementConstraint {
    pub kind: PlacementConstraint,
    pub span: Span,
}

/// Phase 2: per-topic transport binding inside `main locus`.
#[derive(Debug, Clone, PartialEq)]
pub struct BindingsBlock {
    pub entries: Vec<BindingEntry>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BindingEntry {
    /// Topic name being bound — must reference a declared topic.
    pub topic: Ident,
    /// Transport constructor — currently `unix(...)` only; future
    /// `Adapter(LocusLiteral)` variant lands in Wave B of the
    /// bus-transport redesign. Absence-of-binding means same-
    /// process via the cooperative queue (no variant needed).
    pub transport: TransportSpec,
    /// Form K (2026-05-20): operational constraints the dev team
    /// asserts on this binding's route. Empty when the binding
    /// declares only a transport. The typechecker validates that
    /// (a) the transport variant satisfies every constraint and
    /// (b) each topic's payload shape is compatible with the
    /// constraints (e.g. `zero_copy` requires
    /// `is_flat_shapeable` on the topic's payload type).
    ///
    /// Surface: `Topic: unix("/sock") where intra_machine,
    /// zero_copy;`. `where` is a contextual keyword (recognized
    /// in binding-entry position only).
    pub constraints: Vec<SpannedBindingConstraint>,
    /// F.36 Slice 2 (2026-05-28): optional `codec(L { ... })`
    /// clause that wires a pluggable codec for this binding. The
    /// named locus must structurally provide `encode(v: T) ->
    /// Bytes fallible(EncodeError)` and `decode(b: Bytes) -> T
    /// fallible(DecodeError)`, where T is the topic's payload
    /// type. Both methods must be pure (Slice 1's purity
    /// inference asserts at this binding site). When `None`, the
    /// binding uses the m70 default serializer.
    pub codec: Option<CodecSpec>,
    pub span: Span,
}

/// F.36 Slice 2: pluggable codec instance attached to a binding
/// entry. The `locus` names the codec type; `inits` are the
/// instantiation overrides (same shape as adapter loci or any
/// other struct literal).
#[derive(Debug, Clone, PartialEq)]
pub struct CodecSpec {
    pub locus: Ident,
    pub inits: Vec<StructInit>,
    pub span: Span,
}

/// Form K (2026-05-20): individual operational-constraint
/// keyword on a `bindings { }` entry.
///
/// Constraints are user assertions about the route the
/// transport must satisfy. They split into two orthogonal
/// axes:
///   - **Scope** (where the bus can reach): `intra_process`,
///     `intra_machine`, `cross_machine`. Hierarchy is
///     `intra_process ⊂ intra_machine ⊂ cross_machine`.
///   - **Behavior** (operational requirements): `zero_copy`.
///     Future: `durable`, `ordered`, ...
///
/// Multiple constraints from one axis on a single binding is
/// the user's choice — the typechecker accepts them as `AND`.
/// Validity (e.g. `zero_copy` + `cross_machine` is a
/// contradiction) is enforced downstream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingConstraint {
    IntraProcess,
    IntraMachine,
    CrossMachine,
    ZeroCopy,
}

impl BindingConstraint {
    pub fn from_ident(name: &str) -> Option<Self> {
        match name {
            "intra_process" => Some(Self::IntraProcess),
            "intra_machine" => Some(Self::IntraMachine),
            "cross_machine" => Some(Self::CrossMachine),
            "zero_copy" => Some(Self::ZeroCopy),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::IntraProcess => "intra_process",
            Self::IntraMachine => "intra_machine",
            Self::CrossMachine => "cross_machine",
            Self::ZeroCopy => "zero_copy",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpannedBindingConstraint {
    pub kind: BindingConstraint,
    pub span: Span,
}

/// Bus transport constructors. v1.x ships exactly one substrate-
/// provided variant (`Unix`); a future Wave B adds `Adapter(locus)`
/// for user-supplied protocol-layer transports (NATS, MQTT, TCP-
/// with-framing, etc.) once interface-value storage (G20 / F.20
/// Phase B) lands. In-memory delivery is "absence of binding
/// entry," not a variant.
#[derive(Debug, Clone, PartialEq)]
pub enum TransportSpec {
    /// `unix("/path/to/sock")` or `unix("/path", role: listen)`
    /// — AF_UNIX domain socket. Substrate-provided: the runtime's
    /// `lotus_transport_*` owns the delivery contract directly,
    /// no protocol layer involved. Role is optional at the syntax
    /// level; the typechecker fills it in from the bus block when
    /// unambiguous (`publish`-only → connect, `subscribe`-only →
    /// listen). When both publish and subscribe touch the same
    /// topic, role must be explicit.
    Unix {
        path: String,
        role: Option<TransportRole>,
        span: Span,
    },
    /// `MyNatsAdapter { url: "nats://...", ... }` — user-supplied
    /// protocol-layer transport. The named locus must structurally
    /// satisfy `__StdBusAdapter` (i.e. expose
    /// `fn send(subject: String, bytes: Bytes)`). At codegen the
    /// locus is instantiated with program-lifetime allocation, its
    /// `send` method's fn pointer is resolved, and the pair is
    /// handed to the bus runtime via
    /// `lotus_bus_register_remote_adapter`. Wave B of the
    /// bus-transport redesign; shipped 2026-05-18.
    Adapter {
        locus: Ident,
        inits: Vec<StructInit>,
        span: Span,
    },
    /// Form K4b (2026-05-20): POSIX SHM ring transport backing the
    /// zero-copy bus route. `shm_ring("/ring_name",
    /// slot_count: N, on_overflow: <policy>)` — name is the SHM
    /// object name (passed directly to `shm_open`), slot_count
    /// is the ring depth, on_overflow is the back-pressure
    /// policy (Form K7, REQUIRED — no default).
    ///
    /// Implies `zero_copy`. Satisfies `intra_machine`. Rejects
    /// `intra_process` (SHM is cross-process by design) and
    /// `cross_machine` (POSIX SHM is host-local). Payload must
    /// be `is_flat_shapeable`; the typechecker rejects bindings
    /// where the topic's payload contains variadic fields.
    ///
    /// Slot size is derived at codegen from the payload type.
    /// slot_count defaults to 128 when not specified; the
    /// memory cost is `slot_count * slot_size`.
    ShmRing {
        name: String,
        slot_count: u64,
        overflow: ShmRingOverflow,
        /// shm-ring-interop Proposal B: `layout: <Ident>` names a
        /// `ring_layout` declaration describing a foreign ring's byte
        /// layout. `None` = the native `LotusRing` shape (the only
        /// behavior today; back-compat).
        layout: Option<Ident>,
        /// Proposal B M3a: producer ring capacity in bytes (the data
        /// region size). Only meaningful with `layout:` (a foreign
        /// `byte_records` ring has no fixed slots); `None` = the
        /// per-transport default. Ignored for the native ring, which
        /// sizes from `slot_count` × payload size.
        buffer_size: Option<u64>,
        span: Span,
    },
}

/// Form K7 (2026-05-20): publisher-side back-pressure policy
/// for a `shm_ring(...)` binding. Required at the binding site —
/// there is intentionally no default. Forces the user to think
/// about back-pressure semantics for each high-throughput
/// topic.
///
/// Discriminator runtime ABI: matches the C
/// `lotus_shm_overflow_policy_t` enum. Block=0, Drop=1, Fail=2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShmRingOverflow {
    /// `block` — when the ring is full, the publisher spins +
    /// sleeps until a consumer catches up. No timeout in v1;
    /// deadlocks if the consumer dies. Right for control-plane
    /// topics where latency doesn't matter and data must not
    /// be lost.
    Block,
    /// `drop` — when the ring is full, the publisher overwrites
    /// unread slots (today's behavior). Right for stale-is-
    /// worthless feeds (market data tickers, telemetry).
    /// Consumers may silently miss messages.
    Drop,
    /// `fail` — when the ring is full, the publisher panics
    /// with a clear diagnostic. v1's hard-failure form of
    /// the long-promised `fallible(ClaimError)` shape.
    /// Graceful caller-side handling via fallible-`<-` is a
    /// post-K7 follow-up; today, fail = process exits.
    Fail,
}

impl ShmRingOverflow {
    pub fn from_ident(name: &str) -> Option<Self> {
        match name {
            "block" => Some(Self::Block),
            "drop" => Some(Self::Drop),
            "fail" => Some(Self::Fail),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Block => "block",
            Self::Drop => "drop",
            Self::Fail => "fail",
        }
    }

    /// Runtime ABI discriminator. Must match the C
    /// `lotus_shm_overflow_policy_t` enum.
    pub fn runtime_tag(self) -> i32 {
        match self {
            Self::Block => 0,
            Self::Drop => 1,
            Self::Fail => 2,
        }
    }
}

/// Direction-of-traffic for point-to-point substrate transports.
/// Broker-shaped or user-supplied adapters carry direction in
/// their own params blocks, not at the binding-spec level.
#[derive(Debug, Clone, PartialEq, Copy)]
pub enum TransportRole {
    Connect,
    Listen,
}

/// F.22 `capacity { ... }` block: a flat list of slot
/// declarations. Order is significant — slot init runs in
/// declaration order at instantiation; slot teardown runs in
/// reverse declaration order at dissolve.
#[derive(Debug, Clone, PartialEq)]
pub struct CapacityBlock {
    pub slots: Vec<CapacitySlot>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CapacitySlot {
    pub name: Ident,
    pub kind: CapacitySlotKind,
    pub elem_ty: TypeExpr,
    /// F.22 v1.x-4 slot parent-override: when a slot is declared
    /// with `as_parent_for ChildL`, any `ChildL` accepted by this
    /// locus gets its same-named slot pointer overridden with
    /// this parent's allocator at accept time. Generalizes the
    /// chunked-class slot-0 sub-region handoff to slots 1..N.
    /// `None` for ordinary slots that own their allocator.
    pub as_parent_for: Option<Ident>,
    /// v1.x-FORM-4 slot key-by-field: when a slot is declared
    /// with `indexed_by <fieldname>`, the named field of the
    /// cell type serves as the hashmap key. Only meaningful on
    /// `@form(hashmap)` loci; typecheck flags misuse on other
    /// shapes. `None` for ordinary slots.
    pub indexed_by: Option<Ident>,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapacitySlotKind {
    /// Fixed-size cell recycling (`pool entries of Int;`).
    /// Population is bounded; release-acquire rolls memory
    /// through cells without touching the OS.
    Pool,
    /// Individually-freed cells with locus-bounded lifetime
    /// (`heap registry of Command;`). Wholesale teardown at
    /// slot destroy frees any still-live cells.
    Heap,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParamsBlock {
    pub params: Vec<ParamDecl>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParamDecl {
    pub name: Ident,
    pub ty: Option<TypeExpr>,
    pub init: ParamInit,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParamInit {
    Value(Expr),
    Inferred,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContractBlock {
    pub kind: ContractKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ContractKind {
    Inferred,
    Members(Vec<ContractMember>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContractMember {
    pub direction: ContractDirection,
    pub name: ContractName,
    pub ty: Option<TypeExpr>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Copy)]
pub enum ContractDirection {
    Expose,
    Consume,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ContractName {
    Named(Ident),
    Inferred,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BusBlock {
    pub members: Vec<BusMember>,
    pub span: Span,
}

/// A bus subscribe / publish / send site addresses its channel
/// either through a literal subject string (legacy form,
/// `subscribe "log.error" as h of type LogEvent;`) or through a
/// named `topic Foo { ... }` reference. Both flow through the
/// same downstream typecheck + codegen — the topic form gets its
/// payload from the topic decl rather than the call-site `of
/// type T` clause.
#[derive(Debug, Clone, PartialEq)]
pub enum BusSubject {
    /// Legacy bare-string subject. Carries the literal subject
    /// text plus the source span of the string token.
    Literal { subject: String, span: Span },
    /// `topic Foo` reference. The Ident span is the topic name's
    /// source location for diagnostics.
    Topic(Ident),
    /// A7 (G16): multi-segment cross-seed topic reference,
    /// e.g. `subscribe alias::Foo as h;`. Parsed when the bus
    /// subject is a qualified path. The codegen pre-pass at
    /// `build_executable_with_imports` resolves the alias chain
    /// through the per-build path-rename table and rewrites this
    /// variant into a single-segment `Topic(Ident(mangled_name))`
    /// before the existing desugar runs. Multi-segment subjects
    /// never reach desugar/codegen proper.
    QualifiedTopic(QualifiedName),
}

impl BusSubject {
    pub fn span(&self) -> Span {
        match self {
            BusSubject::Literal { span, .. } => *span,
            BusSubject::Topic(i) => i.span,
            BusSubject::QualifiedTopic(qn) => qn.span,
        }
    }

    /// The wire-format subject string this site addresses. For
    /// literal subjects, the string itself. For topic refs, the
    /// topic name. Used by codegen + runtime, which run after
    /// the topic-desugaring pass and see only `Literal` variants
    /// in practice — but this method works on the unnormalized
    /// AST too, so callers can read subjects without branching.
    pub fn canonical(&self) -> &str {
        match self {
            BusSubject::Literal { subject, .. } => subject.as_str(),
            BusSubject::Topic(i) => i.name.as_str(),
            BusSubject::QualifiedTopic(qn) => qn
                .segments
                .last()
                .map(|s| s.name.as_str())
                .unwrap_or(""),
        }
    }
}

impl std::fmt::Display for BusSubject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BusSubject::QualifiedTopic(qn) => {
                let parts: Vec<&str> =
                    qn.segments.iter().map(|s| s.name.as_str()).collect();
                f.write_str(&parts.join("::"))
            }
            _ => f.write_str(self.canonical()),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum BusMember {
    Subscribe {
        subject: BusSubject,
        handler: Ident,
        /// `of type T` clause. Required for literal-subject form;
        /// MUST be `None` for topic-ref form (the topic carries
        /// the payload type). Typecheck enforces this constraint.
        ty: Option<TypeExpr>,
        /// Routing-key filter (Phase 3). `None` = receive every
        /// message on the subject (today's behavior). `Some(...)`
        /// = filter at dispatch time. See `spec/semantics.md` §
        /// "Phase 3: routing keys".
        key_filter: Option<KeyFilter>,
        span: Span,
    },
    Publish {
        subject: BusSubject,
        /// `of type T` clause. Same constraint as `Subscribe.ty`.
        ty: Option<TypeExpr>,
        alias: Option<Ident>,
        span: Span,
    },
}

/// `where key == EXPR` clause on `subscribe`. See
/// `spec/semantics.md` § "Phase 3: routing keys".
#[derive(Debug, Clone, PartialEq)]
pub enum KeyFilter {
    /// `where key == EXPR`. The expression is evaluated at the
    /// subscribing locus's instantiation; the value is captured
    /// into the bus registry. v0.1 restricts EXPR to literal /
    /// const ident / `self.<field>` (typecheck enforces).
    Specific { expr: Expr, span: Span },
    /// `where key == _`. Catch-unmatched subscriber. Only legal
    /// when the topic declares `on_unmatched: fallback`.
    Unmatched { span: Span },
}

impl KeyFilter {
    pub fn span(&self) -> Span {
        match self {
            KeyFilter::Specific { span, .. } | KeyFilter::Unmatched { span } => *span,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LifecycleDecl {
    pub kind: LifecycleKind,
    pub params: Vec<Param>,
    pub ret: Option<TypeExpr>,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Copy)]
pub enum LifecycleKind {
    Birth,
    Accept,
    /// 2026-05-30 — death-side bookend, symmetric to `accept`.
    /// `release(c: Child) { ... }` fires when an accept'd child of
    /// type `Child` completes (its run() returns), after the child
    /// drains and before it dissolves, so the parent observes the
    /// completion and reads the child's final state. Declaring it
    /// also marks `Child` a "flow": its run() completing reclaims it
    /// (vs. a "resident" child, which lives until the parent
    /// dissolves). Same shape as `accept`: one typed child param.
    Release,
    Run,
    Drain,
    Dissolve,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModeDecl {
    pub kind: ModeKind,
    pub params: Vec<Param>,
    pub ret: Option<TypeExpr>,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Copy)]
pub enum ModeKind {
    Bulk,
    Harmonic,
    Resolution,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FailureDecl {
    pub params: Vec<Param>,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClosureDecl {
    pub name: Ident,
    /// v1.x-VIOLATE (F.27): assertion is optional. `epoch
    /// inline` closures have no audit band and omit the
    /// assertion entirely. For all other epochs the assertion
    /// is required (enforced at typecheck, not at parse).
    pub assertion: Option<ClosureAssertion>,
    pub clauses: Vec<ClosureClause>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClosureAssertion {
    pub left: Expr,
    pub right: Expr,
    pub tolerance: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClosureClause {
    Epoch(EpochSpec),
    PersistsThrough(Vec<Ident>),
    ResetsOn(Vec<Ident>),
    /// v1.x-WINDOWED (F.34): after the assertion fires at a
    /// `duration(N)` epoch boundary, the runtime zeros the
    /// listed locus fields. Lets a closure express a per-window
    /// rate budget without the user re-implementing the reset
    /// dance. Only meaningful when paired with `epoch duration(...)`.
    ResetsPerEpoch(Vec<Ident>),
    /// v1.x-VIOLATE (F.27): names locus fields whose values are
    /// snapshotted into the synthesized `ClosureViolation`
    /// payload at fire time. Only meaningful when paired with
    /// `epoch inline`.
    Captures(Vec<Ident>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum EpochSpec {
    Tick,
    Duration(Expr),
    Birth,
    Dissolve,
    Explicit,
    /// v1.x-VIOLATE (F.27): pull-only epoch. The closure never
    /// fires automatically; only `violate NAME;` fires it.
    Inline,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PerspectiveDecl {
    pub name: Ident,
    pub generics: Vec<GenericParam>,
    pub members: Vec<PerspectiveMember>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PerspectiveMember {
    Params(ParamsBlock),
    StableWhen(Block),
    SerializeAs(TypeExpr),
    Fn(FnDecl),
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypeDecl {
    pub name: Ident,
    pub generics: Vec<GenericParam>,
    pub body: TypeDeclBody,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TypeDeclBody {
    Alias(TypeExpr),
    Struct(Vec<StructField>),
    Enum(Vec<EnumVariant>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct StructField {
    pub name: Ident,
    pub ty: TypeExpr,
    pub default: Option<Expr>,
    /// Go-style metadata tag: the raw text of a backtick string written
    /// after the field (`price: Int `wire:"u32_le"`;`). Free-form
    /// `key:"value"` metadata that downstream features parse — the
    /// binary-pack layer (Proposal A′) reads a `wire:` key; ignored
    /// otherwise. The backtick lexer is shared with time literals, which
    /// only occur in expression position; in field-declaration position
    /// it's a tag.
    pub tag: Option<String>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnumVariant {
    pub name: Ident,
    pub fields: Vec<TypeExpr>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConstDecl {
    pub name: Ident,
    pub ty: TypeExpr,
    pub value: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FnDecl {
    pub name: Ident,
    pub generics: Vec<GenericParam>,
    pub params: Vec<Param>,
    pub ret: Option<TypeExpr>,
    /// v1.x-FORM-1: optional `fallible(T)` marker. When present,
    /// the fn can fail with a payload of type T; call sites
    /// MUST address the error via an `or` clause (see
    /// [`Expr::Or`]) or a `match`. Inside the body, `fail <expr>`
    /// (see [`Stmt::Fail`]) exits via the error path with the
    /// expression as the typed payload.
    pub fallible: Option<TypeExpr>,
    /// Stage-1 FFI (2026-05-22): optional `@ffi("c")` annotation
    /// marking this fn as an external C-ABI binding. When `Some`,
    /// `body` is a synthesized empty block (parser accepts no
    /// body; terminator is `;`) and downstream passes (typecheck,
    /// codegen) take the FFI-specific code paths. See
    /// `notes/ffi-design.md` and `spec/ffi.md`.
    pub ffi: Option<FfiAnnotation>,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModuleDecl {
    pub name: Ident,
    pub items: Vec<TopDecl>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GenericParam {
    pub name: Ident,
    pub bound: Option<TypeExpr>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: Ident,
    pub ty: TypeExpr,
    pub default: Option<Expr>,
    pub span: Span,
}

// === Type expressions =====================================

#[derive(Debug, Clone, PartialEq)]
pub enum TypeExpr {
    Primitive(PrimType, Span),
    Named {
        path: QualifiedName,
        generic_args: Vec<TypeExpr>,
        span: Span,
    },
    Projection {
        class: ProjectionClass,
        inner: Box<TypeExpr>,
        span: Span,
    },
    Array {
        elem: Box<TypeExpr>,
        size: Option<Expr>,
        span: Span,
    },
    Tuple(Vec<TypeExpr>, Span),
    Function {
        params: Vec<TypeExpr>,
        ret: Option<Box<TypeExpr>>,
        span: Span,
    },
}

impl TypeExpr {
    pub fn span(&self) -> Span {
        match self {
            TypeExpr::Primitive(_, s) => *s,
            TypeExpr::Named { span, .. } => *span,
            TypeExpr::Projection { span, .. } => *span,
            TypeExpr::Array { span, .. } => *span,
            TypeExpr::Tuple(_, s) => *s,
            TypeExpr::Function { span, .. } => *span,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Copy)]
pub enum PrimType {
    Int,
    Uint,
    Float,
    Decimal,
    String,
    Bool,
    Time,
    Duration,
    Bytes,
    /// F.30 (2026-05-20): non-owning view over a BytesBuilder's
    /// buffer. Runtime layout identical to `Bytes` (same
    /// `[i64 len][u8 data]` pointer), but typecheck-distinct.
    /// Returned by `BytesBuilder.view()`. Coerces to `Bytes`
    /// implicitly at function-argument READ positions; rejected
    /// at storage positions whose declared type is `Bytes`
    /// (caller must explicitly `.clone_to_bytes()` for owned
    /// storage). Storage AS `BytesView` is allowed and signals
    /// the non-owning intent in the type signature.
    BytesView,
    /// F.30 companion: non-owning view over a BytesBuilder's
    /// NUL-terminated buffer (same lifetime contract as
    /// `BytesView`; same construction site, just the C-string
    /// reading shape). Returned by `BytesBuilder.text_view()`.
    /// Coerces to `String` at fn-arg read positions, rejected
    /// at `String`-typed storage.
    StringView,
    /// #3 (2026-06-13): a raw `{ptr, len}` writable/readable window —
    /// the A1 zero-copy ring-write slot and a `MirrorRing` readable /
    /// writable window. Runtime layout is the view struct `{ptr, i64}`
    /// (here `{base, len}`, NOT the `[i64 len][data]` Bytes shape), so
    /// `std::bytes::{read_*, at, find_byte, write_*}` read/write it
    /// directly via their `_raw` siblings. Length is carried in the
    /// struct, not a prefix.
    BytesMut,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QualifiedName {
    pub segments: Vec<Ident>,
    pub span: Span,
}

// === Statements and expressions ===========================

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    /// Trailing expression (no `;`, immediately before the closing
    /// `}`). When the block is used as an expression (Expr::Block
    /// body, Expr::If arm body), this is the block's value. When
    /// the block is used in stmt-context (function body, loop
    /// body, etc.), the tail is lowered for side effects and its
    /// value is discarded — symmetric with how Rust treats a
    /// trailing expression at the end of a statement-context block.
    pub tail: Option<Box<Expr>>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Let {
        is_mut: bool,
        name: Ident,
        ty: Option<TypeExpr>,
        value: Expr,
        span: Span,
    },
    /// Tuple-destructuring let: `let (a, b) = pair;` (or with
    /// `mut`). Flat only — nested patterns wait until a real
    /// need surfaces. The value's type must be a tuple of
    /// matching arity; each `names[i]` binds the i-th component
    /// in the surrounding scope.
    LetTuple {
        is_mut: bool,
        names: Vec<Ident>,
        ty: Option<TypeExpr>,
        value: Expr,
        span: Span,
    },
    Assign {
        target: LValue,
        op: AssignOp,
        value: Expr,
        span: Span,
    },
    If(IfStmt),
    Match(MatchStmt),
    For {
        name: Ident,
        iter: Expr,
        body: Block,
        span: Span,
    },
    While {
        cond: Expr,
        body: Block,
        span: Span,
    },
    Return(Option<Expr>, Span),
    Break(Span),
    Continue(Span),
    /// v1.x-FORM-1: `fail <expr>;` — symmetric to `return` but
    /// exits via the error path of a fallible fn. The expression
    /// is the typed payload; the fn must be declared
    /// `fallible(T)` and the payload type must match T at
    /// typecheck.
    Fail {
        value: Expr,
        span: Span,
    },
    /// Explicit cooperative yield point (m26b). `yield;` drains the
    /// program-wide bus queue at this point, processing any
    /// pending substrate cells. Per spec/runtime.md cooperative
    /// yield points include "explicit `yield` (rare, for
    /// long-running computations)" — the implicit yield points
    /// (handler exit, lifecycle transition, bus dispatch) cover
    /// most cases; `yield` is for the exceptional long-internal-
    /// loop case where you want pending events to fire mid-body.
    Yield(Span),
    /// Ends the current locus's lifecycle from inside one of its own
    /// methods — the locus analogue of `return` (which ends a fn).
    /// `terminate;` sets the locus's `__drain_requested` latch and
    /// exits the current method; when the method/run coro completes
    /// with the latch set, the runtime runs the locus's normal
    /// drain → dissolve → reclaim. Only valid inside a locus method
    /// body. The in-grain "I'm done, reclaim me" signal for an
    /// accept'd child (e.g. a connection that hit EOF) — it INVOKES
    /// the declarative teardown early, never a manual free.
    Terminate(Span),
    Block(Block),
    Recovery {
        op: RecoveryOp,
        args: Vec<Expr>,
        modifier: Option<RecoveryModifier>,
        span: Span,
    },
    /// v1.x-VIOLATE (F.27): `violate NAME;` or
    /// `violate NAME with <expr>;`. Statement-level, divergent
    /// (typechecked as Never). `name` resolves to a closure
    /// declared on the enclosing locus; the typechecker enforces
    /// that the target closure is `epoch inline`. The optional
    /// payload becomes a `payload` field on the synthesized
    /// ClosureViolation; named field snapshots come from the
    /// closure's `captures:` clause.
    Violate {
        name: Ident,
        payload: Option<Expr>,
        span: Span,
    },
    /// Bus send: `subject <- value;`. The `subject` expression must
    /// resolve to a string-typed value naming a publish-declared
    /// channel; the value's type must match the channel's declared
    /// payload type. The compiler verifies both at type-check time.
    ///
    /// Phase 3 routing keys (2026-05-25): when the targeted topic
    /// declares `on_unmatched: fail`, the publish becomes a
    /// fallible expression — `K <- value or raise` /
    /// `or handler(err)` / `or discard` / `or fail <payload>` —
    /// and the optional `or_disposition` carries the handler. The
    /// typechecker requires it on fail-topic publishes and
    /// rejects it on swallow/fallback/unkeyed publishes.
    Send {
        subject: Expr,
        value: Expr,
        or_disposition: Option<OrDisposition>,
        span: Span,
    },
    /// A1 zero-copy ring write: `Topic.write(max) { w => body ; len }`.
    /// Reserves up to `max` bytes in the (layout-bound) `topic`'s ring,
    /// binds the writable view `binding` (a `BytesMut` over the slot) over
    /// the `body`, then commits the byte count the body's tail expression
    /// yields. The reserve/commit are scoped to the body so the view can't
    /// escape and the commit can't be forgotten.
    ShmWrite {
        topic: Ident,
        max: Box<Expr>,
        binding: Ident,
        body: Block,
        span: Span,
    },
    Expr(Expr),
}

#[derive(Debug, Clone, PartialEq, Copy)]
pub enum AssignOp {
    Eq,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    PercentEq,
    AmpEq,
    PipeEq,
    CaretEq,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LValue {
    pub head: Ident,
    pub tail: Vec<LValueSeg>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LValueSeg {
    Field(Ident),
    Index(Expr),
}

#[derive(Debug, Clone, PartialEq)]
pub struct IfStmt {
    pub cond: Expr,
    pub then_block: Block,
    pub else_block: Option<Box<ElseBranch>>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ElseBranch {
    Else(Block),
    ElseIf(IfStmt),
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatchStmt {
    pub scrutinee: Expr,
    pub arms: Vec<MatchArm>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub guard: Option<Expr>,
    pub body: MatchArmBody,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MatchArmBody {
    Expr(Expr),
    Block(Block),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    Literal(Literal, Span),
    Wildcard(Span),
    Binding(Ident),
    Constructor {
        path: QualifiedName,
        args: Vec<Pattern>,
        span: Span,
    },
    Tuple(Vec<Pattern>, Span),
}

/// Recovery primitives invokable from `on_failure` bodies.
///
/// m55 (per The Design's vertical-only-flow): the vocabulary is
/// **restart / restart_in_place / quarantine / bubble +
/// reorganize**. `drain` and `dissolve` are lifecycle methods,
/// not recovery operations — invoking them in `on_failure`
/// would overlap with `bubble(err)` (failure propagates up,
/// runs the lifecycle teardown). Two spellings for one concept
/// violates substrate-invariance, so v0.1 removes them from
/// the recovery vocabulary entirely. To end a locus's role on
/// failure, use `bubble(err)`.
#[derive(Debug, Clone, PartialEq, Copy)]
pub enum RecoveryOp {
    Restart,
    RestartInPlace,
    Quarantine,
    Reorganize,
    Bubble,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RecoveryModifier {
    For(Expr),
    Until(Expr),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Literal(Literal, Span),
    Ident(Ident),
    Path(QualifiedName),
    KwSelf(Span),

    Binary {
        op: BinOp,
        left: Box<Expr>,
        right: Box<Expr>,
        span: Span,
    },
    Unary {
        op: UnaryOp,
        operand: Box<Expr>,
        span: Span,
    },
    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
        span: Span,
    },
    Field {
        receiver: Box<Expr>,
        name: Ident,
        span: Span,
    },
    Index {
        receiver: Box<Expr>,
        index: Box<Expr>,
        span: Span,
    },
    Path2 {
        receiver: Box<Expr>,
        name: Ident,
        span: Span,
    },
    Tuple(Vec<Expr>, Span),
    Array(Vec<Expr>, Span),
    Struct {
        path: QualifiedName,
        inits: Vec<StructInit>,
        span: Span,
    },
    Block(Block),
    If(Box<IfStmt>),
    Match(Box<MatchStmt>),
    Sum(Box<Expr>, Span),
    Prod(Box<Expr>, Span),
    /// Approximate-equality assertion. Only valid inside a closure
    /// block; the parser produces this only in that context.
    Approx {
        left: Box<Expr>,
        right: Box<Expr>,
        tolerance: Box<Expr>,
        span: Span,
    },
    /// Integer range `lo..hi` (exclusive) or `lo..=hi`
    /// (inclusive). v0 surface only allows ranges in for-loop
    /// iterator position; using one elsewhere lowers to nothing
    /// useful in codegen and is rejected at typecheck.
    Range {
        lo: Box<Expr>,
        hi: Box<Expr>,
        inclusive: bool,
        span: Span,
    },
    /// Array-literal repetition `[val; N]`. Evaluates `val` once
    /// and fills an N-element fixed array with the result. `count`
    /// must be a const Int literal at v0 (no const evaluation
    /// engine); the parser enforces that by accepting only
    /// integer-literal counts at parse time. Resolves
    /// `notes/hale-friction.md` 2026-05-10 float-surface-gaps
    /// sub-bullet 3 (`[0.0; 8]` enumeration noise).
    ArrayRepeat {
        val: Box<Expr>,
        count: u64,
        span: Span,
    },
    /// v1.x-FORM-1: `<inner> or <disposition>` — addresses the
    /// error of a fallible call. `inner` must be of fallible
    /// type at typecheck; `disposition` is either `raise`
    /// (convert to closure violation) or a substitute
    /// expression (use as fallback value). Right-associative:
    /// `a() or b() or raise` parses as
    /// `a() or (b() or raise)`.
    ///
    /// On the substitute RHS, the identifier `err` is in scope
    /// (implicit binding) and resolves to the typed payload —
    /// this is a typecheck rule, not a syntactic one. From the
    /// AST's view, the substitute body is just an ordinary
    /// expression.
    Or {
        inner: Box<Expr>,
        disposition: OrDisposition,
        span: Span,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum OrDisposition {
    /// `or raise` — diverge by raising a closure violation
    /// carrying the fallible's payload. The expression's value
    /// type collapses to the underlying success type since this
    /// branch doesn't return.
    Raise(Span),
    /// `or <expr>` — substitute the fallback value. The
    /// expression must be of the success type at typecheck.
    Substitute(Box<Expr>),
    /// `or discard` — swallow the error and substitute Unit.
    /// Sugar for `or noop(err)` with a no-op handler — the agent
    /// pattern the wordfreq-corpus library-shape handoff
    /// surfaced. The underlying call's success type MUST be
    /// Unit; otherwise the typechecker rejects with a hint to
    /// use an explicit substitute value.
    Discard(Span),
    /// B3 / G6 — `or fail <payload>`: symmetric to `or raise` but
    /// the err branch builds a fresh payload value of the
    /// enclosing fallible fn's declared error type, then exits
    /// via the error path. Lets a caller translate one error
    /// payload into another without bouncing through a helper.
    Fail(Box<Expr>, Span),
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Expr::Literal(_, s) => *s,
            Expr::Ident(i) => i.span,
            Expr::Path(p) => p.span,
            Expr::KwSelf(s) => *s,
            Expr::Binary { span, .. } => *span,
            Expr::Unary { span, .. } => *span,
            Expr::Call { span, .. } => *span,
            Expr::Field { span, .. } => *span,
            Expr::Index { span, .. } => *span,
            Expr::Path2 { span, .. } => *span,
            Expr::Tuple(_, s) => *s,
            Expr::Array(_, s) => *s,
            Expr::Struct { span, .. } => *span,
            Expr::Block(b) => b.span,
            Expr::If(i) => i.span,
            Expr::Match(m) => m.span,
            Expr::Sum(_, s) => *s,
            Expr::Prod(_, s) => *s,
            Expr::Approx { span, .. } => *span,
            Expr::Range { span, .. } => *span,
            Expr::ArrayRepeat { span, .. } => *span,
            Expr::Or { span, .. } => *span,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct StructInit {
    pub name: Ident,
    pub value: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Copy)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    NotEq,
    Lt,
    Gt,
    LtEq,
    GtEq,
    And,
    Or,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
}

#[derive(Debug, Clone, PartialEq, Copy)]
pub enum UnaryOp {
    Neg,
    Not,
    BitNot,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    Int(i64),
    Float(f64),
    Decimal(String),
    String(String),
    Bool(bool),
    Nil,
    Duration(i64),
    Time(String),
    Bytes(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Ident {
    pub name: String,
    pub span: Span,
}

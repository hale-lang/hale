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
        }
    }
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
    Schedule(ScheduleClass),
}

#[derive(Debug, Clone, PartialEq, Copy)]
pub enum ProjectionClass {
    Rich,
    Chunked,
    /// Recognition class. As a locus *annotation* the user MUST
    /// commit to a sub-mode at the declaration site
    /// (`: projection recognition(cap=N, fixed_cell(bytes=K))`
    /// and friends), so the variant carries Some(params). As a
    /// *type expression* (`Recognition<T>` in a signature) no
    /// allocator commitment exists at the use site, so the
    /// variant carries None. Locked 2026-05-12 per v1.x-3 handoff:
    /// no default sub-mode at locus declarations; bare
    /// `: projection recognition` is a parse error.
    Recognition(Option<RecognitionParams>),
}

/// v1.x-3: parameters attached to a `: projection recognition(...)`
/// locus annotation. `cap` is the child-count cap; `sub_mode` picks
/// the allocator strategy. Both are commitments the user writes
/// down at the declaration site — same forcing-function shape as
/// the 2026-05-12 two-channel rule (name the channel at the
/// declaration site).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RecognitionParams {
    pub cap: u64,
    pub sub_mode: RecognitionSubMode,
}

/// v1.x-3: storage discipline picked by the user inside
/// `recognition(cap=N, <sub_mode>)`. v1 ships `FixedCell` and
/// `SharedSlab`; `Spillover` and `SummaryOnly` parse + typecheck
/// but reject at codegen with a "v1.x pending" diagnostic
/// (mirrors the v1.x-4 / v1.x-4b surface-then-runtime split).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RecognitionSubMode {
    FixedCell { bytes: u64 },
    Spillover { bytes: u64 },
    SummaryOnly,
    SharedSlab { bytes: u64 },
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

#[derive(Debug, Clone, PartialEq)]
pub enum BusMember {
    Subscribe {
        subject: String,
        handler: Ident,
        ty: Option<TypeExpr>,
        span: Span,
    },
    Publish {
        subject: String,
        ty: TypeExpr,
        alias: Option<Ident>,
        span: Span,
    },
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
    pub assertion: ClosureAssertion,
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
}

#[derive(Debug, Clone, PartialEq)]
pub enum EpochSpec {
    Tick,
    Duration(Expr),
    Birth,
    Dissolve,
    Explicit,
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
    Block(Block),
    Recovery {
        op: RecoveryOp,
        args: Vec<Expr>,
        modifier: Option<RecoveryModifier>,
        span: Span,
    },
    /// Bus send: `subject <- value;`. The `subject` expression must
    /// resolve to a string-typed value naming a publish-declared
    /// channel; the value's type must match the channel's declared
    /// payload type. The compiler verifies both at type-check time.
    Send {
        subject: Expr,
        value: Expr,
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
    /// `notes/aperio-friction.md` 2026-05-10 float-surface-gaps
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

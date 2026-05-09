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
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LocusDecl {
    pub name: Ident,
    pub annotations: Vec<LocusAnnotation>,
    pub members: Vec<LocusMember>,
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
    Recognition,
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

//! Moving a parsed program to another base.
//!
//! A file's parse product does not depend on the base it was parsed
//! at except through its source positions, so the frontend parses a
//! file once at base 0 and reuses that product wherever the file
//! lands in a load, moving it to the file's base there (F.40 phase 3,
//! X1). [`shift_program`] is that move: every `Span` and `Pos` in the
//! tree, and nothing else.
//!
//! Every AST type is matched exhaustively, struct patterns without
//! `..` and enum matches without a `_` arm, and every field is visited
//! whatever its type (a field with no position in it shifts as a
//! no-op). A field or variant added to `ast.rs` therefore fails to
//! compile here until it is handled, rather than quietly keeping its
//! base-0 position. The oracle below checks the result against a real
//! parse at the base over every `.hl` file in the repository.
//!
//! The parser keeps no source offset outside a `Span`: an f-string
//! interpolation's byte range lives on the lexer's token, which
//! `lex_at` moves, and the parser turns it into the sub-parse's spans
//! before any AST exists.

use crate::ast::*;
use crate::span::{Pos, Span};

/// Offset every position the parse recorded in `prog` by `delta` bytes:
/// what `parse_source_at_in(source, base, classes)` returns is
/// `parse_source_at_in(source, 0, classes)` with `shift_program(&mut p, base)`.
pub fn shift_program(prog: &mut Program, delta: u32) {
    prog.shift(Move::By(delta));
}

/// Offset every position in one declaration by `delta` bytes (wrapping,
/// so a declaration moves either way): an edit before it moves it whole
/// (F.40 phase 3, X2, the typing stage's reuse).
pub fn shift_item(item: &mut TopDecl, delta: u32) {
    item.shift(Move::By(delta));
}

/// Set every position in one declaration to zero: two declarations
/// equal after it are one declaration wherever their text sits and
/// however its whitespace runs (X2: what an edit changed, apart from
/// where it put things).
pub fn erase_positions(item: &mut TopDecl) {
    item.shift(Move::Erase);
}

/// What the walk does to each position.
#[derive(Clone, Copy)]
enum Move {
    By(u32),
    Erase,
}

trait Shift {
    fn shift(&mut self, d: Move);
}

impl Shift for Span {
    fn shift(&mut self, d: Move) {
        *self = match d {
            Move::By(d) => self.shifted(d),
            Move::Erase => Span::new(0, 0),
        };
    }
}

impl Shift for Pos {
    fn shift(&mut self, d: Move) {
        *self = match d {
            Move::By(d) => self.shifted(d),
            Move::Erase => Pos(0),
        };
    }
}

/// Values that carry no position. Listing them, rather than skipping
/// such fields at the use site, keeps every struct visiting every
/// field, so a field whose type changes to one with spans is covered.
macro_rules! no_position {
    ($($t:ty),* $(,)?) => {
        $(impl Shift for $t {
            fn shift(&mut self, _: Move) {}
        })*
    };
}

no_position!(String, bool, u8, u16, u32, u64, i64, f64);

impl<T: Shift> Shift for Vec<T> {
    fn shift(&mut self, d: Move) {
        for x in self {
            x.shift(d);
        }
    }
}

impl<T: Shift> Shift for Option<T> {
    fn shift(&mut self, d: Move) {
        if let Some(x) = self {
            x.shift(d);
        }
    }
}

impl<T: Shift> Shift for Box<T> {
    fn shift(&mut self, d: Move) {
        (**self).shift(d);
    }
}

impl<A: Shift, B: Shift> Shift for (A, B) {
    fn shift(&mut self, d: Move) {
        self.0.shift(d);
        self.1.shift(d);
    }
}

/// Shift each named binding.
macro_rules! sh {
    ($d:ident; $($x:ident),* $(,)?) => {{
        $($x.shift($d);)*
    }};
}

/// A struct: destructured without `..`, every field shifted.
macro_rules! shift_struct {
    ($t:ident { $($f:ident),* $(,)? }) => {
        impl Shift for $t {
            fn shift(&mut self, d: Move) {
                let $t { $($f),* } = self;
                sh!(d; $($f),*);
            }
        }
    };
}

/// An enum of unit variants only: matched without a `_` arm, so a new
/// variant (which might carry a position) has to be listed here.
macro_rules! shift_unit_enum {
    ($t:ident { $($v:ident),* $(,)? }) => {
        impl Shift for $t {
            fn shift(&mut self, _: Move) {
                match self {
                    $($t::$v)|* => {}
                }
            }
        }
    };
}

// Top level.

shift_struct!(Program {
    effect_names,
    declared_effects,
    effect_defs,
    imports,
    items,
    span,
});
shift_struct!(EffectClasses { names, declared, defs });
shift_struct!(Import { path, alias, span, path_span });

impl Shift for TopDecl {
    fn shift(&mut self, d: Move) {
        match self {
            TopDecl::Locus(x) => x.shift(d),
            TopDecl::Perspective(x) => x.shift(d),
            TopDecl::Type(x) => x.shift(d),
            TopDecl::Const(x) => x.shift(d),
            TopDecl::Fn(x) => x.shift(d),
            TopDecl::Module(x) => x.shift(d),
            TopDecl::Interface(x) => x.shift(d),
            TopDecl::Topic(x) => x.shift(d),
            TopDecl::RingLayout(x) => x.shift(d),
            TopDecl::Target(x) => x.shift(d),
            TopDecl::Group(x) => x.shift(d),
            TopDecl::Role(x) => x.shift(d),
            TopDecl::Claims(x) => x.shift(d),
            TopDecl::Constitution(x) => x.shift(d),
        }
    }
}

// Claims vocabulary.

shift_struct!(RoleDecl { name, includes, span });
shift_struct!(GroupDecl { name, members, may_be_empty, span, id });
shift_struct!(ConstitutionDecl { name, extends, entries, span });
shift_struct!(GroupMember { segments, glob, span });
shift_struct!(ClaimsBlock { entries, adopts, lib_tier, span });
shift_struct!(ClaimDecl { name, form, span });

impl Shift for ClaimForm {
    fn shift(&mut self, d: Move) {
        match self {
            ClaimForm::ForbidReaches { src, dst, via_calls, via_bus, during, avoiding } => {
                sh!(d; src, dst, via_calls, via_bus, during, avoiding)
            }
            ClaimForm::OnlyEdges { src, dst, grants } => sh!(d; src, dst, grants),
            ClaimForm::Bound { class, class_name, class_span, limit, from } => {
                sh!(d; class, class_name, class_span, limit, from)
            }
            ClaimForm::Require { publishers, group, topic } => sh!(d; publishers, group, topic),
            ClaimForm::RequireSealed { group } => sh!(d; group),
            ClaimForm::RequireAttributed { class_name } => sh!(d; class_name),
            ClaimForm::Cover { alias, group } => sh!(d; alias, group),
            ClaimForm::Count { publishers, topic, cmp, n } => sh!(d; publishers, topic, cmp, n),
        }
    }
}

shift_unit_enum!(CountCmp { Eq, Le, Ge });
shift_struct!(TopicRef { segments, span });
shift_struct!(EdgeGrant { publish, topic, span });

impl Shift for ClaimSet {
    fn shift(&mut self, d: Move) {
        match self {
            ClaimSet::Group(x) => x.shift(d),
            ClaimSet::Effects { class, name, span } => sh!(d; class, name, span),
        }
    }
}

// Topics, ring layouts, targets, interfaces.

shift_struct!(TopicDecl {
    name,
    display,
    parent,
    payload,
    subject,
    keyed_by,
    bounded,
    on_full_fail,
    on_unmatched,
    span,
    id,
});
shift_struct!(RingLayoutDecl {
    name,
    magic,
    data_at,
    scalars,
    cursors,
    framing,
    overflow,
    span,
});
shift_struct!(RingScalarField { name, expect, at, repr, span });
shift_struct!(RingCursorBlock { name, attrs, span });
shift_struct!(RingFramingBlock { kind, attrs, span });
shift_struct!(RingAttr { key, value, span });

impl Shift for RingAttrValue {
    fn shift(&mut self, d: Move) {
        match self {
            RingAttrValue::Ident(x) => x.shift(d),
            RingAttrValue::Int(x) => x.shift(d),
        }
    }
}

shift_unit_enum!(UnmatchedPolicy { Swallow, Fail, Fallback });
shift_struct!(TargetDecl { name, capabilities, span });
shift_struct!(Capability { segments, span });
shift_struct!(InterfaceDecl { name, methods, span, id });
shift_struct!(InterfaceMethodSig { name, params, ret, fallible, span });

// Loci and their annotations.

shift_struct!(LocusDecl {
    name,
    imported,
    display,
    is_main,
    export,
    generics,
    annotations,
    serves,
    form,
    locality,
    bounded,
    phase_effects,
    depends,
    supervised,
    sealed,
    members,
    span,
    id,
});
shift_struct!(FormAnnotation { name, args, span });
shift_struct!(LocalityAnnotation { tier, span });
shift_unit_enum!(LocalityTier { L1, L2, L3, Any });
shift_struct!(FfiAnnotation { abi, span });
shift_struct!(FormArg { name, value, span });

impl Shift for LocusAnnotation {
    fn shift(&mut self, d: Move) {
        match self {
            LocusAnnotation::Tier(x) => x.shift(d),
            LocusAnnotation::Projection(x) => x.shift(d),
        }
    }
}

impl Shift for ProjectionClass {
    fn shift(&mut self, d: Move) {
        match self {
            ProjectionClass::Rich | ProjectionClass::Chunked => {}
            ProjectionClass::Recognition(x) => x.shift(d),
        }
    }
}

shift_struct!(RecognitionParams { cap, sub_mode });
shift_unit_enum!(RecognitionSubMode { FixedCell, Spillover, SummaryOnly, SharedSlab });

impl Shift for ScheduleClass {
    fn shift(&mut self, d: Move) {
        match self {
            ScheduleClass::Cooperative => {}
            ScheduleClass::Pinned(x) => x.shift(d),
        }
    }
}

impl Shift for CoreSpec {
    fn shift(&mut self, d: Move) {
        match self {
            CoreSpec::Single(x) => x.shift(d),
            CoreSpec::Range { lo, hi, inclusive } => sh!(d; lo, hi, inclusive),
            CoreSpec::Set(x) => x.shift(d),
        }
    }
}

impl Shift for PinAffinity {
    fn shift(&mut self, d: Move) {
        match self {
            PinAffinity::Any => {}
            PinAffinity::Cores(x) => x.shift(d),
            PinAffinity::Node(x) => x.shift(d),
            PinAffinity::L3(x) => x.shift(d),
        }
    }
}

shift_struct!(TopologyBlock { reserved, nodes, span });
shift_struct!(TopologyNode { id, id_span, domains, span });
shift_struct!(L3Domain { name, cores, span });

impl Shift for LocusMember {
    fn shift(&mut self, d: Move) {
        match self {
            LocusMember::Params(x) => x.shift(d),
            LocusMember::Contract(x) => x.shift(d),
            LocusMember::Bus(x) => x.shift(d),
            LocusMember::Lifecycle(x) => x.shift(d),
            LocusMember::Mode(x) => x.shift(d),
            LocusMember::Failure(x) => x.shift(d),
            LocusMember::Closure(x) => x.shift(d),
            LocusMember::Fn(x) => x.shift(d),
            LocusMember::Const(x) => x.shift(d),
            LocusMember::Type(x) => x.shift(d),
            LocusMember::Capacity(x) => x.shift(d),
            LocusMember::Bindings(x) => x.shift(d),
            LocusMember::Placement(x) => x.shift(d),
            LocusMember::Topology(x) => x.shift(d),
            LocusMember::Claims(x) => x.shift(d),
            LocusMember::BirthCheck(x) => x.shift(d),
        }
    }
}

shift_struct!(BirthCheckDecl { cond, closure_name, payload, span });

// Deployment seams: placement, bindings, capacity.

shift_struct!(PlacementBlock { entries, span });
shift_struct!(PlacementEntry { field, spec, constraints, span, id });

impl Shift for PlacementSpec {
    fn shift(&mut self, d: Move) {
        match self {
            PlacementSpec::Cooperative { pool, affinity } => sh!(d; pool, affinity),
            PlacementSpec::Pinned { affinity, replicas } => sh!(d; affinity, replicas),
        }
    }
}

shift_unit_enum!(PlacementConstraint { AsyncIo });
shift_struct!(SpannedPlacementConstraint { kind, span });
shift_struct!(BindingsBlock { entries, api, span });
shift_struct!(ApiBinding {
    transport,
    roles,
    bound,
    on_full,
    watch_bound,
    on_watch_full,
    on_unauthorized,
    serve,
    http,
    span,
});
shift_struct!(ApiHttp { host, port, principals, span });
shift_unit_enum!(ApiUnauthorizedPolicy { Refuse, Drop });
shift_struct!(ApiRoles { expr, span });

impl Shift for ApiTransport {
    fn shift(&mut self, d: Move) {
        match self {
            ApiTransport::Unix { path, span } => sh!(d; path, span),
        }
    }
}

shift_unit_enum!(ApiFullPolicy { Refuse });
shift_struct!(BindingEntry { topic, transport, constraints, codec, span, id });
shift_struct!(CodecSpec { locus, inits, span });
shift_unit_enum!(BindingConstraint { IntraProcess, IntraMachine, CrossMachine, ZeroCopy });
shift_struct!(SpannedBindingConstraint { kind, span });

impl Shift for TransportSpec {
    fn shift(&mut self, d: Move) {
        match self {
            TransportSpec::Unix { path, role, span } => sh!(d; path, role, span),
            TransportSpec::Adapter { locus, inits, span } => sh!(d; locus, inits, span),
            TransportSpec::ShmRing { name, slot_count, overflow, layout, buffer_size, span } => {
                sh!(d; name, slot_count, overflow, layout, buffer_size, span)
            }
        }
    }
}

shift_unit_enum!(ShmRingOverflow { Block, Drop, Fail });
shift_unit_enum!(TransportRole { Connect, Listen });
shift_struct!(CapacityBlock { slots, span });
shift_struct!(CapacitySlot { name, kind, elem_ty, as_parent_for, indexed_by, span });
shift_unit_enum!(CapacitySlotKind { Pool, Heap });

// Params, contracts, bus.

shift_struct!(ParamsBlock { params, span });
shift_struct!(ParamDecl { name, ty, init, span, id });

impl Shift for ParamInit {
    fn shift(&mut self, d: Move) {
        match self {
            ParamInit::Value(x) => x.shift(d),
            ParamInit::Inferred => {}
        }
    }
}

shift_struct!(ContractBlock { kind, span });

impl Shift for ContractKind {
    fn shift(&mut self, d: Move) {
        match self {
            ContractKind::Inferred => {}
            ContractKind::Members(x) => x.shift(d),
        }
    }
}

shift_struct!(ContractMember { direction, name, ty, gated, span });
shift_unit_enum!(ContractDirection { Expose, Consume });

impl Shift for ContractName {
    fn shift(&mut self, d: Move) {
        match self {
            ContractName::Named(x) => x.shift(d),
            ContractName::Inferred => {}
        }
    }
}

shift_struct!(BusBlock { members, span });

impl Shift for BusSubject {
    fn shift(&mut self, d: Move) {
        match self {
            BusSubject::Literal { subject, span } => sh!(d; subject, span),
            BusSubject::Topic(x) => x.shift(d),
            BusSubject::QualifiedTopic(x) => x.shift(d),
        }
    }
}

impl Shift for BusMember {
    fn shift(&mut self, d: Move) {
        match self {
            BusMember::Subscribe { subject, handler, ty, key_filter, bound, span, id } => {
                sh!(d; subject, handler, ty, key_filter, bound, span, id)
            }
            BusMember::Publish { subject, ty, alias, gated, span, id } => {
                sh!(d; subject, ty, alias, gated, span, id)
            }
        }
    }
}

shift_struct!(SubBound { cap, policy, span });
shift_unit_enum!(ShedPolicy { DropNew, DropOld });

impl Shift for KeyFilter {
    fn shift(&mut self, d: Move) {
        match self {
            KeyFilter::Specific { expr, span } => sh!(d; expr, span),
            KeyFilter::Unmatched { span } => sh!(d; span),
            KeyFilter::Replica { span } => sh!(d; span),
        }
    }
}

// Lifecycle, modes, failure, closures.

shift_struct!(LifecycleDecl { kind, params, ret, unbounded, body, span, id, synthesized });
shift_unit_enum!(LifecycleKind { Birth, Accept, Release, Run, Drain, Dissolve });
shift_struct!(ModeDecl { kind, params, ret, body, span, id });
shift_unit_enum!(ModeKind { Bulk, Harmonic, Resolution });
shift_struct!(FailureDecl { params, body, span, id });
shift_struct!(ClosureDecl { name, assertion, clauses, span, id });
shift_struct!(ClosureAssertion { left, right, tolerance, span });

impl Shift for ClosureClause {
    fn shift(&mut self, d: Move) {
        match self {
            ClosureClause::Epoch(x) => x.shift(d),
            ClosureClause::PersistsThrough(x) => x.shift(d),
            ClosureClause::ResetsOn(x) => x.shift(d),
            ClosureClause::ResetsPerEpoch(x) => x.shift(d),
            ClosureClause::Captures(x) => x.shift(d),
        }
    }
}

impl Shift for EpochSpec {
    fn shift(&mut self, d: Move) {
        match self {
            EpochSpec::Duration(x) => x.shift(d),
            EpochSpec::Tick
            | EpochSpec::Birth
            | EpochSpec::Dissolve
            | EpochSpec::Explicit
            | EpochSpec::Inline => {}
        }
    }
}

// Perspectives, types, consts.

shift_struct!(PerspectiveDecl { name, generics, members, span, id });

impl Shift for PerspectiveMember {
    fn shift(&mut self, d: Move) {
        match self {
            PerspectiveMember::Params(x) => x.shift(d),
            PerspectiveMember::StableWhen(x) => x.shift(d),
            PerspectiveMember::SerializeAs(x) => x.shift(d),
            PerspectiveMember::Fn(x) => x.shift(d),
            PerspectiveMember::Bus(x) => x.shift(d),
        }
    }
}

shift_struct!(TypeDecl { name, display, generics, body, span, id, synthetic });

impl Shift for TypeDeclBody {
    fn shift(&mut self, d: Move) {
        match self {
            TypeDeclBody::Alias(x) => x.shift(d),
            TypeDeclBody::Struct(x) => x.shift(d),
            TypeDeclBody::Enum(x) => x.shift(d),
        }
    }
}

shift_struct!(StructField { name, ty, default, tag, span });
shift_struct!(EnumVariant { name, fields, span });
shift_struct!(ConstDecl { name, ty, value, span, id });

// Effects and fn contracts.

shift_struct!(PhaseEffects { phases, span });

impl Shift for QuantDim {
    fn shift(&mut self, d: Move) {
        match self {
            QuantDim::StackBytes | QuantDim::BlockPoints | QuantDim::Publish | QuantDim::Fanout => {}
            QuantDim::UserClass(x) => x.shift(d),
        }
    }
}

impl Shift for EffectClass {
    fn shift(&mut self, d: Move) {
        match self {
            EffectClass::User(x) => x.shift(d),
            EffectClass::Syscall
            | EffectClass::Block
            | EffectClass::Time
            | EffectClass::Entropy
            | EffectClass::Env
            | EffectClass::Ffi
            | EffectClass::Publish
            | EffectClass::Spawn
            | EffectClass::Recursion
            | EffectClass::Alloc
            | EffectClass::SecretUse => {}
        }
    }
}

shift_struct!(DependsSet { subjects, span });

impl Shift for EffectAssert {
    fn shift(&mut self, d: Move) {
        match self {
            EffectAssert::Forbid(x) => x.shift(d),
            EffectAssert::PublishSet(x) => x.shift(d),
            EffectAssert::Causes(x) => x.shift(d),
            EffectAssert::Carries(x) => x.shift(d),
            EffectAssert::Only(x) => x.shift(d),
            EffectAssert::NoPanic => {}
        }
    }
}

shift_struct!(FnDecorator { name, span });
shift_struct!(FnDecl {
    name,
    generics,
    params,
    ret,
    fallible,
    ffi,
    export,
    unbounded,
    budget,
    hot,
    effects,
    quantities,
    gated,
    decorators,
    body,
    span,
    id,
});
shift_struct!(ModuleDecl { name, items, span, id });
shift_struct!(GenericParam { name, bound, span });
shift_struct!(Param { name, ty, default, secret, span });

// Type expressions.

impl Shift for TypeExpr {
    fn shift(&mut self, d: Move) {
        match self {
            TypeExpr::Primitive(p, span) => sh!(d; p, span),
            TypeExpr::Named { path, generic_args, span } => sh!(d; path, generic_args, span),
            TypeExpr::Projection { class, inner, span } => sh!(d; class, inner, span),
            TypeExpr::Array { elem, size, span } => sh!(d; elem, size, span),
            TypeExpr::Bounded { elem, cap, span } => sh!(d; elem, cap, span),
            TypeExpr::Tuple(elems, span) => sh!(d; elems, span),
            TypeExpr::Function { params, ret, span } => sh!(d; params, ret, span),
            TypeExpr::Perspective { name, span } => sh!(d; name, span),
        }
    }
}

shift_unit_enum!(PrimType {
    Int,
    Uint,
    Float,
    Decimal,
    String,
    Bool,
    Time,
    Duration,
    Bytes,
    BytesView,
    StringView,
    BytesMut,
});
shift_struct!(QualifiedName { segments, span });

// Statements.

shift_struct!(Block { stmts, tail, span });

impl Shift for Stmt {
    fn shift(&mut self, d: Move) {
        match self {
            Stmt::Let { is_mut, name, ty, value, span, id } => {
                sh!(d; is_mut, name, ty, value, span, id)
            }
            Stmt::LetTuple { is_mut, names, ty, value, span, id } => {
                sh!(d; is_mut, names, ty, value, span, id)
            }
            Stmt::Assign { target, op, value, span, id } => sh!(d; target, op, value, span, id),
            Stmt::If(x) => x.shift(d),
            Stmt::Match(x) => x.shift(d),
            Stmt::For { name, iter, body, span, id } => sh!(d; name, iter, body, span, id),
            Stmt::While { cond, body, span } => sh!(d; cond, body, span),
            Stmt::Return(value, span) => sh!(d; value, span),
            Stmt::Break(span) => sh!(d; span),
            Stmt::Continue(span) => sh!(d; span),
            Stmt::Fail { value, span } => sh!(d; value, span),
            Stmt::Yield(span) => sh!(d; span),
            Stmt::Terminate(span) => sh!(d; span),
            Stmt::Reperspective { field, impl_name, span } => sh!(d; field, impl_name, span),
            Stmt::Block(x) => x.shift(d),
            Stmt::Recovery { op, args, modifier, span } => sh!(d; op, args, modifier, span),
            Stmt::Violate { name, payload, span } => sh!(d; name, payload, span),
            Stmt::Send { subject, value, or_disposition, span, id } => {
                sh!(d; subject, value, or_disposition, span, id)
            }
            Stmt::ShmWrite { topic, max, binding, body, span } => {
                sh!(d; topic, max, binding, body, span)
            }
            Stmt::Expr(x) => x.shift(d),
        }
    }
}

shift_unit_enum!(AssignOp {
    Eq,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    PercentEq,
    AmpEq,
    PipeEq,
    CaretEq,
});
shift_struct!(LValue { head, tail, span });

impl Shift for LValueSeg {
    fn shift(&mut self, d: Move) {
        match self {
            LValueSeg::Field(x) => x.shift(d),
            LValueSeg::Index(x) => x.shift(d),
        }
    }
}

shift_struct!(IfStmt { cond, then_block, else_block, span });

impl Shift for ElseBranch {
    fn shift(&mut self, d: Move) {
        match self {
            ElseBranch::Else(x) => x.shift(d),
            ElseBranch::ElseIf(x) => x.shift(d),
        }
    }
}

shift_struct!(MatchStmt { scrutinee, arms, span });
shift_struct!(MatchArm { pattern, guard, body, span });

impl Shift for MatchArmBody {
    fn shift(&mut self, d: Move) {
        match self {
            MatchArmBody::Expr(x) => x.shift(d),
            MatchArmBody::Block(x) => x.shift(d),
        }
    }
}

impl Shift for Pattern {
    fn shift(&mut self, d: Move) {
        match self {
            Pattern::Literal(lit, span) => sh!(d; lit, span),
            Pattern::Wildcard(span) => sh!(d; span),
            Pattern::Binding(x) => x.shift(d),
            Pattern::Constructor { path, args, span } => sh!(d; path, args, span),
            Pattern::Tuple(elems, span) => sh!(d; elems, span),
        }
    }
}

shift_unit_enum!(RecoveryOp { Restart, RestartInPlace, Quarantine, Reorganize, Bubble });

impl Shift for RecoveryModifier {
    fn shift(&mut self, d: Move) {
        match self {
            RecoveryModifier::For(x) => x.shift(d),
            RecoveryModifier::Until(x) => x.shift(d),
        }
    }
}

impl Shift for NodeId {
    fn shift(&mut self, _: Move) {
        let NodeId(_) = self;
    }
}

// Expressions.

impl Shift for Expr {
    fn shift(&mut self, d: Move) {
        match self {
            Expr::Literal(lit, span) => sh!(d; lit, span),
            Expr::Ident(x) => x.shift(d),
            Expr::Path(x) => x.shift(d),
            Expr::KwSelf(span) => sh!(d; span),
            Expr::Binary { op, left, right, span } => sh!(d; op, left, right, span),
            Expr::Unary { op, operand, span } => sh!(d; op, operand, span),
            Expr::Call { callee, args, span, id } => sh!(d; callee, args, span, id),
            Expr::Field { receiver, name, span } => sh!(d; receiver, name, span),
            Expr::Index { receiver, index, span } => sh!(d; receiver, index, span),
            Expr::Path2 { receiver, name, span } => sh!(d; receiver, name, span),
            Expr::Tuple(elems, span) => sh!(d; elems, span),
            Expr::Array(elems, span) => sh!(d; elems, span),
            Expr::Struct { path, inits, span, id } => sh!(d; path, inits, span, id),
            Expr::Block(x) => x.shift(d),
            Expr::If(x) => x.shift(d),
            Expr::Match(x) => x.shift(d),
            Expr::Sum(inner, span) => sh!(d; inner, span),
            Expr::Prod(inner, span) => sh!(d; inner, span),
            Expr::Approx { left, right, tolerance, span } => {
                sh!(d; left, right, tolerance, span)
            }
            Expr::Range { lo, hi, inclusive, span } => sh!(d; lo, hi, inclusive, span),
            Expr::ArrayRepeat { val, count, span } => sh!(d; val, count, span),
            Expr::Or { inner, disposition, span } => sh!(d; inner, disposition, span),
        }
    }
}

impl Shift for OrDisposition {
    fn shift(&mut self, d: Move) {
        match self {
            OrDisposition::Raise(span) => sh!(d; span),
            OrDisposition::Substitute(x) => x.shift(d),
            OrDisposition::Discard(span) => sh!(d; span),
            OrDisposition::Fail(value, span) => sh!(d; value, span),
            OrDisposition::Wait(span) => sh!(d; span),
        }
    }
}

shift_struct!(StructInit { name, value, span });
shift_unit_enum!(BinOp {
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
});
shift_unit_enum!(UnaryOp { Neg, Not, BitNot });

impl Shift for Literal {
    fn shift(&mut self, d: Move) {
        match self {
            Literal::Int(x) => x.shift(d),
            Literal::Float(x) => x.shift(d),
            Literal::Decimal(x) => x.shift(d),
            Literal::String(x) => x.shift(d),
            Literal::Bool(x) => x.shift(d),
            Literal::Nil => {}
            Literal::Duration(x) => x.shift(d),
            Literal::Time(x) => x.shift(d),
            Literal::Bytes(x) => x.shift(d),
        }
    }
}

shift_struct!(Ident { name, span, id });

#[cfg(test)]
mod tests {
    use super::shift_program;
    use crate::ast::EffectClasses;
    use crate::parse_source_at_in;
    use std::path::{Path, PathBuf};

    /// Bases the oracle moves every program to: one byte, a page and a
    /// byte, and a seven-digit offset no test program comes near.
    const BASES: [u32; 3] = [1, 4097, 1_000_003];

    fn collect_hl(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        paths.sort();
        for p in paths {
            if p.is_dir() {
                // `target/` is build output, `.git/` the object store,
                // `.claude/` agent worktrees (a second copy of the tree).
                let skip = p
                    .file_name()
                    .is_some_and(|n| n == "target" || n == ".git" || n == ".claude");
                if !skip {
                    collect_hl(&p, out);
                }
            } else if p.extension().is_some_and(|e| e == "hl") {
                out.push(p);
            }
        }
    }

    /// The first byte at which `a` and `b` differ, with a little of
    /// each side around it.
    fn first_difference(a: &str, b: &str) -> String {
        let (a, b) = (a.as_bytes(), b.as_bytes());
        let at = a.iter().zip(b).position(|(x, y)| x != y).unwrap_or(a.len().min(b.len()));
        let window = |s: &[u8]| {
            let lo = at.saturating_sub(120);
            let hi = (at + 120).min(s.len());
            String::from_utf8_lossy(&s[lo..hi]).into_owned()
        };
        format!(
            "first difference at Debug byte {at} (lengths {} and {})\n  parsed at base: …{}…\n  shifted:        …{}…",
            a.len(),
            b.len(),
            window(a),
            window(b),
        )
    }

    /// `None` when every base agrees, else what differed.
    fn disagreement(src: &str) -> Option<String> {
        for base in BASES {
            // Prose in both templates: the same rendering of two parses
            // compared here, not a value read back as text.
            let at_base =
                format!("parse at base {:?}", parse_source_at_in(src, base, &mut EffectClasses::default()));
            let shifted = format!(
                "parse at base {:?}",
                parse_source_at_in(src, 0, &mut EffectClasses::default())
                    .map(|mut p| {
                        shift_program(&mut p, base);
                        p
                    })
                    .map_err(|ds| ds.into_iter().map(|d| d.shifted(base)).collect::<Vec<_>>())
            );
            if at_base != shifted {
                return Some(format!("base {base}: {}", first_difference(&at_base, &shifted)));
            }
        }
        None
    }

    /// The oracle: over every `.hl` file in the repository, and every
    /// Hale program the test suites embed in Rust literals, a parse at
    /// a base is the parse at 0 shifted by it — the AST when the file
    /// parses, the diagnostics (`Diag::shifted`) when it does not.
    ///
    /// The embedded stdlib is covered file by file (`crates/hale-stdlib/hl`
    /// is in the walk); its concatenated `AP_SOURCE` is not reachable
    /// from here, since hale-syntax does not depend on hale-stdlib.
    #[test]
    fn shift_matches_parsing_at_the_base_over_the_whole_corpus() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut files = Vec::new();
        collect_hl(&root, &mut files);
        let mut programs: Vec<(String, String)> = files
            .iter()
            .filter_map(|p| {
                let src = std::fs::read_to_string(p).ok()?;
                Some((p.strip_prefix(&root).unwrap_or(p).display().to_string(), src))
            })
            .collect();
        let on_disk = programs.len();
        programs.extend(hale_corpus::embedded().into_iter().map(|p| (p.origin, p.source)));

        let mut parsed = 0usize;
        let mut mismatches = Vec::new();
        for (origin, src) in &programs {
            if crate::parse_source(src).is_ok() {
                parsed += 1;
            }
            if let Some(why) = disagreement(src) {
                mismatches.push((origin, why));
            }
        }
        assert!(on_disk > 500, "the walk found only {on_disk} .hl files under {}", root.display());
        if let Some((origin, why)) = mismatches.first() {
            panic!(
                "{} of {} programs disagree; first is {origin}\n{why}",
                mismatches.len(),
                programs.len(),
            );
        }
        eprintln!(
            "shift oracle: {} programs ({on_disk} .hl files, {} embedded), {parsed} parse, bases {BASES:?}",
            programs.len(),
            programs.len() - on_disk,
        );
    }
}

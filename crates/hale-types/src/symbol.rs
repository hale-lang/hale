//! Symbol tables and the per-scope shapes the resolver builds.
//!
//! A `Bundle` is a set of parsed Programs, keyed by import path,
//! that the type checker treats as one logical compilation unit.
//! For milestone 2 we collect all top-level decls across the
//! bundle into a single `BundleScope` and resolve names against
//! it. Module-decl scoping is deferred (we flatten module decls
//! at the top level for now).

use std::collections::BTreeMap;

use hale_syntax::ast::{ModeKind, ProjectionClass, ScheduleClass};
use hale_syntax::Span;

use crate::ty::Ty;

/// A logical compilation unit: a set of named programs (one per
/// source file) referenced by import path. The type checker
/// builds a single shared scope from the bundle's top-level
/// decls; per-program scopes layer on top.
pub struct Bundle<'a> {
    pub programs: BTreeMap<String, &'a hale_syntax::ast::Program>,
}

/// Top-level symbol — a binding visible at module / bundle scope.
#[derive(Debug, Clone)]
pub enum TopSymbol {
    Locus(LocusInfo),
    Type(TypeInfo),
    Perspective(PerspectiveInfo),
    Const(ConstInfo),
    Fn(FnSig),
    Interface(InterfaceInfo),
    Topic(TopicInfo),
    RingLayout(RingLayoutInfo),
}

impl TopSymbol {
    pub fn span(&self) -> Span {
        match self {
            TopSymbol::Locus(l) => l.span,
            TopSymbol::Type(t) => t.span,
            TopSymbol::Perspective(p) => p.span,
            TopSymbol::Const(c) => c.span,
            TopSymbol::Fn(f) => f.span,
            TopSymbol::Interface(i) => i.span,
            TopSymbol::Topic(t) => t.span,
            TopSymbol::RingLayout(r) => r.span,
        }
    }
}

/// shm-ring-interop Proposal B: a resolved `ring_layout` declaration.
/// Carries the full AST decl so codegen (PR3) can build the runtime
/// ring descriptor from its fields; PR1/PR2 only need it to exist as a
/// resolvable symbol a `shm_ring(..., layout: Name)` binding references.
#[derive(Debug, Clone)]
pub struct RingLayoutInfo {
    pub name: String,
    pub decl: hale_syntax::ast::RingLayoutDecl,
    pub span: Span,
}

/// Resolved `topic Foo { payload: T; }` declaration. The payload
/// type is the single source of truth for every subscribe /
/// publish / send site referencing this topic by name.
///
/// `parent` (Phase 2) is the optional declared parent topic — it
/// roots a hierarchy used to derive a wire subject for transports
/// that benefit from path-shaped routing (NATS, MQTT). The
/// `wire_subject` field is the materialized dot-path: own
/// `subject` if no parent, else `parent.wire_subject + "." +
/// subject`. Defaults to the topic's lowercased name when no
/// `subject:` is declared.
#[derive(Debug, Clone)]
pub struct TopicInfo {
    pub name: String,
    pub payload: Ty,
    pub parent: Option<String>,
    pub subject: String,
    pub wire_subject: String,
    /// Phase 3 routing keys (2026-05-25): the payload field
    /// that holds the routing key, if declared. None for
    /// unkeyed topics.
    pub keyed_by: Option<String>,
    /// Phase 3 routing keys (2026-05-25): policy for keyed
    /// publishes that don't match any subscriber's filter.
    /// None means the topic is unkeyed (no policy
    /// meaningful) OR keyed with the default `swallow`
    /// policy. Codegen treats None as Swallow at lowering
    /// time.
    pub on_unmatched: Option<hale_syntax::ast::UnmatchedPolicy>,
    pub span: Span,
}

/// Resolved interface — a named set of method signatures. Order
/// is significant (vtable layout follows declaration order).
#[derive(Debug, Clone)]
pub struct InterfaceInfo {
    pub name: String,
    pub methods: Vec<InterfaceMethodInfo>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct InterfaceMethodInfo {
    pub name: String,
    pub params: Vec<(String, Ty)>,
    pub ret: Ty,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct LocusInfo {
    pub name: String,
    /// Phase 2a: perspective contracts this locus `serves` — the
    /// `locus L : serves P, Q` clause names. Empty for non-impl
    /// loci. Consulted by `reperspective` (Phase 2b) to verify the
    /// new impl serves the target perspective.
    pub serves: Vec<String>,
    pub params: Vec<ParamInfo>,
    pub bus_publishes: Vec<BusPublishInfo>,
    pub bus_subscribes: Vec<BusSubscribeInfo>,
    pub accept_param: Option<(String, Ty)>,
    pub mode_returns: BTreeMap<ModeKind, Ty>,
    pub annotations: Annotations,
    /// Fields the locus exposes upward to its coordinator
    /// (the F.8 typed surface).
    pub contract_expose: Vec<ContractEntry>,
    /// Fields the locus consumes downward from its
    /// coordinatees. Each entry must match an `expose` on
    /// the accept-param child type.
    pub contract_consume: Vec<ContractEntry>,
    /// Methods callable as `handle.name(...)`: free `fn`
    /// members + the three mode declarations (bulk /
    /// harmonic / resolution).
    pub methods: Vec<MethodInfo>,
    /// F.22 capacity-tuple slot names declared on this locus
    /// (`pool X of T;` / `heap Y of T;`). Just the names —
    /// typecheck only needs to recognize `self.<X>` as
    /// referring to a slot rather than a field, so the
    /// `self.X.method()` shape doesn't error with
    /// "no field `X`". Slot kinds + element types live on
    /// the codegen-side LocusInfo where dispatch happens.
    pub capacity_slot_names: Vec<String>,
    /// v1.x-VIOLATE (F.27): closure declarations on this locus.
    /// Used by typecheck to resolve `violate NAME;` against the
    /// enclosing locus and enforce the `epoch inline` gate.
    pub closures: Vec<ClosureSymInfo>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ClosureSymInfo {
    pub name: String,
    /// True iff the closure has an `epoch inline` clause. Inline
    /// closures fire only via `violate`; auto-epoch closures fire
    /// at epoch boundaries and do not accept `violate`.
    pub is_inline: bool,
    /// Field names from the `captures:` clause (if any). Each
    /// must reference an existing locus param/state field.
    pub captures: Vec<String>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct MethodInfo {
    pub name: String,
    pub params: Vec<Ty>,
    pub ret: Ty,
    /// v1.x-FORM-1: payload type when the method was declared
    /// (or synthesized) `fallible(E)`. Mirrors `FnSig.fallible`
    /// for top-level fns; lets method-call sites produce
    /// `Ty::Fallible` so the caller is forced to address the
    /// error with `or` / `match`.
    pub fallible: Option<Ty>,
}

#[derive(Debug, Clone)]
pub struct ContractEntry {
    pub name: String,
    pub ty: Ty,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ParamInfo {
    pub name: String,
    pub ty: Ty,
    pub has_default: bool,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct BusPublishInfo {
    pub subject: String,
    pub payload: Ty,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct BusSubscribeInfo {
    pub subject: String,
    pub handler: String,
    pub payload: Ty,
    pub span: Span,
}

#[derive(Debug, Clone, Default)]
pub struct Annotations {
    pub tier: Option<i64>,
    pub projection: Option<ProjectionClass>,
    pub schedule: Option<ScheduleClass>,
}

#[derive(Debug, Clone)]
pub struct TypeInfo {
    pub name: String,
    pub kind: TypeKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum TypeKind {
    Struct(Vec<FieldInfo>),
    Enum(Vec<VariantInfo>),
    Alias(Ty),
}

#[derive(Debug, Clone)]
pub struct FieldInfo {
    pub name: String,
    pub ty: Ty,
    pub has_default: bool,
    /// The field's raw backtick metadata tag, if any (e.g.
    /// `repr:"u32_le"`). Carried so the checker can validate
    /// repr-tagged-field accessors (`T::field` / `T::set_field`).
    pub tag: Option<String>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct VariantInfo {
    pub name: String,
    pub fields: Vec<Ty>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct PerspectiveInfo {
    pub name: String,
    pub params: Vec<ParamInfo>,
    pub serialize_as: Option<Ty>,
    pub methods: Vec<MethodInfo>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ConstInfo {
    pub name: String,
    pub ty: Ty,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct FnSig {
    pub name: String,
    pub params: Vec<(String, Ty)>,
    pub ret: Ty,
    /// v1.x-FORM-1: payload type when the fn was declared
    /// `-> T fallible(E)`. Calls to fallible fns produce a
    /// [`Ty::Fallible { success: ret, payload: this }`] result
    /// that the caller must address via `or` or `match`.
    pub fallible: Option<Ty>,
    pub span: Span,
}

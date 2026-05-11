//! Symbol tables and the per-scope shapes the resolver builds.
//!
//! A `Bundle` is a set of parsed Programs, keyed by import path,
//! that the type checker treats as one logical compilation unit.
//! For milestone 2 we collect all top-level decls across the
//! bundle into a single `BundleScope` and resolve names against
//! it. Module-decl scoping is deferred (we flatten module decls
//! at the top level for now).

use std::collections::BTreeMap;

use aperio_syntax::ast::{ModeKind, ProjectionClass, ScheduleClass};
use aperio_syntax::Span;

use crate::ty::Ty;

/// A logical compilation unit: a set of named programs (one per
/// source file) referenced by import path. The type checker
/// builds a single shared scope from the bundle's top-level
/// decls; per-program scopes layer on top.
pub struct Bundle<'a> {
    pub programs: BTreeMap<String, &'a aperio_syntax::ast::Program>,
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
        }
    }
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
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct MethodInfo {
    pub name: String,
    pub params: Vec<Ty>,
    pub ret: Ty,
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
    pub span: Span,
}

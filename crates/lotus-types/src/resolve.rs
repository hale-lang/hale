//! Symbol resolution: build the bundle-wide top-level scope.
//!
//! Phase 1: walk every top-level decl in every program and
//! produce a `TopScope` keyed by name. Each entry carries the
//! pre-resolved [`crate::symbol::TopSymbol`] form (params with
//! types resolved, bus subjects/payloads resolved, etc.).
//!
//! Resolution of a `TypeExpr → Ty` happens here too — but only
//! against primitive types and the bundle's own top-level
//! names. Type names not visible at this stage become
//! `Ty::Unknown`; full call-site / external-path resolution is
//! milestone 3.

use std::collections::BTreeMap;

use lotus_syntax::ast::*;
use lotus_syntax::{Diag, Span};

use crate::symbol::*;
use crate::ty::Ty;

#[derive(Debug, Default)]
pub struct TopScope {
    pub symbols: BTreeMap<String, TopSymbol>,
}

impl TopScope {
    pub fn lookup(&self, name: &str) -> Option<&TopSymbol> {
        self.symbols.get(name)
    }
}

pub fn build_top_scope(bundle: &Bundle<'_>) -> (TopScope, Vec<Diag>) {
    let mut scope = TopScope::default();
    let mut diags = Vec::new();

    // First pass: register every top-level *type-like* name
    // (locus, type, perspective) so type expressions in a
    // second pass can resolve cross-file references.
    let mut known_names: BTreeMap<String, Span> = BTreeMap::new();
    for program in bundle.programs.values() {
        collect_type_names(&program.items, &mut known_names, &mut diags);
    }

    // Second pass: resolve and emit full TopSymbol entries.
    for program in bundle.programs.values() {
        register_top_decls(&program.items, &known_names, &mut scope, &mut diags);
    }

    (scope, diags)
}

fn collect_type_names(
    items: &[TopDecl],
    known: &mut BTreeMap<String, Span>,
    diags: &mut Vec<Diag>,
) {
    for item in items {
        match item {
            TopDecl::Locus(l) => insert_name(known, &l.name, diags),
            TopDecl::Type(t) => insert_name(known, &t.name, diags),
            TopDecl::Perspective(p) => insert_name(known, &p.name, diags),
            TopDecl::Module(m) => collect_type_names(&m.items, known, diags),
            _ => {}
        }
    }
}

fn insert_name(
    known: &mut BTreeMap<String, Span>,
    ident: &Ident,
    diags: &mut Vec<Diag>,
) {
    if let Some(prev) = known.get(&ident.name) {
        diags.push(Diag::ty(
            ident.span,
            format!(
                "duplicate top-level name `{}` (previous declaration at {:?})",
                ident.name, prev
            ),
        ));
        return;
    }
    known.insert(ident.name.clone(), ident.span);
}

fn register_top_decls(
    items: &[TopDecl],
    known: &BTreeMap<String, Span>,
    scope: &mut TopScope,
    diags: &mut Vec<Diag>,
) {
    for item in items {
        match item {
            TopDecl::Locus(l) => register_locus(l, known, scope, diags),
            TopDecl::Type(t) => register_type(t, known, scope, diags),
            TopDecl::Perspective(p) => register_perspective(p, known, scope, diags),
            TopDecl::Const(c) => register_const(c, known, scope, diags),
            TopDecl::Fn(f) => register_fn(f, known, scope, diags),
            TopDecl::Module(m) => {
                register_top_decls(&m.items, known, scope, diags);
            }
        }
    }
}

fn register_locus(
    decl: &LocusDecl,
    known: &BTreeMap<String, Span>,
    scope: &mut TopScope,
    diags: &mut Vec<Diag>,
) {
    let mut params: Vec<ParamInfo> = Vec::new();
    let mut bus_publishes: Vec<BusPublishInfo> = Vec::new();
    let mut bus_subscribes: Vec<BusSubscribeInfo> = Vec::new();
    let mut accept_param: Option<(String, Ty)> = None;
    let mut mode_returns: BTreeMap<ModeKind, Ty> = BTreeMap::new();
    let mut annotations = Annotations::default();
    let mut contract_expose: Vec<ContractEntry> = Vec::new();
    let mut contract_consume: Vec<ContractEntry> = Vec::new();
    let mut methods: Vec<MethodInfo> = Vec::new();

    for ann in &decl.annotations {
        match ann {
            LocusAnnotation::Tier(n) => annotations.tier = Some(*n),
            LocusAnnotation::Projection(p) => annotations.projection = Some(*p),
        }
    }

    for member in &decl.members {
        match member {
            LocusMember::Params(pb) => {
                for p in &pb.params {
                    let ty = match &p.ty {
                        Some(te) => resolve_type_expr(te, known),
                        None => match &p.init {
                            ParamInit::Value(e) => infer_literal_ty(e),
                            ParamInit::Inferred => Ty::Unknown,
                        },
                    };
                    let has_default = matches!(p.init, ParamInit::Value(_));
                    params.push(ParamInfo {
                        name: p.name.name.clone(),
                        ty,
                        has_default,
                        span: p.span,
                    });
                }
            }
            LocusMember::Bus(bb) => {
                for bm in &bb.members {
                    match bm {
                        BusMember::Subscribe { subject, handler, ty, span } => {
                            let payload = match ty {
                                Some(te) => resolve_type_expr(te, known),
                                None => Ty::Unknown,
                            };
                            bus_subscribes.push(BusSubscribeInfo {
                                subject: subject.clone(),
                                handler: handler.name.clone(),
                                payload,
                                span: *span,
                            });
                        }
                        BusMember::Publish { subject, ty, span, .. } => {
                            let payload = resolve_type_expr(ty, known);
                            bus_publishes.push(BusPublishInfo {
                                subject: subject.clone(),
                                payload,
                                span: *span,
                            });
                        }
                    }
                }
            }
            LocusMember::Lifecycle(lc) if matches!(lc.kind, LifecycleKind::Accept) => {
                if let Some(p) = lc.params.first() {
                    let ty = resolve_type_expr(&p.ty, known);
                    accept_param = Some((p.name.name.clone(), ty));
                }
            }
            LocusMember::Mode(md) => {
                let ret = match &md.ret {
                    Some(te) => resolve_type_expr(te, known),
                    None => Ty::Unit,
                };
                mode_returns.insert(md.kind, ret.clone());
                let mname = match md.kind {
                    ModeKind::Bulk => "bulk",
                    ModeKind::Harmonic => "harmonic",
                    ModeKind::Resolution => "resolution",
                };
                methods.push(MethodInfo {
                    name: mname.to_string(),
                    params: md
                        .params
                        .iter()
                        .map(|p| resolve_type_expr(&p.ty, known))
                        .collect(),
                    ret,
                });
            }
            LocusMember::Fn(f) => {
                let ret = match &f.ret {
                    Some(te) => resolve_type_expr(te, known),
                    None => Ty::Unit,
                };
                methods.push(MethodInfo {
                    name: f.name.name.clone(),
                    params: f
                        .params
                        .iter()
                        .map(|p| resolve_type_expr(&p.ty, known))
                        .collect(),
                    ret,
                });
            }
            LocusMember::Contract(cb) => {
                if let ContractKind::Members(members) = &cb.kind {
                    for m in members {
                        let ContractName::Named(name) = &m.name else {
                            continue;
                        };
                        let Some(te) = &m.ty else {
                            continue;
                        };
                        let entry = ContractEntry {
                            name: name.name.clone(),
                            ty: resolve_type_expr(te, known),
                            span: m.span,
                        };
                        match m.direction {
                            ContractDirection::Expose => contract_expose.push(entry),
                            ContractDirection::Consume => contract_consume.push(entry),
                        }
                    }
                }
            }
            _ => {} // closure / failure / fn / const / type members
                    // are not yet hoisted into the locus's external surface in
                    // milestone 2.
        }
    }

    let info = LocusInfo {
        name: decl.name.name.clone(),
        params,
        bus_publishes,
        bus_subscribes,
        accept_param,
        mode_returns,
        annotations,
        contract_expose,
        contract_consume,
        methods,
        span: decl.span,
    };

    register_symbol(scope, &decl.name.name, TopSymbol::Locus(info), decl.span, diags);
}

fn register_type(
    decl: &TypeDecl,
    known: &BTreeMap<String, Span>,
    scope: &mut TopScope,
    diags: &mut Vec<Diag>,
) {
    let kind = match &decl.body {
        TypeDeclBody::Alias(te) => TypeKind::Alias(resolve_type_expr(te, known)),
        TypeDeclBody::Struct(fields) => {
            let infos: Vec<FieldInfo> = fields
                .iter()
                .map(|f| FieldInfo {
                    name: f.name.name.clone(),
                    ty: resolve_type_expr(&f.ty, known),
                    has_default: f.default.is_some(),
                    span: f.span,
                })
                .collect();
            TypeKind::Struct(infos)
        }
        TypeDeclBody::Enum(variants) => {
            let infos: Vec<VariantInfo> = variants
                .iter()
                .map(|v| VariantInfo {
                    name: v.name.name.clone(),
                    fields: v.fields.iter().map(|t| resolve_type_expr(t, known)).collect(),
                    span: v.span,
                })
                .collect();
            TypeKind::Enum(infos)
        }
    };
    let info = TypeInfo {
        name: decl.name.name.clone(),
        kind,
        span: decl.span,
    };
    register_symbol(scope, &decl.name.name, TopSymbol::Type(info), decl.span, diags);
}

fn register_perspective(
    decl: &PerspectiveDecl,
    known: &BTreeMap<String, Span>,
    scope: &mut TopScope,
    diags: &mut Vec<Diag>,
) {
    let mut params = Vec::new();
    let mut serialize_as = None;
    let mut methods: Vec<MethodInfo> = Vec::new();
    for member in &decl.members {
        match member {
            PerspectiveMember::Params(pb) => {
                for p in &pb.params {
                    let ty = match &p.ty {
                        Some(te) => resolve_type_expr(te, known),
                        None => match &p.init {
                            ParamInit::Value(e) => infer_literal_ty(e),
                            ParamInit::Inferred => Ty::Unknown,
                        },
                    };
                    params.push(ParamInfo {
                        name: p.name.name.clone(),
                        ty,
                        has_default: matches!(p.init, ParamInit::Value(_)),
                        span: p.span,
                    });
                }
            }
            PerspectiveMember::SerializeAs(te) => {
                serialize_as = Some(resolve_type_expr(te, known));
            }
            PerspectiveMember::Fn(f) => {
                let ret = match &f.ret {
                    Some(te) => resolve_type_expr(te, known),
                    None => Ty::Unit,
                };
                methods.push(MethodInfo {
                    name: f.name.name.clone(),
                    params: f
                        .params
                        .iter()
                        .map(|p| resolve_type_expr(&p.ty, known))
                        .collect(),
                    ret,
                });
            }
            PerspectiveMember::StableWhen(_) => {
                // stable_when is a built-in method on every
                // perspective: `p.is_stable() -> Bool`.
                methods.push(MethodInfo {
                    name: "is_stable".to_string(),
                    params: Vec::new(),
                    ret: Ty::Prim(PrimType::Bool),
                });
            }
        }
    }
    let info = PerspectiveInfo {
        name: decl.name.name.clone(),
        params,
        serialize_as,
        methods,
        span: decl.span,
    };
    register_symbol(
        scope,
        &decl.name.name,
        TopSymbol::Perspective(info),
        decl.span,
        diags,
    );
}

fn register_const(
    decl: &ConstDecl,
    known: &BTreeMap<String, Span>,
    scope: &mut TopScope,
    diags: &mut Vec<Diag>,
) {
    let ty = resolve_type_expr(&decl.ty, known);
    let info = ConstInfo {
        name: decl.name.name.clone(),
        ty,
        span: decl.span,
    };
    register_symbol(scope, &decl.name.name, TopSymbol::Const(info), decl.span, diags);
}

fn register_fn(
    decl: &FnDecl,
    known: &BTreeMap<String, Span>,
    scope: &mut TopScope,
    diags: &mut Vec<Diag>,
) {
    let params = decl
        .params
        .iter()
        .map(|p| (p.name.name.clone(), resolve_type_expr(&p.ty, known)))
        .collect();
    let ret = match &decl.ret {
        Some(te) => resolve_type_expr(te, known),
        None => Ty::Unit,
    };
    let sig = FnSig {
        name: decl.name.name.clone(),
        params,
        ret,
        span: decl.span,
    };
    register_symbol(scope, &decl.name.name, TopSymbol::Fn(sig), decl.span, diags);
}

fn register_symbol(
    scope: &mut TopScope,
    name: &str,
    sym: TopSymbol,
    span: Span,
    diags: &mut Vec<Diag>,
) {
    if scope.symbols.contains_key(name) {
        // Duplicate already reported by collect_type_names for
        // type-like names; fns/consts get caught here.
        if let TopSymbol::Fn(_) | TopSymbol::Const(_) = &sym {
            diags.push(Diag::ty(
                span,
                format!("duplicate top-level name `{}`", name),
            ));
        }
        return;
    }
    scope.symbols.insert(name.to_string(), sym);
}

/// Resolve a syntactic [`TypeExpr`] to a [`Ty`], using the
/// bundle-wide set of known type-like names. Names not in
/// `known` and not primitive resolve to [`Ty::Unknown`].
pub fn resolve_type_expr(te: &TypeExpr, known: &BTreeMap<String, Span>) -> Ty {
    match te {
        TypeExpr::Primitive(p, _) => Ty::Prim(*p),
        TypeExpr::Named { path, .. } => {
            if path.segments.len() == 1 {
                let name = &path.segments[0].name;
                if known.contains_key(name) {
                    Ty::Named(name.clone())
                } else {
                    Ty::Unknown
                }
            } else {
                // qualified path -> external (stdlib, module)
                Ty::Unknown
            }
        }
        TypeExpr::Projection { class, inner, .. } => {
            Ty::Projection(*class, Box::new(resolve_type_expr(inner, known)))
        }
        TypeExpr::Array { elem, size, .. } => {
            let n = match size {
                Some(Expr::Literal(Literal::Int(n), _)) if *n >= 0 => Some(*n as u64),
                _ => None,
            };
            Ty::Array(Box::new(resolve_type_expr(elem, known)), n)
        }
        TypeExpr::Tuple(parts, _) => {
            Ty::Tuple(parts.iter().map(|t| resolve_type_expr(t, known)).collect())
        }
        TypeExpr::Function { params, ret, .. } => {
            let p = params.iter().map(|t| resolve_type_expr(t, known)).collect();
            let r = match ret {
                Some(te) => resolve_type_expr(te, known),
                None => Ty::Unit,
            };
            Ty::Function {
                params: p,
                ret: Box::new(r),
            }
        }
    }
}

/// Best-effort literal-typing for params declared with a value
/// but no explicit `: ty`. Just enough to give `B: Int = 100`
/// the right type when `: Int` is omitted; falls through to
/// Unknown otherwise.
fn infer_literal_ty(e: &Expr) -> Ty {
    match e {
        Expr::Literal(Literal::Int(_), _) => Ty::Prim(PrimType::Int),
        Expr::Literal(Literal::Float(_), _) => Ty::Prim(PrimType::Float),
        Expr::Literal(Literal::Decimal(_), _) => Ty::Prim(PrimType::Decimal),
        Expr::Literal(Literal::String(_), _) => Ty::Prim(PrimType::String),
        Expr::Literal(Literal::Bool(_), _) => Ty::Prim(PrimType::Bool),
        Expr::Literal(Literal::Duration(_), _) => Ty::Prim(PrimType::Duration),
        Expr::Literal(Literal::Time(_), _) => Ty::Prim(PrimType::Time),
        Expr::Literal(Literal::Bytes(_), _) => Ty::Prim(PrimType::Bytes),
        _ => Ty::Unknown,
    }
}

//! Concrete generic uses shared by lifecycle classification and lowering.
//!
//! This is the former lowering discovery walk (m61–m63, GH #884/#911).
//! The same type-bearing positions and the same type substitution close
//! one request inventory. Emission synthesizes declarations from these
//! requests; it does not discover an additional flow population.
use crate::ty::Ty;
use crate::typed_bodies::{Monomorphs, TemplateKind};
use hale_syntax::ast::*;
use hale_syntax::Span;
use std::collections::{BTreeMap, BTreeSet};

pub type Error = (String, Span);

#[derive(Clone, Debug)]
pub struct Instantiation {
    pub template: String,
    pub name: String,
    pub args: Vec<TypeExpr>,
    pub locus: bool,
}

/// The concrete type and locus specializations the program emits. Typed
/// requests refer only to the user snapshot's identities; the stdlib
/// analysis copy has a separate identity universe.
pub fn derive(
    programs: &[&Program],
    stdlib: Option<&Program>,
    renames: &[(Vec<String>, String)],
    typed: &Monomorphs,
) -> Result<Vec<Instantiation>, Error> {
    let all: Vec<_> = programs.iter().copied().chain(stdlib).collect();
    let mut types = BTreeMap::new();
    let mut loci = BTreeMap::new();
    let mut generic_names: BTreeMap<String, String> = ["Option", "Result"]
        .into_iter()
        .map(|n| (n.to_string(), n.to_string()))
        .collect();
    for program in &all {
        for item in flat_decls(&program.items) {
            match item {
                TopDecl::Type(t) if !t.generics.is_empty() => {
                    types.insert(t.name.name.clone(), t);
                }
                TopDecl::Locus(l) if !l.generics.is_empty() => {
                    loci.insert(l.name.name.clone(), l);
                }
                _ => {}
            }
        }
    }
    for name in types.keys().chain(loci.keys()) {
        generic_names.insert(name.clone(), name.clone());
    }
    for (path, target) in renames {
        if generic_names.contains_key(target) {
            generic_names.insert(path.join("::"), target.clone());
        }
    }
    let mut seen = BTreeSet::new();
    let mut requests = Vec::new();
    let empty = BTreeMap::new();
    for program in &all {
        collect_generic_uses_in_program(program, &generic_names, &mut seen, &mut requests, &empty)?;
    }
    // m63/GH #911: the typing walk supplies concrete uses inside generic
    // functions. Match its template identities in its own universe only.
    for m in typed.rows() {
        let name = programs
            .iter()
            .flat_map(|p| flat_decls(&p.items))
            .find_map(|item| match (m.kind, item) {
                (TemplateKind::Locus, TopDecl::Locus(l))
                    if !l.generics.is_empty() && l.id.0 == m.template.0 =>
                {
                    Some(l.name.name.clone())
                }
                (TemplateKind::Type, TopDecl::Type(t))
                    if !t.generics.is_empty() && t.id.0 == m.template.0 =>
                {
                    Some(t.name.name.clone())
                }
                _ => None,
            });
        let Some(name) = name else { continue };
        let args: Option<Vec<_>> = m.args.iter().map(type_arg).collect();
        if let Some(args) = args {
            if seen.insert(m.name.clone()) {
                requests.push((name, args));
            }
        }
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i < requests.len() {
        if i >= 16_384 {
            return Err(("generic specialization inventory exceeds 16384 instances; no complete flow classification is available".to_string(), Span::new(0, 0)));
        }
        let (template, args) = requests[i].clone();
        i += 1;
        let name = crate::mangle::mangle_generic_name(&template, &args)?;
        if let Some(t) = types.get(&template) {
            let subst = t
                .generics
                .iter()
                .zip(&args)
                .map(|(g, a)| (g.name.name.clone(), a.clone()))
                .collect();
            // Match lowering's supported synthesized type bodies.
            match &t.body {
                TypeDeclBody::Struct(fields) => {
                    for f in fields {
                        collect_type(&f.ty, &generic_names, &mut seen, &mut requests, &subst)?;
                    }
                }
                TypeDeclBody::Enum(variants) => {
                    for v in variants {
                        for f in &v.fields {
                            collect_type(f, &generic_names, &mut seen, &mut requests, &subst)?;
                        }
                    }
                }
                TypeDeclBody::Alias(_) | TypeDeclBody::Scalar(_) => {}
            }
        } else if let Some(l) = loci.get(&template) {
            let subst = l
                .generics
                .iter()
                .zip(&args)
                .map(|(g, a)| (g.name.name.clone(), a.clone()))
                .collect();
            for member in &l.members {
                collect_in_locus_member(member, &generic_names, &mut seen, &mut requests, &subst)?;
            }
        }
        out.push(Instantiation {
            locus: !types.contains_key(&template) && loci.contains_key(&template),
            template,
            name,
            args,
        });
    }
    Ok(out)
}

fn type_arg(ty: &Ty) -> Option<TypeExpr> {
    let span = Span::new(0, 0);
    match ty {
        Ty::Prim(p) => Some(TypeExpr::Primitive(*p, span)),
        Ty::Named(n) => Some(TypeExpr::Named {
            path: QualifiedName {
                segments: vec![Ident::new(n, span)],
                span,
            },
            generic_args: Vec::new(),
            span,
        }),
        _ => None,
    }
}
fn mangle_generic_name(template: &str, args: &[TypeExpr]) -> Result<String, Error> {
    crate::mangle::mangle_generic_name(template, args)
}
fn collect_type(
    t: &TypeExpr,
    names: &BTreeMap<String, String>,
    seen: &mut BTreeSet<String>,
    requests: &mut Vec<(String, Vec<TypeExpr>)>,
    subst: &BTreeMap<String, TypeExpr>,
) -> Result<(), Error> {
    collect_generic_uses(&substitute_type_expr(t, subst), names, seen, requests)
}

pub fn substitute_type_expr(expr: &TypeExpr, subst: &BTreeMap<String, TypeExpr>) -> TypeExpr {
    match expr {
        TypeExpr::Named {
            path,
            generic_args,
            span,
        } if path.segments.len() == 1
            && generic_args.is_empty()
            && subst.contains_key(&path.segments[0].name) =>
        {
            subst[&path.segments[0].name].clone()
        }
        TypeExpr::Named {
            path,
            generic_args,
            span,
        } => TypeExpr::Named {
            path: path.clone(),
            generic_args: generic_args
                .iter()
                .map(|a| substitute_type_expr(a, subst))
                .collect(),
            span: span.clone(),
        },
        TypeExpr::Array { elem, size, span } => TypeExpr::Array {
            elem: Box::new(substitute_type_expr(elem, subst)),
            size: size.clone(),
            span: span.clone(),
        },
        TypeExpr::Tuple(parts, span) => TypeExpr::Tuple(
            parts
                .iter()
                .map(|p| substitute_type_expr(p, subst))
                .collect(),
            span.clone(),
        ),
        TypeExpr::Projection { class, inner, span } => TypeExpr::Projection {
            class: *class,
            inner: Box::new(substitute_type_expr(inner, subst)),
            span: span.clone(),
        },
        other => other.clone(),
    }
}

/// m61b: comprehensive walk for generic instantiations
/// across the entire program. Calls `collect_generic_uses` at
/// every TypeExpr-bearing site we care about: type decl
/// fields, fn / locus signatures, bus payload types, let
/// ascriptions in fn bodies. The walk is purely structural —
/// the typechecker has already validated names — and runs
/// before any synthesis so the synthesized instantiations
/// land in user_types before any concrete decl's bodies are
/// resolved.
fn collect_generic_uses_in_program(
    program: &Program,
    generic_names: &BTreeMap<String, String>,
    seen: &mut BTreeSet<String>,
    requests: &mut Vec<(String, Vec<TypeExpr>)>,
    subst: &BTreeMap<String, TypeExpr>,
) -> Result<(), Error> {
    fn mentions_parameter(te: &TypeExpr, names: &BTreeSet<&str>) -> bool {
        match te {
            TypeExpr::Named {
                path, generic_args, ..
            } => {
                (path.segments.len() == 1 && names.contains(path.segments[0].name.as_str()))
                    || generic_args.iter().any(|t| mentions_parameter(t, names))
            }
            TypeExpr::Projection { inner, .. } => mentions_parameter(inner, names),
            TypeExpr::Array { elem, .. } | TypeExpr::Bounded { elem, .. } => {
                mentions_parameter(elem, names)
            }
            TypeExpr::Tuple(parts, _) => parts.iter().any(|t| mentions_parameter(t, names)),
            TypeExpr::Function { params, ret, .. } => {
                params.iter().any(|t| mentions_parameter(t, names))
                    || ret.as_ref().is_some_and(|t| mentions_parameter(t, names))
            }
            TypeExpr::Primitive(..) | TypeExpr::Perspective { .. } => false,
        }
    }
    // GH #884: module nesting flattened — a generic
    // instantiation written one brace deeper needs the same
    // monomorph synthesized, and the `TopDecl::Module` arm
    // below is now "already descended into" rather than
    // "skipped".
    for item in hale_syntax::ast::flat_decls(&program.items) {
        match item {
            TopDecl::Type(t) if t.generics.is_empty() => {
                match &t.body {
                    TypeDeclBody::Struct(fields) => {
                        for f in fields {
                            collect_type(&f.ty, generic_names, seen, requests, subst)?;
                        }
                    }
                    // m61c: enum variant field types can
                    // reference generic instantiations
                    // (e.g., a non-generic enum variant
                    // carrying a Box<Int> payload).
                    TypeDeclBody::Enum(variants) => {
                        for v in variants {
                            for f in &v.fields {
                                collect_type(f, generic_names, seen, requests, subst)?;
                            }
                        }
                    }
                    // GH #759: an alias may NAME a generic
                    // instantiation (`type Names =
                    // Vec<String>;`). Walk its target so the
                    // monomorph is synthesized even when the
                    // program only ever spells the alias.
                    TypeDeclBody::Alias(te) => {
                        collect_type(te, generic_names, seen, requests, subst)?;
                    }
                    // GH #1076: a unit-dialect scalar names `Int` or
                    // another scalar, never a generic instantiation.
                    TypeDeclBody::Scalar(_) => {}
                }
            }
            TopDecl::Type(_) => {
                /* generic template — its own body's `T`
                 * references aren't instantiations. */
            }
            TopDecl::Fn(f) if f.generics.is_empty() => {
                collect_in_fn_decl(f, generic_names, seen, requests, subst)?;
            }
            TopDecl::Fn(f) => {
                // Like type and locus templates, unbound fn
                // annotations are not concrete instantiations.
                // Keep concretely spelled uses (including builtin
                // Option/Result); the typed rows supply bound uses.
                let names = f.generics.iter().map(|g| g.name.name.as_str()).collect();
                let mut local_seen = BTreeSet::new();
                let mut local_requests = Vec::new();
                collect_in_fn_decl(
                    f,
                    generic_names,
                    &mut local_seen,
                    &mut local_requests,
                    subst,
                )?;
                for (name, args) in local_requests {
                    if !args.iter().any(|t| mentions_parameter(t, &names)) {
                        let mangled = mangle_generic_name(&name, &args)?;
                        if seen.insert(mangled) {
                            requests.push((name, args));
                        }
                    }
                }
            }
            TopDecl::Locus(l) if l.generics.is_empty() => {
                for member in &l.members {
                    collect_in_locus_member(member, generic_names, seen, requests, subst)?;
                }
            }
            TopDecl::Locus(_) => {
                /* m63: generic locus template — its members'
                 * type positions still mention the generic
                 * params (e.g. `Box<T>`) which can't unify
                 * against any actual concrete type yet.
                 * Synthesis later walks the substituted
                 * version. */
            }
            TopDecl::Const(c) => {
                collect_type(&c.ty, generic_names, seen, requests, subst)?;
            }
            TopDecl::Perspective(_) | TopDecl::Module(_) => {
                /* Perspective type-bearing positions could grow
                 * into this when that feature lands in v0.1
                 * codegen. A Module carries nothing of its own:
                 * GH #884 made the loop above flatten it, so its
                 * declarations have already been visited as
                 * themselves by the time this arm sees the node. */
            }
            TopDecl::Interface(_) => {
                /* Interface declarations have no body to walk
                 * for generic uses. Method signatures are
                 * captured separately during the impl-check
                 * pass. */
            }
            TopDecl::Topic(t) => {
                /* The topic's payload type may mention a
                 * generic name. Capture it the same way
                 * other type-bearing positions do. */
                collect_type(&t.payload, generic_names, seen, requests, subst)?;
            }
            TopDecl::Target(_) => {
                // FUv0.8.2 #7: target capability blocks
                // carry no type-bearing positions; nothing
                // to collect for generic monomorph.
            }
            TopDecl::RingLayout(_) => {
                // ring_layout members are layout tokens, not
                // type-bearing positions; nothing to collect.
            }
            TopDecl::Group(_) => {
                // GH #382: groups name decls, not types —
                // no type-bearing positions.
            }
            TopDecl::Role(_) => {
                // GH #1109: roles name roles — no type-bearing
                // positions, and no code.
            }
            TopDecl::Claims(_) | TopDecl::Constitution(_) => {
                // #392 / #409: claims and constitutions lower to
                // no code and carry no type-bearing positions.
            }
            TopDecl::Unit(_) => {
                // GH #1076: a unit is a node of the catalogue the
                // checker closes; it lowers to no code.
            }
            TopDecl::Api(_) => {
                // GH #1417: a surface is model rows; it lowers to no
                // code until a serve site serves it (R2).
            }
            TopDecl::Fleet(_) => {
                // Phase 4 (C2): a fleet is checked, never built; the
                // entry refuses it before anything lowers.
            }
        }
    }
    Ok(())
}

fn collect_in_fn_decl(
    f: &FnDecl,
    generic_names: &BTreeMap<String, String>,
    seen: &mut BTreeSet<String>,
    requests: &mut Vec<(String, Vec<TypeExpr>)>,
    subst: &BTreeMap<String, TypeExpr>,
) -> Result<(), Error> {
    for p in &f.params {
        collect_type(&p.ty, generic_names, seen, requests, subst)?;
    }
    if let Some(ret) = &f.ret {
        collect_type(ret, generic_names, seen, requests, subst)?;
    }
    collect_in_block(&f.body, generic_names, seen, requests, subst)?;
    Ok(())
}

fn collect_in_locus_member(
    member: &LocusMember,
    generic_names: &BTreeMap<String, String>,
    seen: &mut BTreeSet<String>,
    requests: &mut Vec<(String, Vec<TypeExpr>)>,
    subst: &BTreeMap<String, TypeExpr>,
) -> Result<(), Error> {
    match member {
        LocusMember::Params(pb) => {
            for p in &pb.params {
                if let Some(t) = &p.ty {
                    collect_type(
                        t,
                        generic_names, seen, requests, subst,
                    )?;
                }
            }
        }
        LocusMember::Bus(bb) => {
            for bm in &bb.members {
                match bm {
                    BusMember::Subscribe { ty: Some(t), .. } => {
                        collect_type(
                            t,
                            generic_names, seen, requests, subst,
                        )?;
                    }
                    BusMember::Publish { ty: Some(t), .. } => {
                        collect_type(
                            t,
                            generic_names, seen, requests, subst,
                        )?;
                    }
                    BusMember::Subscribe { ty: None, .. }
                    | BusMember::Publish { ty: None, .. } => {}
                }
            }
        }
        LocusMember::Lifecycle(lc) => {
            for p in &lc.params {
                collect_type(
                    &p.ty,
                    generic_names, seen, requests, subst,
                )?;
            }
            if let Some(r) = &lc.ret {
                collect_type(
                    r,
                    generic_names, seen, requests, subst,
                )?;
            }
            collect_in_block(
                &lc.body,
                generic_names, seen, requests, subst,
            )?;
        }
        LocusMember::Fn(fd) => {
            collect_in_fn_decl(
                fd,
                generic_names, seen, requests, subst,
            )?;
        }
        LocusMember::Mode(_) => {
            /* Modes' body / params walk would mirror Fn; not
             * yet wired. m61c. */
        }
        LocusMember::Const(c) => {
            collect_type(
                &c.ty,
                generic_names, seen, requests, subst,
            )?;
        }
        LocusMember::Type(t) if t.generics.is_empty() => {
            match &t.body {
                TypeDeclBody::Struct(fields) => {
                    for f in fields {
                        collect_type(
                            &f.ty,
                            generic_names, seen, requests, subst,
                        )?;
                    }
                }
                TypeDeclBody::Enum(variants) => {
                    for v in variants {
                        for f in &v.fields {
                            collect_type(
                                f,
                                generic_names, seen, requests, subst,
                            )?;
                        }
                    }
                }
                // GH #759: see the top-level arm.
                TypeDeclBody::Alias(te) => {
                    collect_type(
                        te,
                        generic_names, seen, requests, subst,
                    )?;
                }
                // GH #1076: see the top-level arm.
                TypeDeclBody::Scalar(_) => {}
            }
        }
        LocusMember::Failure(fd) => {
            for param in &fd.params {
                collect_type(&param.ty, generic_names, seen, requests, subst)?;
            }
            collect_in_block(&fd.body, generic_names, seen, requests, subst)?;
        }
        LocusMember::Contract(_)
        | LocusMember::Closure(_)
        | LocusMember::Type(_)
        | LocusMember::Bindings(_)
        | LocusMember::Placement(_)
        | LocusMember::Topology(_)
        // GH #382: claims carry no type-bearing positions.
        | LocusMember::Claims(_)
        | LocusMember::BirthCheck(_)
        | LocusMember::Seam(_) => {}
        LocusMember::Capacity(_) => {
            // F.22 slot cell types are concrete in v1; no
            // generic-template use sites. Future generic Pool/
            // Heap (`pool entries of T;` with locus generic
            // T) would walk slot.elem_ty here.
        }
    }
    Ok(())
}

fn collect_in_block(
    block: &Block,
    generic_names: &BTreeMap<String, String>,
    seen: &mut BTreeSet<String>,
    requests: &mut Vec<(String, Vec<TypeExpr>)>,
    subst: &BTreeMap<String, TypeExpr>,
) -> Result<(), Error> {
    for stmt in &block.stmts {
        collect_in_stmt(stmt, generic_names, seen, requests, subst)?;
    }
    Ok(())
}

fn collect_in_stmt(
    stmt: &Stmt,
    generic_names: &BTreeMap<String, String>,
    seen: &mut BTreeSet<String>,
    requests: &mut Vec<(String, Vec<TypeExpr>)>,
    subst: &BTreeMap<String, TypeExpr>,
) -> Result<(), Error> {
    match stmt {
        Stmt::Let { ty: Some(t), .. } => {
            collect_type(t, generic_names, seen, requests, subst)?;
        }
        Stmt::LetTuple { ty: Some(t), .. } => {
            collect_type(t, generic_names, seen, requests, subst)?;
        }
        _ => {}
    }
    Ok(())
}

/// m61: walk a TypeExpr looking for generic instantiations
/// that reference one of the known generic templates. Each
/// unique (template_name, args) tuple is recorded in
/// `requests` (deduped via the mangled name in `seen`), with
/// recursion into the args themselves so nested generics like
/// `Box<Pair<Int, String>>` register `Pair_Int_String` first
/// then `Box_Pair_Int_String`. Discovery is purely structural
/// — the typechecker has already validated names.
fn collect_generic_uses(
    t: &TypeExpr,
    generic_names: &BTreeMap<String, String>,
    seen: &mut BTreeSet<String>,
    requests: &mut Vec<(String, Vec<TypeExpr>)>,
) -> Result<(), Error> {
    match t {
        TypeExpr::Named { path, generic_args, .. } => {
            for arg in generic_args {
                collect_generic_uses(arg, generic_names, seen, requests)?;
            }
            if !generic_args.is_empty() {
                let written = path.segments.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join("::");
                if let Some(template) = generic_names.get(&written) {
                    let mangled = mangle_generic_name(template, generic_args)?;
                    if seen.insert(mangled) {
                        requests.push((template.clone(), generic_args.clone()));
                    }
                }
            }
        }
        TypeExpr::Bounded { elem, .. } => collect_generic_uses(
            elem,
            generic_names,
            seen,
            requests,
        )?,
        TypeExpr::Array { elem, .. } => collect_generic_uses(
            elem,
            generic_names,
            seen,
            requests,
        )?,
        TypeExpr::Tuple(parts, _) => {
            for p in parts {
                collect_generic_uses(
                    p,
                    generic_names,
                    seen,
                    requests,
                )?;
            }
        }
        TypeExpr::Projection { inner, .. } => collect_generic_uses(
            inner,
            generic_names,
            seen,
            requests,
        )?,
        TypeExpr::Primitive(_, _)
        | TypeExpr::Function { .. }
        // Phase 2a: `perspective(P)` carries no generic args.
        | TypeExpr::Perspective { .. } => {}
    }
    Ok(())
}

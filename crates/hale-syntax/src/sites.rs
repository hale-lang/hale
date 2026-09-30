//! The one walk over the AST's identity fields (F.40 phase 1.1b).
//!
//! Every declaration, member and statement site that the snapshot
//! numbers carries an `id: NodeId`, as do the two expression shapes
//! the ownership pre-pass keys its rows by (`Expr::Struct`,
//! `Expr::Call`), every identifier expression (`Expr::Ident`, a `Use`)
//! and every name a declaration with no site of its own binds (a
//! parameter, a pattern binding: a `Binder`), those two by their
//! `Ident`'s id. This module is the traversal that reaches all of
//! them, so the minting pass and every later reader agree on which
//! sites exist and in what order.
//!
//! The mutable and read-only walks are one body instantiated twice by
//! a macro, so they cannot drift apart. Matches over the AST's enums
//! name every variant: a new variant is a compile error here rather
//! than a site the walk silently skips. Type expressions are walked
//! too, because an array type's size is an expression and may hold a
//! call.

use crate::ast::*;
use crate::span::Span;

/// Which kind of semantic site an identity field belongs to.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum SiteKind {
    Locus,
    Fn,
    Topic,
    Type,
    Interface,
    Closure,
    Lifecycle,
    Mode,
    Failure,
    Perspective,
    Const,
    Group,
    Module,
    Param,
    PlacementEntry,
    BindingEntry,
    Publish,
    Subscribe,
    Let,
    LetTuple,
    Assign,
    For,
    Send,
    StructLiteral,
    Call,
    /// An identifier expression (`Expr::Ident`): a name spelled where a
    /// value is read (F.40 phase 2, use-site identity).
    Use,
    /// A name bound by a declaration that has no site of its own: a
    /// parameter of a fn or a hook, a match pattern's binding, a tuple
    /// `let`'s names, a `shm_write` binding. Its id is its `Ident`'s. A
    /// `let` and a `for` bind one name each and are their own sites.
    Binder,
}

/// Visit every identity field of the program in pre-order (a
/// declaration before its members, a member before its body, a
/// statement before the expressions it holds), with the site's kind
/// and its span. The one traversal every reader of identities
/// shares; the minting pass (hale-types) is its first caller.
pub fn for_each_site_mut(
    program: &mut Program,
    f: &mut dyn FnMut(SiteKind, Span, &mut NodeId),
) {
    mutable::program(program, &mut |kind, span, _, id| f(kind, span, id));
}

/// The same traversal, read-only.
pub fn for_each_site(
    program: &Program,
    f: &mut dyn FnMut(SiteKind, Span, NodeId),
) {
    shared::program(program, &mut |kind, span, _, id: &NodeId| {
        f(kind, span, *id)
    });
}

/// The same traversal, read-only, with each site's name where the node
/// carries one as a plain identifier: a declaration's name, the binding
/// of a `let` or `for`, the head of an assignment's target, the name a
/// use spells. `None` for
/// the sites that have none (a lifecycle, a publish, a call).
pub fn for_each_named_site(
    program: &Program,
    f: &mut dyn FnMut(SiteKind, Span, Option<&str>, NodeId),
) {
    shared::program(program, &mut |kind, span, name, id: &NodeId| {
        f(kind, span, name, *id)
    });
}

/// The read-only traversal of one top-level declaration: the sites
/// `for_each_site` visits for it, in the same order.
pub fn for_each_site_in_item(
    item: &TopDecl,
    f: &mut dyn FnMut(SiteKind, Span, NodeId),
) {
    shared::top_decl(item, &mut |kind, span, _, id: &NodeId| {
        f(kind, span, *id)
    });
}

/// Make a copied expression a new site: every identity field in the
/// subtree becomes `NodeId::NONE`, so the next mint numbers it instead
/// of finding the original's id on two sites (a panic). Built on the
/// walk, so a site kind added there is cleared here too.
pub fn clear_ids_in_expr(e: &mut Expr) {
    mutable::expr(e, &mut |_, _, _, id| *id = NodeId::NONE);
}

/// `clear_ids_in_expr` for a copied type expression: an array type's
/// size is an expression and may hold a call.
pub fn clear_ids_in_type(t: &mut TypeExpr) {
    mutable::ty(t, &mut |_, _, _, id| *id = NodeId::NONE);
}

/// `clear_ids_in_expr` for a copied bus member: the member's own id and
/// every identity inside it (a key filter's expression, the payload
/// type).
pub fn clear_ids_in_bus_member(m: &mut BusMember) {
    mutable::bus_member(m, &mut |_, _, _, id| *id = NodeId::NONE);
}

/// The walk, written once. `$m` is `mut` for the mutable instance and
/// empty for the read-only one; match ergonomics carry it from the
/// scrutinee into every binding.
macro_rules! walk {
    ($module:ident, $($m:tt)?) => {
        mod $module {
            use super::*;

            type Visit<'v> =
                dyn FnMut(SiteKind, Span, Option<&str>, & $($m)? NodeId) + 'v;

            pub(super) fn program(p: & $($m)? Program, f: &mut Visit<'_>) {
                items(& $($m)? p.items, f);
            }

            fn items(items: & $($m)? [TopDecl], f: &mut Visit<'_>) {
                for item in items {
                    top_decl(item, f);
                }
            }

            pub(super) fn top_decl(d: & $($m)? TopDecl, f: &mut Visit<'_>) {
                match d {
                    TopDecl::Locus(l) => locus(l, f),
                    TopDecl::Perspective(p) => {
                        f(SiteKind::Perspective, p.span, Some(p.name.name.as_str()), & $($m)? p.id);
                        generics(& $($m)? p.generics, f);
                        for member in & $($m)? p.members {
                            match member {
                                PerspectiveMember::Params(pb) => params_block(pb, f),
                                PerspectiveMember::StableWhen(b) => block(b, f),
                                PerspectiveMember::SerializeAs(t) => ty(t, f),
                                PerspectiveMember::Fn(fd) => fn_decl(fd, f),
                                PerspectiveMember::Bus(bb) => bus(bb, f),
                            }
                        }
                    }
                    TopDecl::Type(t) => type_decl(t, f),
                    TopDecl::Const(c) => const_decl(c, f),
                    TopDecl::Fn(fd) => fn_decl(fd, f),
                    TopDecl::Module(md) => {
                        f(SiteKind::Module, md.span, Some(md.name.name.as_str()), & $($m)? md.id);
                        items(& $($m)? md.items, f);
                    }
                    TopDecl::Interface(i) => {
                        f(SiteKind::Interface, i.span, Some(i.name.name.as_str()), & $($m)? i.id);
                        for sig in & $($m)? i.methods {
                            params(& $($m)? sig.params, f);
                            opt_ty(& $($m)? sig.ret, f);
                            opt_ty(& $($m)? sig.fallible, f);
                        }
                    }
                    TopDecl::Topic(t) => {
                        f(SiteKind::Topic, t.span, Some(t.name.name.as_str()), & $($m)? t.id);
                        ty(& $($m)? t.payload, f);
                    }
                    TopDecl::Group(g) => {
                        f(SiteKind::Group, g.span, Some(g.name.name.as_str()), & $($m)? g.id);
                    }
                    // No identity and no expression inside.
                    TopDecl::RingLayout(_)
                    | TopDecl::Target(_)
                    | TopDecl::Role(_)
                    | TopDecl::Claims(_)
                    | TopDecl::Constitution(_) => {}
                }
            }

            fn locus(l: & $($m)? LocusDecl, f: &mut Visit<'_>) {
                f(SiteKind::Locus, l.span, Some(l.name.name.as_str()), & $($m)? l.id);
                generics(& $($m)? l.generics, f);
                if let Some(form) = & $($m)? l.form {
                    for arg in & $($m)? form.args {
                        expr(& $($m)? arg.value, f);
                    }
                }
                for member in & $($m)? l.members {
                    locus_member(member, f);
                }
            }

            fn locus_member(member: & $($m)? LocusMember, f: &mut Visit<'_>) {
                match member {
                    LocusMember::Params(pb) => params_block(pb, f),
                    LocusMember::Contract(cb) => match & $($m)? cb.kind {
                        ContractKind::Inferred => {}
                        ContractKind::Members(ms) => {
                            for cm in ms {
                                opt_ty(& $($m)? cm.ty, f);
                            }
                        }
                    },
                    LocusMember::Bus(bb) => bus(bb, f),
                    LocusMember::Lifecycle(ld) => {
                        f(SiteKind::Lifecycle, ld.span, None, & $($m)? ld.id);
                        bound_params(& $($m)? ld.params, f);
                        opt_ty(& $($m)? ld.ret, f);
                        block(& $($m)? ld.body, f);
                    }
                    LocusMember::Mode(md) => {
                        f(SiteKind::Mode, md.span, None, & $($m)? md.id);
                        bound_params(& $($m)? md.params, f);
                        opt_ty(& $($m)? md.ret, f);
                        block(& $($m)? md.body, f);
                    }
                    LocusMember::Failure(fd) => {
                        f(SiteKind::Failure, fd.span, None, & $($m)? fd.id);
                        bound_params(& $($m)? fd.params, f);
                        block(& $($m)? fd.body, f);
                    }
                    LocusMember::Closure(cd) => {
                        f(SiteKind::Closure, cd.span, Some(cd.name.name.as_str()), & $($m)? cd.id);
                        if let Some(a) = & $($m)? cd.assertion {
                            expr(& $($m)? a.left, f);
                            expr(& $($m)? a.right, f);
                            expr(& $($m)? a.tolerance, f);
                        }
                        for clause in & $($m)? cd.clauses {
                            match clause {
                                ClosureClause::Epoch(spec) => match spec {
                                    EpochSpec::Duration(e) => expr(e, f),
                                    EpochSpec::Tick
                                    | EpochSpec::Birth
                                    | EpochSpec::Dissolve
                                    | EpochSpec::Explicit
                                    | EpochSpec::Inline => {}
                                },
                                ClosureClause::PersistsThrough(_)
                                | ClosureClause::ResetsOn(_)
                                | ClosureClause::ResetsPerEpoch(_)
                                | ClosureClause::Captures(_) => {}
                            }
                        }
                    }
                    LocusMember::Fn(fd) => fn_decl(fd, f),
                    LocusMember::Const(c) => const_decl(c, f),
                    LocusMember::Type(t) => type_decl(t, f),
                    LocusMember::Capacity(cb) => {
                        for slot in & $($m)? cb.slots {
                            ty(& $($m)? slot.elem_ty, f);
                        }
                    }
                    LocusMember::Bindings(bb) => {
                        for entry in & $($m)? bb.entries {
                            f(SiteKind::BindingEntry, entry.span, None, & $($m)? entry.id);
                            match & $($m)? entry.transport {
                                TransportSpec::Adapter { inits, .. } => {
                                    struct_inits(inits, f)
                                }
                                TransportSpec::Unix { .. }
                                | TransportSpec::ShmRing { .. } => {}
                            }
                            if let Some(codec) = & $($m)? entry.codec {
                                struct_inits(& $($m)? codec.inits, f);
                            }
                        }
                        if let Some(api) = & $($m)? bb.api {
                            match & $($m)? api.transport {
                                ApiTransport::Unix { path, .. } => expr(path, f),
                            }
                            if let Some(roles) = & $($m)? api.roles {
                                expr(& $($m)? roles.expr, f);
                            }
                            if let Some(http) = & $($m)? api.http {
                                expr(& $($m)? http.host, f);
                                expr(& $($m)? http.port, f);
                                opt_expr(& $($m)? http.principals, f);
                            }
                        }
                    }
                    LocusMember::Placement(pb) => {
                        for entry in & $($m)? pb.entries {
                            f(
                                SiteKind::PlacementEntry,
                                entry.span,
                                Some(entry.field.name.as_str()),
                                & $($m)? entry.id,
                            );
                        }
                    }
                    LocusMember::BirthCheck(bc) => {
                        expr(& $($m)? bc.cond, f);
                        opt_expr(& $($m)? bc.payload, f);
                    }
                    // No identity and no expression inside.
                    LocusMember::Topology(_) | LocusMember::Claims(_) => {}
                }
            }

            fn params_block(pb: & $($m)? ParamsBlock, f: &mut Visit<'_>) {
                for p in & $($m)? pb.params {
                    f(SiteKind::Param, p.span, Some(p.name.name.as_str()), & $($m)? p.id);
                    opt_ty(& $($m)? p.ty, f);
                    match & $($m)? p.init {
                        ParamInit::Value(e) => expr(e, f),
                        ParamInit::Inferred => {}
                    }
                }
            }

            fn bus(bb: & $($m)? BusBlock, f: &mut Visit<'_>) {
                for member in & $($m)? bb.members {
                    bus_member(member, f);
                }
            }

            pub(super) fn bus_member(member: & $($m)? BusMember, f: &mut Visit<'_>) {
                match member {
                    BusMember::Subscribe { ty: t, key_filter, span, id, .. } => {
                        f(SiteKind::Subscribe, *span, None, id);
                        opt_ty(t, f);
                        if let Some(kf) = key_filter {
                            match kf {
                                KeyFilter::Specific { expr: e, .. } => expr(e, f),
                                KeyFilter::Unmatched { .. }
                                | KeyFilter::Replica { .. } => {}
                            }
                        }
                    }
                    BusMember::Publish { ty: t, span, id, .. } => {
                        f(SiteKind::Publish, *span, None, id);
                        opt_ty(t, f);
                    }
                }
            }

            fn fn_decl(fd: & $($m)? FnDecl, f: &mut Visit<'_>) {
                f(SiteKind::Fn, fd.span, Some(fd.name.name.as_str()), & $($m)? fd.id);
                generics(& $($m)? fd.generics, f);
                bound_params(& $($m)? fd.params, f);
                opt_ty(& $($m)? fd.ret, f);
                opt_ty(& $($m)? fd.fallible, f);
                block(& $($m)? fd.body, f);
            }

            fn type_decl(t: & $($m)? TypeDecl, f: &mut Visit<'_>) {
                f(SiteKind::Type, t.span, Some(t.name.name.as_str()), & $($m)? t.id);
                generics(& $($m)? t.generics, f);
                match & $($m)? t.body {
                    TypeDeclBody::Alias(a) => ty(a, f),
                    TypeDeclBody::Struct(fields) => {
                        for field in fields {
                            ty(& $($m)? field.ty, f);
                            opt_expr(& $($m)? field.default, f);
                        }
                    }
                    TypeDeclBody::Enum(variants) => {
                        for v in variants {
                            for t in & $($m)? v.fields {
                                ty(t, f);
                            }
                        }
                    }
                }
            }

            fn const_decl(c: & $($m)? ConstDecl, f: &mut Visit<'_>) {
                f(SiteKind::Const, c.span, Some(c.name.name.as_str()), & $($m)? c.id);
                ty(& $($m)? c.ty, f);
                expr(& $($m)? c.value, f);
            }

            fn generics(gs: & $($m)? [GenericParam], f: &mut Visit<'_>) {
                for g in gs {
                    opt_ty(& $($m)? g.bound, f);
                }
            }

            /// A signature's parameters, which bind nothing (an
            /// interface method's).
            fn params(ps: & $($m)? [Param], f: &mut Visit<'_>) {
                for p in ps {
                    ty(& $($m)? p.ty, f);
                    opt_expr(& $($m)? p.default, f);
                }
            }

            /// The parameters of a declaration with a body: each name
            /// is a binder.
            fn bound_params(ps: & $($m)? [Param], f: &mut Visit<'_>) {
                for p in ps {
                    binder(& $($m)? p.name, f);
                    ty(& $($m)? p.ty, f);
                    opt_expr(& $($m)? p.default, f);
                }
            }

            fn binder(i: & $($m)? Ident, f: &mut Visit<'_>) {
                f(SiteKind::Binder, i.span, Some(i.name.as_str()), & $($m)? i.id);
            }

            fn pattern(p: & $($m)? Pattern, f: &mut Visit<'_>) {
                match p {
                    Pattern::Binding(i) => binder(i, f),
                    Pattern::Constructor { args, .. } | Pattern::Tuple(args, _) => {
                        for a in args {
                            pattern(a, f);
                        }
                    }
                    Pattern::Literal(..) | Pattern::Wildcard(_) => {}
                }
            }

            fn opt_ty(t: & $($m)? Option<TypeExpr>, f: &mut Visit<'_>) {
                if let Some(t) = t {
                    ty(t, f);
                }
            }

            pub(super) fn ty(t: & $($m)? TypeExpr, f: &mut Visit<'_>) {
                match t {
                    TypeExpr::Named { generic_args, .. } => {
                        for a in generic_args {
                            ty(a, f);
                        }
                    }
                    TypeExpr::Projection { inner, .. } => ty(inner, f),
                    TypeExpr::Array { elem, size, .. } => {
                        ty(elem, f);
                        opt_expr(size, f);
                    }
                    TypeExpr::Bounded { elem, .. } => ty(elem, f),
                    TypeExpr::Tuple(elems, _) => {
                        for e in elems {
                            ty(e, f);
                        }
                    }
                    TypeExpr::Function { params, ret, .. } => {
                        for p in params {
                            ty(p, f);
                        }
                        if let Some(r) = ret {
                            ty(r, f);
                        }
                    }
                    TypeExpr::Primitive(..) | TypeExpr::Perspective { .. } => {}
                }
            }

            fn block(b: & $($m)? Block, f: &mut Visit<'_>) {
                for s in & $($m)? b.stmts {
                    stmt(s, f);
                }
                if let Some(tail) = & $($m)? b.tail {
                    expr(tail, f);
                }
            }

            fn stmt(s: & $($m)? Stmt, f: &mut Visit<'_>) {
                match s {
                    Stmt::Let { name, ty: t, value, span, id, .. } => {
                        f(SiteKind::Let, *span, Some(name.name.as_str()), id);
                        opt_ty(t, f);
                        expr(value, f);
                    }
                    Stmt::LetTuple { names, ty: t, value, span, id, .. } => {
                        f(SiteKind::LetTuple, *span, None, id);
                        for n in names {
                            binder(n, f);
                        }
                        opt_ty(t, f);
                        expr(value, f);
                    }
                    Stmt::Assign { target, value, span, id, .. } => {
                        f(SiteKind::Assign, *span, Some(target.head.name.as_str()), id);
                        for seg in & $($m)? target.tail {
                            match seg {
                                LValueSeg::Index(e) => expr(e, f),
                                LValueSeg::Field(_) => {}
                            }
                        }
                        expr(value, f);
                    }
                    Stmt::If(i) => if_stmt(i, f),
                    Stmt::Match(ms) => match_stmt(ms, f),
                    Stmt::For { name, iter, body, span, id } => {
                        f(SiteKind::For, *span, Some(name.name.as_str()), id);
                        expr(iter, f);
                        block(body, f);
                    }
                    Stmt::While { cond, body, .. } => {
                        expr(cond, f);
                        block(body, f);
                    }
                    Stmt::Return(e, _) => opt_expr(e, f),
                    Stmt::Fail { value, .. } => expr(value, f),
                    Stmt::Block(b) => block(b, f),
                    Stmt::Recovery { args, modifier, .. } => {
                        for a in args {
                            expr(a, f);
                        }
                        if let Some(md) = modifier {
                            match md {
                                RecoveryModifier::For(e) | RecoveryModifier::Until(e) => {
                                    expr(e, f)
                                }
                            }
                        }
                    }
                    Stmt::Violate { payload, .. } => opt_expr(payload, f),
                    Stmt::Send { subject, value, or_disposition, span, id } => {
                        f(SiteKind::Send, *span, None, id);
                        expr(subject, f);
                        expr(value, f);
                        if let Some(d) = or_disposition {
                            disposition(d, f);
                        }
                    }
                    Stmt::ShmWrite { max, binding, body, .. } => {
                        expr(max, f);
                        binder(binding, f);
                        block(body, f);
                    }
                    Stmt::Expr(e) => expr(e, f),
                    Stmt::Break(_)
                    | Stmt::Continue(_)
                    | Stmt::Yield(_)
                    | Stmt::Terminate(_)
                    | Stmt::Reperspective { .. } => {}
                }
            }

            fn if_stmt(i: & $($m)? IfStmt, f: &mut Visit<'_>) {
                expr(& $($m)? i.cond, f);
                block(& $($m)? i.then_block, f);
                if let Some(eb) = & $($m)? i.else_block {
                    match & $($m)? **eb {
                        ElseBranch::Else(b) => block(b, f),
                        ElseBranch::ElseIf(i) => if_stmt(i, f),
                    }
                }
            }

            fn match_stmt(ms: & $($m)? MatchStmt, f: &mut Visit<'_>) {
                expr(& $($m)? ms.scrutinee, f);
                for arm in & $($m)? ms.arms {
                    pattern(& $($m)? arm.pattern, f);
                    opt_expr(& $($m)? arm.guard, f);
                    match & $($m)? arm.body {
                        MatchArmBody::Expr(e) => expr(e, f),
                        MatchArmBody::Block(b) => block(b, f),
                    }
                }
            }

            fn disposition(d: & $($m)? OrDisposition, f: &mut Visit<'_>) {
                match d {
                    OrDisposition::Substitute(e) | OrDisposition::Fail(e, _) => expr(e, f),
                    OrDisposition::Raise(_)
                    | OrDisposition::Discard(_)
                    | OrDisposition::Wait(_) => {}
                }
            }

            fn struct_inits(inits: & $($m)? [StructInit], f: &mut Visit<'_>) {
                for init in inits {
                    expr(& $($m)? init.value, f);
                }
            }

            fn opt_expr(e: & $($m)? Option<Expr>, f: &mut Visit<'_>) {
                if let Some(e) = e {
                    expr(e, f);
                }
            }

            pub(super) fn expr(e: & $($m)? Expr, f: &mut Visit<'_>) {
                match e {
                    Expr::Call { callee, args, span, id } => {
                        f(SiteKind::Call, *span, None, id);
                        expr(callee, f);
                        for a in args {
                            expr(a, f);
                        }
                    }
                    Expr::Struct { inits, span, id, .. } => {
                        f(SiteKind::StructLiteral, *span, None, id);
                        struct_inits(inits, f);
                    }
                    Expr::Binary { left, right, .. } => {
                        expr(left, f);
                        expr(right, f);
                    }
                    Expr::Unary { operand, .. } => expr(operand, f),
                    Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. } => {
                        expr(receiver, f)
                    }
                    Expr::Index { receiver, index, .. } => {
                        expr(receiver, f);
                        expr(index, f);
                    }
                    Expr::Tuple(parts, _) | Expr::Array(parts, _) => {
                        for p in parts {
                            expr(p, f);
                        }
                    }
                    Expr::Block(b) => block(b, f),
                    Expr::If(i) => if_stmt(i, f),
                    Expr::Match(ms) => match_stmt(ms, f),
                    Expr::Sum(inner, _) | Expr::Prod(inner, _) => expr(inner, f),
                    Expr::Approx { left, right, tolerance, .. } => {
                        expr(left, f);
                        expr(right, f);
                        expr(tolerance, f);
                    }
                    Expr::Range { lo, hi, .. } => {
                        expr(lo, f);
                        expr(hi, f);
                    }
                    Expr::ArrayRepeat { val, .. } => expr(val, f),
                    Expr::Or { inner, disposition: d, .. } => {
                        expr(inner, f);
                        disposition(d, f);
                    }
                    Expr::Ident(i) => {
                        f(SiteKind::Use, i.span, Some(i.name.as_str()), & $($m)? i.id);
                    }
                    Expr::Literal(..) | Expr::Path(_) | Expr::KwSelf(_) => {}
                }
            }
        }
    };
}

walk!(mutable, mut);
walk!(shared,);

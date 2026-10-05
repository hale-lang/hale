//! Every name and every text a declaration spells (F.40 phase 3).
//!
//! The one structural walk that answers "what does this program
//! mention": each identifier (a declaration's name, a path segment, a
//! field, a binder, a use) and each `name` the AST carries as a string
//! (a decorator, an shm ring, an effect class in claim position) is
//! reported as a [`Spelled::Name`]; every other string the program
//! writes (a literal, a subject, a tag, a display spelling) as a
//! [`Spelled::Text`]. A reader that asks whether a program mentions
//! something reads this instead of a `Debug` rendering.
//!
//! Matches name every variant and destructure every field that holds a
//! name, a text or a node, so a new variant or a new field holding one
//! is a compile error here rather than a mention the walk silently
//! misses.

use crate::ast::*;

/// One thing a program spells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spelled<'a> {
    /// An identifier, or a string the AST keeps as a `name`.
    Name(&'a str),
    /// Any other string the program writes.
    Text(&'a str),
}

type Visit<'v, 'a> = dyn FnMut(Spelled<'a>) + 'v;

/// Every name and text in `items`, depth-first.
pub fn for_each_spelled<'a>(items: &'a [TopDecl], f: &mut Visit<'_, 'a>) {
    for item in items {
        top_decl(item, f);
    }
}

/// Every name and text in one top-level declaration.
pub fn for_each_spelled_in_item<'a>(item: &'a TopDecl, f: &mut Visit<'_, 'a>) {
    top_decl(item, f);
}

fn name<'a>(i: &'a Ident, f: &mut Visit<'_, 'a>) {
    f(Spelled::Name(&i.name));
}

fn names<'a>(is: &'a [Ident], f: &mut Visit<'_, 'a>) {
    for i in is {
        name(i, f);
    }
}

fn opt_name<'a>(i: &'a Option<Ident>, f: &mut Visit<'_, 'a>) {
    if let Some(i) = i {
        name(i, f);
    }
}

fn text<'a>(s: &'a str, f: &mut Visit<'_, 'a>) {
    f(Spelled::Text(s));
}

fn opt_text<'a>(s: &'a Option<String>, f: &mut Visit<'_, 'a>) {
    if let Some(s) = s {
        text(s, f);
    }
}

fn qualified<'a>(q: &'a QualifiedName, f: &mut Visit<'_, 'a>) {
    let QualifiedName { segments, span: _ } = q;
    names(segments, f);
}

fn top_decl<'a>(d: &'a TopDecl, f: &mut Visit<'_, 'a>) {
    match d {
        TopDecl::Locus(l) => locus(l, f),
        TopDecl::Perspective(p) => {
            let PerspectiveDecl { name: n, generics: gs, members, span: _, id: _ } = p;
            name(n, f);
            generics(gs, f);
            for m in members {
                match m {
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
        TopDecl::Module(m) => {
            let ModuleDecl { name: n, items, span: _, id: _ } = m;
            name(n, f);
            for_each_spelled(items, f);
        }
        TopDecl::Interface(i) => {
            let InterfaceDecl { name: n, methods, span: _, id: _ } = i;
            name(n, f);
            for sig in methods {
                let InterfaceMethodSig { name: n, params: ps, ret, fallible, span: _ } = sig;
                name(n, f);
                params(ps, f);
                opt_ty(ret, f);
                opt_ty(fallible, f);
            }
        }
        TopDecl::Topic(t) => {
            let TopicDecl {
                name: n,
                display,
                parent,
                payload,
                subject,
                keyed_by,
                bounded: _,
                on_full_fail: _,
                on_unmatched: _,
                span: _,
                id: _,
            } = t;
            name(n, f);
            opt_text(display, f);
            opt_name(parent, f);
            ty(payload, f);
            opt_text(subject, f);
            opt_name(keyed_by, f);
        }
        TopDecl::RingLayout(r) => {
            let RingLayoutDecl { name: n, magic: _, data_at: _, scalars, cursors, framing, overflow, span: _ } = r;
            name(n, f);
            for s in scalars {
                let RingScalarField { name: n, expect: _, at: _, repr, span: _ } = s;
                name(n, f);
                name(repr, f);
            }
            for c in cursors {
                let RingCursorBlock { name: n, attrs, span: _ } = c;
                opt_name(n, f);
                ring_attrs(attrs, f);
            }
            if let Some(RingFramingBlock { kind, attrs, span: _ }) = framing {
                name(kind, f);
                ring_attrs(attrs, f);
            }
            opt_name(overflow, f);
        }
        TopDecl::Target(t) => {
            let TargetDecl { name: n, capabilities, span: _, synthesized: _ } = t;
            name(n, f);
            for c in capabilities {
                let Capability { segments, span: _ } = c;
                names(segments, f);
            }
        }
        TopDecl::Group(g) => {
            let GroupDecl { name: n, members, may_be_empty: _, span: _, id: _ } = g;
            name(n, f);
            for m in members {
                let GroupMember { segments, glob: _, span: _ } = m;
                names(segments, f);
            }
        }
        TopDecl::Role(r) => {
            let RoleDecl { name: n, includes, span: _ } = r;
            name(n, f);
            names(includes, f);
        }
        TopDecl::Claims(cb) => claims_block(cb, f),
        TopDecl::Constitution(c) => {
            let ConstitutionDecl { name: n, extends, entries, span: _ } = c;
            name(n, f);
            names(extends, f);
            for e in entries {
                claim(e, f);
            }
        }
        TopDecl::Unit(u) => {
            let UnitDecl { name: n, equation, span: _, id: _ } = u;
            name(n, f);
            if let Some(UnitEquation { num: _, den: _, target, span: _, id: _ }) = equation {
                opt_name(target, f);
            }
        }
    }
}

fn ring_attrs<'a>(attrs: &'a [RingAttr], f: &mut Visit<'_, 'a>) {
    for a in attrs {
        let RingAttr { key, value, span: _ } = a;
        name(key, f);
        match value {
            RingAttrValue::Ident(i) => name(i, f),
            RingAttrValue::Int(_) => {}
        }
    }
}

fn claims_block<'a>(cb: &'a ClaimsBlock, f: &mut Visit<'_, 'a>) {
    let ClaimsBlock { entries, adopts, lib_tier: _, span: _ } = cb;
    for e in entries {
        claim(e, f);
    }
    names(adopts, f);
}

fn claim<'a>(c: &'a ClaimDecl, f: &mut Visit<'_, 'a>) {
    let ClaimDecl { name: n, form, span: _ } = c;
    name(n, f);
    match form {
        ClaimForm::ForbidReaches { src, dst, via_calls: _, via_bus: _, during, avoiding } => {
            claim_set(src, f);
            claim_set(dst, f);
            opt_name(during, f);
            opt_name(avoiding, f);
        }
        ClaimForm::OnlyEdges { src, dst, grants } => {
            name(src, f);
            name(dst, f);
            for g in grants {
                let EdgeGrant { publish: _, topic, span: _ } = g;
                topic_ref(topic, f);
            }
        }
        ClaimForm::Bound { class: _, class_name, class_span: _, limit: _, from } => {
            f(Spelled::Name(class_name));
            name(from, f);
        }
        ClaimForm::Require { publishers: _, group, topic } => {
            name(group, f);
            topic_ref(topic, f);
        }
        ClaimForm::RequireSealed { group } => name(group, f),
        ClaimForm::RequireAttributed { class_name } => name(class_name, f),
        ClaimForm::Cover { alias, group } => {
            name(alias, f);
            name(group, f);
        }
        ClaimForm::Count { publishers: _, topic, cmp: _, n: _ } => topic_ref(topic, f),
    }
}

fn claim_set<'a>(s: &'a ClaimSet, f: &mut Visit<'_, 'a>) {
    match s {
        ClaimSet::Group(i) => name(i, f),
        ClaimSet::Effects { class: _, name: n, span: _ } => f(Spelled::Name(n)),
    }
}

fn topic_ref<'a>(t: &'a TopicRef, f: &mut Visit<'_, 'a>) {
    let TopicRef { segments, span: _ } = t;
    names(segments, f);
}

fn locus<'a>(l: &'a LocusDecl, f: &mut Visit<'_, 'a>) {
    let LocusDecl {
        name: n,
        imported: _,
        display,
        is_main: _,
        export: _,
        generics: gs,
        annotations: _,
        serves,
        form,
        locality: _,
        bounded: _,
        phase_effects,
        depends,
        supervised: _,
        sealed: _,
        members,
        span: _,
        id: _,
    } = l;
    name(n, f);
    opt_text(display, f);
    generics(gs, f);
    names(serves, f);
    if let Some(FormAnnotation { name: n, args, span: _ }) = form {
        name(n, f);
        for a in args {
            let FormArg { name: n, value, span: _ } = a;
            name(n, f);
            expr(value, f);
        }
    }
    if let Some(PhaseEffects { phases, span: _ }) = phase_effects {
        for (phase, _) in phases {
            text(phase, f);
        }
    }
    if let Some(DependsSet { subjects, span: _ }) = depends {
        for s in subjects {
            text(s, f);
        }
    }
    for m in members {
        locus_member(m, f);
    }
}

fn locus_member<'a>(m: &'a LocusMember, f: &mut Visit<'_, 'a>) {
    match m {
        LocusMember::Params(pb) => params_block(pb, f),
        LocusMember::Contract(cb) => {
            let ContractBlock { kind, span: _ } = cb;
            match kind {
                ContractKind::Inferred => {}
                ContractKind::Members(ms) => {
                    for cm in ms {
                        let ContractMember { direction: _, name: cn, ty: t, gated, span: _ } = cm;
                        match cn {
                            ContractName::Named(i) => name(i, f),
                            ContractName::Inferred => {}
                        }
                        opt_ty(t, f);
                        opt_name(gated, f);
                    }
                }
            }
        }
        LocusMember::Bus(bb) => bus(bb, f),
        LocusMember::Lifecycle(ld) => {
            let LifecycleDecl { kind: _, params: ps, ret, unbounded: _, body, span: _, id: _, synthesized: _ } = ld;
            params(ps, f);
            opt_ty(ret, f);
            block(body, f);
        }
        LocusMember::Mode(md) => {
            let ModeDecl { kind: _, params: ps, ret, body, span: _, id: _ } = md;
            params(ps, f);
            opt_ty(ret, f);
            block(body, f);
        }
        LocusMember::Failure(fd) => {
            let FailureDecl { params: ps, body, span: _, id: _ } = fd;
            params(ps, f);
            block(body, f);
        }
        LocusMember::Closure(cd) => {
            let ClosureDecl { name: n, assertion, clauses, span: _, id: _ } = cd;
            name(n, f);
            if let Some(ClosureAssertion { left, right, tolerance, span: _ }) = assertion {
                expr(left, f);
                expr(right, f);
                expr(tolerance, f);
            }
            for c in clauses {
                match c {
                    ClosureClause::Epoch(spec) => match spec {
                        EpochSpec::Duration(e) => expr(e, f),
                        EpochSpec::Tick
                        | EpochSpec::Birth
                        | EpochSpec::Dissolve
                        | EpochSpec::Explicit
                        | EpochSpec::Inline => {}
                    },
                    ClosureClause::PersistsThrough(is)
                    | ClosureClause::ResetsOn(is)
                    | ClosureClause::ResetsPerEpoch(is)
                    | ClosureClause::Captures(is) => names(is, f),
                }
            }
        }
        LocusMember::Fn(fd) => fn_decl(fd, f),
        LocusMember::Const(c) => const_decl(c, f),
        LocusMember::Type(t) => type_decl(t, f),
        LocusMember::Capacity(cb) => {
            let CapacityBlock { slots, span: _ } = cb;
            for s in slots {
                let CapacitySlot { name: n, kind: _, elem_ty, as_parent_for, indexed_by, span: _ } = s;
                name(n, f);
                ty(elem_ty, f);
                opt_name(as_parent_for, f);
                opt_name(indexed_by, f);
            }
        }
        LocusMember::Bindings(bb) => {
            let BindingsBlock { entries, api, span: _ } = bb;
            for e in entries {
                let BindingEntry { topic, transport, constraints: _, codec, span: _, id: _ } = e;
                name(topic, f);
                match transport {
                    TransportSpec::Unix { path, role: _, span: _ } => text(path, f),
                    TransportSpec::Adapter { locus: l, inits, span: _ } => {
                        name(l, f);
                        struct_inits(inits, f);
                    }
                    TransportSpec::ShmRing {
                        name: n,
                        slot_count: _,
                        overflow: _,
                        layout,
                        buffer_size: _,
                        span: _,
                    } => {
                        f(Spelled::Name(n));
                        opt_name(layout, f);
                    }
                }
                if let Some(CodecSpec { locus: l, inits, span: _ }) = codec {
                    name(l, f);
                    struct_inits(inits, f);
                }
            }
            if let Some(a) = api {
                let ApiBinding {
                    transport,
                    roles,
                    bound: _,
                    on_full: _,
                    watch_bound: _,
                    on_watch_full: _,
                    on_unauthorized: _,
                    serve,
                    http,
                    span: _,
                } = a;
                match transport {
                    ApiTransport::Unix { path, span: _ } => expr(path, f),
                }
                if let Some(ApiRoles { expr: e, span: _ }) = roles {
                    expr(e, f);
                }
                names(serve, f);
                if let Some(ApiHttp { host, port, principals, span: _ }) = http {
                    expr(host, f);
                    expr(port, f);
                    opt_expr(principals, f);
                }
            }
        }
        LocusMember::Placement(pb) => {
            let PlacementBlock { entries, span: _ } = pb;
            for e in entries {
                let PlacementEntry { field, spec, constraints: _, span: _, id: _ } = e;
                name(field, f);
                match spec {
                    PlacementSpec::Cooperative { pool, affinity } => {
                        opt_name(pool, f);
                        pin_affinity(affinity, f);
                    }
                    PlacementSpec::Pinned { affinity, replicas: _ } => pin_affinity(affinity, f),
                }
            }
        }
        LocusMember::Topology(tb) => {
            let TopologyBlock { reserved: _, nodes, span: _ } = tb;
            for n in nodes {
                let TopologyNode { id: _, id_span: _, domains, span: _ } = n;
                for d in domains {
                    let L3Domain { name: dn, cores: _, span: _ } = d;
                    name(dn, f);
                }
            }
        }
        LocusMember::Claims(cb) => claims_block(cb, f),
        LocusMember::BirthCheck(bc) => {
            let BirthCheckDecl { cond, closure_name, payload, span: _ } = bc;
            expr(cond, f);
            name(closure_name, f);
            opt_expr(payload, f);
        }
    }
}

fn pin_affinity<'a>(a: &'a PinAffinity, f: &mut Visit<'_, 'a>) {
    match a {
        PinAffinity::L3(i) => name(i, f),
        PinAffinity::Any | PinAffinity::Cores(_) | PinAffinity::Node(_) => {}
    }
}

fn params_block<'a>(pb: &'a ParamsBlock, f: &mut Visit<'_, 'a>) {
    let ParamsBlock { params: ps, span: _ } = pb;
    for p in ps {
        let ParamDecl { name: n, ty: t, init, span: _, id: _ } = p;
        name(n, f);
        opt_ty(t, f);
        match init {
            ParamInit::Value(e) => expr(e, f),
            ParamInit::Inferred => {}
        }
    }
}

fn bus<'a>(bb: &'a BusBlock, f: &mut Visit<'_, 'a>) {
    let BusBlock { members, span: _ } = bb;
    for m in members {
        match m {
            BusMember::Subscribe { subject, handler, ty: t, key_filter, bound: _, span: _, id: _ } => {
                bus_subject(subject, f);
                name(handler, f);
                opt_ty(t, f);
                if let Some(kf) = key_filter {
                    match kf {
                        KeyFilter::Specific { expr: e, span: _ } => expr(e, f),
                        KeyFilter::Unmatched { .. } | KeyFilter::Replica { .. } => {}
                    }
                }
            }
            BusMember::Publish { subject, ty: t, alias, gated, span: _, id: _ } => {
                bus_subject(subject, f);
                opt_ty(t, f);
                opt_name(alias, f);
                opt_name(gated, f);
            }
        }
    }
}

fn bus_subject<'a>(s: &'a BusSubject, f: &mut Visit<'_, 'a>) {
    match s {
        BusSubject::Literal { subject, span: _ } => text(subject, f),
        BusSubject::Topic(i) => name(i, f),
        BusSubject::QualifiedTopic(q) => qualified(q, f),
    }
}

fn fn_decl<'a>(fd: &'a FnDecl, f: &mut Visit<'_, 'a>) {
    let FnDecl {
        name: n,
        generics: gs,
        params: ps,
        ret,
        fallible,
        ffi,
        export: _,
        unbounded: _,
        budget: _,
        hot: _,
        effects,
        quantities: _,
        gated,
        decorators,
        body,
        span: _,
        id: _,
    } = fd;
    name(n, f);
    generics(gs, f);
    params(ps, f);
    opt_ty(ret, f);
    opt_ty(fallible, f);
    if let Some(FfiAnnotation { abi, span: _ }) = ffi {
        text(abi, f);
    }
    for a in effects {
        match a {
            EffectAssert::PublishSet(subjects) => {
                for s in subjects {
                    text(s, f);
                }
            }
            EffectAssert::Forbid(_)
            | EffectAssert::Causes(_)
            | EffectAssert::Carries(_)
            | EffectAssert::Only(_)
            | EffectAssert::NoPanic => {}
        }
    }
    opt_name(gated, f);
    for d in decorators {
        let FnDecorator { name: dn, span: _ } = d;
        f(Spelled::Name(dn));
    }
    block(body, f);
}

fn type_decl<'a>(t: &'a TypeDecl, f: &mut Visit<'_, 'a>) {
    let TypeDecl { name: n, display, generics: gs, body, span: _, id: _, synthetic: _ } = t;
    name(n, f);
    opt_text(display, f);
    generics(gs, f);
    match body {
        TypeDeclBody::Alias(a) => ty(a, f),
        TypeDeclBody::Struct(fields) => {
            for sf in fields {
                let StructField { name: n, ty: t, default, tag, span: _ } = sf;
                name(n, f);
                ty(t, f);
                opt_expr(default, f);
                opt_text(tag, f);
            }
        }
        TypeDeclBody::Enum(variants) => {
            for v in variants {
                let EnumVariant { name: n, fields, span: _ } = v;
                name(n, f);
                for t in fields {
                    ty(t, f);
                }
            }
        }
        TypeDeclBody::Scalar(s) => {
            let ScalarDecl { kind: _, base, denom, clauses } = s;
            ty(base, f);
            if let Some(Denomination { multiple: _, unit, span: _ }) = denom {
                name(unit, f);
            }
            for c in clauses {
                match c {
                    ScalarClause::Range { lo, hi, inclusive: _, span: _ } => {
                        expr(lo, f);
                        expr(hi, f);
                    }
                    ScalarClause::Round { policy, span: _ } => name(policy, f),
                    ScalarClause::Origin { value: _, unit, span: _ } => name(unit, f),
                }
            }
        }
    }
}

fn const_decl<'a>(c: &'a ConstDecl, f: &mut Visit<'_, 'a>) {
    let ConstDecl { name: n, ty: t, value, span: _, id: _ } = c;
    name(n, f);
    ty(t, f);
    expr(value, f);
}

fn generics<'a>(gs: &'a [GenericParam], f: &mut Visit<'_, 'a>) {
    for g in gs {
        let GenericParam { name: n, bound, span: _ } = g;
        name(n, f);
        opt_ty(bound, f);
    }
}

fn params<'a>(ps: &'a [Param], f: &mut Visit<'_, 'a>) {
    for p in ps {
        let Param { name: n, ty: t, default, secret: _, span: _ } = p;
        name(n, f);
        ty(t, f);
        opt_expr(default, f);
    }
}

fn opt_ty<'a>(t: &'a Option<TypeExpr>, f: &mut Visit<'_, 'a>) {
    if let Some(t) = t {
        ty(t, f);
    }
}

fn ty<'a>(t: &'a TypeExpr, f: &mut Visit<'_, 'a>) {
    match t {
        TypeExpr::Primitive(..) => {}
        TypeExpr::Named { path, generic_args, span: _ } => {
            qualified(path, f);
            for a in generic_args {
                ty(a, f);
            }
        }
        TypeExpr::Projection { class: _, inner, span: _ } => ty(inner, f),
        TypeExpr::Array { elem, size, span: _ } => {
            ty(elem, f);
            opt_expr(size, f);
        }
        TypeExpr::Bounded { elem, cap: _, span: _ } => ty(elem, f),
        TypeExpr::Tuple(elems, _) => {
            for e in elems {
                ty(e, f);
            }
        }
        TypeExpr::Function { params: ps, ret, span: _ } => {
            for p in ps {
                ty(p, f);
            }
            if let Some(r) = ret {
                ty(r, f);
            }
        }
        TypeExpr::Perspective { name: n, span: _ } => name(n, f),
    }
}

fn block<'a>(b: &'a Block, f: &mut Visit<'_, 'a>) {
    let Block { stmts, tail, span: _ } = b;
    for s in stmts {
        stmt(s, f);
    }
    if let Some(t) = tail {
        expr(t, f);
    }
}

fn stmt<'a>(s: &'a Stmt, f: &mut Visit<'_, 'a>) {
    match s {
        Stmt::Let { is_mut: _, name: n, ty: t, value, span: _, id: _ } => {
            name(n, f);
            opt_ty(t, f);
            expr(value, f);
        }
        Stmt::LetTuple { is_mut: _, names: ns, ty: t, value, span: _, id: _ } => {
            names(ns, f);
            opt_ty(t, f);
            expr(value, f);
        }
        Stmt::Assign { target, op: _, value, span: _, id: _ } => {
            let LValue { head, tail, span: _ } = target;
            name(head, f);
            for seg in tail {
                match seg {
                    LValueSeg::Field(i) => name(i, f),
                    LValueSeg::Index(e) => expr(e, f),
                }
            }
            expr(value, f);
        }
        Stmt::If(i) => if_stmt(i, f),
        Stmt::Match(ms) => match_stmt(ms, f),
        Stmt::For { name: n, iter, body, span: _, id: _ } => {
            name(n, f);
            expr(iter, f);
            block(body, f);
        }
        Stmt::While { cond, body, span: _ } => {
            expr(cond, f);
            block(body, f);
        }
        Stmt::Return(e, _) => opt_expr(e, f),
        Stmt::Fail { value, span: _ } => expr(value, f),
        Stmt::Reperspective { field, impl_name, span: _ } => {
            name(field, f);
            name(impl_name, f);
        }
        Stmt::Block(b) => block(b, f),
        Stmt::Recovery { op: _, args, modifier, span: _ } => {
            for a in args {
                expr(a, f);
            }
            if let Some(m) = modifier {
                match m {
                    RecoveryModifier::For(e) | RecoveryModifier::Until(e) => expr(e, f),
                }
            }
        }
        Stmt::Violate { name: n, payload, span: _ } => {
            name(n, f);
            opt_expr(payload, f);
        }
        Stmt::Send { subject, value, or_disposition, span: _, id: _ } => {
            expr(subject, f);
            expr(value, f);
            if let Some(d) = or_disposition {
                disposition(d, f);
            }
        }
        Stmt::ShmWrite { topic, max, binding, body, span: _ } => {
            name(topic, f);
            expr(max, f);
            name(binding, f);
            block(body, f);
        }
        Stmt::Expr(e) => expr(e, f),
        Stmt::Break(_) | Stmt::Continue(_) | Stmt::Yield(_) | Stmt::Terminate(_) => {}
    }
}

fn if_stmt<'a>(i: &'a IfStmt, f: &mut Visit<'_, 'a>) {
    let IfStmt { cond, then_block, else_block, span: _ } = i;
    expr(cond, f);
    block(then_block, f);
    if let Some(eb) = else_block {
        match &**eb {
            ElseBranch::Else(b) => block(b, f),
            ElseBranch::ElseIf(i) => if_stmt(i, f),
        }
    }
}

fn match_stmt<'a>(ms: &'a MatchStmt, f: &mut Visit<'_, 'a>) {
    let MatchStmt { scrutinee, arms, span: _ } = ms;
    expr(scrutinee, f);
    for arm in arms {
        let MatchArm { pattern: p, guard, body, span: _ } = arm;
        pattern(p, f);
        opt_expr(guard, f);
        match body {
            MatchArmBody::Expr(e) => expr(e, f),
            MatchArmBody::Block(b) => block(b, f),
        }
    }
}

fn pattern<'a>(p: &'a Pattern, f: &mut Visit<'_, 'a>) {
    match p {
        Pattern::Literal(l, _) => literal(l, f),
        Pattern::Wildcard(_) => {}
        Pattern::Binding(i) => name(i, f),
        Pattern::Constructor { path, args, span: _ } => {
            qualified(path, f);
            for a in args {
                pattern(a, f);
            }
        }
        Pattern::Tuple(args, _) => {
            for a in args {
                pattern(a, f);
            }
        }
    }
}

fn literal<'a>(l: &'a Literal, f: &mut Visit<'_, 'a>) {
    match l {
        Literal::Decimal(s) | Literal::String(s) | Literal::Time(s) => text(s, f),
        // The unit a quantity literal names.
        Literal::Quantity { value: _, unit } => f(Spelled::Name(unit)),
        Literal::Int(_)
        | Literal::Float(_)
        | Literal::Bool(_)
        | Literal::Nil
        | Literal::Duration(_)
        | Literal::Bytes(_) => {}
    }
}

fn disposition<'a>(d: &'a OrDisposition, f: &mut Visit<'_, 'a>) {
    match d {
        OrDisposition::Substitute(e) | OrDisposition::Fail(e, _) => expr(e, f),
        OrDisposition::Raise(_) | OrDisposition::Discard(_) | OrDisposition::Wait(_) => {}
    }
}

fn struct_inits<'a>(inits: &'a [StructInit], f: &mut Visit<'_, 'a>) {
    for i in inits {
        let StructInit { name: n, value, span: _ } = i;
        name(n, f);
        expr(value, f);
    }
}

fn opt_expr<'a>(e: &'a Option<Expr>, f: &mut Visit<'_, 'a>) {
    if let Some(e) = e {
        expr(e, f);
    }
}

fn expr<'a>(e: &'a Expr, f: &mut Visit<'_, 'a>) {
    match e {
        Expr::Literal(l, _) => literal(l, f),
        Expr::Ident(i) => name(i, f),
        Expr::Path(q) => qualified(q, f),
        Expr::KwSelf(_) => {}
        Expr::Binary { op: _, left, right, span: _ } => {
            expr(left, f);
            expr(right, f);
        }
        Expr::Unary { op: _, operand, span: _ } => expr(operand, f),
        Expr::Call { callee, args, span: _, id: _ } => {
            expr(callee, f);
            for a in args {
                expr(a, f);
            }
        }
        Expr::Field { receiver, name: n, span: _ } | Expr::Path2 { receiver, name: n, span: _ } => {
            expr(receiver, f);
            name(n, f);
        }
        Expr::Index { receiver, index, span: _ } => {
            expr(receiver, f);
            expr(index, f);
        }
        Expr::Tuple(parts, _) | Expr::Array(parts, _) => {
            for p in parts {
                expr(p, f);
            }
        }
        Expr::Struct { path, inits, span: _, id: _ } => {
            qualified(path, f);
            struct_inits(inits, f);
        }
        Expr::Block(b) => block(b, f),
        Expr::If(i) => if_stmt(i, f),
        Expr::Match(ms) => match_stmt(ms, f),
        Expr::Sum(inner, _) | Expr::Prod(inner, _) => expr(inner, f),
        Expr::Approx { left, right, tolerance, span: _ } => {
            expr(left, f);
            expr(right, f);
            expr(tolerance, f);
        }
        Expr::Range { lo, hi, inclusive: _, span: _ } => {
            expr(lo, f);
            expr(hi, f);
        }
        Expr::ArrayRepeat { val, count: _, span: _ } => expr(val, f),
        Expr::Or { inner, disposition: d, span: _ } => {
            expr(inner, f);
            disposition(d, f);
        }
    }
}

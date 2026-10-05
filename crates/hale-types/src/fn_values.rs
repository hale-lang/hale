//! The names a program spells where a value is read (F.40 phase 3, E5).
//!
//! A function value only arises from a function's name read as a value:
//! the language has no lambda, a method is not a value codegen lowers,
//! and the FFI refuses function-pointer types. So the functions an
//! indirect call can reach are the functions whose name some expression
//! reads as a value (their address is taken), which the allocation
//! summary resolves the way a `let` binding of the name resolves.
//!
//! The walk is syntactic and covers every expression a program holds
//! (bodies, params initializers, `on_failure` handlers, constants, field
//! defaults, form arguments, bindings), the coverage of the identity
//! walk in `hale_syntax::sites`. Its matches name every variant, so a
//! new variant is a compile error here rather than an expression the
//! walk silently skips. A callee written as a name or a path is a
//! direct call, not a value; every other spelled name is one, unless a
//! binding in scope (a parameter, a `let`, a loop or pattern binder)
//! spells it, as a local shadows a function in value position.

use std::collections::BTreeSet;

use hale_syntax::ast::*;

/// A name read as a value.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ValueName {
    /// `name`, with no binding in scope of that name.
    Ident(String),
    /// `a::b::name`, joined.
    Path(String),
    /// `receiver.name` or `receiver::name`: a member read as a value.
    Member(String),
}

/// Every name the items read as a value, in no particular order.
pub(crate) fn value_names(items: &[TopDecl], out: &mut Vec<ValueName>) {
    let mut w = Walk { out, scopes: Vec::new() };
    for item in items {
        w.top_decl(item);
    }
}

struct Walk<'o> {
    out: &'o mut Vec<ValueName>,
    /// The bindings in scope, innermost last.
    scopes: Vec<BTreeSet<String>>,
}

impl Walk<'_> {
    fn bound(&self, name: &str) -> bool {
        self.scopes.iter().any(|s| s.contains(name))
    }

    fn bind(&mut self, name: &str) {
        if let Some(s) = self.scopes.last_mut() {
            s.insert(name.to_string());
        }
    }

    /// Walk `f` inside a scope holding `names`.
    fn scoped<'n>(&mut self, names: impl IntoIterator<Item = &'n str>, f: impl FnOnce(&mut Self)) {
        self.scopes.push(names.into_iter().map(str::to_string).collect());
        f(self);
        self.scopes.pop();
    }

    fn top_decl(&mut self, d: &TopDecl) {
        match d {
            TopDecl::Locus(l) => {
                self.generics(&l.generics);
                if let Some(form) = &l.form {
                    for arg in &form.args {
                        self.expr(&arg.value);
                    }
                }
                for member in &l.members {
                    self.locus_member(member);
                }
            }
            TopDecl::Perspective(p) => {
                self.generics(&p.generics);
                for member in &p.members {
                    match member {
                        PerspectiveMember::Params(pb) => self.params_block(pb),
                        PerspectiveMember::StableWhen(b) => self.block(b),
                        PerspectiveMember::SerializeAs(t) => self.ty(t),
                        PerspectiveMember::Fn(fd) => self.fn_decl(fd),
                        PerspectiveMember::Bus(bb) => self.bus(bb),
                    }
                }
            }
            TopDecl::Type(t) => self.type_decl(t),
            TopDecl::Const(c) => self.const_decl(c),
            TopDecl::Fn(fd) => self.fn_decl(fd),
            TopDecl::Module(md) => {
                for item in &md.items {
                    self.top_decl(item);
                }
            }
            TopDecl::Interface(i) => {
                for sig in &i.methods {
                    self.params(&sig.params);
                    self.opt_ty(&sig.ret);
                    self.opt_ty(&sig.fallible);
                }
            }
            TopDecl::Topic(t) => self.ty(&t.payload),
            TopDecl::Group(_)
            | TopDecl::RingLayout(_)
            | TopDecl::Target(_)
            | TopDecl::Role(_)
            | TopDecl::Claims(_)
            | TopDecl::Constitution(_)
            | TopDecl::Unit(_) => {}
        }
    }

    fn locus_member(&mut self, member: &LocusMember) {
        match member {
            LocusMember::Params(pb) => self.params_block(pb),
            LocusMember::Contract(cb) => match &cb.kind {
                ContractKind::Inferred => {}
                ContractKind::Members(ms) => {
                    for cm in ms {
                        self.opt_ty(&cm.ty);
                    }
                }
            },
            LocusMember::Bus(bb) => self.bus(bb),
            LocusMember::Lifecycle(ld) => self.body(&ld.params, |w| {
                w.opt_ty(&ld.ret);
                w.block(&ld.body);
            }),
            LocusMember::Mode(md) => self.body(&md.params, |w| {
                w.opt_ty(&md.ret);
                w.block(&md.body);
            }),
            LocusMember::Failure(fd) => self.body(&fd.params, |w| w.block(&fd.body)),
            LocusMember::Closure(cd) => {
                if let Some(a) = &cd.assertion {
                    self.expr(&a.left);
                    self.expr(&a.right);
                    self.expr(&a.tolerance);
                }
                for clause in &cd.clauses {
                    match clause {
                        ClosureClause::Epoch(EpochSpec::Duration(e)) => self.expr(e),
                        ClosureClause::Epoch(
                            EpochSpec::Tick
                            | EpochSpec::Birth
                            | EpochSpec::Dissolve
                            | EpochSpec::Explicit
                            | EpochSpec::Inline,
                        )
                        | ClosureClause::PersistsThrough(_)
                        | ClosureClause::ResetsOn(_)
                        | ClosureClause::ResetsPerEpoch(_)
                        | ClosureClause::Captures(_) => {}
                    }
                }
            }
            LocusMember::Fn(fd) => self.fn_decl(fd),
            LocusMember::Const(c) => self.const_decl(c),
            LocusMember::Type(t) => self.type_decl(t),
            LocusMember::Capacity(cb) => {
                for slot in &cb.slots {
                    self.ty(&slot.elem_ty);
                }
            }
            LocusMember::Bindings(bb) => {
                for entry in &bb.entries {
                    match &entry.transport {
                        TransportSpec::Adapter { inits, .. } => self.struct_inits(inits),
                        TransportSpec::Unix { .. } | TransportSpec::ShmRing { .. } => {}
                    }
                    if let Some(codec) = &entry.codec {
                        self.struct_inits(&codec.inits);
                    }
                }
                if let Some(api) = &bb.api {
                    match &api.transport {
                        ApiTransport::Unix { path, .. } => self.expr(path),
                    }
                    if let Some(roles) = &api.roles {
                        self.expr(&roles.expr);
                    }
                    if let Some(http) = &api.http {
                        self.expr(&http.host);
                        self.expr(&http.port);
                        self.opt_expr(&http.principals);
                    }
                }
            }
            LocusMember::BirthCheck(bc) => {
                self.expr(&bc.cond);
                self.opt_expr(&bc.payload);
            }
            LocusMember::Placement(_) | LocusMember::Topology(_) | LocusMember::Claims(_) => {}
        }
    }

    fn params_block(&mut self, pb: &ParamsBlock) {
        for p in &pb.params {
            self.opt_ty(&p.ty);
            match &p.init {
                ParamInit::Value(e) => self.expr(e),
                ParamInit::Inferred => {}
            }
        }
    }

    fn bus(&mut self, bb: &BusBlock) {
        for member in &bb.members {
            match member {
                BusMember::Subscribe { ty: t, key_filter, .. } => {
                    self.opt_ty(t);
                    match key_filter {
                        Some(KeyFilter::Specific { expr: e, .. }) => self.expr(e),
                        Some(KeyFilter::Unmatched { .. } | KeyFilter::Replica { .. }) | None => {}
                    }
                }
                BusMember::Publish { ty: t, .. } => self.opt_ty(t),
            }
        }
    }

    /// A body: its parameters' types and defaults, then `f` with the
    /// parameters in scope.
    fn body(&mut self, ps: &[Param], f: impl FnOnce(&mut Self)) {
        self.params(ps);
        self.scoped(ps.iter().map(|p| p.name.name.as_str()), f);
    }

    fn fn_decl(&mut self, fd: &FnDecl) {
        self.generics(&fd.generics);
        self.body(&fd.params, |w| {
            w.opt_ty(&fd.ret);
            w.opt_ty(&fd.fallible);
            w.block(&fd.body);
        });
    }

    fn type_decl(&mut self, t: &TypeDecl) {
        self.generics(&t.generics);
        match &t.body {
            TypeDeclBody::Alias(a) => self.ty(a),
            TypeDeclBody::Struct(fields) => {
                for field in fields {
                    self.ty(&field.ty);
                    self.opt_expr(&field.default);
                }
            }
            TypeDeclBody::Enum(variants) => {
                for v in variants {
                    for t in &v.fields {
                        self.ty(t);
                    }
                }
            }
            TypeDeclBody::Scalar(s) => {
                self.ty(&s.base);
                for c in &s.clauses {
                    if let ScalarClause::Range { lo, hi, .. } = c {
                        self.expr(lo);
                        self.expr(hi);
                    }
                }
            }
        }
    }

    fn const_decl(&mut self, c: &ConstDecl) {
        self.ty(&c.ty);
        self.expr(&c.value);
    }

    fn generics(&mut self, gs: &[GenericParam]) {
        for g in gs {
            self.opt_ty(&g.bound);
        }
    }

    fn params(&mut self, ps: &[Param]) {
        for p in ps {
            self.ty(&p.ty);
            self.opt_expr(&p.default);
        }
    }

    fn opt_ty(&mut self, t: &Option<TypeExpr>) {
        if let Some(t) = t {
            self.ty(t);
        }
    }

    /// An array type's size is an expression.
    fn ty(&mut self, t: &TypeExpr) {
        match t {
            TypeExpr::Named { generic_args, .. } => {
                for a in generic_args {
                    self.ty(a);
                }
            }
            TypeExpr::Projection { inner, .. } => self.ty(inner),
            TypeExpr::Array { elem, size, .. } => {
                self.ty(elem);
                self.opt_expr(size);
            }
            TypeExpr::Bounded { elem, .. } => self.ty(elem),
            TypeExpr::Tuple(elems, _) => {
                for e in elems {
                    self.ty(e);
                }
            }
            TypeExpr::Function { params, ret, .. } => {
                for p in params {
                    self.ty(p);
                }
                if let Some(r) = ret {
                    self.ty(r);
                }
            }
            TypeExpr::Primitive(..) | TypeExpr::Perspective { .. } => {}
        }
    }

    fn block(&mut self, b: &Block) {
        self.scoped([], |w| {
            for s in &b.stmts {
                w.stmt(s);
            }
            if let Some(tail) = &b.tail {
                w.expr(tail);
            }
        });
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Let { name, ty: t, value, .. } => {
                self.opt_ty(t);
                self.expr(value);
                self.bind(&name.name);
            }
            Stmt::LetTuple { names, ty: t, value, .. } => {
                self.opt_ty(t);
                self.expr(value);
                for n in names {
                    self.bind(&n.name);
                }
            }
            Stmt::Assign { target, value, .. } => {
                for seg in &target.tail {
                    match seg {
                        LValueSeg::Index(e) => self.expr(e),
                        LValueSeg::Field(_) => {}
                    }
                }
                self.expr(value);
            }
            Stmt::If(i) => self.if_stmt(i),
            Stmt::Match(ms) => self.match_stmt(ms),
            Stmt::For { name, iter, body, .. } => {
                self.expr(iter);
                self.scoped([name.name.as_str()], |w| w.block(body));
            }
            Stmt::While { cond, body, .. } => {
                self.expr(cond);
                self.block(body);
            }
            Stmt::Return(e, _) => self.opt_expr(e),
            Stmt::Fail { value, .. } => self.expr(value),
            Stmt::Block(b) => self.block(b),
            Stmt::Recovery { args, modifier, .. } => {
                for a in args {
                    self.expr(a);
                }
                match modifier {
                    Some(RecoveryModifier::For(e) | RecoveryModifier::Until(e)) => self.expr(e),
                    None => {}
                }
            }
            Stmt::Violate { payload, .. } => self.opt_expr(payload),
            Stmt::Send { subject, value, or_disposition, .. } => {
                self.expr(subject);
                self.expr(value);
                if let Some(d) = or_disposition {
                    self.disposition(d);
                }
            }
            Stmt::ShmWrite { max, binding, body, .. } => {
                self.expr(max);
                self.scoped([binding.name.as_str()], |w| w.block(body));
            }
            Stmt::Expr(e) => self.expr(e),
            Stmt::Break(_)
            | Stmt::Continue(_)
            | Stmt::Yield(_)
            | Stmt::Terminate(_)
            | Stmt::Reperspective { .. } => {}
        }
    }

    fn if_stmt(&mut self, i: &IfStmt) {
        self.expr(&i.cond);
        self.block(&i.then_block);
        if let Some(eb) = &i.else_block {
            match &**eb {
                ElseBranch::Else(b) => self.block(b),
                ElseBranch::ElseIf(i) => self.if_stmt(i),
            }
        }
    }

    fn match_stmt(&mut self, ms: &MatchStmt) {
        self.expr(&ms.scrutinee);
        for arm in &ms.arms {
            let mut names = Vec::new();
            pattern_names(&arm.pattern, &mut names);
            self.scoped(names.iter().map(String::as_str), |w| {
                w.opt_expr(&arm.guard);
                match &arm.body {
                    MatchArmBody::Expr(e) => w.expr(e),
                    MatchArmBody::Block(b) => w.block(b),
                }
            });
        }
    }

    fn disposition(&mut self, d: &OrDisposition) {
        match d {
            OrDisposition::Substitute(e) | OrDisposition::Fail(e, _) => self.expr(e),
            OrDisposition::Raise(_) | OrDisposition::Discard(_) | OrDisposition::Wait(_) => {}
        }
    }

    fn struct_inits(&mut self, inits: &[StructInit]) {
        for init in inits {
            self.expr(&init.value);
        }
    }

    fn opt_expr(&mut self, e: &Option<Expr>) {
        if let Some(e) = e {
            self.expr(e);
        }
    }

    fn expr(&mut self, e: &Expr) {
        match e {
            Expr::Call { callee, args, .. } => {
                match callee.as_ref() {
                    // A direct call names its callee; it reads no value.
                    Expr::Ident(_) | Expr::Path(_) => {}
                    // A method call: the receiver is read, the method is not.
                    Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. } => self.expr(receiver),
                    other => self.expr(other),
                }
                for a in args {
                    self.expr(a);
                }
            }
            Expr::Struct { inits, .. } => self.struct_inits(inits),
            Expr::Binary { left, right, .. } => {
                self.expr(left);
                self.expr(right);
            }
            Expr::Unary { operand, .. } => self.expr(operand),
            Expr::Field { receiver, name, .. } | Expr::Path2 { receiver, name, .. } => {
                self.out.push(ValueName::Member(name.name.clone()));
                self.expr(receiver);
            }
            Expr::Index { receiver, index, .. } => {
                self.expr(receiver);
                self.expr(index);
            }
            Expr::Tuple(parts, _) | Expr::Array(parts, _) => {
                for p in parts {
                    self.expr(p);
                }
            }
            Expr::Block(b) => self.block(b),
            Expr::If(i) => self.if_stmt(i),
            Expr::Match(ms) => self.match_stmt(ms),
            Expr::Sum(inner, _) | Expr::Prod(inner, _) => self.expr(inner),
            Expr::Approx { left, right, tolerance, .. } => {
                self.expr(left);
                self.expr(right);
                self.expr(tolerance);
            }
            Expr::Range { lo, hi, .. } => {
                self.expr(lo);
                self.expr(hi);
            }
            Expr::ArrayRepeat { val, .. } => self.expr(val),
            Expr::Or { inner, disposition: d, .. } => {
                self.expr(inner);
                self.disposition(d);
            }
            Expr::Ident(i) => {
                if !self.bound(&i.name) {
                    self.out.push(ValueName::Ident(i.name.clone()));
                }
            }
            Expr::Path(qp) => self.out.push(ValueName::Path(
                qp.segments.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join("::"),
            )),
            Expr::Literal(..) | Expr::KwSelf(_) => {}
        }
    }
}

fn pattern_names(p: &Pattern, out: &mut Vec<String>) {
    match p {
        Pattern::Binding(i) => out.push(i.name.clone()),
        Pattern::Constructor { args, .. } | Pattern::Tuple(args, _) => {
            for a in args {
                pattern_names(a, out);
            }
        }
        Pattern::Literal(..) | Pattern::Wildcard(_) => {}
    }
}

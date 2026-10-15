//! A fn value bound in scope shadows a fn of the same name in a call.
//!
//! `fn apply(f: fn(Int) -> Int, x: Int) -> Int { return f(x); }` beside a
//! top-level `fn f` calls the parameter: the binding in scope, as the code
//! reads and as the name already means everywhere else it is read (a value
//! position names the binding, `fn_values`). A call spells its callee as
//! a bare name, though, and every pass that follows calls resolves a bare
//! callee by name, among the program's fns first; lowering did too. So the
//! one place the rule lives is here, in the desugar sequence before the
//! check: a call whose bare callee names a binding in scope declared as a
//! fn value (a parameter or a `let` written with a `fn(..)` type, a `let`
//! or a tuple `let`'s part of a fn's name or of another such binding, a
//! `for` over an array literal of fns' names) and also one of the
//! program's fns or
//! one of the builtins the compiler answers by name (`len`, `min`, a
//! `bounded` intrinsic, a printer: `check::BARE_BUILTIN_CALLEES`) is
//! rewritten to call the binding as a value, `{ f }(x)`. A callee that is
//! not a bare name is a call through a value to every pass already (the
//! checker types the block as the binding's fn type, lowering calls the
//! value it evaluates to, the summaries treat it as indirect), and the
//! name inside it is a read of the binding. A callee no fn or builtin
//! shares its name with is left as written: it already calls the binding.
//!
//! A binding whose declaration says nothing of its type (a pattern's) is
//! not one: the checker refuses a call through it when its type is, or may
//! be, a fn, and leaves the fn or builtin the callee when it is known to
//! be no fn. A call the compiler wrote into the user's scope (an
//! f-string's `to_string` and `__fmt`, a `from_json`'s parser) carries its
//! callee at the call's own span, which no written call does, and is the
//! compiler's whatever binding of the name is in scope, here and in the
//! check.
//!
//! The walk reaches every expression the program holds, every declaration
//! and member kind named (a new one is a compile error here, not a root
//! the pass skips): bodies with their parameters in scope, and the roots
//! outside them (a param's or a field's default, a const, a closure's
//! assertion, an array type's size) with none, a block in one binding its
//! own. The rewritten callee keeps its span.

use std::collections::{BTreeMap, BTreeSet};

use hale_syntax::ast::*;

/// Rewrite every call in `programs` whose bare callee a binding in scope
/// shadows a fn or a builtin of. `context` names fns too and is not
/// rewritten.
pub(crate) fn rewrite_shadowed_callees(programs: &mut [&mut Program], context: &[&Program]) {
    let mut fns: BTreeSet<String> = crate::check::BARE_BUILTIN_CALLEES
        .iter()
        .filter(|n| **n != hale_syntax::parser::FMT_BUILTIN)
        .map(|n| n.to_string())
        .collect();
    let mut program_fns = BTreeSet::new();
    for p in programs.iter().map(|p| &**p).chain(context.iter().copied()) {
        for item in flat_decls(&p.items) {
            if let TopDecl::Fn(f) = item {
                program_fns.insert(f.name.name.clone());
            }
        }
    }
    fns.extend(program_fns.iter().cloned());
    for p in programs.iter_mut() {
        let mut w = Walk { fns: &fns, program_fns: &program_fns, scopes: Vec::new() };
        for item in &mut p.items {
            w.top_decl(item);
        }
    }
}

struct Walk<'f> {
    fns: &'f BTreeSet<String>,
    /// The program's fns alone: what a name read as a value may be.
    program_fns: &'f BTreeSet<String>,
    /// The bindings in scope, innermost last, each with whether it is
    /// declared as a fn value ([`Walk::fn_valued`]).
    scopes: Vec<BTreeMap<String, bool>>,
}

impl Walk<'_> {
    /// The innermost binding of `name` in scope: whether it is a fn value.
    fn bound(&self, name: &str) -> Option<bool> {
        self.scopes.iter().rev().find_map(|s| s.get(name).copied())
    }

    fn bind(&mut self, name: &str, is_fn: bool) {
        if let Some(s) = self.scopes.last_mut() {
            s.insert(name.to_string(), is_fn);
        }
    }

    fn scoped(&mut self, names: Vec<(String, bool)>, f: impl FnOnce(&mut Self)) {
        self.scopes.push(names.into_iter().collect());
        f(self);
        self.scopes.pop();
    }

    /// Is a value spelled `e` a fn value, as far as its spelling says: a
    /// fn's name (no binding in scope of the name, or one that is a fn
    /// value itself).
    fn fn_named(&self, e: &Expr) -> bool {
        match e {
            Expr::Ident(id) => self.bound(&id.name).unwrap_or_else(|| self.program_fns.contains(&id.name)),
            _ => false,
        }
    }

    /// Whether a binding declared with type `ty` (written or not) and
    /// value `value` is a fn value, as its declaration says: its written
    /// type is a fn type, or it is bound to a fn's name.
    fn fn_valued(&self, ty: Option<&TypeExpr>, value: Option<&Expr>) -> bool {
        match ty {
            Some(t) => matches!(t, TypeExpr::Function { .. }),
            None => value.is_some_and(|v| self.fn_named(v)),
        }
    }

    fn top_decl(&mut self, d: &mut TopDecl) {
        match d {
            TopDecl::Locus(l) => {
                if let Some(form) = &mut l.form {
                    for arg in &mut form.args {
                        self.expr(&mut arg.value);
                    }
                }
                for member in &mut l.members {
                    self.locus_member(member);
                }
            }
            TopDecl::Perspective(p) => {
                for member in &mut p.members {
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
                for item in &mut md.items {
                    self.top_decl(item);
                }
            }
            TopDecl::Interface(i) => {
                for sig in &mut i.methods {
                    self.params(&mut sig.params);
                }
            }
            TopDecl::Topic(t) => self.ty(&mut t.payload),
            TopDecl::Group(_)
            | TopDecl::RingLayout(_)
            | TopDecl::Target(_)
            | TopDecl::Role(_)
            | TopDecl::Claims(_)
            | TopDecl::Constitution(_)
            | TopDecl::Unit(_)
            | TopDecl::Api(_) => {}
        }
    }

    fn locus_member(&mut self, member: &mut LocusMember) {
        match member {
            LocusMember::Params(pb) => self.params_block(pb),
            LocusMember::Contract(_) => {}
            LocusMember::Bus(bb) => self.bus(bb),
            LocusMember::Lifecycle(ld) => self.body(&mut ld.params, &mut ld.body),
            LocusMember::Mode(md) => self.body(&mut md.params, &mut md.body),
            LocusMember::Failure(fd) => self.body(&mut fd.params, &mut fd.body),
            LocusMember::Closure(cd) => {
                if let Some(a) = &mut cd.assertion {
                    self.expr(&mut a.left);
                    self.expr(&mut a.right);
                    self.expr(&mut a.tolerance);
                }
                for clause in &mut cd.clauses {
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
                for slot in &mut cb.slots {
                    self.ty(&mut slot.elem_ty);
                }
            }
            LocusMember::Bindings(bb) => {
                for entry in &mut bb.entries {
                    match &mut entry.transport {
                        TransportSpec::Adapter { inits, .. } => self.struct_inits(inits),
                        TransportSpec::Unix { .. } | TransportSpec::ShmRing { .. } => {}
                    }
                    if let Some(codec) = &mut entry.codec {
                        self.struct_inits(&mut codec.inits);
                    }
                }
            }
            LocusMember::BirthCheck(bc) => {
                self.expr(&mut bc.cond);
                if let Some(p) = &mut bc.payload {
                    self.expr(p);
                }
            }
            LocusMember::Placement(_) | LocusMember::Topology(_) | LocusMember::Claims(_) => {}
        }
    }

    fn params_block(&mut self, pb: &mut ParamsBlock) {
        for p in &mut pb.params {
            if let Some(t) = &mut p.ty {
                self.ty(t);
            }
            match &mut p.init {
                ParamInit::Value(e) => self.expr(e),
                ParamInit::Inferred => {}
            }
        }
    }

    fn bus(&mut self, bb: &mut BusBlock) {
        for member in &mut bb.members {
            match member {
                BusMember::Subscribe { ty, key_filter, .. } => {
                    if let Some(t) = ty {
                        self.ty(t);
                    }
                    match key_filter {
                        Some(KeyFilter::Specific { expr, .. }) => self.expr(expr),
                        Some(KeyFilter::Unmatched { .. } | KeyFilter::Replica { .. }) | None => {}
                    }
                }
                BusMember::Publish { ty, .. } => {
                    if let Some(t) = ty {
                        self.ty(t);
                    }
                }
            }
        }
    }

    fn type_decl(&mut self, t: &mut TypeDecl) {
        match &mut t.body {
            TypeDeclBody::Alias(a) => self.ty(a),
            TypeDeclBody::Struct(fields) => {
                for field in fields {
                    self.ty(&mut field.ty);
                    if let Some(d) = &mut field.default {
                        self.expr(d);
                    }
                }
            }
            TypeDeclBody::Enum(variants) => {
                for v in variants {
                    for t in &mut v.fields {
                        self.ty(t);
                    }
                }
            }
            TypeDeclBody::Scalar(sc) => {
                self.ty(&mut sc.base);
                for c in &mut sc.clauses {
                    if let ScalarClause::Range { lo, hi, .. } = c {
                        self.expr(lo);
                        self.expr(hi);
                    }
                }
            }
        }
    }

    fn const_decl(&mut self, c: &mut ConstDecl) {
        self.ty(&mut c.ty);
        self.expr(&mut c.value);
    }

    fn fn_decl(&mut self, fd: &mut FnDecl) {
        self.body(&mut fd.params, &mut fd.body);
    }

    /// A parameter's default is evaluated where the call leaves it, in
    /// no binding of the body's: walked outside the parameters' scope.
    fn params(&mut self, ps: &mut [Param]) {
        for p in ps {
            self.ty(&mut p.ty);
            if let Some(d) = &mut p.default {
                self.expr(d);
            }
        }
    }

    /// A body: its parameters' types and defaults, then the body with the
    /// parameters in scope.
    fn body(&mut self, ps: &mut [Param], b: &mut Block) {
        self.params(ps);
        let names = ps.iter().map(|p| (p.name.name.clone(), matches!(p.ty, TypeExpr::Function { .. }))).collect();
        self.scoped(names, |w| w.block(b));
    }

    /// An array type's size is an expression.
    fn ty(&mut self, t: &mut TypeExpr) {
        match t {
            TypeExpr::Named { generic_args, .. } => {
                for a in generic_args {
                    self.ty(a);
                }
            }
            TypeExpr::Projection { inner, .. } => self.ty(inner),
            TypeExpr::Array { elem, size, .. } => {
                self.ty(elem);
                if let Some(e) = size {
                    self.expr(e);
                }
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

    fn struct_inits(&mut self, inits: &mut [StructInit]) {
        for init in inits {
            self.expr(&mut init.value);
        }
    }

    fn block(&mut self, b: &mut Block) {
        self.scoped(Vec::new(), |w| {
            for s in &mut b.stmts {
                w.stmt(s);
            }
            if let Some(tail) = &mut b.tail {
                w.expr(tail);
            }
        });
    }

    fn stmt(&mut self, s: &mut Stmt) {
        match s {
            Stmt::Let { name, ty, value, .. } => {
                self.expr(value);
                let is_fn = self.fn_valued(ty.as_ref(), Some(value));
                self.bind(&name.name, is_fn);
            }
            Stmt::LetTuple { names, ty, value, .. } => {
                self.expr(value);
                for (i, n) in names.iter().enumerate() {
                    let is_fn = match (&*ty, &*value) {
                        (Some(TypeExpr::Tuple(parts, _)), _) => matches!(parts.get(i), Some(TypeExpr::Function { .. })),
                        (Some(_), _) => false,
                        (None, Expr::Tuple(parts, _)) => parts.get(i).is_some_and(|p| self.fn_named(p)),
                        (None, _) => false,
                    };
                    self.bind(&n.name, is_fn);
                }
            }
            Stmt::Assign { target, value, .. } => {
                for seg in &mut target.tail {
                    if let LValueSeg::Index(e) = seg {
                        self.expr(e);
                    }
                }
                self.expr(value);
            }
            Stmt::If(i) => self.if_stmt(i),
            Stmt::Match(ms) => self.match_stmt(ms),
            Stmt::For { name, iter, body, .. } => {
                self.expr(iter);
                // Over an array literal of fns' names: a fn value each.
                let is_fn = matches!(&*iter, Expr::Array(parts, _) if !parts.is_empty() && parts.iter().all(|p| self.fn_named(p)));
                let name = name.name.clone();
                self.scoped(vec![(name, is_fn)], |w| w.block(body));
            }
            Stmt::While { cond, body, .. } => {
                self.expr(cond);
                self.block(body);
            }
            Stmt::Return(Some(e), _) => self.expr(e),
            Stmt::Fail { value, .. } => self.expr(value),
            Stmt::Block(b) => self.block(b),
            Stmt::Recovery { args, modifier, .. } => {
                for a in args {
                    self.expr(a);
                }
                if let Some(RecoveryModifier::For(e) | RecoveryModifier::Until(e)) = modifier {
                    self.expr(e);
                }
            }
            Stmt::Violate { payload: Some(p), .. } => self.expr(p),
            Stmt::Send { subject, value, or_disposition, .. } => {
                self.expr(subject);
                self.expr(value);
                if let Some(d) = or_disposition {
                    self.disposition(d);
                }
            }
            Stmt::ShmWrite { max, binding, body, .. } => {
                self.expr(max);
                let name = binding.name.clone();
                self.scoped(vec![(name, false)], |w| w.block(body));
            }
            Stmt::Expr(e) => self.expr(e),
            Stmt::Return(None, _)
            | Stmt::Violate { payload: None, .. }
            | Stmt::Break(_)
            | Stmt::Continue(_)
            | Stmt::Yield(_)
            | Stmt::Terminate(_)
            | Stmt::Reperspective { .. } => {}
        }
    }

    fn if_stmt(&mut self, i: &mut IfStmt) {
        self.expr(&mut i.cond);
        self.block(&mut i.then_block);
        if let Some(eb) = &mut i.else_block {
            match &mut **eb {
                ElseBranch::Else(b) => self.block(b),
                ElseBranch::ElseIf(i) => self.if_stmt(i),
            }
        }
    }

    fn match_stmt(&mut self, ms: &mut MatchStmt) {
        self.expr(&mut ms.scrutinee);
        for arm in &mut ms.arms {
            let mut names = Vec::new();
            pattern_names(&arm.pattern, &mut names);
            // A pattern's binding says nothing of its type.
            self.scoped(names.into_iter().map(|n| (n, false)).collect(), |w| {
                if let Some(g) = &mut arm.guard {
                    w.expr(g);
                }
                match &mut arm.body {
                    MatchArmBody::Expr(e) => w.expr(e),
                    MatchArmBody::Block(b) => w.block(b),
                }
            });
        }
    }

    fn disposition(&mut self, d: &mut OrDisposition) {
        match d {
            OrDisposition::Substitute(e) | OrDisposition::Fail(e, _) => self.expr(e),
            OrDisposition::Raise(_) | OrDisposition::Discard(_) | OrDisposition::Wait(_) => {}
        }
    }

    fn expr(&mut self, e: &mut Expr) {
        match e {
            Expr::Call { callee, args, span: call_span, .. } => {
                if let Expr::Ident(id) = callee.as_ref() {
                    // A call the compiler wrote (an f-string's `to_string`,
                    // `__fmt`, a `from_json`'s parser) carries its callee at
                    // the call's own span, which no written call does: the
                    // compiler's, never a binding's.
                    let generated = id.span == *call_span;
                    if !generated && self.bound(&id.name) == Some(true) && self.fns.contains(&id.name) {
                        let span = id.span;
                        let read = std::mem::replace(callee.as_mut(), Expr::Literal(Literal::Nil, span));
                        **callee = Expr::Block(Block { stmts: Vec::new(), tail: Some(Box::new(read)), span });
                    }
                } else {
                    self.expr(callee);
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
            Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. } => self.expr(receiver),
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
            Expr::Or { inner, disposition, .. } => {
                self.expr(inner);
                self.disposition(disposition);
            }
            Expr::Ident(_) | Expr::Path(_) | Expr::Literal(..) | Expr::KwSelf(_) => {}
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

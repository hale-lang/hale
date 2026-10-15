//! A fn value bound in scope shadows a fn of the same name in a call.
//!
//! `fn apply(f: fn(Int) -> Int, x: Int) -> Int { return f(x); }` beside a
//! top-level `fn f` calls the parameter: the binding in scope, as the code
//! reads and as the name already means everywhere else it is read (a value
//! position names the binding, `fn_values`). A call spells its callee as
//! a bare name, though, and every pass that follows calls resolves a bare
//! callee by name, among the program's fns first; lowering did too. So the
//! one place the rule lives is here, in the desugar sequence before the
//! check: a call whose bare callee names a binding in scope (a parameter,
//! a `let`, a tuple `let`'s name, a `for` binder, a `match` pattern's
//! binding, a `shm_write` binding) and also one of the program's fns or
//! one of the builtins the compiler answers by name (`len`, `min`, a
//! `bounded` intrinsic, a printer: `check::BARE_BUILTIN_CALLEES`) is
//! rewritten to call the binding as a value, `{ f }(x)`. A callee that is
//! not a bare name is a call through a value to every pass already (the
//! checker types the block as the binding's fn type, lowering calls the
//! value it evaluates to, the summaries treat it as indirect), and the
//! name inside it is a read of the binding. A callee no fn or builtin
//! shares its name with is left as written: it already calls the binding.
//!
//! `__fmt`, the desugaring of `f"{x:spec}"`, is the one builtin no binding
//! shadows: the parser writes the call into the user's own scope, so it is
//! the compiler's whatever a binding of the name holds, in the check as in
//! lowering.
//!
//! Only bodies hold bindings: a fn's, a locus's fns and hooks, a
//! perspective's fns and `stable_when`. The rewritten callee keeps its
//! span.

use std::collections::BTreeSet;

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
    for p in programs.iter().map(|p| &**p).chain(context.iter().copied()) {
        for item in flat_decls(&p.items) {
            if let TopDecl::Fn(f) = item {
                fns.insert(f.name.name.clone());
            }
        }
    }
    for p in programs.iter_mut() {
        let mut w = Walk { fns: &fns, scopes: Vec::new() };
        for item in &mut p.items {
            w.top_decl(item);
        }
    }
}

struct Walk<'f> {
    fns: &'f BTreeSet<String>,
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

    fn scoped(&mut self, names: Vec<String>, f: impl FnOnce(&mut Self)) {
        self.scopes.push(names.into_iter().collect());
        f(self);
        self.scopes.pop();
    }

    fn top_decl(&mut self, d: &mut TopDecl) {
        match d {
            TopDecl::Fn(fd) => self.fn_decl(fd),
            TopDecl::Module(md) => {
                for item in &mut md.items {
                    self.top_decl(item);
                }
            }
            TopDecl::Locus(l) => {
                for member in &mut l.members {
                    match member {
                        LocusMember::Fn(fd) => self.fn_decl(fd),
                        LocusMember::Lifecycle(ld) => self.body(&ld.params, &mut ld.body),
                        LocusMember::Mode(md) => self.body(&md.params, &mut md.body),
                        LocusMember::Failure(fd) => self.body(&fd.params, &mut fd.body),
                        _ => {}
                    }
                }
            }
            TopDecl::Perspective(p) => {
                for member in &mut p.members {
                    match member {
                        PerspectiveMember::Fn(fd) => self.fn_decl(fd),
                        PerspectiveMember::StableWhen(b) => self.block(b),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    fn fn_decl(&mut self, fd: &mut FnDecl) {
        self.body(&fd.params, &mut fd.body);
    }

    /// A body, its parameters in scope.
    fn body(&mut self, ps: &[Param], b: &mut Block) {
        let names = ps.iter().map(|p| p.name.name.clone()).collect();
        self.scoped(names, |w| w.block(b));
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
            Stmt::Let { name, value, .. } => {
                self.expr(value);
                self.bind(&name.name);
            }
            Stmt::LetTuple { names, value, .. } => {
                self.expr(value);
                for n in names.iter() {
                    self.bind(&n.name);
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
                let name = name.name.clone();
                self.scoped(vec![name], |w| w.block(body));
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
                self.scoped(vec![name], |w| w.block(body));
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
            self.scoped(names, |w| {
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
            Expr::Call { callee, args, .. } => {
                if let Expr::Ident(id) = callee.as_ref() {
                    if self.bound(&id.name) && self.fns.contains(&id.name) {
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
            Expr::Struct { inits, .. } => {
                for i in inits {
                    self.expr(&mut i.value);
                }
            }
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

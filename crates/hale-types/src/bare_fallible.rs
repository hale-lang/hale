//! GH #738 — a bare fallible stdlib call is a call that says nothing
//! about failure.
//!
//! Every stdlib entry point the signature table marks `fallible` can
//! be called bare, with no `or`: the call keeps the legacy form (the
//! success value, or an Int status for the write fns), and the corpus
//! relies on it, so the checker has always let it through. The ruling
//! of 2026-09-20 stages the end of that: by default the bare call is a
//! **warning** naming the callee and the missing disposition;
//! `hale check --strict-fallible` makes it an **error**; the default
//! flips at the next minor with a migration note. A handled call
//! (`or raise`, `or <fallback>`, `or handler(err)`) and a deliberately
//! discarded one (`or discard`) say nothing here.
//!
//! The inventory is the table itself: [`crate::stdlib_surface::FnSig`]
//! rows whose `fallible` is `Some`. This pass reads it, so an entry
//! point added to the table is covered the day it is added.
//!
//! A standalone walk over the seed's own declarations, like the strict
//! `@secret` pass: the checker's typing of the bare call (permissive,
//! the success type) is unchanged, so `hale build` lowers it exactly
//! as before.

use hale_syntax::ast::{
    Block, ElseBranch, Expr, LocusMember, MatchArmBody, ParamInit,
    Program, Stmt, TopDecl,
};
use hale_syntax::error::Diag;

/// Every bare fallible stdlib call in `programs`, as warnings, or as
/// errors under `strict`.
pub fn bare_fallible_calls(programs: &[&Program], strict: bool) -> Vec<Diag> {
    let mut w = Walk { strict, diags: Vec::new() };
    for p in programs {
        w.items(&p.items);
    }
    w.diags
}

struct Walk {
    strict: bool,
    diags: Vec<Diag>,
}

impl Walk {
    fn items(&mut self, items: &[TopDecl]) {
        for item in items {
            match item {
                TopDecl::Fn(fd) => self.block(&fd.body),
                TopDecl::Const(c) => self.expr(&c.value, false),
                TopDecl::Module(m) => self.items(&m.items),
                TopDecl::Locus(l) => {
                    for m in &l.members {
                        match m {
                            LocusMember::Fn(fd) => self.block(&fd.body),
                            LocusMember::Lifecycle(lc) => self.block(&lc.body),
                            LocusMember::Mode(md) => self.block(&md.body),
                            LocusMember::Const(c) => self.expr(&c.value, false),
                            LocusMember::Params(pb) => {
                                for pd in &pb.params {
                                    if let ParamInit::Value(e) = &pd.init {
                                        self.expr(e, false);
                                    }
                                }
                            }
                            LocusMember::Closure(cd) => {
                                if let Some(a) = &cd.assertion {
                                    self.expr(&a.left, false);
                                    self.expr(&a.right, false);
                                    self.expr(&a.tolerance, false);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn block(&mut self, b: &Block) {
        for s in &b.stmts {
            self.stmt(s);
        }
        if let Some(t) = &b.tail {
            self.expr(t, false);
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Let { value, .. } | Stmt::LetTuple { value, .. } => self.expr(value, false),
            Stmt::Assign { target, value, .. } => {
                for seg in &target.tail {
                    if let hale_syntax::ast::LValueSeg::Index(ix) = seg {
                        self.expr(ix, false);
                    }
                }
                self.expr(value, false);
            }
            Stmt::If(i) => self.if_stmt(i),
            Stmt::Match(m) => self.match_stmt(m),
            Stmt::For { iter, body, .. } => {
                self.expr(iter, false);
                self.block(body);
            }
            Stmt::While { cond, body, .. } => {
                self.expr(cond, false);
                self.block(body);
            }
            Stmt::Return(Some(e), _) => self.expr(e, false),
            Stmt::Fail { value, .. } => self.expr(value, false),
            Stmt::Block(b) => self.block(b),
            Stmt::Recovery { args, modifier, .. } => {
                for a in args {
                    self.expr(a, false);
                }
                match modifier {
                    Some(hale_syntax::ast::RecoveryModifier::For(e))
                    | Some(hale_syntax::ast::RecoveryModifier::Until(e)) => self.expr(e, false),
                    _ => {}
                }
            }
            Stmt::Violate { payload: Some(e), .. } => self.expr(e, false),
            Stmt::Send { subject, value, .. } => {
                self.expr(subject, false);
                self.expr(value, false);
            }
            Stmt::ShmWrite { max, body, .. } => {
                self.expr(max, false);
                self.block(body);
            }
            Stmt::Expr(e) => self.expr(e, false),
            _ => {}
        }
    }

    fn if_stmt(&mut self, i: &hale_syntax::ast::IfStmt) {
        self.expr(&i.cond, false);
        self.block(&i.then_block);
        match i.else_block.as_deref() {
            Some(ElseBranch::Else(b)) => self.block(b),
            Some(ElseBranch::ElseIf(n)) => self.if_stmt(n),
            None => {}
        }
    }

    fn match_stmt(&mut self, m: &hale_syntax::ast::MatchStmt) {
        self.expr(&m.scrutinee, false);
        for arm in &m.arms {
            if let Some(g) = &arm.guard {
                self.expr(g, false);
            }
            match &arm.body {
                MatchArmBody::Expr(e) => self.expr(e, false),
                MatchArmBody::Block(b) => self.block(b),
            }
        }
    }

    /// `addressed` is true for the direct operand of an `or`: that
    /// call has said what happens on failure. Everything beneath it
    /// has not.
    fn expr(&mut self, e: &Expr, addressed: bool) {
        match e {
            Expr::Call { callee, args, span, .. } => {
                if !addressed {
                    if let Expr::Path(qn) = callee.as_ref() {
                        let segs: Vec<&str> =
                            qn.segments.iter().map(|s| s.name.as_str()).collect();
                        if let Some(sig) = crate::stdlib_surface::signature_for(&segs) {
                            if let Some(payload) = sig.fallible {
                                let msg = format!(
                                    "`{}` can fail ({}) and this call says nothing \
                                     about it: write `or raise`, `or <fallback>`, \
                                     `or discard`, or `or handler(err)`. The bare \
                                     call keeps the legacy form for now — the \
                                     success value, or an Int status for a write — \
                                     and is refused under `hale check \
                                     --strict-fallible`; the default becomes an \
                                     error at the next minor (GH #738).",
                                    sig.display_path(),
                                    payload
                                );
                                self.diags.push(if self.strict {
                                    Diag::ty(*span, msg)
                                } else {
                                    Diag::warn(*span, msg)
                                });
                            }
                        }
                    }
                }
                self.expr(callee, false);
                for a in args {
                    self.expr(a, false);
                }
            }
            Expr::Or { inner, disposition, .. } => {
                self.expr(inner, true);
                if let hale_syntax::ast::OrDisposition::Substitute(sub) = disposition {
                    self.expr(sub, false);
                }
            }
            Expr::Binary { left, right, .. } => {
                self.expr(left, false);
                self.expr(right, false);
            }
            Expr::Unary { operand, .. } => self.expr(operand, false),
            Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. } => {
                self.expr(receiver, false)
            }
            Expr::Index { receiver, index, .. } => {
                self.expr(receiver, false);
                self.expr(index, false);
            }
            Expr::Tuple(es, _) | Expr::Array(es, _) => {
                for x in es {
                    self.expr(x, false);
                }
            }
            Expr::Struct { inits, .. } => {
                for i in inits {
                    self.expr(&i.value, false);
                }
            }
            Expr::Block(b) => self.block(b),
            Expr::If(i) => self.if_stmt(i),
            Expr::Match(m) => self.match_stmt(m),
            Expr::Sum(x, _) | Expr::Prod(x, _) => self.expr(x, false),
            Expr::Approx { left, right, tolerance, .. } => {
                self.expr(left, false);
                self.expr(right, false);
                self.expr(tolerance, false);
            }
            Expr::Range { lo, hi, .. } => {
                self.expr(lo, false);
                self.expr(hi, false);
            }
            Expr::ArrayRepeat { val, .. } => self.expr(val, false),
            Expr::Literal(..) | Expr::Ident(_) | Expr::Path(_) | Expr::KwSelf(_) => {}
        }
    }
}

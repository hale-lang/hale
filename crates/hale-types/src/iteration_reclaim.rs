//! Per-iteration reclaim: which loop bodies free what an iteration
//! makes when the iteration ends.
//!
//! A frame with a per-call scratch (a locus method, a lifecycle hook, a
//! handler) allocates into that scratch until it returns, so a loop in
//! it used to keep every iteration's values until the frame returned: a
//! `run()` that loops for its locus's whole life kept them until the
//! locus dissolved, while the same body moved into a method stayed flat.
//! Lowering now gives such a loop body an arena of its own for each
//! iteration (`lotus_arena_iteration_enter`), freed when the next
//! iteration starts or the loop is left. This module is the one rule
//! deciding which bodies get it, read by lowering
//! (`hale_codegen::Cx::lower_loop_body`) and by the allocation summary
//! (`crate::alloc_summary`), which reports the sites such a body makes
//! as reclaimed per iteration.
//!
//! What an iteration hands on keeps the ownership it has without the
//! rule: a value stored into `self` (or any locus) is copied into that
//! locus's arena, a sent value into the payload arena, a returned one
//! into the caller's (the `return` is lowered outside the iteration's
//! arena). What the rule refuses is the one way left for a value to
//! outlive its iteration: a binding declared before the body. So a body
//! reclaims per iteration unless it
//!
//! - assigns a binding declared outside it that can hold arena memory
//!   (anything but a scalar or a locus reference), whole or through a
//!   field or an index;
//! - writes an element into a `bounded[T; N]` held by such a binding
//!   (`push(kept.items, r)`, `set`, `truncate`, `clear`): the element is
//!   copied into the current arena, which is the iteration's;
//! - passes such a binding, or a path rooted in one, to a call that is
//!   not a method of a locus and not a builtin that only reads: a free fn
//!   handed `outer` can write into it (`fn keep(b: Bag, p: Bytes) {
//!   push(b.items, p) }`), and what it writes is made in its caller
//!   arena, the iteration's. A locus's method owns what it stores (it
//!   copies into its own arena), so `h.update(c)` and `buf.snapshot()`
//!   stay admitted;
//! - names an outside binding whose value grows in the frame's arena when
//!   used (a `bounded[T; N]` or a `BytesMut` itself);
//! - makes a locus (a literal or a factory call): its instance, its slot
//!   and the GH #815 per-iteration teardown live past the iteration;
//! - holds a statement whose teardown runs outside the body's view
//!   (`reperspective`, a recovery op, `violate`, a shared-memory write).
//!
//! Names are resolved lexically: a `let` (or a `for` binder) inside the
//! body shadows the outside binding from there on. A binding of the body
//! whose value names such an outside binding (`let o = outer;`, `for r in
//! outer.items`, `o = outer.inner`; a carrier's arms and a block's
//! statements are read too) is an alias of it and is judged as the
//! outside binding is; an alias declared before the loop is an
//! outside binding of its own and needs nothing more. A match-arm binder
//! is not declared here, so an assignment to one, and a use of one
//! sharing an outside buffer's name, refuse the body: the conservative
//! side.

use std::collections::BTreeMap;

use hale_syntax::ast::*;

/// What a binding declared outside the loop body holds, as the reader
/// knows it (lowering from the binding's type, the summary from its
/// declaration).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OuterLocal {
    /// An Int, Float, Bool, Duration, Decimal or Time: no arena memory.
    Scalar,
    /// A locus (or interface, perspective) reference: an assignment
    /// re-points it, and a store through it copies into that locus.
    Locus,
    /// Anything that may point into an arena.
    Heap,
    /// A `bounded[T; N]` or a `BytesMut`: using it at all may grow it in
    /// the frame's current arena.
    Buffer,
}

/// Does `body` reclaim per iteration? `outer` classifies a name bound
/// outside the body (`None`: no such local, e.g. a field or a constant
/// spelled without `self`, which an assignment then refuses on);
/// `produces_locus` says whether a struct literal or call makes a locus.
pub fn reclaims_per_iteration(
    body: &Block,
    outer: &dyn Fn(&str) -> Option<OuterLocal>,
    produces_locus: &dyn Fn(&Expr) -> bool,
) -> bool {
    let mut w = Walk { outer, produces_locus, scopes: Vec::new() };
    w.block(body)
}

/// The `bounded[T; N]` intrinsics that write into their first argument.
const BOUNDED_WRITES: [&str; 4] = ["push", "set", "truncate", "clear"];

/// The binding a place expression reads (`kept.items[i]` → `kept`).
fn root_ident(e: &Expr) -> Option<&str> {
    match e {
        Expr::Ident(i) => Some(i.name.as_str()),
        Expr::KwSelf(_) => Some("self"),
        Expr::Field { receiver, .. } | Expr::Index { receiver, .. } => root_ident(receiver),
        _ => None,
    }
}

/// The builtins that only read their arguments (no user fn can take
/// their names): a call of one hands nothing anywhere.
const READ_BUILTINS: [&str; 13] =
    ["len", "to_string", "Int", "Float", "abs", "min", "max", "starts_with", "contains", "println", "print", "eprintln", "eprint"];

struct Walk<'a> {
    outer: &'a dyn Fn(&str) -> Option<OuterLocal>,
    produces_locus: &'a dyn Fn(&Expr) -> bool,
    /// The body's bindings, innermost scope last: whether each is an
    /// alias of storage held outside the body.
    scopes: Vec<BTreeMap<String, bool>>,
}

impl Walk<'_> {
    /// A binding of the body: whether it aliases outside storage.
    fn local(&self, name: &str) -> Option<bool> {
        self.scopes.iter().rev().find_map(|s| s.get(name).copied())
    }

    fn declared(&self, name: &str) -> bool {
        self.local(name).is_some()
    }

    fn declare(&mut self, name: &str, alias: bool) {
        if let Some(s) = self.scopes.last_mut() {
            s.insert(name.to_string(), alias);
        }
    }

    /// Re-mark a binding of the body as an alias (an assignment gave it
    /// outside storage).
    fn mark_alias(&mut self, name: &str) {
        if let Some(s) = self.scopes.iter_mut().rev().find(|s| s.contains_key(name)) {
            s.insert(name.to_string(), true);
        }
    }

    /// Does `name` hold storage from outside the body: an outside binding
    /// that is not a scalar or a locus reference, or an alias of one.
    fn holds_outside(&self, name: &str) -> bool {
        match self.local(name) {
            Some(alias) => alias,
            None => matches!((self.outer)(name), Some(OuterLocal::Heap | OuterLocal::Buffer)),
        }
    }

    /// Does `e` name storage from outside the body anywhere in it (a
    /// carrier or block is taken to, the conservative side)?
    fn mentions_outside(&self, e: &Expr) -> bool {
        match e {
            Expr::Ident(i) => self.holds_outside(&i.name),
            Expr::Literal(..) | Expr::Path(_) | Expr::KwSelf(_) => false,
            Expr::Binary { left, right, .. } => self.mentions_outside(left) || self.mentions_outside(right),
            Expr::Unary { operand, .. } => self.mentions_outside(operand),
            Expr::Call { callee, args, .. } => self.mentions_outside(callee) || args.iter().any(|a| self.mentions_outside(a)),
            Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. } => self.mentions_outside(receiver),
            Expr::Index { receiver, index, .. } => self.mentions_outside(receiver) || self.mentions_outside(index),
            Expr::Tuple(xs, _) | Expr::Array(xs, _) => xs.iter().any(|x| self.mentions_outside(x)),
            Expr::Struct { inits, .. } => inits.iter().any(|i| self.mentions_outside(&i.value)),
            Expr::Sum(x, _) | Expr::Prod(x, _) => self.mentions_outside(x),
            Expr::Approx { left, right, tolerance, .. } => {
                self.mentions_outside(left) || self.mentions_outside(right) || self.mentions_outside(tolerance)
            }
            Expr::Range { lo, hi, .. } => self.mentions_outside(lo) || self.mentions_outside(hi),
            Expr::ArrayRepeat { val, .. } => self.mentions_outside(val),
            Expr::Or { inner, disposition, .. } => {
                self.mentions_outside(inner)
                    || match disposition {
                        OrDisposition::Substitute(x) | OrDisposition::Fail(x, _) => self.mentions_outside(x),
                        _ => false,
                    }
            }
            Expr::Block(b) => self.block_mentions_outside(b),
            Expr::If(i) => self.if_mentions_outside(i),
            Expr::Match(m) => {
                self.mentions_outside(&m.scrutinee)
                    || m.arms.iter().any(|a| match &a.body {
                        MatchArmBody::Expr(x) => self.mentions_outside(x),
                        MatchArmBody::Block(b) => self.block_mentions_outside(b),
                    })
            }
        }
    }

    /// [`Self::mentions_outside`] over a block a value comes out of: its
    /// statements and its tail, a statement it cannot read through taken
    /// to mention.
    fn block_mentions_outside(&self, b: &Block) -> bool {
        b.tail.as_deref().is_some_and(|t| self.mentions_outside(t))
            || b.stmts.iter().any(|st| match st {
                Stmt::Expr(x) | Stmt::Let { value: x, .. } | Stmt::LetTuple { value: x, .. } => self.mentions_outside(x),
                Stmt::Return(x, _) => x.as_ref().is_some_and(|x| self.mentions_outside(x)),
                Stmt::Fail { value, .. } => self.mentions_outside(value),
                Stmt::If(i) => self.if_mentions_outside(i),
                Stmt::Block(b) => self.block_mentions_outside(b),
                Stmt::Break(_) | Stmt::Continue(_) | Stmt::Yield(_) | Stmt::Terminate(_) => false,
                _ => true,
            })
    }

    fn if_mentions_outside(&self, i: &IfStmt) -> bool {
        self.mentions_outside(&i.cond)
            || self.block_mentions_outside(&i.then_block)
            || match i.else_block.as_deref() {
                None => false,
                Some(ElseBranch::Else(b)) => self.block_mentions_outside(b),
                Some(ElseBranch::ElseIf(e)) => self.if_mentions_outside(e),
            }
    }

    /// Is the call `callee(..)` one that cannot write into its
    /// arguments' storage: a method of a locus (`self`, or a binding from
    /// outside the body holding a locus reference), or a builtin that
    /// only reads.
    fn call_owns_its_stores(&self, callee: &Expr) -> bool {
        match callee {
            Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. } => match root_ident(receiver) {
                Some("self") => true,
                Some(r) => !self.declared(r) && (self.outer)(r) == Some(OuterLocal::Locus),
                None => false,
            },
            Expr::Ident(f) => !self.declared(&f.name) && READ_BUILTINS.contains(&f.name.as_str()),
            _ => false,
        }
    }

    fn block(&mut self, b: &Block) -> bool {
        self.scopes.push(BTreeMap::new());
        let ok = b.stmts.iter().all(|s| self.stmt(s)) && b.tail.as_deref().is_none_or(|t| self.expr(t));
        self.scopes.pop();
        ok
    }

    fn stmt(&mut self, s: &Stmt) -> bool {
        match s {
            Stmt::Let { name, value, .. } => {
                let ok = self.expr(value);
                let alias = self.mentions_outside(value);
                self.declare(&name.name, alias);
                ok
            }
            Stmt::LetTuple { names, value, .. } => {
                let ok = self.expr(value);
                let alias = self.mentions_outside(value);
                for n in names {
                    self.declare(&n.name, alias);
                }
                ok
            }
            Stmt::Assign { target, value, .. } => {
                let ok = self.assign_target(target) && self.expr(value);
                if ok && self.mentions_outside(value) {
                    self.mark_alias(&target.head.name);
                }
                ok
            }
            Stmt::If(i) => self.if_stmt(i),
            Stmt::Match(m) => self.match_stmt(m),
            Stmt::For { name, iter, body, .. } => {
                if !self.expr(iter) {
                    return false;
                }
                let alias = self.mentions_outside(iter);
                self.scopes.push(BTreeMap::from([(name.name.clone(), alias)]));
                let ok = self.block(body);
                self.scopes.pop();
                ok
            }
            Stmt::While { cond, body, .. } => self.expr(cond) && self.block(body),
            Stmt::Return(e, _) => e.as_ref().is_none_or(|e| self.expr(e)),
            Stmt::Fail { value, .. } => self.expr(value),
            Stmt::Send { subject, value, or_disposition, .. } => {
                self.expr(subject) && self.expr(value) && or_disposition.as_ref().is_none_or(|d| self.disposition(d))
            }
            Stmt::Block(b) => self.block(b),
            Stmt::Expr(e) => self.expr(e),
            Stmt::Break(_) | Stmt::Continue(_) | Stmt::Yield(_) | Stmt::Terminate(_) => true,
            Stmt::Reperspective { .. } | Stmt::Recovery { .. } | Stmt::Violate { .. } | Stmt::ShmWrite { .. } => false,
        }
    }

    /// May `head` be written from the body: `self`, a binding the body
    /// declares, or an outside one holding no arena memory of its own.
    fn writable(&self, head: &str) -> bool {
        head == "self"
            || match self.local(head) {
                Some(alias) => !alias,
                None => matches!((self.outer)(head), Some(OuterLocal::Scalar | OuterLocal::Locus)),
            }
    }

    fn assign_target(&mut self, target: &LValue) -> bool {
        let held = self.writable(target.head.name.as_str());
        held && target.tail.iter().all(|seg| match seg {
            LValueSeg::Field(_) => true,
            LValueSeg::Index(e) => self.expr(e),
        })
    }

    fn if_stmt(&mut self, i: &IfStmt) -> bool {
        self.expr(&i.cond)
            && self.block(&i.then_block)
            && match i.else_block.as_deref() {
                None => true,
                Some(ElseBranch::Else(b)) => self.block(b),
                Some(ElseBranch::ElseIf(e)) => self.if_stmt(e),
            }
    }

    fn match_stmt(&mut self, m: &MatchStmt) -> bool {
        self.expr(&m.scrutinee)
            && m.arms.iter().all(|arm| {
                arm.guard.as_ref().is_none_or(|g| self.expr(g))
                    && match &arm.body {
                        MatchArmBody::Expr(e) => self.expr(e),
                        MatchArmBody::Block(b) => self.block(b),
                    }
            })
    }

    fn disposition(&mut self, d: &OrDisposition) -> bool {
        match d {
            OrDisposition::Substitute(e) | OrDisposition::Fail(e, _) => self.expr(e),
            OrDisposition::Raise(_) | OrDisposition::Discard(_) | OrDisposition::Wait(_) => true,
        }
    }

    fn expr(&mut self, e: &Expr) -> bool {
        match e {
            Expr::Ident(i) => self.declared(&i.name) || (self.outer)(&i.name) != Some(OuterLocal::Buffer),
            Expr::Literal(..) | Expr::Path(_) | Expr::KwSelf(_) => true,
            Expr::Binary { left, right, .. } => self.expr(left) && self.expr(right),
            Expr::Unary { operand, .. } => self.expr(operand),
            Expr::Call { callee, args, .. } => {
                // A `bounded` intrinsic that writes copies its element into
                // the current arena: a write into what the root holds.
                let writes_into = match (callee.as_ref(), args.first()) {
                    (Expr::Ident(f), Some(target)) if BOUNDED_WRITES.contains(&f.name.as_str()) => root_ident(target),
                    _ => None,
                };
                // A call that may write into its arguments is handed none
                // of the storage held outside the body.
                let hands_outside = !self.call_owns_its_stores(callee) && args.iter().any(|a| self.mentions_outside(a));
                writes_into.is_none_or(|root| self.writable(root))
                    && !hands_outside
                    && !(self.produces_locus)(e)
                    && self.expr(callee)
                    && args.iter().all(|a| self.expr(a))
            }
            Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. } => self.expr(receiver),
            Expr::Index { receiver, index, .. } => self.expr(receiver) && self.expr(index),
            Expr::Tuple(xs, _) | Expr::Array(xs, _) => xs.iter().all(|x| self.expr(x)),
            Expr::Struct { inits, .. } => !(self.produces_locus)(e) && inits.iter().all(|i| self.expr(&i.value)),
            Expr::Block(b) => self.block(b),
            Expr::If(i) => self.if_stmt(i),
            Expr::Match(m) => self.match_stmt(m),
            Expr::Sum(x, _) | Expr::Prod(x, _) => self.expr(x),
            Expr::Approx { left, right, tolerance, .. } => self.expr(left) && self.expr(right) && self.expr(tolerance),
            Expr::Range { lo, hi, .. } => self.expr(lo) && self.expr(hi),
            Expr::ArrayRepeat { val, .. } => self.expr(val),
            Expr::Or { inner, disposition, .. } => self.expr(inner) && self.disposition(disposition),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body_of(src: &str) -> Block {
        let program = hale_syntax::parse_source(src).expect("parses");
        for item in &program.items {
            if let TopDecl::Fn(f) = item {
                for s in &f.body.stmts {
                    if let Stmt::While { body, .. } = s {
                        return body.clone();
                    }
                }
            }
        }
        panic!("no while loop in the fn");
    }

    fn outer(name: &str) -> Option<OuterLocal> {
        match name {
            "i" | "total" => Some(OuterLocal::Scalar),
            "h" | "buf" => Some(OuterLocal::Locus),
            "last" => Some(OuterLocal::Heap),
            "ring" => Some(OuterLocal::Buffer),
            _ => None,
        }
    }

    fn reclaims(src: &str) -> bool {
        reclaims_per_iteration(&body_of(src), &outer, &|_| false)
    }

    #[test]
    fn scalar_counters_and_locus_calls_reclaim() {
        assert!(reclaims(
            "fn f() { while true { let c = buf.snapshot() or { break; }; h.update(c); total = total + len(c); i += 1; } }"
        ));
    }

    #[test]
    fn a_heap_binding_from_outside_refuses() {
        assert!(!reclaims("fn f() { while true { let c = buf.snapshot() or { break; }; last = c; } }"));
    }

    #[test]
    fn a_shadowing_let_reclaims() {
        assert!(reclaims("fn f() { while true { let last = buf.snapshot() or { break; }; last = b\"\"; } }"));
    }

    #[test]
    fn a_buffer_named_refuses() {
        assert!(!reclaims("fn f() { while true { ring.push(1); } }"));
    }

    #[test]
    fn a_bounded_write_into_an_outside_record_refuses() {
        assert!(!reclaims("fn f() { while true { push(last.items, 1) or discard; } }"));
        assert!(reclaims("fn f() { while true { push(self.items, 1) or discard; } }"));
        assert!(reclaims("fn f() { while true { let kept = T { }; push(kept.items, 1) or discard; } }"));
    }

    /// The review's counterexample: a free fn handed an outside record
    /// writes into it, from the caller's arena, the iteration's.
    #[test]
    fn an_outside_record_handed_to_a_free_fn_refuses() {
        assert!(!reclaims("fn f() { while true { let p = buf.snapshot() or b\"\"; keep(last, p); } }"));
        assert!(!reclaims("fn f() { while true { let p = buf.snapshot() or b\"\"; keep(last.items, p); } }"));
        assert!(!reclaims("fn f() { while true { while true { let p = buf.snapshot() or b\"\"; keep(last, p); } } }"));
        // through an alias declared in the body, or a binder over it
        assert!(!reclaims("fn f() { while true { let o = last; let p = buf.snapshot() or b\"\"; keep(o, p); } }"));
        assert!(!reclaims("fn f() { while true { let o = b\"\"; o = last; keep(o, b\"x\"); } }"));
        assert!(!reclaims("fn f() { while true { for r in last.items { keep(r, b\"x\"); } } }"));
        assert!(!reclaims("fn f() { while true { let o = last; o.n = 1; } }"));
        // a locus's method, a reading builtin, and the body's own values stay admitted
        assert!(reclaims("fn f() { while true { h.update(last); total = total + len(last); println(last); } }"));
        assert!(reclaims("fn f() { while true { let p = buf.snapshot() or b\"\"; keep(p, p); } }"));
    }

    #[test]
    fn an_unknown_assigned_name_refuses() {
        assert!(!reclaims("fn f() { while true { nobody = 1; } }"));
    }
}

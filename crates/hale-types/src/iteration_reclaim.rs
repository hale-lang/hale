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
//! - hands a call anything but what the call cannot write through:
//!   unless the callee is a builtin that only reads (`len`, `println`,
//!   `to_string`, …, names no fn can take), every argument has to be
//!   the iteration's own value or a locus reference passed whole. Each
//!   value is one of three ([`Held`]): *owned* (a literal, a scalar, a
//!   value this iteration made, a path into one), *whole* (`self`, an
//!   outside locus reference, a body binding of one: passed whole it is
//!   the locus, whose stores copy into its own arena, but `alias.bag` is
//!   that locus's storage), or *outside* (an outside record, any path
//!   through a locus or an outside value, an element of an outside
//!   array, a match binder, a call's result when an argument was not
//!   owned). A callee handed outside storage could write into it (a
//!   free fn through its parameter, whatever its name; a fn value held
//!   in a field, `self.callback(outer, p)`; a method through a record
//!   parameter), and what it writes is made in its caller arena, the
//!   iteration's. The downloader's calls (`body.chunk()`,
//!   `h.update(c)`, `out.write_bytes(c)`, `buf.snapshot()`) are
//!   admitted;
//! - names an outside binding whose value grows in the frame's arena when
//!   used (a `bounded[T; N]` or a `BytesMut` itself);
//! - makes a locus (a literal or a factory call): its instance, its slot
//!   and the GH #815 per-iteration teardown live past the iteration;
//! - holds a statement whose teardown runs outside the body's view
//!   (`reperspective`, a recovery op, `violate`, a shared-memory write).
//!
//! Names are resolved lexically: a `let` (or a `for` binder) inside the
//! body shadows the outside binding from there on, and holds what its
//! value is: a `let`, a tuple `let` and a `for` binder (an element of
//! what is iterated) take it when declared, an assignment joins it in
//! (`o = outer` makes `o` outside from then on, in every iteration), and
//! a carrier, an `or` and a container literal are the join of their arms
//! and parts. A match-arm binder is not declared here, so an assignment
//! to one, and passing one to a call, refuse the body: the conservative
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

/// The builtins that only read their arguments, by names no fn can take:
/// the parser refuses a `fn` named by a built-in call form
/// (`hale_syntax::parser::builtin_call_form`: `to_string`, `min`, `max`,
/// `abs`, `starts_with`, `contains`, the printers), and lowering refuses
/// one named `len`, `Int` or `Float` (GH #880). The bounded intrinsics
/// (`push`, `set`, …) are not here: a user fn may take those names.
fn read_only_builtin(name: &str) -> bool {
    matches!(name, "len" | "Int" | "Float")
        || hale_syntax::parser::builtin_call_form(name).is_some_and(|(n, _)| n != "sum" && n != "prod")
}

/// What a value is, for handing it to a call: two facts, whether it may
/// be passed whole and whether the storage its paths reach is the
/// iteration's own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Held {
    /// Made by this iteration (or a scalar): passed whole, and every path
    /// into it reaches the iteration's own storage.
    Owned,
    /// A locus reference (`self`, an outside locus, a body binding of
    /// one, a value that may hold one): passed whole it is that locus,
    /// whose stores copy into its own arena, but a path through it
    /// (`alias.bag`) is that locus's storage, outside the iteration.
    Whole,
    /// Storage from outside the iteration, or what may be: not handed to
    /// a call that could write into it, and not a path either.
    Outside,
}

impl Held {
    /// A value that may be any of two (a carrier's arms, an `or`'s
    /// sides, a container's parts, a binding over the iterations).
    fn join(self, other: Held) -> Held {
        self.max(other)
    }

    /// What a field or an element of a value of this kind is.
    fn path(self) -> Held {
        match self {
            Held::Owned => Held::Owned,
            Held::Whole | Held::Outside => Held::Outside,
        }
    }
}

struct Walk<'a> {
    outer: &'a dyn Fn(&str) -> Option<OuterLocal>,
    produces_locus: &'a dyn Fn(&Expr) -> bool,
    /// The body's bindings, innermost scope last, with what each holds.
    scopes: Vec<BTreeMap<String, Held>>,
}

impl Walk<'_> {
    /// A binding of the body: what it holds.
    fn local(&self, name: &str) -> Option<Held> {
        self.scopes.iter().rev().find_map(|s| s.get(name).copied())
    }

    fn declared(&self, name: &str) -> bool {
        self.local(name).is_some()
    }

    fn declare(&mut self, name: &str, held: Held) {
        if let Some(s) = self.scopes.last_mut() {
            s.insert(name.to_string(), held);
        }
    }

    /// A body binding assigned a value of kind `held`: it holds either,
    /// whichever iteration or branch ran last.
    fn assigned(&mut self, name: &str, held: Held) {
        if let Some(s) = self.scopes.iter_mut().rev().find(|s| s.contains_key(name)) {
            let now = s[name].join(held);
            s.insert(name.to_string(), now);
        }
    }

    /// What `e` is, for handing it to a call ([`Held`]):
    ///
    /// - a literal, a scalar binding: owned;
    /// - `self`, an outside locus reference, a path (a constant, a fn):
    ///   whole only;
    /// - a body binding: what it was bound or assigned;
    /// - a field or element: owned through an owned value, else outside;
    /// - a call's result: owned when every argument is (a method returns
    ///   a copy in its caller's arena; a free fn returns what it was
    ///   handed, made fresh, or a path of it), else outside (a free fn
    ///   handed a locus may return a field of it);
    /// - an operator, a literal of a container, an `or`, a range, a
    ///   carrier: the join of its parts or arms (an arm with no tail
    ///   diverges or is unit, `or { break; }`);
    /// - anything else (an unknown name, a match binder): outside.
    fn held(&self, e: &Expr) -> Held {
        match e {
            Expr::Literal(..) => Held::Owned,
            Expr::Path(_) | Expr::KwSelf(_) => Held::Whole,
            Expr::Ident(i) => match self.local(&i.name) {
                Some(h) => h,
                None => match (self.outer)(&i.name) {
                    Some(OuterLocal::Scalar) => Held::Owned,
                    Some(OuterLocal::Locus) => Held::Whole,
                    _ => Held::Outside,
                },
            },
            Expr::Call { args, .. } => {
                if args.iter().all(|a| self.held(a) == Held::Owned) {
                    Held::Owned
                } else {
                    Held::Outside
                }
            }
            Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. } => self.held(receiver).path(),
            Expr::Index { receiver, index, .. } => self.held(receiver).path().join(self.held(index)),
            Expr::Binary { left, right, .. } => self.held(left).join(self.held(right)),
            Expr::Unary { operand, .. } => self.held(operand),
            Expr::Tuple(xs, _) | Expr::Array(xs, _) => xs.iter().fold(Held::Owned, |h, x| h.join(self.held(x))),
            Expr::Struct { inits, .. } => inits.iter().fold(Held::Owned, |h, i| h.join(self.held(&i.value))),
            Expr::ArrayRepeat { val, .. } => self.held(val),
            Expr::Range { lo, hi, .. } => self.held(lo).join(self.held(hi)),
            Expr::Or { inner, disposition, .. } => self.held(inner).join(match disposition {
                OrDisposition::Substitute(x) => self.held(x),
                OrDisposition::Raise(_) | OrDisposition::Discard(_) | OrDisposition::Wait(_) | OrDisposition::Fail(..) => {
                    Held::Owned
                }
            }),
            Expr::Block(b) => self.block_held(b),
            Expr::If(i) => self.if_held(i),
            Expr::Match(m) => m.arms.iter().fold(Held::Owned, |h, a| {
                h.join(match &a.body {
                    MatchArmBody::Expr(x) => self.held(x),
                    MatchArmBody::Block(b) => self.block_held(b),
                })
            }),
            Expr::Sum(..) | Expr::Prod(..) | Expr::Approx { .. } => Held::Outside,
        }
    }

    fn block_held(&self, b: &Block) -> Held {
        b.tail.as_deref().map_or(Held::Owned, |t| self.held(t))
    }

    fn if_held(&self, i: &IfStmt) -> Held {
        self.block_held(&i.then_block).join(match i.else_block.as_deref() {
            None => Held::Owned,
            Some(ElseBranch::Else(b)) => self.block_held(b),
            Some(ElseBranch::ElseIf(e)) => self.if_held(e),
        })
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
                let held = self.held(value);
                self.declare(&name.name, held);
                ok
            }
            Stmt::LetTuple { names, value, .. } => {
                let ok = self.expr(value);
                let held = self.held(value);
                for n in names {
                    self.declare(&n.name, held);
                }
                ok
            }
            Stmt::Assign { target, value, .. } => {
                let ok = self.assign_target(target) && self.expr(value);
                if ok && target.tail.is_empty() {
                    let held = self.held(value);
                    self.assigned(&target.head.name, held);
                }
                ok
            }
            Stmt::If(i) => self.if_stmt(i),
            Stmt::Match(m) => self.match_stmt(m),
            Stmt::For { name, iter, body, .. } => {
                if !self.expr(iter) {
                    return false;
                }
                // A binder is an element of what is iterated.
                let held = self.held(iter).path();
                self.scopes.push(BTreeMap::from([(name.name.clone(), held)]));
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
                // A store through a locus reference copies into it.
                Some(held) => held != Held::Outside,
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
                // A call that may write into its arguments is handed only
                // what it cannot write into outside the iteration.
                let reads_only = matches!(callee.as_ref(), Expr::Ident(f)
                    if !self.declared(&f.name) && (self.outer)(&f.name).is_none() && read_only_builtin(&f.name));
                (reads_only || args.iter().all(|a| self.held(a) != Held::Outside))
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
        // no exemption by name: a fn may be called `push`
        assert!(!reclaims("fn f() { while true { push(self.items, 1) or discard; } }"));
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
        // a fn value in a field, a method handed an outside record, a field of `self`
        assert!(!reclaims("fn f() { while true { let p = buf.snapshot() or b\"\"; self.callback(last, p); } }"));
        assert!(!reclaims("fn f() { while true { let p = buf.snapshot() or b\"\"; h.put(last, p); } }"));
        assert!(!reclaims("fn f() { while true { let p = buf.snapshot() or b\"\"; keep(self.bag, p); } }"));
        assert!(!reclaims("fn f() { while true { let o = self.bag; keep(o, b\"x\"); } }"));
        // a reading builtin, the body's own values and locus references stay admitted
        assert!(reclaims("fn f() { while true { total = total + len(last); println(last); } }"));
        assert!(reclaims("fn f() { while true { let p = self.buf.snapshot() or b\"\"; h.update(p); keep(p, p); self.s.put(p); } }"));
        assert!(!reclaims("fn f() { while true { push(self.items, b\"x\") or discard; } }"));
    }

    /// A body binding of a locus reference is passed whole, never a path
    /// through it: each way of binding one.
    #[test]
    fn a_body_binding_of_a_locus_is_whole_not_a_path() {
        // `let` of `self` and of an outside locus, then a path through it
        assert!(!reclaims("fn f() { while true { let a = self; keep(a.bag, b\"x\"); } }"));
        assert!(!reclaims("fn f() { while true { let a = h; keep(a.bag, b\"x\"); keep(a.bag.items, b\"x\"); } }"));
        // passed whole it is admitted
        assert!(reclaims("fn f() { while true { let a = self; let b = h; keep(a, b\"x\"); keep(b, i); } }"));
        // an assignment joins: an owned binding assigned a locus, or outside storage
        assert!(!reclaims("fn f() { while true { let a = T { }; a = h; keep(a.bag, b\"x\"); } }"));
        assert!(!reclaims("fn f() { while true { let a = T { }; a = last; keep(a, b\"x\"); } }"));
        // a carrier's arms, an `or`'s sides, a match's arms
        assert!(!reclaims("fn f() { while true { let a = if i > 0 { self } else { h }; keep(a.bag, b\"x\"); } }"));
        assert!(!reclaims("fn f() { while true { let a = { h }; keep(a.bag, b\"x\"); } }"));
        assert!(!reclaims("fn f() { while true { let a = get() or h; keep(a.bag, b\"x\"); } }"));
        assert!(!reclaims("fn f() { while true { let a = match i { 0 -> h, _ -> self, }; keep(a.bag, b\"x\"); } }"));
        // a container holding a locus, a tuple `let`, a `for` binder
        assert!(!reclaims("fn f() { while true { let w = W { h: h }; keep(w.h.bag, b\"x\"); } }"));
        assert!(!reclaims("fn f() { while true { let (a, b) = (h, 1); keep(a.bag, b\"x\"); } }"));
        assert!(!reclaims("fn f() { while true { for a in self.children { keep(a, b\"x\"); } } }"));
        // a call's result when it was handed a locus (it may be a field of it)
        assert!(!reclaims("fn f() { while true { let a = field_of(h); keep(a, b\"x\"); } }"));
        assert!(reclaims("fn f() { while true { let a = make(i); keep(a.items, b\"x\"); } }"));
    }

    /// The audit: each way a call can be handed storage that outlives the
    /// iteration, refused; each way of handing it only what the iteration
    /// made or a locus reference, admitted.
    #[test]
    fn only_what_the_iteration_made_is_handed_to_a_call() {
        // a user fn named `push` handed a field of `self`; a field of
        // another locus; an element of an outside array or tuple; a match
        // binder over outside storage; an `if` value naming outside
        // storage; a method of a locus handed a field of `self`
        assert!(!reclaims("fn f() { while true { let p = buf.snapshot() or b\"\"; push(self.bag, p); } }"));
        assert!(!reclaims("fn f() { while true { let p = buf.snapshot() or b\"\"; keep(h.bag, p); keep(h.bag.items, p); } }"));
        assert!(!reclaims("fn f() { while true { keep(last[0], b\"x\"); } }"));
        assert!(!reclaims("fn f() { while true { keep(last.0, b\"x\"); } }"));
        assert!(!reclaims("fn f() { while true { match last { Some(v) -> { keep(v, b\"x\"); }, _ -> { }, } } }"));
        assert!(!reclaims("fn f() { while true { let o = if total > 0 { last } else { b\"\" }; keep(o, b\"x\"); } }"));
        assert!(!reclaims("fn f() { while true { h.put(self.x, b\"x\"); } }"));
        // `self` and an outside locus passed whole, a scalar, the
        // iteration's own values and paths into them, a call's result
        assert!(reclaims("fn f() { while true { keep(self, b\"x\"); keep(h, i); } }"));
        assert!(reclaims("fn f() { while true { let r = T { n: i }; keep(r, r.n); keep(r.items, buf.snapshot() or b\"\"); } }"));
        assert!(reclaims("fn f() { while true { let c = body.chunk() or { break; }; h.update(c); out.write_bytes(c) or { break; }; } }"));
    }

    #[test]
    fn an_unknown_assigned_name_refuses() {
        assert!(!reclaims("fn f() { while true { nobody = 1; } }"));
    }
}

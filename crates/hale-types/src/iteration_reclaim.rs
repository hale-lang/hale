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

/// Where a use resolves: the snapshot's answer (`Snapshot::binding_of`,
/// resolved once by the mint under lexical scoping) for an identifier
/// expression's id, or an assignment statement's id for the binding its
/// head writes: the declaration's site index and its span. `None` for a
/// use that names no local binding (a field, a fn, a constant) and for a
/// node the mint did not number.
pub type DeclOf<'a> = dyn Fn(NodeId) -> Option<(u32, hale_syntax::Span)> + 'a;

/// [`DeclOf`] over a snapshot: its resolution of the uses it minted.
pub fn decl_of(ids: &crate::snapshot::Snapshot) -> impl Fn(NodeId) -> Option<(u32, hale_syntax::Span)> + '_ {
    move |node| {
        let d = ids.declaration_of(node)?;
        ids.site(d).map(|site| (d.index, site.span))
    }
}

/// Does `body` reclaim per iteration? `outer` classifies a name bound
/// outside the body (`None`: no such local, e.g. a field or a constant
/// spelled without `self`, which an assignment then refuses on); it is
/// asked only of a use the snapshot resolves to a declaration outside the
/// body. `produces_locus` says whether a struct literal or call makes a
/// locus; `decl_of` is where each use resolves ([`DeclOf`]).
pub fn reclaims_per_iteration(
    body: &Block,
    outer: &dyn Fn(&str) -> Option<OuterLocal>,
    produces_locus: &dyn Fn(&Expr) -> bool,
    decl_of: &DeclOf<'_>,
) -> bool {
    let mut w = Walk { outer, produces_locus, decl_of, span: body.span, kinds: BTreeMap::new() };
    w.block(body).is_some()
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
    decl_of: &'a DeclOf<'a>,
    /// The body's span: a declaration inside it is the body's own.
    span: hale_syntax::Span,
    /// What each of the body's declarations holds, by its site.
    kinds: BTreeMap<u32, Held>,
}

/// Where a use stands, by the snapshot's resolution.
enum Resolved {
    /// A declaration of the body, with what it holds; a body declaration
    /// the walk installed no kind for (a binder it does not read) is
    /// outside.
    Body(Held),
    /// A declaration before the body: the reader classifies it by name.
    Outer,
    /// No local binding (a fn, a constant, a topic) or an unnumbered use.
    Unresolved,
}

impl Walk<'_> {
    fn contains(&self, at: hale_syntax::Span) -> bool {
        self.span.start.0 <= at.start.0 && at.end.0 <= self.span.end.0
    }

    fn resolve(&self, node: NodeId) -> Resolved {
        match (self.decl_of)(node) {
            None => Resolved::Unresolved,
            Some((d, at)) if self.contains(at) => Resolved::Body(self.kinds.get(&d).copied().unwrap_or(Held::Outside)),
            Some(_) => Resolved::Outer,
        }
    }

    /// Install a declaration of the body (its site is its node's id).
    fn declare(&mut self, node: NodeId, held: Held) {
        if !node.is_none() {
            self.kinds.insert(node.0, held);
        }
    }

    /// A pattern's binders, of a value of kind `held`: a binding of the
    /// whole value is that value; one inside a constructor or a tuple is
    /// a part of it.
    fn bind_pattern(&mut self, p: &Pattern, held: Held) {
        match p {
            Pattern::Binding(i) => self.declare(i.id, held),
            Pattern::Constructor { args, .. } | Pattern::Tuple(args, _) => {
                for a in args {
                    self.bind_pattern(a, held.path());
                }
            }
            Pattern::Literal(..) | Pattern::Wildcard(_) => {}
        }
    }

    /// A block, walked in its own scope: its value's kind (its tail's,
    /// computed while its bindings are live; owned when it has none, as an
    /// arm that diverges or is unit, `or { break; }`), or `None` when it
    /// refuses the body.
    fn block(&mut self, b: &Block) -> Option<Held> {
        if !b.stmts.iter().all(|s| self.stmt(s)) {
            return None;
        }
        match b.tail.as_deref() {
            Some(t) => self.expr(t),
            None => Some(Held::Owned),
        }
    }

    fn stmt(&mut self, s: &Stmt) -> bool {
        match s {
            Stmt::Let { value, id, .. } => match self.expr(value) {
                Some(held) => {
                    self.declare(*id, held);
                    true
                }
                None => false,
            },
            Stmt::LetTuple { names, value, .. } => match self.expr(value) {
                Some(held) => {
                    for n in names {
                        self.declare(n.id, held);
                    }
                    true
                }
                None => false,
            },
            Stmt::Assign { target, value, id, .. } => {
                if !self.assign_target(target, *id) {
                    return false;
                }
                let Some(held) = self.expr(value) else { return false };
                // A body binding assigned a value holds either, whichever
                // iteration or branch ran last.
                if target.tail.is_empty() {
                    if let Some((d, at)) = (self.decl_of)(*id) {
                        if self.contains(at) {
                            let now = self.kinds.get(&d).copied().unwrap_or(Held::Outside).join(held);
                            self.kinds.insert(d, now);
                        }
                    }
                }
                true
            }
            Stmt::If(i) => self.if_expr(i).is_some(),
            Stmt::Match(m) => self.match_expr(m).is_some(),
            Stmt::For { iter, body, id, .. } => {
                // A binder is an element of what is iterated.
                let Some(held) = self.expr(iter) else { return false };
                self.declare(*id, held.path());
                self.block(body).is_some()
            }
            Stmt::While { cond, body, .. } => self.expr(cond).is_some() && self.block(body).is_some(),
            Stmt::Return(e, _) => e.as_ref().is_none_or(|e| self.expr(e).is_some()),
            Stmt::Fail { value, .. } => self.expr(value).is_some(),
            Stmt::Send { subject, value, or_disposition, .. } => {
                self.expr(subject).is_some()
                    && self.expr(value).is_some()
                    && or_disposition.as_ref().is_none_or(|d| self.disposition(d).is_some())
            }
            Stmt::Block(b) => self.block(b).is_some(),
            Stmt::Expr(e) => self.expr(e).is_some(),
            Stmt::Break(_) | Stmt::Continue(_) | Stmt::Yield(_) | Stmt::Terminate(_) => true,
            Stmt::Reperspective { .. } | Stmt::Recovery { .. } | Stmt::Violate { .. } | Stmt::ShmWrite { .. } => false,
        }
    }

    /// May `head` be written from the body: `self`, a binding the body
    /// declares, or an outside one holding no arena memory of its own.
    fn writable(&self, head: &str, assign: NodeId) -> bool {
        head == "self"
            || match self.resolve(assign) {
                // A store through a locus reference copies into it.
                Resolved::Body(held) => held != Held::Outside,
                Resolved::Outer => matches!((self.outer)(head), Some(OuterLocal::Scalar | OuterLocal::Locus)),
                Resolved::Unresolved => false,
            }
    }

    fn assign_target(&mut self, target: &LValue, assign: NodeId) -> bool {
        self.writable(target.head.name.as_str(), assign)
            && target.tail.iter().all(|seg| match seg {
                LValueSeg::Field(_) => true,
                LValueSeg::Index(e) => self.expr(e).is_some(),
            })
    }

    /// An `if`, statement or value: the join of its arms' kinds.
    fn if_expr(&mut self, i: &IfStmt) -> Option<Held> {
        self.expr(&i.cond)?;
        let then = self.block(&i.then_block)?;
        let other = match i.else_block.as_deref() {
            None => Held::Owned,
            Some(ElseBranch::Else(b)) => self.block(b)?,
            Some(ElseBranch::ElseIf(e)) => self.if_expr(e)?,
        };
        Some(then.join(other))
    }

    /// A `match`, statement or value: the join of its arms' kinds.
    fn match_expr(&mut self, m: &MatchStmt) -> Option<Held> {
        let scrutinee = self.expr(&m.scrutinee)?;
        let mut held = Held::Owned;
        for arm in &m.arms {
            self.bind_pattern(&arm.pattern, scrutinee);
            if let Some(g) = &arm.guard {
                self.expr(g)?;
            }
            let arm_held = match &arm.body {
                MatchArmBody::Expr(e) => self.expr(e)?,
                MatchArmBody::Block(b) => self.block(b)?,
            };
            held = held.join(arm_held);
        }
        Some(held)
    }

    /// An `or`'s other side: the substitute's kind (a disposition that
    /// diverges or discards adds none).
    fn disposition(&mut self, d: &OrDisposition) -> Option<Held> {
        match d {
            OrDisposition::Substitute(e) => self.expr(e),
            OrDisposition::Fail(e, _) => self.expr(e).map(|_| Held::Owned),
            OrDisposition::Raise(_) | OrDisposition::Discard(_) | OrDisposition::Wait(_) => Some(Held::Owned),
        }
    }

    /// Walk `e` with the body's scopes as they stand where it is, and
    /// answer what its value is ([`Held`]), or `None` when it refuses the
    /// body. The one place a kind is computed:
    ///
    /// - a literal, a scalar binding: owned;
    /// - `self`, an outside locus reference, a path (a constant, a fn):
    ///   whole;
    /// - a body binding: what it was bound or assigned;
    /// - a field or element: owned through an owned value, else outside;
    /// - a call's result: owned when every argument is (a method returns
    ///   a copy in its caller's arena; a free fn returns what it was
    ///   handed, made fresh, or a path of it), else outside (a free fn
    ///   handed a locus may return a field of it);
    /// - an operator, a container literal, an `or`, a range, a carrier:
    ///   the join of its parts or arms;
    /// - anything else (an unknown name, a match binder): outside.
    ///
    /// A call (but a builtin that only reads) handed an outside argument,
    /// a locus made, or an outside buffer named, refuses the body.
    fn expr(&mut self, e: &Expr) -> Option<Held> {
        let join_all = |w: &mut Self, xs: &mut dyn Iterator<Item = &Expr>| -> Option<Held> {
            let mut held = Held::Owned;
            for x in xs {
                held = held.join(w.expr(x)?);
            }
            Some(held)
        };
        match e {
            Expr::Literal(..) => Some(Held::Owned),
            Expr::Path(_) | Expr::KwSelf(_) => Some(Held::Whole),
            Expr::Ident(i) => match self.resolve(i.id) {
                Resolved::Body(held) => Some(held),
                Resolved::Outer => match (self.outer)(&i.name) {
                    Some(OuterLocal::Buffer) => None,
                    Some(OuterLocal::Scalar) => Some(Held::Owned),
                    Some(OuterLocal::Locus) => Some(Held::Whole),
                    Some(OuterLocal::Heap) | None => Some(Held::Outside),
                },
                Resolved::Unresolved => Some(Held::Outside),
            },
            Expr::Binary { left, right, .. } => Some(self.expr(left)?.join(self.expr(right)?)),
            Expr::Unary { operand, .. } => self.expr(operand),
            Expr::Call { callee, args, .. } => {
                if (self.produces_locus)(e) {
                    return None;
                }
                let reads_only = matches!(callee.as_ref(), Expr::Ident(f)
                    if matches!(self.resolve(f.id), Resolved::Unresolved) && read_only_builtin(&f.name));
                if !reads_only {
                    self.expr(callee)?;
                }
                let mut all_owned = true;
                for a in args {
                    let held = self.expr(a)?;
                    // A call that may write into its arguments is handed
                    // only what it cannot write into outside the iteration.
                    if held == Held::Outside && !reads_only {
                        return None;
                    }
                    all_owned &= held == Held::Owned;
                }
                Some(if all_owned { Held::Owned } else { Held::Outside })
            }
            Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. } => Some(self.expr(receiver)?.path()),
            Expr::Index { receiver, index, .. } => Some(self.expr(receiver)?.path().join(self.expr(index)?)),
            Expr::Tuple(xs, _) | Expr::Array(xs, _) => join_all(self, &mut xs.iter()),
            Expr::Struct { inits, .. } => {
                if (self.produces_locus)(e) {
                    return None;
                }
                join_all(self, &mut inits.iter().map(|i| &i.value))
            }
            Expr::Block(b) => self.block(b),
            Expr::If(i) => self.if_expr(i),
            Expr::Match(m) => self.match_expr(m),
            Expr::Sum(x, _) | Expr::Prod(x, _) => self.expr(x).map(|_| Held::Outside),
            Expr::Approx { left, right, tolerance, .. } => {
                self.expr(left)?;
                self.expr(right)?;
                self.expr(tolerance)?;
                Some(Held::Outside)
            }
            Expr::Range { lo, hi, .. } => Some(self.expr(lo)?.join(self.expr(hi)?)),
            Expr::ArrayRepeat { val, .. } => self.expr(val),
            Expr::Or { inner, disposition, .. } => Some(self.expr(inner)?.join(self.disposition(disposition)?)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The names the test bodies use from outside the loop, declared as
    /// the fn's parameters so the mint resolves them (an undeclared name is
    /// unresolved, and outside).
    const OUTER_PARAMS: &str =
        "fn f(i: Int, total: Int, n: Int, h: H, buf: H, body: H, out: H, last: L, ring: R)";

    /// Judge the fn's first `while` body of `src` (whose `fn f()` takes
    /// [`OUTER_PARAMS`]), resolved by the mint as lowering's is.
    fn judge(src: &str, outer: &dyn Fn(&str) -> Option<OuterLocal>) -> bool {
        let src = src.replacen("fn f()", OUTER_PARAMS, 1);
        let (program, ids) = crate::alloc_summary::tests::minted(&src);
        let decl_of = decl_of(&ids);
        for item in &program.items {
            if let TopDecl::Fn(f) = item {
                for s in &f.body.stmts {
                    if let Stmt::While { body, .. } = s {
                        return reclaims_per_iteration(body, outer, &|_| false, &decl_of);
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
        judge(src, &outer)
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

    /// Every binder form × every source, the kind each must give the
    /// binding `a`, checked through both uses: handing `a` whole (refused
    /// only when outside) and handing a path through it (admitted only
    /// when owned). `n` is an outside scalar; the "shadowed" source rebinds
    /// it to a locus inside the body, and the forms marked shadowing bind
    /// `n` itself, so a use resolved by spelling would find the outside
    /// scalar. Each form is `(name, template, what a is of X)`: `X` is the
    /// source, `U` the use of `a`.
    #[test]
    fn every_binder_form_gives_its_source_kind() {
        #[derive(Clone, Copy)]
        enum Of {
            /// `a` is X itself.
            Whole,
            /// `a` is a part of X (an element, a field, a pattern's part).
            Part,
            /// `a` is no binding the walk installs (an `or`'s `err`).
            Unbound,
        }
        let outer = |name: &str| match name {
            "n" | "i" => Some(OuterLocal::Scalar),
            "h" | "buf" => Some(OuterLocal::Locus),
            "last" => Some(OuterLocal::Heap),
            _ => None,
        };
        let sources: [(&str, &str, &str, Held); 6] = [
            ("self", "", "self", Held::Whole),
            ("outer locus", "", "h", Held::Whole),
            ("outer record", "", "last", Held::Outside),
            ("outer scalar", "", "n", Held::Owned),
            ("scalar shadowed by a locus", "let n = h; ", "n", Held::Whole),
            ("fresh value", "", "T { k: 1 }", Held::Owned),
        ];
        let forms: [(&str, &str, Of); 17] = [
            ("let", "let a = X; U", Of::Whole),
            ("let shadowing", "let n = X; let a = n; U", Of::Whole),
            ("block tail", "let a = { X }; U", Of::Whole),
            ("block tail shadowing", "let a = { let n = X; n }; U", Of::Whole),
            ("if", "let a = if i > 0 { X } else { X }; U", Of::Whole),
            ("match value", "let a = match i { 0 -> X, _ -> X, }; U", Of::Whole),
            ("or", "let a = buf.snapshot() or X; U", Of::Whole),
            ("tuple let", "let (a, z) = (X, 1); U", Of::Whole),
            ("assignment", "let a = 1; a = X; U", Of::Whole),
            ("container", "let w = W { f: X }; let a = w.f; U", Of::Part),
            ("for", "for a in [X] { U }", Of::Part),
            ("match binder", "match X { a -> { U } }", Of::Whole),
            ("match binder shadowing", "match X { n -> { let a = n; U } }", Of::Whole),
            ("match constructor binder", "match X { Some(a) -> { U }, _ -> { }, }", Of::Part),
            ("match tuple binder", "match (X, 1) { (a, z) -> { U } }", Of::Part),
            ("match nested binder", "match X { Some((a, z)) -> { U }, _ -> { }, }", Of::Part),
            ("or error binder", "let c = buf.snapshot() or { let a = err; U b\"\" };", Of::Unbound),
        ];
        let mut checked = 0;
        for (sname, pre, x, kind) in sources {
            for (fname, form, of) in forms {
                let kind = match of {
                    Of::Whole => kind,
                    Of::Part => kind.path(),
                    Of::Unbound => Held::Outside,
                };
                for (use_, admitted) in [("keep(a, b\"x\");", kind != Held::Outside), ("keep(a.bag, b\"x\");", kind == Held::Owned)] {
                    let body = form.replace('X', x).replace('U', use_);
                    let src = format!("fn f() {{ while true {{ {pre}{body} }} }}");
                    assert_eq!(judge(&src, &outer), admitted, "{fname} of {sname} ({kind:?}), `{use_}`: {src}");
                    checked += 1;
                }
            }
        }
        assert_eq!(checked, 6 * 17 * 2);
    }

    /// Every construct of the grammar that binds a name in a body, by the
    /// snapshot's own walk (`resolve_uses`): each either has its kind
    /// installed by the walk or is judged outside. The list is the AST's
    /// binder positions; a new one (a `Stmt` or `Pattern` variant that
    /// binds) has to be added here and to the walk, or this fails to
    /// compile (the matches are exhaustive).
    #[test]
    fn every_binder_construct_is_installed_or_outside() {
        fn binds(s: &Stmt) -> &'static str {
            match s {
                Stmt::Let { .. } => "installed: the value's kind",
                Stmt::LetTuple { .. } => "installed: the value's kind, each name",
                Stmt::For { .. } => "installed: an element of the iterated value",
                Stmt::Match(_) => "installed: each pattern binder, the scrutinee or a part",
                Stmt::ShmWrite { .. } => "refused: the body is not reclaimed",
                Stmt::Assign { .. }
                | Stmt::If(_)
                | Stmt::While { .. }
                | Stmt::Return(..)
                | Stmt::Break(_)
                | Stmt::Continue(_)
                | Stmt::Fail { .. }
                | Stmt::Yield(_)
                | Stmt::Terminate(_)
                | Stmt::Reperspective { .. }
                | Stmt::Block(_)
                | Stmt::Recovery { .. }
                | Stmt::Violate { .. }
                | Stmt::Send { .. }
                | Stmt::Expr(_) => "binds nothing",
            }
        }
        fn pattern_binds(p: &Pattern) -> &'static str {
            match p {
                Pattern::Binding(_) => "installed",
                Pattern::Constructor { .. } | Pattern::Tuple(..) => "its parts, installed",
                Pattern::Literal(..) | Pattern::Wildcard(_) => "binds nothing",
            }
        }
        // `or`'s `err` resolves to no declaration (the mint gives it none),
        // so every use of it is unresolved, and outside; an `Expr::Match`
        // binds as `Stmt::Match` does; fn parameters are outside every body.
        let _ = (binds, pattern_binds);
        let outer = |_: &str| None;
        assert!(!judge("fn f() { while true { let c = buf.snapshot() or { keep(err, b\"x\"); b\"\" }; } }", &outer));
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

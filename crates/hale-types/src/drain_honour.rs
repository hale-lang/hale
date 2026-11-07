//! GH #1858: does a locus that takes the drain into its own hands stop on
//! it?
//!
//! The runtime ends every `run()` loop at its back-edge when a stop (a
//! signal's drain) begins, unless the locus's `run()` reads `draining`
//! itself or through what it calls (the flow rows' watches-drain column):
//! such a locus is *manual*, and its loops are the author's. This module
//! says, conservatively, whether a manual `run()` honours the drain: every
//! loop it runs, in its own body or in a fn or method of the program it
//! calls (transitively, the walk the column takes), either
//!
//! - is a `for` over a collection or a range (it ends on its own);
//! - has a condition that reads `draining` leading out, as a conjunct of
//!   `!…draining` (`while !self.draining { … }`,
//!   `while open && !self.draining { … }`);
//! - tests it on every iteration with the test leading out: a statement of
//!   the body's own, reached before anything that could `continue` past
//!   it, `if …draining { …; break }` (or `return`; the flag a `||`
//!   operand of the test), or `if !…draining { … } else { break }`;
//! - or can end without the drain: its condition reads something besides
//!   the literal `true` and fields of `self` (a local, a call: what the
//!   loop can change), or it has an exit (`break`, `return`, `fail`,
//!   `terminate`, `violate`, `or raise`, `or fail`) that is not under a
//!   test of `draining`.
//!
//! What it cannot prove is reported, never judged wrong: the flag copied
//! into a field and tested later, a test on only some paths, a loop whose
//! only exit is under a drain test that does not lead out on every
//! iteration, an endless inner loop, a loop that waits for another locus.
//! Waits need no rule: every blocking wait wakes on the drain, so one does
//! not hold an iteration.

use std::collections::BTreeSet;

use hale_syntax::ast::{BinOp, Block, ElseBranch, Expr, IfStmt, Literal, LocusDecl, MatchArmBody, OrDisposition, Stmt, UnaryOp};
use hale_syntax::Span;

use crate::flows::FreeFns;
use crate::listener_window::{visit_block, visit_expr, Visited};

/// A loop a manual `run()` runs that the analysis cannot see stop on the
/// drain: where the loop is, and the fn or method of the program it sits
/// in when that is not the `run()` itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unproven {
    pub loop_span: Span,
    pub in_fn: Option<String>,
}

/// The loops of `run` (a manual locus's `run()` body) and of what it calls
/// that do not provably honour the drain, each once, in the order found.
pub fn unhonoured_loops<'a>(run: &'a Block, decl: &'a LocusDecl, free_fns: &FreeFns<'a>) -> Vec<Unproven> {
    let mut out: Vec<Unproven> = Vec::new();
    let mut seen_spans: BTreeSet<(usize, usize)> = BTreeSet::new();
    let mut judge = |body: &'a Block, in_fn: Option<&str>, out: &mut Vec<Unproven>| {
        visit_block(body, &mut |v| {
            let Visited::Stmt(Stmt::While { cond, body, span }) = v else { return };
            if honours(cond, body) {
                return;
            }
            if seen_spans.insert((span.start.as_usize(), span.end.as_usize())) {
                out.push(Unproven { loop_span: *span, in_fn: in_fn.map(str::to_string) });
            }
        });
    };
    judge(run, None, &mut out);
    let mut seen: BTreeSet<(bool, &'a str)> = BTreeSet::new();
    let mut stack: Vec<&'a Block> = vec![run];
    while let Some(body) = stack.pop() {
        for (method, name, f) in followed_callees(body, decl, free_fns) {
            if !seen.insert((method, name)) {
                continue;
            }
            judge(&f.body, Some(name), &mut out);
            stack.push(&f.body);
        }
    }
    out
}

/// The fns a body calls that the honour walk follows: a method of
/// `decl`'s called on `self` (`self.m()`), and a bare call (`m()`), as a
/// method of `decl`'s and as a free fn. A call on another receiver is
/// another locus's code, and not followed.
fn followed_callees<'a>(
    body: &'a Block,
    decl: &'a LocusDecl,
    free_fns: &FreeFns<'a>,
) -> Vec<(bool, &'a str, &'a hale_syntax::ast::FnDecl)> {
    let mut names: Vec<(bool, &'a str)> = Vec::new();
    visit_block(body, &mut |v| {
        let Visited::Expr(Expr::Call { callee, .. }) = v else { return };
        match callee.as_ref() {
            Expr::Field { receiver, name, .. } | Expr::Path2 { receiver, name, .. }
                if matches!(receiver.as_ref(), Expr::KwSelf(_)) =>
            {
                names.push((true, name.name.as_str()));
            }
            Expr::Ident(id) => {
                names.push((true, id.name.as_str()));
                names.push((false, id.name.as_str()));
            }
            _ => {}
        }
    });
    names
        .into_iter()
        .filter_map(|(method, name)| {
            let f = if method {
                decl.members.iter().find_map(|m| match m {
                    hale_syntax::ast::LocusMember::Fn(f) if f.name.name == name => Some(f),
                    _ => None,
                })
            } else {
                free_fns.get(name).copied()
            };
            f.map(|f| (method, name, f))
        })
        .collect()
}

/// Whether one `while` honours the drain or ends without it (the module's
/// rules; a `for` never reaches here).
fn honours(cond: &Expr, body: &Block) -> bool {
    conj_leads_out(cond) || body_leads_out(body) || ends_on_its_own(cond, body)
}

/// `e` reads a locus's `draining` flag (by name: the receiver is the
/// column's business, a record field so named is rare in a loop test).
fn is_draining(e: &Expr) -> bool {
    matches!(e, Expr::Field { name, .. } if name.name == "draining")
}

/// `e` is false whenever the flag is up: `!…draining`, or a `&&` with it
/// as an operand.
fn conj_leads_out(e: &Expr) -> bool {
    match e {
        Expr::Unary { op: UnaryOp::Not, operand, .. } => is_draining(operand),
        Expr::Binary { op: BinOp::And, left, right, .. } => conj_leads_out(left) || conj_leads_out(right),
        _ => false,
    }
}

/// `e` is true whenever the flag is up: `…draining`, or a `||` with it as
/// an operand.
fn disj_leads_in(e: &Expr) -> bool {
    match e {
        _ if is_draining(e) => true,
        Expr::Binary { op: BinOp::Or, left, right, .. } => disj_leads_in(left) || disj_leads_in(right),
        _ => false,
    }
}

/// Whether `e` reads the flag anywhere.
fn mentions_draining(e: &Expr) -> bool {
    let mut found = false;
    visit_expr(e, &mut |v| {
        if let Visited::Expr(x) = v {
            if is_draining(x) {
                found = true;
            }
        }
    });
    found
}

/// A block whose every path through its end leaves the loop or the fn:
/// it ends in `break`, `return`, `fail`, `terminate` or `violate` (or a
/// nested block that does).
fn always_leaves(b: &Block) -> bool {
    if b.tail.is_some() {
        return false;
    }
    match b.stmts.last() {
        Some(Stmt::Break(_) | Stmt::Return(..) | Stmt::Fail { .. } | Stmt::Terminate(_) | Stmt::Violate { .. }) => true,
        Some(Stmt::Block(inner)) => always_leaves(inner),
        Some(Stmt::If(i)) => if_always_leaves(i),
        _ => false,
    }
}

fn if_always_leaves(i: &IfStmt) -> bool {
    always_leaves(&i.then_block)
        && match i.else_block.as_deref() {
            Some(ElseBranch::Else(b)) => always_leaves(b),
            Some(ElseBranch::ElseIf(e)) => if_always_leaves(e),
            None => false,
        }
}

/// A test of the flag that leads out of the loop whenever it is up.
fn test_leads_out(i: &IfStmt) -> bool {
    (disj_leads_in(&i.cond) && always_leaves(&i.then_block))
        || (conj_leads_out(&i.cond)
            && matches!(i.else_block.as_deref(), Some(ElseBranch::Else(b)) if always_leaves(b)))
}

/// The body tests the flag on every iteration, leading out: a statement of
/// its own that is such a test, with nothing before it that could
/// `continue` past it.
fn body_leads_out(body: &Block) -> bool {
    for s in &body.stmts {
        if let Stmt::If(i) = s {
            if test_leads_out(i) {
                return true;
            }
        }
        if may_continue(s) {
            return false;
        }
    }
    false
}

/// Whether `s` holds a `continue` of the loop it sits in (one in a nested
/// loop belongs to that loop).
fn may_continue(s: &Stmt) -> bool {
    let mut found = false;
    visit_stmt_shallow(s, &mut |x| {
        if matches!(x, Stmt::Continue(_)) {
            found = true;
        }
    });
    found
}

/// The statements `s` holds, itself included, through ifs, matches and
/// blocks but not into a nested loop's body.
fn visit_stmt_shallow<'a>(s: &'a Stmt, f: &mut dyn FnMut(&'a Stmt)) {
    f(s);
    match s {
        Stmt::If(i) => visit_if_shallow(i, f),
        Stmt::Match(m) => {
            for arm in &m.arms {
                if let MatchArmBody::Block(b) = &arm.body {
                    for x in &b.stmts {
                        visit_stmt_shallow(x, f);
                    }
                }
            }
        }
        Stmt::Block(b) => {
            for x in &b.stmts {
                visit_stmt_shallow(x, f);
            }
        }
        _ => {}
    }
}

fn visit_if_shallow<'a>(i: &'a IfStmt, f: &mut dyn FnMut(&'a Stmt)) {
    for x in &i.then_block.stmts {
        visit_stmt_shallow(x, f);
    }
    match i.else_block.as_deref() {
        Some(ElseBranch::Else(b)) => {
            for x in &b.stmts {
                visit_stmt_shallow(x, f);
            }
        }
        Some(ElseBranch::ElseIf(e)) => visit_if_shallow(e, f),
        None => {}
    }
}

/// The loop can end without the drain: its condition reads something the
/// loop can change, or it has an exit no drain test guards.
fn ends_on_its_own(cond: &Expr, body: &Block) -> bool {
    !state_only(cond) || has_unguarded_exit(body)
}

/// A condition that reads only the literal `true`/`false` and fields of
/// `self`: state the loop's own locals do not move, which only another
/// body (a handler, a hook, another locus) can flip.
fn state_only(e: &Expr) -> bool {
    match e {
        Expr::Literal(Literal::Bool(_), _) => true,
        Expr::Field { receiver, .. } => self_rooted(receiver),
        Expr::Unary { op: UnaryOp::Not, operand, .. } => state_only(operand),
        Expr::Binary { op: BinOp::And | BinOp::Or, left, right, .. } => state_only(left) && state_only(right),
        _ => false,
    }
}

fn self_rooted(e: &Expr) -> bool {
    match e {
        Expr::KwSelf(_) => true,
        Expr::Field { receiver, .. } => self_rooted(receiver),
        _ => false,
    }
}

/// Whether the loop body has an exit that no test of the flag guards: a
/// `break` of this loop, or a `return`, `fail`, `terminate`, `violate`,
/// `or raise` or `or fail` anywhere in it. Exits under an `if` whose test
/// reads the flag are the drain's, and do not count.
fn has_unguarded_exit(body: &Block) -> bool {
    body.stmts.iter().any(|s| stmt_exits(s, 0))
}

fn stmt_exits(s: &Stmt, depth: usize) -> bool {
    match s {
        Stmt::Break(_) => depth == 0,
        Stmt::Return(..) | Stmt::Fail { .. } | Stmt::Terminate(_) | Stmt::Violate { .. } => true,
        Stmt::If(i) => if_exits(i, depth),
        Stmt::Match(m) => {
            exprs_exit(&m.scrutinee)
                || m.arms.iter().any(|arm| match &arm.body {
                    MatchArmBody::Block(b) => block_exits(b, depth),
                    MatchArmBody::Expr(e) => exprs_exit(e),
                })
        }
        Stmt::Block(b) => block_exits(b, depth),
        Stmt::While { cond, body, .. } => exprs_exit(cond) || block_exits(body, depth + 1),
        Stmt::For { iter, body, .. } => exprs_exit(iter) || block_exits(body, depth + 1),
        Stmt::Let { value, .. } | Stmt::LetTuple { value, .. } | Stmt::Assign { value, .. } => exprs_exit(value),
        Stmt::Expr(e) => exprs_exit(e),
        _ => false,
    }
}

fn block_exits(b: &Block, depth: usize) -> bool {
    b.stmts.iter().any(|s| stmt_exits(s, depth)) || b.tail.as_deref().is_some_and(exprs_exit)
}

fn if_exits(i: &IfStmt, depth: usize) -> bool {
    if mentions_draining(&i.cond) {
        return false;
    }
    block_exits(&i.then_block, depth)
        || match i.else_block.as_deref() {
            Some(ElseBranch::Else(b)) => block_exits(b, depth),
            Some(ElseBranch::ElseIf(e)) => if_exits(e, depth),
            None => false,
        }
}

/// An exit written inside an expression: `x or raise`, `x or fail e`, or a
/// `return` in an `or { … }` block (any `break` there is counted too: the
/// conservative side, a missed report, never a false one).
fn exprs_exit(e: &Expr) -> bool {
    let mut found = false;
    visit_expr(e, &mut |v| match v {
        Visited::Expr(Expr::Or { disposition: OrDisposition::Raise(_) | OrDisposition::Fail(..), .. }) => found = true,
        Visited::Stmt(Stmt::Break(_) | Stmt::Return(..) | Stmt::Fail { .. } | Stmt::Terminate(_) | Stmt::Violate { .. }) => {
            found = true
        }
        _ => {}
    });
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unproven(src: &str) -> Vec<usize> {
        let p = hale_syntax::parse_source(src).expect("parse");
        let free = crate::flows::free_fns_of(&[&p]);
        let mut lines = Vec::new();
        for item in &p.items {
            let hale_syntax::ast::TopDecl::Locus(l) = item else { continue };
            let run = l.members.iter().find_map(|m| match m {
                hale_syntax::ast::LocusMember::Lifecycle(lc) if lc.kind == hale_syntax::ast::LifecycleKind::Run => Some(&lc.body),
                _ => None,
            });
            for u in unhonoured_loops(run.expect("a run"), l, &free) {
                lines.push(src[..u.loop_span.start.as_usize()].matches('\n').count() + 1);
            }
        }
        lines
    }

    #[test]
    fn proven_shapes_are_silent() {
        let src = "locus A { run() {\n\
                   while !self.draining { }\n\
                   while self.open && !self.draining { }\n\
                   while true { if self.draining { break; } work(); }\n\
                   while true { if self.draining || self.done { return; } }\n\
                   while true { if !self.draining { work(); } else { break; } }\n\
                   let mut i = 0;\n\
                   while i < 10 { i = i + 1; }\n\
                   while true { if next() == 0 { break; } }\n\
                   for x in [1, 2, 3] { }\n\
                   } fn work() { } fn next() -> Int { return 0; }\n\
                   params { open: Bool = true; done: Bool = false; } }\n\
                   fn main() { }\n";
        assert_eq!(unproven(src), Vec::<usize>::new());
    }

    #[test]
    fn copied_flags_conditional_tests_and_endless_loops_are_reported() {
        let src = "locus A {\n\
                   params { stopping: Bool = false; x: Bool = false; }\n\
                   run() {\n\
                   if self.draining { self.stopping = true; }\n\
                   while !self.stopping { }\n\
                   while true { if self.x { if self.draining { break; } } }\n\
                   while true { work(); if self.draining { break; } }\n\
                   self.spin();\n\
                   }\n\
                   fn spin() { while true { } }\n\
                   fn work() { }\n\
                   }\n\
                   fn main() { }\n";
        // line 7's test leads out (it is a statement of the body's own, and
        // nothing before it continues); the others do not.
        assert_eq!(unproven(src), vec![5, 6, 10]);
    }

    #[test]
    fn a_continue_before_the_test_skips_it() {
        let src = "locus A { params { x: Bool = false; } run() {\n\
                   while true { if self.x { continue; } if self.draining { break; } }\n\
                   } }\n\
                   fn main() { }\n";
        assert_eq!(unproven(src), vec![2]);
    }
}

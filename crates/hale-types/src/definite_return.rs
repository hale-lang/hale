//! GH #1940: definite return.
//!
//! A body with a declared return type (a fn, a locus method, a mode with
//! `-> T`) returns a value on every path that reaches its end. One that
//! can run off its end without a value is refused at the check, located
//! at the declaration's name, with the path that falls through as a
//! secondary location. Before this the check passed such a body and the
//! build refused it with no file or line.
//!
//! The judgment is syntactic and conservative in the direction of
//! refusing only what lowering cannot lower: a path **leaves** without
//! reaching the end when it meets `return`, `fail`, `break`, `continue`,
//! `terminate` or `violate`; an `if` with an `else` whose every branch
//! leaves; a `match` (exhaustive by F.18) whose every arm leaves; a block
//! that leaves; a `while true` with no `break` of its own (a `break` in a
//! nested loop leaves that loop); or a statement whose value is a
//! diverging `match` / `if` / block expression. Every other statement can
//! complete, and the body reaches its end when its last statement can.
//!
//! The body's **result position** supplies the value (spec/semantics.md
//! § "Definite return"): its tail, and, when its last statement is a
//! `match`, an `if` or a block, that statement's arms and tails in turn.
//! `desugar_tail_returns` writes those leaves as `return`s before the
//! check, so here a result-position leaf is a value whether it is still
//! spelled as an arm or a tail or already as the `return` it became.

use std::collections::BTreeSet;

use hale_syntax::ast::{
    flat_decls, Block, ElseBranch, Expr, FnDecl, IfStmt, Literal, LocusMember, MatchArmBody, MatchStmt,
    PerspectiveMember, Stmt, TopDecl, TypeExpr,
};
use hale_syntax::{Diag, Span};

use crate::symbol::Bundle;

/// Where a body can reach its end without a value: the place to point
/// at, and what to say there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FallThrough {
    pub span: Span,
    pub why: String,
}

/// Every body of the bundle with a declared return type that can reach
/// its end without returning a value, as a type error at its name.
pub fn definite_return_diags(bundle: &Bundle<'_>) -> Vec<Diag> {
    let mut out = Vec::new();
    for program in bundle.programs.values() {
        for item in flat_decls(&program.items) {
            match item {
                TopDecl::Fn(f) => fn_diag(&format!("fn `{}`", f.name.name), f, &mut out),
                TopDecl::Locus(l) => {
                    for m in &l.members {
                        match m {
                            LocusMember::Fn(f) => {
                                fn_diag(&format!("method `{}.{}`", l.name.name, f.name.name), f, &mut out)
                            }
                            LocusMember::Mode(md) => {
                                if let Some(ret) = &md.ret {
                                    let what = format!("mode `{}` of `{}`", mode_word(md.kind), l.name.name);
                                    if let Some(fall) = fall_through(&md.body) {
                                        out.push(refusal(&what, ret, md.span, fall));
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                TopDecl::Perspective(p) => {
                    for m in &p.members {
                        // a contract signature (`fn route(r: R) -> S;`) has
                        // no body to judge; a bodied default does
                        if let PerspectiveMember::Fn(f) = m {
                            if !f.body.stmts.is_empty() || f.body.tail.is_some() {
                                fn_diag(&format!("fn `{}.{}`", p.name.name, f.name.name), f, &mut out);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    out
}

fn mode_word(kind: hale_syntax::ast::ModeKind) -> &'static str {
    match kind {
        hale_syntax::ast::ModeKind::Bulk => "bulk",
        hale_syntax::ast::ModeKind::Harmonic => "harmonic",
        hale_syntax::ast::ModeKind::Resolution => "resolution",
    }
}

fn fn_diag(what: &str, f: &FnDecl, out: &mut Vec<Diag>) {
    let Some(ret) = &f.ret else { return };
    if f.ffi.is_some() {
        return;
    }
    if let Some(fall) = fall_through(&f.body) {
        out.push(refusal(what, ret, f.name.span, fall));
    }
}

fn refusal(what: &str, ret: &TypeExpr, at: Span, fall: FallThrough) -> Diag {
    let ty = match ret {
        TypeExpr::Primitive(p, _) => crate::ty::prim_name(*p).to_string(),
        other => crate::check::type_expr_text(other),
    };
    Diag::ty(
        at,
        format!(
            "{what} returns `{ty}`, and its body can reach its end without a value: {}. Return a value on \
             that path, or end the body with its value (a tail expression, or a `match`, an `if … else` or \
             a block whose every arm ends in one)",
            fall.why
        ),
    )
    .with_related(fall.span, "the path that reaches the end without a value")
}

/// Can this body, in result position, reach its end without a value?
/// `None` when every path returns or leaves.
pub fn fall_through(body: &Block) -> Option<FallThrough> {
    if body.stmts.is_empty() && body.tail.is_none() {
        return Some(FallThrough { span: body.span, why: "the body is empty".to_string() });
    }
    block(body, true)
}

/// Can `b` complete normally? In result position (`result`), its tail and
/// its last statement's leaves are values, so they do not fall through.
fn block(b: &Block, result: bool) -> Option<FallThrough> {
    let n = b.stmts.len();
    for (i, s) in b.stmts.iter().enumerate() {
        let last = i + 1 == n && b.tail.is_none();
        match stmt(s, result && last) {
            // a statement that never completes: nothing after it runs
            None => return None,
            Some(fall) if last => return Some(fall),
            Some(_) => {}
        }
    }
    match &b.tail {
        Some(_) if result => None,
        Some(t) if diverges(t) => None,
        Some(t) => Some(FallThrough { span: t.span(), why: "the block ends after this expression".to_string() }),
        None if n == 0 => Some(FallThrough { span: b.span, why: "this block is empty".to_string() }),
        // the loop above returned for the last statement
        None => unreachable!("a block with statements returned at its last one"),
    }
}

fn stmt(s: &Stmt, result: bool) -> Option<FallThrough> {
    match s {
        Stmt::Return(..)
        | Stmt::Fail { .. }
        | Stmt::Break(_)
        | Stmt::Continue(_)
        | Stmt::Terminate(_)
        | Stmt::Violate { .. } => None,
        Stmt::If(i) => if_stmt(i, result),
        Stmt::Match(m) => match_stmt(m, result),
        Stmt::Block(b) => {
            if b.stmts.is_empty() && b.tail.is_none() {
                return Some(FallThrough { span: s.span(), why: "the body ends in an empty block".to_string() });
            }
            block(b, result)
        }
        Stmt::While { cond: Expr::Literal(Literal::Bool(true), _), body, .. } => own_break(body).map(|at| FallThrough {
            span: at,
            why: "this `break` leaves the `while true` loop, and the body ends after it".to_string(),
        }),
        Stmt::While { span, .. } => Some(FallThrough {
            span: *span,
            why: "this `while` loop can finish (its condition can be false), and the body ends after it".to_string(),
        }),
        Stmt::For { span, .. } => Some(FallThrough {
            span: *span,
            why: "this `for` loop can finish, and the body ends after it".to_string(),
        }),
        Stmt::Let { value, .. } | Stmt::LetTuple { value, .. } | Stmt::Assign { value, .. } | Stmt::Expr(value)
            if diverges(value) =>
        {
            None
        }
        _ => Some(FallThrough { span: s.span(), why: "the body ends after this statement".to_string() }),
    }
}

fn if_stmt(i: &IfStmt, result: bool) -> Option<FallThrough> {
    let then = block(&i.then_block, result);
    let other = match i.else_block.as_deref() {
        None => Some(FallThrough {
            span: i.span,
            why: "this `if` has no `else`, so when its condition is false the body ends after it".to_string(),
        }),
        Some(ElseBranch::Else(b)) => block(b, result),
        Some(ElseBranch::ElseIf(n)) => if_stmt(n, result),
    };
    then.or(other)
}

fn match_stmt(m: &MatchStmt, result: bool) -> Option<FallThrough> {
    if m.arms.is_empty() {
        return Some(FallThrough { span: m.span, why: "this `match` has no arms".to_string() });
    }
    // exhaustive (F.18, refused where it is not): a path leaves the match
    // only through an arm
    m.arms.iter().find_map(|arm| match &arm.body {
        MatchArmBody::Expr(_) if result => None,
        MatchArmBody::Expr(e) if diverges(e) => None,
        MatchArmBody::Expr(_) => Some(FallThrough {
            span: arm.span,
            why: "this arm's value is discarded, and the body ends after the `match`".to_string(),
        }),
        MatchArmBody::Block(b) => block(b, result).map(|fall| {
            if b.stmts.is_empty() && b.tail.is_none() {
                FallThrough { span: arm.span, why: "this arm is empty".to_string() }
            } else {
                fall
            }
        }),
    })
}

/// Does evaluating `e` always leave instead of producing a value? A
/// `match` expression whose every arm leaves, an `if` expression both of
/// whose arms do, a block that does.
fn diverges(e: &Expr) -> bool {
    match e {
        Expr::Match(m) => {
            !m.arms.is_empty()
                && m.arms.iter().all(|arm| match &arm.body {
                    MatchArmBody::Expr(x) => diverges(x),
                    MatchArmBody::Block(b) => block(b, false).is_none(),
                })
        }
        Expr::If(i) => i.else_block.is_some() && if_stmt(i, false).is_none(),
        Expr::Block(b) => block(b, false).is_none(),
        _ => false,
    }
}

/// The first `break` in `body` that leaves this loop: anywhere in its
/// statements and expressions (an `or { break; }` substitute's included)
/// except inside a nested loop, whose `break`s leave that loop.
fn own_break(body: &Block) -> Option<Span> {
    let mut all: Vec<Span> = Vec::new();
    let mut nested: BTreeSet<(u32, u32)> = BTreeSet::new();
    crate::listener_window::visit_block(body, &mut |v| {
        if let crate::listener_window::Visited::Stmt(s) = v {
            match s {
                Stmt::Break(at) => all.push(*at),
                Stmt::While { body: inner, .. } | Stmt::For { body: inner, .. } => {
                    crate::listener_window::visit_block(inner, &mut |w| {
                        if let crate::listener_window::Visited::Stmt(Stmt::Break(at)) = w {
                            nested.insert((at.start.0, at.end.0));
                        }
                    });
                }
                _ => {}
            }
        }
    });
    all.into_iter().find(|at| !nested.contains(&(at.start.0, at.end.0)))
}

#[cfg(test)]
mod tests {
    use super::fall_through;

    // As parsed, before the tail-return desugar: a result-position leaf is
    // a value spelled either way (the integration tests run the desugar).
    fn body_of(src: &str) -> hale_syntax::ast::Block {
        let program = hale_syntax::parse_source(src).expect("parses");
        match program.items.into_iter().next() {
            Some(hale_syntax::ast::TopDecl::Fn(f)) => f.body,
            _ => panic!("a fn first"),
        }
    }

    #[test]
    fn result_positions_return() {
        for src in [
            "fn f(x: Int) -> Int { x }",
            "fn f(x: Int) -> String { match x { 1 -> \"one\", _ -> \"other\" } }",
            "fn f(x: Int) -> Int { if x > 0 { 1 } else { 2 } }",
            "fn f(x: Int) -> Int { if x > 0 { return 1; } else { 2 } }",
            "fn f(x: Int) -> Int { let y = x; { y + 1 } }",
            "fn f(x: Int) -> Int { while true { return x; } }",
            "fn f(x: Int) -> Int { while true { for i in 0..x { break; } return x; } }",
            "fn f(x: Int) -> Int { match x { 1 -> { let y = 2; y }, _ -> { return 3; } } }",
        ] {
            assert_eq!(fall_through(&body_of(src)), None, "{src}");
        }
    }

    #[test]
    fn paths_that_fall_through() {
        for (src, why) in [
            ("fn f(n: Int) -> Int { if n > 0 { return 1; } }", "has no `else`"),
            ("fn f(n: Int) -> Int { while true { if n > 0 { break; } } }", "`break` leaves"),
            ("fn f(n: Int) -> Int { while n > 0 { return 1; } }", "`while` loop can finish"),
            ("fn f(n: Int) -> Int { print(n); }", "ends after this statement"),
            ("fn f(n: Int) -> Int { }", "body is empty"),
            ("fn f(n: Int) -> Int { match n { 1 -> { print(n); }, _ -> { return 2; } } }", "ends after this statement"),
        ] {
            let fall = fall_through(&body_of(src)).unwrap_or_else(|| panic!("{src} falls through"));
            assert!(fall.why.contains(why), "{src}: {}", fall.why);
        }
    }
}

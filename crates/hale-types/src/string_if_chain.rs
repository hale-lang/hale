//! A run of `if` tests comparing one String with literals, which a
//! `match` says once (GH #1995 S5).
//!
//! String `match` works (`match verb { "assign" | "unassign" -> …, _ ->
//! … }`) and was all but unused: dispatch on a string was written as a
//! ladder of `if verb == "assign" { … }` tests, either chained with
//! `else if` or as sibling statements each of which returns. The ladder
//! repeats the subject on every line, hides that the tests are one
//! decision, and has no catch-all the reader can find.
//!
//! The rule, per statement list (a body, and every block in it): three
//! or more `if` tests in a row, each comparing the same place (a local,
//! or `self` and its fields, as `place` spells one) with a string literal
//! by `==` (either side, or an `||` of such comparisons), is warned at the
//! first test, with the `match` it reads as. The tests may be one `if`'s
//! `else if` links or consecutive `if` statements; a link whose condition
//! is anything else, a final plain `else`, or any other statement ends the
//! run. A trailing `if` used as a value is held to the same rule over its
//! links. Code that already matches has no such ladder and is never warned.
//!
//! Syntactic, over the program's own source as written (never the
//! stdlib's, nor stdlib text an expansion appends), and run once per
//! check. Only when asked for, by `--warn-string-match` on `hale check` or
//! `hale verify` (`Config::string_match_advisory`): `verify` fails on any
//! finding, advisories included, so a suggestion it always reported would
//! fail every program that has such a ladder.

use hale_syntax::ast::{flat_decls, BinOp, Block, ElseBranch, Expr, IfStmt, Literal, LocusMember, Stmt, TopDecl};
use hale_syntax::error::Diag;
use hale_syntax::Span;

use crate::listener_window::{visit_block, Visited};
use crate::symbol::Bundle;

/// The fewest tests in a row the advisory names.
const LADDER: usize = 3;

/// Every run of string-literal `if` tests on one place in `bundle`'s
/// bodies.
pub(crate) fn string_if_chain_diags(bundle: &Bundle<'_>) -> Vec<Diag> {
    let mut out = Vec::new();
    for (seed, p) in &bundle.programs {
        if seed == crate::snapshot::STDLIB_SEED {
            continue;
        }
        for d in flat_decls(&p.items) {
            match d {
                TopDecl::Fn(f) => body(&f.body, &mut out),
                TopDecl::Locus(l) => {
                    for m in &l.members {
                        match m {
                            LocusMember::Fn(f) => body(&f.body, &mut out),
                            LocusMember::Lifecycle(lc) => body(&lc.body, &mut out),
                            LocusMember::Mode(md) => body(&md.body, &mut out),
                            LocusMember::Failure(fd) => body(&fd.body, &mut out),
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }
    // The program's own source only: stdlib text an expansion appended
    // to it (the api runtime, the streamed body readers, the TCP
    // substrate, …) is the stdlib's to write, not the author's to fix.
    out.retain(|d| !stdlib_text(d.span.start.0));
    out
}

/// Whether a parse-space offset is stdlib source appended to the program.
fn stdlib_text(pos: u32) -> bool {
    crate::rpc_expand::runtime_source_at(pos).is_some()
        || crate::rpc_expand::is_appended_stdlib_pos(pos)
        || crate::rpc_expand::is_tcp_serve_appended_pos(pos)
}

/// A place spelled as a local or `self` and its fields (`verb`,
/// `self.kind`, `req.method`), as text; `None` for anything else.
fn place(e: &Expr) -> Option<String> {
    match e {
        Expr::Ident(id) => Some(id.name.clone()),
        Expr::KwSelf(_) => Some("self".to_string()),
        Expr::Field { receiver, name, .. } => Some(format!("{}.{}", place(receiver)?, name.name)),
        _ => None,
    }
}

/// The place a condition compares with string literals: `p == "lit"`,
/// `"lit" == p`, or an `||` of such comparisons all on one place.
fn tested_place(cond: &Expr) -> Option<String> {
    match cond {
        Expr::Binary { op: BinOp::Eq, left, right, .. } => match (left.as_ref(), right.as_ref()) {
            (Expr::Literal(Literal::String(_), _), other) | (other, Expr::Literal(Literal::String(_), _)) => {
                place(other)
            }
            _ => None,
        },
        Expr::Binary { op: BinOp::Or, left, right, .. } => {
            let l = tested_place(left)?;
            (tested_place(right)? == l).then_some(l)
        }
        _ => None,
    }
}

/// One `if`'s links that test a place with literals: the place, how many
/// links in a row test it (from the first), where the first test is, and
/// whether a following sibling `if` may extend the run (every link tests
/// the place and the chain has no plain `else`).
fn chain(i: &IfStmt) -> Option<(String, usize, Span, bool)> {
    let subject = tested_place(&i.cond)?;
    let first = i.cond.span();
    let mut n = 1;
    let mut link = i;
    loop {
        match link.else_block.as_deref() {
            None => return Some((subject, n, first, true)),
            Some(ElseBranch::Else(_)) => return Some((subject, n, first, false)),
            Some(ElseBranch::ElseIf(next)) => {
                if tested_place(&next.cond).as_deref() != Some(subject.as_str()) {
                    return Some((subject, n, first, false));
                }
                n += 1;
                link = next;
            }
        }
    }
}

fn warn(subject: &str, n: usize, at: Span, out: &mut Vec<Diag>) {
    out.push(Diag::warn(
        at,
        format!(
            "`{subject}` is compared with a string literal by {n} `if` tests in a row; a `match` makes the \
             one decision once, with a catch-all: `match {subject} {{ \"a\" -> …, \"b\" | \"c\" -> …, _ -> … }}`"
        ),
    ));
}

/// The runs in one statement list (and, through the visit, in every
/// block under it).
fn body(b: &Block, out: &mut Vec<Diag>) {
    visit_block(b, &mut |v| match v {
        Visited::Enter(block) => statements(block, out),
        // a trailing `if` used as a value, or one in a `let`
        Visited::Expr(Expr::If(i)) => {
            if let Some((subject, n, at, _)) = chain(i) {
                if n >= LADDER {
                    warn(&subject, n, at, out);
                }
            }
        }
        _ => {}
    });
}

fn statements(block: &Block, out: &mut Vec<Diag>) {
    // (place, tests so far, the first test, whether the run may go on)
    let mut run: Option<(String, usize, Span, bool)> = None;
    let flush = |run: &mut Option<(String, usize, Span, bool)>, out: &mut Vec<Diag>| {
        if let Some((subject, n, at, _)) = run.take() {
            if n >= LADDER {
                warn(&subject, n, at, out);
            }
        }
    };
    for s in &block.stmts {
        let Stmt::If(i) = s else {
            flush(&mut run, out);
            continue;
        };
        let Some((subject, n, at, open)) = chain(i) else {
            flush(&mut run, out);
            continue;
        };
        match run.as_mut() {
            Some(r) if r.3 && r.0 == subject => {
                r.1 += n;
                r.3 = open;
            }
            _ => {
                flush(&mut run, out);
                run = Some((subject, n, at, open));
            }
        }
        if !open {
            flush(&mut run, out);
        }
    }
    flush(&mut run, out);
}

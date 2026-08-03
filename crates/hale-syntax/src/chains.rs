//! Recognized element chains (#353 cluster B).
//!
//! `xs.filter(it > 2).count()` is not a value being built — it is a
//! form the compiler recognizes and lowers to ONE loop. That is the
//! whole point: nothing is produced at any step, so there is no
//! sequence value, no owner for it, and no arena question. The
//! knot that made "add closures, then add iterators" look expensive
//! was self-inflicted by assuming each stage must yield something.
//!
//! Consequences of doing it this way, all of which fall out rather
//! than being engineered:
//!
//! - **Zero allocation.** No intermediate exists, so a chain is legal
//!   inside `@hot` / `@budget(alloc_per_call = 0)` — composition is
//!   usable in exactly the code Hale is built for. A design that
//!   returned a new collection would have been illegal there.
//! - **Effects stay attributed where written.** The chain is eager, so
//!   a predicate's effects are reached at the predicate's own source
//!   position. A lazy chain would execute it at the terminal and the
//!   witness path would name the wrong line.
//! - **No lambdas.** The predicate is an argument position the
//!   compiler knows about, not a value, so there is no closure to
//!   represent — no capture modes, no escape analysis, no `self`
//!   capture policy, no cross-thread question. `it` is bound per
//!   element by this desugar.
//!
//! Done as a POST-PARSE AST rewrite rather than in codegen, so
//! typecheck and codegen both see an ordinary `while` loop. No new
//! machinery in either, and a chain over a non-form receiver fails
//! with the ordinary "no method `len`" diagnostic.
//!
//! v1 surface: `filter` as the elementwise op, `count` and `into` as
//! terminals. The mechanism is the deliverable; more ops are rows in
//! the same table.

use crate::ast::*;
use crate::span::Span;

/// Rewrite every recognized chain in the program.
pub fn desugar_chains(program: &mut Program) {
    let mut n = 0usize;
    for item in &mut program.items {
        match item {
            TopDecl::Fn(f) => block(&mut f.body, &mut n),
            TopDecl::Locus(l) => {
                for m in &mut l.members {
                    match m {
                        LocusMember::Fn(f) => block(&mut f.body, &mut n),
                        LocusMember::Lifecycle(lc) => block(&mut lc.body, &mut n),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
}

fn block(b: &mut Block, n: &mut usize) {
    let mut out: Vec<Stmt> = Vec::new();
    for mut s in std::mem::take(&mut b.stmts) {
        stmt(&mut s, n);
        // A chain in STATEMENT position (`xs.filter(..).into(out);`)
        // lowers to a block-expression with no tail. Splice its
        // statements into the enclosing block rather than leaving a
        // bare block expression, which codegen does not accept as a
        // statement. Loop temporaries are numbered, so two chains in
        // one block cannot collide.
        if let Stmt::Expr(Expr::Block(inner)) = &mut s {
            if inner.tail.is_none() && !inner.stmts.is_empty() {
                out.append(&mut inner.stmts);
                continue;
            }
        }
        out.push(s);
    }
    b.stmts = out;
    if let Some(t) = &mut b.tail {
        expr(t, n);
    }
}

fn stmt(s: &mut Stmt, n: &mut usize) {
    match s {
        Stmt::Let { value, .. } => expr(value, n),
        Stmt::Assign { value, .. } => expr(value, n),
        Stmt::Expr(e) => expr(e, n),
        Stmt::While { body, .. } => block(body, n),
        Stmt::For { body, .. } => block(body, n),
        Stmt::If(i) => if_stmt(i, n),
        Stmt::Return(Some(e), _) => expr(e, n),
        _ => {}
    }
}

fn if_stmt(i: &mut IfStmt, n: &mut usize) {
    expr(&mut i.cond, n);
    block(&mut i.then_block, n);
    if let Some(e) = &mut i.else_block {
        match e.as_mut() {
            ElseBranch::Else(b) => block(b, n),
            ElseBranch::ElseIf(e) => if_stmt(e, n),
        }
    }
}

/// A chain's shape: the source receiver, the accumulated predicates,
/// and the terminal.
struct Chain {
    src: Expr,
    preds: Vec<Expr>,
    terminal: String,
    term_args: Vec<Expr>,
}

/// Peel `src.filter(p).filter(q).count()` into its parts. `None` when
/// this is not a recognized chain — an ordinary method call.
fn peel(e: &Expr) -> Option<Chain> {
    let Expr::Call { callee, args, .. } = e else {
        return None;
    };
    let Expr::Field { receiver, name, .. } = callee.as_ref() else {
        return None;
    };
    if !matches!(name.name.as_str(), "count" | "into") {
        return None;
    }
    let terminal = name.name.clone();
    let term_args = args.clone();

    // Walk down the filter stages.
    let mut preds = Vec::new();
    let mut cur = receiver.as_ref();
    loop {
        let Expr::Call { callee, args, .. } = cur else { break };
        let Expr::Field { receiver, name, .. } = callee.as_ref() else {
            break;
        };
        if name.name != "filter" || args.len() != 1 {
            break;
        }
        preds.push(args[0].clone());
        cur = receiver.as_ref();
    }
    // A bare `xs.count()` with no stage is an ordinary method call,
    // not a chain — leave it alone so existing form methods still
    // resolve.
    if preds.is_empty() {
        return None;
    }
    preds.reverse();
    Some(Chain { src: cur.clone(), preds, terminal, term_args })
}

fn expr(e: &mut Expr, n: &mut usize) {
    // Rewrite innermost-first so a chain inside a chain's predicate is
    // handled before the outer one is consumed.
    match e {
        Expr::Call { callee, args, .. } => {
            expr(callee, n);
            for a in args {
                expr(a, n);
            }
        }
        Expr::Field { receiver, .. } => expr(receiver, n),
        Expr::Binary { left, right, .. } => {
            expr(left, n);
            expr(right, n);
        }
        Expr::Block(b) => block(b, n),
        _ => {}
    }
    if let Some(c) = peel(e) {
        *n += 1;
        *e = lower(c, e.span(), *n);
    }
}

// ---- construction helpers -----------------------------------------

fn id(n: &str, sp: Span) -> Ident {
    Ident { name: n.to_string(), span: sp }
}
fn var(n: &str, sp: Span) -> Expr {
    Expr::Ident(id(n, sp))
}
fn int(v: i64, sp: Span) -> Expr {
    Expr::Literal(Literal::Int(v), sp)
}
fn method(recv: Expr, name: &str, args: Vec<Expr>, sp: Span) -> Expr {
    Expr::Call {
        callee: Box::new(Expr::Field {
            receiver: Box::new(recv),
            name: id(name, sp),
            span: sp,
        }),
        args,
        span: sp,
    }
}
fn bin(op: BinOp, l: Expr, r: Expr, sp: Span) -> Expr {
    Expr::Binary {
        op,
        left: Box::new(l),
        right: Box::new(r),
        span: sp,
    }
}
fn let_mut(n: &str, v: Expr, sp: Span) -> Stmt {
    Stmt::Let {
        is_mut: true,
        name: id(n, sp),
        ty: None,
        value: v,
        span: sp,
    }
}
fn assign(n: &str, v: Expr, sp: Span) -> Stmt {
    Stmt::Assign {
        target: LValue { head: id(n, sp), tail: Vec::new(), span: sp },
        op: AssignOp::Eq,
        value: v,
        span: sp,
    }
}

/// `src.filter(p)…<terminal>` -> a block containing one loop.
fn lower(c: Chain, sp: Span, uid: usize) -> Expr {
    let i_name = format!("__hale_chain_i{}", uid);
    let acc_name = format!("__hale_chain_n{}", uid);
    let (i, acc) = (i_name.as_str(), acc_name.as_str());

    // The per-element body, innermost first: the terminal's action,
    // wrapped in each predicate.
    let action: Stmt = match c.terminal.as_str() {
        "count" => assign(
            acc,
            bin(BinOp::Add, var(acc, sp), int(1, sp), sp),
            sp,
        ),
        // `into(target)` pushes the element.
        _ => Stmt::Expr(method(
            c.term_args
                .first()
                .cloned()
                .unwrap_or_else(|| var("__hale_chain_missing", sp)),
            "push",
            vec![var("it", sp)],
            sp,
        )),
    };

    let mut body_stmts = vec![action];
    for p in c.preds.iter().rev() {
        body_stmts = vec![Stmt::If(IfStmt {
            cond: p.clone(),
            then_block: Block {
                stmts: body_stmts,
                tail: None,
                span: sp,
            },
            else_block: None,
            span: sp,
        })];
    }

    // `let it = src.get(i) or { break; };` — the element binding. The
    // diverging fallback is why #353's diverging-`or` fix had to land
    // first: there is no typed default to invent for an arbitrary
    // element type.
    let bind_it = Stmt::Let {
        is_mut: false,
        name: id("it", sp),
        ty: None,
        value: Expr::Or {
            inner: Box::new(method(
                c.src.clone(),
                "get",
                vec![var(i, sp)],
                sp,
            )),
            disposition: OrDisposition::Substitute(Box::new(Expr::Block(
                Block {
                    stmts: vec![Stmt::Break(sp)],
                    tail: None,
                    span: sp,
                },
            ))),
            span: sp,
        },
        span: sp,
    };

    let mut loop_body = vec![bind_it];
    loop_body.extend(body_stmts);
    loop_body.push(assign(
        i,
        bin(BinOp::Add, var(i, sp), int(1, sp), sp),
        sp,
    ));

    let mut stmts = vec![let_mut(i, int(0, sp), sp)];
    if c.terminal == "count" {
        stmts.push(let_mut(acc, int(0, sp), sp));
    }
    stmts.push(Stmt::While {
        cond: bin(
            BinOp::Lt,
            var(i, sp),
            method(c.src.clone(), "len", Vec::new(), sp),
            sp,
        ),
        body: Block { stmts: loop_body, tail: None, span: sp },
        span: sp,
    });

    Expr::Block(Block {
        stmts,
        tail: if c.terminal == "count" {
            Some(Box::new(var(acc, sp)))
        } else {
            None
        },
        span: sp,
    })
}

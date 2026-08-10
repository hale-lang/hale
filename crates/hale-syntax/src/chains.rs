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
//!   element by this desugar. (`each { ... }` LOOKS lambda-shaped
//!   but is spliced as the fused loop's body in the enclosing scope
//!   — no capture semantics exist to define.)
//!
//! Done as a POST-PARSE AST rewrite rather than in codegen, so
//! typecheck and codegen both see an ordinary `while` loop. No new
//! machinery in either, and a chain over a non-form receiver fails
//! with the ordinary "no method `len`" diagnostic.
//!
//! ## Vocabulary (2026-08-04 tranche; v1 was filter/count/into)
//!
//! Elementwise stages — fuse into the one loop:
//!   - `filter(pred)`  — `it`-predicate, drops non-matching elements
//!   - `map(expr)`     — `it`-expression, rebinds the element
//!
//! Terminals:
//!   - `count()`            → Int
//!   - `into(target)`       → pushes surviving (mapped) elements
//!   - `sum()`              → Int (v1: Int elements only — the
//!                            accumulator's typed zero needs literal
//!                            suffixes for Float/Decimal/Duration)
//!   - `any(pred?)`         → Bool; empty ⇒ false (vacuous)
//!   - `all(pred)`          → Bool; empty ⇒ true  (vacuous)
//!   - `first()`            → element, FALLIBLE on empty (IndexError,
//!                            handled with ordinary `or`)
//!   - `find(pred?)`        → first(); `find(p)` ≡ `filter(p).first()`
//!   - `min(key?)`/`max(key?)` → the ELEMENT with the least/greatest
//!                            key (`min_by_key` shape); bare form
//!                            compares elements themselves; FALLIBLE
//!                            on empty like `first`
//!   - `each { ... }`       → side-effectful visitation; the block is
//!                            the fused loop body with `it` bound
//!
//! Recognition is deliberately conservative so user-declared facade
//! methods never get hijacked:
//!   - with ≥1 stage, every terminal is recognized (stages make the
//!     expression impossible as an ordinary method chain);
//!   - stage-less, a terminal is recognized ONLY when it carries an
//!     argument that mentions `it` (`xs.any(it.v > 3)`) — `it` is
//!     unbound outside a chain, so no valid user call looks like
//!     this — or when it is `each` with a block argument (no user
//!     surface takes a block).
//!   Bare `xs.sum()` / `xs.first()` therefore stay ordinary method
//!   calls (`first` stage-less is just `xs.get(0) or …`).
//!
//! The element-valued fallible terminals (`first`/`find`/`min`/`max`)
//! reuse the source's OWN fallible `get`: the loop finds an index,
//! and the chain's value is `src.get(idx)` — a miss produces the
//! ordinary IndexError, so `or raise` / `or fallback` / `or
//! handler(err)` all work with zero new error machinery. This is
//! also why they do not compose with `map` (the `or` fallback would
//! need the mapped type while `get` yields the source type): project
//! AFTER the find, on the returned element. A mapped find is left
//! unrecognized and fails with the ordinary no-method diagnostic.

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

/// One elementwise stage.
enum Stage {
    Filter(Expr),
    Map(Expr),
}

/// A chain's shape: the source receiver, the accumulated stages, and
/// the terminal.
struct Chain {
    src: Expr,
    stages: Vec<Stage>,
    terminal: String,
    term_args: Vec<Expr>,
    /// The fallible element fetch the loop rides. `"get"` for a bare
    /// source (a `@form(vec)`, whose `get(i: Int)` this loop was built
    /// around); `"entry_at"` when the source is anchored on `.entries`
    /// (a `@form(hashmap)`, whose `entry_at(i: Int)` has the identical
    /// `Int -> T fallible(IndexError)` shape and walks occupied slots).
    /// Chosen syntactically here — chains desugar before typecheck — so
    /// an accessor that does not resolve on the real source form fails
    /// with the ordinary no-method diagnostic at the chain's site.
    accessor: &'static str,
}

const TERMINALS: &[&str] = &[
    "count", "into", "sum", "any", "all", "first", "find", "min", "max",
    "each",
];

/// Does this expression tree mention the identifier `it`? Used for
/// the stage-less recognition gate: `it` is unbound outside a chain,
/// so an argument that mentions it cannot be a valid ordinary call.
/// Conservative direction: an unrecognized Expr variant returns
/// false, which merely leaves the call as an ordinary method call —
/// and if it truly used `it`, typecheck rejects the unbound name.
fn mentions_it(e: &Expr) -> bool {
    match e {
        Expr::Ident(i) => i.name == "it",
        Expr::Field { receiver, .. } => mentions_it(receiver),
        Expr::Call { callee, args, .. } => {
            mentions_it(callee) || args.iter().any(mentions_it)
        }
        Expr::Binary { left, right, .. } => {
            mentions_it(left) || mentions_it(right)
        }
        Expr::Unary { operand, .. } => mentions_it(operand),
        Expr::Index { receiver, index, .. } => {
            mentions_it(receiver) || mentions_it(index)
        }
        _ => false,
    }
}

/// Peel `src.filter(p).map(f).<terminal>(…)` into its parts. `None`
/// when this is not a recognized chain — an ordinary method call.
fn peel(e: &Expr) -> Option<Chain> {
    let Expr::Call { callee, args, .. } = e else {
        return None;
    };
    let Expr::Field { receiver, name, .. } = callee.as_ref() else {
        return None;
    };
    if !TERMINALS.contains(&name.name.as_str()) {
        return None;
    }
    let terminal = name.name.clone();
    let term_args = args.clone();

    // Walk down the stages.
    let mut stages = Vec::new();
    let mut cur = receiver.as_ref();
    loop {
        let Expr::Call { callee, args, .. } = cur else { break };
        let Expr::Field { receiver, name, .. } = callee.as_ref() else {
            break;
        };
        let stage = match name.name.as_str() {
            "filter" if args.len() == 1 => Stage::Filter(args[0].clone()),
            "map" if args.len() == 1 => Stage::Map(args[0].clone()),
            _ => break,
        };
        stages.push(stage);
        cur = receiver.as_ref();
    }
    stages.reverse();

    let has_map = stages.iter().any(|s| matches!(s, Stage::Map(_)));

    // Arity / shape gates per terminal. Anything that does not match
    // is left as an ordinary method call (fail closed: user facade
    // methods keep resolving; a genuine mistake gets the ordinary
    // no-method / arity diagnostic).
    let shape_ok = match terminal.as_str() {
        "count" | "sum" | "first" => term_args.is_empty(),
        "into" => term_args.len() == 1,
        "any" | "find" => term_args.len() <= 1,
        "all" => term_args.len() == 1,
        "min" | "max" => term_args.len() <= 1,
        "each" => {
            term_args.len() == 1
                && matches!(term_args[0], Expr::Block(_))
        }
        _ => false,
    };
    if !shape_ok {
        return None;
    }
    // The element-valued fallible terminals return via the source's
    // `get`, so a mapped element has nowhere to live — see module
    // docs. Not recognized; project after the find instead.
    if has_map && matches!(terminal.as_str(), "first" | "find" | "min" | "max")
    {
        return None;
    }

    // Recognition gate.
    let recognized = if !stages.is_empty() {
        true
    } else {
        match terminal.as_str() {
            // A stage-less predicate/key form is unambiguous exactly
            // when its argument mentions `it`.
            "any" | "all" | "find" | "min" | "max" => {
                term_args.first().map_or(false, mentions_it)
            }
            // No user surface takes a block argument.
            "each" => true,
            _ => false,
        }
    };
    if !recognized {
        return None;
    }

    // The source form's element cursor. A source anchored on the
    // `@form` iteration pseudo-fields picks the matching accessor:
    // `.entries` → a hashmap's `entry_at`, `.items` → a vec's `get`
    // (also the bare-source default). Anchored, the pseudo-field is
    // stripped so the accessor is called on the collection itself.
    // `for e in m.entries` already recognizes these pseudo-fields by
    // shape in check.rs; this mirrors that so a chain reads the same.
    let (src, accessor) = match cur {
        Expr::Field { receiver, name, .. } if name.name == "entries" => {
            ((**receiver).clone(), "entry_at")
        }
        Expr::Field { receiver, name, .. } if name.name == "items" => {
            ((**receiver).clone(), "get")
        }
        other => (other.clone(), "get"),
    };

    Some(Chain { src, stages, terminal, term_args, accessor })
}

// ---- `it` substitution --------------------------------------------
//
// Stage/terminal expressions are written against the literal name
// `it`; the lowering binds each chain's element to a numbered local
// (`__hale_it<uid>`) so nested chains cannot capture each other's
// element. The walkers below rewrite `it` → that local. An Expr
// variant the walker does not descend into leaves any inner `it`
// untouched, which fails CLOSED: the unbound name is a typecheck
// error at the chain's site, never a silently wrong binding.

fn subst_expr(e: &mut Expr, to: &str) {
    match e {
        Expr::Ident(i) if i.name == "it" => {
            i.name = to.to_string();
        }
        Expr::Field { receiver, .. } => subst_expr(receiver, to),
        Expr::Call { callee, args, .. } => {
            subst_expr(callee, to);
            for a in args {
                subst_expr(a, to);
            }
        }
        Expr::Binary { left, right, .. } => {
            subst_expr(left, to);
            subst_expr(right, to);
        }
        Expr::Unary { operand, .. } => subst_expr(operand, to),
        Expr::Index { receiver, index, .. } => {
            subst_expr(receiver, to);
            subst_expr(index, to);
        }
        Expr::Path2 { receiver, .. } => subst_expr(receiver, to),
        Expr::Block(b) => subst_block(b, to),
        Expr::Or { inner, disposition, .. } => {
            subst_expr(inner, to);
            match disposition {
                OrDisposition::Substitute(e) => subst_expr(e, to),
                OrDisposition::Fail(e, _) => subst_expr(e, to),
                _ => {}
            }
        }
        Expr::Struct { inits, .. } => {
            for init in inits {
                subst_expr(&mut init.value, to);
            }
        }
        Expr::Array(parts, _) => {
            for p in parts {
                subst_expr(p, to);
            }
        }
        _ => {}
    }
}

fn subst_block(b: &mut Block, to: &str) {
    for s in &mut b.stmts {
        subst_stmt(s, to);
    }
    if let Some(t) = &mut b.tail {
        subst_expr(t, to);
    }
}

fn subst_stmt(s: &mut Stmt, to: &str) {
    match s {
        Stmt::Let { value, .. } => subst_expr(value, to),
        Stmt::Assign { value, target, .. } => {
            subst_expr(value, to);
            // `it` cannot be assigned (it names an element copy the
            // loop rebinds) — but an index expression in the lvalue
            // tail may mention it. Leave the head; heads named `it`
            // become the unbound-name error, fail closed.
            let _ = target;
        }
        Stmt::Expr(e) => subst_expr(e, to),
        Stmt::While { cond, body, .. } => {
            subst_expr(cond, to);
            subst_block(body, to);
        }
        Stmt::If(i) => subst_if(i, to),
        Stmt::Return(Some(e), _) => subst_expr(e, to),
        Stmt::Fail { value, .. } => subst_expr(value, to),
        Stmt::Send { subject, value, .. } => {
            subst_expr(subject, to);
            subst_expr(value, to);
        }
        _ => {}
    }
}

fn subst_if(i: &mut IfStmt, to: &str) {
    subst_expr(&mut i.cond, to);
    subst_block(&mut i.then_block, to);
    if let Some(e) = &mut i.else_block {
        match e.as_mut() {
            ElseBranch::Else(b) => subst_block(b, to),
            ElseBranch::ElseIf(e) => subst_if(e, to),
        }
    }
}

fn expr(e: &mut Expr, n: &mut usize) {
    // The element-valued fallible terminals bind to a user-written
    // `or` (`xs.filter(p).first() or fallback`). Rewrite at the Or
    // node so the disposition moves INSIDE the lowered block, onto
    // the `src.get(idx)` tail — the shape the existing or-machinery
    // already lowers.
    if let Expr::Or { inner, disposition, span } = e {
        if let Some(c) = peel(inner) {
            if matches!(
                c.terminal.as_str(),
                "first" | "find" | "min" | "max"
            ) {
                *n += 1;
                let lowered = lower_indexed(
                    c,
                    *span,
                    *n,
                    Some(disposition.clone()),
                );
                *e = lowered;
                // The moved disposition's own sub-expressions still
                // deserve chain rewriting.
                expr(e, n);
                return;
            }
        }
    }
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
        Expr::Or { inner, .. } => expr(inner, n),
        _ => {}
    }
    if let Some(c) = peel(e) {
        *n += 1;
        let sp = e.span();
        *e = match c.terminal.as_str() {
            "first" | "find" | "min" | "max" => {
                lower_indexed(c, sp, *n, None)
            }
            _ => lower(c, sp, *n),
        };
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
fn boolean(v: bool, sp: Span) -> Expr {
    Expr::Literal(Literal::Bool(v), sp)
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
fn not(e: Expr, sp: Span) -> Expr {
    Expr::Unary { op: UnaryOp::Not, operand: Box::new(e), span: sp }
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
fn let_val(n: &str, v: Expr, sp: Span) -> Stmt {
    Stmt::Let {
        is_mut: false,
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
fn if_then(cond: Expr, then: Vec<Stmt>, sp: Span) -> Stmt {
    Stmt::If(IfStmt {
        cond,
        then_block: Block { stmts: then, tail: None, span: sp },
        else_block: None,
        span: sp,
    })
}
fn get_or_break(
    src: &Expr,
    idx_var: &str,
    bind: &str,
    accessor: &str,
    sp: Span,
) -> Stmt {
    // `let <bind> = src.<accessor>(i) or { break; };` — the diverging
    // fallback is why #353's diverging-`or` fix had to land first:
    // there is no typed default to invent for an arbitrary element
    // type. `accessor` is `get` for a vec source and `entry_at` for a
    // hashmap one; both are `Int -> T fallible(IndexError)`, so the
    // `or { break; }` exit is the same either way.
    Stmt::Let {
        is_mut: false,
        name: id(bind, sp),
        ty: None,
        value: Expr::Or {
            inner: Box::new(method(
                src.clone(),
                accessor,
                vec![var(idx_var, sp)],
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
    }
}

/// Wrap `action` in the chain's stages, innermost-out. Returns the
/// statements forming the per-element body, and binds the running
/// element name: stage k's expressions see the element as `elem`,
/// and a `map` rebinds it to a fresh local for the stages after it.
fn apply_stages(
    stages: &[Stage],
    elem0: &str,
    uid: usize,
    mut action: Vec<Stmt>,
    sp: Span,
) -> Vec<Stmt> {
    // Compute the element name seen AFTER each stage prefix.
    let mut names: Vec<String> = vec![elem0.to_string()];
    let mut mapn = 0usize;
    for s in stages {
        match s {
            Stage::Filter(_) => {
                names.push(names.last().unwrap().clone());
            }
            Stage::Map(_) => {
                mapn += 1;
                names.push(format!("__hale_m{}_{}", uid, mapn));
            }
        }
    }
    // Innermost-out: wrap the action with each stage, last first.
    for (k, s) in stages.iter().enumerate().rev() {
        let seen = names[k].clone(); // element name THIS stage reads
        match s {
            Stage::Filter(p) => {
                let mut p = p.clone();
                subst_expr(&mut p, &seen);
                action = vec![if_then(p, action, sp)];
            }
            Stage::Map(f) => {
                let mut f = f.clone();
                subst_expr(&mut f, &seen);
                let bound = names[k + 1].clone();
                let mut stmts = vec![let_val(&bound, f, sp)];
                stmts.extend(action);
                action = stmts;
            }
        }
    }
    action
}

/// The element name the TERMINAL sees (after all stages).
fn final_elem(stages: &[Stage], elem0: &str, uid: usize) -> String {
    let mapn = stages.iter().filter(|s| matches!(s, Stage::Map(_))).count();
    if mapn == 0 {
        elem0.to_string()
    } else {
        format!("__hale_m{}_{}", uid, mapn)
    }
}

/// Shared loop skeleton:
/// `let mut i = -1; while true { i = i + 1;
///  let elem = src.get(i) or { break; }; <body> }`.
///
/// The increment comes FIRST and the fallible `get` is the loop's
/// only terminator. Two properties hang on this shape:
///  - `continue` inside an `each { ... }` body skips to the next
///    element instead of spinning forever on the same index (an
///    end-of-body increment would be skipped by the `continue`);
///  - no `len()` call per iteration — `get` already bounds-checks,
///    and its `or { break; }` is the exit.
fn build_loop(
    src: &Expr,
    uid: usize,
    body: Vec<Stmt>,
    accessor: &str,
    sp: Span,
) -> Vec<Stmt> {
    let i_name = format!("__hale_chain_i{}", uid);
    let elem = format!("__hale_it{}", uid);
    let mut loop_body = vec![
        assign(
            &i_name,
            bin(BinOp::Add, var(&i_name, sp), int(1, sp), sp),
            sp,
        ),
        get_or_break(src, &i_name, &elem, accessor, sp),
    ];
    loop_body.extend(body);
    vec![
        let_mut(&i_name, int(-1, sp), sp),
        Stmt::While {
            cond: boolean(true, sp),
            body: Block { stmts: loop_body, tail: None, span: sp },
            span: sp,
        },
    ]
}

/// Value-accumulating and side-effect terminals:
/// count / into / sum / any / all / each.
fn lower(mut c: Chain, sp: Span, uid: usize) -> Expr {
    let acc_name = format!("__hale_chain_n{}", uid);
    let acc = acc_name.as_str();
    let elem0 = format!("__hale_it{}", uid);

    // Predicate-form `any(p)` is `filter(p).any()`.
    if c.terminal == "any" && !c.term_args.is_empty() {
        c.stages.push(Stage::Filter(c.term_args.remove(0)));
    }

    let fin = final_elem(&c.stages, &elem0, uid);

    // The innermost per-element action + (init, tail) for the block.
    let (action, init, tail): (Vec<Stmt>, Option<Stmt>, Option<Expr>) =
        match c.terminal.as_str() {
            "count" => (
                vec![assign(
                    acc,
                    bin(BinOp::Add, var(acc, sp), int(1, sp), sp),
                    sp,
                )],
                Some(let_mut(acc, int(0, sp), sp)),
                Some(var(acc, sp)),
            ),
            // v1: Int elements only — see module docs.
            "sum" => (
                vec![assign(
                    acc,
                    bin(BinOp::Add, var(acc, sp), var(&fin, sp), sp),
                    sp,
                )],
                Some(let_mut(acc, int(0, sp), sp)),
                Some(var(acc, sp)),
            ),
            "any" => (
                vec![assign(acc, boolean(true, sp), sp), Stmt::Break(sp)],
                Some(let_mut(acc, boolean(false, sp), sp)),
                Some(var(acc, sp)),
            ),
            "all" => {
                let mut p = c.term_args[0].clone();
                subst_expr(&mut p, &fin);
                (
                    vec![if_then(
                        not(p, sp),
                        vec![
                            assign(acc, boolean(false, sp), sp),
                            Stmt::Break(sp),
                        ],
                        sp,
                    )],
                    Some(let_mut(acc, boolean(true, sp), sp)),
                    Some(var(acc, sp)),
                )
            }
            "each" => {
                let Expr::Block(mut b) = c.term_args.remove(0) else {
                    unreachable!("peel gated each on a block arg");
                };
                subst_block(&mut b, &fin);
                let mut stmts = b.stmts;
                if let Some(t) = b.tail {
                    stmts.push(Stmt::Expr(*t));
                }
                (stmts, None, None)
            }
            // `into(target)` pushes the (mapped) element.
            _ => (
                vec![Stmt::Expr(method(
                    c.term_args
                        .first()
                        .cloned()
                        .unwrap_or_else(|| var("__hale_chain_missing", sp)),
                    "push",
                    vec![var(&fin, sp)],
                    sp,
                ))],
                None,
                None,
            ),
        };

    let body = apply_stages(&c.stages, &elem0, uid, action, sp);
    let mut stmts = Vec::new();
    if let Some(i) = init {
        stmts.push(i);
    }
    stmts.extend(build_loop(&c.src, uid, body, c.accessor, sp));
    Expr::Block(Block {
        stmts,
        tail: tail.map(Box::new),
        span: sp,
    })
}

/// Element-valued fallible terminals: first / find / min / max.
///
/// The loop resolves an INDEX; the chain's value is `src.get(idx)`.
/// idx starts at -1, so an empty result is the ordinary IndexError
/// from the source's own fallible `get` — `or` handles it with zero
/// new machinery. `disposition`, when present, is the user's `or`
/// moved onto that tail.
fn lower_indexed(
    mut c: Chain,
    sp: Span,
    uid: usize,
    disposition: Option<OrDisposition>,
) -> Expr {
    let idx_name = format!("__hale_chain_idx{}", uid);
    let idx = idx_name.as_str();
    let elem0 = format!("__hale_it{}", uid);

    // `find(p)` is `filter(p).first()`.
    if c.terminal == "find" && !c.term_args.is_empty() {
        c.stages.push(Stage::Filter(c.term_args.remove(0)));
    }

    let i_name = format!("__hale_chain_i{}", uid);
    let action: Vec<Stmt> = match c.terminal.as_str() {
        "first" | "find" => vec![
            assign(idx, var(&i_name, sp), sp),
            Stmt::Break(sp),
        ],
        // min/max: keep the index of the best element so far. The
        // best's key is re-derived through the source's own `get` —
        // its `or { break; }` can never fire (idx is a previously
        // visited index) — so no accumulator of unknown type exists.
        _ => {
            let is_min = c.terminal == "min";
            let key_of = |e: Expr, sp: Span| -> Expr {
                match c.term_args.first() {
                    Some(k) => {
                        // Key expression over the candidate.
                        let kname = format!("__hale_k{}", uid);
                        let mut ke = k.clone();
                        subst_expr(&mut ke, &kname);
                        Expr::Block(Block {
                            stmts: vec![let_val(&kname, e, sp)],
                            tail: Some(Box::new(ke)),
                            span: sp,
                        })
                    }
                    None => e,
                }
            };
            let best_bind = format!("__hale_best{}", uid);
            let cand_key = key_of(var(&elem0, sp), sp);
            let best_key = key_of(var(&best_bind, sp), sp);
            let cmp = bin(
                if is_min { BinOp::Lt } else { BinOp::Gt },
                cand_key,
                best_key,
                sp,
            );
            vec![if_then(
                bin(BinOp::Lt, var(idx, sp), int(0, sp), sp),
                vec![assign(idx, var(&i_name, sp), sp)],
                sp,
            ), if_then(
                bin(BinOp::GtEq, var(idx, sp), int(0, sp), sp),
                vec![
                    get_or_break(&c.src, idx, &best_bind, c.accessor, sp),
                    if_then(
                        cmp,
                        vec![assign(idx, var(&i_name, sp), sp)],
                        sp,
                    ),
                ],
                sp,
            )]
        }
    };

    // NOTE: the min/max action above compares even on the iteration
    // that seeded idx (best == candidate; strict compare is false, so
    // idx stays) — one redundant compare per seed keeps the shape to
    // two ifs with no else machinery.

    let body = apply_stages(&c.stages, &elem0, uid, action, sp);
    let mut stmts = vec![let_mut(idx, int(-1, sp), sp)];
    stmts.extend(build_loop(&c.src, uid, body, c.accessor, sp));

    // The element-valued result rides the same accessor the loop did —
    // `get` for a vec, `entry_at` for a hashmap — so first/find/min/max
    // return the element from the source form they actually iterated.
    let get_tail =
        method(c.src.clone(), c.accessor, vec![var(idx, sp)], sp);
    let tail = match disposition {
        Some(d) => Expr::Or {
            inner: Box::new(get_tail),
            disposition: d,
            span: sp,
        },
        None => get_tail,
    };
    Expr::Block(Block {
        stmts,
        tail: Some(Box::new(tail)),
        span: sp,
    })
}

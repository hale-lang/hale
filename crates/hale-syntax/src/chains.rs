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
//! ## Vocabulary (2026-08-04 tranche + 2026-08-11 tranche;
//! v1 was filter/count/into)
//!
//! Elementwise stages — fuse into the one loop:
//!   - `filter(pred)`  — `it`-predicate, drops non-matching elements
//!   - `map(expr)`     — `it`-expression, rebinds the element
//!   - `take(n)`       — stop the chain after n elements pass here
//!   - `skip(n)`       — drop the first n elements reaching here
//!   - `enumerate()`   — binds `idx` (0-based count of elements
//!                       reaching this stage) in later stages and
//!                       the terminal; explicit opt-in so a user's
//!                       own `idx` local is never captured silently
//!
//! Terminals:
//!   - `count()`            → Int
//!   - `into(target)`       → pushes surviving (mapped) elements
//!   - `sum()` / `sum(seed)` → `sum()` is Int; `sum(seed)` seeds the
//!                            accumulator, and the seed IS its typed
//!                            zero — `sum(0.0)` for Float elements
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
//!   - `sort_into(target, cmp?)` → push survivors, then reorder the
//!                            caller storage in place (`sort()` /
//!                            `sort_by(cmp)`) — the whole-set ops
//!                            materialize into CALLER storage, so
//!                            the chain itself still allocates
//!                            nothing
//!   - `reverse_into(target)` → push survivors, then swap ends
//!                            inward on the caller storage
//!   - `group_count_into(target, key?)` → one hashmap `bump(key)`
//!                            per survivor (increment-or-init); the
//!                            bare form keys on the element itself
//!
//! A min/max KEY mentioning `idx` is left unrecognized: the best
//! element's key is re-derived from `src.get(idx)` at compare time,
//! and its enumerate count is not recoverable from the element —
//! rejecting beats silently comparing with the wrong counter.
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
//!   calls (`first` stage-less is just `xs.get(0) or …`). The
//!   whole-set terminals (`sort_into` / `reverse_into` /
//!   `group_count_into`) are the exception: recognized stage-less
//!   too — their compound names are this vocabulary's own, so
//!   `xs.sort_into(sorted)` works as written.
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
    /// 2026-08-11 tranche. `take(n)`: stop the whole chain once n
    /// elements have passed this point; `skip(n)`: drop the first n
    /// that reach it. Both count elements ARRIVING at their own
    /// stage position, so `filter(p).skip(2)` skips the first two
    /// matches, not the first two source elements. Their counters
    /// are pre-loop inits (see `apply_stages`).
    Take(Expr),
    Skip(Expr),
    /// `enumerate()`: binds `idx` — the 0-based count of elements
    /// reaching this stage — in every LATER stage expression and in
    /// the terminal. An explicit opt-in stage (not an always-bound
    /// name) so a user's own `idx` local is never captured by a
    /// chain that didn't ask.
    Enumerate,
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
    "each", "sort_into", "reverse_into", "group_count_into",
];

/// Does this expression tree mention the identifier `name`? Used for
/// the stage-less recognition gate: `it` is unbound outside a chain,
/// so an argument that mentions it cannot be a valid ordinary call.
/// Conservative direction: an unrecognized Expr variant returns
/// false, which merely leaves the call as an ordinary method call —
/// and if it truly used `it`, typecheck rejects the unbound name.
fn mentions_ident(e: &Expr, name: &str) -> bool {
    fn block_mentions(b: &Block, name: &str) -> bool {
        b.stmts.iter().any(|s| match s {
            Stmt::Let { value, .. }
            | Stmt::Assign { value, .. }
            | Stmt::Fail { value, .. } => mentions_ident(value, name),
            Stmt::Expr(e) | Stmt::Return(Some(e), _) => {
                mentions_ident(e, name)
            }
            Stmt::If(i) => if_mentions(i, name),
            Stmt::While { cond, body, .. } => {
                mentions_ident(cond, name) || block_mentions(body, name)
            }
            _ => false,
        }) || b.tail.as_ref().map_or(false, |t| mentions_ident(t, name))
    }
    fn if_mentions(i: &IfStmt, name: &str) -> bool {
        mentions_ident(&i.cond, name)
            || block_mentions(&i.then_block, name)
            || i.else_block.as_ref().map_or(false, |e| match e.as_ref() {
                ElseBranch::Else(b) => block_mentions(b, name),
                ElseBranch::ElseIf(ei) => if_mentions(ei, name),
            })
    }
    match e {
        Expr::Ident(i) => i.name == name,
        Expr::Field { receiver, .. } => mentions_ident(receiver, name),
        Expr::Call { callee, args, .. } => {
            mentions_ident(callee, name)
                || args.iter().any(|a| mentions_ident(a, name))
        }
        Expr::Binary { left, right, .. } => {
            mentions_ident(left, name) || mentions_ident(right, name)
        }
        Expr::Unary { operand, .. } => mentions_ident(operand, name),
        Expr::Index { receiver, index, .. } => {
            mentions_ident(receiver, name) || mentions_ident(index, name)
        }
        // A conditional key — `group_count_into(t, if it % 2 == 0 {
        // "even" } else { "odd" })` — must count as an it-mention for
        // the stage-less recognition gate, same reach as the subst
        // walker (which already descends these).
        Expr::If(i) => if_mentions(i, name),
        Expr::Match(m) => {
            mentions_ident(&m.scrutinee, name)
                || m.arms.iter().any(|arm| {
                    arm.guard
                        .as_ref()
                        .map_or(false, |g| mentions_ident(g, name))
                        || match &arm.body {
                            MatchArmBody::Expr(e) => {
                                mentions_ident(e, name)
                            }
                            MatchArmBody::Block(b) => {
                                block_mentions(b, name)
                            }
                        }
                })
        }
        Expr::Block(b) => block_mentions(b, name),
        Expr::Struct { inits, .. } => {
            inits.iter().any(|i| mentions_ident(&i.value, name))
        }
        Expr::Tuple(parts, _) | Expr::Array(parts, _) => {
            parts.iter().any(|p| mentions_ident(p, name))
        }
        Expr::Or { inner, disposition, .. } => {
            mentions_ident(inner, name)
                || match disposition {
                    OrDisposition::Substitute(e)
                    | OrDisposition::Fail(e, _) => {
                        mentions_ident(e, name)
                    }
                    _ => false,
                }
        }
        _ => false,
    }
}

fn mentions_it(e: &Expr) -> bool {
    mentions_ident(e, "it")
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
            "take" if args.len() == 1 => Stage::Take(args[0].clone()),
            "skip" if args.len() == 1 => Stage::Skip(args[0].clone()),
            "enumerate" if args.is_empty() => Stage::Enumerate,
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
        "count" | "first" => term_args.is_empty(),
        // `sum()` = Int accumulator; `sum(seed)` = the seed IS the
        // accumulator's typed zero (`sum(0.0)` for Float elements).
        "sum" => term_args.len() <= 1,
        "into" | "reverse_into" => term_args.len() == 1,
        "any" | "find" => term_args.len() <= 1,
        "all" => term_args.len() == 1,
        "min" | "max" => term_args.len() <= 1,
        // `sort_into(target)` rides the vec's own `sort()`;
        // `sort_into(target, cmp)` rides `sort_by(cmp)`.
        "sort_into" => (1..=2).contains(&term_args.len()),
        // `group_count_into(target, key?)` rides the hashmap's
        // `bump`; bare form uses the element itself as the key.
        "group_count_into" => (1..=2).contains(&term_args.len()),
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
    // A min/max KEY that mentions `idx` cannot lower: the best
    // element's key is re-derived from `src.get(idx)` at compare
    // time, and its enumerate count is not recoverable from the
    // element. Left unrecognized (ordinary diagnostics), never
    // silently miscompared.
    if matches!(terminal.as_str(), "min" | "max")
        && stages.iter().any(|s| matches!(s, Stage::Enumerate))
        && term_args
            .first()
            .map_or(false, |a| mentions_ident(a, "idx"))
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
            // The whole-set terminals are recognized even stage-less:
            // `xs.sort_into(sorted)` is their natural spelling, and
            // the compound `*_into` names are this vocabulary's own —
            // unlike bare `into`, no plausible user facade carries
            // them. Same reasoning as `each`'s block argument.
            "sort_into" | "reverse_into" | "group_count_into" => true,
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

fn subst_expr(e: &mut Expr, from: &str, to: &str) {
    match e {
        Expr::Ident(i) if i.name == from => {
            i.name = to.to_string();
        }
        Expr::Field { receiver, .. } => subst_expr(receiver, from, to),
        Expr::Call { callee, args, .. } => {
            subst_expr(callee, from, to);
            for a in args {
                subst_expr(a, from, to);
            }
        }
        Expr::Binary { left, right, .. } => {
            subst_expr(left, from, to);
            subst_expr(right, from, to);
        }
        Expr::Unary { operand, .. } => subst_expr(operand, from, to),
        Expr::Index { receiver, index, .. } => {
            subst_expr(receiver, from, to);
            subst_expr(index, from, to);
        }
        Expr::Path2 { receiver, .. } => subst_expr(receiver, from, to),
        Expr::Block(b) => subst_block(b, from, to),
        // A conditional projection — `map(if it > 0 { "pos" } else
        // { "neg" })` — is an if-EXPRESSION; descend so its arms see
        // the element.
        Expr::If(i) => subst_if(i, from, to),
        Expr::Match(m) => {
            subst_expr(&mut m.scrutinee, from, to);
            for arm in &mut m.arms {
                if let Some(g) = &mut arm.guard {
                    subst_expr(g, from, to);
                }
                match &mut arm.body {
                    MatchArmBody::Expr(e) => subst_expr(e, from, to),
                    MatchArmBody::Block(b) => subst_block(b, from, to),
                }
            }
        }
        Expr::Tuple(parts, _) => {
            for p in parts {
                subst_expr(p, from, to);
            }
        }
        Expr::Or { inner, disposition, .. } => {
            subst_expr(inner, from, to);
            match disposition {
                OrDisposition::Substitute(e) => subst_expr(e, from, to),
                OrDisposition::Fail(e, _) => subst_expr(e, from, to),
                _ => {}
            }
        }
        Expr::Struct { inits, .. } => {
            for init in inits {
                subst_expr(&mut init.value, from, to);
            }
        }
        Expr::Array(parts, _) => {
            for p in parts {
                subst_expr(p, from, to);
            }
        }
        _ => {}
    }
}

fn subst_block(b: &mut Block, from: &str, to: &str) {
    for s in &mut b.stmts {
        subst_stmt(s, from, to);
    }
    if let Some(t) = &mut b.tail {
        subst_expr(t, from, to);
    }
}

fn subst_stmt(s: &mut Stmt, from: &str, to: &str) {
    match s {
        Stmt::Let { value, .. } => subst_expr(value, from, to),
        Stmt::Assign { value, target, .. } => {
            subst_expr(value, from, to);
            // `it` cannot be assigned (it names an element copy the
            // loop rebinds) — but an index expression in the lvalue
            // tail may mention it. Leave the head; heads named `it`
            // become the unbound-name error, fail closed.
            let _ = target;
        }
        Stmt::Expr(e) => subst_expr(e, from, to),
        Stmt::While { cond, body, .. } => {
            subst_expr(cond, from, to);
            subst_block(body, from, to);
        }
        Stmt::If(i) => subst_if(i, from, to),
        Stmt::Return(Some(e), _) => subst_expr(e, from, to),
        Stmt::Fail { value, .. } => subst_expr(value, from, to),
        Stmt::Send { subject, value, .. } => {
            subst_expr(subject, from, to);
            subst_expr(value, from, to);
        }
        _ => {}
    }
}

fn subst_if(i: &mut IfStmt, from: &str, to: &str) {
    subst_expr(&mut i.cond, from, to);
    subst_block(&mut i.then_block, from, to);
    if let Some(e) = &mut i.else_block {
        match e.as_mut() {
            ElseBranch::Else(b) => subst_block(b, from, to),
            ElseBranch::ElseIf(e) => subst_if(e, from, to),
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
/// pre-loop init statements the stages need (take/skip/enumerate
/// counters — their state lives across iterations) and the
/// statements forming the per-element body, and binds the running
/// element name: stage k's expressions see the element as `elem`,
/// and a `map` rebinds it to a fresh local for the stages after it.
fn apply_stages(
    stages: &[Stage],
    elem0: &str,
    uid: usize,
    mut action: Vec<Stmt>,
    sp: Span,
) -> (Vec<Stmt>, Vec<Stmt>) {
    // Compute the element name seen AFTER each stage prefix.
    let mut names: Vec<String> = vec![elem0.to_string()];
    let mut mapn = 0usize;
    for s in stages {
        match s {
            Stage::Map(_) => {
                mapn += 1;
                names.push(format!("__hale_m{}_{}", uid, mapn));
            }
            // The positional stages read and pass on the same element.
            Stage::Filter(_)
            | Stage::Take(_)
            | Stage::Skip(_)
            | Stage::Enumerate => {
                names.push(names.last().unwrap().clone());
            }
        }
    }
    // Innermost-out: wrap the action with each stage, last first.
    let mut inits: Vec<Stmt> = Vec::new();
    for (k, s) in stages.iter().enumerate().rev() {
        let seen = names[k].clone(); // element name THIS stage reads
        match s {
            Stage::Filter(p) => {
                let mut p = p.clone();
                subst_expr(&mut p, "it", &seen);
                action = vec![if_then(p, action, sp)];
            }
            Stage::Map(f) => {
                let mut f = f.clone();
                subst_expr(&mut f, "it", &seen);
                let bound = names[k + 1].clone();
                let mut stmts = vec![let_val(&bound, f, sp)];
                stmts.extend(action);
                action = stmts;
            }
            // `take(n)`: break the whole loop once n elements have
            // passed this point. Saturation ends the chain — no
            // later element could contribute anything. The limit is
            // evaluated ONCE, before the loop.
            Stage::Take(nexpr) => {
                let lim = format!("__hale_tk{}_{}", uid, k);
                let ctr = format!("__hale_tkc{}_{}", uid, k);
                inits.push(let_val(&lim, nexpr.clone(), sp));
                inits.push(let_mut(&ctr, int(0, sp), sp));
                let mut stmts = vec![
                    if_then(
                        bin(BinOp::GtEq, var(&ctr, sp), var(&lim, sp), sp),
                        vec![Stmt::Break(sp)],
                        sp,
                    ),
                    assign(
                        &ctr,
                        bin(BinOp::Add, var(&ctr, sp), int(1, sp), sp),
                        sp,
                    ),
                ];
                stmts.extend(action);
                action = stmts;
            }
            // `skip(n)`: count every element reaching this point and
            // run the rest only from the (n+1)-th on.
            Stage::Skip(nexpr) => {
                let lim = format!("__hale_sk{}_{}", uid, k);
                let ctr = format!("__hale_skc{}_{}", uid, k);
                inits.push(let_val(&lim, nexpr.clone(), sp));
                inits.push(let_mut(&ctr, int(0, sp), sp));
                let mut stmts = vec![assign(
                    &ctr,
                    bin(BinOp::Add, var(&ctr, sp), int(1, sp), sp),
                    sp,
                )];
                stmts.push(if_then(
                    bin(BinOp::Gt, var(&ctr, sp), var(&lim, sp), sp),
                    action,
                    sp,
                ));
                action = stmts;
            }
            // `enumerate()`: bind `idx` in everything AFTER this
            // stage. Wrapping runs innermost-out, so a later
            // enumerate has already substituted its own scope's
            // `idx` by the time an earlier one wraps — shadowing
            // falls out of the ordering.
            Stage::Enumerate => {
                let ctr = format!("__hale_en{}_{}", uid, k);
                inits.push(let_mut(&ctr, int(-1, sp), sp));
                for a in &mut action {
                    subst_stmt(a, "idx", &ctr);
                }
                let mut stmts = vec![assign(
                    &ctr,
                    bin(BinOp::Add, var(&ctr, sp), int(1, sp), sp),
                    sp,
                )];
                stmts.extend(action);
                action = stmts;
            }
        }
    }
    (inits, action)
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

    // The innermost per-element action + (init, tail, post) for the
    // block. `post` runs after the loop — the whole-set terminals
    // (sort_into / reverse_into) do their reordering there, on the
    // caller storage the loop filled.
    let (action, init, tail, post): (
        Vec<Stmt>,
        Option<Stmt>,
        Option<Expr>,
        Vec<Stmt>,
    ) =
        match c.terminal.as_str() {
            "count" => (
                vec![assign(
                    acc,
                    bin(BinOp::Add, var(acc, sp), int(1, sp), sp),
                    sp,
                )],
                Some(let_mut(acc, int(0, sp), sp)),
                Some(var(acc, sp)),
                Vec::new(),
            ),
            // `sum()` = Int accumulator zero; `sum(seed)` = the seed
            // is the accumulator's initial value AND its typed zero
            // (`sum(0.0)` for Float elements) — evaluated once,
            // before the loop, so chains stay pre-typecheck-safe.
            "sum" => (
                vec![assign(
                    acc,
                    bin(BinOp::Add, var(acc, sp), var(&fin, sp), sp),
                    sp,
                )],
                Some(let_mut(
                    acc,
                    c.term_args
                        .first()
                        .cloned()
                        .unwrap_or_else(|| int(0, sp)),
                    sp,
                )),
                Some(var(acc, sp)),
                Vec::new(),
            ),
            "any" => (
                vec![assign(acc, boolean(true, sp), sp), Stmt::Break(sp)],
                Some(let_mut(acc, boolean(false, sp), sp)),
                Some(var(acc, sp)),
                Vec::new(),
            ),
            "all" => {
                let mut p = c.term_args[0].clone();
                subst_expr(&mut p, "it", &fin);
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
                    Vec::new(),
                )
            }
            "each" => {
                let Expr::Block(mut b) = c.term_args.remove(0) else {
                    unreachable!("peel gated each on a block arg");
                };
                subst_block(&mut b, "it", &fin);
                let mut stmts = b.stmts;
                if let Some(t) = b.tail {
                    stmts.push(Stmt::Expr(*t));
                }
                (stmts, None, None, Vec::new())
            }
            // `sort_into(target, cmp?)`: push each surviving element,
            // then reorder the caller storage in place — the vec's
            // own `sort()` (primitive ordering) or `sort_by(cmp)`.
            "sort_into" => {
                let target = c.term_args[0].clone();
                let post = match c.term_args.get(1) {
                    Some(cmp) => vec![Stmt::Expr(method(
                        target.clone(),
                        "sort_by",
                        vec![cmp.clone()],
                        sp,
                    ))],
                    None => vec![Stmt::Expr(method(
                        target.clone(),
                        "sort",
                        Vec::new(),
                        sp,
                    ))],
                };
                (
                    vec![Stmt::Expr(method(
                        target,
                        "push",
                        vec![var(&fin, sp)],
                        sp,
                    ))],
                    None,
                    None,
                    post,
                )
            }
            // `reverse_into(target)`: push, then swap ends inward.
            // The `get` misses cannot fire (both indices are in
            // bounds by construction) and `set`'s error channel is
            // discarded for the same reason.
            "reverse_into" => {
                let target = c.term_args[0].clone();
                let l = format!("__hale_rl{}", uid);
                let r = format!("__hale_rr{}", uid);
                let a = format!("__hale_ra{}", uid);
                let b = format!("__hale_rb{}", uid);
                let or_discard = |inner: Expr| {
                    Stmt::Expr(Expr::Or {
                        inner: Box::new(inner),
                        disposition: OrDisposition::Discard(sp),
                        span: sp,
                    })
                };
                let swap_body = vec![
                    get_or_break(&target, &l, &a, "get", sp),
                    get_or_break(&target, &r, &b, "get", sp),
                    or_discard(method(
                        target.clone(),
                        "set",
                        vec![var(&l, sp), var(&b, sp)],
                        sp,
                    )),
                    or_discard(method(
                        target.clone(),
                        "set",
                        vec![var(&r, sp), var(&a, sp)],
                        sp,
                    )),
                    assign(
                        &l,
                        bin(BinOp::Add, var(&l, sp), int(1, sp), sp),
                        sp,
                    ),
                    assign(
                        &r,
                        bin(BinOp::Sub, var(&r, sp), int(1, sp), sp),
                        sp,
                    ),
                ];
                let post = vec![
                    let_mut(&l, int(0, sp), sp),
                    let_mut(
                        &r,
                        bin(
                            BinOp::Sub,
                            method(target.clone(), "len", Vec::new(), sp),
                            int(1, sp),
                            sp,
                        ),
                        sp,
                    ),
                    Stmt::While {
                        cond: bin(BinOp::Lt, var(&l, sp), var(&r, sp), sp),
                        body: Block {
                            stmts: swap_body,
                            tail: None,
                            span: sp,
                        },
                        span: sp,
                    },
                ];
                (
                    vec![Stmt::Expr(method(
                        target,
                        "push",
                        vec![var(&fin, sp)],
                        sp,
                    ))],
                    None,
                    None,
                    post,
                )
            }
            // `group_count_into(target, key?)`: one `bump` per
            // surviving element — the hashmap's increment-or-init
            // counter primitive. Bare form keys on the element
            // itself.
            "group_count_into" => {
                let target = c.term_args[0].clone();
                let key = match c.term_args.get(1) {
                    Some(k) => {
                        let mut k = k.clone();
                        subst_expr(&mut k, "it", &fin);
                        k
                    }
                    None => var(&fin, sp),
                };
                (
                    vec![Stmt::Expr(method(
                        target,
                        "bump",
                        vec![key],
                        sp,
                    ))],
                    None,
                    None,
                    Vec::new(),
                )
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
                Vec::new(),
            ),
        };

    let (stage_inits, body) =
        apply_stages(&c.stages, &elem0, uid, action, sp);
    let mut stmts = Vec::new();
    if let Some(i) = init {
        stmts.push(i);
    }
    stmts.extend(stage_inits);
    stmts.extend(build_loop(&c.src, uid, body, c.accessor, sp));
    stmts.extend(post);
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
                        subst_expr(&mut ke, "it", &kname);
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

    let (stage_inits, body) =
        apply_stages(&c.stages, &elem0, uid, action, sp);
    let mut stmts = vec![let_mut(idx, int(-1, sp), sp)];
    stmts.extend(stage_inits);
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

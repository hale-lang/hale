//! GH #2162 (F.65): a checked sum of a narrower integer accumulates
//! wide and is checked once.
//!
//! `s = s + x` over a `U32` checks every add (`llvm.uadd.with.overflow`)
//! and branches to `lotus_fixed_violation` on a carry: a side exit in
//! every iteration, which LLVM 18's loop vectorizer does not take, so a
//! checked reduction over `I32` … `U8` stayed scalar (F.64). Here the loop
//! keeps the sum in an `i64` of its own instead: each term is
//! sign- or zero-extended and added with a plain `add`, which cannot
//! wrap for any term count this lowering admits, and the total is held to
//! the element type's range once, at the loop's exit, where it is
//! narrowed back into the accumulator. The body has no exit of its own
//! left, so the add is a reduction the vectorizer widens.
//!
//! A term that is one `+`, `-` or `*` of two operands of the
//! accumulator's type (an element chain's `.map(it * 3)` before its
//! `.sum(U32(0))`, or `s = s + x * 3` written out) is computed wide too:
//! two 32-bit operands' product fits in 64 bits. Whether it fits the
//! element type is not branched on per element: it is or-ed into a flag,
//! itself a reduction, and the flag is a violation at the exit, before the
//! sum's own check, named as the operator it was (a multiply's overflow).
//!
//! What changes is where a violation is reported: at the end of the loop
//! rather than at the element that crossed the bound. A sum that leaves
//! the range and comes back into it is the mathematical sum, in range, and
//! is no violation (spec/semantics.md § Checked sums of narrow integers).
//!
//! The loop must be one where that is all that changes, so it is admitted
//! by its shape alone, before any type is read ([`Shape::of`]):
//!
//! - it walks a `bounded[T; N]` or a fixed array, by `for` or as an
//!   element chain's loop (`src.get(i) or { break; }` with `i` stepping by
//!   one): its term count is at most `N` per update, so the wide total
//!   cannot wrap (`N` times the update count below 2^32);
//! - its body does nothing observable but compute: locals, `if`, `break`,
//!   `continue`, operators, casts, the natively lowered `T::f` intrinsics
//!   and `len` / `min` / `max` / `abs`. No call into the program or the
//!   runtime, no loop, no `return`, `raise` or `fail`, no write to
//!   anything but a local: the only thing that runs after an overflowing
//!   element that did not run before is arithmetic, which a violation
//!   at the exit makes unobservable;
//! - the accumulator is a local declared before the loop, every mention
//!   of it inside is an update `s = s + e`, `s = e + s` or `s += e` with
//!   `e` not naming it, and the loop's header does not name it.
//!
//! Its type is the checker's (`Cx::place_ty` of an update's target, GH
//! #2111). Anything else lowers as before, each add checked where it
//! happens: a product, `Int` and `U64` (no wider type), a sum whose
//! running value the loop reads, a `vec` or a range (no static term
//! count), a body with a call.

use std::collections::BTreeMap;

use hale_syntax::ast::{AssignOp, BinOp, Block, ElseBranch, Expr, IfStmt, LValue, Literal, OrDisposition, PrimType, Stmt, UnaryOp};
use inkwell::values::{FunctionValue, IntValue, PointerValue};
use inkwell::IntPredicate as IP;

use crate::codegen::{BlockEnd, CodegenError, CodegenTy, Cx, Scope};
use crate::stdlib::fixed::Violation;

fn emit<T>(r: Result<T, inkwell::builder::BuilderError>) -> Result<T, CodegenError> {
    r.map_err(|e| CodegenError::LlvmEmit(e.to_string()))
}

/// The most terms (iterations times updates an iteration) a wide total
/// takes: below 2^32, each term inside a 32-bit range, the total stays
/// inside an `i64` (and a `u64`).
const MAX_TERMS: u128 = 1 << 32;

/// What a loop's syntax admits, before a type is read.
#[derive(Debug, Default)]
pub(crate) struct Shape<'a> {
    /// The collection the loop walks (a `for`'s iterand, a chain's
    /// source): its type, the checker's, bounds the term count.
    pub(crate) walks: Option<&'a Expr>,
    /// Each accumulator: its updates, each its target and its term.
    pub(crate) accs: BTreeMap<String, Vec<(&'a LValue, &'a Expr)>>,
    /// A `let` whose only use is an accumulator's whole term, with the
    /// operator it binds, and that accumulator: computed wide.
    pub(crate) lets: BTreeMap<String, (&'a Expr, String)>,
}

impl<'a> Shape<'a> {
    /// The shape of `stmt`, a loop: `None` unless it is one this lowering
    /// takes, with at least one accumulator.
    pub(crate) fn of(stmt: &'a Stmt) -> Option<Shape<'a>> {
        let (header, walks, body, skip, element): (&Expr, &Expr, &Block, usize, Option<&str>) = match stmt {
            Stmt::For { name, iter, body, .. } => {
                if matches!(iter, Expr::Range { .. }) {
                    return None;
                }
                (iter, iter, body, 0, Some(&name.name))
            }
            Stmt::While { cond, body, .. } => (cond, chain_source(cond, body)?, body, 2, None),
            _ => return None,
        };
        let stmts = &body.stmts[skip..];
        if !stmts.iter().all(pure_stmt) || body.tail.is_some() {
            return None;
        }
        let mut updates: Vec<(&'a LValue, &'a Expr, usize)> = Vec::new();
        collect_updates(stmts, &mut updates);
        let mut accs: BTreeMap<String, Vec<(&'a LValue, &'a Expr)>> = BTreeMap::new();
        for (target, term, _) in &updates {
            accs.entry(target.head.name.clone()).or_default().push((target, term));
        }
        // a local of the enclosing scope (not the loop's element), named
        // in the loop only by its updates, none of whose terms names it
        accs.retain(|name, ups| {
            let named: usize = updates.iter().filter(|(t, _, _)| &t.head.name == name).map(|(_, _, m)| m).sum();
            Some(name.as_str()) != element
                && mentions_expr(header, name) == 0
                && ups.iter().all(|(_, term)| mentions_expr(term, name) == 0)
                && mentions_block(body, name) == named
        });
        if accs.is_empty() {
            return None;
        }
        let mut lets = BTreeMap::new();
        wide_lets(stmts, &accs, body, &mut lets);
        Some(Shape { walks: Some(walks), accs, lets })
    }
}

/// An element chain's loop (`crate::chains` in hale-syntax): `while true {
/// i = i + 1; let it = src.get(i) or { break; }; … }`, with `i` named
/// nowhere else, so the loop takes at most one iteration an element. Its
/// source, when it is one.
fn chain_source<'a>(cond: &'a Expr, body: &'a Block) -> Option<&'a Expr> {
    if !matches!(cond, Expr::Literal(Literal::Bool(true), _)) || body.stmts.len() < 3 {
        return None;
    }
    let Stmt::Assign { target, op: AssignOp::Eq, value, .. } = &body.stmts[0] else { return None };
    let i = &target.head.name;
    let steps = target.tail.is_empty()
        && matches!(value, Expr::Binary { op: BinOp::Add, left, right, .. }
            if matches!(left.as_ref(), Expr::Ident(l) if &l.name == i)
                && matches!(right.as_ref(), Expr::Literal(Literal::Int(1), _)));
    if !steps {
        return None;
    }
    let Stmt::Let { value: Expr::Or { inner, disposition: OrDisposition::Substitute(fallback), .. }, .. } = &body.stmts[1] else {
        return None;
    };
    let breaks = matches!(fallback.as_ref(), Expr::Block(b) if b.tail.is_none() && matches!(b.stmts.as_slice(), [Stmt::Break(_)]));
    let Expr::Call { callee, args, .. } = inner.as_ref() else { return None };
    let Expr::Field { receiver, name, .. } = callee.as_ref() else { return None };
    let indexed = matches!(args.as_slice(), [Expr::Ident(a)] if &a.name == i);
    if !breaks || name.name != "get" || !indexed || !place(receiver) {
        return None;
    }
    // `i` in its step (twice) and the `get` only; the source's root
    // never assigned
    if mentions_block(body, i) != 3 {
        return None;
    }
    let root = root_name(receiver)?;
    if body.stmts[2..].iter().any(|s| assigns(s, root)) {
        return None;
    }
    Some(receiver)
}

/// A read of a local or of a field of one (`xs`, `f.xs`, `self.xs`).
fn place(e: &Expr) -> bool {
    match e {
        Expr::Ident(_) | Expr::KwSelf(_) => true,
        Expr::Field { receiver, .. } => place(receiver),
        _ => false,
    }
}

fn root_name(e: &Expr) -> Option<&str> {
    match e {
        Expr::Ident(i) => Some(&i.name),
        Expr::KwSelf(_) => Some("self"),
        Expr::Field { receiver, .. } => root_name(receiver),
        _ => None,
    }
}

fn assigns(s: &Stmt, name: &str) -> bool {
    match s {
        Stmt::Assign { target, .. } => target.head.name == name,
        Stmt::Let { name: n, .. } => n.name == name,
        Stmt::If(i) => if_any(i, &|s| assigns(s, name)),
        Stmt::Block(b) => b.stmts.iter().any(|s| assigns(s, name)),
        _ => false,
    }
}

fn if_any(i: &IfStmt, f: &dyn Fn(&Stmt) -> bool) -> bool {
    i.then_block.stmts.iter().any(f)
        || match i.else_block.as_deref() {
            Some(ElseBranch::Else(b)) => b.stmts.iter().any(f),
            Some(ElseBranch::ElseIf(e)) => if_any(e, f),
            None => false,
        }
}

/// A statement that does nothing observable but compute.
fn pure_stmt(s: &Stmt) -> bool {
    match s {
        Stmt::Let { value, .. } => pure_expr(value),
        Stmt::Assign { target, value, .. } => target.tail.is_empty() && target.head.name != "self" && pure_expr(value),
        Stmt::If(i) => pure_if(i),
        Stmt::Block(b) => pure_block(b),
        Stmt::Break(_) | Stmt::Continue(_) => true,
        _ => false,
    }
}

fn pure_block(b: &Block) -> bool {
    b.stmts.iter().all(pure_stmt) && b.tail.as_deref().is_none_or(pure_expr)
}

fn pure_if(i: &IfStmt) -> bool {
    pure_expr(&i.cond)
        && pure_block(&i.then_block)
        && match i.else_block.as_deref() {
            Some(ElseBranch::Else(b)) => pure_block(b),
            Some(ElseBranch::ElseIf(e)) => pure_if(e),
            None => true,
        }
}

/// The fixed-width integers' own namespaces (`U32::wrapping_add`): every
/// `T::f` is lowered natively at its call, with no effect (F.64).
fn narrow_type_name(name: &str) -> bool {
    matches!(name, "I32" | "U32" | "I16" | "U16" | "I8" | "U8")
}

fn pure_callee(callee: &Expr) -> bool {
    match callee {
        // a cast (`U32(i) or wrap`, `Int(x)`) or a pure builtin
        Expr::Ident(f) => {
            matches!(
                hale_syntax::parser::primitive_from_name(&f.name),
                Some(PrimType::Int | PrimType::U64 | PrimType::I32 | PrimType::U32 | PrimType::I16 | PrimType::U16 | PrimType::I8 | PrimType::U8 | PrimType::Float)
            ) || matches!(f.name.as_str(), "len" | "min" | "max" | "abs")
        }
        Expr::Path(q) => q.segments.len() == 2 && narrow_type_name(&q.segments[0].name),
        Expr::Path2 { receiver, .. } => matches!(receiver.as_ref(), Expr::Ident(t) if narrow_type_name(&t.name)),
        _ => false,
    }
}

fn pure_expr(e: &Expr) -> bool {
    match e {
        Expr::Literal(..) | Expr::Ident(_) | Expr::KwSelf(_) | Expr::Path(_) => true,
        Expr::Binary { left, right, .. } => pure_expr(left) && pure_expr(right),
        Expr::Unary { operand, .. } => pure_expr(operand),
        Expr::Field { receiver, .. } => pure_expr(receiver),
        Expr::Index { receiver, index, .. } => pure_expr(receiver) && pure_expr(index),
        Expr::Call { callee, args, .. } => pure_callee(callee) && args.iter().all(pure_expr),
        Expr::Or { inner, disposition, .. } => {
            pure_expr(inner)
                && match disposition {
                    OrDisposition::Substitute(v) => match v.as_ref() {
                        Expr::Block(b) => pure_block(b),
                        other => pure_expr(other),
                    },
                    _ => false,
                }
        }
        _ => false,
    }
}

/// Every update `s = s + e`, `s = e + s` or `s += e` of a local in
/// `stmts`, nested `if`s and blocks included: its target, its term and
/// how many times it names `s` (two, or one for `+=`).
fn collect_updates<'a>(stmts: &'a [Stmt], out: &mut Vec<(&'a LValue, &'a Expr, usize)>) {
    for s in stmts {
        match s {
            Stmt::Assign { target, op, value, .. } if target.tail.is_empty() => {
                if let Some((term, mentions)) = update_term(&target.head.name, *op, value) {
                    out.push((target, term, mentions));
                }
            }
            Stmt::If(i) => collect_if(i, out),
            Stmt::Block(b) => collect_updates(&b.stmts, out),
            _ => {}
        }
    }
}

fn collect_if<'a>(i: &'a IfStmt, out: &mut Vec<(&'a LValue, &'a Expr, usize)>) {
    collect_updates(&i.then_block.stmts, out);
    match i.else_block.as_deref() {
        Some(ElseBranch::Else(b)) => collect_updates(&b.stmts, out),
        Some(ElseBranch::ElseIf(e)) => collect_if(e, out),
        None => {}
    }
}

/// The term of an update of `name` (`name = name + e`, `name = e + name`,
/// `name += e`) and how many times the statement names `name`.
pub(crate) fn update_term<'a>(name: &str, op: AssignOp, value: &'a Expr) -> Option<(&'a Expr, usize)> {
    let is = |e: &Expr| matches!(e, Expr::Ident(i) if i.name == name);
    match (op, value) {
        (AssignOp::PlusEq, e) => Some((e, 1)),
        (AssignOp::Eq, Expr::Binary { op: BinOp::Add, left, right, .. }) if is(left) => Some((right, 2)),
        (AssignOp::Eq, Expr::Binary { op: BinOp::Add, left, right, .. }) if is(right) => Some((left, 2)),
        _ => None,
    }
}

/// A term computed wide: one `+`, `-` or `*` of two operands, each a
/// read or a literal (no operator inside, whose own check would come
/// first).
pub(crate) fn wide_op(e: &Expr) -> Option<(BinOp, &Expr, &Expr)> {
    let operand = |e: &Expr| match e {
        Expr::Literal(Literal::Int(_), _) => true,
        Expr::Unary { op: UnaryOp::Neg, operand, .. } => matches!(operand.as_ref(), Expr::Literal(Literal::Int(_), _)),
        other => place(other),
    };
    match e {
        Expr::Binary { op: op @ (BinOp::Add | BinOp::Sub | BinOp::Mul), left, right, .. } if operand(left) && operand(right) => {
            Some((*op, left, right))
        }
        _ => None,
    }
}

/// The `let`s of `stmts` (and of the blocks inside) bound to a wide
/// operator whose name is used once, as an accumulator's whole term, by a
/// later update in the same statement list.
fn wide_lets<'a>(
    stmts: &'a [Stmt],
    accs: &BTreeMap<String, Vec<(&'a LValue, &'a Expr)>>,
    body: &'a Block,
    out: &mut BTreeMap<String, (&'a Expr, String)>,
) {
    for (k, s) in stmts.iter().enumerate() {
        match s {
            Stmt::Let { is_mut: false, name, value, .. } if wide_op(value).is_some() => {
                let used_by = stmts[k + 1..].iter().find_map(|later| match later {
                    Stmt::Assign { target, op, value: v, .. } if target.tail.is_empty() && accs.contains_key(&target.head.name) => {
                        match update_term(&target.head.name, *op, v) {
                            Some((Expr::Ident(t), _)) if t.name == name.name => Some(target.head.name.clone()),
                            _ => None,
                        }
                    }
                    _ => None,
                });
                // bound once, named once
                let once = mentions_block(body, &name.name) == 2;
                if let (Some(acc), true) = (used_by, once) {
                    out.insert(name.name.clone(), (value, acc));
                }
            }
            Stmt::If(i) => {
                wide_lets(&i.then_block.stmts, accs, body, out);
                let mut e = i.else_block.as_deref();
                while let Some(branch) = e {
                    match branch {
                        ElseBranch::Else(b) => {
                            wide_lets(&b.stmts, accs, body, out);
                            e = None;
                        }
                        ElseBranch::ElseIf(n) => {
                            wide_lets(&n.then_block.stmts, accs, body, out);
                            e = n.else_block.as_deref();
                        }
                    }
                }
            }
            Stmt::Block(b) => wide_lets(&b.stmts, accs, body, out),
            _ => {}
        }
    }
}

/// How many times `name` is named in `b`: a read, an assignment's target,
/// a binding. (Only the statements [`pure_stmt`] admits are counted in
/// full; any other counts once, which no shape here takes.)
fn mentions_block(b: &Block, name: &str) -> usize {
    b.stmts.iter().map(|s| mentions_stmt(s, name)).sum::<usize>() + b.tail.as_ref().map_or(0, |t| mentions_expr(t, name))
}

fn mentions_stmt(s: &Stmt, name: &str) -> usize {
    match s {
        Stmt::Let { name: n, value, .. } => usize::from(n.name == name) + mentions_expr(value, name),
        Stmt::Assign { target, value, .. } => usize::from(target.head.name == name) + mentions_expr(value, name),
        Stmt::If(i) => mentions_if(i, name),
        Stmt::Block(b) => mentions_block(b, name),
        Stmt::Break(_) | Stmt::Continue(_) => 0,
        _ => 1,
    }
}

fn mentions_if(i: &IfStmt, name: &str) -> usize {
    mentions_expr(&i.cond, name)
        + mentions_block(&i.then_block, name)
        + match i.else_block.as_deref() {
            Some(ElseBranch::Else(b)) => mentions_block(b, name),
            Some(ElseBranch::ElseIf(e)) => mentions_if(e, name),
            None => 0,
        }
}

fn mentions_expr(e: &Expr, name: &str) -> usize {
    match e {
        Expr::Ident(i) => usize::from(i.name == name),
        Expr::Literal(..) | Expr::KwSelf(_) | Expr::Path(_) => 0,
        Expr::Binary { left, right, .. } => mentions_expr(left, name) + mentions_expr(right, name),
        Expr::Unary { operand, .. } => mentions_expr(operand, name),
        Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. } => mentions_expr(receiver, name),
        Expr::Index { receiver, index, .. } => mentions_expr(receiver, name) + mentions_expr(index, name),
        Expr::Call { callee, args, .. } => mentions_expr(callee, name) + args.iter().map(|a| mentions_expr(a, name)).sum::<usize>(),
        Expr::Range { lo, hi, .. } => mentions_expr(lo, name) + mentions_expr(hi, name),
        Expr::Or { inner, disposition, .. } => {
            mentions_expr(inner, name)
                + match disposition {
                    OrDisposition::Substitute(v) => mentions_expr(v, name),
                    _ => 0,
                }
        }
        Expr::Block(b) => mentions_block(b, name),
        // anything else is not in a shape this lowering takes; count it as
        // a mention so nothing is admitted on a guess
        _ => 1,
    }
}

/// One accumulator a loop sums wide.
#[derive(Clone)]
pub(crate) struct WideAcc<'ctx> {
    prim: PrimType,
    /// The checker's type of the accumulator, `prim` as a layout.
    ty: CodegenTy,
    /// The accumulator's own slot, narrowed into at the exit.
    narrow: PointerValue<'ctx>,
    /// The `i64` it sums into in the loop.
    wide: PointerValue<'ctx>,
}

/// The wide sums of the loop being lowered, every choice made at its
/// preheader ([`Cx::wide_sums_enter`]).
pub(crate) struct WideSums<'ctx> {
    /// The fn the loop is in: a statement of any other is not this loop's.
    func: Option<FunctionValue<'ctx>>,
    accs: BTreeMap<String, WideAcc<'ctx>>,
    /// A wide `let`: its accumulator, the `i64` slot it is computed into
    /// and its operator's flag.
    lets: BTreeMap<String, (String, PointerValue<'ctx>, PointerValue<'ctx>)>,
    /// A term written as the operator itself (`s = s + x * 3`), by its
    /// node: its operator's flag.
    ops: BTreeMap<usize, PointerValue<'ctx>>,
    /// Each operator's out-of-range flag: its slot, the operator and the
    /// element type it is named by.
    flags: Vec<(PointerValue<'ctx>, Violation, PrimType)>,
}

fn node(e: &Expr) -> usize {
    e as *const Expr as usize
}

fn violation_of(op: BinOp) -> Violation {
    match op {
        BinOp::Add => Violation::Add,
        BinOp::Sub => Violation::Sub,
        _ => Violation::Mul,
    }
}

impl<'ctx, 'p> Cx<'ctx, 'p> {
    /// Before the loop `stmt`, at its preheader: the wide sums it takes,
    /// each started from its accumulator's value, and the flags of the
    /// operators computed wide, cleared. `None` when it takes none (the
    /// loop lowers as before).
    pub(crate) fn wide_sums_enter(&mut self, stmt: &Stmt, scope: &Scope<'ctx>) -> Result<Option<WideSums<'ctx>>, CodegenError> {
        let Some(shape) = Shape::of(stmt) else { return Ok(None) };
        // the term count: the collection's capacity, the checker's
        let cap = match shape.walks.and_then(|w| self.ty_at(w)) {
            Some(CodegenTy::Bounded(_, n) | CodegenTy::Array(_, n)) => u128::from(n),
            _ => return Ok(None),
        };
        let mut sums = WideSums {
            func: self.current_fn,
            accs: BTreeMap::new(),
            lets: BTreeMap::new(),
            ops: BTreeMap::new(),
            flags: Vec::new(),
        };
        let i64_t = self.context.i64_type();
        for (name, updates) in &shape.accs {
            if (cap + 1) * (updates.len() as u128) >= MAX_TERMS {
                continue;
            }
            let Ok(ty) = self.place_ty(updates[0].0) else { continue };
            let Some(prim) = crate::stdlib::fixed::int_prim(&ty).filter(|p| p.is_narrow_int()) else { continue };
            // the local's own slot, laid out as the checker typed it
            let Some((narrow, local)) = scope.locals.get(name) else { continue };
            if *local != ty {
                continue;
            }
            let wide = self.alloca_in_entry(i64_t.into(), &format!("{name}.wide"))?;
            let bits = prim.machine_int().expect("a machine integer").0;
            let v = emit(self.builder.build_load(self.context.custom_width_int_type(bits), *narrow, "wide.seed"))?.into_int_value();
            let v = self.resize_int(v, prim, PrimType::Int)?;
            emit(self.builder.build_store(wide, v))?;
            let acc = WideAcc { prim, ty, narrow: *narrow, wide };
            // a term written as the operator: wide when it is the
            // accumulator's type (the checker's)
            for (_, term) in updates {
                if let Some((op, _, _)) = wide_op(term) {
                    if self.ty_at(term).as_ref() == Some(&acc.ty) {
                        let flag = self.wide_flag(&mut sums, violation_of(op), prim)?;
                        sums.ops.insert(node(term), flag);
                    }
                }
            }
            sums.accs.insert(name.clone(), acc);
        }
        if sums.accs.is_empty() {
            return Ok(None);
        }
        for (name, (value, acc)) in &shape.lets {
            let Some(a) = sums.accs.get(acc) else { continue };
            let Some((op, _, _)) = wide_op(value) else { continue };
            if self.ty_at(value).as_ref() != Some(&a.ty) {
                continue;
            }
            let prim = a.prim;
            let slot = self.alloca_in_entry(i64_t.into(), &format!("{name}.wide"))?;
            let flag = self.wide_flag(&mut sums, violation_of(op), prim)?;
            sums.lets.insert(name.clone(), (acc.clone(), slot, flag));
        }
        Ok(Some(sums))
    }

    /// The flag of `what` over `p` in `sums`, made and cleared here (the
    /// preheader) the first time an operator needs it.
    fn wide_flag(&mut self, sums: &mut WideSums<'ctx>, what: Violation, p: PrimType) -> Result<PointerValue<'ctx>, CodegenError> {
        if let Some((slot, _, _)) = sums.flags.iter().find(|(_, w, q)| *w == what && *q == p) {
            return Ok(*slot);
        }
        let bool_t = self.context.bool_type();
        let slot = self.alloca_in_entry(bool_t.into(), "wide.flag")?;
        emit(self.builder.build_store(slot, bool_t.const_zero()))?;
        sums.flags.push((slot, what, p));
        Ok(slot)
    }

    /// Lower the loop `stmt` (`lower`), its checked narrow sums wide when
    /// it takes any: started at its preheader, checked and narrowed at its
    /// exit.
    pub(crate) fn lower_wide_sum_loop(
        &mut self,
        stmt: &Stmt,
        scope: &mut Scope<'ctx>,
        lower: impl FnOnce(&mut Self, &mut Scope<'ctx>) -> Result<BlockEnd, CodegenError>,
    ) -> Result<BlockEnd, CodegenError> {
        let sums = self.wide_sums_enter(stmt, scope)?;
        let Some(sums) = sums else {
            // a loop inside one is not lowered wide (none is taken), and
            // its own sums are its own
            let outer = self.wide_sums.take();
            let end = lower(self, scope);
            self.wide_sums = outer;
            return end;
        };
        let outer = self.wide_sums.replace(sums);
        let end = lower(self, scope);
        let sums = std::mem::replace(&mut self.wide_sums, outer).expect("set above");
        let end = end?;
        self.wide_sums_leave(sums, end)?;
        Ok(end)
    }

    /// A statement of a wide-sum loop's body this lowering takes over: an
    /// update of a wide accumulator, or a wide `let`. `None` for any other
    /// (lowered as before).
    pub(crate) fn lower_wide_sum_stmt(&mut self, stmt: &Stmt, scope: &Scope<'ctx>) -> Result<Option<BlockEnd>, CodegenError> {
        let Some(sums) = self.wide_sums.as_ref().filter(|s| s.func == self.current_fn) else { return Ok(None) };
        match stmt {
            Stmt::Let { name, value, .. } => {
                let Some((acc, slot, flag)) = sums.lets.get(&name.name).cloned() else { return Ok(None) };
                let a = sums.accs.get(&acc).cloned().expect("a wide let's accumulator is wide");
                let (op, l, r) = wide_op(value).expect("a wide let binds an operator");
                let t = self.wide_term_op(op, l, r, flag, &a, scope)?;
                emit(self.builder.build_store(slot, t))?;
                Ok(Some(BlockEnd::Open))
            }
            Stmt::Assign { target, op, value, .. } if target.tail.is_empty() => {
                let Some(a) = sums.accs.get(&target.head.name).cloned() else { return Ok(None) };
                let (term, _) = update_term(&target.head.name, *op, value).expect("every mention of a wide accumulator is an update");
                let t = self.wide_term(term, &a, scope)?;
                let i64_t = self.context.i64_type();
                let cur = emit(self.builder.build_load(i64_t, a.wide, "wide.acc"))?.into_int_value();
                let next = emit(self.builder.build_int_add(cur, t, "wide.add"))?;
                emit(self.builder.build_store(a.wide, next))?;
                Ok(Some(BlockEnd::Open))
            }
            _ => Ok(None),
        }
    }

    /// A term of the accumulator `a`, as an `i64`: a wide `let`'s value, an
    /// operator computed wide, or the value of any other expression,
    /// lowered as it is and extended.
    fn wide_term(&mut self, term: &Expr, a: &WideAcc<'ctx>, scope: &Scope<'ctx>) -> Result<IntValue<'ctx>, CodegenError> {
        let sums = self.wide_sums.as_ref().expect("lowering a wide sum");
        if let Expr::Ident(i) = term {
            if let Some((_, slot, _)) = sums.lets.get(&i.name) {
                let slot = *slot;
                return Ok(emit(self.builder.build_load(self.context.i64_type(), slot, "wide.term"))?.into_int_value());
            }
        }
        if let Some(flag) = sums.ops.get(&node(term)).copied() {
            let (op, l, r) = wide_op(term).expect("a wide term is an operator");
            return self.wide_term_op(op, l, r, flag, a, scope);
        }
        self.wide_operand(term, a, scope)
    }

    /// `e`, a value of `a`'s type (or a literal of it), extended to `i64`.
    fn wide_operand(&mut self, e: &Expr, a: &WideAcc<'ctx>, scope: &Scope<'ctx>) -> Result<IntValue<'ctx>, CodegenError> {
        let (v, t) = self.lower_expr(e, scope)?;
        let v = self.fit_int(v, &t, &a.ty)?;
        let bits = a.prim.machine_int().expect("a machine integer").0;
        if !v.is_int_value() || v.into_int_value().get_type().get_bit_width() != bits {
            return Err(CodegenError::Unsupported(format!(
                "a term of a checked `{}` sum lowered as a {t}",
                hale_types::ty::prim_name(a.prim)
            )));
        }
        self.resize_int(v.into_int_value(), a.prim, PrimType::Int)
    }

    /// `l op r` over `a`'s type, computed in `i64` (exact for two operands
    /// of 32 bits or fewer), whether it is outside the type or-ed into
    /// `flag`, which the loop's exit checks.
    fn wide_term_op(
        &mut self,
        op: BinOp,
        l: &Expr,
        r: &Expr,
        flag: PointerValue<'ctx>,
        a: &WideAcc<'ctx>,
        scope: &Scope<'ctx>,
    ) -> Result<IntValue<'ctx>, CodegenError> {
        let lv = self.wide_operand(l, a, scope)?;
        let rv = self.wide_operand(r, a, scope)?;
        let t = match op {
            BinOp::Add => emit(self.builder.build_int_add(lv, rv, "wide.term.add"))?,
            BinOp::Sub => emit(self.builder.build_int_sub(lv, rv, "wide.term.sub"))?,
            _ => emit(self.builder.build_int_mul(lv, rv, "wide.term.mul"))?,
        };
        let out = self.wide_out_of_range(t, a.prim)?;
        let prev = emit(self.builder.build_load(self.context.bool_type(), flag, "wide.flag.cur"))?.into_int_value();
        let next = emit(self.builder.build_or(prev, out, "wide.flag.or"))?;
        emit(self.builder.build_store(flag, next))?;
        Ok(t)
    }

    /// Whether `v`, an `i64`, is outside `p`'s range: unsigned, above its
    /// maximum read as unsigned (a negative value is huge); signed, `v -
    /// min` above `max - min`.
    fn wide_out_of_range(&mut self, v: IntValue<'ctx>, p: PrimType) -> Result<IntValue<'ctx>, CodegenError> {
        let (bits, signed) = p.machine_int().expect("a machine integer");
        let i64_t = self.context.i64_type();
        let span = i64_t.const_int((1u64 << bits) - 1, false);
        let v = if signed { emit(self.builder.build_int_add(v, i64_t.const_int(1u64 << (bits - 1), false), "wide.shift"))? } else { v };
        Ok(emit(self.builder.build_int_compare(IP::UGT, v, span, "wide.out"))?)
    }

    /// At the loop's exit (where the builder stands when the loop ended
    /// open): each operator's flag a violation, then each total held to
    /// its type's range and narrowed into its accumulator.
    pub(crate) fn wide_sums_leave(&mut self, sums: WideSums<'ctx>, end: BlockEnd) -> Result<(), CodegenError> {
        if end != BlockEnd::Open {
            return Ok(());
        }
        for (slot, what, p) in &sums.flags {
            let bad = emit(self.builder.build_load(self.context.bool_type(), *slot, "wide.flag.end"))?.into_int_value();
            self.fixed_violation_if(bad, *what, *p)?;
        }
        for a in sums.accs.values() {
            let total = emit(self.builder.build_load(self.context.i64_type(), a.wide, "wide.total"))?.into_int_value();
            let bad = self.wide_out_of_range(total, a.prim)?;
            self.fixed_violation_if(bad, Violation::Add, a.prim)?;
            let v = self.resize_int(total, PrimType::Int, a.prim)?;
            emit(self.builder.build_store(a.narrow, v))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loop_of(src: &str) -> Stmt {
        let mut p = hale_syntax::parse_source(&format!("fn f() {{ {src} }}")).expect("parses");
        hale_syntax::chains::desugar_chains(&mut p);
        let hale_syntax::ast::TopDecl::Fn(f) = &p.items[0] else { panic!("a fn") };
        find(&f.body).expect("a loop")
    }

    /// The first loop in `b`, a chain's (inside its `let`'s block) included.
    fn find(b: &Block) -> Option<Stmt> {
        b.stmts.iter().find_map(|s| match s {
            Stmt::For { .. } | Stmt::While { .. } => Some(s.clone()),
            Stmt::Let { value: Expr::Block(inner), .. } => find(inner),
            _ => None,
        })
    }

    fn accs(src: &str) -> Vec<String> {
        let s = loop_of(src);
        Shape::of(&s).map(|sh| sh.accs.keys().cloned().collect()).unwrap_or_default()
    }

    fn lets(src: &str) -> Vec<String> {
        let s = loop_of(src);
        Shape::of(&s).map(|sh| sh.lets.keys().cloned().collect()).unwrap_or_default()
    }

    #[test]
    fn a_for_sum_and_its_spellings() {
        assert_eq!(accs("let mut s = 0; for x in xs { s = s + x; }"), ["s"]);
        assert_eq!(accs("let mut s = 0; for x in xs { s = x + s; }"), ["s"]);
        assert_eq!(accs("let mut s = 0; for x in xs { s += x * 3; }"), ["s"]);
        assert_eq!(accs("let mut s = 0; for x in xs { if x > 2 { s = s + x; } }"), ["s"]);
    }

    #[test]
    fn a_chain_sum_and_its_map() {
        assert_eq!(accs("let t = xs.map(it * 3).sum(U32(0));"), ["__hale_chain_n1"]);
        assert_eq!(lets("let t = xs.map(it * 3).sum(U32(0));"), ["__hale_m1_1"]);
        assert_eq!(accs("let t = xs.filter(it > 2).sum(U32(0));"), ["__hale_chain_n1"]);
        // two maps: the first is a term of the second, not of the sum
        assert_eq!(lets("let t = xs.map(it * 3).map(it + 1).sum(U32(0));"), ["__hale_m1_2"]);
        // a mapped value the filter reads is named twice: checked
        assert!(lets("let t = xs.map(it * 3).filter(it > 2).sum(U32(0));").is_empty());
    }

    #[test]
    fn what_is_not_taken() {
        // the running sum is read
        assert!(accs("let mut s = 0; for x in xs { s = s + x; if s > 9 { break; } }").is_empty());
        // a product
        assert!(accs("let mut s = 1; for x in xs { s = s * x; }").is_empty());
        // a call into the program, a print, a return, a nested loop
        assert!(accs("let mut s = 0; for x in xs { s = s + g(x); }").is_empty());
        assert!(accs("let mut s = 0; for x in xs { s = s + x; println(x); }").is_empty());
        assert!(accs("let mut s = 0; for x in xs { if x > 9 { return; } s = s + x; }").is_empty());
        assert!(accs("let mut s = 0; for x in xs { for y in xs { s = s + y; } }").is_empty());
        // a range has no capacity
        assert!(accs("let mut s = 0; for i in 0..9 { s = s + 1; }").is_empty());
        // a hand-written while is not a chain's loop
        assert!(accs("let mut s = 0; let mut i = 0; while i < 9 { s = s + 1; i = i + 1; }").is_empty());
        // the term names the accumulator
        assert!(accs("let mut s = 0; for x in xs { s = s + s; }").is_empty());
        // `or raise` leaves the frame
        assert!(accs("let mut s = 0; for x in xs { s = s + (U32(x) or raise); }").is_empty());
    }
}

//! Appending assignments (GH #1947): `s = s + a + b`, `s += a`, and the
//! f-string spelling `s = f"{s}…"`.
//!
//! Each `+` used to build a fresh String in the frame's arena, and the
//! arena keeps everything until the frame ends, so a loop that grows a
//! String by appending held every prefix it ever built: a 350 KB result
//! peaked at 4.5 GB. Lowering now treats an appending assignment as an
//! append into a buffer the place owns, which grows geometrically:
//!
//! - a local declared `let mut` whose block holds an appending
//!   assignment to it keeps a growable buffer beside its slot. The
//!   buffer is the local's own until a read of the local may keep the
//!   value (anything but an operand of a String `+` chain, the argument
//!   of `len`, an operand of a comparison, or the head of an appending
//!   assignment): from that read on, the next append copies into a fresh
//!   buffer, so a value read before an append never sees it;
//! - a String field of `self` appends into its field block while the
//!   block's size class has room, and otherwise moves to a block of a
//!   class half again as large.
//!
//! This module is the one statement of which assignments are appends and
//! which loops still copy on every append, read by lowering
//! (`hale_codegen`) and by the allocation summary, whose
//! [`QuadraticAppend`] rows the hot-path lint reports.

use hale_syntax::ast::*;
use hale_syntax::Span;

use crate::expr_walk::{visit_expr, visit_stmt, Visited};

/// The operands an appending assignment appends to its place, in order,
/// or `None` when the assignment is not one.
///
/// `target += e` appends `e`. `target = target + e1 + … + en`, a chain of
/// `+` whose leftmost operand reads the place the assignment writes,
/// appends `e1 … en`; `target = f"{target}…"` is the same chain (the
/// f-string's leading `""` and its `to_string` of the place are the
/// place itself, a String). The place is a name followed by field
/// accesses (`out`, `self.text`, `rec.body`); an indexed place is never
/// an append. Whether the place is a String is the caller's to know.
pub fn appended_parts<'a>(target: &LValue, op: &AssignOp, value: &'a Expr) -> Option<Vec<&'a Expr>> {
    if target.tail.iter().any(|s| matches!(s, LValueSeg::Index(_))) {
        return None;
    }
    match op {
        AssignOp::PlusEq => Some(vec![value]),
        AssignOp::Eq => {
            let mut leaves = Vec::new();
            chain_leaves(value, &mut leaves);
            if leaves.len() < 2 {
                return None;
            }
            if denotes(leaves[0], target) {
                return Some(leaves[1..].to_vec());
            }
            // `f"{s},{i}"` is `"" + to_string(s) + "," + to_string(i)`.
            if leaves.len() >= 3 && is_empty_literal(leaves[0]) && is_to_string_of(leaves[1], target) {
                return Some(leaves[2..].to_vec());
            }
            None
        }
        _ => None,
    }
}

/// The leaves of the left spine of a `+` chain, in evaluation order:
/// `((a + b) + c)` is `[a, b, c]`; a parenthesized right operand
/// (`a + (b + c)`) is one leaf.
fn chain_leaves<'a>(e: &'a Expr, out: &mut Vec<&'a Expr>) {
    match e {
        Expr::Binary { op: BinOp::Add, left, right, .. } => {
            chain_leaves(left, out);
            out.push(right);
        }
        other => out.push(other),
    }
}

fn is_empty_literal(e: &Expr) -> bool {
    matches!(e, Expr::Literal(Literal::String(s), _) if s.is_empty())
}

fn is_to_string_of(e: &Expr, target: &LValue) -> bool {
    match e {
        Expr::Call { callee, args, .. } => {
            matches!(callee.as_ref(), Expr::Ident(i) if i.name == "to_string")
                && args.len() == 1
                && denotes(&args[0], target)
        }
        _ => false,
    }
}

/// Does `e` read the place `target` writes: its name, then the same
/// fields?
pub fn denotes(e: &Expr, target: &LValue) -> bool {
    let mut fields: Vec<&str> = Vec::new();
    let mut cur = e;
    while let Expr::Field { receiver, name, .. } = cur {
        fields.push(name.name.as_str());
        cur = receiver;
    }
    fields.reverse();
    let head = match cur {
        Expr::Ident(i) => target.head.name != "self" && i.name == target.head.name,
        Expr::KwSelf(_) => target.head.name == "self",
        _ => false,
    };
    head && fields.len() == target.tail.len()
        && target.tail.iter().zip(&fields).all(|(s, f)| matches!(s, LValueSeg::Field(n) if n.name == *f))
}

/// The place an appending assignment writes, as lowering treats it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppendPlace<'a> {
    /// A bare local: `out = out + …`.
    Local(&'a str),
    /// A field of `self`: `self.text = self.text + …`.
    SelfField(&'a str),
    /// Anything else (`rec.body`, `self.page.body`, `h.text` through a
    /// handle): the append builds a fresh String, as `+` does.
    Other,
}

/// Which kind of place `target` is.
pub fn append_place(target: &LValue) -> AppendPlace<'_> {
    match (target.head.name.as_str(), target.tail.as_slice()) {
        ("self", [LValueSeg::Field(f)]) => AppendPlace::SelfField(f.name.as_str()),
        ("self", _) => AppendPlace::Other,
        (name, []) => AppendPlace::Local(name),
        _ => AppendPlace::Other,
    }
}

/// Does any statement in `stmts` (or the `tail`) append to the bare local
/// `name`? Lowering asks it of the statements after a `let mut name`,
/// which are the binding's whole scope, to decide whether the local
/// keeps a growable buffer. A later binding of the same name is not told
/// apart: the answer is then yes for both, which costs the outer one an
/// unused buffer and nothing else.
pub fn appends_to_local(stmts: &[Stmt], tail: Option<&Expr>, name: &str) -> bool {
    let mut found = false;
    let mut f = |v: Visited<'_>| {
        if let Visited::Stmt(Stmt::Assign { target, op, value, .. }) = v {
            if append_place(target) == AppendPlace::Local(name) && appended_parts(target, op, value).is_some() {
                found = true;
            }
        }
    };
    for s in stmts {
        visit_stmt(s, &mut f);
    }
    if let Some(t) = tail {
        visit_expr(t, &mut f);
    }
    found
}

/// An appending assignment in a loop that copies the whole String on
/// every iteration: the hot-path lint's finding (spec/verification.md §
/// "Hot-path allocation lint").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuadraticAppend {
    /// The assignment statement.
    pub span: Span,
    /// The place it writes, as written (`out`, `rec.body`).
    pub place: String,
    pub why: QuadraticWhy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuadraticWhy {
    /// The loop also reads the local in a way that may keep its value
    /// (the read at the span: passed to a call, bound, stored,
    /// returned), so the local's buffer is shared again on every
    /// iteration and each append copies it.
    SharedInLoop { read: Span },
    /// The place is not a local or a field of `self`: every append
    /// builds a fresh String the size of the whole.
    Place,
}

/// Every appending assignment of a String in a loop of `body` that copies
/// the String it extends on each iteration ([`QuadraticWhy`]).
///
/// A place is known to hold a String when an appended operand is a
/// string literal (the chain's value is then a String, and so is the
/// place it is assigned to), or, for a local, when its `let` is
/// annotated `String` or initialized with a string literal. Anything it
/// cannot prove it leaves alone. An append to a local declared inside
/// the loop starts afresh each iteration and is not reported.
pub fn quadratic_appends(body: &Block) -> Vec<QuadraticAppend> {
    let mut w = Quad { scopes: vec![Vec::new()], loops: Vec::new(), out: Vec::new() };
    w.block(body);
    w.out
}

/// A local binding in scope: its name, the number of loops enclosing its
/// declaration, and whether it is known to be a String.
struct Binding {
    name: String,
    loop_depth: usize,
    string: bool,
}

struct Quad<'a> {
    scopes: Vec<Vec<Binding>>,
    /// The bodies of the loops enclosing the walk, outermost first, each
    /// with its condition (a `while`'s; a `for`'s iterable is read once).
    loops: Vec<(&'a Block, Option<&'a Expr>)>,
    out: Vec<QuadraticAppend>,
}

impl<'a> Quad<'a> {
    fn lookup(&self, name: &str) -> Option<&Binding> {
        self.scopes.iter().rev().flat_map(|s| s.iter().rev()).find(|b| b.name == name)
    }

    fn block(&mut self, b: &'a Block) {
        self.scopes.push(Vec::new());
        for s in &b.stmts {
            self.stmt(s);
        }
        if let Some(t) = &b.tail {
            self.expr(t);
        }
        self.scopes.pop();
    }

    fn declare(&mut self, name: &str, string: bool) {
        let depth = self.loops.len();
        if let Some(s) = self.scopes.last_mut() {
            s.push(Binding { name: name.to_string(), loop_depth: depth, string });
        }
    }

    fn stmt(&mut self, s: &'a Stmt) {
        match s {
            Stmt::Let { name, ty, value, .. } => {
                self.expr(value);
                let string = matches!(ty, Some(TypeExpr::Primitive(PrimType::String, _)))
                    || matches!(value, Expr::Literal(Literal::String(_), _));
                self.declare(&name.name, string);
            }
            Stmt::LetTuple { names, value, .. } => {
                self.expr(value);
                for n in names {
                    self.declare(&n.name, false);
                }
            }
            Stmt::Assign { target, op, value, span, .. } => {
                if let Some(parts) = appended_parts(target, op, value) {
                    self.judge(target, &parts, *span);
                }
                self.expr(value);
            }
            Stmt::While { cond, body, .. } => {
                self.expr(cond);
                self.loops.push((body, Some(cond)));
                self.block(body);
                self.loops.pop();
            }
            Stmt::For { name, iter, body, .. } => {
                self.expr(iter);
                self.loops.push((body, None));
                self.scopes.push(Vec::new());
                self.declare(&name.name, false);
                self.block(body);
                self.scopes.pop();
                self.loops.pop();
            }
            other => {
                // Every other statement: its nested blocks are walked for
                // their own statements; its expressions for blocks.
                let mut nested: Vec<&'a Block> = Vec::new();
                let mut exprs: Vec<&'a Expr> = Vec::new();
                match other {
                    Stmt::If(i) => if_parts(i, &mut exprs, &mut nested),
                    Stmt::Match(m) => match_parts(m, &mut exprs, &mut nested),
                    Stmt::Block(b) => nested.push(b),
                    Stmt::ShmWrite { body, .. } => nested.push(body),
                    Stmt::Return(Some(e), _) | Stmt::Fail { value: e, .. } | Stmt::Expr(e) => exprs.push(e),
                    _ => {}
                }
                for e in exprs {
                    self.expr(e);
                }
                for b in nested {
                    self.block(b);
                }
            }
        }
    }

    /// Walk an expression for the blocks it holds (an `if` or `match`
    /// value, a block value), whose statements may append.
    fn expr(&mut self, e: &'a Expr) {
        let mut nested: Vec<&'a Block> = Vec::new();
        let mut exprs: Vec<&'a Expr> = Vec::new();
        match e {
            Expr::Block(b) => nested.push(b),
            Expr::If(i) => if_parts(i, &mut exprs, &mut nested),
            Expr::Match(m) => match_parts(m, &mut exprs, &mut nested),
            Expr::Binary { left, right, .. } => {
                exprs.push(left);
                exprs.push(right);
            }
            Expr::Unary { operand, .. } => exprs.push(operand),
            Expr::Call { callee, args, .. } => {
                exprs.push(callee);
                exprs.extend(args.iter());
            }
            Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. } => exprs.push(receiver),
            Expr::Index { receiver, index, .. } => {
                exprs.push(receiver);
                exprs.push(index);
            }
            Expr::Tuple(xs, _) | Expr::Array(xs, _) => exprs.extend(xs.iter()),
            Expr::Struct { inits, .. } => exprs.extend(inits.iter().map(|i| &i.value)),
            Expr::Or { inner, disposition, .. } => {
                exprs.push(inner);
                if let OrDisposition::Substitute(x) | OrDisposition::Fail(x, _) = disposition {
                    exprs.push(x);
                }
            }
            _ => {}
        }
        for x in exprs {
            self.expr(x);
        }
        for b in nested {
            self.block(b);
        }
    }

    fn judge(&mut self, target: &LValue, parts: &[&Expr], span: Span) {
        if self.loops.is_empty() {
            return;
        }
        let literal_part = parts.iter().any(|p| chain_has_literal(p));
        match append_place(target) {
            AppendPlace::Local(name) => {
                let Some(b) = self.lookup(name) else { return };
                if !(b.string || literal_part) || b.loop_depth >= self.loops.len() {
                    return;
                }
                // The outermost loop the binding is declared outside of
                // holds every iteration that appends to the same buffer.
                let (body, cond) = self.loops[b.loop_depth];
                let read = cond.and_then(|c| sharing_read_in_expr(c, name)).or_else(|| sharing_read_in_block(body, name));
                if let Some(read) = read {
                    self.out.push(QuadraticAppend {
                        span,
                        place: name.to_string(),
                        why: QuadraticWhy::SharedInLoop { read },
                    });
                }
            }
            AppendPlace::SelfField(_) => {}
            AppendPlace::Other => {
                if literal_part {
                    self.out.push(QuadraticAppend { span, place: place_text(target), why: QuadraticWhy::Place });
                }
            }
        }
    }
}

fn if_parts<'a>(i: &'a IfStmt, exprs: &mut Vec<&'a Expr>, nested: &mut Vec<&'a Block>) {
    exprs.push(&i.cond);
    nested.push(&i.then_block);
    match i.else_block.as_deref() {
        Some(ElseBranch::Else(b)) => nested.push(b),
        Some(ElseBranch::ElseIf(n)) => if_parts(n, exprs, nested),
        None => {}
    }
}

fn match_parts<'a>(m: &'a MatchStmt, exprs: &mut Vec<&'a Expr>, nested: &mut Vec<&'a Block>) {
    exprs.push(&m.scrutinee);
    for a in &m.arms {
        if let Some(g) = &a.guard {
            exprs.push(g);
        }
        match &a.body {
            MatchArmBody::Expr(e) => exprs.push(e),
            MatchArmBody::Block(b) => nested.push(b),
        }
    }
}

/// Is a string literal among the leaves of `e`'s `+` chain?
fn chain_has_literal(e: &Expr) -> bool {
    let mut leaves = Vec::new();
    chain_leaves(e, &mut leaves);
    leaves.len() > 1 && leaves.iter().any(|l| matches!(l, Expr::Literal(Literal::String(_), _)))
        || matches!(e, Expr::Literal(Literal::String(_), _))
}

fn place_text(target: &LValue) -> String {
    let mut s = target.head.name.clone();
    for seg in &target.tail {
        match seg {
            LValueSeg::Field(f) => {
                s.push('.');
                s.push_str(&f.name);
            }
            LValueSeg::Index(_) => s.push_str("[…]"),
        }
    }
    s
}

/// Does a call of `callee` keep none of its arguments? Then an argument
/// that names a local is a read that keeps nothing, and lowering hands
/// the local's value over without giving up its append buffer.
///
/// - `len` and the printers, by names no fn can take: `len` measures,
///   a printer writes the bytes out and keeps nothing.
/// - the built-in call forms `contains` and `starts_with`, the same
///   native readers as their `std::str` rows.
/// - a `std::` function whose surface row is public, lowered natively
///   (`Lower::Intrinsic`: no Hale body of its own, so nothing it does
///   is a store the stdlib's source makes), classified pure or
///   allocating only (`EffectSet::PURE` / `ALLOC`: it touches no
///   locus, wire or process state), not fallible (an error payload
///   could carry an argument), and returning a value that cannot be
///   one of its arguments: an `Int`, `Float`, `Bool`, `Decimal`, 256-bit
///   integer, `Duration`, `Time` or nothing. `std::str::contains`,
///   `index_of`, `starts_with`, `ends_with`, `byte_at`, `can_parse_int`
///   are such calls; `substring` (it returns a `String`) and every
///   program fn are not.
pub fn call_keeps_no_argument(callee: &Expr) -> bool {
    use crate::stdlib_surface::{row, EffectSet, Lower, SigTy};
    match callee {
        Expr::Ident(i) => {
            matches!(i.name.as_str(), "len" | "println" | "print" | "eprintln" | "eprint" | "contains" | "starts_with")
        }
        Expr::Path(q) => {
            let segs: Vec<&str> = q.segments.iter().map(|s| s.name.as_str()).collect();
            let Some(f) = row(&segs) else { return false };
            let Some(sig) = f.sig.as_ref() else { return false };
            f.is_public()
                && matches!(f.lower, Lower::Intrinsic(_))
                && !f.effects.is_unclassified()
                && EffectSet::ALLOC.contains(f.effects)
                && sig.fallible.is_none()
                && matches!(
                    sig.ret,
                    SigTy::Int
                        | SigTy::Uint
                        | SigTy::Float
                        | SigTy::Bool
                        | SigTy::Decimal
                        | SigTy::U256
                        | SigTy::I256
                        | SigTy::Duration
                        | SigTy::Time
                        | SigTy::Unit
                )
        }
        _ => false,
    }
}

/// The reads of a local that never keep its value, as lowering lowers
/// them without giving up the local's buffer: an operand of a `+` chain
/// (the chain copies it), an operand of a comparison, an argument of a
/// call that keeps none ([`call_keeps_no_argument`]), the String a range
/// slices (`s[lo..hi]` copies), and the place an appending assignment
/// reads.
///
/// Calls `on_read` with every other read of `name` in the statements.
fn sharing_reads(stmts: &[&Stmt], exprs: &[&Expr], name: &str, on_read: &mut dyn FnMut(Span)) {
    // Ident nodes read without sharing, by address.
    let mut consumed: Vec<*const Expr> = Vec::new();
    let mut reads: Vec<(*const Expr, Span)> = Vec::new();
    let mut f = |v: Visited<'_>| match v {
        Visited::Expr(e) => match e {
            Expr::Ident(i) if i.name == name => reads.push((e as *const Expr, i.span)),
            Expr::Binary { op, left, right, .. } => {
                if matches!(
                    op,
                    BinOp::Add | BinOp::Eq | BinOp::NotEq | BinOp::Lt | BinOp::Gt | BinOp::LtEq | BinOp::GtEq
                ) {
                    consumed.push(left.as_ref() as *const Expr);
                    consumed.push(right.as_ref() as *const Expr);
                }
            }
            Expr::Call { callee, args, .. } => {
                if call_keeps_no_argument(callee) {
                    consumed.extend(args.iter().map(|a| a as *const Expr));
                }
            }
            // `s[lo..hi]` copies the bytes it names into a fresh String.
            Expr::Index { receiver, index, .. } if matches!(index.as_ref(), Expr::Range { .. }) => {
                consumed.push(receiver.as_ref() as *const Expr);
            }
            _ => {}
        },
        Visited::Stmt(Stmt::Assign { target, op, value, .. }) => {
            if append_place(target) == AppendPlace::Local(name) && appended_parts(target, op, value).is_some() {
                let mut leaves = Vec::new();
                chain_leaves(value, &mut leaves);
                if let Some(head) = leaves.first() {
                    consumed.push(*head as *const Expr);
                }
                // the f-string spelling's `to_string(name)`
                if let Some(Expr::Call { args, .. }) = leaves.get(1) {
                    if let Some(a) = args.first() {
                        consumed.push(a as *const Expr);
                    }
                }
            }
        }
        _ => {}
    };
    for s in stmts {
        visit_stmt(s, &mut f);
    }
    for e in exprs {
        visit_expr(e, &mut f);
    }
    for (p, span) in reads {
        if !consumed.contains(&p) {
            on_read(span);
        }
    }
}

/// The first read of `name` in the loop body `b` that gives the local's
/// buffer up while a later iteration still appends to that buffer.
///
/// A read is excused, as the runtime makes it cheap, when what follows
/// it in its own block (before any append to the local) starts the
/// local afresh — a plain assignment, `tok = ""` after handing `tok` on
/// — or leaves the frame (`return`, `fail`): the next append then copies
/// the fresh value, or never comes. A read in a `return` or `fail`, and
/// one in a plain assignment to the local itself (`s = f(s)`), is
/// excused the same way.
fn sharing_read_in_block(b: &Block, name: &str) -> Option<Span> {
    unexcused_in_block(b, name).into_iter().next()
}

fn flat_reads(stmts: &[&Stmt], exprs: &[&Expr], name: &str) -> Vec<Span> {
    let mut out = Vec::new();
    sharing_reads(stmts, exprs, name, &mut |s| out.push(s));
    out
}

fn unexcused_in_block(b: &Block, name: &str) -> Vec<Span> {
    let mut out = Vec::new();
    for (i, st) in b.stmts.iter().enumerate() {
        let reads = unexcused_in_stmt(st, name);
        if reads.is_empty() || starts_afresh(&b.stmts[i + 1..], name) {
            continue;
        }
        out.extend(reads);
    }
    if let Some(t) = &b.tail {
        out.extend(flat_reads(&[], &[t], name));
    }
    out
}

fn unexcused_in_stmt(st: &Stmt, name: &str) -> Vec<Span> {
    match st {
        Stmt::Return(..) | Stmt::Fail { .. } => Vec::new(),
        // `s = f(s)`: the statement that reads the local starts it afresh
        Stmt::Assign { target, op, value, .. }
            if append_place(target) == AppendPlace::Local(name) && appended_parts(target, op, value).is_none() =>
        {
            Vec::new()
        }
        Stmt::If(i) => unexcused_in_if(i, name),
        Stmt::While { cond, body, .. } => {
            let mut v = flat_reads(&[], &[cond], name);
            v.extend(unexcused_in_block(body, name));
            v
        }
        Stmt::For { iter, body, .. } => {
            let mut v = flat_reads(&[], &[iter], name);
            v.extend(unexcused_in_block(body, name));
            v
        }
        Stmt::Block(b) => unexcused_in_block(b, name),
        Stmt::Match(m) => {
            let mut v = flat_reads(&[], &[&m.scrutinee], name);
            for a in &m.arms {
                if let Some(g) = &a.guard {
                    v.extend(flat_reads(&[], &[g], name));
                }
                match &a.body {
                    MatchArmBody::Expr(e) => v.extend(flat_reads(&[], &[e], name)),
                    MatchArmBody::Block(b) => v.extend(unexcused_in_block(b, name)),
                }
            }
            v
        }
        other => flat_reads(&[other], &[], name),
    }
}

fn unexcused_in_if(i: &IfStmt, name: &str) -> Vec<Span> {
    let mut v = flat_reads(&[], &[&i.cond], name);
    v.extend(unexcused_in_block(&i.then_block, name));
    match i.else_block.as_deref() {
        Some(ElseBranch::Else(b)) => v.extend(unexcused_in_block(b, name)),
        Some(ElseBranch::ElseIf(n)) => v.extend(unexcused_in_if(n, name)),
        None => {}
    }
    v
}

/// Do the statements after a read start the local afresh (a plain
/// assignment to it) or leave the frame (`return`, `fail`) before any of
/// them appends to it?
fn starts_afresh(rest: &[Stmt], name: &str) -> bool {
    for st in rest {
        match st {
            Stmt::Return(..) | Stmt::Fail { .. } => return true,
            Stmt::Assign { target, op, value, .. }
                if append_place(target) == AppendPlace::Local(name) && appended_parts(target, op, value).is_none() =>
            {
                return true;
            }
            _ => {}
        }
        if appends_to_local(std::slice::from_ref(st), None, name) {
            return false;
        }
    }
    false
}

fn sharing_read_in_expr(e: &Expr, name: &str) -> Option<Span> {
    let mut first = None;
    sharing_reads(&[], &[e], name, &mut |s| {
        first.get_or_insert(s);
    });
    first
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body_of(src: &str) -> Block {
        let prog = hale_syntax::parse_source(src).expect("parses");
        for item in &prog.items {
            if let TopDecl::Fn(f) = item {
                if f.name.name == "f" {
                    return f.body.clone();
                }
            }
        }
        panic!("no fn f");
    }

    fn found(src: &str) -> Vec<QuadraticAppend> {
        quadratic_appends(&body_of(src))
    }

    #[test]
    fn an_append_whose_buffer_stays_the_locals_is_not_reported() {
        let src = "fn f(n: Int) -> Int {\n let mut out = \"\";\n let mut i = 0;\n while i < n {\n out = out + \"row \" + to_string(i) + \"\\n\";\n i = i + 1;\n }\n return len(out);\n}\n";
        assert!(found(src).is_empty(), "{:?}", found(src));
    }

    #[test]
    fn a_read_that_keeps_the_value_in_the_loop_is_reported() {
        let src = "fn g(s: String) -> Int { return len(s); }\nfn f(n: Int) -> Int {\n let mut out = \"\";\n let mut i = 0;\n while i < n {\n out += \"row\";\n let k = g(out);\n i = i + k;\n }\n return len(out);\n}\n";
        let got = found(src);
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(got[0].place, "out");
        assert!(matches!(got[0].why, QuadraticWhy::SharedInLoop { .. }));
    }

    #[test]
    fn a_std_reader_that_keeps_nothing_is_not_a_sharing_read() {
        let src = "fn f(n: Int) -> Int {\n let mut seen = \"\\n\";\n let mut i = 0;\n while i < n {\n let k = to_string(i % 7);\n if !std::str::contains(seen, \"\\n\" + k + \"\\n\") && std::str::index_of(seen, k) < 0 { seen = seen + k + \"\\n\"; }\n println(seen);\n let head = seen[0..1];\n i = i + len(head);\n }\n return len(seen);\n}\n";
        assert!(found(src).is_empty(), "{:?}", found(src));
    }

    #[test]
    fn a_std_fn_returning_a_string_is_a_sharing_read() {
        let src = "fn f(n: Int) -> Int {\n let mut out = \"\";\n let mut i = 0;\n while i < n {\n out += \"row \";\n let t = std::str::trim(out);\n i = i + len(t);\n }\n return len(out);\n}\n";
        assert_eq!(found(src).len(), 1, "{:?}", found(src));
    }

    #[test]
    fn a_read_followed_by_a_reset_or_an_exit_is_not_reported() {
        // a token handed on and started afresh; a `return` of the value;
        // `s = f(s)` replacing it
        let src = "fn g(s: String) -> Int { return len(s); }\nfn f(text: String) -> Int {\n let mut tok = \"\";\n let mut out = \"\";\n let mut k = 0;\n let mut i = 0;\n while i < len(text) {\n let ch = text[i..(i + 1)];\n if ch == \" \" { k = k + g(tok); tok = \"\"; } else { tok = tok + ch; }\n if k > 100 { return g(out); }\n out = out + ch;\n out = std::str::replace(out, \"aa\", \"a\");\n i = i + 1;\n }\n return k;\n}\n";
        assert!(found(src).is_empty(), "{:?}", found(src));
    }

    #[test]
    fn the_calls_that_keep_nothing() {
        let callee = |src: &str| {
            let b = body_of(&format!("fn f() {{ let s = \"x\"; {src}; }}"));
            match &b.stmts[1] {
                Stmt::Expr(Expr::Call { callee, .. }) => callee_keeps(callee),
                other => panic!("{other:?}"),
            }
        };
        fn callee_keeps(c: &Expr) -> bool {
            call_keeps_no_argument(c)
        }
        assert!(callee("std::str::contains(s, \"a\")"));
        assert!(callee("std::str::index_of(s, \"a\")"));
        assert!(callee("std::str::ends_with(s, \"a\")"));
        assert!(callee("len(s)"));
        assert!(callee("println(s)"));
        assert!(!callee("std::str::trim(s)"), "returns a String");
        assert!(!callee("std::str::substring(s, 0, 1)"), "returns a String");
        assert!(!callee("g(s)"), "a program fn");
    }

    #[test]
    fn a_local_declared_in_the_loop_is_not_reported() {
        let src = "fn g(s: String) -> Int { return len(s); }\nfn f(n: Int) -> Int {\n let mut i = 0;\n while i < n {\n let mut out = \"\";\n out += \"row\";\n i = i + g(out);\n }\n return i;\n}\n";
        assert!(found(src).is_empty());
    }

    #[test]
    fn an_append_to_a_record_field_in_a_loop_is_reported() {
        let src = "type R { body: String; }\nfn f(n: Int) -> Int {\n let mut r = R { body: \"\" };\n let mut i = 0;\n while i < n {\n r.body = r.body + \"row\";\n i = i + 1;\n }\n return len(r.body);\n}\n";
        let got = found(src);
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(got[0].place, "r.body");
        assert_eq!(got[0].why, QuadraticWhy::Place);
    }

    #[test]
    fn the_spellings_of_an_append() {
        let t = LValue { head: Ident::new("s", Span::new(0, 0)), tail: vec![], span: Span::new(0, 0) };
        let parse_expr = |src: &str| {
            let b = body_of(&format!("fn f() {{ let mut s = \"\"; let i = 1; s = {src}; }}"));
            match &b.stmts[2] {
                Stmt::Assign { value, .. } => value.clone(),
                other => panic!("{other:?}"),
            }
        };
        let e = parse_expr("s + \"a\" + to_string(i)");
        assert_eq!(appended_parts(&t, &AssignOp::Eq, &e).map(|p| p.len()), Some(2));
        let e = parse_expr("f\"{s},{i}\"");
        assert_eq!(appended_parts(&t, &AssignOp::Eq, &e).map(|p| p.len()), Some(2));
        let e = parse_expr("\"a\" + s");
        assert_eq!(appended_parts(&t, &AssignOp::Eq, &e), None);
        let e = parse_expr("s + (\"a\" + s)");
        assert_eq!(appended_parts(&t, &AssignOp::Eq, &e).map(|p| p.len()), Some(1));
    }
}

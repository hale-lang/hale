//! GH #2105: which whole stores into a local's `bounded` field may free
//! the elements they replace.
//!
//! `q.tags = r.tags` copies each element of `r.tags`, nested blocks and
//! all (GH #2099), and nothing freed the elements it replaced, so the
//! store repeated in a loop grew the frame's arena by a copy each time. A
//! locus-held field retires what it replaced, since every store into it
//! copies; a local's field cannot in general, because its elements may
//! share blocks with other bindings (a pushed record's String is the
//! block of the binding it came from).
//!
//! Lowering therefore makes the copy of such a store in an arena of the
//! field's own, made fresh at each store that retires: the one block set
//! that store frees is the previous copy, which only the field's earlier
//! stores ever allocated in. A block that reached the field any other way
//! (a push, a literal, a call's fresh result) lies in some other arena and
//! is never freed by it, however many names share it.
//!
//! What remains is a name that reached into a copy after it was made: a
//! binding read from the field (`let t = at(q.tags, 0) or …`, `for t in
//! q.tags`, `let w = q`), a place a value read from it was stored into
//! (`other.name = t.name`, `push(list, t)`), the local itself when such a
//! value lands in another of its fields, or any binding a callee handed
//! such a value may have written it into. This module finds them, and a
//! store retires only when none is in scope where the store runs:
//!
//! - **The field.** A store targets `q.f₁…fₙ` (fields only) where `q` is a
//!   `let` of this body; the field is that path. An expression *reaches*
//!   it when it names `q` whole, a prefix of the path, or a place under it.
//!   `q.id` does not reach `q.tags`.
//! - **Flows.** A `let`, a tuple `let`, a `for` binder and a match binder
//!   take what their value reaches; an assignment gives it to its target's
//!   root; a bounded writer (`push`, `set`, …) to its list's root; any
//!   other call, a method included, to the root of every argument and of
//!   its receiver (the callee may write one argument into another), and
//!   its result reaches what its arguments do. A binding given a value
//!   that reaches the field, or a binding already found, is found too
//!   (to a fixpoint). What only reads and answers a scalar (`len`,
//!   `count`, `println`, a comparison) carries nothing; `to_string` is not
//!   one, as a `String`'s is the `String` itself. A `return`, a `fail` and
//!   a send copy.
//! - **The local.** If a value reaching the field (or a found binding) is
//!   stored into `q` anywhere but under the field, or `q` is handed whole
//!   to a call, `q` itself may hold a block of a copy: no store into that
//!   field retires.
//! - **Scope.** A store retires unless a found binding's lexical scope
//!   holds it. A binding declared later in a loop's body than the store is
//!   out of scope there: each iteration's ended before the next store.
//! - **The store.** It is a statement (not inside an expression, where an
//!   operand evaluated before it may still hold the old elements) inside a
//!   loop of the body (a store that runs once per call has no earlier copy
//!   of its own to free, and its arena would cost a creation per call),
//!   and its value reaches neither the field nor a found binding (the old
//!   copy is freed before the new one is made).
//!
//! The answer is a set of assignment ids. Lowering applies it only where
//! the place is a `bounded` field of a record local (no locus holds it, no
//! member parameter names it) and the value is copied, not fresh.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use hale_syntax::ast::*;

/// The ids of the whole stores in `body` (whose parameters are `params`)
/// that may free the elements they replace ([`self`]).
/// `user_fn` says whether a bare name is a fn of the program, which a
/// builtin's name (`count`, `at`, `push`) may also be.
pub fn retiring_stores(params: &[Param], body: &Block, user_fn: &dyn Fn(&str) -> bool) -> HashSet<u32> {
    let mut w = Walk { user_fn: Some(user_fn), ..Walk::default() };
    w.scopes.push(BTreeMap::new());
    for p in params {
        w.bind(&p.name.name, body.span.start.0, body.span.end.0, false);
    }
    w.block(body, false);
    w.decide()
}

/// A binding of the body (or a parameter).
struct Binding {
    /// Its lexical scope, by offsets: where a store it may reach into
    /// cannot free what it holds.
    scope: (u32, u32),
    /// A `let` (or tuple `let`) of the body: a local whose field a store
    /// may retire.
    local: bool,
}

/// A place a value may carry blocks of: a binding and the field path
/// below it as far as the place spells fields.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Src {
    binding: usize,
    path: Vec<String>,
}

/// Where a flow lands: a binding, under a field path of it.
#[derive(Clone, Debug)]
struct Target {
    binding: usize,
    path: Vec<String>,
    /// Handed whole to a callee as an argument: the callee may write
    /// anywhere under it.
    whole_arg: bool,
}

/// One flow: what `sources` reach may land in each of `targets`.
struct Flow {
    sources: Vec<Src>,
    targets: Vec<Target>,
}

/// A candidate store: `q.path = value` at statement level.
struct Store {
    id: u32,
    at: u32,
    binding: usize,
    path: Vec<String>,
    value: Vec<Src>,
}

#[derive(Default)]
struct Walk<'a> {
    user_fn: Option<&'a dyn Fn(&str) -> bool>,
    bindings: Vec<Binding>,
    scopes: Vec<BTreeMap<String, usize>>,
    flows: Vec<Flow>,
    stores: Vec<Store>,
    /// How many loops of the body enclose the walk's position.
    loops: u32,
}

/// Does `a` lie on the path to `b` or under it: one is a prefix of the
/// other.
fn overlaps(a: &[String], b: &[String]) -> bool {
    let n = a.len().min(b.len());
    a[..n] == b[..n]
}

/// The builtins that only read their arguments and return a scalar or
/// nothing (a count, a flag, a number), so their result holds no block of
/// any argument. Nothing that returns text is here: `to_string` of a
/// `String` is that `String` itself (GH #2107 review), and `min`/`max`
/// may answer an argument; they and every other call carry what their
/// arguments reach.
fn reads_only(name: &str) -> bool {
    matches!(
        name,
        "len" | "count" | "Int" | "Float" | "println" | "print" | "eprintln" | "eprint" | "contains" | "starts_with"
            | "ends_with" | "abs" | "is_full" | "is_empty" | "capacity"
    )
}

/// The bounded readers: their result is (a part of) their list's element.
fn reads_element(name: &str) -> bool {
    matches!(name, "at" | "get" | "first" | "last")
}

/// The bounded writers: their first argument is the list written.
fn writes_list(name: &str) -> bool {
    matches!(name, "push" | "set" | "insert" | "clear" | "truncate" | "pop" | "remove" | "swap")
}

impl Walk<'_> {
    fn bind(&mut self, name: &str, from: u32, to: u32, local: bool) -> usize {
        let b = self.bindings.len();
        self.bindings.push(Binding { scope: (from, to), local });
        self.scopes.last_mut().expect("a scope").insert(name.to_string(), b);
        b
    }

    fn resolve(&self, name: &str) -> Option<usize> {
        self.scopes.iter().rev().find_map(|s| s.get(name).copied())
    }

    /// The binding and field path a place expression spells (`q`,
    /// `q.a.b`, `q.a[i].b` as `q.a`), if it is rooted at a binding.
    fn place(&self, e: &Expr) -> Option<Src> {
        match e {
            Expr::Ident(i) => self.resolve(&i.name).map(|binding| Src { binding, path: Vec::new() }),
            Expr::Field { receiver, name, .. } | Expr::Path2 { receiver, name, .. } => {
                let mut s = self.place(receiver)?;
                if !matches!(receiver.as_ref(), Expr::Index { .. }) {
                    s.path.push(name.name.clone());
                }
                Some(s)
            }
            Expr::Index { receiver, .. } => self.place(receiver),
            _ => None,
        }
    }

    fn block(&mut self, b: &Block, in_expr: bool) -> Vec<Src> {
        self.scopes.push(BTreeMap::new());
        for s in &b.stmts {
            self.stmt(s, b.span.end.0, in_expr);
        }
        let out = match &b.tail {
            Some(t) => self.expr(t),
            None => Vec::new(),
        };
        self.scopes.pop();
        out
    }

    fn stmt(&mut self, s: &Stmt, block_end: u32, in_expr: bool) {
        match s {
            Stmt::Let { name, value, span, .. } => {
                let sources = self.expr(value);
                let b = self.bind(&name.name, span.end.0, block_end, true);
                self.flows.push(Flow { sources, targets: vec![Target { binding: b, path: Vec::new(), whole_arg: false }] });
            }
            Stmt::LetTuple { names, value, span, .. } => {
                let sources = self.expr(value);
                let targets = names
                    .iter()
                    .map(|n| Target { binding: self.bind(&n.name, span.end.0, block_end, true), path: Vec::new(), whole_arg: false })
                    .collect();
                self.flows.push(Flow { sources, targets });
            }
            Stmt::Assign { target, op, value, id, span } => {
                for seg in &target.tail {
                    if let LValueSeg::Index(i) = seg {
                        self.expr(i);
                    }
                }
                let sources = self.expr(value);
                let Some(binding) = self.resolve(&target.head.name) else { return };
                let mut path = Vec::new();
                let mut fields_only = true;
                for seg in &target.tail {
                    match seg {
                        LValueSeg::Field(f) if fields_only => path.push(f.name.clone()),
                        _ => fields_only = false,
                    }
                }
                if fields_only && !path.is_empty() && !in_expr && self.loops > 0 && *op == AssignOp::Eq && self.bindings[binding].local {
                    self.stores.push(Store { id: id.0, at: span.start.0, binding, path: path.clone(), value: sources.clone() });
                }
                self.flows.push(Flow { sources, targets: vec![Target { binding, path, whole_arg: false }] });
            }
            Stmt::If(i) => {
                self.if_stmt(i, in_expr);
            }
            Stmt::Match(m) => {
                self.match_expr(m, in_expr);
            }
            Stmt::For { name, iter, body, .. } => {
                let sources = self.expr(iter);
                self.scopes.push(BTreeMap::new());
                let b = self.bind(&name.name, body.span.start.0, body.span.end.0, false);
                self.flows.push(Flow { sources, targets: vec![Target { binding: b, path: Vec::new(), whole_arg: false }] });
                self.loops += 1;
                self.block(body, in_expr);
                self.loops -= 1;
                self.scopes.pop();
            }
            Stmt::While { cond, body, .. } => {
                self.loops += 1;
                self.expr(cond);
                self.block(body, in_expr);
                self.loops -= 1;
            }
            Stmt::Return(e, _) => {
                if let Some(e) = e {
                    self.expr(e);
                }
            }
            Stmt::Fail { value, .. } => {
                self.expr(value);
            }
            Stmt::Send { subject, value, or_disposition, .. } => {
                self.expr(subject);
                self.expr(value);
                if let Some(d) = or_disposition {
                    self.disposition(d);
                }
            }
            Stmt::Block(b) => {
                self.block(b, in_expr);
            }
            Stmt::Expr(e) => {
                self.expr(e);
            }
            Stmt::Recovery { args, .. } => {
                for a in args {
                    self.expr(a);
                }
            }
            Stmt::Violate { payload, .. } => {
                if let Some(p) = payload {
                    self.expr(p);
                }
            }
            Stmt::ShmWrite { max, body, .. } => {
                self.expr(max);
                self.block(body, true);
            }
            Stmt::Break(_) | Stmt::Continue(_) | Stmt::Yield(_) | Stmt::Terminate(_) => {}
        }
    }

    fn if_stmt(&mut self, i: &IfStmt, in_expr: bool) -> Vec<Src> {
        self.expr(&i.cond);
        let mut out = self.block(&i.then_block, in_expr);
        match i.else_block.as_deref() {
            Some(ElseBranch::Else(b)) => out.extend(self.block(b, in_expr)),
            Some(ElseBranch::ElseIf(e)) => out.extend(self.if_stmt(e, in_expr)),
            None => {}
        }
        out
    }

    fn match_expr(&mut self, m: &MatchStmt, in_expr: bool) -> Vec<Src> {
        let scrutinee = self.expr(&m.scrutinee);
        let mut out = Vec::new();
        for arm in &m.arms {
            self.scopes.push(BTreeMap::new());
            let span = match &arm.body {
                MatchArmBody::Expr(e) => e.span(),
                MatchArmBody::Block(b) => b.span,
            };
            let mut names = Vec::new();
            pattern_names(&arm.pattern, &mut names);
            let targets = names
                .iter()
                .map(|n| Target { binding: self.bind(n, span.start.0, span.end.0, false), path: Vec::new(), whole_arg: false })
                .collect();
            self.flows.push(Flow { sources: scrutinee.clone(), targets });
            if let Some(g) = &arm.guard {
                self.expr(g);
            }
            out.extend(match &arm.body {
                MatchArmBody::Expr(e) => self.expr(e),
                MatchArmBody::Block(b) => self.block(b, in_expr),
            });
            self.scopes.pop();
        }
        out
    }

    fn disposition(&mut self, d: &OrDisposition) -> Vec<Src> {
        match d {
            OrDisposition::Substitute(e) => self.expr(e),
            OrDisposition::Fail(e, _) => {
                self.expr(e);
                Vec::new()
            }
            OrDisposition::Raise(_) | OrDisposition::Discard(_) | OrDisposition::Wait(_) => Vec::new(),
        }
    }

    /// Walk `e` (recording the flows its calls and nested statements
    /// make) and answer the places its value may hold blocks of.
    fn expr(&mut self, e: &Expr) -> Vec<Src> {
        match e {
            Expr::Literal(..) | Expr::Path(_) | Expr::KwSelf(_) | Expr::Range { .. } => {
                if let Expr::Range { lo, hi, .. } = e {
                    self.expr(lo);
                    self.expr(hi);
                }
                Vec::new()
            }
            Expr::Ident(_) | Expr::Field { .. } | Expr::Path2 { .. } | Expr::Index { .. } => {
                if let Expr::Index { index, .. } = e {
                    self.expr(index);
                }
                match self.place(e) {
                    Some(s) => vec![s],
                    None => match e {
                        Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. } | Expr::Index { receiver, .. } => {
                            self.expr(receiver)
                        }
                        _ => Vec::new(),
                    },
                }
            }
            Expr::Binary { op, left, right, .. } => {
                let l = self.expr(left);
                let r = self.expr(right);
                match op {
                    BinOp::Eq | BinOp::NotEq | BinOp::Lt | BinOp::Gt | BinOp::LtEq | BinOp::GtEq | BinOp::And | BinOp::Or => {
                        Vec::new()
                    }
                    _ => l.into_iter().chain(r).collect(),
                }
            }
            Expr::Unary { operand, .. } => self.expr(operand),
            Expr::Call { callee, args, .. } => self.call(callee, args),
            Expr::Tuple(xs, _) | Expr::Array(xs, _) => xs.iter().flat_map(|x| self.expr(x)).collect(),
            Expr::Struct { inits, .. } => inits.iter().flat_map(|i| self.expr(&i.value)).collect(),
            Expr::Block(b) => self.block(b, true),
            Expr::If(i) => self.if_stmt(i, true),
            Expr::Match(m) => self.match_expr(m, true),
            Expr::Sum(x, _) | Expr::Prod(x, _) => {
                self.expr(x);
                Vec::new()
            }
            Expr::Approx { left, right, tolerance, .. } => {
                self.expr(left);
                self.expr(right);
                self.expr(tolerance);
                Vec::new()
            }
            Expr::ArrayRepeat { val, .. } => self.expr(val),
            Expr::Or { inner, disposition, .. } => {
                let mut out = self.expr(inner);
                out.extend(self.disposition(disposition));
                out
            }
        }
    }

    fn call(&mut self, callee: &Expr, args: &[Expr]) -> Vec<Src> {
        // A bare builtin name no binding shadows.
        let builtin = match callee {
            Expr::Ident(f) if self.resolve(&f.name).is_none() && !self.user_fn.is_some_and(|u| u(&f.name)) => {
                Some(f.name.as_str())
            }
            _ => None,
        };
        let arg_sources: Vec<Vec<Src>> = args.iter().map(|a| self.expr(a)).collect();
        match builtin {
            Some(n) if reads_only(n) => return Vec::new(),
            Some(n) if reads_element(n) => return arg_sources.into_iter().next().unwrap_or_default(),
            Some(n) if writes_list(n) => {
                let sources: Vec<Src> = arg_sources.iter().skip(1).flatten().cloned().collect();
                if let Some(t) = args.first().and_then(|a| self.place(a)) {
                    self.flows.push(Flow { sources, targets: vec![Target { binding: t.binding, path: t.path, whole_arg: false }] });
                }
                // A writer's result may be an element it took out (`pop`,
                // `remove`) or handed back: it carries every argument.
                return arg_sources.into_iter().flatten().collect();
            }
            _ => {}
        }
        // Any other call: a user fn, a method (its receiver an argument),
        // a fn value. It may write any argument's value into any other.
        let mut handed: Vec<&Expr> = args.iter().collect();
        let receiver = match callee {
            Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. } => Some(receiver.as_ref()),
            _ => None,
        };
        let mut sources: Vec<Src> = arg_sources.into_iter().flatten().collect();
        if let Some(r) = receiver {
            sources.extend(self.expr(r));
            handed.push(r);
        } else if !matches!(callee, Expr::Ident(_) | Expr::Path(_)) {
            sources.extend(self.expr(callee));
        }
        let targets: Vec<Target> = handed
            .iter()
            .filter_map(|a| self.place(a))
            .map(|t| Target { whole_arg: t.path.is_empty(), binding: t.binding, path: t.path })
            .collect();
        if !targets.is_empty() && !sources.is_empty() {
            self.flows.push(Flow { sources: sources.clone(), targets });
        }
        sources
    }

    /// The stores that retire ([`retiring_stores`]).
    fn decide(&self) -> HashSet<u32> {
        let mut out = HashSet::new();
        let fields: BTreeSet<(usize, Vec<String>)> = self.stores.iter().map(|s| (s.binding, s.path.clone())).collect();
        for (q, field) in fields {
            let reaches = |s: &Src, found: &[bool]| (s.binding == q && overlaps(&s.path, &field)) || found[s.binding];
            // The bindings that may hold a block of a copy, to a fixpoint;
            // and whether the local itself may, outside the field.
            let mut found = vec![false; self.bindings.len()];
            let mut tainted = false;
            loop {
                let mut changed = false;
                for f in &self.flows {
                    if !f.sources.iter().any(|s| reaches(s, &found)) {
                        continue;
                    }
                    for t in &f.targets {
                        if t.binding == q {
                            let within = !t.whole_arg && t.path.len() >= field.len() && t.path[..field.len()] == field[..];
                            if !within {
                                tainted = true;
                            }
                        } else if !found[t.binding] {
                            found[t.binding] = true;
                            changed = true;
                        }
                    }
                }
                if !changed {
                    break;
                }
            }
            if tainted {
                continue;
            }
            for s in self.stores.iter().filter(|s| s.binding == q && s.path == field) {
                let in_scope = found
                    .iter()
                    .enumerate()
                    .any(|(b, &f)| f && self.bindings[b].scope.0 <= s.at && s.at < self.bindings[b].scope.1);
                let value_reaches = s.value.iter().any(|v| reaches(v, &found));
                if !in_scope && !value_reaches {
                    out.insert(s.id);
                }
            }
        }
        out
    }
}

/// The names a pattern binds.
fn pattern_names(p: &Pattern, out: &mut Vec<String>) {
    match p {
        Pattern::Binding(i) => out.push(i.name.clone()),
        Pattern::Constructor { args, .. } | Pattern::Tuple(args, _) => {
            for a in args {
                pattern_names(a, out);
            }
        }
        Pattern::Or(alts, _) => {
            for a in alts {
                pattern_names(a, out);
            }
        }
        Pattern::Literal(..) | Pattern::Wildcard(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ids of the stores into a `tags` field in `b`: the bounded
    /// stores lowering would act on (a `label` store is a candidate too,
    /// but no `bounded` place).
    fn tags_stores(b: &Block, out: &mut BTreeSet<u32>) {
        for st in &b.stmts {
            match st {
                Stmt::Assign { target, id, .. } => {
                    if matches!(target.tail.last(), Some(LValueSeg::Field(f)) if f.name == "tags") {
                        out.insert(id.0);
                    }
                }
                Stmt::While { body, .. } | Stmt::For { body, .. } | Stmt::Block(body) => tags_stores(body, out),
                Stmt::If(i) => {
                    tags_stores(&i.then_block, out);
                    if let Some(ElseBranch::Else(e)) = i.else_block.as_deref() {
                        tags_stores(e, out);
                    }
                }
                _ => {}
            }
        }
    }

    /// How many of `f`'s stores into a `tags` field retire, in `src`
    /// (which declares `Tag`, `Row { label, tags }` and the fns).
    fn retiring(src: &str, f: &str) -> usize {
        let src = format!(
            "type Tag {{ name: String = \"\"; }}\ntype Row {{ label: String = \"\"; tags: bounded[Tag; 4]; }}\n{src}\nfn main() {{ }}\n"
        );
        let (program, _) = crate::alloc_summary::tests::minted(&src);
        let fns: BTreeSet<String> = program
            .items
            .iter()
            .filter_map(|i| if let TopDecl::Fn(d) = i { Some(d.name.name.clone()) } else { None })
            .collect();
        let user_fn = |n: &str| fns.contains(n);
        for item in &program.items {
            if let TopDecl::Fn(d) = item {
                if d.name.name == f {
                    let mut tags = BTreeSet::new();
                    tags_stores(&d.body, &mut tags);
                    return retiring_stores(&d.params, &d.body, &user_fn).iter().filter(|id| tags.contains(id)).count();
                }
            }
        }
        panic!("no fn `{f}`");
    }

    #[test]
    fn a_store_no_other_name_reaches_into_retires() {
        let src = "fn f(r: Row, n: Int) -> Int { let mut q = Row { }; let mut t = 0; let mut i = 0;
            while i < n { q.tags = r.tags; t = t + count(q.tags) + len(q.label); i = i + 1; } return t; }";
        assert_eq!(retiring(src, "f"), 1);
    }

    #[test]
    fn a_parameter_or_an_indexed_store_or_one_outside_a_loop_does_not() {
        let src = "fn f(r: Row, q: Row) { while true { q.tags = r.tags; } }
            fn g(r: Row, qs: [Row; 2]) { while true { qs[0].tags = r.tags; } }
            fn h(r: Row) { let mut q = Row { }; while true { q.tags += r.tags; } }
            fn once(r: Row) -> Row { let mut v = Row { }; v.tags = r.tags; return v; }";
        assert_eq!(retiring(src, "f"), 0);
        assert_eq!(retiring(src, "g"), 0);
        assert_eq!(retiring(src, "h"), 0);
        assert_eq!(retiring(src, "once"), 0);
    }

    #[test]
    fn a_binding_read_from_the_field_holds_the_stores_in_its_scope() {
        // `t` is bound after the first store: the loop's are in its scope
        // (and the first, outside a loop, runs once)
        let src = "fn f(r: Row) -> String { let mut q = Row { }; q.tags = r.tags; let t = at(q.tags, 0) or Tag { };
            let mut i = 0; while i < 3 { q.tags = r.tags; i = i + 1; } return t.name; }";
        assert_eq!(retiring(src, "f"), 0);
        // bound in the body after the store: out of scope at the next iteration's
        let src = "fn f(r: Row) -> Int { let mut q = Row { }; let mut n = 0; let mut i = 0;
            while i < 3 { q.tags = r.tags; let t = at(q.tags, 0) or Tag { }; n = n + len(t.name); i = i + 1; } return n; }";
        assert_eq!(retiring(src, "f"), 1);
        // a `for` binder over the list
        let src = "fn f(r: Row) { let mut q = Row { }; for t in q.tags { q.tags = r.tags; println(t.name); } }";
        assert_eq!(retiring(src, "f"), 0);
        // a whole alias of the local
        let src = "fn f(r: Row) -> Int { let mut q = Row { }; let w = q; q.tags = r.tags; return count(w.tags); }";
        assert_eq!(retiring(src, "f"), 0);
    }

    #[test]
    fn a_place_a_read_value_lands_in_is_found_transitively() {
        // into another record declared before the loop, through a binding
        let src = "fn f(r: Row) -> String { let mut q = Row { }; let mut o = Row { }; let mut i = 0;
            while i < 3 { q.tags = r.tags; let t = at(q.tags, 0) or Tag { }; let s = t.name; o.label = s; i = i + 1; } return o.label; }";
        assert_eq!(retiring(src, "f"), 0);
        // pushed into an outer list
        let src = "type Rows { items: bounded[Row; 4]; }
            fn f(r: Row) -> Int { let mut rows = Rows { }; let mut i = 0;
            while i < 3 { let mut q = Row { }; q.tags = r.tags; push(rows.items, q) or discard; i = i + 1; } return count(rows.items); }";
        assert_eq!(retiring(src, "f"), 0);
        // a callee handed the element and another record may write one into the other
        let src = "fn put(o: Row, t: Tag) { o.label = t.name; }
            fn f(r: Row) -> String { let mut q = Row { }; let mut o = Row { }; let mut i = 0;
            while i < 3 { q.tags = r.tags; put(o, at(q.tags, 0) or Tag { }); i = i + 1; } return o.label; }";
        assert_eq!(retiring(src, "f"), 0);
    }

    #[test]
    fn the_local_holding_a_read_value_elsewhere_never_retires() {
        let src = "fn f(r: Row) -> String { let mut q = Row { }; let mut i = 0;
            while i < 3 { q.tags = r.tags; q.label = (at(q.tags, 0) or Tag { }).name; i = i + 1; } return q.label; }";
        assert_eq!(retiring(src, "f"), 0);
        let src = "fn g(q: Row) -> Int { return count(q.tags); }
            fn f(r: Row) -> Int { let mut q = Row { }; let mut n = 0; let mut i = 0;
            while i < 3 { q.tags = r.tags; n = n + g(q); i = i + 1; } return n; }";
        assert_eq!(retiring(src, "f"), 0);
        // writes within the field itself keep it the field's
        let src = "fn f(r: Row) { let mut q = Row { }; let mut i = 0;
            while i < 3 { q.tags = r.tags; push(q.tags, at(q.tags, 0) or Tag { }) or discard; i = i + 1; } }";
        assert_eq!(retiring(src, "f"), 1);
    }

    #[test]
    fn a_store_in_an_expression_or_reading_the_field_does_not() {
        let src = "fn f(r: Row) -> Int { let mut q = Row { }; let n = { q.tags = r.tags; 1 }; return n; }";
        assert_eq!(retiring(src, "f"), 0);
        let src = "fn f(r: Row, c: Bool) { let mut q = Row { }; q.tags = if c { q.tags } else { r.tags }; }";
        assert_eq!(retiring(src, "f"), 0);
    }

    #[test]
    fn a_user_fn_named_like_a_reader_is_a_call() {
        // `count` here may write its argument's element into `o`
        let src = "fn count(l: bounded[Tag; 4], o: Row) -> Int { o.label = (at(l, 0) or Tag { }).name; return 0; }
            fn f(r: Row) -> String { let mut q = Row { }; let mut o = Row { }; let mut i = 0;
            while i < 3 { q.tags = r.tags; let _ = count(q.tags, o); i = i + 1; } return o.label; }";
        assert_eq!(retiring(src, "f"), 0);
    }

    #[test]
    fn a_string_kept_through_to_string_or_a_writers_result_holds_the_stores() {
        // `to_string` of a String answers that String; `pop` the element
        // it took out
        for kept in ["to_string((at(q.tags, 0) or Tag { }).name)", "(pop(q.tags) or Tag { }).name"] {
            let src = format!(
                "fn f(r: Row) -> String {{ let mut q = Row {{ }}; let mut k = \"\"; let mut i = 0;
                while i < 3 {{ q.tags = r.tags; k = {kept}; i = i + 1; }} return k; }}"
            );
            assert_eq!(retiring(&src, "f"), 0, "{kept}");
        }
    }
}

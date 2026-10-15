//! GH #1565: what must finish on the main locus's birth before a server's
//! listener exists.
//!
//! A server whose listener address reads the server (`bind: self.bind`)
//! has its listener (a param of the main locus the serve expansion adds,
//! `rpc_expand`) born just after the param of the main locus the server
//! sits under. Everything the main locus births up to and including that
//! param must therefore finish before the listener can serve. The window
//! is walked over the **actual construction**: each literal of the main
//! locus the programs write anywhere ([`for_each_expression_root`]), a field
//! a literal supplies replacing the default it would otherwise run, at every
//! depth.
//!
//! | construction step in the window | walked |
//! |---|---|
//! | where the main locus is built | every literal of it anywhere the programs hold an expression: a free fn's body and its params' defaults, a const, a record type's field defaults, a locus's param defaults, hooks (`birth`, `run`, …), modes, failure handlers (each with its params' defaults), `birth_check`, closure assertions, fns (with theirs), consts and subscription keys, a perspective's param defaults, `stable_when` and fns; each site's window judged, the program refused if any may not return; with no literal anywhere (built only dynamically), the declaration's defaults |
//! | a param of the main locus, up to and including the server's | its value: the literal's field, else its default, else (no default, typed as a record whose every field has one) the `T { }` the compiler builds for it |
//! | a literal anywhere in a walked expression (a param's value, an array or tuple element, a block's statement or tail, an `if`/`match` arm, a call's argument, another literal's field, a default) | a construction: below |
//! | a locus construction `C { … }` | each param of `C` the literal does not supply (its default, or the synthesized `T { }` of an all-defaulted record param, walked); the `where key == …` key of each of `C`'s subscriptions; `C`'s `birth()` (awaited even when `C` is placed on a pool) and `birth_check`; `C`'s `run()`, run where `C` is born, except the server's own param's when it is placed off the main thread |
//! | a record construction `T { … }` | each field of `T` the literal does not supply: its default, walked in the literal's scope, so a record or locus literal in it is a construction in turn |
//! | a call in a walked expression | resolved as the checker resolves it: a free fn (an imported one through the import renames), `self.m()`, `self.f.m()` by `f`'s type, a standard-library or builtin call by its classes; an enum variant's constructor returns |
//! | the arguments a call leaves out | each omitted param's default, evaluated at the call site |
//! | a const read in a walked expression | its value: a const that is no literal is evaluated where it is read. A read is the checker's resolution of the site (the typed-body table's `ident_reads`): a local or a parameter of the same name in scope shadows the const, a binding of a block that has ended does not |
//! | a resolved fn, a `birth()` or `run()` | judged by its effect row, and its body and every fn its row reaches walked for loops, `or wait`, literals, omitted arguments and consts read; a hook's params' defaults walked first |
//!
//! What runs nothing at construction, so has nothing to walk: an enum
//! variant's payload (no default in the grammar); a generic parameter (a
//! type bound, no value); `@form(…)` arguments and capacity clauses
//! (integer literals the compiler reads); an array type's size and a
//! scalar's `range:` (compile-time constants); `placement`, `topology`,
//! `bindings` and `claims` (the compiler's, emitting no user code); a
//! closure assertion (judged at closure, not at birth); a perspective's
//! `params` (not lowered: a perspective is a contract, its slot holds a
//! locus built by a literal walked above).
//!
//! A step **may not return** when: its effect row is in the `BLOCK` class
//! (interprocedural: a wait on an exposure, a sleep, a read), or reaches an
//! edge the analysis cannot name; a call in a walked expression cannot be
//! resolved (an edge whose return behaviour is unknown, never taken as
//! proof that it returns); a standard-library call is classified `BLOCK` or
//! unclassified; or a `while` loop or an `or wait` sits anywhere in the
//! step's statements or expressions or in a fn it reaches (the classes
//! cannot judge a loop that spins, nor a publish that waits for room). The
//! walk visits every statement and expression ([`visit_block`], exhaustive
//! over the AST, so a new form of either is a compile error here until it
//! is visited).

use std::collections::{BTreeMap, BTreeSet};

use hale_syntax::ast::{
    flat_decls, Block, BusMember, ElseBranch, Expr, IfStmt, KeyFilter, LifecycleKind, LocusDecl, LocusMember,
    MatchArmBody, MatchStmt, OrDisposition, Param, ParamDecl, ParamInit, PlacementSpec, Program, RecoveryModifier,
    Stmt, StructField, TypeDecl, TypeDeclBody, TypeExpr, LValueSeg,
};

use crate::alloc_summary::FnKey;
use crate::effect_rows::EffectRows;
use crate::stdlib_surface::EffectSet;

/// One node a visit reaches.
pub(crate) enum Visited<'a> {
    Stmt(&'a Stmt),
    Expr(&'a Expr),
    /// `or wait`: a publish that waits for room.
    OrWait,
}

/// Visit every statement and expression of `b`, pre-order.
pub(crate) fn visit_block<'a>(b: &'a Block, f: &mut dyn FnMut(Visited<'a>)) {
    for s in &b.stmts {
        visit_stmt(s, f);
    }
    if let Some(t) = &b.tail {
        visit_expr(t, f);
    }
}

fn visit_if<'a>(i: &'a IfStmt, f: &mut dyn FnMut(Visited<'a>)) {
    visit_expr(&i.cond, f);
    visit_block(&i.then_block, f);
    match i.else_block.as_deref() {
        Some(ElseBranch::Else(b)) => visit_block(b, f),
        Some(ElseBranch::ElseIf(e)) => visit_if(e, f),
        None => {}
    }
}

fn visit_match<'a>(m: &'a MatchStmt, f: &mut dyn FnMut(Visited<'a>)) {
    visit_expr(&m.scrutinee, f);
    for arm in &m.arms {
        if let Some(g) = &arm.guard {
            visit_expr(g, f);
        }
        match &arm.body {
            MatchArmBody::Expr(e) => visit_expr(e, f),
            MatchArmBody::Block(b) => visit_block(b, f),
        }
    }
}

fn visit_or<'a>(d: &'a OrDisposition, f: &mut dyn FnMut(Visited<'a>)) {
    match d {
        OrDisposition::Substitute(e) | OrDisposition::Fail(e, _) => visit_expr(e, f),
        OrDisposition::Wait(_) => f(Visited::OrWait),
        OrDisposition::Raise(_) | OrDisposition::Discard(_) => {}
    }
}

pub(crate) fn visit_stmt<'a>(s: &'a Stmt, f: &mut dyn FnMut(Visited<'a>)) {
    f(Visited::Stmt(s));
    match s {
        Stmt::Let { value, .. } | Stmt::LetTuple { value, .. } => visit_expr(value, f),
        Stmt::Assign { target, value, .. } => {
            for seg in &target.tail {
                match seg {
                    LValueSeg::Index(e) => visit_expr(e, f),
                    LValueSeg::Field(_) => {}
                }
            }
            visit_expr(value, f);
        }
        Stmt::If(i) => visit_if(i, f),
        Stmt::Match(m) => visit_match(m, f),
        Stmt::For { iter, body, .. } => {
            visit_expr(iter, f);
            visit_block(body, f);
        }
        Stmt::While { cond, body, .. } => {
            visit_expr(cond, f);
            visit_block(body, f);
        }
        Stmt::Return(e, _) => {
            if let Some(e) = e {
                visit_expr(e, f);
            }
        }
        Stmt::Fail { value, .. } => visit_expr(value, f),
        Stmt::Block(b) => visit_block(b, f),
        Stmt::Recovery { args, modifier, .. } => {
            for a in args {
                visit_expr(a, f);
            }
            match modifier {
                Some(RecoveryModifier::For(e)) | Some(RecoveryModifier::Until(e)) => visit_expr(e, f),
                None => {}
            }
        }
        Stmt::Violate { payload, .. } => {
            if let Some(p) = payload {
                visit_expr(p, f);
            }
        }
        Stmt::Send { subject, value, or_disposition, .. } => {
            visit_expr(subject, f);
            visit_expr(value, f);
            if let Some(d) = or_disposition {
                visit_or(d, f);
            }
        }
        Stmt::ShmWrite { max, body, .. } => {
            visit_expr(max, f);
            visit_block(body, f);
        }
        Stmt::Expr(e) => visit_expr(e, f),
        Stmt::Break(_) | Stmt::Continue(_) | Stmt::Yield(_) | Stmt::Terminate(_) | Stmt::Reperspective { .. } => {}
    }
}

pub(crate) fn visit_expr<'a>(e: &'a Expr, f: &mut dyn FnMut(Visited<'a>)) {
    f(Visited::Expr(e));
    match e {
        Expr::Binary { left, right, .. } => {
            visit_expr(left, f);
            visit_expr(right, f);
        }
        Expr::Unary { operand, .. } => visit_expr(operand, f),
        Expr::Call { callee, args, .. } => {
            visit_expr(callee, f);
            for a in args {
                visit_expr(a, f);
            }
        }
        Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. } => visit_expr(receiver, f),
        Expr::Index { receiver, index, .. } => {
            visit_expr(receiver, f);
            visit_expr(index, f);
        }
        Expr::Tuple(es, _) | Expr::Array(es, _) => {
            for x in es {
                visit_expr(x, f);
            }
        }
        Expr::Struct { inits, .. } => {
            for i in inits {
                visit_expr(&i.value, f);
            }
        }
        Expr::Block(b) => visit_block(b, f),
        Expr::If(i) => visit_if(i, f),
        Expr::Match(m) => visit_match(m, f),
        Expr::Sum(x, _) | Expr::Prod(x, _) => visit_expr(x, f),
        Expr::Approx { left, right, tolerance, .. } => {
            visit_expr(left, f);
            visit_expr(right, f);
            visit_expr(tolerance, f);
        }
        Expr::Range { lo, hi, .. } => {
            visit_expr(lo, f);
            visit_expr(hi, f);
        }
        Expr::ArrayRepeat { val, .. } => visit_expr(val, f),
        Expr::Or { inner, disposition, .. } => {
            visit_expr(inner, f);
            visit_or(disposition, f);
        }
        Expr::Literal(..) | Expr::Ident(_) | Expr::Path(_) | Expr::KwSelf(_) => {}
    }
}

/// A place a program holds an expression: a body, or one expression.
pub(crate) enum Root<'a> {
    Block(&'a Block),
    Expr(&'a Expr),
}

/// Every place the programs hold an expression, exhaustive over the
/// declarations and members that can hold one (a new kind is a compile
/// error here until it is visited): a free fn's body and its params'
/// defaults; a const's value; a record type's field defaults; a locus's
/// param defaults, hooks, modes, failure handlers (each with its params'
/// defaults), closure assertions, fns (with theirs), consts, record types'
/// field defaults and subscriptions' `where key == …` keys; a
/// perspective's param defaults, `stable_when` and fns (with theirs).
pub(crate) fn for_each_expression_root<'a>(items: &'a [hale_syntax::ast::TopDecl], f: &mut dyn FnMut(Root<'a>)) {
    use hale_syntax::ast::{PerspectiveMember, TopDecl};
    let defaults = |params: &'a [Param], f: &mut dyn FnMut(Root<'a>)| {
        for p in params {
            if let Some(d) = &p.default {
                f(Root::Expr(d));
            }
        }
    };
    let fields = |t: &'a TypeDecl, f: &mut dyn FnMut(Root<'a>)| {
        if let TypeDeclBody::Struct(fields) = &t.body {
            for x in fields {
                if let Some(d) = &x.default {
                    f(Root::Expr(d));
                }
            }
        }
    };
    for d in flat_decls(items) {
        match d {
            TopDecl::Fn(func) => {
                defaults(&func.params, f);
                f(Root::Block(&func.body));
            }
            TopDecl::Const(c) => f(Root::Expr(&c.value)),
            TopDecl::Type(t) => fields(t, f),
            TopDecl::Locus(l) => {
                for m in &l.members {
                    match m {
                        LocusMember::Params(pb) => {
                            for p in &pb.params {
                                if let ParamInit::Value(e) = &p.init {
                                    f(Root::Expr(e));
                                }
                            }
                        }
                        LocusMember::Lifecycle(lc) => {
                            defaults(&lc.params, f);
                            f(Root::Block(&lc.body));
                        }
                        LocusMember::Mode(md) => {
                            defaults(&md.params, f);
                            f(Root::Block(&md.body));
                        }
                        LocusMember::Failure(fd) => {
                            defaults(&fd.params, f);
                            f(Root::Block(&fd.body));
                        }
                        LocusMember::Closure(c) => {
                            if let Some(a) = &c.assertion {
                                f(Root::Expr(&a.left));
                                f(Root::Expr(&a.right));
                                f(Root::Expr(&a.tolerance));
                            }
                        }
                        LocusMember::BirthCheck(bc) => f(Root::Expr(&bc.cond)),
                        LocusMember::Fn(func) => {
                            defaults(&func.params, f);
                            f(Root::Block(&func.body));
                        }
                        LocusMember::Const(c) => f(Root::Expr(&c.value)),
                        LocusMember::Type(t) => fields(t, f),
                        LocusMember::Bus(b) => {
                            for k in key_filters(b) {
                                f(Root::Expr(k));
                            }
                        }
                        LocusMember::Contract(_)
                        | LocusMember::Capacity(_)
                        | LocusMember::Bindings(_)
                        | LocusMember::Placement(_)
                        | LocusMember::Topology(_)
                        | LocusMember::Claims(_) => {}
                    }
                }
            }
            TopDecl::Perspective(pd) => {
                for m in &pd.members {
                    match m {
                        PerspectiveMember::Params(pb) => {
                            for p in &pb.params {
                                if let ParamInit::Value(e) = &p.init {
                                    f(Root::Expr(e));
                                }
                            }
                        }
                        PerspectiveMember::StableWhen(b) => f(Root::Block(b)),
                        PerspectiveMember::Fn(func) => {
                            defaults(&func.params, f);
                            f(Root::Block(&func.body));
                        }
                        PerspectiveMember::SerializeAs(_) | PerspectiveMember::Bus(_) => {}
                    }
                }
            }
            // `flat_decls` opens a module; the rest hold no expression
            // a program evaluates
            _ => {}
        }
    }
}

/// The `where key == …` expressions of a locus's subscriptions, evaluated
/// where the locus is built.
fn key_filters(b: &hale_syntax::ast::BusBlock) -> impl Iterator<Item = &Expr> {
    b.members.iter().filter_map(|m| match m {
        BusMember::Subscribe { key_filter: Some(KeyFilter::Specific { expr, .. }), .. } => Some(expr),
        _ => None,
    })
}

/// A fn, a method or a hook by where it is declared: `(locus, name)`.
type Key = (Option<String>, String);

/// The analysis the window asks: the effect rows, the bodies and the
/// params by key, the loci, the record types, the consts, the import
/// renames and the declared types.
pub(crate) struct Window<'a> {
    effects: Option<&'a EffectRows>,
    loci: &'a BTreeMap<&'a str, &'a LocusDecl>,
    bodies: BTreeMap<Key, &'a Block>,
    params: BTreeMap<Key, &'a [Param]>,
    records: BTreeMap<String, &'a [StructField]>,
    consts: BTreeMap<Key, &'a Expr>,
    renames: &'a [(Vec<String>, String)],
    types: BTreeSet<String>,
    /// What the checker resolved each identifier to: a const read is
    /// the checker's answer, never a matching spelling.
    typed: &'a crate::typed_bodies::TypedBodies,
}

/// What a walked expression holds, collected before it is judged.
#[derive(Default)]
struct Found<'a> {
    loops: bool,
    calls: Vec<&'a Expr>,
    literals: Vec<&'a Expr>,
    /// every name read (`Ident`, `Path`): a const among them is its
    /// value, evaluated where it is read
    names: Vec<&'a Expr>,
}

fn found_in_expr(e: &Expr) -> Found<'_> {
    let mut found = Found::default();
    visit_expr(e, &mut |v| collect(v, &mut found));
    found
}

fn found_in_block(b: &Block) -> Found<'_> {
    let mut found = Found::default();
    visit_block(b, &mut |v| collect(v, &mut found));
    found
}

fn collect<'a>(v: Visited<'a>, found: &mut Found<'a>) {
    match v {
        Visited::Stmt(Stmt::While { .. }) | Visited::OrWait => found.loops = true,
        Visited::Expr(e @ Expr::Call { .. }) => found.calls.push(e),
        Visited::Expr(e @ Expr::Struct { .. }) => found.literals.push(e),
        Visited::Expr(e @ (Expr::Ident(_) | Expr::Path(_))) => found.names.push(e),
        _ => {}
    }
}

const LOOP: &str = "a `while` loop or an `or wait`, which may not end";

/// A step named by where it is reached from: `the default of `App.n``,
/// through `settle` when the step is a call or a construction inside it.
fn through(at: String, inner: String) -> String {
    if inner.is_empty() { at } else { format!("{at}, through {inner},") }
}

impl<'a> Window<'a> {
    /// The literals that build the main locus `main` anywhere the
    /// programs hold an expression ([`for_each_expression_root`]): its
    /// actual constructions.
    pub(crate) fn constructions_of(&self, programs: &[&'a Program], main: &str) -> Vec<&'a Expr> {
        let mut out = Vec::new();
        for p in programs {
            for_each_expression_root(&p.items, &mut |root| {
                let found = match root {
                    Root::Block(b) => found_in_block(b),
                    Root::Expr(e) => found_in_expr(e),
                };
                for lit in found.literals {
                    let Expr::Struct { path, .. } = lit else { continue };
                    let segs: Vec<&str> = path.segments.iter().map(|s| s.name.as_str()).collect();
                    if self.resolve_path(&segs) == main {
                        out.push(lit);
                    }
                }
            });
        }
        out
    }

    pub(crate) fn new(
        programs: &[&'a Program],
        loci: &'a BTreeMap<&'a str, &'a LocusDecl>,
        effects: Option<&'a EffectRows>,
        renames: &'a [(Vec<String>, String)],
        typed: &'a crate::typed_bodies::TypedBodies,
    ) -> Self {
        let mut bodies = BTreeMap::new();
        let mut params = BTreeMap::new();
        let mut records = BTreeMap::new();
        let mut consts = BTreeMap::new();
        let mut types = BTreeSet::new();
        let record = |t: &'a TypeDecl, records: &mut BTreeMap<String, &'a [StructField]>| {
            if let TypeDeclBody::Struct(fields) = &t.body {
                records.entry(t.name.name.clone()).or_insert(fields.as_slice());
            }
        };
        for p in programs {
            for d in flat_decls(&p.items) {
                match d {
                    hale_syntax::ast::TopDecl::Fn(f) => {
                        bodies.insert((None, f.name.name.clone()), &f.body);
                        params.insert((None, f.name.name.clone()), f.params.as_slice());
                    }
                    hale_syntax::ast::TopDecl::Const(c) => {
                        consts.insert((None, c.name.name.clone()), &c.value);
                    }
                    hale_syntax::ast::TopDecl::Type(t) => {
                        types.insert(t.name.name.clone());
                        record(t, &mut records);
                    }
                    hale_syntax::ast::TopDecl::Locus(x) => {
                        let at = Some(x.name.name.clone());
                        for m in &x.members {
                            match m {
                                LocusMember::Fn(f) => {
                                    bodies.insert((at.clone(), f.name.name.clone()), &f.body);
                                    params.insert((at.clone(), f.name.name.clone()), f.params.as_slice());
                                }
                                LocusMember::Lifecycle(lc) => {
                                    let hook = match lc.kind {
                                        LifecycleKind::Birth => "birth",
                                        LifecycleKind::Run => "run",
                                        _ => continue,
                                    };
                                    bodies.insert((at.clone(), hook.to_string()), &lc.body);
                                    params.insert((at.clone(), hook.to_string()), lc.params.as_slice());
                                }
                                LocusMember::Const(c) => {
                                    consts.insert((at.clone(), c.name.name.clone()), &c.value);
                                }
                                LocusMember::Type(t) => record(t, &mut records),
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        Window { effects, loci, bodies, params, records, consts, renames, types, typed }
    }

    fn row(&self, locus: Option<&str>, name: &str) -> Option<&'a crate::effect_rows::EffectRow> {
        self.effects?.rows.values().find(|r| r.key.locus.as_deref() == locus && r.key.fn_name == name)
    }

    /// A path as the program names a declaration: itself, or what the
    /// import renames make of it (`lib::settle` is the imported seed's).
    fn resolve_path(&self, segs: &[&str]) -> String {
        let owned: Vec<String> = segs.iter().map(|s| s.to_string()).collect();
        self.renames
            .iter()
            .find(|(path, _)| *path == owned)
            .map_or_else(|| segs.join("::"), |(_, mangled)| mangled.clone())
    }

    /// The locus a literal builds, when it builds one.
    fn literal_locus(&self, e: &Expr) -> Option<&'a LocusDecl> {
        let Expr::Struct { path, .. } = e else { return None };
        let segs: Vec<&str> = path.segments.iter().map(|s| s.name.as_str()).collect();
        let name = self.resolve_path(&segs);
        self.loci.get(name.as_str()).copied()
    }

    /// A resolved fn, judged: its row's classes, and its body and every fn
    /// its row reaches walked for loops, constructions and what a body
    /// evaluates implicitly ([`Window::implicit`]).
    fn judge_fn(&self, row: &'a crate::effect_rows::EffectRow, seen: &mut BTreeSet<&'a FnKey>, depth: usize) -> Option<(String, &'static str)> {
        let show = |k: &FnKey| match &k.locus {
            Some(l) => format!("`{l}::{}`", k.fn_name),
            None => format!("`{}`", k.fn_name),
        };
        if row.effects.contains(EffectSet::BLOCK) {
            return Some((show(&row.key), "it may block: a wait, a sleep or a read, here or in what it calls"));
        }
        if row.unknown {
            return Some((show(&row.key), "it reaches a call the blocking analysis cannot name"));
        }
        let effects = self.effects?;
        let mut stack: Vec<&'a FnKey> = vec![&row.key];
        while let Some(k) = stack.pop() {
            if !seen.insert(k) {
                continue;
            }
            if let Some(body) = self.bodies.get(&(k.locus.clone(), k.fn_name.clone())) {
                let ctx = k.locus.as_deref().and_then(|l| self.loci.get(l).copied());
                let found = found_in_block(body);
                if found.loops {
                    return Some((show(k), LOOP));
                }
                if let Some(step) = self.implicit(&found, ctx, depth + 1) {
                    return Some(step);
                }
                for lit in found.literals {
                    if let Some(step) = self.literal(lit, ctx, true, depth + 1) {
                        return Some(step);
                    }
                }
            }
            if let Some(r) = effects.rows.get(k) {
                stack.extend(r.targets.iter());
            }
        }
        None
    }

    /// A hook (`birth`, `run`) of `locus`: its params' defaults, then its
    /// row.
    fn hook(&self, locus: &'a LocusDecl, hook: &str, depth: usize) -> Option<(String, &'static str)> {
        let key = (Some(locus.name.name.clone()), hook.to_string());
        let at = format!("`{}`'s `{hook}()`", locus.name.name);
        if let Some((inner, why)) = self.omitted_args(&key, 0, Some(locus), depth) {
            return Some((through(at, inner), why));
        }
        let body = self.bodies.get(&key)?;
        if body.stmts.is_empty() && body.tail.is_none() {
            return None;
        }
        let Some(row) = self.row(Some(&locus.name.name), hook) else {
            return Some((at, "it has a statement of its own, and the blocking analysis has no row for it"));
        };
        let mut seen = BTreeSet::new();
        self.judge_fn(row, &mut seen, depth).map(|(_, why)| (at, why))
    }

    /// The declaration a call's callee names, as the checker resolves it:
    /// `f(…)`, `lib::f(…)` through the import renames, `Type::member(…)`,
    /// `self.m(…)`, `self.f.m(…)` by `f`'s type.
    fn callee(&self, callee: &Expr, ctx: Option<&LocusDecl>) -> Option<Key> {
        match callee {
            Expr::Ident(i) => Some((None, i.name.clone())),
            Expr::Field { receiver, name, .. } => {
                let ctx = ctx?;
                let locus = match receiver.as_ref() {
                    Expr::KwSelf(_) => Some(ctx.name.name.clone()),
                    Expr::Field { receiver: inner, name: field, .. } if matches!(inner.as_ref(), Expr::KwSelf(_)) => {
                        crate::surfaces::param_types(ctx).get(&field.name).cloned()
                    }
                    _ => None,
                }?;
                Some((Some(locus), name.name.clone()))
            }
            Expr::Path(qn) => {
                let segs: Vec<&str> = qn.segments.iter().map(|s| s.name.as_str()).collect();
                let name = self.resolve_path(&segs);
                if self.row(None, &name).is_some() || self.params.contains_key(&(None, name.clone())) {
                    return Some((None, name));
                }
                let (member, owner) = segs.split_last()?;
                Some((Some(self.resolve_path(owner)), member.to_string()))
            }
            _ => None,
        }
    }

    /// A call in a walked expression evaluated in `ctx`, resolved as the
    /// checker resolves it.
    fn call(&self, call: &'a Expr, ctx: Option<&LocusDecl>, depth: usize) -> Option<(String, &'static str)> {
        let Expr::Call { callee, .. } = call else { return None };
        // a serve site: the exposure its param builds, whose pieces the walk
        // visits where they are written
        if hale_syntax::ast::ServeSite::of(call).is_some() {
            return None;
        }
        let unnamed = |what: String| Some((what, "an edge whose return the blocking analysis cannot name"));
        let mut seen = BTreeSet::new();
        if let Expr::Path(qn) = callee.as_ref() {
            let segs: Vec<&str> = qn.segments.iter().map(|s| s.name.as_str()).collect();
            if segs.first() == Some(&"std") {
                return match crate::stdlib_surface::effects_for(&segs) {
                    Some(set) if set.contains(EffectSet::BLOCK) => {
                        Some((format!("`{}`", segs.join("::")), "it may block: a standard-library call that blocks"))
                    }
                    Some(set) if !set.is_unclassified() => None,
                    _ => unnamed(format!("`{}`", segs.join("::"))),
                };
            }
        }
        let key = self.callee(callee, ctx);
        if let Some((locus, name)) = &key {
            if let Some(row) = self.row(locus.as_deref(), name) {
                return self.judge_fn(row, &mut seen, depth);
            }
        }
        match (callee.as_ref(), key) {
            (Expr::Ident(i), _) if crate::check::BARE_BUILTIN_CALLEES.contains(&i.name.as_str()) => None,
            // an enum variant's constructor
            (Expr::Path(_), Some((Some(owner), _))) if self.types.contains(&owner) => None,
            (Expr::Ident(i), _) => unnamed(format!("the call of `{}`", i.name)),
            (Expr::Field { name, .. }, _) => unnamed(format!("the call of `{}`", name.name)),
            (Expr::Path(qn), _) => {
                let segs: Vec<&str> = qn.segments.iter().map(|s| s.name.as_str()).collect();
                unnamed(format!("`{}`", segs.join("::")))
            }
            _ => unnamed("a call through a value".to_string()),
        }
    }

    /// The defaults a call of `key` leaves to its params from `given` on,
    /// evaluated at the call site in `ctx`.
    fn omitted_args(&self, key: &Key, given: usize, ctx: Option<&'a LocusDecl>, depth: usize) -> Option<(String, &'static str)> {
        let params = self.params.get(key)?;
        for p in params.iter().skip(given) {
            if let Some(d) = &p.default {
                if let Some((at, why)) = self.expr(d, ctx, depth + 1) {
                    let owner = key.0.as_ref().map(|l| format!("{l}::")).unwrap_or_default();
                    return Some((through(format!("the default of `{owner}{}`'s param `{}`", key.1, p.name.name), at), why));
                }
            }
        }
        None
    }

    /// What a walked expression or body evaluates without writing it: the
    /// defaults its calls leave to their params, and the value of each
    /// const it reads (a const that is not a literal is evaluated where
    /// it is read).
    fn implicit(&self, found: &Found<'a>, ctx: Option<&'a LocusDecl>, depth: usize) -> Option<(String, &'static str)> {
        if depth > 32 {
            return None;
        }
        for c in &found.calls {
            let Expr::Call { callee, args, .. } = c else { continue };
            if let Some(key) = self.callee(callee, ctx) {
                if let Some(step) = self.omitted_args(&key, args.len(), ctx, depth) {
                    return Some(step);
                }
            }
        }
        for n in &found.names {
            let (key, shown) = match n {
                // a const read is the checker's resolution of the site: a
                // local or a parameter in scope shadows the const, a
                // binding of a block that has ended does not; a site the
                // checker never typed (the default of a record field no
                // literal leaves, which only the compiler's `T { }`
                // builds, with no binding in scope) is read by its name
                Expr::Ident(i) => {
                    if self.typed.reads_const(i.id) == Some(false) {
                        continue;
                    }
                    ((None, i.name.clone()), i.name.clone())
                }
                Expr::Path(qn) => {
                    let segs: Vec<&str> = qn.segments.iter().map(|s| s.name.as_str()).collect();
                    let name = self.resolve_path(&segs);
                    let key = if self.consts.contains_key(&(None, name.clone())) {
                        (None, name)
                    } else {
                        match segs.split_last() {
                            Some((member, owner)) if !owner.is_empty() => (Some(self.resolve_path(owner)), member.to_string()),
                            _ => (None, name),
                        }
                    };
                    (key, segs.join("::"))
                }
                _ => continue,
            };
            if let Some(value) = self.consts.get(&key) {
                if let Some((at, why)) = self.expr(value, ctx, depth + 1) {
                    return Some((through(format!("the value of the const `{shown}`"), at), why));
                }
            }
        }
        None
    }

    /// A walked expression evaluated in `ctx`: its loops, its calls, what it
    /// evaluates implicitly, and each literal in it, built (a locus's
    /// `run()` run where it is born).
    fn expr(&self, e: &'a Expr, ctx: Option<&'a LocusDecl>, depth: usize) -> Option<(String, &'static str)> {
        if depth > 32 {
            return None;
        }
        let found = found_in_expr(e);
        if found.loops {
            return Some((String::new(), LOOP));
        }
        for c in &found.calls {
            if let Some(step) = self.call(c, ctx, depth) {
                return Some(step);
            }
        }
        if let Some(step) = self.implicit(&found, ctx, depth) {
            return Some(step);
        }
        for lit in found.literals {
            if let Some(step) = self.literal(lit, ctx, true, depth + 1) {
                return Some(step);
            }
        }
        None
    }

    /// The record type a param of type `T` and no default is built as: the
    /// compiler builds `T { }` for it when every field of `T` has a default.
    fn synthesized(&self, p: &ParamDecl) -> Option<(&str, &'a [StructField])> {
        if !matches!(p.init, ParamInit::Inferred) {
            return None;
        }
        let Some(TypeExpr::Named { path, generic_args, .. }) = &p.ty else { return None };
        if !generic_args.is_empty() || path.segments.len() != 1 {
            return None;
        }
        let (name, fields) = self.records.get_key_value(&path.segments[0].name)?;
        (!fields.is_empty() && fields.iter().all(|f| f.default.is_some())).then_some((name.as_str(), *fields))
    }

    /// The fields of the record `ty` a literal does not supply (`given`):
    /// their defaults, walked in `ctx`, the literal's scope.
    fn omitted_fields(&self, ty: &str, fields: &'a [StructField], given: &BTreeSet<&str>, ctx: Option<&'a LocusDecl>, depth: usize) -> Option<(String, &'static str)> {
        for f in fields {
            if given.contains(f.name.name.as_str()) {
                continue;
            }
            if let Some(d) = &f.default {
                if let Some((at, why)) = self.expr(d, ctx, depth) {
                    return Some((through(format!("the default of `{ty}.{}`", f.name.name), at), why));
                }
            }
        }
        None
    }

    /// A literal built, in `ctx`. A record `T { … }`: the fields it does
    /// not supply (their defaults, walked). A locus `C { … }`: the params
    /// of `C` it does not supply (their defaults, or the `T { }` the
    /// compiler builds for an all-defaulted record param, walked), the
    /// `where key == …` keys of `C`'s subscriptions, `C`'s `birth()`, its
    /// `birth_check`, and its `run()` when `with_run`. The fields a literal
    /// supplies are walked where the literal is.
    fn literal(&self, lit: &'a Expr, ctx: Option<&'a LocusDecl>, with_run: bool, depth: usize) -> Option<(String, &'static str)> {
        if depth > 32 {
            return None;
        }
        let Expr::Struct { path, inits, .. } = lit else { return None };
        let given: BTreeSet<&str> = inits.iter().map(|i| i.name.name.as_str()).collect();
        let segs: Vec<&str> = path.segments.iter().map(|s| s.name.as_str()).collect();
        let name = self.resolve_path(&segs);
        let record = self.records.get_key_value(&name).or_else(|| segs.last().and_then(|l| self.records.get_key_value(*l)));
        if let Some((ty, fields)) = record {
            return self.omitted_fields(ty, fields, &given, ctx, depth + 1);
        }
        let Some(locus) = self.literal_locus(lit) else { return None };
        for m in &locus.members {
            let LocusMember::Params(pb) = m else { continue };
            for p in &pb.params {
                if given.contains(p.name.name.as_str()) {
                    continue;
                }
                let at = format!("the default of `{}.{}`", locus.name.name, p.name.name);
                let step = match &p.init {
                    ParamInit::Value(e) => self.expr(e, Some(locus), depth),
                    ParamInit::Inferred => self
                        .synthesized(p)
                        .and_then(|(ty, fields)| self.omitted_fields(ty, fields, &BTreeSet::new(), Some(locus), depth + 1)),
                };
                if let Some((inner, why)) = step {
                    return Some((through(at, inner), why));
                }
            }
        }
        for m in &locus.members {
            let LocusMember::Bus(b) = m else { continue };
            for k in key_filters(b) {
                if let Some((at, why)) = self.expr(k, Some(locus), depth) {
                    return Some((through(format!("a `where key == …` of `{}`", locus.name.name), at), why));
                }
            }
        }
        if let Some(step) = self.hook(locus, "birth", depth) {
            return Some(step);
        }
        // `birth_check` runs after `birth()`, before `run()`
        for m in &locus.members {
            if let LocusMember::BirthCheck(bc) = m {
                if let Some((at, why)) = self.expr(&bc.cond, Some(locus), depth) {
                    return Some((through(format!("`{}`'s `birth_check`", locus.name.name), at), why));
                }
            }
        }
        if with_run {
            if let Some(step) = self.hook(locus, "run", depth) {
                return Some(step);
            }
        }
        None
    }

    /// The first step the main locus `main` births up to and including the
    /// param `root`, over its actual constructions (`site`: the literals a
    /// free fn builds it with; none, its declaration's defaults), that may
    /// not return: where, and why. Compiler-added params (`__…`) are the
    /// runtime's.
    pub(crate) fn before_listener(&self, main: &'a LocusDecl, root: &str, sites: &[&'a Expr]) -> Option<(String, &'static str)> {
        let plain = [None];
        let sites: Vec<Option<&'a Expr>> =
            if sites.is_empty() { plain.to_vec() } else { sites.iter().map(|s| Some(*s)).collect() };
        for site in sites {
            let supplied = |name: &str| -> Option<&'a Expr> {
                let Some(Expr::Struct { inits, .. }) = site else { return None };
                inits.iter().find(|i| i.name.name == name).map(|i| &i.value)
            };
            'blocks: for m in &main.members {
                let LocusMember::Params(pb) = m else { continue };
                for p in &pb.params {
                    let name = p.name.name.as_str();
                    if !name.starts_with("__") {
                        let value: Option<&'a Expr> = supplied(name).or(match &p.init {
                            ParamInit::Value(e) => Some(e),
                            ParamInit::Inferred => None,
                        });
                        let on_main = match crate::rpc_expand::placement_entry(main, name) {
                            None => true,
                            Some(e) => matches!(&e.spec, PlacementSpec::Cooperative { pool, .. } if pool.as_ref().is_none_or(|p| p.name == "main")),
                        };
                        let step = match value {
                            // the param's own literal: its run() is the
                            // param's, not awaited when it is placed
                            Some(value @ Expr::Struct { inits, .. }) if self.literal_locus(value).is_some() => self
                                .literal(value, Some(main), on_main, 0)
                                .or_else(|| inits.iter().find_map(|i| self.expr(&i.value, Some(main), 0))),
                            Some(value) => self.expr(value, Some(main), 0),
                            // no value written: the `T { }` the compiler
                            // builds for an all-defaulted record param
                            None => self
                                .synthesized(p)
                                .and_then(|(ty, fields)| self.omitted_fields(ty, fields, &BTreeSet::new(), Some(main), 1)),
                        };
                        if let Some((at, why)) = step {
                            let what = if supplied(name).is_some() { "the value given" } else { "the default" };
                            return Some((through(format!("{what} of `{}.{name}`", main.name.name), at), why));
                        }
                    }
                    if name == root {
                        break 'blocks;
                    }
                }
            }
        }
        None
    }
}

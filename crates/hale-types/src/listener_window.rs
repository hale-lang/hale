//! GH #1565: what must finish on the main locus's birth before a server's
//! listener exists.
//!
//! A server whose listener address reads the server (`bind: self.bind`)
//! has its listener (a param of the main locus the serve expansion adds,
//! `rpc_expand`) born just after the param of the main locus the server
//! sits under. Everything the main locus births up to and including that
//! param must therefore finish before the listener can serve. The window
//! is walked over the **actual construction**: the literal of the main
//! locus a free fn writes (`fn main() { App { server: Server { bind: … } }; }`),
//! or its declaration's defaults, a field a literal supplies replacing the
//! default it would otherwise run, at every depth.
//!
//! | construction step in the window | walked |
//! |---|---|
//! | a param of the main locus, up to and including the server's | its value: the literal's field, else its default |
//! | a locus literal anywhere in a walked expression (a param's value, an array or tuple element, a block's statement or tail, an `if`/`match` arm, a call's argument, another literal's field) | a construction: below |
//! | a construction `C { … }` | each param of `C` the literal does not supply (its default, walked); `C`'s `birth()` (awaited even when `C` is placed on a pool); `C`'s `run()`, run where `C` is born, except the server's own param's when it is placed off the main thread |
//! | a call in a walked expression | resolved as the checker resolves it: a free fn (an imported one through the import renames), `self.m()`, `self.f.m()` by `f`'s type, a standard-library or builtin call by its classes; an enum variant's constructor returns |
//! | a resolved fn, a `birth()` or `run()` | judged by its effect row, and its body and every fn its row reaches walked for loops, `or wait` and locus literals |
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
    flat_decls, Block, ElseBranch, Expr, IfStmt, LifecycleKind, LocusDecl, LocusMember, MatchArmBody,
    MatchStmt, OrDisposition, ParamInit, PlacementSpec, Program, RecoveryModifier, Stmt, LValueSeg,
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

/// The analysis the window asks: the effect rows, the bodies by key, the
/// loci, the import renames and the declared types.
pub(crate) struct Window<'a> {
    effects: Option<&'a EffectRows>,
    loci: &'a BTreeMap<&'a str, &'a LocusDecl>,
    bodies: BTreeMap<(Option<String>, String), &'a Block>,
    renames: &'a [(Vec<String>, String)],
    types: BTreeSet<String>,
}

/// What a walked expression holds, collected before it is judged.
#[derive(Default)]
struct Found<'a> {
    loops: bool,
    calls: Vec<&'a Expr>,
    literals: Vec<&'a Expr>,
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
    /// The literals that build the main locus `main` in the programs' free
    /// fns (`fn main() { App { … }; }`): its actual constructions.
    pub(crate) fn constructions_of(&self, programs: &[&'a Program], main: &str) -> Vec<&'a Expr> {
        let mut out = Vec::new();
        for p in programs {
            for d in flat_decls(&p.items) {
                if let hale_syntax::ast::TopDecl::Fn(f) = d {
                    for lit in found_in_block(&f.body).literals {
                        let Expr::Struct { path, .. } = lit else { continue };
                        let segs: Vec<&str> = path.segments.iter().map(|s| s.name.as_str()).collect();
                        if self.resolve_path(&segs) == main {
                            out.push(lit);
                        }
                    }
                }
            }
        }
        out
    }

    pub(crate) fn new(
        programs: &[&'a Program],
        loci: &'a BTreeMap<&'a str, &'a LocusDecl>,
        effects: Option<&'a EffectRows>,
        renames: &'a [(Vec<String>, String)],
    ) -> Self {
        let mut bodies = BTreeMap::new();
        let mut types = BTreeSet::new();
        for p in programs {
            for d in flat_decls(&p.items) {
                match d {
                    hale_syntax::ast::TopDecl::Fn(f) => {
                        bodies.insert((None, f.name.name.clone()), &f.body);
                    }
                    hale_syntax::ast::TopDecl::Type(t) => {
                        types.insert(t.name.name.clone());
                    }
                    hale_syntax::ast::TopDecl::Locus(x) => {
                        for m in &x.members {
                            match m {
                                LocusMember::Fn(f) => {
                                    bodies.insert((Some(x.name.name.clone()), f.name.name.clone()), &f.body);
                                }
                                LocusMember::Lifecycle(lc) => {
                                    let hook = match lc.kind {
                                        LifecycleKind::Birth => "birth",
                                        LifecycleKind::Run => "run",
                                        _ => continue,
                                    };
                                    bodies.insert((Some(x.name.name.clone()), hook.to_string()), &lc.body);
                                }
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        Window { effects, loci, bodies, renames, types }
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
    /// its row reaches walked for loops and constructions.
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
                let found = found_in_block(body);
                if found.loops {
                    return Some((show(k), LOOP));
                }
                for lit in found.literals {
                    if let Some(step) = self.literal(lit, true, depth + 1) {
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

    /// A hook (`birth`, `run`) of `locus`.
    fn hook(&self, locus: &LocusDecl, hook: &str, depth: usize) -> Option<(String, &'static str)> {
        let body = self.bodies.get(&(Some(locus.name.name.clone()), hook.to_string()))?;
        if body.stmts.is_empty() && body.tail.is_none() {
            return None;
        }
        let at = format!("`{}`'s `{hook}()`", locus.name.name);
        let Some(row) = self.row(Some(&locus.name.name), hook) else {
            return Some((at, "it has a statement of its own, and the blocking analysis has no row for it"));
        };
        let mut seen = BTreeSet::new();
        self.judge_fn(row, &mut seen, depth).map(|(_, why)| (at, why))
    }

    /// A call in a walked expression evaluated in `ctx`, resolved as the
    /// checker resolves it.
    fn call(&self, call: &'a Expr, ctx: &LocusDecl, depth: usize) -> Option<(String, &'static str)> {
        let Expr::Call { callee, .. } = call else { return None };
        // a serve site: the exposure its param builds, whose pieces the walk
        // visits where they are written
        if hale_syntax::ast::ServeSite::of(call).is_some() {
            return None;
        }
        let unnamed = |what: String| Some((what, "an edge whose return the blocking analysis cannot name"));
        let mut seen = BTreeSet::new();
        match callee.as_ref() {
            Expr::Ident(i) => match self.row(None, &i.name) {
                Some(row) => self.judge_fn(row, &mut seen, depth),
                None if crate::check::BARE_BUILTIN_CALLEES.contains(&i.name.as_str()) => None,
                None => unnamed(format!("the call of `{}`", i.name)),
            },
            Expr::Field { receiver, name, .. } => {
                let locus = match receiver.as_ref() {
                    Expr::KwSelf(_) => Some(ctx.name.name.clone()),
                    Expr::Field { receiver: inner, name: field, .. } if matches!(inner.as_ref(), Expr::KwSelf(_)) => {
                        crate::surfaces::param_types(ctx).get(&field.name).cloned()
                    }
                    _ => None,
                };
                match locus.and_then(|l| self.row(Some(&l), &name.name)) {
                    Some(row) => self.judge_fn(row, &mut seen, depth),
                    None => unnamed(format!("the call of `{}`", name.name)),
                }
            }
            Expr::Path(qn) => {
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
                let name = self.resolve_path(&segs);
                if let Some(row) = self.row(None, &name) {
                    return self.judge_fn(row, &mut seen, depth);
                }
                // `Type::member`: an associated fn the rows key under the
                // type, or an enum variant's constructor
                if let Some((member, owner)) = segs.split_last() {
                    let owner = self.resolve_path(owner);
                    if let Some(row) = self.row(Some(&owner), member) {
                        return self.judge_fn(row, &mut seen, depth);
                    }
                    if self.types.contains(&owner) {
                        return None;
                    }
                }
                unnamed(format!("`{}`", segs.join("::")))
            }
            _ => unnamed("a call through a value".to_string()),
        }
    }

    /// A walked expression evaluated in `ctx`: its loops, its calls, and
    /// each locus literal in it, built (its `run()` run where it is born).
    fn expr(&self, e: &'a Expr, ctx: &LocusDecl, depth: usize) -> Option<(String, &'static str)> {
        let found = found_in_expr(e);
        if found.loops {
            return Some((String::new(), LOOP));
        }
        for c in found.calls {
            if let Some(step) = self.call(c, ctx, depth) {
                return Some(step);
            }
        }
        for lit in found.literals {
            if let Some(step) = self.literal(lit, true, depth + 1) {
                return Some(step);
            }
        }
        None
    }

    /// A locus literal `C { … }` built: the params of `C` it does not
    /// supply (their defaults, walked; the fields it supplies are walked
    /// where the literal is), `C`'s `birth()`, and its `run()` when
    /// `with_run`.
    fn literal(&self, lit: &'a Expr, with_run: bool, depth: usize) -> Option<(String, &'static str)> {
        if depth > 32 {
            return None;
        }
        let Some(locus) = self.literal_locus(lit) else { return None };
        let Expr::Struct { inits, .. } = lit else { return None };
        let given: BTreeSet<&str> = inits.iter().map(|i| i.name.name.as_str()).collect();
        for m in &locus.members {
            let LocusMember::Params(pb) = m else { continue };
            for p in &pb.params {
                if given.contains(p.name.name.as_str()) {
                    continue;
                }
                if let ParamInit::Value(e) = &p.init {
                    if let Some((at, why)) = self.expr(e, locus, depth) {
                        return Some((through(format!("the default of `{}.{}`", locus.name.name, p.name.name), at), why));
                    }
                }
            }
        }
        if let Some(step) = self.hook(locus, "birth", depth) {
            return Some(step);
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
                        if let Some(value) = value {
                            let on_main = match crate::rpc_expand::placement_entry(main, name) {
                                None => true,
                                Some(e) => matches!(&e.spec, PlacementSpec::Cooperative { pool, .. } if pool.as_ref().is_none_or(|p| p.name == "main")),
                            };
                            // the param's own literal: its run() is the
                            // param's, not awaited when it is placed
                            let step = match value {
                                Expr::Struct { inits, .. } if self.literal_locus(value).is_some() => self
                                    .literal(value, on_main, 0)
                                    .or_else(|| inits.iter().find_map(|i| self.expr(&i.value, main, 0))),
                                _ => self.expr(value, main, 0),
                            };
                            if let Some((at, why)) = step {
                                let what = if supplied(name).is_some() { "the value given" } else { "the default" };
                                return Some((through(format!("{what} of `{}.{name}`", main.name.name), at), why));
                            }
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

//! GH #1565: what must finish on the main locus's birth before a server's
//! listener exists.
//!
//! A server whose listener address reads the server (`bind: self.bind`)
//! has its listener (a param of the main locus the serve expansion adds,
//! `rpc_expand`) born just after the param of the main locus the server
//! sits under. Everything the main locus births up to and including that
//! param must therefore finish before the listener can serve:
//!
//! - each param's default, evaluated where it is born (a call it makes);
//! - for a param that is a locus: its own params, recursively (a child is
//!   built inline, its `run()` run where it is born: spec/semantics.md §
//!   Birth order is load-bearing), its `birth()` (awaited even when the
//!   param is placed on a pool: the main locus waits for a placed child's
//!   readiness), and its `run()` when it runs on the main thread (the
//!   param unplaced, or on pool `main`; a placed root's `run()` runs on its
//!   pool, not awaited).
//!
//! A step **may not return** when the blocking analysis says so: the effect
//! rows' `BLOCK` class, interprocedural (a wait on an exposure, a sleep, a
//! read, reached through any fn or method the step calls), or an edge the
//! analysis cannot name. The classes cannot judge a loop that spins without
//! blocking, nor a publish that waits for room (`or wait`), so a `while`
//! loop and an `or wait` anywhere in a step's body or expressions, or in a
//! fn it reaches, count too: the walk visits every statement and every
//! expression ([`visit_block`], exhaustive over the AST, so a new form of
//! either is a compile error here until it is visited).

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

/// The analysis the window asks: the effect rows and the bodies by key.
pub(crate) struct Window<'a> {
    effects: Option<&'a EffectRows>,
    loci: &'a BTreeMap<&'a str, &'a LocusDecl>,
    bodies: BTreeMap<(Option<String>, String), &'a Block>,
}

impl<'a> Window<'a> {
    pub(crate) fn new(programs: &[&'a Program], loci: &'a BTreeMap<&'a str, &'a LocusDecl>, effects: Option<&'a EffectRows>) -> Self {
        let mut bodies = BTreeMap::new();
        for p in programs {
            for d in flat_decls(&p.items) {
                match d {
                    hale_syntax::ast::TopDecl::Fn(f) => {
                        bodies.insert((None, f.name.name.clone()), &f.body);
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
        Window { effects, loci, bodies }
    }

    fn row(&self, locus: Option<&str>, name: &str) -> Option<&'a crate::effect_rows::EffectRow> {
        self.effects?.rows.values().find(|r| r.key.locus.as_deref() == locus && r.key.fn_name == name)
    }

    /// Whether a body may spin or wait where the classes say nothing: a
    /// `while` loop or an `or wait`, here or in a fn it reaches.
    fn loops(&self, b: &Block, row: Option<&crate::effect_rows::EffectRow>) -> bool {
        let looping = |b: &Block| {
            let mut found = false;
            visit_block(b, &mut |v| {
                if matches!(v, Visited::Stmt(Stmt::While { .. }) | Visited::OrWait) {
                    found = true;
                }
            });
            found
        };
        if looping(b) {
            return true;
        }
        let (Some(effects), Some(row)) = (self.effects, row) else { return false };
        let mut seen: BTreeSet<&FnKey> = BTreeSet::new();
        let mut stack: Vec<&FnKey> = row.targets.iter().collect();
        while let Some(k) = stack.pop() {
            if !seen.insert(k) {
                continue;
            }
            if self.bodies.get(&(k.locus.clone(), k.fn_name.clone())).is_some_and(|b| looping(b)) {
                return true;
            }
            if let Some(r) = effects.rows.get(k) {
                stack.extend(r.targets.iter());
            }
        }
        false
    }

    /// Why a row's fn may not return, by the classes.
    fn row_blocks(row: &crate::effect_rows::EffectRow) -> Option<&'static str> {
        if row.effects.contains(EffectSet::BLOCK) {
            return Some("it may block: a wait, a sleep or a read, here or in what it calls");
        }
        if row.unknown {
            return Some("it reaches a call the blocking analysis cannot name");
        }
        None
    }

    /// Why `locus`'s hook (`birth`, `run`) may not return.
    fn hook(&self, locus: &LocusDecl, hook: &str) -> Option<&'static str> {
        let body = self.bodies.get(&(Some(locus.name.name.clone()), hook.to_string()))?;
        if body.stmts.is_empty() && body.tail.is_none() {
            return None;
        }
        let Some(row) = self.row(Some(&locus.name.name), hook) else {
            return Some("it has a statement of its own, and the blocking analysis has no row for it");
        };
        if let Some(why) = Self::row_blocks(row) {
            return Some(why);
        }
        self.loops(body, Some(row)).then_some("a `while` loop or an `or wait`, here or in what it calls, which may not end")
    }

    /// Why an expression evaluated in `ctx` (a param's default) may not
    /// return: a call that may block, a loop, an `or wait`.
    fn expr(&self, e: &Expr, ctx: &LocusDecl) -> Option<&'static str> {
        let mut why: Option<&'static str> = None;
        visit_expr(e, &mut |v| {
            if why.is_some() {
                return;
            }
            match v {
                Visited::Stmt(Stmt::While { .. }) | Visited::OrWait => {
                    why = Some("a `while` loop or an `or wait`, which may not end");
                }
                Visited::Expr(Expr::Call { callee, .. }) => {
                    let row = match callee.as_ref() {
                        Expr::Ident(i) => self.row(None, &i.name).map(Some).unwrap_or_else(|| {
                            // a builtin, by its classes
                            if crate::stdlib_surface::builtin_effects(&i.name).is_some_and(|s| s.contains(EffectSet::BLOCK)) {
                                why = Some("it may block: a builtin that blocks");
                            }
                            None
                        }),
                        Expr::Field { receiver, name, .. } if matches!(receiver.as_ref(), Expr::KwSelf(_)) => {
                            self.row(Some(&ctx.name.name), &name.name)
                        }
                        Expr::Path(qn) => {
                            let segs: Vec<&str> = qn.segments.iter().map(|s| s.name.as_str()).collect();
                            match crate::stdlib_surface::effects_for(&segs) {
                                Some(s) if s.contains(EffectSet::BLOCK) || s.is_unclassified() => {
                                    why = Some("it may block: a standard-library call that blocks");
                                }
                                _ => {}
                            }
                            None
                        }
                        // a method of another instance: its class is the
                        // receiver's, which this walk does not type
                        _ => {
                            why = Some("it calls a method the blocking analysis cannot name here");
                            None
                        }
                    };
                    if let Some(row) = row {
                        if let Some(w) = Self::row_blocks(row) {
                            why = Some(w);
                        } else if self.bodies.get(&(row.key.locus.clone(), row.key.fn_name.clone())).is_some_and(|b| self.loops(b, Some(row))) {
                            why = Some("a `while` loop or an `or wait` in what it calls, which may not end");
                        }
                    }
                }
                _ => {}
            }
        });
        why
    }

    /// The first step of building `locus` (its params, recursively, its
    /// `birth()`, and its `run()` when `with_run`) that may not return:
    /// where, and why.
    pub(crate) fn construction(&self, locus: &LocusDecl, with_run: bool, depth: usize) -> Option<(String, &'static str)> {
        if depth > 16 {
            return None;
        }
        let types = crate::surfaces::param_types(locus);
        for m in &locus.members {
            let LocusMember::Params(pb) = m else { continue };
            for p in &pb.params {
                if let ParamInit::Value(e) = &p.init {
                    if let Some(why) = self.expr(e, locus) {
                        return Some((format!("the default of `{}.{}`", locus.name.name, p.name.name), why));
                    }
                }
                if let Some(child) = types.get(&p.name.name).and_then(|t| self.loci.get(t.as_str())) {
                    if !child.is_main {
                        if let Some(found) = self.construction(child, true, depth + 1) {
                            return Some(found);
                        }
                    }
                }
            }
        }
        if let Some(why) = self.hook(locus, "birth") {
            return Some((format!("`{}`'s `birth()`", locus.name.name), why));
        }
        if with_run {
            if let Some(why) = self.hook(locus, "run") {
                return Some((format!("`{}`'s `run()`", locus.name.name), why));
            }
        }
        None
    }

    /// The first step the main locus `main` births up to and including the
    /// param `root` that may not return: each param's default, and each
    /// locus param's construction, its `run()` included when it runs on the
    /// main thread. Compiler-added params (`__…`) are the runtime's.
    pub(crate) fn before_listener(&self, main: &LocusDecl, root: &str) -> Option<(String, &'static str)> {
        let types = crate::surfaces::param_types(main);
        for m in &main.members {
            let LocusMember::Params(pb) = m else { continue };
            for p in &pb.params {
                let name = p.name.name.as_str();
                if !name.starts_with("__") {
                    if let ParamInit::Value(e) = &p.init {
                        if let Some(why) = self.expr(e, main) {
                            return Some((format!("the default of `{}.{name}`", main.name.name), why));
                        }
                    }
                    if let Some(child) = types.get(name).and_then(|t| self.loci.get(t.as_str())) {
                        let on_main = match crate::rpc_expand::placement_entry(main, name) {
                            None => true,
                            Some(e) => matches!(&e.spec, PlacementSpec::Cooperative { pool, .. } if pool.as_ref().is_none_or(|p| p.name == "main")),
                        };
                        if let Some(found) = self.construction(child, on_main, 0) {
                            return Some(found);
                        }
                    }
                }
                if name == root {
                    return None;
                }
            }
        }
        None
    }
}

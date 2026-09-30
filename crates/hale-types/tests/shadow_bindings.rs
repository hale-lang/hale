//! The binding-facts shadow (F.40 phase 1.2b): the three name-keyed
//! maps lowering joined to a `let` through `current_fn`'s LLVM name,
//! against the owner table's one row per binding site.
//!
//! OLD is what lowering answered at a `let` before 1.2b: the
//! returned-bindings map (`ResolvedProgram::returned_bindings`,
//! `let_is_returned` asked at the `let`'s identifier), the `=` map
//! (`compute_assign_moved_bindings`) and the stack-array map
//! (`compute_stack_array_bindings`), each looked up under the LLVM
//! name the body lowers under — `fn_name` for a free fn,
//! `{locus}.{member}` for a locus fn, mode or lifecycle — and asked by
//! the binding's name. A body the lowering emits under another name
//! finds no entry and answers `false`: a generic fn or locus lowers
//! only as its monomorphs (`mangle_generic_name`), and an `@export`
//! fn's body lowers as `__hale_impl_{name}`. Every other `let` —
//! a body no map walked — answers `false` three times. NEW is
//! `OwnerTable::binding_facts` at the `let`'s snapshot identity. Both
//! are keyed by the site's snapshot index, so the correspondence is
//! the identity itself.
//!
//! Every corpus program `resolve_program` accepts is shadowed; the
//! ones it refuses decide nothing and are skipped, as the placement
//! shadow skips the checker's refusals. A program is named by its
//! content (`hale_graph::shadow::program_id`).
//!
//! The gate: every divergence over the corpus is classified in
//! `fixtures/shadow_bindings.txt` (known old bug, correction, spec
//! disagreement) with a note, and none is a regression. Regenerate
//! the fixture with `HALE_SHADOW_REGEN=1`, then classify by hand.

use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

use hale_graph::shadow::{gate_message, parse_fixture, program_id, Report};
use hale_syntax::ast::{
    Block, ElseBranch, Expr, Ident, IfStmt, LValueSeg, LifecycleKind, LocusMember,
    MatchArmBody, MatchStmt, ModeKind, OrDisposition, Program, RecoveryModifier, Stmt,
    TopDecl,
};
use hale_syntax::sites::{for_each_named_site, SiteKind};
use hale_types::ownership::{compute_assign_moved_bindings, compute_stack_array_bindings};

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/shadow_bindings.txt")
}

/// The three answers a `let` reads, as one comparable fact.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
struct Facts {
    returned: bool,
    assign_moved: bool,
    stack_array: bool,
}

impl fmt::Display for Facts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "returned={} assign_moved={} stack_array={}",
            self.returned, self.assign_moved, self.stack_array
        )
    }
}

/// Where a `let` stands, as the old join saw it.
struct Position<'p> {
    /// The LLVM function name the body lowers under, or `None` when it
    /// lowers under a name no map was keyed by.
    llvm: Option<String>,
    /// Which walks the old maps gave the body.
    walks: (bool, bool, bool),
    decl: String,
    name: &'p Ident,
}

/// Every `let` in a walked body, with the position the old join saw.
fn walked_lets(program: &Program) -> BTreeMap<u32, Position<'_>> {
    type Body<'p> = (Option<String>, (bool, bool, bool), String, &'p Block);
    let mut bodies: Vec<Body<'_>> = Vec::new();
    for item in hale_syntax::ast::flat_decls(&program.items) {
        match item {
            TopDecl::Fn(f) => {
                let llvm = if !f.generics.is_empty() || f.export {
                    None
                } else {
                    Some(f.name.name.clone())
                };
                bodies.push((llvm, (true, true, true), f.name.name.clone(), &f.body));
            }
            TopDecl::Locus(l) => {
                let generic = !l.generics.is_empty();
                for member in &l.members {
                    let (member_name, walks, body) = match member {
                        LocusMember::Fn(f) => (f.name.name.clone(), (false, true, true), &f.body),
                        LocusMember::Mode(md) => {
                            let m = match md.kind {
                                ModeKind::Bulk => "bulk",
                                ModeKind::Harmonic => "harmonic",
                                ModeKind::Resolution => "resolution",
                            };
                            (m.to_string(), (true, true, true), &md.body)
                        }
                        LocusMember::Lifecycle(lc) => {
                            let m = match lc.kind {
                                LifecycleKind::Birth => "birth",
                                LifecycleKind::Accept => "accept",
                                LifecycleKind::Release => "release",
                                LifecycleKind::Run => "run",
                                LifecycleKind::Drain => "drain",
                                LifecycleKind::Dissolve => "dissolve",
                            };
                            (m.to_string(), (false, false, true), &lc.body)
                        }
                        _ => continue,
                    };
                    let key = format!("{}.{}", l.name.name, member_name);
                    let llvm = (!generic).then(|| key.clone());
                    bodies.push((llvm, walks, key, body));
                }
            }
            _ => {}
        }
    }
    let mut out = BTreeMap::new();
    for (llvm, walks, decl, body) in bodies {
        let mut lets = Vec::new();
        body_lets(body, &mut lets);
        for (name, id) in lets {
            if !id.is_none() {
                out.insert(
                    id.0,
                    Position { llvm: llvm.clone(), walks, decl: decl.clone(), name },
                );
            }
        }
    }
    out
}

/// How many old-side rows said `true`, per fact: the vacuity check, so
/// a walk that answered `false` everywhere cannot pass by agreeing.
#[derive(Default)]
struct Trues {
    returned: usize,
    assign_moved: usize,
    stack_array: usize,
}

fn shadow_one(report: &mut Report, trues: &mut Trues, origin: &str, src: &str) {
    let Ok(program) = hale_syntax::parse_source(src) else { return };
    let Ok(resolved) = hale_types::resolved::resolve_program(&program, &[], None, None) else {
        return;
    };
    let merged = &resolved.merged;
    let moved = compute_assign_moved_bindings(merged);
    let stack = compute_stack_array_bindings(merged);
    let walked = walked_lets(merged);

    // OLD: every `let` site of the program, answered by the join.
    let mut old: Vec<(u32, Facts)> = Vec::new();
    let mut describe: BTreeMap<u32, String> = BTreeMap::new();
    for_each_named_site(merged, &mut |kind, _, name, id| {
        if kind != SiteKind::Let || id.is_none() {
            return;
        }
        let facts = match walked.get(&id.0) {
            Some(p) => {
                let key = p.llvm.as_deref();
                let returned = p.walks.0
                    && key
                        .and_then(|k| resolved.returned_bindings.get(k))
                        .map(|rb| rb.let_is_returned(p.name))
                        .unwrap_or(false);
                let assign_moved = p.walks.1
                    && key
                        .and_then(|k| moved.get(k))
                        .map(|s| s.contains(&p.name.name))
                        .unwrap_or(false);
                let stack_array = p.walks.2
                    && key
                        .and_then(|k| stack.get(k))
                        .map(|s| s.contains(&p.name.name))
                        .unwrap_or(false);
                describe.insert(
                    id.0,
                    format!(
                        "`let {}` in `{}` (lowered as {})",
                        p.name.name,
                        p.decl,
                        key.map(|k| format!("`{k}`")).unwrap_or_else(|| "another name".into())
                    ),
                );
                Facts { returned, assign_moved, stack_array }
            }
            None => {
                describe.insert(
                    id.0,
                    format!("`let {}` in a body no map walked", name.unwrap_or("?")),
                );
                Facts::default()
            }
        };
        trues.returned += usize::from(facts.returned);
        trues.assign_moved += usize::from(facts.assign_moved);
        trues.stack_array += usize::from(facts.stack_array);
        old.push((id.0, facts));
    });

    // NEW: the rows.
    let new: Vec<(u32, Facts)> = resolved
        .owner_table
        .binding_rows()
        .map(|(id, f)| {
            (
                *id,
                Facts {
                    returned: f.returned,
                    assign_moved: f.assign_moved,
                    stack_array: f.stack_array,
                },
            )
        })
        .collect();

    let id = program_id(origin, src);
    report.compare_rows(
        &id,
        &old,
        &new,
        |k| Some(*k),
        |k| Some(*k),
        |k| vec![describe.get(k).cloned().unwrap_or_else(|| format!("site #{k}"))],
        |_| {
            vec![
                "codegen `Stmt::Let`: caller-arena routing for a handed-back binding in a method with scratch".into(),
                "codegen `Stmt::Let`: the GH #383 fresh-factory dissolve (suppressed for a handed-back or `=`-moved binding)".into(),
                "codegen `Stmt::Let`: the GH #767 frame-local `[c; N]`".into(),
            ]
        },
    );
}

#[test]
fn the_binding_maps_and_the_binding_rows_agree_or_every_divergence_is_classified() {
    let mut report = Report::new("ownership (binding facts)");
    let mut trues = Trues::default();
    for p in hale_corpus::parseable(|s| hale_syntax::parse_source(s).is_ok()) {
        shadow_one(&mut report, &mut trues, &p.origin, &p.source);
    }
    assert!(report.programs > 300, "the corpus walk is vacuous ({} programs)", report.programs);
    assert!(report.rows_compared > 1000, "the corpus walk is vacuous ({} rows)", report.rows_compared);
    assert!(
        trues.returned > 100 && trues.assign_moved > 100 && trues.stack_array > 10,
        "the old side answered `true` too rarely to compare anything \
         (returned {}, assign_moved {}, stack_array {})",
        trues.returned,
        trues.assign_moved,
        trues.stack_array
    );
    let path = fixture_path();
    let existing = std::fs::read_to_string(&path)
        .ok()
        .map(|t| parse_fixture(&t).unwrap_or_else(|e| panic!("{}: {e}", path.display())))
        .unwrap_or_default();
    if std::env::var("HALE_SHADOW_REGEN").as_deref() == Ok("1") {
        std::fs::write(&path, report.render_fixture(&existing)).expect("write fixture");
        eprintln!("{}", report.render());
        return;
    }
    let (unexplained, stale) = report.explain(&existing);
    assert!(
        unexplained.is_empty() && stale.is_empty(),
        "{}",
        gate_message(&report, &unexplained, &stale, "crates/hale-types/tests/fixtures/shadow_bindings.txt")
    );
    eprintln!(
        "shadow `{}`: {} programs, {} rows compared, {} divergence(s); old-side trues: \
         returned {}, assign_moved {}, stack_array {}",
        report.family,
        report.programs,
        report.rows_compared,
        report.divergences.len(),
        trues.returned,
        trues.assign_moved,
        trues.stack_array
    );
}

// -------------------------------------------------------------------
// Every `let` of a body, wherever it stands (exhaustive, so a new
// statement or expression form cannot hide one from the old side).
// -------------------------------------------------------------------

type Lets<'e> = Vec<(&'e Ident, hale_syntax::ast::NodeId)>;

fn body_lets<'e>(b: &'e Block, out: &mut Lets<'e>) {
    for s in &b.stmts {
        stmt(s, out);
    }
    if let Some(t) = b.tail.as_deref() {
        expr(t, out);
    }
}

fn if_chain<'e>(i: &'e IfStmt, out: &mut Lets<'e>) {
    expr(&i.cond, out);
    body_lets(&i.then_block, out);
    match i.else_block.as_deref() {
        Some(ElseBranch::Else(b)) => body_lets(b, out),
        Some(ElseBranch::ElseIf(n)) => if_chain(n, out),
        None => {}
    }
}

fn match_arms<'e>(m: &'e MatchStmt, out: &mut Lets<'e>) {
    expr(&m.scrutinee, out);
    for a in &m.arms {
        if let Some(g) = &a.guard {
            expr(g, out);
        }
        match &a.body {
            MatchArmBody::Block(b) => body_lets(b, out),
            MatchArmBody::Expr(x) => expr(x, out),
        }
    }
}

fn disposition<'e>(d: &'e OrDisposition, out: &mut Lets<'e>) {
    match d {
        OrDisposition::Substitute(e) | OrDisposition::Fail(e, _) => expr(e, out),
        OrDisposition::Raise(_) | OrDisposition::Discard(_) | OrDisposition::Wait(_) => {}
    }
}

fn stmt<'e>(s: &'e Stmt, out: &mut Lets<'e>) {
    match s {
        Stmt::Let { name, value, id, .. } => {
            expr(value, out);
            out.push((name, *id));
        }
        Stmt::LetTuple { value, .. } => expr(value, out),
        Stmt::Assign { target, value, .. } => {
            for seg in &target.tail {
                match seg {
                    LValueSeg::Index(ix) => expr(ix, out),
                    LValueSeg::Field(_) => {}
                }
            }
            expr(value, out);
        }
        Stmt::Return(value, _) => {
            if let Some(e) = value {
                expr(e, out);
            }
        }
        Stmt::If(i) => if_chain(i, out),
        Stmt::Match(m) => match_arms(m, out),
        Stmt::For { iter, body, .. } => {
            expr(iter, out);
            body_lets(body, out);
        }
        Stmt::ShmWrite { max, body, .. } => {
            expr(max, out);
            body_lets(body, out);
        }
        Stmt::While { cond, body, .. } => {
            expr(cond, out);
            body_lets(body, out);
        }
        Stmt::Block(body) => body_lets(body, out),
        Stmt::Fail { value, .. } => expr(value, out),
        Stmt::Recovery { args, modifier, .. } => {
            for a in args {
                expr(a, out);
            }
            if let Some(RecoveryModifier::For(e) | RecoveryModifier::Until(e)) = modifier {
                expr(e, out);
            }
        }
        Stmt::Violate { payload, .. } => {
            if let Some(p) = payload {
                expr(p, out);
            }
        }
        Stmt::Send { subject, value, or_disposition, .. } => {
            expr(subject, out);
            expr(value, out);
            if let Some(d) = or_disposition {
                disposition(d, out);
            }
        }
        Stmt::Expr(e) => expr(e, out),
        Stmt::Break(_)
        | Stmt::Continue(_)
        | Stmt::Yield(_)
        | Stmt::Terminate(_)
        | Stmt::Reperspective { .. } => {}
    }
}

fn expr<'e>(e: &'e Expr, out: &mut Lets<'e>) {
    match e {
        Expr::Ident(_) | Expr::Literal(..) | Expr::Path(_) | Expr::KwSelf(_) => {}
        Expr::Binary { left, right, .. } => {
            expr(left, out);
            expr(right, out);
        }
        Expr::Unary { operand, .. } => expr(operand, out),
        Expr::Call { callee, args, .. } => {
            expr(callee, out);
            for a in args {
                expr(a, out);
            }
        }
        Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. } => expr(receiver, out),
        Expr::Index { receiver, index, .. } => {
            expr(receiver, out);
            expr(index, out);
        }
        Expr::Tuple(v, _) | Expr::Array(v, _) => {
            for x in v {
                expr(x, out);
            }
        }
        Expr::Struct { inits, .. } => {
            for i in inits {
                expr(&i.value, out);
            }
        }
        Expr::Block(b) => body_lets(b, out),
        Expr::If(i) => if_chain(i, out),
        Expr::Match(m) => match_arms(m, out),
        Expr::Sum(x, _) | Expr::Prod(x, _) => expr(x, out),
        Expr::Approx { left, right, tolerance, .. } => {
            expr(left, out);
            expr(right, out);
            expr(tolerance, out);
        }
        Expr::Range { lo, hi, .. } => {
            expr(lo, out);
            expr(hi, out);
        }
        Expr::ArrayRepeat { val, .. } => expr(val, out),
        Expr::Or { inner, disposition: d, .. } => {
            expr(inner, out);
            disposition(d, out);
        }
    }
}

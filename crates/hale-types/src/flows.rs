//! GH #736: which `release` clause makes a locus type a flow.
//!
//! A child type is a *flow* — its `run()` completing reclaims it — when
//! any locus anywhere in the program declares `release(c: T)`, whether or
//! not that locus is ever instantiated; otherwise it is a *resident*,
//! reclaimed only when its owner dissolves (spec/semantics.md, "Flow and
//! resident children"). The rule is type-wide on purpose, and it can
//! surprise: removing the release hook from one owner leaves a child
//! reclaimed at `run()`'s end because another declaration, perhaps in an
//! imported seed, still names its type.
//!
//! `hale check --flows` answers "why is this a flow" by naming every
//! clause responsible. It reads the declarations codegen classifies by —
//! a release hook's first parameter — and keys the type as written: a
//! one-segment path by its name (an imported type is one mangled segment
//! by then), a longer one (a stdlib path, `std::io::tcp::Stream`) by the
//! whole path, never by its last segment, which would name a local
//! `Stream` that is no flow.
//!
//! Each row also carries the locus the type denotes as lowering names it
//! ([`Flow::locus`], resolved once by `handler_routing::child_locus_name`:
//! through aliases, generic instantiations and qualified paths), and
//! lowering reads flow-ness from there: a locus is a flow when a row
//! names it.
//!
//! A clause whose type mentions its owner's type parameters
//! (`locus Manager<T> { release(c: T) { } }`) names no locus by itself:
//! which one it names is decided by each specialization lowering
//! creates. Such a clause carries its template ([`FlowClause::template`]:
//! the owner's identity, its parameters in order, the type as written),
//! and [`FlowRows::specialize`] answers for a specialization by applying
//! the consumer's own substitution to it and resolving the result the
//! way a concrete clause's type is resolved. The flow facts then cover
//! the concrete loci lowering creates, not only the ones written out
//! (outside review of #1295, finding 1).
//!
//! Beside the clauses, every locus declaration has a run row
//! ([`RunRow`]): whether its `run()` is *long-running* (a body of its
//! own) and whether it *never returns*. The checker asks both, of
//! different rules, so they are two columns, never one predicate
//! (F.40 phase 3, E2).

use std::collections::BTreeMap;

use hale_syntax::ast::{
    Block, ElseBranch, Expr, IfStmt, LValueSeg, LifecycleKind, Literal, LocusDecl, LocusMember,
    MatchArmBody, NodeId, ParamInit, Program, Stmt, TopDecl, TypeExpr, UnaryOp,
};
use hale_syntax::Span;

use crate::handler_routing::{child_locus_name, ChildRef, DeclaredNames};

/// One `release(c: T)` clause: the locus declaring it, the parameter's
/// name, where it is, and the locus its type denotes as lowering names
/// it (`None` when it denotes no declared locus, or when it is a
/// template's clause). Two instantiations of one generic share the
/// written key and resolve apart.
pub struct FlowClause {
    pub owner: String,
    pub param: String,
    pub span: Span,
    pub locus: Option<String>,
    /// Set when the clause's type mentions one of its owner's type
    /// parameters: the locus it names is the specialization's to decide.
    pub template: Option<TemplateClause>,
}

/// A generic owner's clause, before substitution.
pub struct TemplateClause {
    /// The owner's identity: a specialization is a clone of its
    /// template's declaration and keeps it.
    pub owner_id: NodeId,
    /// The owner's type parameters, in declaration order: a
    /// specialization's arguments are positional.
    pub generics: Vec<String>,
    /// The parameter's type as written (`T`, `Box<T>`).
    pub ty: TypeExpr,
}

/// A flow type and every clause that makes it one, in source order.
pub struct Flow {
    /// The child type as written (its path joined by `::`).
    pub child: String,
    pub clauses: Vec<FlowClause>,
}

/// A locus's `run()`, as the two questions the checker asks of it
/// (F.40 phase 3, E2). They are different questions, asked by
/// different rules, so each is a column of its own
/// (spec/runtime.md § Typecheck enforcement):
///
/// - **long-running**: the `run()` body has a statement of its own. A
///   nested cooperative child runs its `run()` to completion before its
///   parent's begins, so any body delays the parent whether or not it
///   ever returns: the nested-long-running-child rule asks this.
/// - **never returns**: the `run()` body provably never returns (its
///   last statement is a `while` with no exit whose condition never
///   flips false). A cooperative pool runs each `run()` cell to
///   completion, so only such a body starves the cells after it: the
///   starvation and birth-order laws ask this.
///
/// A body that never returns is long-running; the converse does not
/// hold (`run() { std::time::sleep(1m); }` is long-running and returns).
pub struct RunRow {
    /// The locus as declared, and where: a declaration is found by its
    /// name and span ([`FlowRows::run_of`]).
    pub locus: String,
    pub span: Span,
    pub long_running: bool,
    /// The terminal `while`, when the body never returns.
    pub never_returns: Option<Span>,
}

/// The `flows` family's rows, with what a template clause is resolved
/// against once a specialization substitutes it: the declared loci and
/// aliases, and the bundle's import renames. Derefs to the rows.
pub struct FlowRows {
    flows: Vec<Flow>,
    /// One run row per locus declaration, a module's included, in
    /// declaration order.
    runs: Vec<RunRow>,
    declared: DeclaredNames,
    renames: Vec<(Vec<String>, String)>,
}

impl std::ops::Deref for FlowRows {
    type Target = [Flow];
    fn deref(&self) -> &[Flow] {
        &self.flows
    }
}

impl FlowRows {
    /// The run row of a declaration the rows were surveyed over.
    pub fn run_of(&self, decl: &LocusDecl) -> Option<&RunRow> {
        self.runs.iter().find(|r| r.locus == decl.name.name && r.span == decl.span)
    }

    /// The loci a specialization of `template` makes flows: each of the
    /// template's clauses, its type passed through `substitute` (the
    /// consumer's substitution of the template's parameters by the
    /// specialization's arguments) and resolved the way a concrete
    /// clause's type is. The template is matched by its identity, or by
    /// its name and span when the program was never minted.
    pub fn specialize(&self, template: &LocusDecl, substitute: impl Fn(&TypeExpr) -> TypeExpr) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for c in self.flows.iter().flat_map(|f| &f.clauses) {
            let Some(t) = &c.template else { continue };
            let same = if t.owner_id.is_none() || template.id.is_none() {
                c.owner == template.name.name
                    && template.span.start <= c.span.start
                    && c.span.end <= template.span.end
            } else {
                t.owner_id.0 == template.id.0
            };
            if !same {
                continue;
            }
            if let Some(name) = self.locus(&substitute(&t.ty)) {
                if !out.contains(&name) {
                    out.push(name);
                }
            }
        }
        out
    }

    /// The locus `ty` denotes as lowering names it, if it denotes one.
    fn locus(&self, ty: &TypeExpr) -> Option<String> {
        match child_locus_name(ty, &self.declared, &self.renames) {
            ChildRef::Locus(name) => Some(name),
            ChildRef::External(_) => None,
        }
    }
}

/// Whether a row makes `locus` (a locus as lowering names it) a flow.
/// A template's clause names no locus here; a consumer that creates
/// specializations asks [`FlowRows::specialize`] for theirs.
pub fn is_flow(flows: &[Flow], locus: &str) -> bool {
    flows.iter().flat_map(|f| &f.clauses).any(|c| c.locus.as_deref() == Some(locus))
}

/// Whether `ty` mentions one of `generics`: the parameter itself, or a
/// generic argument that does (`Box<T>`).
fn mentions_param(ty: &TypeExpr, generics: &[String]) -> bool {
    let TypeExpr::Named { path, generic_args, .. } = ty else { return false };
    (path.segments.len() == 1 && generic_args.is_empty() && generics.contains(&path.segments[0].name))
        || generic_args.iter().any(|a| mentions_param(a, generics))
}

fn walk(
    items: &[TopDecl],
    rows: &FlowRows,
    out: &mut BTreeMap<String, Vec<FlowClause>>,
    runs: &mut Vec<RunRow>,
) {
    for item in items {
        match item {
            TopDecl::Locus(l) => {
                let run = l.members.iter().find_map(|m| match m {
                    LocusMember::Lifecycle(lc) if lc.kind == LifecycleKind::Run => Some(&lc.body),
                    _ => None,
                });
                runs.push(RunRow {
                    locus: l.name.name.clone(),
                    span: l.span,
                    long_running: run.is_some_and(|b| !b.stmts.is_empty()),
                    never_returns: run.and_then(|b| run_statically_nonreturning(b, l)),
                });
                let generics: Vec<String> = l.generics.iter().map(|g| g.name.name.clone()).collect();
                for member in &l.members {
                    let LocusMember::Lifecycle(lc) = member else { continue };
                    if lc.kind != LifecycleKind::Release {
                        continue;
                    }
                    let Some(p) = lc.params.first() else { continue };
                    let TypeExpr::Named { path, .. } = &p.ty else { continue };
                    if path.segments.is_empty() {
                        continue;
                    }
                    let child = path.segments.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join("::");
                    // A parameter shadows a declared name: `T` in
                    // `Manager<T>` is the argument, never a locus `T`.
                    let template = mentions_param(&p.ty, &generics).then(|| TemplateClause {
                        owner_id: l.id,
                        generics: generics.clone(),
                        ty: p.ty.clone(),
                    });
                    let locus = match template {
                        Some(_) => None,
                        None => rows.locus(&p.ty),
                    };
                    out.entry(child).or_default().push(FlowClause {
                        owner: l.name.name.clone(),
                        param: p.name.name.clone(),
                        span: lc.span,
                        locus,
                        template,
                    });
                }
            }
            TopDecl::Module(m) => walk(&m.items, rows, out, runs),
            _ => {}
        }
    }
}

/// Every flow type in the bundle, by name as written, with the clauses
/// that make it one ordered by where they sit, each with the locus its
/// type denotes under the bundle's `import_renames` (or, for a generic
/// owner's clause, the template a specialization resolves).
pub fn survey(programs: &[&Program], import_renames: &[(Vec<String>, String)]) -> FlowRows {
    let mut rows = FlowRows {
        flows: Vec::new(),
        runs: Vec::new(),
        declared: DeclaredNames::of(programs),
        renames: import_renames.to_vec(),
    };
    let mut by_child: BTreeMap<String, Vec<FlowClause>> = BTreeMap::new();
    let mut runs: Vec<RunRow> = Vec::new();
    for p in programs {
        walk(&p.items, &rows, &mut by_child, &mut runs);
    }
    rows.runs = runs;
    rows.flows = by_child
        .into_iter()
        .map(|(child, mut clauses)| {
            clauses.sort_by_key(|c| c.span.start.as_usize());
            Flow { child, clauses }
        })
        .collect();
    rows
}

// === statically non-returning run(): the never-returns column =====
//
// Moved here from the checker (F.40 phase 3, E2) so the run row holds
// it beside the long-running column; the starvation and birth-order
// laws read the column. A cooperative pool runs each posted `run()`
// cell to completion, so
// two loci on one pool whose `run()` bodies never return means the
// second never starts — silently (bus handlers still fire at
// sleep/yield drains, which makes the hang look like a healthy idle).
// The predicate below is deliberately conservative (same style as
// `while_counter_bounded` in alloc_summary.rs): it only claims
// "statically never returns" for shapes it can prove, so the
// starvation warning never false-fires on a loop that can exit.

/// Does this block contain a statement that can exit the enclosing
/// `run()` loop — `break`, `return`, `terminate`, `fail`, or
/// `violate`? Walked recursively through nested statement bodies but
/// NOT into expressions (a `return` inside a failure-closure exits
/// the closure, not `run()`). A `break` in a *nested* loop only exits
/// that loop, but counting it as an exit here is the conservative
/// direction (a missed warning, never a false one).
fn block_has_loop_exit(block: &Block) -> bool {
    block.stmts.iter().any(stmt_has_loop_exit)
}

fn stmt_has_loop_exit(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Break(_)
        | Stmt::Return(..)
        | Stmt::Terminate(_)
        | Stmt::Fail { .. }
        | Stmt::Violate { .. } => true,
        Stmt::If(if_stmt) => if_has_loop_exit(if_stmt),
        Stmt::Match(m) => m.arms.iter().any(|arm| match &arm.body {
            MatchArmBody::Block(b) => block_has_loop_exit(b),
            MatchArmBody::Expr(_) => false,
        }),
        Stmt::For { body, .. } | Stmt::While { body, .. } => block_has_loop_exit(body),
        Stmt::Block(b) => block_has_loop_exit(b),
        Stmt::ShmWrite { body, .. } => block_has_loop_exit(body),
        _ => false,
    }
}

fn if_has_loop_exit(if_stmt: &IfStmt) -> bool {
    if block_has_loop_exit(&if_stmt.then_block) {
        return true;
    }
    match if_stmt.else_block.as_deref() {
        Some(ElseBranch::Else(b)) => block_has_loop_exit(b),
        Some(ElseBranch::ElseIf(inner)) => if_has_loop_exit(inner),
        None => false,
    }
}

/// `Some(field_name)` iff the expression is a bare `self.<field>` read.
fn self_bool_field(e: &Expr) -> Option<&str> {
    match e {
        Expr::Field { receiver, name, .. }
            if matches!(receiver.as_ref(), Expr::KwSelf(_)) =>
        {
            Some(name.name.as_str())
        }
        _ => None,
    }
}

/// Is `self.<field> = ...` (or a compound assign to it) present in any
/// member body of the locus? Walked through nested statement bodies.
fn locus_assigns_self_field(decl: &LocusDecl, field: &str) -> bool {
    fn block_assigns(block: &Block, field: &str) -> bool {
        block.stmts.iter().any(|s| stmt_assigns(s, field))
    }
    fn if_assigns(if_stmt: &IfStmt, field: &str) -> bool {
        block_assigns(&if_stmt.then_block, field)
            || match if_stmt.else_block.as_deref() {
                Some(ElseBranch::Else(b)) => block_assigns(b, field),
                Some(ElseBranch::ElseIf(inner)) => if_assigns(inner, field),
                None => false,
            }
    }
    fn stmt_assigns(stmt: &Stmt, field: &str) -> bool {
        match stmt {
            Stmt::Assign { target, .. } => {
                target.head.name == "self"
                    && matches!(
                        target.tail.first(),
                        Some(LValueSeg::Field(f)) if f.name == field
                    )
            }
            Stmt::If(if_stmt) => if_assigns(if_stmt, field),
            Stmt::Match(m) => m.arms.iter().any(|arm| match &arm.body {
                MatchArmBody::Block(b) => block_assigns(b, field),
                MatchArmBody::Expr(_) => false,
            }),
            Stmt::For { body, .. } | Stmt::While { body, .. } => {
                block_assigns(body, field)
            }
            Stmt::Block(b) => block_assigns(b, field),
            Stmt::ShmWrite { body, .. } => block_assigns(body, field),
            _ => false,
        }
    }
    decl.members.iter().any(|m| match m {
        LocusMember::Fn(f) => block_assigns(&f.body, field),
        LocusMember::Lifecycle(l) => block_assigns(&l.body, field),
        LocusMember::Mode(md) => block_assigns(&md.body, field),
        LocusMember::Failure(fd) => block_assigns(&fd.body, field),
        _ => false,
    })
}

/// The literal Bool default of a params field, if it has one.
fn param_bool_default(decl: &LocusDecl, field: &str) -> Option<bool> {
    decl.members.iter().find_map(|m| {
        let LocusMember::Params(pb) = m else { return None };
        pb.params.iter().find_map(|p| {
            if p.name.name != field {
                return None;
            }
            match &p.init {
                ParamInit::Value(Expr::Literal(Literal::Bool(b), _)) => Some(*b),
                _ => None,
            }
        })
    })
}

/// `Some(span of the terminal while)` iff this `run()` body statically
/// never returns: its last statement is a `while` whose body contains
/// no exit statement and whose condition provably never flips false —
///   - `while true`,
///   - `while !self.draining` (the synthetic drain flag flips only at
///     shutdown, so for the pool's purposes the loop runs forever),
///   - `while !self.f` / `while self.f` where `f` is a Bool params
///     field that no member body ever assigns and whose declared
///     default keeps the loop live (`false` / `true` respectively).
fn run_statically_nonreturning(run_body: &Block, decl: &LocusDecl) -> Option<Span> {
    let Some(Stmt::While { cond, body, span }) = run_body.stmts.last() else {
        return None;
    };
    if block_has_loop_exit(body) {
        return None;
    }
    let never_flips = match cond {
        Expr::Literal(Literal::Bool(true), _) => true,
        Expr::Unary { op: UnaryOp::Not, operand, .. } => {
            match self_bool_field(operand) {
                Some("draining") => true,
                Some(f) => {
                    !locus_assigns_self_field(decl, f)
                        && param_bool_default(decl, f) == Some(false)
                }
                None => false,
            }
        }
        _ => match self_bool_field(cond) {
            Some(f) if f != "draining" => {
                !locus_assigns_self_field(decl, f)
                    && param_bool_default(decl, f) == Some(true)
            }
            _ => false,
        },
    };
    if never_flips {
        Some(*span)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// "Long-running" and "never returns" are two columns: a `run()`
    /// that sleeps once is long-running and returns; a terminal
    /// `while true` is both; an empty `run()` and no `run()` are neither.
    /// A module's loci have rows too.
    #[test]
    fn long_running_and_never_returns_are_two_columns() {
        let src = "locus Sleeper { run() { std::time::sleep(1m); } }\n\
                   locus Daemon { run() { while true { std::time::sleep(1s); } } }\n\
                   locus Idle { run() { } }\n\
                   locus Plain { params { n: Int = 0; } }\n\
                   module inner { locus Nested { run() { while true { } } } }\n\
                   fn main() { }\n";
        let p = hale_syntax::parse_source(src).expect("parse");
        let rows = survey(&[&p], &[]);
        let col = |name: &str| {
            let r = rows.runs.iter().find(|r| r.locus == name).expect("a run row");
            (r.long_running, r.never_returns.is_some())
        };
        assert_eq!(col("Sleeper"), (true, false));
        assert_eq!(col("Daemon"), (true, true));
        assert_eq!(col("Idle"), (false, false));
        assert_eq!(col("Plain"), (false, false));
        assert_eq!(col("Nested"), (true, true));
    }

    /// A clause's child is the locus lowering names: an alias is
    /// followed to its target, so two spellings of one child are two
    /// written keys over one flow; a type that is no locus is no flow.
    #[test]
    fn a_clause_names_the_locus_lowering_names() {
        let src = "locus Worker { params { id: Int = 0; } }\n\
                   type Job = Worker;\n\
                   locus A { accept(c: Worker) { } release(c: Worker) { } }\n\
                   locus B { accept(c: Job) { } release(c: Job) { } }\n\
                   locus C { release(c: Nothing) { } }\n\
                   fn main() { }\n";
        let p = hale_syntax::parse_source(src).expect("parse");
        let flows = survey(&[&p], &[]);
        let keys: Vec<(&str, Vec<Option<&str>>)> = flows
            .iter()
            .map(|f| (f.child.as_str(), f.clauses.iter().map(|c| c.locus.as_deref()).collect()))
            .collect();
        assert_eq!(
            keys,
            vec![("Job", vec![Some("Worker")]), ("Nothing", vec![None]), ("Worker", vec![Some("Worker")])]
        );
        assert!(is_flow(&flows, "Worker"));
        assert!(!is_flow(&flows, "Job") && !is_flow(&flows, "Nothing") && !is_flow(&flows, "A"));
    }

    /// A clause over its owner's type parameter is the template's: it
    /// names no locus (not even a declared locus spelled like the
    /// parameter), and a specialization names one through the
    /// substitution it is handed, the parameter's position deciding
    /// which argument; a parameter nested in a generic child substitutes
    /// inside it.
    #[test]
    fn a_template_clause_names_the_specialization_s_argument() {
        let src = "locus Worker { }\nlocus T { }\ntype Job = Worker;\n\
                   locus Cell<U> { }\n\
                   locus Manager<K, T> { accept(c: T) { } release(c: T) { } }\n\
                   locus Boxer<T> { release(c: Cell<T>) { } }\n\
                   fn main() { }\n";
        let p = hale_syntax::parse_source(src).expect("parse");
        let rows = survey(&[&p], &[]);
        assert!(!is_flow(&rows, "T") && !is_flow(&rows, "Worker"), "a template clause names no locus");
        let decl = |name: &str| {
            p.items
                .iter()
                .find_map(|i| match i {
                    TopDecl::Locus(l) if l.name.name == name => Some(l.clone()),
                    _ => None,
                })
                .unwrap()
        };
        let named = |s: &str| TypeExpr::Named {
            path: hale_syntax::ast::QualifiedName {
                segments: vec![hale_syntax::ast::Ident::new(s, Span::new(0, 0))],
                span: Span::new(0, 0),
            },
            generic_args: Vec::new(),
            span: Span::new(0, 0),
        };
        // the substitution a consumer applies: the template's parameters
        // by position
        fn subst(ty: &TypeExpr, by: &BTreeMap<String, TypeExpr>) -> TypeExpr {
            match ty {
                TypeExpr::Named { path, generic_args, .. }
                    if path.segments.len() == 1 && generic_args.is_empty() && by.contains_key(&path.segments[0].name) =>
                {
                    by[&path.segments[0].name].clone()
                }
                TypeExpr::Named { path, generic_args, span } => TypeExpr::Named {
                    path: path.clone(),
                    generic_args: generic_args.iter().map(|a| subst(a, by)).collect(),
                    span: *span,
                },
                other => other.clone(),
            }
        }
        let manager = decl("Manager");
        let by: BTreeMap<String, TypeExpr> =
            [("K".to_string(), named("Int")), ("T".to_string(), named("Job"))].into_iter().collect();
        assert_eq!(rows.specialize(&manager, |t| subst(t, &by)), vec!["Worker".to_string()]);
        let boxer = decl("Boxer");
        let by: BTreeMap<String, TypeExpr> = [("T".to_string(), named("Int"))].into_iter().collect();
        let cell_int = crate::mangle::mangle_generic_name("Cell", &[named("Int")]).unwrap();
        assert_eq!(rows.specialize(&boxer, |t| subst(t, &by)), vec![cell_int]);
        // a specialization of another template answers nothing
        assert!(rows.specialize(&decl("Cell"), |t| t.clone()).is_empty());
    }
}

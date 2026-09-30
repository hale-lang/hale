//! The handler-routing shadow (F.40 phase 1.4, step 1 of 4): the
//! child-type resolver of the `handler_routing` rows, run beside the
//! three ways the child of an `on_failure` handler was named before,
//! over the whole corpus.
//!
//! One key per handler, `(program, parent#ordinal)`, and one facet per
//! old naming, each computed here the way its site computes it today:
//!
//! - `checker`: `resolve_type_expr(&fd.params[0].ty, known).display()`
//!   with the checker's known-names table (rebuilt from the top scope
//!   as `check::collect_known_names` builds it; that function is
//!   private). The checker's duplicate rule skips an `Unknown` child;
//!   the law over the rows skips an `External` one, so the
//!   correspondence renders both as `external`. New: the rows over the
//!   checked program, as the checker will read them.
//! - `model`: the model's `te_name` (the written path joined by `::`),
//!   a locus when the model has a locus of that name, else external.
//!   New: the rows over the checked program, a row naming a locus the
//!   model has no id for by its written spelling (a monomorph's is its
//!   template; a stdlib locus's is its path).
//! - `lowering`: codegen's naming lives inside `Cx` and is not
//!   reachable from a test, so the rows lowering reads (built over the
//!   resolved program's merged, stdlib-bearing program) are held to
//!   the program they came from: a `Locus` row must name a locus the
//!   resolved program declares, an `External` row must not.
//! - `in_place`: codegen's restart-in-place attribution, a search of
//!   the handler body's Debug string for `RestartInPlace`, against the
//!   rows' recovery ops.
//! - `ops`: the model's former recovery-op walk (statements, `if`,
//!   loops and blocks only), against the rows' walk.
//!
//! Only programs that check clean are shadowed. Each program is parsed
//! alone (no imports), so a cross-seed path never resolves here.
//!
//! The gate: every divergence is classified in
//! `fixtures/shadow_handler_routing.txt`. Regenerate with
//! `HALE_SHADOW_REGEN=1`, then classify by hand.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use hale_graph::shadow::{gate_message, parse_fixture, program_id, Report};
use hale_syntax::ast::{
    flat_decls, Block, ElseBranch, Expr, Literal, LocusMember, RecoveryModifier,
    Stmt, TopDecl, TypeExpr,
};
use hale_types::handler_routing::{handler_rows, op_name, ChildRef, HandlerRouting};
use hale_types::resolve::{resolve_type_expr, KnownNames, TopScope};
use hale_types::symbol::{SourceFile, TopSymbol, TypeKind};
use hale_types::ty::Ty;
use hale_types::Bundle;

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/shadow_handler_routing.txt")
}

/// `check::collect_known_names`, as it builds the table.
fn known_names(top: &TopScope, renames: &[(Vec<String>, String)]) -> KnownNames {
    let mut m = KnownNames::default();
    m.set_imports(renames);
    for (name, sym) in &top.symbols {
        if matches!(
            sym,
            TopSymbol::Locus(_) | TopSymbol::Type(_) | TopSymbol::Perspective(_)
        ) {
            m.insert(name.clone(), sym.span());
        }
        if let TopSymbol::Type(info) = sym {
            if let TypeKind::Alias(t) = &info.kind {
                m.set_alias(name.clone(), t.clone());
            }
        }
    }
    m
}

/// The model's `te_name`.
fn te_name(t: &TypeExpr) -> String {
    match t {
        TypeExpr::Named { path, .. } => path
            .segments
            .iter()
            .map(|s| s.name.clone())
            .collect::<Vec<_>>()
            .join("::"),
        _ => "?".to_string(),
    }
}

/// The model's former `walk_ops`, as it was.
fn old_walk_ops(b: &Block, ops: &mut Vec<String>, retry: &mut Option<i64>) {
    for st in &b.stmts {
        match st {
            Stmt::Recovery { op, modifier, .. } => {
                let n = op_name(*op);
                if !ops.iter().any(|o| o == n) {
                    ops.push(n.to_string());
                }
                if let Some(RecoveryModifier::For(Expr::Literal(Literal::Int(kk), _))) = modifier {
                    *retry = Some(*kk);
                }
            }
            Stmt::If(i) => {
                old_walk_ops(&i.then_block, ops, retry);
                let mut cur = i.else_block.as_deref();
                while let Some(eb) = cur {
                    match eb {
                        ElseBranch::Else(bb) => {
                            old_walk_ops(bb, ops, retry);
                            cur = None;
                        }
                        ElseBranch::ElseIf(ei) => {
                            old_walk_ops(&ei.then_block, ops, retry);
                            cur = ei.else_block.as_deref();
                        }
                    }
                }
            }
            Stmt::While { body, .. } | Stmt::For { body, .. } => old_walk_ops(body, ops, retry),
            Stmt::Block(bb) => old_walk_ops(bb, ops, retry),
            _ => {}
        }
    }
}

fn ops_value(ops: &[String], retry: Option<i64>) -> String {
    let o = if ops.is_empty() { "none".to_string() } else { ops.join(",") };
    match retry {
        Some(n) => format!("{o};retry={n}"),
        None => o,
    }
}

fn row_of<'a>(
    routing: &'a HandlerRouting,
    parent: &str,
    ordinal: u32,
) -> Option<&'a hale_types::handler_routing::HandlerRow> {
    routing.handlers_of(parent).find(|r| r.ordinal == ordinal)
}

/// Shadow one program; false when it is not shadowed (it does not
/// parse, or does not check clean).
fn shadow_one(
    report: &mut Report,
    origin: &str,
    src: &str,
    renames: &[(Vec<String>, String)],
) -> bool {
    let Ok(program) = hale_syntax::parse_source(src) else { return false };
    // The program the checker and the model read.
    let mut programs = BTreeMap::new();
    programs.insert("app.hl".to_string(), &program);
    let mut bundle = Bundle::new(programs);
    bundle.import_renames = renames.to_vec();
    let diags = if renames.is_empty() {
        hale_types::check_program(&program)
    } else {
        hale_types::check_bundle_opts_whole_program(&bundle, false)
    };
    if diags.iter().any(|d| d.is_error()) {
        return false;
    }
    bundle.sources = vec![SourceFile {
        id: 0,
        path: "app.hl".to_string(),
        digest: "0".to_string(),
        base: 0,
        len: src.len() as u32,
    }];
    let (top, _diags) = hale_types::resolve::build_top_scope(&bundle);
    let known = known_names(&top, renames);
    let user_loci: BTreeSet<String> = flat_decls(&program.items)
        .filter_map(|i| match i {
            TopDecl::Locus(l) => Some(l.name.name.clone()),
            _ => None,
        })
        .collect();
    let checked_rows = handler_rows(&program, renames);
    // The program lowering reads.
    let Ok(resolved) = hale_types::resolved::resolve_program(&program, renames, None, None) else {
        return false;
    };
    let merged_loci: BTreeSet<String> = flat_decls(&resolved.merged.items)
        .filter_map(|i| match i {
            TopDecl::Locus(l) => Some(l.name.name.clone()),
            _ => None,
        })
        .collect();

    let mut old: Vec<(String, String)> = Vec::new();
    let mut new: Vec<(String, String)> = Vec::new();
    let mut written: BTreeMap<String, String> = BTreeMap::new();
    for item in flat_decls(&program.items) {
        let TopDecl::Locus(l) = item else { continue };
        let mut ordinal: u32 = 0;
        for member in &l.members {
            let LocusMember::Failure(fd) = member else { continue };
            if fd.params.len() != 2 {
                continue;
            }
            let parent = &l.name.name;
            let key = |facet: &str| format!("{parent}#{ordinal}/{facet}");
            let child_te = &fd.params[0].ty;
            for facet in ["checker", "model", "lowering", "in_place", "ops"] {
                written.insert(key(facet), format!("on_failure(_: {}) in `{parent}`", te_name(child_te)));
            }

            // (a) the checker
            let checker = match resolve_type_expr(child_te, &known) {
                Ty::Unknown => "external".to_string(),
                t => t.display(),
            };
            old.push((key("checker"), checker));
            // (b) the model
            let joined = te_name(child_te);
            let model = if user_loci.contains(&joined) {
                format!("locus:{joined}")
            } else {
                format!("external:{joined}")
            };
            old.push((key("model"), model));
            if let Some(row) = row_of(&checked_rows, parent, ordinal) {
                new.push((
                    key("checker"),
                    match &row.child {
                        ChildRef::Locus(n) => n.clone(),
                        ChildRef::External(_) => "external".to_string(),
                    },
                ));
                // The model has ids for the program's own declarations
                // only: a monomorph (`Cell_Int`) is its template, the
                // declaration written (`Cell`); a locus it has no
                // declaration for (a stdlib one) is referenced by its
                // written spelling.
                new.push((
                    key("model"),
                    match &row.child {
                        ChildRef::Locus(n) if user_loci.contains(n) => format!("locus:{n}"),
                        ChildRef::Locus(_) if user_loci.contains(&row.written) => {
                            format!("locus:{}", row.written)
                        }
                        _ => format!("external:{}", row.written),
                    },
                ));
            }

            // (c) lowering: the merged rows against the merged program
            // (d) restart in place, (e) the op walk
            let debug_in_place = format!("{:?}", fd.body).contains("RestartInPlace");
            old.push((key("in_place"), debug_in_place.to_string()));
            let mut ops = Vec::new();
            let mut retry = None;
            old_walk_ops(&fd.body, &mut ops, &mut retry);
            old.push((key("ops"), ops_value(&ops, retry)));
            if let Some(row) = row_of(&resolved.handlers, parent, ordinal) {
                // The declared set's verdict on the row's name, against
                // the row itself.
                let (verdict, name) = match &row.child {
                    ChildRef::Locus(n) if merged_loci.contains(n) => {
                        (format!("locus:{n}"), format!("locus:{n}"))
                    }
                    ChildRef::Locus(n) => (format!("undeclared:{n}"), format!("locus:{n}")),
                    ChildRef::External(w) if merged_loci.contains(w) => {
                        (format!("declared:{w}"), format!("external:{w}"))
                    }
                    ChildRef::External(w) => (format!("external:{w}"), format!("external:{w}")),
                };
                old.push((key("lowering"), verdict));
                new.push((key("lowering"), name));
                new.push((
                    key("in_place"),
                    row.ops.contains(&hale_syntax::ast::RecoveryOp::RestartInPlace).to_string(),
                ));
                let names: Vec<String> = row.ops.iter().map(|o| op_name(*o).to_string()).collect();
                new.push((key("ops"), ops_value(&names, row.retry_bound)));
            }
            ordinal += 1;
        }
    }
    let id = program_id(origin, src);
    report.compare_rows(
        &id,
        &old,
        &new,
        |k| Some(k.clone()),
        |k| Some(k.clone()),
        |k| vec![written.get(k).cloned().unwrap_or_default()],
        |k| {
            let facet = k.rsplit('/').next().unwrap_or("");
            vec![match facet {
                "checker" => "the duplicate-handler law (check_duplicate_failure_handlers)",
                "model" => "the model's supervises rows",
                "lowering" => "the handler table and the failure route (decl.rs, resolve_failure_route)",
                "in_place" => "restart-in-place attribution (the params snapshot, __resume_<L>)",
                _ => "the model's recovery ops and retry bound",
            }
            .to_string()]
        },
    );
    true
}

/// The shapes the corpus does not hold, each a program that checks
/// clean: a child named through an alias, a generic instantiation, a
/// stdlib path and a cross-seed path, and recovery ops inside a
/// `match` arm and an `if` used as a value. Plain literals, not raw
/// ones: the corpus harvests `r#"…"#` programs from test files, and
/// these belong to this shadow only.
const PROBES: &[(&str, &str, &[(&[&str], &str)])] = &[
    (
        "probe:alias",
        "
locus Child { params { n: Int = 0; } closure boom { captures: n; epoch inline; } fn go() { violate boom; } }
type Kid = Child;
main locus App {
    params { k: Child = Child { }; }
    on_failure(c: Kid, err: ClosureViolation) { restart(c) for 2; }
    run() { self.k.go(); }
}
fn main() { App { }; }
",
        &[],
    ),
    (
        "probe:generic",
        "
locus Cell<T> { params { n: Int = 0; } }
main locus App {
    on_failure(c: Cell<Int>, err: ClosureViolation) { bubble(err); }
}
fn main() { App { }; }
",
        &[],
    ),
    (
        "probe:std_path",
        "
main locus App {
    on_failure(t: std::bus::UnixTransport, err: ClosureViolation) { restart(t); }
}
fn main() { App { }; }
",
        &[],
    ),
    (
        "probe:cross_seed",
        "
locus __lib_x_Worker { params { n: Int = 0; } }
main locus App {
    on_failure(w: lib::Worker, err: ClosureViolation) { restart(w); }
}
fn main() { App { }; }
",
        &[(&["lib", "Worker"], "__lib_x_Worker")],
    ),
    (
        "probe:match_arm",
        "
locus Child { params { n: Int = 0; } closure boom { captures: n; epoch inline; } fn go() { violate boom; } }
main locus App {
    params { k: Child = Child { }; }
    on_failure(c: Child, err: ClosureViolation) {
        match c.n { 0 -> { restart_in_place(c); }, _ -> { bubble(err); } }
    }
    run() { self.k.go(); }
}
fn main() { App { }; }
",
        &[],
    ),
    (
        "probe:if_value",
        "
locus Child { params { n: Int = 0; } closure boom { captures: n; epoch inline; } fn go() { violate boom; } }
main locus App {
    params { k: Child = Child { }; seen: Int = 0; }
    on_failure(c: Child, err: ClosureViolation) {
        let x = if c.n == 0 { restart_in_place(c); 1 } else { 2 };
        self.seen = x;
    }
    run() { self.k.go(); }
}
fn main() { App { }; }
",
        &[],
    ),
];

#[test]
fn the_child_naming_of_every_handler_agrees_or_every_divergence_is_classified() {
    let mut report = Report::new("handler_routing");
    for p in hale_corpus::parseable(|s| hale_syntax::parse_source(s).is_ok()) {
        shadow_one(&mut report, &p.origin, &p.source, &[]);
    }
    for (origin, src, renames) in PROBES {
        let renames: Vec<(Vec<String>, String)> = renames
            .iter()
            .map(|(segs, m)| (segs.iter().map(|s| s.to_string()).collect(), m.to_string()))
            .collect();
        assert!(
            shadow_one(&mut report, origin, src, &renames),
            "{origin} does not check clean: the probe is vacuous"
        );
    }
    assert!(report.programs > 300, "the corpus walk is vacuous ({} programs)", report.programs);
    assert!(report.rows_compared > 100, "too few handlers compared ({})", report.rows_compared);
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
    eprintln!("{}", report.render().lines().next().unwrap_or_default());
    assert!(
        unexplained.is_empty() && stale.is_empty(),
        "{}",
        gate_message(&report, &unexplained, &stale, "crates/hale-types/tests/fixtures/shadow_handler_routing.txt")
    );
}

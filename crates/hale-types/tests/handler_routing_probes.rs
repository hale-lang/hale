//! Which child an `on_failure` handler names, and which recovery ops
//! its body invokes, pinned on probes (F.40 phase 1.4).
//!
//! The handler-routing shadow held the rows' one child resolver
//! (`child_locus_name`) beside the three namings it replaced, over the
//! corpus, and was deleted once they agreed. The corpus holds none of
//! these shapes, so its probes stay here: a child named through an
//! alias, a generic instantiation, a stdlib path and a cross-seed
//! path, and recovery ops inside a `match` arm and inside an `if` used
//! as a value, the two places the model's former op walk never
//! entered. Plain literals, not raw ones: the corpus harvests
//! `r#"…"#` programs from test files, and these belong to this test.

use hale_syntax::ast::RecoveryOp;
use hale_types::handler_routing::{handler_rows, ChildRef};

struct Probe {
    what: &'static str,
    src: &'static str,
    renames: &'static [(&'static [&'static str], &'static str)],
    child: ChildRef,
    ops: &'static [RecoveryOp],
    retry_bound: Option<i64>,
}

fn probes() -> Vec<Probe> {
    vec![
        Probe {
            what: "a child named through a type alias",
            src: "
locus Child { params { n: Int = 0; } closure boom { captures: n; epoch inline; } fn go() { violate boom; } }
type Kid = Child;
main locus App {
    params { k: Child = Child { }; }
    on_failure(c: Kid, err: ClosureViolation) { restart(c) for 2; }
    run() { self.k.go(); }
}
fn main() { App { }; }
",
            renames: &[],
            child: ChildRef::Locus("Child".to_string()),
            ops: &[RecoveryOp::Restart],
            retry_bound: Some(2),
        },
        Probe {
            what: "a child named by a generic instantiation",
            src: "
locus Cell<T> { params { n: Int = 0; } }
main locus App {
    on_failure(c: Cell<Int>, err: ClosureViolation) { bubble(err); }
}
fn main() { App { }; }
",
            renames: &[],
            child: ChildRef::Locus("Cell_Int".to_string()),
            ops: &[RecoveryOp::Bubble],
            retry_bound: None,
        },
        Probe {
            what: "a child named by a stdlib path",
            src: "
main locus App {
    on_failure(t: std::bus::UnixTransport, err: ClosureViolation) { restart(t); }
}
fn main() { App { }; }
",
            renames: &[],
            child: ChildRef::Locus("__StdBusUnixConnectTransport".to_string()),
            ops: &[RecoveryOp::Restart],
            retry_bound: None,
        },
        Probe {
            what: "a child named by a cross-seed path",
            src: "
locus __lib_x_Worker { params { n: Int = 0; } }
main locus App {
    on_failure(w: lib::Worker, err: ClosureViolation) { restart(w); }
}
fn main() { App { }; }
",
            renames: &[(&["lib", "Worker"], "__lib_x_Worker")],
            child: ChildRef::Locus("__lib_x_Worker".to_string()),
            ops: &[RecoveryOp::Restart],
            retry_bound: None,
        },
        Probe {
            what: "a generic child named by a qualified import",
            src: "
locus Cell<T> { params { n: Int = 0; } }
locus ImportedCell<T> { params { n: Int = 0; } }
main locus App {
    on_failure(c: lib::Cell<Int>, err: ClosureViolation) { restart_in_place(c); }
}
fn main() { App { }; }
",
            renames: &[(&["lib", "Cell"], "ImportedCell")],
            child: ChildRef::Locus("ImportedCell_Int".to_string()),
            ops: &[RecoveryOp::RestartInPlace],
            retry_bound: None,
        },
        Probe {
            what: "a qualified generic child behind an alias",
            src: "
locus Cell<T> { params { n: Int = 0; } }
locus ImportedCell<T> { params { n: Int = 0; } }
type Job = lib::Cell<Int>;
main locus App {
    on_failure(c: Job, err: ClosureViolation) { bubble(err); }
}
fn main() { App { }; }
",
            renames: &[(&["lib", "Cell"], "ImportedCell")],
            child: ChildRef::Locus("ImportedCell_Int".to_string()),
            ops: &[RecoveryOp::Bubble],
            retry_bound: None,
        },
        Probe {
            what: "recovery ops inside `match` arms",
            src: "
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
            renames: &[],
            child: ChildRef::Locus("Child".to_string()),
            ops: &[RecoveryOp::RestartInPlace, RecoveryOp::Bubble],
            retry_bound: None,
        },
        Probe {
            what: "a recovery op inside an `if` used as a value",
            src: "
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
            renames: &[],
            child: ChildRef::Locus("Child".to_string()),
            ops: &[RecoveryOp::RestartInPlace],
            retry_bound: None,
        },
    ]
}

#[test]
fn every_probe_handler_names_its_child_and_ops_through_the_rows() {
    let mut wrong = Vec::new();
    for p in probes() {
        let program = hale_syntax::parse_source(p.src)
            .unwrap_or_else(|d| panic!("probe `{}` does not parse: {d:?}", p.what));
        let renames: Vec<(Vec<String>, String)> = p
            .renames
            .iter()
            .map(|(segs, m)| (segs.iter().map(|s| s.to_string()).collect(), m.to_string()))
            .collect();
        let routing = handler_rows(&[&program], &renames, &Default::default());
        let rows: Vec<_> = routing.handlers_of("App").collect();
        let [row] = rows.as_slice() else {
            wrong.push(format!("{}: {} rows for App (expected 1)", p.what, rows.len()));
            continue;
        };
        if row.child != p.child {
            wrong.push(format!("{}: child {:?} (expected {:?})", p.what, row.child, p.child));
        }
        // the route a failing child takes is the row's, by its resolved name
        if routing.route("App", p.child.name()).map(|r| r.ordinal) != Some(row.ordinal) {
            wrong.push(format!("{}: App does not route `{}` to the row", p.what, p.child.name()));
        }
        if row.ops != p.ops {
            wrong.push(format!("{}: ops {:?} (expected {:?})", p.what, row.ops, p.ops));
        }
        if row.retry_bound != p.retry_bound {
            wrong.push(format!(
                "{}: retry bound {:?} (expected {:?})",
                p.what, row.retry_bound, p.retry_bound
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Restart-in-place attribution is a question over the rows' ops: the
/// two probes that restart in place only inside a `match` arm or an
/// `if` value make their child keep a copy of its params.
#[test]
fn restart_in_place_inside_a_match_arm_or_an_if_value_is_seen() {
    for p in probes() {
        if !p.ops.contains(&RecoveryOp::RestartInPlace) {
            continue;
        }
        let program = hale_syntax::parse_source(p.src).expect("probe parses");
        let renames = p.renames.iter().map(|(path, name)| {
            (path.iter().map(|s| s.to_string()).collect(), name.to_string())
        }).collect::<Vec<_>>();
        let routing = handler_rows(&[&program], &renames, &Default::default());
        assert!(
            routing.restarts_in_place(p.child.name()),
            "{}: the rows do not see the restart in place",
            p.what
        );
    }
}

/// The program the identity join is probed on: two handlers of `App`,
/// for two child types. A plain literal, as the probes are.
const TWO_HANDLERS: &str = "
locus Alpha { params { n: Int = 0; } closure boom { captures: n; epoch inline; } fn go() { violate boom; } }
locus Beta { params { n: Int = 0; } closure boom { captures: n; epoch inline; } fn go() { violate boom; } }
main locus App {
    params { a: Alpha = Alpha { }; b: Beta = Beta { }; }
    on_failure(x: Alpha, err: ClosureViolation) { restart(x); }
    on_failure(y: Beta, err: ClosureViolation) { quarantine(y); }
    run() { self.a.go(); self.b.go(); }
}
fn main() { App { }; }
";

/// `App`'s `on_failure` declarations, in order.
fn app_handlers(p: &mut hale_syntax::ast::Program) -> Vec<&mut hale_syntax::ast::FailureDecl> {
    use hale_syntax::ast::{LocusMember, TopDecl};
    p.items
        .iter_mut()
        .filter_map(|i| match i {
            TopDecl::Locus(l) if l.name.name == "App" => Some(l),
            _ => None,
        })
        .flat_map(|l| l.members.iter_mut())
        .filter_map(|m| match m {
            LocusMember::Failure(fd) => Some(fd),
            _ => None,
        })
        .collect()
}

/// Outside review of #1276, finding 3: a declaration is joined to its
/// handler row by identity, not by span. Two handlers built by hand to
/// carry one span (the shape a synthetic AST with shared provenance, or
/// a desugar that stamps one span on several declarations, produces)
/// but distinct ids route two different children to two different
/// ordinals, and each declaration is its own row's alone. Joined by
/// span, both declarations found the first row, and lowering put both
/// bodies in one fn.
#[test]
fn handlers_sharing_a_span_join_their_rows_by_identity() {
    let mut program = hale_syntax::parse_source(TWO_HANDLERS).expect("parses");
    let shared = app_handlers(&mut program)[0].span;
    app_handlers(&mut program)[1].span = shared;
    let snapshot = hale_types::snapshot::mint([("main.hl", &mut program)], &[]);
    let routing = handler_rows(&[&program], &[], &snapshot);

    let alpha = routing.route("App", "Alpha").expect("Alpha routes");
    let beta = routing.route("App", "Beta").expect("Beta routes");
    assert_eq!((alpha.ordinal, beta.ordinal), (0, 1));
    assert_eq!(alpha.span, beta.span, "the two rows share one span");
    assert!(alpha.id.is_some() && beta.id.is_some() && alpha.id != beta.id);

    let decls = app_handlers(&mut program);
    // `NodeId`'s `==` is always true (structural AST equality ignores
    // identity), so the ids are compared by their numbers.
    assert!(!decls[0].id.is_none() && decls[0].id.0 != decls[1].id.0);
    assert!(alpha.is_row_of(decls[0]) && !alpha.is_row_of(decls[1]));
    assert!(beta.is_row_of(decls[1]) && !beta.is_row_of(decls[0]));
}

/// The span stays the join where nothing was minted: the checker's own
/// tests build a bundle no entry point minted, whose rows carry no site.
#[test]
fn an_unminted_handler_joins_its_row_by_span() {
    let mut program = hale_syntax::parse_source(TWO_HANDLERS).expect("parses");
    let routing = handler_rows(&[&program], &[], &Default::default());
    let alpha = routing.route("App", "Alpha").expect("Alpha routes");
    let beta = routing.route("App", "Beta").expect("Beta routes");
    assert!(alpha.id.is_none() && beta.id.is_none());
    let decls = app_handlers(&mut program);
    assert!(decls[0].id.is_none());
    assert!(alpha.is_row_of(decls[0]) && !alpha.is_row_of(decls[1]));
    assert!(beta.is_row_of(decls[1]) && !beta.is_row_of(decls[0]));
}

#[test]
fn specialization_preserves_handler_sites_and_resolves_each_child_declaration() {
    use hale_syntax::ast::{flat_decls, LocusMember, TopDecl, TypeExpr};
    use hale_types::placement::SiteRef;
    let src = "
locus Alpha { }
locus Beta { }
locus T { }
locus Cell<T> { }
locus ImportedCell<T> { }
locus Parent<T> {
    on_failure(c: T, err: ClosureViolation) { bubble(err); }
    on_failure(c: lib::Cell<T>, err: ClosureViolation) { restart_in_place(c); }
}
locus Other<T> {
    on_failure(c: T, err: ClosureViolation) { quarantine(c); }
}
locus First {
    on_failure(c: Alpha, err: ClosureViolation) { bubble(err); }
    on_failure(c: lib::Cell<Int>, err: ClosureViolation) { bubble(err); }
}
locus Second {
    on_failure(c: Beta, err: ClosureViolation) { bubble(err); }
    on_failure(c: lib::Cell<String>, err: ClosureViolation) { bubble(err); }
}
";
    let mut program = hale_syntax::parse_source(src).expect("parse");
    // Distinct declarations can share source spans after synthesis.
    let parent_span = program.items.iter().find_map(|d| match d {
        TopDecl::Locus(l) if l.name.name == "Parent" => Some(l.span),
        _ => None,
    }).unwrap();
    for d in &mut program.items {
        if let TopDecl::Locus(l) = d {
            if l.name.name == "Other" { l.span = parent_span; }
        }
    }
    let snapshot = hale_types::snapshot::mint([("main.hl", &mut program)], &[]);
    let loci = flat_decls(&program.items).filter_map(|d| match d {
        TopDecl::Locus(l) => Some((l.name.name.as_str(), l)),
        _ => None,
    }).collect::<std::collections::BTreeMap<_, _>>();
    let parent = loci["Parent"];
    let renames = vec![(vec!["lib".into(), "Cell".into()], "ImportedCell".into())];
    let mut routing = handler_rows(&[&program], &renames, &snapshot);
    let original = routing.handlers_of_decl(parent.id).cloned().collect::<Vec<_>>();
    assert_eq!(original.len(), 2);
    for (name, concrete) in [("Parent_Alpha", "First"), ("Parent_Beta", "Second")] {
        let types = loci[concrete].members.iter().filter_map(|m| match m {
            LocusMember::Failure(f) => Some(f.params[0].ty.clone()),
            _ => None,
        }).collect::<Vec<_>>();
        routing.specialize(parent, name, |ty| match ty {
            TypeExpr::Named { path, .. } if path.segments[0].name == "T" => types[0].clone(),
            _ => types[1].clone(),
        });
    }
    for (name, child, cell) in [
        ("Parent_Alpha", "Alpha", "ImportedCell_Int"),
        ("Parent_Beta", "Beta", "ImportedCell_String"),
    ] {
        let rows = routing.handlers_of_instance(parent.id, name).collect::<Vec<_>>();
        assert_eq!(rows.len(), 2);
        for (row, template) in rows.iter().zip(&original) {
            assert_eq!(row.id, template.id);
            assert_eq!(row.parent_id, template.parent_id);
            assert_eq!(row.ops, template.ops);
            assert_eq!(row.ordinal, template.ordinal);
        }
        assert_eq!(rows[0].child, ChildRef::Locus(child.into()));
        assert_eq!(rows[1].child, ChildRef::Locus(cell.into()));
        assert_eq!(rows[0].child_decl, snapshot.site_id(loci[child].id).map(SiteRef::user));
        assert_eq!(rows[1].child_decl, snapshot.site_id(loci["ImportedCell"].id).map(SiteRef::user));
        assert_eq!(routing.route_instance(parent.id, name, cell).unwrap().id, original[1].id);
        assert!(routing.restarts_in_place(cell));
        assert!(routing.handlers_of_instance(loci["Other"].id, name)
            .all(|r| r.ops == [RecoveryOp::Quarantine]));
    }
    assert!(routing.route_instance(parent.id, "Parent_Alpha", "Beta").is_none());
    assert!(routing.route_instance(parent.id, "Parent_Beta", "ImportedCell_Int").is_none());
    assert_eq!(routing.handlers_of_decl(parent.id).next().unwrap().child, original[0].child);
    assert_eq!(routing.rows().len(), 7, "specialization does not change snapshot rows");
}

/// The restart rows' failure column: a locus declaring a closure of
/// any epoch (an `inline` one, which every `violate` names, included)
/// or a `birth_check` can fail, and pays for restart points; one with
/// neither cannot. The column is per declaration, by its declared name:
/// a monomorph's name is none, so a generic template's specializations
/// answer no (known open), as lowering's walk over the declarations did.
#[test]
fn the_failure_column_names_each_declaration_a_failure_can_originate_in() {
    let src = "
locus Ticks { params { n: Int = 0; } closure small { self.n ~~ 0 within 9; epoch tick; } }
locus Inline { params { n: Int = 0; } closure boom { captures: n; epoch inline; } run() { violate boom; } }
locus Checked { params { n: Int = 0; } birth_check { self.n >= 0 } -> violate negative; }
locus Plain { params { n: Int = 0; } run() { } }
locus Cell<T> { params { value: T; n: Int = 0; } closure boom { captures: n; epoch inline; } }
";
    let program = hale_syntax::parse_source(src).expect("parse");
    let routing = handler_rows(&[&program], &[], &Default::default());
    for (locus, fails) in
        [("Ticks", true), ("Inline", true), ("Checked", true), ("Plain", false), ("Cell", true), ("Cell_Int", false)]
    {
        assert_eq!(routing.can_fail(locus), fails, "{locus}");
    }
}

/// The restart rows' bounds: every `for` a recovery statement writes,
/// keyed by the statement's span, which is how lowering reads its
/// bound (`retry_bound_at`). A literal is its value; any other
/// expression is the site of the expression the statement writes,
/// lowered once where the statement runs. The model's `retry_bound`
/// is the last literal of the same entries, so the two are one fact.
#[test]
fn every_for_bound_is_an_entry_lowering_reads_by_the_statements_span() {
    use hale_syntax::ast::{flat_decls, LocusMember, Stmt, TopDecl};
    use hale_types::handler_routing::RetryBound;
    let src = "
locus Worker { params { n: Int = 0; } closure boom { captures: n; epoch inline; } }
locus App {
    params { max: Int = 2; }
    on_failure(c: Worker, err: ClosureViolation) {
        if self.max > 1 { restart(c) for 3; } else { restart_in_place(c) for self.max + 1; }
        restart(c);
    }
}
";
    let program = hale_syntax::parse_source(src).expect("parse");
    let routing = handler_rows(&[&program], &[], &Default::default());
    let row = routing.route("App", "Worker").expect("App routes Worker");
    assert_eq!(row.retry_bound, Some(3), "the model's bound is the last literal");
    assert_eq!(row.bounds.len(), 2, "the unbounded restart states none: {:?}", row.bounds);

    let handler = flat_decls(&program.items)
        .find_map(|d| match d {
            TopDecl::Locus(l) if l.name.name == "App" => l.members.iter().find_map(|m| match m {
                LocusMember::Failure(fd) => Some(fd),
                _ => None,
            }),
            _ => None,
        })
        .expect("App's handler");
    let Stmt::If(branch) = &handler.body.stmts[0] else { panic!("an if first") };
    let recovery = |b: &hale_syntax::ast::Block| match &b.stmts[0] {
        Stmt::Recovery { span, modifier: Some(hale_syntax::ast::RecoveryModifier::For(e)), .. } => (*span, e.span()),
        other => panic!("a bounded recovery: {other:?}"),
    };
    let (literal, _) = recovery(&branch.then_block);
    let Some(hale_syntax::ast::ElseBranch::Else(otherwise)) = branch.else_block.as_deref() else {
        panic!("an else block")
    };
    let (runtime, written) = recovery(otherwise);
    assert_eq!(routing.retry_bound_at(literal), Some(RetryBound::Const(3)));
    assert_eq!(routing.retry_bound_at(runtime), Some(RetryBound::Expr(written)));
    let Stmt::Recovery { span: unbounded, .. } = &handler.body.stmts[1] else { panic!("a recovery second") };
    assert_eq!(routing.retry_bound_at(*unbounded), None);
}

/// A concrete locus's routing identity (`instance_key`), which the
/// failure route matches a supervising parent's child by: a locus by
/// its declaration's site, a monomorph by its template's site and its
/// specialization, and an unminted declaration by its name.
#[test]
fn a_concrete_locus_is_keyed_by_its_declaration_and_specialization() {
    use hale_syntax::ast::{flat_decls, LocusMember, NodeId, TopDecl};
    use hale_types::handler_routing::InstanceKey;
    let src = "
locus Alpha { }
locus Beta { }
locus Cell<T> {
    on_failure(c: T, err: ClosureViolation) { bubble(err); }
}
locus Holder {
    on_failure(c: Alpha, err: ClosureViolation) { bubble(err); }
}
";
    let mut program = hale_syntax::parse_source(src).expect("parse");
    let snapshot = hale_types::snapshot::mint([("main.hl", &mut program)], &[]);
    let loci = flat_decls(&program.items).filter_map(|d| match d {
        TopDecl::Locus(l) => Some((l.name.name.as_str(), l)),
        _ => None,
    }).collect::<std::collections::BTreeMap<_, _>>();
    let alpha_ty = loci["Holder"].members.iter().find_map(|m| match m {
        LocusMember::Failure(f) => Some(f.params[0].ty.clone()),
        _ => None,
    }).unwrap();
    let mut routing = handler_rows(&[&program], &[], &snapshot);
    routing.specialize(loci["Cell"], "Cell_Alpha", |_| alpha_ty.clone());
    let (alpha, beta, cell) = (loci["Alpha"].id, loci["Beta"].id, loci["Cell"].id);
    assert_eq!(routing.instance_key(alpha, "Alpha"), InstanceKey::Decl(alpha.0, None));
    assert_ne!(routing.instance_key(alpha, "Alpha"), routing.instance_key(beta, "Beta"));
    assert_eq!(
        routing.instance_key(cell, "Cell_Alpha"),
        InstanceKey::Decl(cell.0, Some("Cell_Alpha".to_string()))
    );
    assert_ne!(routing.instance_key(cell, "Cell_Alpha"), routing.instance_key(cell, "Cell_Beta"));
    assert_eq!(routing.instance_key(NodeId::NONE, "Alpha"), InstanceKey::Unminted("Alpha".to_string()));
}

// F.40 phase 4, Q1: the lowering view's handler rows are the snapshot's,
// carried by the correspondence, with the stdlib's derived over the
// merged program's tail and the indexes rebuilt over the union. The view
// used to derive them over the whole merged program; these pin that the
// fold keeps every answer lowering reads.

const SUPERVISED: &str = "
locus Worker {
    params { n: Int = 0; }
    closure boom { captures: n; epoch inline; }
    fn go() { violate boom; }
}
locus Sup<T> {
    on_failure(c: T, err: ClosureViolation) { restart_in_place(c) for 2; }
}
main locus App {
    params { max: Int = 2; w: Worker = Worker { }; b: std::bytes::BytesBuilder = std::bytes::BytesBuilder { }; }
    on_failure(c: Worker, err: ClosureViolation) {
        if self.max > 1 { restart(c) for 3; } else { restart_in_place(c) for self.max + 1; }
    }
    on_failure(c: std::bytes::BytesBuilder, err: ClosureViolation) { quarantine(c); }
    fn again() { restart(self.w) for 4; }
}
fn main() { App { }; }
";

// The recovery statements outside every handler are the typed bodies'
// `recoveries` column (F.40 phase 4's leftovers; W3 read them from the
// syntax), each with its operation and the locus the checker typed its
// receiver as; a handler's statements stay its row's ops.

/// Each recovery row of the checked program: its parent, op, bound, the
/// locus its receiver names and the statement's text.
fn recoveries(src: &str) -> Vec<(Option<String>, RecoveryOp, bool, Option<String>, String)> {
    use hale_frontend::snapshot::{Config, Snapshot};
    use hale_syntax::ast::{flat_decls, TopDecl};
    let program = hale_syntax::parse_source(src).expect("parses");
    let Ok(s) = Snapshot::from_program(program, Vec::new(), Config::check(true, false)) else { panic!("snapshot") };
    let checked = s.demand_check().unwrap_or_else(|_| panic!("checked"));
    let errors: Vec<&String> = checked.diags.iter().filter(|x| x.is_error()).map(|x| &x.message).collect();
    assert!(errors.is_empty(), "{errors:?}");
    let table = s.demand_typed_bodies().unwrap_or_else(|_| panic!("the table"));
    let p = s.program().unwrap();
    let name_of = |r: &hale_types::typed_bodies::LocusRef| {
        flat_decls(&p.items).find_map(|d| match d {
            TopDecl::Locus(l) if l.id.0 == r.decl.0 => Some(l.name.name.clone()),
            _ => None,
        })
    };
    table
        .recoveries()
        .map(|(_, r)| {
            let text = src[r.statement.start.0 as usize..r.statement.end.0 as usize].to_string();
            (r.parent_name.clone(), r.op, r.bounded, r.child.as_ref().and_then(name_of), text)
        })
        .collect()
}

#[test]
fn a_recovery_statement_outside_a_handler_is_a_row_with_its_child() {
    assert_eq!(
        recoveries(SUPERVISED),
        [(Some("App".to_string()), RecoveryOp::Restart, true, Some("Worker".to_string()), "restart(self.w) for 4;".to_string())],
        "the handlers' statements are their rows' ops, not this column's"
    );
}

#[test]
fn a_receiver_is_named_by_the_type_the_checker_gives_it() {
    // A param, an alias, a local, a field of another locus's value and a
    // call's result: the checker types each, so each names its child.
    let src = "
locus Worker { params { n: Int = 0; } closure boom { captures: n; epoch inline; } fn go() { violate boom; } }
type Hand = Worker;
locus Keeper { params { w: Worker = Worker { }; } }
main locus App {
    params { w: Worker = Worker { }; h: Hand = Worker { }; k: Keeper = Keeper { }; }
    fn by_param(c: Worker) { quarantine(c); }
    fn by_alias() { restart_in_place(self.h); }
    fn by_local() { let x = self.w; restart(x); }
    fn by_field() { restart(self.k.w); }
    fn by_call() { quarantine(make()); }
}
fn make() -> Worker { return Worker { }; }
fn by_free_param(c: Worker) { restart(c); }
fn main() { App { }; }
";
    let worker = Some("Worker".to_string());
    let app = Some("App".to_string());
    let rows: Vec<(Option<String>, RecoveryOp, Option<String>)> =
        recoveries(src).into_iter().map(|(p, op, _, child, _)| (p, op, child)).collect();
    assert_eq!(
        rows,
        [
            (app.clone(), RecoveryOp::Quarantine, worker.clone()),
            (app.clone(), RecoveryOp::RestartInPlace, worker.clone()),
            (app.clone(), RecoveryOp::Restart, worker.clone()),
            (app.clone(), RecoveryOp::Restart, worker.clone()),
            (app, RecoveryOp::Quarantine, worker.clone()),
            (None, RecoveryOp::Restart, worker),
        ]
    );
}

/// The bounds are keyed by the statement's span alone, and the stdlib's
/// spans overlap the first user file's: a stdlib bound at a user bound's
/// span would answer for it. The stdlib writes no recovery statement and
/// declares no handler, so its rows hold no bound to collide with; the
/// failure column is the one thing it adds (its loci declaring a closure
/// or a `birth_check`).
#[test]
fn the_stdlib_states_no_recovery_bound_and_declares_no_handler() {
    let std = hale_types::stdlib_bodies::program().expect("the stdlib parses");
    let ids = hale_types::stdlib_bodies::identities().expect("and is minted");
    let rows = handler_rows(&[std], &[], ids);
    assert!(rows.rows().is_empty(), "the stdlib declares no on_failure handler");
    assert_eq!(rows.bounds().count(), 0, "the stdlib writes no `for` bound");
    assert!(rows.can_fail("__StdBytesBytesBuilder"), "a stdlib locus with a closure can fail");
}

/// Every answer lowering reads of the view's rows, as `handler_rows`
/// gives it over the merged program (the derivation the view ran before
/// the fold): the rows and their indexes, the failure column, the
/// bounds, the in-place restarts, and a specialization's rows and key.
/// The rows' identities are compared by index, the key lowering joins a
/// declaration by (`is_row_of`, `handlers_of_decl`).
#[test]
fn the_views_rows_answer_as_the_merged_programs_did() {
    use hale_frontend::frontend::LoadMode;
    use hale_frontend::snapshot::{Config, Snapshot, Target};
    use hale_syntax::ast::{flat_decls, LocusDecl, TopDecl, TypeExpr};
    use hale_types::handler_routing::{HandlerRouting, HandlerRow};

    let d = std::env::temp_dir().join(format!("hale-handler-fold-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("app.hl"), SUPERVISED).unwrap();
    let config = Config::build(Target::host());
    let s = Snapshot::load(&d.join("app.hl"), LoadMode::WholeSeed, &hale_frontend::source::Disk, config)
        .unwrap_or_else(|_| panic!("the fixture loads"));
    let checked = s.demand_check().expect("checked");
    let errors: Vec<&String> = checked.diags.iter().filter(|x| x.is_error()).map(|x| &x.message).collect();
    assert!(errors.is_empty(), "{errors:?}");
    let view = s.demand_lowering().unwrap_or_else(|_| panic!("a clean program is lowered"));
    let mut lowered = view.handlers.clone();
    let mut merged = handler_rows(&[&view.merged], &view.import_renames, &view.snapshot);

    let loci: Vec<&LocusDecl> = flat_decls(&view.merged.items)
        .filter_map(|i| match i {
            TopDecl::Locus(l) => Some(l),
            _ => None,
        })
        .collect();
    let row = |r: &HandlerRow| {
        format!(
            "{} {:?} {:?} {} {} {} {:?} {:?} {:?} {:?} {:?}",
            r.parent,
            r.parent_id.map(|s| s.index),
            r.child,
            r.written,
            r.error_type,
            r.ordinal,
            r.id.map(|s| s.index),
            r.span,
            r.ops,
            r.bounds,
            r.retry_bound
        )
    };
    let answers = |h: &HandlerRouting| {
        let mut out: Vec<String> = h.rows().iter().map(row).collect();
        for r in h.rows() {
            let first = h.route(&r.parent, r.child.name()).map(row);
            out.push(format!("route {} {} -> {first:?}", r.parent, r.child.name()));
        }
        for l in &loci {
            let name = &l.name.name;
            out.push(format!("decl {name}: {:?}", h.handlers_of_decl(l.id).map(row).collect::<Vec<_>>()));
            out.push(format!("by name {name}: {:?}", h.handlers_of(name).map(row).collect::<Vec<_>>()));
            out.push(format!("can_fail {name}: {}", h.can_fail(name)));
            out.push(format!("in place {name}: {}", h.restarts_in_place(name)));
            out.push(format!("key {name}: {:?}", h.instance_key(l.id, name)));
        }
        out.extend(h.bounds().map(|b| format!("bound {b:?}")));
        out
    };
    assert_eq!(answers(&lowered), answers(&merged), "the view's rows answer as the merged program's did");
    assert_eq!(lowered.bounds().count(), 4, "three in the handler, one in a method");
    assert!(lowered.can_fail("__StdBytesBytesBuilder") && lowered.can_fail("Worker"));
    // The one column the fold moves: a user row's `child_decl` is the
    // snapshot's, a stdlib child named in the analysis copy, where the
    // merged program named its merged twin. No lowering reader reads it
    // (the model reads the snapshot's rows).
    use hale_types::placement::SiteUniverse;
    let universe = |h: &HandlerRouting| {
        h.rows().iter().find(|r| r.written == "std::bytes::BytesBuilder").and_then(|r| r.child_decl).map(|d| d.universe)
    };
    assert_eq!(universe(&lowered), Some(SiteUniverse::StdlibAnalysis));
    assert_eq!(universe(&lowered), universe(s.demand_handlers().expect("the snapshot's rows")));
    assert_eq!(universe(&merged), Some(SiteUniverse::User));
    // The recovery statements outside the handlers are no column of these
    // rows: the typed bodies hold them, the method's one statement.
    let typed = s.demand_typed_bodies().unwrap_or_else(|_| panic!("the typed bodies"));
    assert_eq!(typed.recoveries().count(), 1);

    // A specialization: the template's rows under lowering's substitution.
    let sup = *loci.iter().find(|l| l.name.name == "Sup").expect("the generic supervisor");
    let zero = hale_syntax::Span::new(0, 0);
    let worker = TypeExpr::Named {
        path: hale_syntax::ast::QualifiedName {
            segments: vec![hale_syntax::ast::Ident::new("Worker", zero)],
            span: zero,
        },
        generic_args: Vec::new(),
        span: zero,
    };
    for h in [&mut lowered, &mut merged] {
        h.specialize(sup, "Sup_Worker", |_| worker.clone());
    }
    let special = |h: &HandlerRouting| {
        (
            h.handlers_of_instance(sup.id, "Sup_Worker").map(row).collect::<Vec<_>>(),
            h.instance_key(sup.id, "Sup_Worker"),
            h.restarts_in_place("Worker"),
        )
    };
    assert_eq!(special(&lowered), special(&merged));
    let _ = std::fs::remove_dir_all(&d);
}

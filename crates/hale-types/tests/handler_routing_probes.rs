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

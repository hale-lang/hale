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
        let routing = handler_rows(&[&program], &[], &Default::default());
        assert!(
            routing.restarts_in_place("Child"),
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

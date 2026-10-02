//! The typed-body table (F.40 phase 3, E4): what the checker typed,
//! keyed by declaration identity, one per snapshot.
//!
//! The checker records its own answers as it walks; the snapshot
//! packages them on demand (`Snapshot::demand_typed_bodies`), and a
//! check that never asks builds no table.

use hale_frontend::snapshot::{Config, Snapshot, Target};
use hale_syntax::ast::{LocusMember, NodeId, Program, Stmt, TopDecl};
use hale_types::ty::Ty;
use hale_types::typed_bodies::{AccumulatorKind, CalleeKind, TemplateKind, Typed};

const PROGRAM: &str = r#"
type Fault { why: String = ""; }
type Box<T> { value: T; }

interface Reading { fn value() -> Int; }

locus Sensor {
    params { base: Int = 10; }
    fn value() -> Int { return self.base; }
}

locus Mute {
    params { n: Int = 0; }
}

locus Tracker {
    params { delta: Int = 0; level: Float = 0.0; }
    closure band { mean(self.delta) ~~ sum(self.level) within 100.0; epoch tick; }
    closure counted { count() ~~ 2 within 0; epoch tick; }
}

fn risky(n: Int) -> Int fallible(Fault) { return n; }
fn first<T>(x: T) -> T { return x; }

fn main() {
    let b: Box<Int> = Box { value: 1 };
    let v = first(42);
    let r = risky(v) or 0;
    let p = std::str::parse_int("4") or 0;
    println(r + p + b.value);
}
"#;

fn snapshot(config: Config) -> Snapshot {
    let program = hale_syntax::parse_source(PROGRAM).expect("the fixture parses");
    match Snapshot::from_program(program, Vec::new(), config) {
        Ok(s) => s,
        Err(_) => panic!("a bare program's snapshot is not refused"),
    }
}

fn program(s: &Snapshot) -> &Program {
    s.program().expect("one program")
}

fn decl<'p>(p: &'p Program, name: &str) -> &'p TopDecl {
    p.items
        .iter()
        .find(|i| match i {
            TopDecl::Locus(l) => l.name.name == name,
            TopDecl::Fn(f) => f.name.name == name,
            TopDecl::Interface(i) => i.name.name == name,
            TopDecl::Type(t) => t.name.name == name,
            _ => false,
        })
        .unwrap_or_else(|| panic!("`{name}` is declared"))
}

fn id_of(d: &TopDecl) -> NodeId {
    match d {
        TopDecl::Locus(l) => l.id,
        TopDecl::Fn(f) => f.id,
        TopDecl::Interface(i) => i.id,
        TopDecl::Type(t) => t.id,
        _ => NodeId::NONE,
    }
}

/// The calls `main` makes, in order, by their minted sites.
fn main_calls(p: &Program) -> Vec<NodeId> {
    let TopDecl::Fn(main) = decl(p, "main") else { unreachable!() };
    let mut out = Vec::new();
    for s in &main.body.stmts {
        let Stmt::Let { value, .. } = s else { continue };
        let call = match value {
            hale_syntax::ast::Expr::Or { inner, .. } => inner.as_ref(),
            other => other,
        };
        if let hale_syntax::ast::Expr::Call { id, .. } = call {
            out.push(*id);
        }
    }
    out
}

#[test]
fn a_check_builds_no_table_and_a_demand_builds_one() {
    let s = snapshot(Config::check(true, false));
    s.demand_check().expect("checked");
    assert_eq!(s.builds()["typed_bodies"], 0, "a check never asks for the table");
    assert_eq!(s.builds()["expression_typing"], 1);
    let first = s.demand_typed_bodies().expect("the table") as *const _;
    let again = s.demand_typed_bodies().expect("still there") as *const _;
    assert_eq!(first, again);
    assert_eq!(s.builds()["typed_bodies"], 1, "one table per snapshot");
    assert_eq!(s.builds()["expression_typing"], 1, "the table is the check's record: no second check");
}

#[test]
fn each_column_holds_the_checker_s_answer_by_identity() {
    let s = snapshot(Config::check(true, false));
    let table = s.demand_typed_bodies().expect("the table");
    let p = program(&s);

    // 1. accumulators, per closure, in slot order.
    let TopDecl::Locus(tracker) = decl(p, "Tracker") else { unreachable!() };
    let closure = |name: &str| {
        tracker
            .members
            .iter()
            .find_map(|m| match m {
                LocusMember::Closure(c) if c.name.name == name => Some(c.id),
                _ => None,
            })
            .unwrap()
    };
    let band = table.accumulators(closure("band"));
    assert_eq!(band.len(), 2);
    assert_eq!(band[0].kind, AccumulatorKind::Mean);
    assert_eq!(band[0].elem, Some(Typed::Known(Ty::Prim(hale_syntax::ast::PrimType::Int))));
    assert_eq!(band[1].kind, AccumulatorKind::Sum);
    assert_eq!(band[1].elem, Some(Typed::Known(Ty::Prim(hale_syntax::ast::PrimType::Float))));
    let counted = table.accumulators(closure("counted"));
    assert_eq!(counted.len(), 1);
    assert_eq!((counted[0].kind, &counted[0].elem), (AccumulatorKind::Count, &None));

    // 2. the generic call, and 3. its monomorph and the written one.
    let calls = main_calls(p);
    assert_eq!(calls.len(), 3, "first, risky, parse_int");
    let first = id_of(decl(p, "first"));
    let Some(Typed::Known(call)) = table.generic_call(calls[0]) else {
        panic!("`first(42)` is typed: {:?}", table.generic_call(calls[0]))
    };
    assert_eq!(call.template, first);
    assert_eq!(call.type_args, vec![Ty::Prim(hale_syntax::ast::PrimType::Int)]);
    assert_eq!(call.params, vec![Ty::Prim(hale_syntax::ast::PrimType::Int)]);
    let mono = table.monomorphs().of(first, &call.type_args).expect("the specialization");
    assert_eq!((mono.kind, mono.name.as_str()), (TemplateKind::Fn, "first_Int"));
    let boxed = table
        .monomorphs()
        .of(id_of(decl(p, "Box")), &[Ty::Prim(hale_syntax::ast::PrimType::Int)])
        .expect("`Box<Int>` is written");
    assert_eq!((boxed.kind, boxed.name.as_str()), (TemplateKind::Type, "Box_Int"));
    assert!(std::ptr::eq(table.monomorphs().named("Box_Int").unwrap(), boxed));

    // 4. conformance, per declared pair, with the witness.
    let reading = id_of(decl(p, "Reading"));
    let sensor = table.conformance(id_of(decl(p, "Sensor")), reading).expect("a row");
    assert_eq!(sensor.verdict, Ok(()));
    let mute = table.conformance(id_of(decl(p, "Mute")), reading).expect("a row");
    assert!(mute.verdict.as_ref().unwrap_err().contains("missing method `value`"));

    // 5. the fallible calls: the user's, typed `Fallible`, and the
    // stdlib's, the signature table's mark.
    let risky = table.fallible_call(calls[1]).expect("`risky` can fail");
    assert_eq!(risky.kind, CalleeKind::Typed);
    assert_eq!(risky.payload, Ty::Named("Fault".into()));
    let parse = table.fallible_call(calls[2]).expect("`parse_int` can fail");
    assert_eq!(parse.kind, CalleeKind::Stdlib);
    assert_eq!(parse.payload, Ty::Named("ParseError".into()));
    assert!(table.fallible_call(calls[0]).is_none(), "`first` cannot fail");
}

/// A generic locus's closure: the template's `self.x` is `T`, which
/// the checker types `Unknown` (a hole), and each monomorph the program
/// writes gets its rows with the field's type substituted, found by the
/// template's site and the monomorph's arguments.
#[test]
fn a_generic_locus_s_accumulators_are_specialized_per_monomorph() {
    let src = "locus Acc<T> {\n    params { x: T; }\n    \
               closure total { sum(self.x) ~~ 0 within 1000; epoch tick; }\n}\n\
               fn main() {\n    let a: Acc<Int> = Acc { x: 3 };\n    let b: Acc<Float> = Acc { x: 1.5 };\n}\n";
    let program = hale_syntax::parse_source(src).expect("parses");
    let Ok(s) = Snapshot::from_program(program, Vec::new(), Config::check(true, false)) else {
        panic!("not refused")
    };
    let table = s.demand_typed_bodies().expect("the table");
    let TopDecl::Locus(acc) = decl(s.program().unwrap(), "Acc") else { unreachable!() };
    let total = acc
        .members
        .iter()
        .find_map(|m| match m {
            LocusMember::Closure(c) => Some(c.id),
            _ => None,
        })
        .unwrap();
    assert!(
        matches!(table.accumulators(total)[0].elem, Some(Typed::Hole(_))),
        "the template's `self.x` is a `T`: {:?}",
        table.accumulators(total)
    );
    for (name, want) in [("Acc_Int", hale_syntax::ast::PrimType::Int), ("Acc_Float", hale_syntax::ast::PrimType::Float)] {
        let mono = table.monomorphs().named(name).expect("written");
        assert_eq!(mono.template, acc.id);
        let rows = table.specialized_accumulators(total, mono);
        assert_eq!(rows.len(), 1, "{name}");
        assert_eq!(rows[0].elem, Some(Typed::Known(Ty::Prim(want))), "{name}");
    }
}

/// A site the checker could not type is a hole with its reason.
#[test]
fn an_unpinned_generic_call_is_a_hole() {
    let src = "fn make<T>(n: Int) -> Int { return n; }\nfn main() { let x = make(1); println(x); }\n";
    let program = hale_syntax::parse_source(src).expect("parses");
    let Ok(s) = Snapshot::from_program(program, Vec::new(), Config::harness(Target::host())) else {
        panic!("not refused")
    };
    let table = s.demand_typed_bodies().expect("the table of a program no check gates");
    let p = s.program().unwrap();
    let call = main_calls(p)[0];
    match table.generic_call(call) {
        Some(Typed::Hole(h)) => assert!(h.reason.contains("pins `T`"), "{}", h.reason),
        other => panic!("expected a hole: {other:?}"),
    }
}

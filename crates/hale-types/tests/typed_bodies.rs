//! The typed-body table (F.40 phase 3, E4): what the checker typed,
//! keyed by declaration identity, one per snapshot.
//!
//! The checker records its own answers as it walks; the snapshot
//! packages them on demand (`Snapshot::demand_typed_bodies`), once:
//! the check demands it for the `bare_fallible` law, and runs no
//! second check to build it.

use hale_frontend::snapshot::{Config, Snapshot, Target};
use hale_syntax::ast::{LocusMember, NodeId, Program, Stmt, TopDecl};
use hale_types::ty::Ty;
use hale_types::typed_bodies::{AccumulatorKind, CalleeKind, Handling, TemplateKind, Typed, Unsatisfied};

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
fn a_check_demands_the_table_once_and_runs_no_second_check() {
    let s = snapshot(Config::check(true, false));
    s.demand_check().expect("checked");
    assert_eq!(s.builds()["typed_bodies"], 1, "the check's `bare_fallible` law reads the table");
    assert_eq!(s.builds()["expression_typing"], 1, "the table is the check's record: no second check");
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
    assert_eq!(call.template.0, first.0);
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
    assert_eq!(mute.verdict, Err(Unsatisfied::Missing { method: "value".into() }));

    // 5. the fallible calls: the user's, a fn the program declares, and
    // the stdlib's, the signature table's mark; each the operand of an
    // `or`.
    let risky = table.fallible_call(calls[1]).expect("`risky` can fail");
    assert_eq!(risky.kind, CalleeKind::Declared);
    assert_eq!(risky.callee, "risky");
    assert_eq!(risky.payload, Ty::Named("Fault".into()));
    assert_eq!(risky.handled, Handling::Or);
    let parse = table.fallible_call(calls[2]).expect("`parse_int` can fail");
    assert_eq!(parse.kind, CalleeKind::Stdlib);
    assert_eq!(parse.callee, "std::str::parse_int");
    assert_eq!(parse.payload, Ty::Named("ParseError".into()));
    assert_eq!(parse.handled, Handling::Or);
    assert!(table.fallible_call(calls[0]).is_none(), "`first` cannot fail");
}

/// The fallible column's marks and positions, which the `bare_fallible`
/// law reads: a declared fn as an `or`'s handler, a built-in method and
/// a generic fn as one, and the bare positions.
#[test]
fn the_fallible_column_records_the_callee_and_what_addresses_it() {
    const SRC: &str = r#"
type Fault { why: String = ""; }
fn risky(n: Int) -> Int fallible(Fault) { return n; }
fn pick<T>(x: T) -> T fallible(Fault) { return x; }
fn take(n: Int) -> Int { return n; }
fn via(n: Int) -> Int fallible(Fault) {
    let a = risky(n) or risky(1);
    let b = risky(n) or pick(2);
    return a + b;
}
fn main() {
    let arr = [1, 2, 3];
    let c = arr.get(1) or 0;
    let d = take(risky(3));
    match risky(4) { _ -> { println(c, d); }, }
}
"#;
    let program = hale_syntax::parse_source(SRC).expect("the fixture parses");
    let s = match Snapshot::from_program(program, Vec::new(), Config::check(true, false)) {
        Ok(s) => s,
        Err(_) => panic!("a bare program's snapshot is not refused"),
    };
    let table = s.demand_typed_bodies().expect("the table");
    let mut rows: Vec<(String, CalleeKind, &'static str)> = table
        .fallible_calls()
        .map(|r| {
            let at = match r.handled {
                Handling::Or => "or",
                Handling::Handler(_) => "handler",
                Handling::Bare => "bare",
            };
            (r.callee.clone(), r.kind, at)
        })
        .collect();
    rows.sort_by(|a, b| (a.0.as_str(), a.2).cmp(&(b.0.as_str(), b.2)));
    assert_eq!(
        rows,
        vec![
            ("arr.get".to_string(), CalleeKind::Typed, "or"),
            ("pick".to_string(), CalleeKind::Typed, "handler"),
            ("risky".to_string(), CalleeKind::Declared, "bare"),
            ("risky".to_string(), CalleeKind::Declared, "bare"),
            ("risky".to_string(), CalleeKind::Declared, "handler"),
            ("risky".to_string(), CalleeKind::Declared, "or"),
            ("risky".to_string(), CalleeKind::Declared, "or"),
        ]
    );
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
        assert_eq!(mono.template.0, acc.id.0);
        let rows = table.specialized_accumulators(total, mono);
        assert_eq!(rows.len(), 1, "{name}");
        assert_eq!(rows[0].elem, Some(Typed::Known(Ty::Prim(want))), "{name}");
    }
}

/// A generic fn's body: the call `first(x)` inside `twice<T>` pins
/// nothing in the template (`x` is a `T`), and each of `twice`'s
/// monomorphs gets its rows with `T` bound, the inner specialization
/// named in the monomorph table.
#[test]
fn a_generic_fn_s_calls_are_specialized_per_monomorph() {
    let src = "fn first<T>(x: T) -> T { return x; }\n\
               fn twice<T>(x: T) -> T { return first(x); }\n\
               fn main() { let n = twice(3); let s = twice(\"ok\"); println(n, s); }\n";
    let program = hale_syntax::parse_source(src).expect("parses");
    let Ok(s) = Snapshot::from_program(program, Vec::new(), Config::check(true, false)) else {
        panic!("not refused")
    };
    let table = s.demand_typed_bodies().expect("the table");
    let p = s.program().unwrap();
    let TopDecl::Fn(twice) = decl(p, "twice") else { unreachable!() };
    let Some(Stmt::Return(Some(hale_syntax::ast::Expr::Call { id: inner, .. }), _)) = twice.body.stmts.first() else {
        panic!("`return first(x);`")
    };
    assert!(matches!(table.generic_call(*inner), Some(Typed::Hole(_))), "{:?}", table.generic_call(*inner));
    let first = id_of(decl(p, "first"));
    for want in [Ty::Prim(hale_syntax::ast::PrimType::Int), Ty::Prim(hale_syntax::ast::PrimType::String)] {
        let Some(Typed::Known(call)) = table.specialized_generic_call(twice.id, std::slice::from_ref(&want), *inner) else {
            panic!("twice<{want:?}>: {:?}", table.body(twice.id))
        };
        assert_eq!((call.template.0, &call.type_args), (first.0, &vec![want.clone()]));
        assert!(table.monomorphs().of(first, &[want]).is_some());
    }
}

/// The conformance column is the checker's one function: a locus whose
/// method matches the interface's by name and not by return type does
/// not satisfy it (the witness says which requirement), no more than a
/// generic locus's specialization does, and the lowering view extends
/// the snapshot's column with the merged stdlib's pairs, found by the
/// identities its mint gave them.
#[test]
fn conformance_is_judged_by_signature_and_the_view_adds_the_stdlib_s_pairs() {
    let src = "interface Greeter { fn greet() -> Int; }\n\
               locus Hi { params { n: Int = 1; } fn greet() -> Int { return self.n; } }\n\
               locus Odd { params { n: Int = 2; } fn greet() -> String { return \"x\"; } }\n\
               locus Holder<T> { params { v: T; } fn greet() -> Int { return 7; } }\n\
               locus Sink { params { n: Int = 0; } fn send(subject: String, bytes: Bytes) { } }\n\
               fn make() -> Greeter { let h: Holder<Int> = Holder { v: 3 }; return Hi { }; }\n\
               fn main() { let g = make(); println(g.greet()); }\n";
    let program = hale_syntax::parse_source(src).expect("parses");
    let Ok(s) = Snapshot::from_program(program, Vec::new(), Config::build(Target::host())) else {
        panic!("not refused")
    };
    let table = s.demand_typed_bodies().expect("the table");
    let p = s.program().unwrap();
    let greeter = id_of(decl(p, "Greeter"));
    let odd = table.conformance(id_of(decl(p, "Odd")), greeter).expect("a row");
    assert_eq!(
        odd.verdict,
        Err(Unsatisfied::Ret {
            method: "greet".into(),
            want: Ty::Prim(hale_syntax::ast::PrimType::Int),
            got: Ty::Prim(hale_syntax::ast::PrimType::String),
        })
    );
    assert_eq!(table.conformance(id_of(decl(p, "Hi")), greeter).expect("a row").verdict, Ok(()));
    let holder = table.monomorphs().named("Holder_Int").expect("the specialization");
    assert_eq!(table.monomorph_conformance(holder, greeter), Some(&Err(Unsatisfied::NotALocus)));

    let view = s.demand_lowering().expect("lowered");
    let adapter = view
        .merged
        .items
        .iter()
        .find_map(|i| match i {
            TopDecl::Interface(f) if f.name.name == "__StdBusAdapter" => Some(f.id),
            _ => None,
        })
        .expect("the merged stdlib declares the adapter contract");
    let sink = id_of(decl(p, "Sink"));
    assert_eq!(view.typed.conformance(sink, adapter).expect("the view's row").verdict, Ok(()));
    assert!(table.conformance(sink, adapter).is_none(), "the check's bundle does not declare it");
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

/// A bare builtin argument is typed by the signature table lowering
/// reads, so the generic call it feeds binds its parameter instead of
/// staying a hole.
#[test]
fn a_bare_builtin_argument_pins_a_generic_call() {
    let src = r#"
fn first<T>(x: T) -> T { return x; }
fn main() {
    let a = first(len("abc"));
    let b = first(abs(-2.5));
    let c = first(to_string(1));
    let d = first(Float(2));
    let e = first(starts_with("ab", "a"));
    println(a, b, c, d, e);
}

"#;
    let program = hale_syntax::parse_source(src).expect("parses");
    let Ok(s) = Snapshot::from_program(program, Vec::new(), Config::check(true, false)) else {
        panic!("not refused")
    };
    let table = s.demand_typed_bodies().expect("the table");
    let p = s.program().unwrap();
    use hale_syntax::ast::PrimType::{Bool, Float, Int, String};
    let want = [Int, Float, String, Float, Bool];
    let calls = main_calls(p);
    assert_eq!(calls.len(), want.len());
    for (call, want) in calls.into_iter().zip(want) {
        let Some(Typed::Known(row)) = table.generic_call(call) else {
            panic!("typed: {:?}", table.generic_call(call))
        };
        assert_eq!(row.type_args, vec![Ty::Prim(want)]);
    }
}

#[test]
fn generic_locus_members_keep_separate_call_rows_by_body_and_type_arguments() {
    let src = r#"
fn first<T>(x: T) -> T { return x; }
locus Holder<T> {
    params { v: T; copied: T = first(self.v); }
    birth() { println(first(self.v)); }
    fn read() -> T { return first(self.copied); }
    run() {
        println(first(self.v));
        println(first(self.read()));
        let x = self.v;
        println(first(x));
    }
    dissolve() { println(first(self.v)); }
}
fn spawn<T>(x: T) {
    let h: Holder<T> = Holder { v: first(x) };
}
fn main() {
    spawn(42);
    spawn("text");
}
"#;
    let program = hale_syntax::parse_source(src).expect("parses");
    let Ok(s) = Snapshot::from_program(program, Vec::new(), Config::check(true, false)) else {
        panic!("not refused")
    };
    let checked = s.demand_check().expect("checked");
    assert!(checked.diags.is_empty(), "{:?}", checked.diags);
    let table = s.demand_typed_bodies().expect("the table");
    let TopDecl::Locus(holder) = decl(s.program().unwrap(), "Holder") else { unreachable!() };
    let mut bodies = vec![holder.id];
    bodies.extend(holder.members.iter().filter_map(|member| match member {
        LocusMember::Fn(f) => Some(f.id),
        LocusMember::Lifecycle(lc) => Some(lc.id),
        _ => None,
    }));
    let mut calls = 0;
    for body in bodies {
        let rows = table.body(body).expect("body recorded");
        for (site, template) in &rows.generic_calls {
            calls += 1;
            let call = NodeId(*site);
            assert!(matches!(template, Typed::Hole(_)), "template parameter is unbound");
            assert_eq!(table.generic_call_body(call).map(|id| id.0), Some(body.0));
            assert_eq!(table.generic_call_locus(call).map(|id| id.0), Some(holder.id.0));
            for prim in [hale_syntax::ast::PrimType::Int, hale_syntax::ast::PrimType::String] {
                let args = vec![Ty::Prim(prim)];
                let Some(Typed::Known(row)) = table.specialized_generic_call_at(&args, call) else {
                    panic!("{body:?}/{prim:?}: {:?}", table.specialized_generic_call_at(&args, call));
                };
                assert_eq!(row.type_args, args);
                assert_eq!(row.params, args);
                assert!(table.monomorphs().of(row.template, &args).is_some());
            }
        }
    }
    assert_eq!(calls, 7, "params default, birth, method, three run calls, dissolve");
}

#[test]
fn default_call_rows_preserve_one_source_site_for_each_invocation_and_caller_monomorph() {
    let src = r#"
fn first<T>(x: T) -> T { return x; }
fn show(n: String = to_string(first(value))) { println(n); }
fn caller<T>(value: T) { show(); }
fn main() {
    let value = 42;
    show();
    { let value = "text"; show(); }
    caller(7);
    caller("other");
}
"#;
    let program = hale_syntax::parse_source(src).expect("parses");
    let Ok(s) = Snapshot::from_program(program, Vec::new(), Config::check(true, false)) else { panic!("snapshot") };
    assert!(s.demand_check().unwrap().diags.is_empty());
    let table = s.demand_typed_bodies().expect("typed rows");
    let main = table.body(id_of(decl(s.program().unwrap(), "main"))).unwrap();
    assert_eq!(main.default_calls.len(), 2);
    let mut source = None;
    for (evaluation, prim) in main.default_calls.iter().zip([hale_syntax::ast::PrimType::Int, hale_syntax::ast::PrimType::String]) {
        assert_eq!(evaluation.invocations.len(), 1);
        assert_eq!(evaluation.specialization, None);
        let (&site, row) = evaluation.calls.iter().next().unwrap();
        if let Some(id) = source { assert_eq!(site, id); } else { source = Some(site); }
        let Typed::Known(row) = row else { panic!("{row:?}") };
        assert_eq!(row.type_args, vec![Ty::Prim(prim)]);
        assert!(table.monomorphs().of(row.template, &row.type_args).is_some());
    }
    assert_ne!(main.default_calls[0].invocations, main.default_calls[1].invocations);
    let caller = table.body(id_of(decl(s.program().unwrap(), "caller"))).unwrap();
    assert_eq!(caller.default_calls.len(), 3);
    let generic = &caller.default_calls[0];
    assert_eq!(generic.specialization, None);
    assert!(matches!(generic.calls[&source.unwrap()], Typed::Hole(_)));
    for prim in [hale_syntax::ast::PrimType::Int, hale_syntax::ast::PrimType::String] {
        let args = vec![Ty::Prim(prim)];
        let evaluation = caller.default_calls.iter().find(|r| r.specialization.as_ref() == Some(&args)).unwrap();
        assert_eq!(evaluation.invocations, generic.invocations);
        assert!(matches!(table.default_generic_call(&evaluation.invocations, Some(&args), NodeId(source.unwrap())), Some(Typed::Known(row)) if row.type_args == args));
    }
}

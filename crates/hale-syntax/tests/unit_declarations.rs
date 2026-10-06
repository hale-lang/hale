//! GH #1076, step U1: the declaration layer of the unit dialect parses.
//!
//! Every declaration of the committed example (the #1212 comment "The
//! unit dialect, concretely") parses to the tree the plan fixes: a
//! `unit` declaration with its equation, and the scalar body of a
//! `type` declaration with its kind, base, denomination and clauses.
//! The words the dialect recognizes by position (`unit`, `quantity`,
//! `point`, `distinct`, and the clause names) stay identifiers
//! everywhere else, and the duration literal reads as it always did.

use hale_syntax::ast::{
    Expr, Literal, LocusMember, ScalarClause, ScalarDecl, ScalarKind, Stmt, TopDecl,
    TypeDeclBody, TypeExpr, UnitDecl,
};
use hale_syntax::{lex, parse_source, TokenKind};

/// The declarations of the committed example, as written there.
const COMMITTED: &str = "\
// units are edges with rational factors; there is no base unit
unit us  = 1_000 ns;
unit ms  = 1_000 us;
unit s   = 1_000 ms;
unit min = 60 s;

unit KiB = 1024 B;
unit MiB = 1024 KiB;

unit USD = 100 cent;
unit pct = 100 bp;
unit tick;                                   // a unit with no edges is its own component

// a quantity names its node; the built-ins become ordinary declarations
type Duration  = quantity Int in ns;
type Time      = point Duration;
type Bytes     = quantity Int in B;
type Money     = quantity Int in cent;
type Ratio     = quantity Int in bp;
type Tick      = quantity Int in tick;
type Price     = point Tick;

// two origins over one quantity, no affine edges needed
type TempDelta = quantity Int in mK;
type Kelvin    = point TempDelta;
type Celsius   = point TempDelta { origin: 273_150 mK; }

// identities and ranges share the one rule; a width is a range
type OrderId   = distinct Int;
type SeqNo     = distinct Int;
type Session   = distinct Int { range: 0..64; }
type Byte      = Int { range: 0..256; }

// a boundary type carries its narrowing policy
type WireStamp = Time in us { round: floor; }
type Bucket    = quantity Int in 100ms { round: floor; }
type Ledger    = quantity Int in cent { round: half_even; }
";

fn parse(src: &str) -> Vec<TopDecl> {
    match parse_source(src) {
        Ok(p) => p.items,
        Err(d) => panic!("parse failed: {:?}", d.iter().map(|d| &d.message).collect::<Vec<_>>()),
    }
}

fn parse_err(src: &str) -> String {
    match parse_source(src) {
        Ok(_) => panic!("expected a parse error for {src:?}"),
        Err(d) => d.iter().map(|d| d.message.clone()).collect::<Vec<_>>().join("\n"),
    }
}

fn units(items: &[TopDecl]) -> Vec<&UnitDecl> {
    items
        .iter()
        .filter_map(|i| match i {
            TopDecl::Unit(u) => Some(u),
            _ => None,
        })
        .collect()
}

fn scalar<'a>(items: &'a [TopDecl], name: &str) -> &'a ScalarDecl {
    for i in items {
        if let TopDecl::Type(t) = i {
            if t.name.name == name {
                match &t.body {
                    TypeDeclBody::Scalar(s) => return s,
                    other => panic!("`{name}` is not a scalar: {other:?}"),
                }
            }
        }
    }
    panic!("no type `{name}`")
}

/// `(num, den, target)` of a unit's equation.
fn equation(u: &UnitDecl) -> Option<(u64, u64, Option<&str>)> {
    u.equation
        .as_ref()
        .map(|e| (e.num, e.den, e.target.as_ref().map(|t| t.name.as_str())))
}

fn base_name(s: &ScalarDecl) -> String {
    match &s.base {
        TypeExpr::Primitive(p, _) => format!("{p:?}"),
        TypeExpr::Named { path, .. } => path.segments[0].name.clone(),
        other => panic!("unexpected base {other:?}"),
    }
}

fn denom(s: &ScalarDecl) -> Option<(u64, &str)> {
    s.denom.as_ref().map(|d| (d.multiple, d.unit.name.as_str()))
}

#[test]
fn the_committed_units_parse_to_their_equations() {
    let items = parse(COMMITTED);
    let us = units(&items);
    let got: Vec<_> = us.iter().map(|u| (u.name.name.as_str(), equation(u))).collect();
    assert_eq!(
        got,
        vec![
            ("us", Some((1_000, 1, Some("ns")))),
            ("ms", Some((1_000, 1, Some("us")))),
            ("s", Some((1_000, 1, Some("ms")))),
            ("min", Some((60, 1, Some("s")))),
            ("KiB", Some((1024, 1, Some("B")))),
            ("MiB", Some((1024, 1, Some("KiB")))),
            ("USD", Some((100, 1, Some("cent")))),
            ("pct", Some((100, 1, Some("bp")))),
            ("tick", None),
        ]
    );
}

#[test]
fn the_committed_scalars_parse_to_their_shapes() {
    let items = parse(COMMITTED);
    use ScalarKind::*;
    let shapes = [
        ("Duration", Some(Quantity), "Int", Some((1, "ns"))),
        ("Time", Some(Point), "Duration", None),
        ("Bytes", Some(Quantity), "Int", Some((1, "B"))),
        ("Money", Some(Quantity), "Int", Some((1, "cent"))),
        ("Ratio", Some(Quantity), "Int", Some((1, "bp"))),
        ("Tick", Some(Quantity), "Int", Some((1, "tick"))),
        ("Price", Some(Point), "Tick", None),
        ("TempDelta", Some(Quantity), "Int", Some((1, "mK"))),
        ("Kelvin", Some(Point), "TempDelta", None),
        ("Celsius", Some(Point), "TempDelta", None),
        ("OrderId", Some(Distinct), "Int", None),
        ("SeqNo", Some(Distinct), "Int", None),
        ("Session", Some(Distinct), "Int", None),
        ("Byte", None, "Int", None),
        ("WireStamp", None, "Time", Some((1, "us"))),
        ("Bucket", Some(Quantity), "Int", Some((100, "ms"))),
        ("Ledger", Some(Quantity), "Int", Some((1, "cent"))),
    ];
    for (name, kind, base, den) in shapes {
        let s = scalar(&items, name);
        assert_eq!(s.kind, kind, "{name}: kind");
        assert_eq!(base_name(s), base, "{name}: base");
        assert_eq!(denom(s), den, "{name}: denomination");
    }

    let clause = |name: &str| {
        let s = scalar(&items, name);
        assert_eq!(s.clauses.len(), 1, "{name}: one clause");
        s.clauses[0].clone()
    };
    match clause("Celsius") {
        ScalarClause::Origin { value, unit, .. } => {
            assert_eq!((value, unit.name.as_str()), (273_150, "mK"));
        }
        other => panic!("Celsius: {other:?}"),
    }
    for (name, hi) in [("Session", 64), ("Byte", 256)] {
        match clause(name) {
            ScalarClause::Range { lo, hi: h, inclusive, .. } => {
                assert!(matches!(lo, Expr::Literal(Literal::Int(0), _)), "{name}: lo");
                assert!(matches!(h, Expr::Literal(Literal::Int(n), _) if n == hi), "{name}: hi");
                assert!(!inclusive, "{name}: exclusive");
            }
            other => panic!("{name}: {other:?}"),
        }
    }
    for (name, policy) in [("WireStamp", "floor"), ("Bucket", "floor"), ("Ledger", "half_even")] {
        match clause(name) {
            ScalarClause::Round { policy: p, .. } => assert_eq!(p.name, policy, "{name}"),
            other => panic!("{name}: {other:?}"),
        }
    }
    // Everything without a kind word, an `in` or a block is not a scalar.
    for name in ["Duration", "Time", "Kelvin", "OrderId"] {
        assert!(scalar(&items, name).clauses.is_empty());
    }
}

#[test]
fn a_type_with_none_of_the_three_is_still_an_alias() {
    let items = parse("type A = Int;\ntype P = point;\ntype Q = quantity<Int>;\n");
    for i in &items {
        let TopDecl::Type(t) = i else { panic!() };
        assert!(matches!(t.body, TypeDeclBody::Alias(_)), "{}: {:?}", t.name.name, t.body);
    }
}

#[test]
fn the_two_spellings_of_a_unit_equation_agree() {
    let spaced = parse("unit us = 1_000 ns;\nunit KiB = 1024 B;\nunit x = 3 tick;\n");
    let adjacent = parse("unit us = 1_000ns;\nunit KiB = 1024B;\nunit x = 3tick;\n");
    let eqs = |items: &[TopDecl]| -> Vec<_> {
        units(items).iter().map(|u| (u.name.name.clone(), equation(u).map(|(n, d, t)| (n, d, t.map(String::from))))).collect()
    };
    assert_eq!(eqs(&spaced), eqs(&adjacent));
    assert_eq!(eqs(&spaced)[0].1, Some((1_000, 1, Some("ns".to_string()))));
}

#[test]
fn a_unit_against_the_pure_number() {
    let items = parse("unit pct = 1/100;\nunit dozen = 12;\nunit inch = 127/5 mm;\n");
    let u = units(&items);
    assert_eq!(equation(u[0]), Some((1, 100, None)));
    assert_eq!(equation(u[1]), Some((12, 1, None)));
    assert_eq!(equation(u[2]), Some((127, 5, Some("mm"))));
}

#[test]
fn a_denomination_may_be_written_apart() {
    let items = parse("type B1 = quantity Int in 100ms;\ntype B2 = quantity Int in 100 ms;\n");
    assert_eq!(denom(scalar(&items, "B1")), Some((100, "ms")));
    assert_eq!(denom(scalar(&items, "B2")), Some((100, "ms")));
}

#[test]
fn zero_is_refused_with_its_own_message() {
    for src in ["unit z = 0 ns;", "unit z = 0ns;", "unit z = 1/0;", "unit z = 0/3;"] {
        let msg = parse_err(src);
        assert!(msg.contains("must be positive: zero relates no unit to another"), "{src}: {msg}");
    }
    let msg = parse_err("type Z = quantity Int in 0 ms;");
    assert!(msg.contains("must be positive"), "{msg}");
}

#[test]
fn a_repeated_clause_is_refused() {
    let msg = parse_err("type R = Int { range: 0..4; range: 0..8; }");
    assert!(msg.contains("the `range` clause appears twice"), "{msg}");
}

#[test]
fn an_unknown_clause_is_refused_listing_the_three() {
    let msg = parse_err("type R = Int { width: 8; }");
    assert!(
        msg.contains("unknown scalar clause `width`: the clauses are `range`, `round` and `origin`"),
        "{msg}"
    );
}

/// The words the dialect recognizes by position, and the clause names,
/// stay ordinary identifiers: a local, a field, a parameter, a fn and a
/// method may each be named by any of them.
#[test]
fn the_contextual_words_stay_identifiers() {
    for w in ["unit", "quantity", "point", "distinct", "range", "round", "origin", "split"] {
        let src = format!(
            "type Holder {{ {w}: Int; }}\n\
             fn {w}({w}: Int) -> Int {{\n    let {w} = {w} + 1;\n    {w}\n}}\n\
             locus L {{\n    fn {w}() -> Int {{ 1 }}\n}}\n\
             fn user(h: Holder, l: L) -> Int {{\n    let {w} = h.{w};\n    {w} + l.{w}() + {w}({w})\n}}\n"
        );
        let items = parse(&src);
        assert!(units(&items).is_empty(), "{w}");
        assert_eq!(items.len(), 4, "{w}");
        assert!(matches!(&items[0], TopDecl::Type(t) if matches!(t.body, TypeDeclBody::Struct(_))));
        let TopDecl::Locus(l) = &items[2] else { panic!("{w}: locus") };
        assert!(l.members.iter().any(|m| matches!(m, LocusMember::Fn(f) if f.name.name == w)));
    }
}

fn main_body(items: &[TopDecl]) -> &[Stmt] {
    for i in items {
        if let TopDecl::Fn(f) = i {
            return &f.body.stmts;
        }
    }
    panic!("no fn")
}

fn let_value(s: &Stmt) -> &Expr {
    match s {
        Stmt::Let { value, .. } => value,
        other => panic!("not a let: {other:?}"),
    }
}

#[test]
fn in_and_split_are_ordinary_method_calls() {
    let items = parse("fn f(d: Int) {\n    let a = d.in(s);\n    let b = d.split(s);\n    let c = d.in(100ms);\n}\n");
    let body = main_body(&items);
    for (stmt, method) in body.iter().zip(["in", "split", "in"]) {
        match let_value(stmt) {
            Expr::Call { callee, args, .. } => {
                let Expr::Field { name, .. } = &**callee else { panic!("{method}: {callee:?}") };
                assert_eq!(name.name, method);
                assert_eq!(args.len(), 1);
            }
            other => panic!("{method}: not a method call: {other:?}"),
        }
    }
}

#[test]
fn a_quantity_literal_is_its_magnitude_and_its_unit() {
    let items = parse("fn f() {\n    let a = 3bp;\n    let b = 1_250_000USD;\n    let c = 2EUR;\n}\n");
    let body = main_body(&items);
    let got: Vec<_> = body
        .iter()
        .map(|s| match let_value(s) {
            Expr::Literal(Literal::Quantity { value, unit }, _) => (*value, unit.clone()),
            other => panic!("not a quantity literal: {other:?}"),
        })
        .collect();
    assert_eq!(got, vec![(3, "bp".into()), (1_250_000, "USD".into()), (2, "EUR".into())]);
}

#[test]
fn a_duration_literal_reads_as_it_always_did() {
    let items = parse("fn f() {\n    let a = 500ms;\n}\n");
    assert!(matches!(let_value(&main_body(&items)[0]), Expr::Literal(Literal::Duration(500_000_000), _)));
    // The token keeps what was written beside the nanoseconds.
    let toks = lex("500ms 1_000ns").unwrap();
    assert_eq!(
        toks[0].kind,
        TokenKind::DurationLit { ns: 500_000_000, spelled: Some((500, "ms".into())) }
    );
    assert_eq!(toks[1].kind, TokenKind::DurationLit { ns: 1_000, spelled: Some((1_000, "ns".into())) });
    // An exponent is still an exponent.
    assert_eq!(lex("1e5").unwrap()[0].kind, TokenKind::FloatLit(1e5));
    assert_eq!(lex("1E-3").unwrap()[0].kind, TokenKind::FloatLit(1e-3));
}

#[test]
fn a_float_with_a_suffix_is_still_an_error() {
    parse_err("fn f() {\n    let a = 1.5kg;\n}\n");
    parse_err("fn f() {\n    let a = 1.5s;\n}\n");
}

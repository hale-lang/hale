//! GH #1995 S1: open-ended slices. `s[a..]`, `s[..b]`, `s[..=b]` and
//! `s[..]` parse in an index's brackets and nowhere else, as the closed
//! range they read as (`0` for a missing start, an end the slice clamps
//! to the receiver's length), so every later pass sees an ordinary
//! `Expr::Range`; `hale fmt` keeps them as written.

use hale_syntax::ast::*;
use hale_syntax::fmt::format_source;
use hale_syntax::parse_source;

/// The index expression of `let x = <e>;`, the first statement of `main`.
fn index_of(e: &str) -> Expr {
    let src = format!("fn main() {{\n    let x = {e};\n}}\n");
    let p = parse_source(&src).unwrap_or_else(|d| panic!("`{e}` parses: {d:?}"));
    let TopDecl::Fn(f) = &p.items[0] else { panic!("a fn") };
    let Stmt::Let { value, .. } = &f.body.stmts[0] else { panic!("a let") };
    let Expr::Index { index, .. } = value else { panic!("`{e}` is an index") };
    (**index).clone()
}

fn int(e: &Expr) -> Option<i64> {
    match e {
        Expr::Literal(Literal::Int(n), _) => Some(*n),
        _ => None,
    }
}

#[test]
fn an_open_end_is_a_closed_range_to_the_clamp() {
    let Expr::Range { lo, hi, inclusive, .. } = index_of("s[a..]") else { panic!("a range") };
    assert!(matches!(*lo, Expr::Ident(ref i) if i.name == "a"));
    assert_eq!(int(&hi), Some(i64::MAX));
    assert!(!inclusive);
    let sp = hi.span();
    assert_eq!(sp.start, sp.end, "the synthesized end has no text");
}

#[test]
fn an_open_start_is_zero() {
    let Expr::Range { lo, hi, inclusive, .. } = index_of("s[..b]") else { panic!("a range") };
    assert_eq!(int(&lo), Some(0));
    assert!(matches!(*hi, Expr::Ident(ref i) if i.name == "b"));
    assert!(!inclusive);
    let Expr::Range { inclusive, .. } = index_of("s[..=b]") else { panic!("a range") };
    assert!(inclusive, "`..=` keeps its end");
    let Expr::Range { lo, hi, .. } = index_of("s[..]") else { panic!("a range") };
    assert_eq!((int(&lo), int(&hi)), (Some(0), Some(i64::MAX)), "`s[..]` is the whole string");
}

#[test]
fn a_bound_is_any_expression() {
    let Expr::Range { lo, hi, .. } = index_of("s[(at + 1)..]") else { panic!("a range") };
    assert!(matches!(*lo, Expr::Binary { .. }), "{lo:?}");
    assert_eq!(int(&hi), Some(i64::MAX));
    let Expr::Range { lo, hi, .. } = index_of("f(x)[..n - 1]") else { panic!("a range") };
    assert_eq!(int(&lo), Some(0));
    assert!(matches!(*hi, Expr::Binary { .. }), "{hi:?}");
    // a closed range and a plain index parse as before
    let Expr::Range { lo, hi, .. } = index_of("s[1..3]") else { panic!("a range") };
    assert_eq!((int(&lo), int(&hi)), (Some(1), Some(3)));
    assert_eq!(int(&index_of("xs[2]")), Some(2));
}

#[test]
fn an_inclusive_range_needs_its_end() {
    for e in ["s[a..=]", "s[..=]"] {
        let src = format!("fn main() {{\n    let x = {e};\n}}\n");
        let err = parse_source(&src).expect_err("`..=` with no end is refused");
        assert!(err[0].message.contains("`..=` needs an end"), "{e}: {}", err[0].message);
    }
}

#[test]
fn an_open_range_outside_an_index_is_still_a_parse_error() {
    for body in [
        "let r = 3..;",
        "let r = ..3;",
        "for i in 0.. { }",
        "for i in ..3 { }",
        "f(a..);",
    ] {
        let src = format!("fn main() {{\n    {body}\n}}\n");
        assert!(parse_source(&src).is_err(), "`{body}` must not parse");
    }
}

#[test]
fn fmt_keeps_an_open_slice_as_written() {
    let src = "fn main() {\n    let s = \"hello world\";\n    let a = s[6..];\n    let b = s[..5];\n    let c = s[..=4];\n    let d = s[..];\n    let e = s[(a + 1)..];\n}\n";
    let out = format_source(src).expect("formats");
    assert_eq!(out, src, "already canonical, so unchanged");
    let loose = "fn main() {\n    let a = s[ 6 .. ];\n    let b = s[ .. 5 ];\n    let d = s[ .. ];\n}\n";
    let out = format_source(loose).expect("formats");
    assert_eq!(out, "fn main() {\n    let a = s[6..];\n    let b = s[..5];\n    let d = s[..];\n}\n");
    assert_eq!(format_source(&out).expect("formats"), out, "a fixed point");
}

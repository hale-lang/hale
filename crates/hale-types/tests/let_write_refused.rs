//! GH #2022: a field or element write through a `let` binding is an
//! error (spec/types.md § Mutability).
//!
//! `let p = Point { … }; p.x = 99;` compiled and changed the value: the
//! check refused only a bare reassignment of a non-`mut` local, while a
//! `push` into a `let` `bounded` list was refused outright. The ruling:
//! `mut` gates every write into a `let` binding's value, at any depth.
//! v0.35 warned (`deprecated:`, GH #2055); the pre-1.0 policy
//! (STABILITY.md) made it an error without waiting for a further minor.

#[path = "support/entries.rs"]
mod entries;

/// `(is_error, message)` for every diagnostic of `body` inside a `main`
/// beside the declarations every case uses.
fn diags(body: &str) -> Vec<(bool, String)> {
    let src = format!(
        r#"
type Point {{ x: Int; y: Int; }}
type Line {{ a: Point; b: Point; }}
type Shape = enum {{ Dot(Point), Empty }};
locus Counter {{ params {{ n: Int = 0; }} }}
locus Cell<T> {{ params {{ value: T; }} }}
type IntCell = Cell<Int>;

fn bump(p: Point) -> Int {{
    p.x = p.x + 1;
    return p.x;
}}

fn main() {{
{body}
}}
"#
    );
    let prog = hale_syntax::parse_source(&src).unwrap_or_else(|e| panic!("parse failed: {:?}\n{}", e, src));
    entries::check_program(&prog).into_iter().map(|d| (d.is_error(), d.message)).collect()
}

fn let_writes(ds: &[(bool, String)]) -> Vec<&(bool, String)> {
    ds.iter().filter(|(_, m)| m.contains("declared with `let`")).collect()
}

#[test]
fn a_write_through_a_let_is_an_error() {
    for (body, written, name) in [
        ("    let p = Point { x: 1, y: 2 };\n    p.x = 99;\n    println(p.x);", "p.x", "p"),
        ("    let p = Point { x: 1, y: 2 };\n    p.x += 1;\n    println(p.x);", "p.x", "p"),
        ("    let xs = [1, 2, 3];\n    xs[0] = 7;\n    println(xs[0]);", "xs[..]", "xs"),
        (
            "    let l = Line { a: Point { x: 1, y: 2 }, b: Point { x: 3, y: 4 } };\n    l.a.y = 5;\n    println(l.a.y);",
            "l.a.y",
            "l",
        ),
        (
            "    let ls = [Point { x: 1, y: 2 }];\n    ls[0].x = 5;\n    println(ls[0].x);",
            "ls[..].x",
            "ls",
        ),
        ("    let (p, n) = (Point { x: 1, y: 2 }, 3);\n    p.y = n;\n    println(p.y);", "p.y", "p"),
    ] {
        let ds = diags(body);
        let hits = let_writes(&ds);
        assert_eq!(hits.len(), 1, "`{body}` is refused once: {ds:?}");
        let (is_error, msg) = hits[0];
        assert!(is_error, "a write through a `let` is an error: {msg}");
        assert!(!msg.starts_with("deprecated"), "no longer a deprecation: {msg}");
        assert!(msg.contains(&format!("`{written}` writes into `{name}`")), "{msg}");
        assert!(msg.contains(&format!("declare it `let mut {name}`")), "{msg}");
        assert!(!msg.contains("v0.36"), "names no future release: {msg}");
        assert_eq!(ds.iter().filter(|(e, _)| *e).count(), 1, "nothing else: {ds:?}");
    }
}

#[test]
fn what_the_rule_does_not_reach_stays_silent() {
    for body in [
        // declared `let mut`
        "    let mut p = Point { x: 1, y: 2 };\n    p.x = 99;\n    let mut xs = [1, 2];\n    xs[0] = 7;\n    println(p.x, xs[0]);",
        // a `for` element: no `mut` spelling
        "    for e in [Point { x: 1, y: 2 }] {\n        e.x = 3;\n        println(e.x);\n    }",
        // a `match` payload: no `mut` spelling
        "    let s = Shape::Dot(Point { x: 1, y: 2 });\n    match s {\n        Shape::Dot(q) -> { q.x = 3; println(q.x); },\n        Shape::Empty -> { },\n    }",
        // a locus handle: the write is the locus's state
        "    let c = Counter { };\n    c.n = 4;\n    println(c.n);",
        // a generic locus's handle, and one through an alias (GH #2055 review)
        "    let c: Cell<Int> = Cell { value: 1 };\n    c.value = 2;\n    println(c.value);",
        "    let c: IntCell = Cell { value: 1 };\n    c.value = 2;\n    println(c.value);",
        // a locus handle on the way to the written place: an element of
        // an array of loci, and a locus field of a record
        "    let cs = [Counter { }, Counter { }];\n    cs[0].n = 4;\n    println(cs[0].n);",
        // a parameter (in `bump`, above) and a reassignment's own rule
        "    println(bump(Point { x: 1, y: 2 }));",
    ] {
        let ds = diags(body);
        assert!(let_writes(&ds).is_empty(), "`{body}` is no write through a `let`: {ds:?}");
        assert!(!ds.iter().any(|(e, _)| *e), "and no error: {ds:?}");
    }
}

/// The bare reassignment of a `let` stays the error it was (m50).
#[test]
fn reassigning_a_let_is_still_an_error() {
    let ds = diags("    let n = 1;\n    n = 2;\n    println(n);");
    assert!(ds.iter().any(|(e, m)| *e && m.contains("cannot assign to `n`")), "{ds:?}");
    assert!(let_writes(&ds).is_empty(), "{ds:?}");
}

/// A binding whose type the check could not settle may hold a locus (its
/// initializer names a declaration the checked files do not hold, as
/// when one file of a seed is checked alone), so the write is not
/// refused: the initializer's own error is the one reported.
#[test]
fn a_binding_of_unsettled_type_is_not_refused() {
    let ds = diags("    let q = not_declared_here();\n    q.x = 1;\n    println(q.x);");
    assert!(let_writes(&ds).is_empty(), "{ds:?}");
    assert!(ds.iter().any(|(e, _)| *e), "the unbound call is still an error: {ds:?}");
}

/// A generic fn's binding of its parameter's type is `Unknown` in the
/// template's walk, which leaves the write alone (the argument may be a
/// locus); the walk of the monomorph, with `T` bound to a record, refuses
/// it, once however many monomorphs reach it (codex review of #2079).
#[test]
fn a_write_through_a_let_in_a_generic_fn_is_refused_at_its_record_monomorph() {
    let src = r#"
type P { x: Int; }
type Q { x: Int; }
locus L { params { x: Int = 0; } }
fn touch<T>(x: T) -> T {
    let q = x;
    q.x = 2;
    return q;
}
fn own<T>(x: T) -> Int {
    let p = P { x: 1 };
    p.x = 3;
    return p.x;
}
fn main() {
    println(touch(P { x: 1 }).x);
    println(touch(Q { x: 1 }).x);
    println(own(1) + own("a"));
}
"#;
    let prog = hale_syntax::parse_source(src).unwrap_or_else(|e| panic!("parse failed: {:?}", e));
    let ds: Vec<(bool, String)> = entries::check_program(&prog).into_iter().map(|d| (d.is_error(), d.message)).collect();
    let hits = let_writes(&ds);
    assert_eq!(hits.len(), 2, "`q.x` once for both monomorphs, `p.x` once: {ds:?}");
    assert!(hits.iter().all(|(e, _)| *e), "{ds:?}");
    assert!(hits.iter().any(|(_, m)| m.contains("`q.x` writes into `q`")), "{ds:?}");
    assert!(hits.iter().any(|(_, m)| m.contains("`p.x` writes into `p`")), "{ds:?}");

    // At a locus monomorph the write is the locus's state.
    let src = r#"
locus L { params { x: Int = 0; } }
fn touch<T>(x: T) -> Int {
    let q = x;
    q.x = 2;
    return q.x;
}
fn main() { println(touch(L { })); }
"#;
    let prog = hale_syntax::parse_source(src).unwrap_or_else(|e| panic!("parse failed: {:?}", e));
    let ds: Vec<(bool, String)> = entries::check_program(&prog).into_iter().map(|d| (d.is_error(), d.message)).collect();
    assert!(let_writes(&ds).is_empty(), "{ds:?}");
}

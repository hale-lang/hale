//! GH #2022: a field or element write through a `let` binding is
//! deprecated (spec/types.md § Mutability, STABILITY.md § Deprecation).
//!
//! `let p = Point { … }; p.x = 99;` compiled and changed the value: the
//! check refused only a bare reassignment of a non-`mut` local, while a
//! `push` into a `let` `bounded` list was refused outright. The ruling:
//! `mut` gates every write into a `let` binding's value, at any depth.
//! This minor warns (`deprecated:`, never an error); the next refuses.

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

fn deprecations(ds: &[(bool, String)]) -> Vec<&(bool, String)> {
    ds.iter().filter(|(_, m)| m.contains("declared with `let`")).collect()
}

#[test]
fn a_write_through_a_let_warns_and_is_no_error() {
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
        let hits = deprecations(&ds);
        assert_eq!(hits.len(), 1, "`{body}` warns once: {ds:?}");
        let (is_error, msg) = hits[0];
        assert!(!is_error, "a deprecation is never an error: {msg}");
        assert!(msg.starts_with("deprecated: "), "{msg}");
        assert!(msg.contains(&format!("`{written}` writes into `{name}`")), "{msg}");
        assert!(msg.contains(&format!("declare it `let mut {name}`")), "{msg}");
        assert!(msg.contains("v0.36"), "the warning names the release that refuses it: {msg}");
        assert!(!ds.iter().any(|(e, _)| *e), "nothing else: {ds:?}");
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
        // a parameter (in `bump`, above) and a reassignment's own rule
        "    println(bump(Point { x: 1, y: 2 }));",
    ] {
        let ds = diags(body);
        assert!(deprecations(&ds).is_empty(), "`{body}` is no write through a `let`: {ds:?}");
        assert!(!ds.iter().any(|(e, _)| *e), "and no error: {ds:?}");
    }
}

/// The bare reassignment of a `let` stays the error it was (m50).
#[test]
fn reassigning_a_let_is_still_an_error() {
    let ds = diags("    let n = 1;\n    n = 2;\n    println(n);");
    assert!(ds.iter().any(|(e, m)| *e && m.contains("cannot assign to `n`")), "{ds:?}");
    assert!(deprecations(&ds).is_empty(), "{ds:?}");
}

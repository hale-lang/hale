//! Which fns `ownership::fresh_factories` classifies as fresh
//! factories, pinned on probes (F.40 phase 1.2c).
//!
//! The producer's escape walk once ended in a catch-all that searched a
//! node's Debug rendering for `name: "<binding>"`, so the binding's
//! name spelled anywhere in the node (a field or method name, a
//! struct-init name, a pattern, a path segment) answered "escapes".
//! The walk now matches every expression form and answers by
//! identifier. The corpus has no fn whose answer that correction
//! moves, so these probes pin the answers it gives.

use hale_types::ownership::fresh_factories;

/// Each probe is `(what, body of make, make is a fresh factory)` over
/// `locus Node { params { n: Int = 0; } }` and `fn make() -> Node { … }`.
#[test]
fn the_correction_on_probes() {
    let probes: &[(&str, &str, bool)] = &[
        (
            "a receiver use inside a tuple",
            "let n = Node { n: 1 }; let t = (n.n, 2); return n;",
            true,
        ),
        (
            "another value's field that spells the binding, inside an `if`",
            "let n = Node { n: 1 }; let o = Node { n: 2 }; let v = if true { o.n } else { 0 }; return n;",
            true,
        ),
        (
            "a struct-init name that spells the binding, inside a tuple",
            "let n = Node { n: 1 }; let t = (Node { n: 3 }, 1); return n;",
            true,
        ),
        (
            "a method name that spells the binding, inside a range",
            "let n = Node { n: 1 }; let o = Node { n: 2 }; for i in 0..o.n() { } return n;",
            true,
        ),
        (
            "the binding passed as an argument inside a tuple: a real escape",
            "let n = Node { n: 1 }; let t = (keep(n), 1); return n;",
            false,
        ),
        (
            "the binding returned from an `if` arm into another binding: a real escape",
            "let n = Node { n: 1 }; let m = if true { n } else { Node { n: 2 } }; return n;",
            false,
        ),
        (
            "a `match` arm that rebinds the name",
            "let n = Node { n: 1 }; let v = match 3 { n -> 1, _ -> 2 }; return n;",
            false,
        ),
        (
            "a statement form the walk does not model, inside an `if` expression",
            "let n = Node { n: 1 }; let v = if true { yield; 1 } else { 2 }; return n;",
            false,
        ),
    ];
    let mut wrong = Vec::new();
    for (what, body, expected) in probes {
        let src = format!(
            "locus Node {{ params {{ n: Int = 0; }} fn n() -> Int {{ return 0; }} }}\n\
             fn keep(x: Node) -> Int {{ return 0; }}\n\
             fn make() -> Node {{ {body} }}\n"
        );
        let program = hale_syntax::parse_source(&src)
            .unwrap_or_else(|d| panic!("probe `{what}` does not parse: {d:?}"));
        let fresh = fresh_factories(&[&program], &[]).contains_key("make");
        if fresh != *expected {
            wrong.push(format!("{what}: {fresh} (expected {expected})"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

//! GH #732: interface methods declare `fallible(E)`. Conformance is
//! checked at check time and the diagnostic names both signatures:
//! an infallible method satisfies a fallible interface method, a
//! fallible one never satisfies an infallible one, and the error
//! types must be the same type. What a call through the interface
//! computes is tested in `tests/hale/interface_fallible_test.hl`.

use hale_syntax::parse_source;
use hale_types::symbol::Bundle;

fn diags(src: &str) -> Vec<String> {
    let program = parse_source(src).expect("parse");
    let mut programs: std::collections::BTreeMap<
        String,
        &hale_syntax::ast::Program,
    > = std::collections::BTreeMap::new();
    programs.insert("test.hl".to_string(), &program);
    let bundle = Bundle::new(programs);
    let (scope, mut ds) = hale_types::resolve::build_top_scope(&bundle);
    ds.extend(hale_types::check::check_bundle(&bundle, &scope, true));
    ds.iter().map(|d| d.message.clone()).collect()
}

const TYPES: &str = r#"
    type E { kind: String = ""; }
    type F { kind: String = ""; }
"#;

#[test]
fn a_fallible_method_does_not_satisfy_an_infallible_one() {
    let src = format!(
        "{TYPES}{}",
        r#"
        interface Store { fn put(k: String) -> Int; }
        locus A { fn put(k: String) -> Int fallible(E) { return 1; } }
        fn one(s: Store) -> Int { return s.put("x"); }
        fn main() { let a = A { }; println(one(a)); }
    "#
    );
    let ds = diags(&src);
    assert!(
        ds.iter().any(|m| m.contains("locus `A` method `put` is fallible where the interface's is not")
            && m.contains("interface `Store` declares `fn put(String) -> Int`")
            && m.contains("locus declares `fn put(String) -> Int fallible(E)`")),
        "{ds:#?}"
    );
}

#[test]
fn a_different_error_type_is_rejected() {
    let src = format!(
        "{TYPES}{}",
        r#"
        interface Store { fn put(k: String) -> Int fallible(E); }
        locus B { fn put(k: String) -> Int fallible(F) { return 1; } }
        fn two(s: Store) -> Int { return s.put("x") or 0; }
        fn main() { let b = B { }; println(two(b)); }
    "#
    );
    let ds = diags(&src);
    assert!(
        ds.iter().any(|m| m.contains("locus `B` method `put` declares a different error type")
            && m.contains("interface `Store` declares `fn put(String) -> Int fallible(E)`")
            && m.contains("locus declares `fn put(String) -> Int fallible(F)`")),
        "{ds:#?}"
    );
}

#[test]
fn an_infallible_method_satisfies_a_fallible_one() {
    let src = format!(
        "{TYPES}{}",
        r#"
        interface Store { fn put(k: String) -> Int fallible(E); }
        locus C { fn put(k: String) -> Int { return 1; } }
        locus D { fn put(k: String) -> Int fallible(E) { return 2; } }
        fn three(s: Store) -> Int { return s.put("x") or 0; }
        fn main() { let c = C { }; let d = D { }; println(three(c)); println(three(d)); }
    "#
    );
    let ds = diags(&src);
    assert!(ds.is_empty(), "{ds:#?}");
}

#[test]
fn an_unaddressed_call_through_a_typed_interface_is_a_check_error() {
    let src = format!(
        "{TYPES}{}",
        r#"
        interface Store { fn put(k: String) -> Int fallible(E); }
        locus D { fn put(k: String) -> Int fallible(E) { return 2; } }
        locus H { params { s: Store = D { }; } fn go() -> Int { return self.s.put("x"); } }
        fn main() { let h = H { }; println(h.go()); }
    "#
    );
    let ds = diags(&src);
    assert!(ds.iter().any(|m| m.contains("error not addressed")), "{ds:#?}");
}

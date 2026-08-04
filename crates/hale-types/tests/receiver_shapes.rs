//! #382 receiver-typing root fix — the EFFECT SYSTEM's negative
//! controls.
//!
//! Four receiver shapes used to land as untyped unresolved call
//! edges that `@effects(none:)` (and every `@no_*` certificate)
//! silently dropped: a fn could reach a declared carrier through
//! any of them and still certify. The summarizer now types those
//! receivers, so each shape is a resolved edge the engine walks.
//! These are the audit's four repro programs as standing negative
//! controls — a checker that cannot fail proves nothing.

use hale_syntax::parse_source;

fn errs(src: &str) -> Vec<String> {
    let program = parse_source(src).expect("parse");
    hale_types::check_program(&program)
        .into_iter()
        .map(|d| d.message)
        .collect()
}

const CARRIER: &str = r#"
    effect money;
    locus B {
        @effects(is: {money})
        fn work(n: Int) -> Int { return n; }
    }
    locus Mid { params { inner: B = B { }; } }
    fn make_b() -> B { return B { }; }
"#;

fn fires(body: &str, shape: &str) {
    let src = format!(
        "{CARRIER}\
         locus A {{\n\
             params {{ mid: Mid = Mid {{ }}; }}\n\
             @effects(none: {{money}})\n\
             fn go(n: Int) -> Int {{ {body} }}\n\
         }}\n\
         fn main() {{ println(A {{ }}.go(1)); }}"
    );
    let ds = errs(&src);
    assert!(
        ds.iter().any(|m| m.contains("effect assertion violated")),
        "{}: reaching the carrier must violate: {:?}",
        shape,
        ds
    );
}

#[test]
fn a_literal_receiver_carrier_fires() {
    fires("return B { }.work(n);", "struct-literal receiver");
}

#[test]
fn a_chained_field_receiver_carrier_fires() {
    fires("return self.mid.inner.work(n);", "chained field receiver");
}

#[test]
fn a_call_result_receiver_carrier_fires() {
    fires(
        "let b = make_b(); return b.work(n);",
        "call-result receiver",
    );
}

#[test]
fn a_branch_valued_receiver_carrier_fires() {
    fires(
        "let b = if n > 0 { B { } } else { B { } }; return b.work(n);",
        "branch-valued receiver",
    );
}

/// The control: the same shapes reaching a NON-carrier stay
/// certifiable — typing receivers must not turn every method call
/// into a violation.
#[test]
fn a_typed_receiver_to_a_clean_locus_still_certifies() {
    let src = r#"
        effect money;
        locus C { fn calc(n: Int) -> Int { return n + 1; } }
        locus A {
            @effects(none: {money})
            fn go(n: Int) -> Int { return C { }.calc(n); }
        }
        fn main() { println(A { }.go(1)); }
    "#;
    let ds = errs(src);
    assert!(
        !ds.iter().any(|m| m.contains("effect assertion violated")),
        "a clean resolved path must certify: {:?}",
        ds
    );
}

/// The wrapper variant (the follow-up review's counterexample) —
/// the carrier is one resolved hop past the newly-typed receiver.
#[test]
fn a_wrapper_behind_a_literal_receiver_fires() {
    let src = r#"
        effect money;
        @effects(is: {money})
        fn charge(n: Int) -> Int { return n; }
        locus Bridge { fn hop(n: Int) -> Int { return charge(n); } }
        locus A {
            @effects(none: {money})
            fn go(n: Int) -> Int { return Bridge { }.hop(n); }
        }
        fn main() { println(A { }.go(1)); }
    "#;
    let ds = errs(src);
    assert!(
        ds.iter().any(|m| m.contains("effect assertion violated")),
        "the carrier one wrapper hop away must violate: {:?}",
        ds
    );
}

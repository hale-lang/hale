//! GH #241: diagnostic-quality checks — user errors that
//! previously escaped to spanless codegen internal errors now
//! die at check phase, and typo diags carry did-you-mean hints.

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

#[test]
fn printing_a_struct_is_a_check_error() {
    let src = r#"
        type P { x: Int = 0; }
        fn main() {
            let p = P { x: 1 };
            println("p=", p);
        }
    "#;
    let ds = diags(src);
    assert!(
        ds.iter().any(|m| m.contains("cannot render") && m.contains("`P`")),
        "expected printable diag; got: {:?}",
        ds
    );
}

#[test]
fn string_plus_struct_is_a_check_error() {
    let src = r#"
        type P { x: Int = 0; }
        fn main() {
            let p = P { x: 1 };
            let s = "v: " + p;
            println(s);
        }
    "#;
    let ds = diags(src);
    assert!(
        !ds.is_empty(),
        "expected a diag for String + struct; got none"
    );
}

#[test]
fn printing_an_enum_stays_legal() {
    let src = r#"
        type Light = enum { Red, Green };
        fn main() {
            let l = Light::Red;
            println("light: ", l);
        }
    "#;
    let ds = diags(src);
    assert!(
        !ds.iter().any(|m| m.contains("cannot render")),
        "enum printing must stay legal; got: {:?}",
        ds
    );
}

#[test]
fn abs_on_string_is_a_check_error() {
    let src = r#"
        fn main() {
            println(abs("hi"));
        }
    "#;
    let ds = diags(src);
    assert!(
        ds.iter().any(|m| m.contains("`abs` takes numeric")),
        "expected numeric-builtin diag; got: {:?}",
        ds
    );
}

#[test]
fn field_typo_gets_did_you_mean() {
    let src = r#"
        type Order { quantity: Int = 0; }
        fn main() {
            let o = Order { quantity: 2 };
            println(o.quantty);
        }
    "#;
    let ds = diags(src);
    assert!(
        ds.iter().any(|m| m.contains("did you mean `quantity`")),
        "expected did-you-mean; got: {:?}",
        ds
    );
}

#[test]
fn a_fixed_array_and_a_bounded_are_chain_sources() {
    // These two used to fail with "no field `get`": chains desugar
    // to a loop fetching each element through the source's `get`,
    // and the type-level collections had no such accessor, so they
    // could not anchor a chain at all. They answer `get` now, so the
    // programs below are ordinary (downstream handoff).
    //
    // Kept as a DIAGNOSTIC test rather than only a codegen one: the
    // failure mode was a confusing message on a reasonable program,
    // and the fix is that there is no message.
    for src in [
        r#"
            locus L {
                params { arr: [Int; 4] = [1, 2, 3, 4]; }
                fn c() -> Int { return self.arr.filter(it > 2).count(); }
            }
            fn main() { }
        "#,
        r#"
            fn take(b: bounded[Int; 8]) -> Int {
                return b.filter(it > 2).count();
            }
            fn main() { }
        "#,
    ] {
        let ds = diags(src);
        assert!(
            ds.is_empty(),
            "a chain over a type-level collection should check clean; \
             got: {:?}",
            ds
        );
    }
}

#[test]
fn a_real_field_typo_still_gets_did_you_mean_not_the_chain_hint() {
    // The chain hint must not swallow the ordinary did-you-mean path:
    // `.get` is the chain accessor, but a plain field typo on a struct
    // is not a chain and keeps its own hint.
    let src = r#"
        type Order { quantity: Int = 0; }
        fn main() {
            let o = Order { quantity: 2 };
            println(o.quantty);
        }
    "#;
    let ds = diags(src);
    assert!(
        ds.iter().any(|m| m.contains("did you mean `quantity`")
            && !m.contains("element chain")),
        "did-you-mean must be unaffected by the chain hint; got: {:?}",
        ds
    );
}

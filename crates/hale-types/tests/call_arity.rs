//! GH #229: arity mismatches are check-phase errors with spans —
//! previously `a.go(7)` on a zero-arg method passed `hale check`
//! and died at codegen as a locationless internal error. The
//! check enforces the UPPER bound only (extra args are always
//! wrong; omitted args may be excused by default params, whose
//! counts aren't modeled in FnSig/MethodInfo yet — the lower
//! bound is the issue's remaining follow-through).

use hale_syntax::parse_source;
use hale_types::symbol::Bundle;

fn diags(src: &str) -> Vec<String> {
    let program = parse_source(src).expect("parse");
    let mut programs: std::collections::BTreeMap<
        String,
        &hale_syntax::ast::Program,
    > = std::collections::BTreeMap::new();
    programs.insert("test.hl".to_string(), &program);
    let bundle = Bundle { programs };
    let (scope, mut ds) = hale_types::resolve::build_top_scope(&bundle);
    ds.extend(hale_types::check::check_bundle(&bundle, &scope, true));
    ds.iter().map(|d| d.message.clone()).collect()
}

#[test]
fn method_over_arity_is_a_check_error() {
    let src = r#"
        locus A {
            fn go() { }
        }
        fn main() {
            let a = A { };
            a.go(7);
        }
    "#;
    let ds = diags(src);
    assert!(
        ds.iter().any(|m| m.contains("method `go`")
            && m.contains("at most 0")
            && m.contains("got 1")),
        "expected over-arity diag; got: {:?}",
        ds
    );
}

#[test]
fn free_fn_over_arity_is_a_check_error() {
    let src = r#"
        fn add(a: Int, b: Int) -> Int { a + b }
        fn main() {
            let x = add(1, 2, 3);
            println(x);
        }
    "#;
    let ds = diags(src);
    assert!(
        ds.iter().any(|m| m.contains("fn `add`")
            && m.contains("at most 2")
            && m.contains("got 3")),
        "expected over-arity diag; got: {:?}",
        ds
    );
}

#[test]
fn exact_arity_stays_clean() {
    let src = r#"
        locus A {
            fn go(n: Int) { println(n); }
        }
        fn main() {
            let a = A { };
            a.go(7);
        }
    "#;
    let ds = diags(src);
    assert!(
        !ds.iter().any(|m| m.contains("at most")),
        "unexpected arity diag on exact call; got: {:?}",
        ds
    );
}

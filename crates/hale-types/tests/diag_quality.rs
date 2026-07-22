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
    let bundle = Bundle { programs };
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

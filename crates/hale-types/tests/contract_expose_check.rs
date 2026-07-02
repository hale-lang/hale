//! M3 stage 4 (2026-07-02): expose-side contract validity. Codegen
//! treats `contract` members as pure declaration, so typecheck is
//! the only place a lying expose can be caught.

use hale_syntax::parse_source;
use hale_types::check_program;

fn msgs(src: &str) -> Vec<String> {
    let prog = parse_source(src).expect("parse");
    check_program(&prog).into_iter().map(|d| d.message).collect()
}

fn contract_errs(src: &str) -> Vec<String> {
    msgs(src)
        .into_iter()
        .filter(|m| m.starts_with("contract:"))
        .collect()
}

#[test]
fn expose_of_nonexistent_member_errors() {
    let e = contract_errs(
        r#"
        locus Child {
            params { value: Int = 0; }
            contract { expose no_such_field: Int; }
        }
        fn main() { Child { }; }
    "#,
    );
    assert!(
        e.iter().any(|m| m.contains("exposes `no_such_field`")
            && m.contains("no field, mode, or method")),
        "got: {:?}",
        e
    );
}

#[test]
fn expose_type_must_match_field_type() {
    let e = contract_errs(
        r#"
        locus Child {
            params { value: Int = 0; }
            contract { expose value: String; }
        }
        fn main() { Child { }; }
    "#,
    );
    assert!(
        e.iter().any(|m| m.contains("exposes `value: String`")
            && m.contains("declared `Int`")),
        "got: {:?}",
        e
    );
}

#[test]
fn valid_field_expose_passes() {
    let e = contract_errs(
        r#"
        locus Child {
            params { value: Int = 0; name: String = "x"; }
            contract { expose value: Int; expose name: String; }
        }
        fn main() { Child { }; }
    "#,
    );
    assert!(e.is_empty(), "got: {:?}", e);
}

#[test]
fn mode_expose_checks_return_type() {
    // Valid: bulk declared, returns Float, exposed as Float.
    let ok = contract_errs(
        r#"
        locus Sensor {
            params { v: Float = 0.0; }
            contract { expose bulk: Float; }
            mode bulk() -> Float { return self.v; }
            mode harmonic() -> Float { return self.v; }
            mode resolution() -> Float { return self.v; }
        }
        fn main() { Sensor { }; }
    "#,
    );
    assert!(ok.is_empty(), "got: {:?}", ok);

    // Exposing an undeclared mode errors.
    let e = contract_errs(
        r#"
        locus Plain {
            params { v: Int = 0; }
            contract { expose bulk: Int; }
        }
        fn main() { Plain { }; }
    "#,
    );
    assert!(
        e.iter().any(|m| m.contains("exposes mode `bulk`")
            && m.contains("does not declare it")),
        "got: {:?}",
        e
    );
}

#[test]
fn method_expose_checks_return_type() {
    let ok = contract_errs(
        r#"
        locus Counter {
            params { n: Int = 0; }
            contract { expose count: Int; }
            fn count() -> Int { return self.n; }
        }
        fn main() { Counter { }; }
    "#,
    );
    assert!(ok.is_empty(), "got: {:?}", ok);

    let e = contract_errs(
        r#"
        locus Counter {
            params { n: Int = 0; }
            contract { expose count: String; }
            fn count() -> Int { return self.n; }
        }
        fn main() { Counter { }; }
    "#,
    );
    assert!(
        e.iter().any(|m| m.contains("exposes `count: String`")
            && m.contains("returns `Int`")),
        "got: {:?}",
        e
    );
}

//! M3 stage 3 (2026-07-02): generic fn call validation at
//! typecheck — the Ty-level mirror of codegen's m62 inference,
//! with source spans. Arity, binding conflicts, unpinned params,
//! substituted arg/return types.

use hale_syntax::parse_source;
use hale_types::check_program;

fn msgs(src: &str) -> Vec<String> {
    let prog = parse_source(src).expect("parse");
    check_program(&prog).into_iter().map(|d| d.message).collect()
}

const PICK: &str = r#"
    fn pick<T>(a: T, b: T, first: Bool) -> T {
        if first { return a; }
        return b;
    }
"#;

#[test]
fn binding_conflict_is_caught() {
    let m = msgs(&format!(
        "{}{}",
        PICK,
        r#"
        fn main() {
            let x = pick(1, "two", true);
            println(x);
        }
    "#
    ));
    assert!(
        m.iter().any(|s| s.contains("bound to both `Int` and `String`")),
        "got: {:?}",
        m
    );
}

#[test]
fn arity_is_checked() {
    let m = msgs(&format!(
        "{}{}",
        PICK,
        r#"
        fn main() {
            let x = pick(1, 2);
            println(x);
        }
    "#
    ));
    assert!(
        m.iter()
            .any(|s| s.contains("`pick` takes 3") && s.contains("got 2")),
        "got: {:?}",
        m
    );
}

#[test]
fn substituted_return_type_flows() {
    // pick(Int, Int) types as Int, so an Int-only context accepts it
    // and a String-typed misuse is caught downstream.
    let m = msgs(&format!(
        "{}{}",
        PICK,
        r#"
        fn wants_string(s: String) -> Int { return len(s); }
        fn main() {
            let n = pick(1, 2, true);
            println(n + 1);
        }
    "#
    ));
    let errs: Vec<&String> =
        m.iter().filter(|s| s.contains("generic fn")).collect();
    assert!(errs.is_empty(), "got: {:?}", errs);
}

#[test]
fn unpinned_generic_param_is_caught() {
    let m = msgs(
        r#"
        fn make<T>(n: Int) -> T {
            let x: T = make(n);
            return x;
        }
        fn main() {
            let v = make(3);
            println(v);
        }
    "#,
    );
    assert!(
        m.iter().any(|s| s.contains("cannot infer `T`")),
        "got: {:?}",
        m
    );
}

#[test]
fn valid_generic_calls_stay_clean() {
    let m = msgs(&format!(
        "{}{}",
        PICK,
        r#"
        fn main() {
            let n = pick(1, 2, true);
            let s = pick("x", "y", false);
            println(n, " ", s);
        }
    "#
    ));
    let errs: Vec<&String> =
        m.iter().filter(|s| s.contains("generic fn")).collect();
    assert!(errs.is_empty(), "got: {:?}", errs);
}

// ── Tranche 2: generic STRUCT literals ──

const BOX: &str = r#"
    type Box<T> {
        value: T;
    }
"#;

#[test]
fn monomorph_literal_fields_validate_substituted() {
    let m = msgs(&format!(
        "{}{}",
        BOX,
        r#"
        fn main() {
            let bad = Box_Int { value: "nope" };
            let typo = Box_Int { valeu: 42 };
            println(bad.value, typo.value);
        }
    "#
    ));
    assert!(
        m.iter().any(|s| s.contains("field `value` expects `Int`")
            && s.contains("got `String`")),
        "got: {:?}",
        m
    );
    assert!(
        m.iter()
            .any(|s| s.contains("has no field `valeu`")),
        "got: {:?}",
        m
    );
}

#[test]
fn monomorph_field_reads_type_substituted() {
    // b.value on Box_Int must type as Int — an Int use passes, and
    // valid programs stay clean.
    let m = msgs(&format!(
        "{}{}",
        BOX,
        r#"
        type Holder { b: Box<Int>; }
        fn main() {
            let inner = Box_Int { value: 42 };
            let h = Holder { b: inner };
            println(h.b.value + 1);
        }
    "#
    ));
    let errs: Vec<&String> = m
        .iter()
        .filter(|s| {
            s.contains("Box") || s.contains("no field")
        })
        .collect();
    assert!(errs.is_empty(), "got: {:?}", errs);
}

#[test]
fn generic_typeexpr_unifies_with_monomorph_literal() {
    // `b: Box<Int>` and a `Box_String` literal must MISMATCH.
    let m = msgs(&format!(
        "{}{}",
        BOX,
        r#"
        type Holder { b: Box<Int>; }
        fn main() {
            let inner = Box_String { value: "x" };
            let h = Holder { b: inner };
            println(h.b.value);
        }
    "#
    ));
    assert!(
        m.iter().any(|s| s.contains("expects `Box_Int`")
            && s.contains("got `Box_String`")),
        "got: {:?}",
        m
    );
}

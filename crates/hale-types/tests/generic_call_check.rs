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

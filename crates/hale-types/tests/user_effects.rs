//! User-declared effect classes (#345).
//!
//! A payments system wants to say which functions move money; the
//! propagation engine is already generic — it unions a bitmask over
//! the call graph and does not care what the bits mean. What was
//! missing is a way for a program to name its own bits.
//!
//! Grounded exactly like a built-in: attached to a leaf with
//! `@effects(is: {...})` and propagated by the same engine. The
//! compiler owns propagation; the program owns classification, which
//! is the same split the stdlib registry has with a different owner.

use hale_syntax::parse_source;

fn errs(src: &str) -> Vec<String> {
    let program = parse_source(src).expect("parse");
    hale_types::check_program(&program)
        .into_iter()
        .map(|d| d.message)
        .collect()
}

const MONEY: &str = "\
effect money;
@effects(is: {money})
fn charge(cents: Int) -> Int { return cents; }
fn calc(n: Int) -> Int { return n * 2; }
";

#[test]
fn a_user_effect_propagates_to_an_assertion() {
    let ds = errs(&format!(
        "{MONEY}@effects(none: {{money}})\n\
         fn price(n: Int) -> Int {{ return charge(n); }}\n\
         fn main() {{ println(price(5)); }}"
    ));
    assert!(
        ds.iter().any(|m| m.contains("effect assertion violated")),
        "reaching a fn that carries `money` must violate: {:?}",
        ds
    );
}

/// The control. Without it an over-broad attribution would satisfy the
/// test above while making every call violate everything.
#[test]
fn a_path_avoiding_the_carrier_still_certifies() {
    let ds = errs(&format!(
        "{MONEY}@effects(none: {{money}})\n\
         fn price(n: Int) -> Int {{ return calc(n); }}\n\
         fn main() {{ println(price(5)); }}"
    ));
    assert!(
        !ds.iter().any(|m| m.contains("effect assertion violated")),
        "a path that never reaches the carrier must certify: {:?}",
        ds
    );
}

/// Propagation is transitive — the whole point of using the engine
/// rather than a local check.
#[test]
fn it_propagates_through_an_intermediate() {
    let ds = errs(&format!(
        "{MONEY}fn mid(n: Int) -> Int {{ return charge(n); }}\n\
         @effects(none: {{money}})\n\
         fn price(n: Int) -> Int {{ return mid(n); }}\n\
         fn main() {{ println(price(5)); }}"
    ));
    assert!(
        ds.iter().any(|m| m.contains("effect assertion violated")),
        "a user effect must propagate through a helper: {:?}",
        ds
    );
}

/// Built-in classes must be unaffected by the user-class machinery.
#[test]
fn builtin_classes_still_work_alongside() {
    let ds = errs(
        "effect money;\n\
         @no_syscall\n\
         fn f() -> Int { return len(std::io::fs::read_file(\"/x\") or \"\"); }\n\
         fn main() { println(f()); }",
    );
    assert!(
        ds.iter().any(|m| m.contains("must not reach `syscall`")),
        "declaring a user effect must not disturb built-ins: {:?}",
        ds
    );
}

//! `hale check` must compare types at call boundaries (#335).
//!
//! It compared types at assignment sites and never at calls, so
//! `take("s")` against `fn take(n: Int)` reached codegen and surfaced
//! as "unsupported in codegen v0: fn `take` arg 0 type mismatch".
//!
//! That mattered more than an ordinary gap because `hale check` is the
//! documented oracle — AGENTS.md tells coding models to iterate against
//! it until it prints `ok`, so `ok` has to mean the program compiles.
//!
//! The legal-coercion tests below are the important half. A first cut
//! of this check flagged seven files in a downstream application that
//! build fine, because the in-tree corpus has no call site passing a
//! view where an owned type is expected.

use hale_syntax::parse_source;

fn errs(src: &str) -> Vec<String> {
    let program = parse_source(src).expect("parse");
    hale_types::check_program(&program)
        .into_iter()
        .map(|d| d.message)
        .collect()
}

fn rejects(src: &str, what: &str) {
    let ds = errs(src);
    assert!(
        ds.iter()
            .any(|m| m.contains("type mismatch") || m.contains("return: expected")),
        "{} must be caught by check, not left to codegen: {:?}",
        what,
        ds
    );
}

fn accepts(src: &str, what: &str) {
    let ds = errs(src);
    assert!(
        !ds.iter()
            .any(|m| m.contains("type mismatch") || m.contains("return: expected")),
        "{} is legal and must not be flagged: {:?}",
        what,
        ds
    );
}

#[test]
fn free_fn_argument_type_is_checked() {
    rejects(
        "fn take(n: Int) -> Int { return n; }\n\
         fn main() { println(take(\"s\")); }",
        "a String passed to an Int parameter",
    );
}

#[test]
fn locus_method_argument_type_is_checked() {
    rejects(
        "locus L { params { n: Int = 0; } fn take(v: Int) -> Int { return v; } }\n\
         main locus App { params { l: L = L { }; } birth() { println(self.l.take(\"s\")); } }\n\
         fn main() { App { }; }",
        "a String passed to a locus method's Int parameter",
    );
}

#[test]
fn self_method_argument_type_is_checked() {
    rejects(
        "locus L { params { n: Int = 0; }\n\
           fn take(v: Int) -> Int { return v; }\n\
           fn go() -> Int { return self.take(\"s\"); } }\n\
         main locus App { params { l: L = L { }; } birth() { println(self.l.go()); } }\n\
         fn main() { App { }; }",
        "a String passed through self.method(...)",
    );
}

#[test]
fn return_type_is_checked_in_a_non_fallible_fn() {
    // Only fallible bodies had a return check; a plain fn had none.
    rejects(
        "fn go() -> Int { return \"s\"; }\nfn main() { println(go()); }",
        "returning a String from a fn declared -> Int",
    );
}

#[test]
fn user_type_mismatch_is_checked_both_ways() {
    rejects(
        "type A { n: Int; }\ntype B { s: String; }\n\
         fn f(a: A) -> Int { return a.n; }\n\
         fn main() { println(f(B { s: \"x\" })); }",
        "an unrelated user type as an argument",
    );
    rejects(
        "type A { n: Int; }\ntype B { s: String; }\n\
         fn f() -> A { return B { s: \"x\" }; }\n\
         fn main() { println(f().n); }",
        "returning an unrelated user type",
    );
}

/// Int -> Float at a CALL is legal (codegen widens) even though the
/// same conversion is rejected at an assignment. Pinned so the check
/// is not "tightened" into breaking it.
#[test]
fn int_to_float_widening_at_a_call_is_legal() {
    accepts(
        "fn f(x: Float) -> Float { return x; }\nfn main() { println(f(3)); }",
        "Int -> Float widening at a call",
    );
}

/// An interface-typed parameter accepts any locus that satisfies it.
/// `assignable_from` is nominal and would reject that, so the check
/// defers to the structural conformance check.
#[test]
fn a_satisfying_locus_may_be_passed_to_an_interface_parameter() {
    accepts(
        "interface Greeter { fn hi() -> Int; }\n\
         locus Hi { params { n: Int = 0; } fn hi() -> Int { return 1; } }\n\
         fn use_it(g: Greeter) -> Int { return g.hi(); }\n\
         fn main() { let h = Hi { }; println(use_it(h)); }",
        "a locus satisfying an interface passed to an interface parameter",
    );
}

/// Arity was already checked; make sure it still is and that the two
/// checks do not mask each other.
#[test]
fn arity_is_still_checked_independently() {
    let ds = errs(
        "fn take(a: Int, b: Int) -> Int { return a + b; }\n\
         fn main() { println(take(1)); }",
    );
    assert!(
        ds.iter().any(|m| m.contains("takes at least")),
        "arity must still be reported: {:?}",
        ds
    );
}

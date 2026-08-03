//! An indirect call must not void a certificate (#353).
//!
//! Function pointers were the first genuinely open-world construct in
//! the language, and nothing noticed. A call through a function-typed
//! parameter reached the call graph as `Callee::Unresolved(param_name)`
//! — indistinguishable from a call to an unknown free fn, which
//! contributed nothing to any effect set or budget. So:
//!
//! ```text
//! @no_syscall
//! fn apply(f: fn(Int) -> Int, v: Int) -> Int { return f(v); }
//! ```
//!
//! typechecked, and the program printed the side effect. The same hole
//! swallowed `@budget`, and by extension `@hot`, `@deterministic`,
//! `@no_panic` and causality — every certificate the language offers.
//!
//! The fix is fail-closed: the enclosing fn's parameter list is in
//! hand when the edge is built (exactly as it is for `recv_ty`), so
//! the edge is marked `indirect`, and an indirect call is treated as
//! "may do anything" rather than "does nothing".
//!
//! This is deliberately conservative. Resolving the target exactly is
//! possible — Hale is whole-program and closed-world, and every
//! function value in the corpus is a literal name at its binding site
//! — but a conservative certificate is wrong in the safe direction and
//! an optimistic one is not.

use hale_syntax::parse_source;

fn errs(src: &str) -> Vec<String> {
    let program = parse_source(src).expect("parse");
    hale_types::check_program(&program)
        .into_iter()
        .map(|d| d.message)
        .collect()
}

#[test]
fn an_effect_certificate_cannot_pass_through_an_indirect_call() {
    let ds = errs(
        "fn does_syscall(x: Int) -> Int { println(\"side effect\"); return x; }\n\
         @no_syscall\n\
         fn apply(f: fn(Int) -> Int, v: Int) -> Int { return f(v); }\n\
         fn main() { println(apply(does_syscall, 1)); }",
    );
    let d = ds
        .iter()
        .find(|m| m.contains("effect assertion violated"))
        .unwrap_or_else(|| {
            panic!("`@no_syscall` must not hold over an indirect call: {:?}", ds)
        });
    assert!(
        d.contains("indirect call"),
        "the diagnostic must say WHY it cannot certify — the reader has \
         to know the target is unknowable, not that a syscall was \
         found: {}",
        d
    );
}

#[test]
fn a_budget_cannot_pass_through_an_indirect_call() {
    let ds = errs(
        "fn allocates(n: Int) -> String { return \"x\" + \"y\"; }\n\
         @budget(alloc_per_call = 0)\n\
         fn apply(f: fn(Int) -> String, v: Int) -> String { return f(v); }\n\
         fn main() { println(apply(allocates, 1)); }",
    );
    assert!(
        ds.iter().any(|m| m.contains("budget exceeded")),
        "`alloc_per_call = 0` must not hold over an indirect call — the \
         callee, and so the allocation count, is the caller's choice: {:?}",
        ds
    );
}

/// The conservatism must be SCOPED. A fn with no function-typed
/// parameter is unaffected, or every certificate in the language would
/// start failing.
#[test]
fn a_direct_call_still_certifies() {
    let ds = errs(
        "fn pure_double(x: Int) -> Int { return x * 2; }\n\
         @no_syscall\n\
         fn apply(v: Int) -> Int { return pure_double(v); }\n\
         fn main() { println(apply(1)); }",
    );
    assert!(
        ds.is_empty(),
        "a direct call to a syscall-free fn must still certify: {:?}",
        ds
    );
}

/// A fn-typed parameter that is never CALLED is not an indirect call
/// site. Passing a function through must stay free.
#[test]
fn merely_holding_a_fn_param_is_not_an_indirect_call() {
    let ds = errs(
        "fn inner(f: fn(Int) -> Int, v: Int) -> Int { return v; }\n\
         @no_syscall\n\
         fn outer(f: fn(Int) -> Int, v: Int) -> Int { return inner(f, v); }\n\
         fn main() { println(outer(inner_id, 1)); }\n\
         fn inner_id(x: Int) -> Int { return x; }",
    );
    assert!(
        !ds.iter().any(|m| m.contains("indirect call")),
        "neither fn calls through its parameter, so nothing is \
         indirect: {:?}",
        ds
    );
}

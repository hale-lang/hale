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
//!
//! F.40 E5 resolves it where the program states it: an indirect call
//! reaches the program's function values (the functions some expression
//! reads as a value) of its arity, narrowed by the parameter's declared
//! type, so the certificate names the function the call can reach. A
//! call no such value can be stays the "may do anything" call.

use hale_syntax::parse_source;

fn errs(src: &str) -> Vec<String> {
    let program = parse_source(src).expect("parse");
    hale_types::check_program(&program)
        .into_iter()
        .map(|d| d.message)
        .collect()
}

/// F.40 E5, a classified correction: the call through `f` reaches the
/// program's one function value of its type, `does_syscall`, and the
/// violation names it (it named the indirect call).
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
    assert!(d.contains("apply -> does_syscall"), "the witness is the function the call reaches: {}", d);
}

/// No function value of the parameter's type: the call stays the one
/// whose target is unknowable, and the diagnostic says so.
#[test]
fn an_indirect_call_no_value_can_be_stays_unknowable() {
    let ds = errs(
        "fn does_syscall(x: Int) -> Int { println(\"side effect\"); return x; }\n\
         @no_syscall\n\
         fn apply(f: fn(String) -> Int, v: String) -> Int { return f(v); }\n\
         fn main() { println(does_syscall(1)); }",
    );
    let d = ds
        .iter()
        .find(|m| m.contains("effect assertion violated"))
        .unwrap_or_else(|| panic!("`@no_syscall` must not hold over an indirect call: {:?}", ds));
    assert!(
        d.contains("indirect call through a function-typed parameter"),
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

/// P3 2 of 3, a classified correction: a call through a local bound to
/// a fn name or path reaches that fn. The edge used to be `Unresolved`
/// with the local's name, a call to nothing, so `@no_syscall` certified
/// a fn that performs a syscall through `let f = …; f()`.
#[test]
fn a_certificate_sees_a_call_through_a_let_bound_fn() {
    for (bound, prelude) in [
        ("does_syscall", "fn does_syscall() -> Int { println(\"side effect\"); return 1; }\n"),
        ("std::process::pid", ""),
    ] {
        let ds = errs(&format!(
            "{prelude}@no_syscall\n\
             fn g() -> Int {{ let f = {bound}; return f(); }}\n\
             fn main() {{ println(g()); }}"
        ));
        let d = ds
            .iter()
            .find(|m| m.contains("effect assertion violated"))
            .unwrap_or_else(|| panic!("`let f = {bound}; f()` performs a syscall: {:?}", ds));
        assert!(!d.contains("indirect call"), "the call is resolved, not indirect: {d}");
    }
    // The control: a local bound to a syscall-free fn still certifies.
    let ds = errs(
        "fn pure_double(x: Int) -> Int { return x * 2; }\n\
         @no_syscall\n\
         fn apply(v: Int) -> Int { let f = pure_double; return f(v); }\n\
         fn main() { println(apply(1)); }",
    );
    assert!(ds.is_empty(), "a local bound to a syscall-free fn certifies: {:?}", ds);
}

/// F.40 E5, a classified correction: a call through a local the
/// summary cannot follow to one fn is indirect, as a call through a
/// function-typed parameter is. It was a call to nothing, so
/// `@no_syscall` certified both of the #1318 review's programs while
/// each performed the syscall: a binding chosen by a branch, and a
/// binding a loop reassigns after the call (the second iteration calls
/// what the first assigned). With the call resolved to the program's
/// function values (E5), the violation names `pid`.
#[test]
fn a_certificate_cannot_pass_through_an_unresolved_function_value() {
    const FNS: &str = "fn pure() -> Int { return 1; }\n\
                       fn pid() -> Int { return std::process::pid(); }\n";
    for body in [
        "let f = if len(\"ab\") == 2 { pid } else { pure };\n return f();",
        "let mut f = pure;\n let mut i = 0;\n let mut n = 0;\n \
         while i < 2 { n = f(); f = pid; i = i + 1; }\n return n;",
    ] {
        let ds = errs(&format!(
            "{FNS}@no_syscall\nfn via() -> Int {{\n {body}\n}}\nfn main() {{ println(via()); }}"
        ));
        let d = ds
            .iter()
            .find(|m| m.contains("effect assertion violated"))
            .unwrap_or_else(|| panic!("`f()` may call `pid`, so `via` may syscall: {body}\n{:?}", ds));
        assert!(d.contains("via -> pid"), "{d}");
    }
    // The control: a binding nothing reassigns is followed to its fn,
    // and a syscall-free one still certifies.
    let ds = errs(&format!(
        "{FNS}@no_syscall\nfn via() -> Int {{\n let f = pure;\n let mut n = 0;\n \
         let mut i = 0;\n while i < 2 {{ n = n + f(); i = i + 1; }}\n return n;\n}}\n\
         fn main() {{ println(via()); }}"
    ));
    assert!(ds.is_empty(), "a stable binding to a syscall-free fn certifies: {:?}", ds);
}

/// The budget reads the same edge: an allocation behind an unresolved
/// function value counts as unbounded, not zero.
#[test]
fn a_budget_cannot_pass_through_an_unresolved_function_value() {
    let ds = errs(
        "fn allocates() -> String { return \"x\" + \"y\"; }\n\
         fn empty() -> String { return \"\"; }\n\
         @budget(alloc_per_call = 0)\n\
         fn via() -> String { let f = if len(\"ab\") == 2 { allocates } else { empty }; return f(); }\n\
         fn main() { println(via()); }",
    );
    assert!(
        ds.iter().any(|m| m.contains("budget exceeded")),
        "`alloc_per_call = 0` must not hold over a call to an unresolved function value: {:?}",
        ds
    );
}

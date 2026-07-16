//! Lever 2 (2026-07-16) — `@budget(alloc_per_call = N)`, the opt-in
//! hot-path allocation contract. A hard error when an annotated fn/method
//! allocates more than its declared per-call ceiling; silent when the
//! contract holds. Enforced in `hale_types::budget_check`.

use hale_syntax::parse_source;
use hale_types::check_program;

/// Budget-contract diagnostics only (the check emits hard `Type` errors;
/// unrelated diagnostics from the rest of the checker are filtered out).
fn budget_errors(src: &str) -> Vec<String> {
    let prog = parse_source(src).expect("parse failed");
    check_program(&prog)
        .into_iter()
        .filter(|d| {
            d.message.contains("budget exceeded")
                || d.message.contains("alloc budget")
        })
        .map(|d| d.message)
        .collect()
}

// ---- clean: the contract holds ---------------------------------------

#[test]
fn zero_alloc_fn_with_no_allocation_is_clean() {
    let src = r#"
@budget(alloc_per_call = 0)
fn tick(x: Int) -> Int {
    x + 1
}

fn main() { }
"#;
    assert!(
        budget_errors(src).is_empty(),
        "a genuinely zero-alloc fn must satisfy `= 0`, got: {:?}",
        budget_errors(src)
    );
}

#[test]
fn budget_of_two_with_two_allocations_is_clean() {
    let src = r#"
type P { a: Int; }

@budget(alloc_per_call = 2)
fn two() -> P {
    let x = P { a: 1 };
    let y = P { a: 2 };
    y
}

fn main() { }
"#;
    assert!(
        budget_errors(src).is_empty(),
        "two allocations under a budget of 2 must be clean, got: {:?}",
        budget_errors(src)
    );
}

#[test]
fn zero_alloc_method_is_clean() {
    let src = r#"
locus Handler {
    @budget(alloc_per_call = 0)
    fn on_msg(x: Int) -> Int {
        x + 1
    }
}

fn main() { }
"#;
    assert!(
        budget_errors(src).is_empty(),
        "a zero-alloc method must satisfy `= 0`, got: {:?}",
        budget_errors(src)
    );
}

// ---- violations: the contract bites ----------------------------------

#[test]
fn zero_alloc_fn_that_instantiates_is_rejected() {
    let src = r#"
type P { a: Int; }

@budget(alloc_per_call = 0)
fn make() -> P {
    P { a: 1 }
}

fn main() { }
"#;
    let errs = budget_errors(src);
    assert!(
        errs.iter().any(|m| m.contains("budget exceeded") && m.contains("make")),
        "an allocation under `= 0` must be rejected, got: {:?}",
        errs
    );
}

#[test]
fn budget_counts_transitively_through_resolved_callees() {
    // `caller` itself allocates nothing, but the fn it calls does —
    // the budget sees through resolved (bundle-local) calls.
    let src = r#"
type P { a: Int; }

fn helper() -> P {
    P { a: 1 }
}

@budget(alloc_per_call = 0)
fn caller() -> P {
    helper()
}

fn main() { }
"#;
    let errs = budget_errors(src);
    assert!(
        errs.iter().any(|m| m.contains("budget exceeded") && m.contains("caller")),
        "a transitive allocation must count against the budget, got: {:?}",
        errs
    );
}

#[test]
fn allocation_in_a_loop_is_unbounded_per_call() {
    let src = r#"
type P { a: Int; }

@budget(alloc_per_call = 4)
fn spin() {
    let mut n = 0;
    while n < 10 {
        let x = P { a: n };
        n = n + 1;
    }
}

fn main() { }
"#;
    let errs = budget_errors(src);
    assert!(
        errs.iter().any(|m| m.contains("budget exceeded")
            && m.contains("unbounded")),
        "a loop-nested allocation must read as unbounded per call, got: {:?}",
        errs
    );
}

#[test]
fn allocating_recv_in_a_loop_is_rejected_under_zero_budget() {
    // The known-allocating `recv` family is counted like a visible
    // allocation; in a loop it's unbounded per call.
    let src = r#"
@budget(alloc_per_call = 0)
fn pump(fd: Int) {
    let mut n = 0;
    while n < 10 {
        let msg = std::io::udp::recv(fd, 2048) or discard;
        n = n + 1;
    }
}

fn main() { }
"#;
    let errs = budget_errors(src);
    assert!(
        errs.iter().any(|m| m.contains("budget exceeded")),
        "an allocating recv in a loop must bust a zero budget, got: {:?}",
        errs
    );
    // And the pinpoint should steer toward recv_into.
    assert!(
        errs.iter().any(|m| m.contains("recv_into")),
        "expected a recv_into pinpoint, got: {:?}",
        errs
    );
}

#[test]
fn budget_of_one_with_two_allocations_is_rejected() {
    let src = r#"
type P { a: Int; }

@budget(alloc_per_call = 1)
fn two() -> P {
    let x = P { a: 1 };
    let y = P { a: 2 };
    y
}

fn main() { }
"#;
    let errs = budget_errors(src);
    assert!(
        errs.iter().any(|m| m.contains("budget exceeded")),
        "two allocations must bust a budget of 1, got: {:?}",
        errs
    );
}

#[test]
fn unannotated_fn_is_never_checked() {
    // No `@budget` = no contract = no diagnostic, however much it
    // allocates. The check is strictly opt-in.
    let src = r#"
type P { a: Int; }

fn allocates_freely() -> P {
    let mut n = 0;
    while n < 10 {
        let x = P { a: n };
        n = n + 1;
    }
    P { a: 0 }
}

fn main() { }
"#;
    assert!(
        budget_errors(src).is_empty(),
        "an unannotated fn must never trip the budget check, got: {:?}",
        budget_errors(src)
    );
}

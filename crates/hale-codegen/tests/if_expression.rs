//! Phase 2b — `if`-as-expression with trailing-expression block value.
//!
//! Closes notes/hale-friction.md `if-needs-block-value`. The form
//! "block evaluates to its trailing non-`;` expression" was the
//! plan's symmetry target — match-arm direct expressions
//! (`MatchArmBody::Expr`) and function-body returns already produced
//! values; only `if` was the holdout that forced callers to write
//! the equivalent via `let mut x = ...; if cond { x = i; } else
//! { x = compute(); }`. After Phase 2b, `let x = if cond { i }
//! else { compute() };` is a first-class form, composing with
//! let-bindings, function calls, and nested if/else.

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn build_and_run(name: &str, source: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(source).expect("parse");
    let bin = harness::unique_bin(&format!("hale_test_ifexpr_{}", name));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (String::from_utf8_lossy(&output.stdout).to_string(), output.status)
}

#[test]
fn if_expr_in_let_int_arms() {
    let src = r#"
        fn main() {
            let cond: Bool = true;
            let x: Int = if cond { 10 } else { 20 };
            println("x=", x);
        }
    "#;
    let (stdout, status) = build_and_run("let_int", src);
    assert!(status.success(), "exit: {:?}", status);
    assert!(stdout.contains("x=10"), "got: {:?}", stdout);
}

#[test]
fn if_expr_else_branch_taken() {
    let src = r#"
        fn main() {
            let cond: Bool = false;
            let x: Int = if cond { 10 } else { 20 };
            println("x=", x);
        }
    "#;
    let (stdout, status) = build_and_run("else_taken", src);
    assert!(status.success(), "exit: {:?}", status);
    assert!(stdout.contains("x=20"), "got: {:?}", stdout);
}

#[test]
fn if_expr_with_string_arms() {
    let src = r#"
        fn main() {
            let cond: Bool = true;
            let s: String = if cond { "yes" } else { "no" };
            println("s=", s);
        }
    "#;
    let (stdout, status) = build_and_run("string", src);
    assert!(status.success(), "exit: {:?}", status);
    assert!(stdout.contains("s=yes"), "got: {:?}", stdout);
}

#[test]
fn if_expr_arms_are_function_calls() {
    let src = r#"
        fn compute_a() -> Int { return 100; }
        fn compute_b() -> Int { return 200; }
        fn main() {
            let pick_a: Bool = true;
            let r: Int = if pick_a { compute_a() } else { compute_b() };
            println("r=", r);
        }
    "#;
    let (stdout, status) = build_and_run("fn_call_arms", src);
    assert!(status.success(), "exit: {:?}", status);
    assert!(stdout.contains("r=100"), "got: {:?}", stdout);
}

#[test]
fn if_expr_else_if_chain() {
    // Three-way: form is `if a { ... } else if b { ... } else { ... }`.
    // Verifies the ElseIf wrapper carries through the value path.
    let src = r#"
        fn main() {
            let kind: Int = 2;
            let label: String = if kind == 1 { "one" }
                                else if kind == 2 { "two" }
                                else { "other" };
            println("label=", label);
        }
    "#;
    let (stdout, status) = build_and_run("else_if", src);
    assert!(status.success(), "exit: {:?}", status);
    assert!(stdout.contains("label=two"), "got: {:?}", stdout);
}

#[test]
fn if_stmt_form_still_works() {
    // Backwards compat: `if cond { ... } else { ... }` as a STATEMENT
    // (no surrounding `let` or expression context) still lowers via
    // the pre-Phase-2b stmt path. The blocks have no trailing-tail
    // here (each ends with a `;`-terminated stmt) so this is the
    // unchanged behavior.
    let src = r#"
        fn main() {
            let mut x: Int = 0;
            let cond: Bool = true;
            if cond {
                x = 7;
            } else {
                x = 13;
            }
            println("x=", x);
        }
    "#;
    let (stdout, status) = build_and_run("stmt_form", src);
    assert!(status.success(), "exit: {:?}", status);
    assert!(stdout.contains("x=7"), "got: {:?}", stdout);
}

#[test]
fn if_expr_arm_with_let_then_tail() {
    // Block-tail composes with let-bindings inside the arm. The
    // let-bound `t` is scoped to the then-block; the tail `t * 2`
    // is the block's value.
    let src = r#"
        fn main() {
            let cond: Bool = true;
            let r: Int = if cond {
                let t: Int = 21;
                t * 2
            } else {
                0
            };
            println("r=", r);
        }
    "#;
    let (stdout, status) = build_and_run("let_then_tail", src);
    assert!(status.success(), "exit: {:?}", status);
    assert!(stdout.contains("r=42"), "got: {:?}", stdout);
}

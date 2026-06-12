//! WS3.2 — a nested `if` as a block's tail value.
//!
//! `if_expression.rs` (Phase 2b) made `let x = if c { a } else { b }`
//! a value. But an `if` appearing as the *tail of a block* — e.g.
//! the then-arm of an outer `if` — was still parsed as a statement,
//! so the enclosing block's tail was `()`:
//!
//!     let x = if a { if b { p } else { q } } else { r };
//!     //          ^^^^^^^^^^^^^^^^^^^^^^^^^ then-arm typed `()`
//!
//! failing typecheck with `then=() else=Float`, contradicting
//! docs/basics "if is an expression."
//!
//! WS3.2 makes a *value-producing* trailing `if` (every arm ends in
//! a tail expression) the block's tail expression. A side-effect
//! `if` (no else, or any arm with no tail) stays a statement — the
//! regression guards below lock that in, because routing those
//! through `Expr::If` breaks every compile (the stdlib is full of
//! side-effect trailing ifs).

use std::process::Command;

use hale_codegen::build_executable;

fn build_and_run(name: &str, source: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_ws3_nested_if_{}", name));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        output.status,
    )
}

#[test]
fn nested_if_as_then_arm_tail() {
    // The friction target: inner `if` is the tail of the outer
    // then-arm block.
    let src = r#"
        fn main() {
            let a: Bool = true;
            let b: Bool = false;
            let p: Float = 1.0;
            let q: Float = 2.0;
            let r: Float = 3.0;
            let x: Float = if a { if b { p } else { q } } else { r };
            println("x=", x);
        }
    "#;
    let (stdout, status) = build_and_run("then_arm", src);
    assert!(status.success(), "exit: {:?}", status);
    // a=true, b=false → inner else → q = 2.0
    assert!(stdout.contains("x=2"), "got: {:?}", stdout);
}

#[test]
fn nested_if_as_else_arm_tail() {
    let src = r#"
        fn main() {
            let a: Bool = false;
            let b: Bool = true;
            let x: Int = if a { 10 } else { if b { 20 } else { 30 } };
            println("x=", x);
        }
    "#;
    let (stdout, status) = build_and_run("else_arm", src);
    assert!(status.success(), "exit: {:?}", status);
    // a=false → else → b=true → 20
    assert!(stdout.contains("x=20"), "got: {:?}", stdout);
}

#[test]
fn if_else_if_chain_as_tail_value() {
    let src = r#"
        fn classify(n: Int) -> Int {
            let r: Int = if n < 0 { 0 } else if n == 0 { 1 } else { 2 };
            return r;
        }
        fn main() {
            println("neg=", classify(0 - 5));
            println("zero=", classify(0));
            println("pos=", classify(7));
        }
    "#;
    let (stdout, status) = build_and_run("elseif_chain", src);
    assert!(status.success(), "exit: {:?}", status);
    assert!(stdout.contains("neg=0"), "got: {:?}", stdout);
    assert!(stdout.contains("zero=1"), "got: {:?}", stdout);
    assert!(stdout.contains("pos=2"), "got: {:?}", stdout);
}

#[test]
fn if_as_block_expression_tail() {
    // A bare block-expression whose tail is a value-producing if.
    let src = r#"
        fn main() {
            let c: Bool = true;
            let x: Int = { let base: Int = 5; if c { base + 1 } else { base } };
            println("x=", x);
        }
    "#;
    let (stdout, status) = build_and_run("block_tail", src);
    assert!(status.success(), "exit: {:?}", status);
    assert!(stdout.contains("x=6"), "got: {:?}", stdout);
}

#[test]
fn side_effect_trailing_if_without_else_still_a_statement() {
    // Regression guard: an else-less trailing `if` used for side
    // effects must NOT become a tail expression (it has no value).
    let src = r#"
        fn main() {
            let c: Bool = true;
            let mut n: Int = 0;
            if c { n = 41; }
            println("n=", n + 1);
        }
    "#;
    let (stdout, status) = build_and_run("sideeffect_noelse", src);
    assert!(status.success(), "exit: {:?}", status);
    assert!(stdout.contains("n=42"), "got: {:?}", stdout);
}

#[test]
fn side_effect_trailing_if_with_else_still_a_statement() {
    // Regression guard: a trailing `if/else` whose arms are
    // statement-only (no tail) stays a statement.
    let src = r#"
        fn main() {
            let c: Bool = false;
            let mut n: Int = 0;
            if c { n = 1; } else { n = 99; }
            println("n=", n);
        }
    "#;
    let (stdout, status) = build_and_run("sideeffect_else", src);
    assert!(status.success(), "exit: {:?}", status);
    assert!(stdout.contains("n=99"), "got: {:?}", stdout);
}

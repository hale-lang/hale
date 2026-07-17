//! Gap C (2026-07-17) — `match`-as-expression.
//!
//! `Expr::Match` parsed and typechecked but had no codegen lowering
//! (`Unsupported("expression form Discriminant(15)")`), so
//! `let x = match n { ... };` — a form the docs' control-flow
//! chapter already showed — failed to build. The lowering shares
//! `lower_match_core` with the statement form (every pattern kind,
//! guards, bindings) and phi-merges the arm-body values at
//! `match.after`, mirroring `lower_if_expr`. Typecheck now types
//! the expression as the join of its arm types (statement-position
//! arms stay heterogeneous-legal) and rejects mismatched value
//! arms with a spanned diag.

use std::process::Command;

use hale_codegen::build_executable;

fn build_and_run(name: &str, source: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_test_matchexpr_{}", name));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (String::from_utf8_lossy(&output.stdout).to_string(), output.status)
}

#[test]
fn match_expr_int_scrutinee_in_let_and_return() {
    let src = r#"
        fn pick(n: Int) -> Int {
            return match n {
                0 -> 10,
                1 -> 11,
                _ -> 99,
            };
        }
        fn main() {
            let x = match 1 { 1 -> 5, _ -> 6, };
            println("x=", x, " p0=", pick(0), " p7=", pick(7));
        }
    "#;
    let (stdout, status) = build_and_run("int_let_return", src);
    assert!(status.success(), "exit: {:?}", status);
    assert!(stdout.contains("x=5 p0=10 p7=99"), "got: {:?}", stdout);
}

#[test]
fn match_expr_string_scrutinee_dynamic_key() {
    // Concat-built scrutinee so the String compare runs against an
    // arena value, not a .rodata pointer-equal literal.
    let src = r#"
        fn code(cmd: String) -> Int {
            return match cmd {
                "move" -> 1,
                "fire" -> 2,
                _ -> 0,
            };
        }
        fn main() {
            println("m=", code("mo" + "ve"), " f=", code("fire"), " u=", code("xyz"));
        }
    "#;
    let (stdout, status) = build_and_run("string_scrutinee", src);
    assert!(status.success(), "exit: {:?}", status);
    assert!(stdout.contains("m=1 f=2 u=0"), "got: {:?}", stdout);
}

#[test]
fn match_expr_enum_payload_destructuring_and_string_result() {
    let src = r#"
        type Ev = enum { Tick(Int), Halt };
        type Color = enum { Red, Green, Blue };
        fn ev_value(e: Ev) -> Int {
            return match e {
                Ev::Tick(n) -> n * 2,
                Ev::Halt -> -1,
            };
        }
        fn color_name(c: Color) -> String {
            return match c {
                Color::Red -> "red",
                Color::Green -> "green",
                Color::Blue -> "blue",
            };
        }
        fn main() {
            println("t=", ev_value(Ev::Tick(7)), " h=", ev_value(Ev::Halt),
                    " c=", color_name(Color::Green));
        }
    "#;
    let (stdout, status) = build_and_run("enum_payload", src);
    assert!(status.success(), "exit: {:?}", status);
    assert!(stdout.contains("t=14 h=-1 c=green"), "got: {:?}", stdout);
}

#[test]
fn match_expr_guards_and_block_arms() {
    let src = r#"
        fn sign(n: Int) -> String {
            return match n {
                v if v < 0 -> "neg",
                0 -> "zero",
                _ -> "pos",
            };
        }
        fn blocky(n: Int) -> Int {
            let x = match n {
                0 -> { let a = 5; a * 2 },
                _ -> { let b = n; b + 100 },
            };
            return x;
        }
        fn main() {
            println("s=", sign(-3), sign(0), sign(9),
                    " b0=", blocky(0), " b4=", blocky(4));
        }
    "#;
    let (stdout, status) = build_and_run("guards_blocks", src);
    assert!(status.success(), "exit: {:?}", status);
    assert!(stdout.contains("s=negzeropos b0=10 b4=104"), "got: {:?}", stdout);
}

#[test]
fn match_expr_all_guarded_fallthrough_yields_zero() {
    // Every arm guarded and every guard false at runtime — the one
    // reachable no-match case in expression position. The spec'd
    // behavior is the zero value of the result type (mirrors the
    // statement form's silent no-op), never poison/UB.
    let src = r#"
        fn f(n: Int) -> Int {
            return match n {
                v if v > 100 -> 1,
                v if v < -100 -> 2,
                _ if false -> 3,
            };
        }
        fn main() {
            println("z=", f(0));
        }
    "#;
    let (stdout, status) = build_and_run("guarded_fallthrough", src);
    assert!(status.success(), "exit: {:?}", status);
    assert!(stdout.contains("z=0"), "got: {:?}", stdout);
}

#[test]
fn match_expr_nested_in_arithmetic() {
    let src = r#"
        fn main() {
            let total = match 1 { 1 -> 10, _ -> 0, } + match 2 { 2 -> 5, _ -> 0, };
            println("total=", total);
        }
    "#;
    let (stdout, status) = build_and_run("nested_arith", src);
    assert!(status.success(), "exit: {:?}", status);
    assert!(stdout.contains("total=15"), "got: {:?}", stdout);
}

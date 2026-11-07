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

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

fn build_and_run(name: &str, source: &str) -> (String, std::process::ExitStatus) {
    let bin = harness::unique_bin(&format!("hale_test_matchexpr_{}", name));
    build_opts::build_source(source, &bin, &build_opts::options()).expect("build");
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

/// A match arm's block whose trailing `match` diverges in every arm
/// (`return`s): the block's value is that match (spec/semantics.md,
/// `match` as a block's value), and with no arm value to merge the
/// merge block `match.after` is reached by no edge but the no-arm-matched
/// one, which is closed as the statement form closes it. Until 2026-11-06
/// (downstream handoff review) the block was left with no terminator and
/// the build failed in LLVM's verifier.
#[test]
fn match_expr_arm_block_whose_trailing_match_diverges_in_every_arm() {
    let src = r#"
        fn choose(n: Int) -> Int {
            let x = match n {
                0 -> {
                    match n {
                        0 -> { return 1; },
                        _ -> { return 2; },
                    }
                },
                _ -> 3,
            };
            return x;
        }
        fn main() { println(choose(0)); println(choose(5)); }
    "#;
    let (out, status) = build_and_run("diverging_trailing_match", src);
    assert!(status.success(), "{status:?}");
    assert_eq!(out, "1\n3\n");
}

/// A block in value position whose trailing `match` diverges in every
/// arm ends control flow, and the consumer of the value — a `let`, an
/// `if` expression's arms, an `or` substitute, a call argument, a
/// `return`, an assignment — emits nothing live after it: the lowering
/// continues in a block nothing reaches, sealed at the statement
/// boundary. Until 2026-11-06 (downstream handoff review) the `let`
/// stored its binding after the match's `unreachable` ("Terminator
/// found in the middle of a basic block") and the build failed.
#[test]
fn match_expr_diverging_block_value_through_every_consumer() {
    let src = r#"
        type E { k: Int = 0; }
        fn risky(n: Int) -> Int fallible(E) { if n > 0 { fail E { k: n }; } return n; }
        fn twice(n: Int) -> Int { return n * 2; }
        fn via_let(n: Int) -> Int {
            let x = { match n { 0 -> { return 1; }, _ -> { return 2; } } };
            return x;
        }
        fn via_if(n: Int) -> Int {
            let x = if n > 5 {
                match n { 6 -> { return 6; }, _ -> { return 7; } }
            } else {
                match n { 0 -> { return 10; }, _ -> { return 11; } }
            };
            return x;
        }
        fn via_or(n: Int) -> Int {
            let x = risky(n) or { match err.k { 1 -> { return 21; }, _ -> { return 22; } } };
            return x + 100;
        }
        fn via_arg(n: Int) -> Int {
            let x = twice({ match n { 0 -> { return 31; }, _ -> { return 32; } } });
            return x;
        }
        fn via_return(n: Int) -> Int {
            return { match n { 0 -> { return 41; }, _ -> { return 42; } } };
        }
        fn via_assign(n: Int) -> Int {
            let mut x = 0;
            x = { match n { 0 -> { return 51; }, _ -> { return 52; } } };
            return x;
        }
        fn main() {
            println(via_let(0), " ", via_let(3));
            println(via_if(6), " ", via_if(9), " ", via_if(0), " ", via_if(2));
            println(via_or(0), " ", via_or(1), " ", via_or(2));
            println(via_arg(0), " ", via_arg(1));
            println(via_return(0), " ", via_return(1));
            println(via_assign(0), " ", via_assign(1));
        }
    "#;
    let (out, status) = build_and_run("diverging_block_consumers", src);
    assert!(status.success(), "{status:?}");
    assert_eq!(out, "1 2\n6 7 10 11\n100 21 22\n31 32\n41 42\n51 52\n");
}

/// The diverged value's consumer may want a type other than the
/// placeholder's: a String or Bytes parameter, a String return, store or
/// record field, a Bool condition. Each refuses the placeholder, and the
/// refusal is unreachable code's: the statement is terminated and the
/// build goes on. An Int parameter takes the placeholder as a well-typed
/// `i64 undef` (it was an `i32`, which the CLI build's verifier caught:
/// downstream handoff review, 2026-11-07). Every shape here is also in
/// tests/hale/block_tail_match_test.hl, built by the CLI.
#[test]
fn match_expr_diverging_block_value_into_typed_consumers() {
    let src = r#"
        type Rec { name: String = ""; }
        fn twice(n: Int) -> Int { return n * 2; }
        fn shout(s: String) -> String { return s + "!"; }
        fn size(b: Bytes) -> Int { return len(b); }
        fn int_arg(n: Int) -> Int {
            let x = twice({ match n { 0 -> { return 1; }, _ -> { return 2; } } });
            return x;
        }
        fn string_arg(n: Int) -> String {
            let x = shout({ match n { 0 -> { return "a"; }, _ -> { return "b"; } } });
            return x;
        }
        fn bytes_arg(n: Int) -> Int {
            let x = size({ match n { 0 -> { return 11; }, _ -> { return 12; } } });
            return x;
        }
        fn string_return(n: Int) -> String {
            return { match n { 0 -> { return "c"; }, _ -> { return "d"; } } };
        }
        fn string_assign(n: Int) -> String {
            let mut s = "";
            s = { match n { 0 -> { return "e"; }, _ -> { return "f"; } } };
            return s;
        }
        fn string_field(n: Int) -> String {
            let r = Rec { name: { match n { 0 -> { return "g"; }, _ -> { return "h"; } } } };
            return r.name;
        }
        fn bool_cond(n: Int) -> Int {
            if { match n { 0 -> { return 21; }, _ -> { return 22; } } } { return 0; }
            return 1;
        }
        fn main() {
            println(int_arg(0), " ", int_arg(1));
            println(string_arg(0), " ", string_arg(1));
            println(bytes_arg(0), " ", bytes_arg(1));
            println(string_return(0), " ", string_return(1));
            println(string_assign(0), " ", string_assign(1));
            println(string_field(0), " ", string_field(1));
            println(bool_cond(0), " ", bool_cond(1));
        }
    "#;
    let (out, status) = build_and_run("diverging_typed_consumers", src);
    assert!(status.success(), "{status:?}");
    assert_eq!(out, "1 2\na b\n11 12\nc d\ne f\ng h\n21 22\n");
}

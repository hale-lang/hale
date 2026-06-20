//! WS3.1 — `std::math::int_to_float` / `std::math::float_to_int`
//! in expression position.
//!
//! Fathom mdgw-evm item 3 / pond reported these named conversions
//! failing codegen ("unsupported in codegen v0 ... in expression
//! position"), forcing numeric consumers to round-trip through
//! ASCII (`to_string` + `parse_*`). They lower to the trivial
//! `sitofp` (widening) / `fptosi` (narrowing, round-toward-zero)
//! — the same primitives behind implicit Float-arg widening and
//! the `Int(x)` cast, surfaced as named functions in the
//! `std::math` namespace the downstream code is written against.

use std::process::Command;

use hale_codegen::build_executable;

fn build_and_run(name: &str, source: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_ws3_int_float_{}", name));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        output.status,
    )
}

#[test]
fn int_to_float_widens() {
    let src = r#"
        fn main() {
            let i: Int = 42;
            let f: Float = std::math::int_to_float(i);
            println("f=", f);
            // result flows into Float arithmetic with no annotation
            let g = std::math::int_to_float(10);
            println("g/4=", g / 4.0);
        }
    "#;
    let (stdout, status) = build_and_run("itof", src);
    assert!(status.success(), "exit: {:?}", status);
    assert!(stdout.contains("f=42"), "got: {:?}", stdout);
    assert!(stdout.contains("g/4=2.5"), "got: {:?}", stdout);
}

#[test]
fn float_to_int_truncates_toward_zero() {
    let src = r#"
        fn main() {
            let n: Int = std::math::float_to_int(3.99);
            println("n=", n);
            let neg: Int = std::math::float_to_int(0.0 - 3.99);
            println("neg=", neg);
            // result flows into Int arithmetic with no annotation
            let k = std::math::float_to_int(7.5);
            println("k+1=", k + 1);
        }
    "#;
    let (stdout, status) = build_and_run("ftoi", src);
    assert!(status.success(), "exit: {:?}", status);
    assert!(stdout.contains("n=3"), "got: {:?}", stdout);
    assert!(stdout.contains("neg=-3"), "round-toward-zero: {:?}", stdout);
    assert!(stdout.contains("k+1=8"), "got: {:?}", stdout);
}

#[test]
fn round_half_away_from_zero() {
    // std::math::round(Float) -> Int. Conventional rounding: half
    // rounds away from zero (3.5 -> 4, -3.5 -> -4), distinct from
    // the truncating float_to_int / trunc. Pure-LLVM lowering
    // (compare + select + fadd + fptosi), no libm symbol.
    let src = r#"
        fn main() {
            let a: Int = std::math::round(3.7);
            let b: Int = std::math::round(3.2);
            let c: Int = std::math::round(2.5);
            let d: Int = std::math::round(0.0 - 2.5);
            let e: Int = std::math::round(0.0 - 3.7);
            println("a=", a, " b=", b, " c=", c, " d=", d, " e=", e);
        }
    "#;
    let (stdout, status) = build_and_run("round", src);
    assert!(status.success(), "exit: {:?}", status);
    assert!(
        stdout.contains("a=4 b=3 c=3 d=-3 e=-4"),
        "round half-away-from-zero: {:?}",
        stdout
    );
}

#[test]
fn trunc_toward_zero() {
    // std::math::trunc(Float) -> Int — the friendlier-named alias
    // of float_to_int (round toward zero).
    let src = r#"
        fn main() {
            let a: Int = std::math::trunc(3.9);
            let b: Int = std::math::trunc(0.0 - 3.9);
            println("a=", a, " b=", b);
        }
    "#;
    let (stdout, status) = build_and_run("trunc", src);
    assert!(status.success(), "exit: {:?}", status);
    assert!(stdout.contains("a=3 b=-3"), "trunc toward zero: {:?}", stdout);
}

#[test]
fn round_trip_is_exact_for_small_integers() {
    let src = r#"
        fn main() {
            let r: Int = std::math::float_to_int(std::math::int_to_float(123));
            println("rt=", r);
        }
    "#;
    let (stdout, status) = build_and_run("rt", src);
    assert!(status.success(), "exit: {:?}", status);
    assert!(stdout.contains("rt=123"), "got: {:?}", stdout);
}

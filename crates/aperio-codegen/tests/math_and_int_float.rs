//! Phase 2c — std::math libm primitives + Int → Float widening.
//!
//! Two friction sub-bullets from notes/aperio-friction.md
//! 2026-05-10 float-surface-gaps:
//!   - "No `Int → Float` coercion at `let nf: Float = self.n;`"
//!   - "No `sqrt`/`exp`/`pow`/`log` in stdlib"
//!
//! End-to-end coverage: every new path through codegen has a
//! representative test. Float-array repetition (the third sub-
//! bullet) is its own follow-up (Phase 2d).

use std::process::Command;

use aperio_codegen::build_executable;

fn build_and_run(name: &str, source: &str) -> (String, std::process::ExitStatus) {
    let program = aperio_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("aperio_test_math_{}", name));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (String::from_utf8_lossy(&output.stdout).to_string(), output.status)
}

#[test]
fn sqrt_returns_float() {
    let src = r#"
        fn main() {
            let r = std::math::sqrt(2.0);
            println("sqrt2=", r);
        }
    "#;
    let (stdout, status) = build_and_run("sqrt", src);
    assert!(status.success(), "exit: {:?}", status);
    // libm sqrt(2.0) ≈ 1.41421356; %g prints "1.41421"
    assert!(
        stdout.contains("sqrt2=1.41421"),
        "got: {:?}",
        stdout
    );
}

#[test]
fn pow_returns_float() {
    let src = r#"
        fn main() {
            let r = std::math::pow(2.0, 10.0);
            println("p=", r);
        }
    "#;
    let (stdout, status) = build_and_run("pow", src);
    assert!(status.success(), "exit: {:?}", status);
    assert!(stdout.contains("p=1024"), "got: {:?}", stdout);
}

#[test]
fn exp_log_round_trip() {
    let src = r#"
        fn main() {
            let r = std::math::log(std::math::exp(1.5));
            println("r=", r);
        }
    "#;
    let (stdout, status) = build_and_run("explog", src);
    assert!(status.success(), "exit: {:?}", status);
    // log(exp(x)) ≈ x; allow %g rounding
    assert!(stdout.contains("r=1.5"), "got: {:?}", stdout);
}

#[test]
fn floor_and_ceil() {
    let src = r#"
        fn main() {
            let f = std::math::floor(3.7);
            let c = std::math::ceil(3.2);
            println("floor=", f, " ceil=", c);
        }
    "#;
    let (stdout, status) = build_and_run("floorceil", src);
    assert!(status.success(), "exit: {:?}", status);
    assert!(stdout.contains("floor=3 ceil=4"), "got: {:?}", stdout);
}

#[test]
fn int_widens_to_float_at_let_ascription() {
    // The canonical friction: `let nf: Float = self.n;` where
    // n is Int. Before this commit, codegen rejected the
    // mismatch; after, the Int widens via sitofp.
    let src = r#"
        locus Counter {
            params {
                n: Int = 7;
            }
            birth() {
                let nf: Float = self.n;
                println("nf=", nf);
            }
        }
        fn main() {
            Counter { };
        }
    "#;
    let (stdout, status) = build_and_run("let_widen", src);
    assert!(status.success(), "exit: {:?}", status);
    assert!(stdout.contains("nf=7"), "got: {:?}", stdout);
}

#[test]
fn int_widens_to_float_at_math_call() {
    // sqrt expects Float; passing Int should widen at the
    // call site, not error.
    let src = r#"
        fn main() {
            let n = 16;
            let r = std::math::sqrt(n);
            println("r=", r);
        }
    "#;
    let (stdout, status) = build_and_run("math_call_widen", src);
    assert!(status.success(), "exit: {:?}", status);
    assert!(stdout.contains("r=4"), "got: {:?}", stdout);
}

#[test]
fn int_widens_to_float_at_user_fn_arg() {
    // Generic case beyond stdlib math: a user fn declaring a
    // Float param accepts an Int arg via sitofp.
    let src = r#"
        fn scale(x: Float) -> Float {
            return x * 1.5;
        }
        fn main() {
            let n = 8;
            let s = scale(n);
            println("s=", s);
        }
    "#;
    let (stdout, status) = build_and_run("user_fn_widen", src);
    assert!(status.success(), "exit: {:?}", status);
    assert!(stdout.contains("s=12"), "got: {:?}", stdout);
}

//! C8 (pond follow-up): `std::math::{tanh, nan, is_nan, inf}` —
//! end-to-end build+run test for the IEEE 754 surface that
//! pond/ml/neural (hand-rolled tanh from exp) and pond/math/matrix
//! (synthesised `nan_sentinel() = 0.0/0.0` + `is_nan(f) = f != f`)
//! reach for. tanh routes through libm via the lotus_math_tanh
//! wrapper; nan / inf return platform-quiet-NaN / +infinity; is_nan
//! is the canonical `f != f` test.
//!
//! NaN-printing caveat: `to_string(nan())` may render as `nan`,
//! `NaN`, or `-nan` depending on the platform printf. Tests assert
//! correctness via `is_nan(x)` rather than by comparing the printed
//! value of NaN itself.

use std::process::Command;

use aperio_codegen::build_executable;

fn build_and_run(name: &str, source: &str) -> std::process::Output {
    let program = aperio_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("lotus_test_math_nan_inf_{}", name));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    out
}

#[test]
fn tanh_zero_is_exactly_zero() {
    // tanh(0) is exactly 0.0 — no rounding-tolerance dance needed.
    // %g prints 0.0 as "0".
    let src = r#"
fn main() {
    let r = std::math::tanh(0.0);
    println("tanh0=", r);
}
"#;
    let out = build_and_run("tanh_zero", src);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("tanh0=0"),
        "expected tanh(0.0) == 0; got stdout: {:?}",
        stdout
    );
}

#[test]
fn tanh_saturates_for_large_input() {
    // tanh saturates to 1 for large positive input. %g prints
    // 1.0 as "1".
    let src = r#"
fn main() {
    let r = std::math::tanh(100.0);
    if r > 0.99 {
        println("tanh-saturates");
    } else {
        println("tanh-broken");
    }
}
"#;
    let out = build_and_run("tanh_sat", src);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("tanh-saturates"),
        "expected tanh(100.0) > 0.99 (saturating); got stdout: {:?}",
        stdout
    );
}

#[test]
fn is_nan_of_nan_is_true() {
    let src = r#"
fn main() {
    let n = std::math::nan();
    if std::math::is_nan(n) {
        println("nan-is-nan");
    } else {
        println("nan-broken");
    }
}
"#;
    let out = build_and_run("isnan_of_nan", src);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("nan-is-nan"),
        "expected is_nan(nan()) to be true; got stdout: {:?}",
        stdout
    );
}

#[test]
fn is_nan_of_inf_is_false() {
    // Infinity is a finite-distance-away sentinel, not NaN. The
    // IEEE 754 is_nan test must return false for +inf.
    let src = r#"
fn main() {
    let x = std::math::inf();
    if std::math::is_nan(x) {
        println("inf-is-nan-broken");
    } else {
        println("inf-not-nan");
    }
}
"#;
    let out = build_and_run("isnan_of_inf", src);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("inf-not-nan"),
        "expected !is_nan(inf()); got stdout: {:?}",
        stdout
    );
}

#[test]
fn inf_is_greater_than_any_finite_float() {
    // +inf > 1e300 confirms inf() really is the IEEE infinity
    // sentinel (any finite double comparison returns true).
    let src = r#"
fn main() {
    let x = std::math::inf();
    if x > 1e300 {
        println("inf-large");
    } else {
        println("inf-broken");
    }
}
"#;
    let out = build_and_run("inf_large", src);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("inf-large"),
        "expected inf() > 1e300; got stdout: {:?}",
        stdout
    );
}

#[test]
fn is_nan_of_ordinary_float_is_false() {
    // Sanity: a regular finite Float must not test as NaN.
    let src = r#"
fn main() {
    if std::math::is_nan(1.5) {
        println("finite-is-nan-broken");
    } else {
        println("finite-not-nan");
    }
}
"#;
    let out = build_and_run("isnan_of_finite", src);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("finite-not-nan"),
        "expected !is_nan(1.5); got stdout: {:?}",
        stdout
    );
}

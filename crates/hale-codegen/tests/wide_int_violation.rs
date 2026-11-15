//! `U256` / `I256` (GH #1934): an operator never wraps. An overflow, a
//! division by zero, a negated `I256::min()`, a negative `Int` made a
//! `U256` and a negative shift count are each a violation where they
//! happen: the process prints what happened and exits 1, and nothing after
//! the operator runs. (`U256::checked_*` and `mul_div` fail instead;
//! `tests/hale/u256_arith_test.hl` holds those.) The i256 and i512
//! arithmetic lowers inline on every target, so each program here also
//! proves it links with no compiler-rt builtin.

use std::process::Command;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

fn run(name: &str, body: &str) -> std::process::Output {
    let src = format!("fn main() {{\n    println(\"before\");\n{body}\n    println(\"after\");\n}}\n");
    let bin = harness::unique_bin(&format!("hale_test_wide_violation_{name}"));
    build_opts::build_source(&src, &bin, &build_opts::options()).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    out
}

fn assert_violation(name: &str, body: &str, want: &str) {
    let out = run(name, body);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{name}: stdout={stdout:?} stderr={stderr:?}");
    assert!(stdout.contains("before") && !stdout.contains("after"), "{name}: stdout={stdout:?}");
    assert!(stderr.contains(want), "{name}: want {want:?} in stderr={stderr:?}");
}

#[test]
fn an_overflow_is_a_violation_never_a_wrapped_value() {
    assert_violation(
        "add",
        "    let x = U256::max() + U256::from_int(1);\n    println(x);",
        "violation: U256 `+` overflows: the result is outside U256's range, 0 through 2^256 - 1",
    );
    assert_violation(
        "sub",
        "    let x = U256::from_int(0) - U256::from_int(1);\n    println(x);",
        "violation: U256 `-` overflows",
    );
    assert_violation(
        "mul",
        "    let x = (U256::from_int(1) << 128) * (U256::from_int(1) << 128);\n    println(x);",
        "violation: U256 `*` overflows",
    );
    assert_violation(
        "smul",
        "    let x = I256::min() * I256::from_int(-1);\n    println(x);",
        "violation: I256 `*` overflows: the result is outside I256's range, -2^255 through 2^255 - 1",
    );
    assert_violation(
        "sadd",
        "    let x = I256::max() + I256::from_int(1);\n    println(x);",
        "violation: I256 `+` overflows",
    );
}

#[test]
fn a_division_by_zero_and_the_one_signed_quotient_past_the_range() {
    assert_violation(
        "div0",
        "    let x = U256::from_int(1) / U256::from_int(0);\n    println(x);",
        "violation: U256 division by zero",
    );
    assert_violation(
        "rem0",
        "    let x = I256::from_int(1) % I256::from_int(0);\n    println(x);",
        "violation: I256 division by zero",
    );
    assert_violation(
        "min_div",
        "    let x = I256::min() / I256::from_int(-1);\n    println(x);",
        "violation: I256 `/` overflows",
    );
    assert_violation(
        "neg_min",
        "    let x = -I256::min();\n    println(x);",
        "violation: I256 `unary -` overflows",
    );
}

#[test]
fn a_negative_int_is_no_u256_and_a_shift_count_is_not_negative() {
    assert_violation(
        "from_neg",
        "    let n = 0 - 3;\n    let x = U256::from_int(n);\n    println(x);",
        "violation: U256::from_int of a negative Int",
    );
    assert_violation(
        "shift_neg",
        "    let n = 0 - 1;\n    let x = U256::from_int(1) << n;\n    println(x);",
        "violation: U256 shift by a negative count",
    );
}

#[test]
fn what_fits_runs_through() {
    let out = run(
        "fits",
        "    let m = U256::max();\n    println(m - U256::from_int(1) + U256::from_int(1) == m);\n    \
         println(I256::min() % I256::from_int(-1));\n    println(U256::mul_div(m, m, m) or U256::from_int(0));",
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "stdout={stdout:?} stderr={:?}", String::from_utf8_lossy(&out.stderr));
    assert!(
        stdout.contains(
            "before\ntrue\n0\n115792089237316195423570985008687907853269984665640564039457584007913129639935\nafter"
        ),
        "{stdout:?}"
    );
}

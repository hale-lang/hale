//! m78: std::str — minimal string parsing primitives.

use std::process::Command;

use aperio_codegen::build_executable;

fn build_and_run(name: &str, source: &str) -> (String, std::process::ExitStatus) {
    let program = aperio_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("aperio_test_stdlib_str_{}", name));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (String::from_utf8_lossy(&output.stdout).to_string(), output.status)
}

#[test]
fn parse_int_handles_basic_digit_strings() {
    // 2026-05-17 — parse_int returns `Int fallible(ParseError)`.
    // For known-valid inputs the test uses `or raise` to surface
    // an interpreter panic if the parser ever rejects something
    // it should have accepted.
    let src = r#"
        fn main() {
            let a = std::str::parse_int("42") or raise;
            let b = std::str::parse_int("0") or raise;
            let c = std::str::parse_int("-7") or raise;
            let d = std::str::parse_int("9999999999") or raise;
            println("a=", a);
            println("b=", b);
            println("c=", c);
            println("d=", d);
        }
    "#;
    let (stdout, status) = build_and_run("basic", src);
    assert!(status.success());
    assert!(stdout.contains("a=42"), "got: {:?}", stdout);
    assert!(stdout.contains("b=0"), "got: {:?}", stdout);
    assert!(stdout.contains("c=-7"), "got: {:?}", stdout);
    assert!(stdout.contains("d=9999999999"), "got: {:?}", stdout);
}

#[test]
fn parse_int_err_arm_substitutes_zero_on_garbage_input() {
    // The fallible flip means "garbage in" routes through `or`
    // rather than silently returning 0. Test uses `or 0` as the
    // substitute so the expected sentinel still appears.
    let src = r#"
        fn main() {
            let bad1 = std::str::parse_int("abc") or 0;
            let bad2 = std::str::parse_int("12abc") or 0;
            let bad3 = std::str::parse_int("") or 0;
            let bad4 = std::str::parse_int("  42  ") or 0;
            println("bad1=", bad1);
            println("bad2=", bad2);
            println("bad3=", bad3);
            println("bad4=", bad4);
        }
    "#;
    let (stdout, status) = build_and_run("garbage", src);
    assert!(status.success());
    // strtoll-ish: trailing non-NUL chars reject. "  42  "
    // rejects because the trailing spaces aren't consumed.
    assert!(stdout.contains("bad1=0"), "got: {:?}", stdout);
    assert!(stdout.contains("bad2=0"), "got: {:?}", stdout);
    assert!(stdout.contains("bad3=0"), "got: {:?}", stdout);
    assert!(stdout.contains("bad4=0"), "got: {:?}", stdout);
}

#[test]
fn parse_int_err_payload_carries_kind_and_input() {
    // The substitute RHS sees `err: ParseError { kind, input }`
    // — both fields readable in scope for diagnostics / logging.
    let src = r#"
        fn main() {
            let s = "totally bogus";
            let n = std::str::parse_int(s) or {
                println("kind=", err.kind, " input=", err.input);
                -1
            };
            println("n=", n);
        }
    "#;
    let (stdout, status) = build_and_run("err_payload", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("kind=parse_int"), "got: {:?}", stdout);
    assert!(stdout.contains("input=totally bogus"), "got: {:?}", stdout);
    assert!(stdout.contains("n=-1"), "got: {:?}", stdout);
}

#[test]
fn can_parse_int_distinguishes_valid_from_invalid() {
    let src = r#"
        fn main() {
            let v1 = std::str::can_parse_int("42");
            let v2 = std::str::can_parse_int("-7");
            let v3 = std::str::can_parse_int("abc");
            let v4 = std::str::can_parse_int("");
            let v5 = std::str::can_parse_int("0");
            println("v1=", v1);
            println("v2=", v2);
            println("v3=", v3);
            println("v4=", v4);
            println("v5=", v5);
        }
    "#;
    let (stdout, status) = build_and_run("can_parse", src);
    assert!(status.success());
    assert!(stdout.contains("v1=true"), "got: {:?}", stdout);
    assert!(stdout.contains("v2=true"), "got: {:?}", stdout);
    assert!(stdout.contains("v3=false"), "got: {:?}", stdout);
    assert!(stdout.contains("v4=false"), "got: {:?}", stdout);
    assert!(stdout.contains("v5=true"), "got: {:?}", stdout);
}

#[test]
fn parse_int_round_trips_with_arithmetic() {
    // Confirms the parsed Int actually behaves as Int — can be
    // compared, arithmetic'd, etc. — not some opaque thing.
    let src = r#"
        fn main() {
            let n = std::str::parse_int("100") or raise;
            let doubled = n * 2;
            if n > 50 {
                println("doubled=", doubled);
            }
        }
    "#;
    let (stdout, status) = build_and_run("arithmetic", src);
    assert!(status.success());
    assert!(stdout.contains("doubled=200"), "got: {:?}", stdout);
}

#[test]
fn parse_decimal_handles_basic_inputs() {
    // 2026-05-20 — parse_decimal returns `Decimal fallible(ParseError)`.
    // Mantissa is i128 with implicit scale 9 (matches Decimal literal
    // codegen). Trailing-zero precision survives — the IEEE 754
    // rounding that bit parse_float on high-precision decimals doesn't apply.
    let src = r#"
        fn main() {
            let a = std::str::parse_decimal("100.5") or raise;
            let b = std::str::parse_decimal("0") or raise;
            let c = std::str::parse_decimal("-7.25") or raise;
            let d = std::str::parse_decimal("0.00005100") or raise;
            let e = std::str::parse_decimal("12345.678901234") or raise;
            println("a=", a);
            println("b=", b);
            println("c=", c);
            println("d=", d);
            println("e=", e);
        }
    "#;
    let (stdout, status) = build_and_run("parse_decimal_basic", src);
    assert!(status.success());
    assert!(stdout.contains("a=100.5"), "got: {:?}", stdout);
    assert!(stdout.contains("b=0"), "got: {:?}", stdout);
    assert!(stdout.contains("c=-7.25"), "got: {:?}", stdout);
    // Trailing zeros past 9 fractional digits get truncated, but
    // 8 digits round-trip — high-precision decimal use case.
    assert!(stdout.contains("d=0.000051"), "got: {:?}", stdout);
    assert!(stdout.contains("e=12345.678901234"), "got: {:?}", stdout);
}

#[test]
fn parse_decimal_err_arm_substitutes_zero_on_garbage_input() {
    let src = r#"
        fn main() {
            let bad1 = std::str::parse_decimal("abc") or 0.0d;
            let bad2 = std::str::parse_decimal("12.3abc") or 0.0d;
            let bad3 = std::str::parse_decimal("") or 0.0d;
            let bad4 = std::str::parse_decimal(".") or 0.0d;
            println("bad1=", bad1);
            println("bad2=", bad2);
            println("bad3=", bad3);
            println("bad4=", bad4);
        }
    "#;
    let (stdout, status) = build_and_run("parse_decimal_garbage", src);
    assert!(status.success());
    assert!(stdout.contains("bad1=0"), "got: {:?}", stdout);
    assert!(stdout.contains("bad2=0"), "got: {:?}", stdout);
    assert!(stdout.contains("bad3=0"), "got: {:?}", stdout);
    assert!(stdout.contains("bad4=0"), "got: {:?}", stdout);
}

#[test]
fn parse_decimal_err_payload_carries_kind_and_input() {
    let src = r#"
        fn main() {
            let s = "not a number";
            let v = std::str::parse_decimal(s) or {
                println("kind=", err.kind, " input=", err.input);
                -1.0d
            };
            println("v=", v);
        }
    "#;
    let (stdout, status) = build_and_run("parse_decimal_err_payload", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("kind=parse_decimal"), "got: {:?}", stdout);
    assert!(stdout.contains("input=not a number"), "got: {:?}", stdout);
    assert!(stdout.contains("v=-1"), "got: {:?}", stdout);
}

#[test]
fn parse_decimal_round_trips_through_arithmetic() {
    // Confirms the parsed Decimal behaves as Decimal — i128
    // mantissa arithmetic survives the fallible flip.
    let src = r#"
        fn main() {
            let p = std::str::parse_decimal("100.40") or raise;
            let q = std::str::parse_decimal("0.005") or raise;
            let total = p + q;
            println("total=", total);
        }
    "#;
    let (stdout, status) = build_and_run("parse_decimal_arith", src);
    assert!(status.success());
    assert!(stdout.contains("total=100.405"), "got: {:?}", stdout);
}

#[test]
fn std_str_parse_error_qualified_path_resolves() {
    // v1.x polish (2026-05-20): `std::str::ParseError` resolves
    // to the same struct the stdlib's parse_* fns inject. Lets
    // users disambiguate explicitly in fn signatures and `as e`
    // bindings — useful when a project also has its own local
    // error types.
    let src = r#"
        fn handle(e: std::str::ParseError) -> Int {
            println("kind=", e.kind);
            println("input=", e.input);
            return -1;
        }
        fn main() {
            let v = std::str::parse_int("nope") or handle(err);
            println("v=", v);
        }
    "#;
    let (stdout, status) = build_and_run("qualified_path", src);
    assert!(status.success(), "build/run failed: {:?}", stdout);
    assert!(
        stdout.contains("kind=parse_int") && stdout.contains("input=nope"),
        "expected qualified-path handler to see stdlib ParseError fields, \
         got stdout: {:?}",
        stdout
    );
}

#[test]
fn std_str_parse_user_parse_error_collision_diagnoses_cleanly() {
    // v1.x polish (2026-05-20): when a user declares their own
    // `type ParseError` with non-stdlib-compatible fields, the
    // codegen previously panicked with `ParseError.kind field`.
    // Now it returns a clean diagnostic naming the fix paths.
    let src = r#"
        type ParseError { msg: String; venue: String; }
        fn handle(e: ParseError) -> Int { let _ = e; return -1; }
        fn main() {
            let v = std::str::parse_int("nope") or handle(err);
            let _ = v;
        }
    "#;
    let program = aperio_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push("aperio_test_stdlib_str_collision");
    let result = build_executable(&program, &bin);
    let _ = std::fs::remove_file(&bin);
    let err = match result {
        Err(e) => e,
        Ok(()) => panic!("expected codegen failure, but build succeeded"),
    };
    let msg = format!("{:?}", err);
    assert!(
        msg.contains("user-declared `type ParseError`")
            && msg.contains("std::str::ParseError"),
        "expected clean diag naming the qualified-path fix, got: {}",
        msg
    );
}

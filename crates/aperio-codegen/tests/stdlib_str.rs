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
    let src = r#"
        fn main() {
            let a = std::str::parse_int("42");
            let b = std::str::parse_int("0");
            let c = std::str::parse_int("-7");
            let d = std::str::parse_int("9999999999");
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
fn parse_int_returns_zero_on_garbage_input() {
    let src = r#"
        fn main() {
            let bad1 = std::str::parse_int("abc");
            let bad2 = std::str::parse_int("12abc");
            let bad3 = std::str::parse_int("");
            let bad4 = std::str::parse_int("  42  ");
            println("bad1=", bad1);
            println("bad2=", bad2);
            println("bad3=", bad3);
            println("bad4=", bad4);
        }
    "#;
    let (stdout, status) = build_and_run("garbage", src);
    assert!(status.success());
    // strtoll's strict check: trailing non-NUL chars reject.
    // "  42  " rejects because the trailing spaces aren't
    // consumed.
    assert!(stdout.contains("bad1=0"), "got: {:?}", stdout);
    assert!(stdout.contains("bad2=0"), "got: {:?}", stdout);
    assert!(stdout.contains("bad3=0"), "got: {:?}", stdout);
    assert!(stdout.contains("bad4=0"), "got: {:?}", stdout);
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
            let n = std::str::parse_int("100");
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

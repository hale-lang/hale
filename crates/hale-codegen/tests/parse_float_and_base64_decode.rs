//! v1.x-16: `std::str::parse_float`, `std::str::can_parse_float`,
//! and `std::text::base64::decode`.
//!
//! parse_float mirrors parse_int's contract (strict trailing-NUL,
//! 0.0 on failure, paired with can_parse_float for the boolean
//! version). base64::decode is the inverse of base64::encode; it
//! returns a Bytes blob and rejects non-alphabet input by
//! returning an empty Bytes.

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn build_and_run(name: &str, source: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(source).expect("parse");
    let bin = harness::unique_bin(&format!("hale_test_parse_b64_{}", name));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (String::from_utf8_lossy(&output.stdout).to_string(), output.status)
}

#[test]
fn parse_float_basic() {
    // 2026-05-17 — parse_float returns Float fallible(ParseError);
    // known-valid input uses `or raise`.
    let src = r#"
        fn main() {
            let f = std::str::parse_float("3.14159") or raise;
            println(f);
        }
    "#;
    let (stdout, status) = build_and_run("basic", src);
    assert!(status.success());
    assert!(stdout.trim().starts_with("3.14"), "got: {:?}", stdout);
}

#[test]
fn parse_float_err_arm_substitutes_zero_on_failure() {
    // Garbage input routes through the err arm rather than
    // returning 0.0 silently. Use `or 0.0` substitute.
    let src = r#"
        fn main() {
            let f = std::str::parse_float("not a number") or 0.0;
            println(f);
        }
    "#;
    let (stdout, status) = build_and_run("fail_zero", src);
    assert!(status.success());
    assert!(stdout.trim() == "0", "got: {:?}", stdout);
}

#[test]
fn can_parse_float_discriminates() {
    let src = r#"
        fn main() {
            let yes = std::str::can_parse_float("2.5");
            let no = std::str::can_parse_float("nope");
            println(f"yes={yes} no={no}");
        }
    "#;
    let (stdout, status) = build_and_run("can_parse", src);
    assert!(status.success());
    assert!(
        stdout.contains("yes=true") && stdout.contains("no=false"),
        "got: {:?}", stdout
    );
}

#[test]
fn parse_float_round_trip_through_arithmetic() {
    let src = r#"
        fn main() {
            let a = std::str::parse_float("2.5") or raise;
            let b = std::str::parse_float("1.5") or raise;
            let s = a + b;
            println(s);
        }
    "#;
    let (stdout, status) = build_and_run("round_trip", src);
    assert!(status.success());
    assert!(stdout.trim() == "4", "got: {:?}", stdout);
}

#[test]
fn base64_decode_hello() {
    // "hello" encoded is "aGVsbG8="; decoded length is 5.
    let src = r#"
        fn main() {
            let b = std::text::base64::decode("aGVsbG8=");
            println(f"len={len(b)}");
        }
    "#;
    let (stdout, status) = build_and_run("decode_hello", src);
    assert!(status.success());
    assert!(stdout.contains("len=5"), "got: {:?}", stdout);
}

#[test]
fn base64_decode_round_trip_against_encode() {
    // encode then decode then re-encode — second encode should
    // match the first, end-to-end check on the codec.
    let src = r#"
        fn main() {
            let bs = std::bytes::from_string("hello world");
            let enc1 = std::text::base64::encode(bs);
            let dec = std::text::base64::decode(enc1);
            let enc2 = std::text::base64::encode(dec);
            println(f"enc1={enc1}");
            println(f"enc2={enc2}");
            if enc1 == enc2 {
                println("round-trip-ok");
            }
        }
    "#;
    let (stdout, status) = build_and_run("round_trip_b64", src);
    assert!(status.success());
    assert!(
        stdout.contains("round-trip-ok"),
        "encode/decode/encode didn't round-trip; got: {:?}",
        stdout
    );
}

#[test]
fn base64_decode_rejects_garbage_returns_empty() {
    let src = r#"
        fn main() {
            let b = std::text::base64::decode("not!base64@@");
            println(f"len={len(b)}");
        }
    "#;
    let (stdout, status) = build_and_run("garbage", src);
    assert!(status.success());
    assert!(stdout.contains("len=0"), "got: {:?}", stdout);
}

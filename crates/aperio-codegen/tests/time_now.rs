//! `std::time::now() -> Int` — end-to-end build+run test for the
//! C7 (pond follow-up) wall-clock seconds-since-epoch primitive.
//! Verifies the spec entry in spec/stdlib.md is real rather than
//! aspirational. Surfacing consumer: pond/sessions cookie
//! expiries, which must survive a process restart (the monotonic
//! origin resets at boot; the wall-clock origin does not).

use std::process::Command;

use aperio_codegen::build_executable;

fn build_and_run(name: &str, source: &str) -> std::process::Output {
    let program = aperio_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("lotus_test_{}", name));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    out
}

#[test]
fn now_returns_seconds_since_epoch() {
    // 1700000000 is 2023-11-14T22:13:20Z — anything after that
    // is a real wall-clock value (and the test runs in late 2025
    // or later given the spec ship date).
    let src = r#"
fn main() {
    let t = std::time::now();
    if t > 1700000000 {
        println("now-after-2023");
    } else {
        println("now-too-early");
    }
}
"#;
    let out = build_and_run("time_now_basic", src);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("now-after-2023"),
        "expected wall-clock seconds > 1700000000; got stdout: {:?}",
        stdout
    );
}

#[test]
fn now_returns_int_and_arithmetic_works() {
    // Confirms the path-call expression is an Int, not a Duration:
    // arithmetic against an Int literal type-checks and lowers.
    let src = r#"
fn main() {
    let t = std::time::now();
    let plus_one = t + 1;
    if plus_one > t {
        println("arithmetic-ok");
    } else {
        println("arithmetic-broken");
    }
}
"#;
    let out = build_and_run("time_now_arithmetic", src);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("arithmetic-ok"),
        "expected std::time::now() to type as Int and admit + 1; \
         got stdout: {:?}",
        stdout
    );
}

#[test]
fn now_monotonic_progression() {
    // Two consecutive calls should produce non-decreasing values
    // — CLOCK_REALTIME can drift backward under NTP slewing, but
    // not within a single test run's microseconds. We assert the
    // second sample is >= the first.
    let src = r#"
fn main() {
    let a = std::time::now();
    let b = std::time::now();
    if b >= a {
        println("non-decreasing");
    } else {
        println("backward");
    }
}
"#;
    let out = build_and_run("time_now_progression", src);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("non-decreasing"),
        "expected two consecutive std::time::now() calls to be \
         non-decreasing; got stdout: {:?}",
        stdout
    );
}

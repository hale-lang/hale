//! ws-echo friction `random-seed-missing` — `std::rand::next_int`
//! + `std::rand::seed_from_time`. Cheap xorshift64* RNG for
//! nonces / jitter / shuffles. NOT cryptographic.

use std::process::Command;

use aperio_codegen::build_executable;

fn build(name: &str, src: &str) -> std::path::PathBuf {
    let program = aperio_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("aperio_test_rand_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

#[test]
fn next_int_stays_in_range_and_varies() {
    // Pull 8 values from [0, 100). They should all be in
    // range, and at least 5 of 8 should be distinct (with
    // 100 buckets, collisions are rare).
    let src = r#"
        fn main() {
            std::rand::seed_from_time();
            let mut i = 0;
            while i < 8 {
                let v = std::rand::next_int(100);
                println("v=", v);
                i = i + 1;
            }
        }
    "#;
    let bin = build("range_vary", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut vals: Vec<i64> = Vec::new();
    for line in stdout.lines() {
        if let Some(s) = line.strip_prefix("v=") {
            let v: i64 = s.trim().parse().expect("int parse");
            vals.push(v);
        }
    }
    assert_eq!(vals.len(), 8, "want 8 values; got {:?}", vals);
    for v in &vals {
        assert!(
            *v >= 0 && *v < 100,
            "out of range: {} in {:?}",
            v,
            vals
        );
    }
    let mut sorted = vals.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert!(
        sorted.len() >= 5,
        "want >=5 distinct values, got {}: {:?}",
        sorted.len(),
        vals
    );
}

#[test]
fn next_int_zero_max_returns_zero() {
    let src = r#"
        fn main() {
            let v = std::rand::next_int(0);
            println("v=", v);
        }
    "#;
    let bin = build("zero_max", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("v=0"), "got: {:?}", stdout);
}

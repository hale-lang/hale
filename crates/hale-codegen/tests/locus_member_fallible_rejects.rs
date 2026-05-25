//! Open-question #24 MVP rejection tests. The v0.1 ship
//! restricts fallible success / err types to value-only (no
//! heap pointer in the payload bytes); this file pins the
//! rejection diagnostics so the constraint is visible to
//! callers and a future v0.2 widening of the rule can find +
//! retire these tests cleanly.

use hale_codegen::build_executable;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn unique_path(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "lt-locus-fallible-reject-{}-{}-{}.bin",
        tag,
        std::process::id(),
        nanos,
    ));
    p
}

fn build_err(src: &str) -> String {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = unique_path("reject");
    let err = build_executable(&program, &bin)
        .err()
        .map(|e| format!("{:?}", e))
        .unwrap_or_else(|| "expected build to fail".to_string());
    let _ = std::fs::remove_file(&bin);
    err
}

fn build_ok(src: &str) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = unique_path("ok");
    build_executable(&program, &bin).expect("build");
    let _ = std::fs::remove_file(&bin);
}

fn build_and_run(src: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = unique_path("run");
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status,
    )
}

#[test]
fn rejects_string_in_err_payload() {
    let src = r#"
        type E { msg: String; }
        locus L {
            fn check() -> Int fallible(E) {
                fail E { msg: "boom" };
            }
            run() { let v = self.check() or 0; println(v); }
        }
        fn main() { L { }; }
    "#;
    let err = build_err(src);
    assert!(
        err.contains("heap-bearing") && err.contains("v0.1"),
        "expected v0.1 heap-payload rejection; got: {}",
        err
    );
}

#[test]
fn rejects_string_success_type() {
    let src = r#"
        type E { code: Int; }
        locus L {
            fn name() -> String fallible(E) {
                return "hi";
            }
            run() { let v = self.name() or "?"; println(v); }
        }
        fn main() { L { }; }
    "#;
    let err = build_err(src);
    assert!(
        err.contains("heap-bearing") && err.contains("v0.1"),
        "expected v0.1 heap-return rejection; got: {}",
        err
    );
}

#[test]
fn rejects_nested_string_field_in_err_struct() {
    // Recursive flat check: an err struct with an inner struct
    // whose field is String is still heap-bearing.
    let src = r#"
        type Inner { tag: String; }
        type E { code: Int; inner: Inner; }
        locus L {
            fn check() -> Int fallible(E) {
                fail E { code: 1, inner: Inner { tag: "x" } };
            }
            run() { let v = self.check() or 0; println(v); }
        }
        fn main() { L { }; }
    "#;
    let err = build_err(src);
    assert!(
        err.contains("heap-bearing"),
        "expected recursive heap-check to fire; got: {}",
        err
    );
}

#[test]
fn accepts_nested_flat_struct_in_err_payload() {
    // Recursive flat check should pass for an all-flat nested
    // shape. Verify it actually builds + runs.
    let src = r#"
        type Inner { tag: Int; flag: Bool; }
        type E { code: Int; inner: Inner; }
        locus L {
            fn check(x: Int) -> Int fallible(E) {
                if x < 0 {
                    fail E { code: 1, inner: Inner { tag: 42, flag: true } };
                }
                return x;
            }
            run() {
                let v = self.check(7) or 0;
                println("v=", v);
                let w = self.check(-1) or err.inner.tag;
                println("w=", w);
            }
        }
        fn main() { L { }; }
    "#;
    build_ok(src);
    let (stdout, status) = build_and_run(src);
    assert!(status.success(), "non-zero: {:?}\n{}", status, stdout);
    assert!(stdout.contains("v=7"), "got: {}", stdout);
    assert!(stdout.contains("w=42"), "got: {}", stdout);
}

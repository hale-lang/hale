//! Open-question #24 MVP (2026-05-25) — user-declared locus
//! member fns may now carry `fallible(E)`. Black-box behavioral
//! coverage of the three call-site dispositions (`or raise` /
//! `or <substitute>` / `or <substitute-with-err-binding>`)
//! across both the ok and fail paths. Also covers the
//! cross-locus dispatch shape (calling a fallible method on a
//! locus-typed field, not just `self.method()`).
//!
//! v0.1 scope is value-only success / err payloads (no String /
//! Bytes / heap-bearing fields). Heap-bearing payloads are
//! rejected at codegen with a clear diagnostic; tests covering
//! that diagnostic live in `locus_member_fallible_rejects.rs`.

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use hale_codegen::build_executable;

fn unique_path(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "lt-locus-member-fallible-{}-{}-{}.bin",
        tag,
        std::process::id(),
        nanos,
    ));
    p
}

fn build_and_run(tag: &str, src: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = unique_path(tag);
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run binary");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    (stdout, out.status)
}

#[test]
fn self_method_or_substitute_ok_and_fail() {
    // `check(x)` returns x when x >= 0, fails with code=42 otherwise.
    // Three call sites:
    //  - fail with literal substitute → 999
    //  - ok path → returns 7
    //  - fail with err-binding-using substitute → 1042
    let src = r#"
        type E { code: Int; }
        locus Calc {
            fn check(x: Int) -> Int fallible(E) {
                if x < 0 { fail E { code: 42 }; }
                return x;
            }
            fn fallback(e: E) -> Int { return e.code + 1000; }
            run() {
                let v = self.check(-5) or 999;
                println("v=", v);
                let w = self.check(7) or 999;
                println("w=", w);
                let z = self.check(-1) or self.fallback(err);
                println("z=", z);
            }
        }
        fn main() { Calc { }; }
    "#;
    let (stdout, status) = build_and_run("subst", src);
    assert!(status.success(), "non-zero: {:?}\n{}", status, stdout);
    assert!(stdout.contains("v=999"), "got: {}", stdout);
    assert!(stdout.contains("w=7"), "got: {}", stdout);
    assert!(stdout.contains("z=1042"), "got: {}", stdout);
}

#[test]
fn self_method_or_raise_propagates_to_root() {
    // `or raise` on a fallible self-method call exits non-zero
    // (the root panic path) when the failure fires. Ok path
    // continues cleanly.
    let src_ok = r#"
        type E { code: Int; }
        locus Calc {
            fn check(x: Int) -> Int fallible(E) {
                if x < 0 { fail E { code: 1 }; }
                return x * 2;
            }
            run() {
                let v = self.check(5) or raise;
                println("ok=", v);
            }
        }
        fn main() { Calc { }; }
    "#;
    let (stdout, status) = build_and_run("raise_ok", src_ok);
    assert!(status.success(), "ok path should succeed: {:?}\n{}", status, stdout);
    assert!(stdout.contains("ok=10"), "got: {}", stdout);

    let src_fail = r#"
        type E { code: Int; }
        locus Calc {
            fn check(x: Int) -> Int fallible(E) {
                if x < 0 { fail E { code: 1 }; }
                return x * 2;
            }
            run() {
                let v = self.check(-5) or raise;
                println("should-not-print=", v);
            }
        }
        fn main() { Calc { }; }
    "#;
    let (stdout, status) = build_and_run("raise_fail", src_fail);
    assert!(
        !status.success(),
        "fail path should exit non-zero (root panic); stdout: {}",
        stdout
    );
    assert!(
        !stdout.contains("should-not-print"),
        "ok branch should not have run on fail path; stdout: {}",
        stdout
    );
}

#[test]
fn cross_locus_method_fallible_call() {
    // Fallible call on a locus-typed field — not `self.method()`
    // but `child.method()`. The dispatch site is the same
    // `lower_fallible_method_call`; receiver lowers to a
    // LocusRef and we look up the method on that locus.
    let src = r#"
        type E { code: Int; }
        locus Inner {
            fn try_double(x: Int) -> Int fallible(E) {
                if x < 0 { fail E { code: 7 }; }
                return x + x;
            }
        }
        locus Outer {
            params { i: Inner = Inner { }; }
            run() {
                let a = self.i.try_double(10) or 0;
                println("a=", a);
                let b = self.i.try_double(-1) or 0;
                println("b=", b);
            }
        }
        fn main() { Outer { }; }
    "#;
    let (stdout, status) = build_and_run("cross", src);
    assert!(status.success(), "non-zero: {:?}\n{}", status, stdout);
    assert!(stdout.contains("a=20"), "got: {}", stdout);
    assert!(stdout.contains("b=0"), "got: {}", stdout);
}

#[test]
fn err_field_access_in_substitute() {
    // The substitute RHS sees `err` bound to the payload type;
    // `err.field` reads through it. Pins the err-binding shape
    // works on method-call fallibles the same as free-fn
    // fallibles.
    let src = r#"
        type E { code: Int; tag: Int; }
        locus L {
            fn fail_at(x: Int) -> Int fallible(E) {
                fail E { code: x, tag: 999 };
            }
            run() {
                let v = self.fail_at(5) or err.code;
                println("code=", v);
                let w = self.fail_at(11) or err.tag;
                println("tag=", w);
            }
        }
        fn main() { L { }; }
    "#;
    let (stdout, status) = build_and_run("err_field", src);
    assert!(status.success(), "non-zero: {:?}\n{}", status, stdout);
    assert!(stdout.contains("code=5"), "got: {}", stdout);
    assert!(stdout.contains("tag=999"), "got: {}", stdout);
}

#[test]
fn fallible_method_with_int_payload_no_struct() {
    // Simplest possible shape: Int err payload, Int success.
    // No struct construction in the fail path. Confirms the
    // value-only ABI works for bare primitives.
    let src = r#"
        locus L {
            fn parse(s: Int) -> Int fallible(Int) {
                if s < 0 { fail 1; }
                return s + 1;
            }
            run() {
                let v = self.parse(5) or 999;
                println("v=", v);
                let w = self.parse(-1) or err;
                println("w=", w);
            }
        }
        fn main() { L { }; }
    "#;
    let (stdout, status) = build_and_run("int_payload", src);
    assert!(status.success(), "non-zero: {:?}\n{}", status, stdout);
    assert!(stdout.contains("v=6"), "got: {}", stdout);
    assert!(stdout.contains("w=1"), "got: {}", stdout);
}

#[test]
fn fallible_method_with_unit_success_or_discard() {
    // Unit success: `fn op() -> () fallible(E)`. The `or
    // discard` disposition swallows the err and produces no
    // value. Confirms the Unit-success ABI (no out_val sret
    // slot) works on locus methods too.
    let src = r#"
        type E { code: Int; }
        locus L {
            params { tries: Int = 0; }
            fn ensure(x: Int) fallible(E) {
                if x < 0 { fail E { code: x }; }
            }
            run() {
                self.ensure(1) or discard;
                println("after-ok");
                self.ensure(-1) or discard;
                println("after-fail-discard");
            }
        }
        fn main() { L { }; }
    "#;
    let (stdout, status) = build_and_run("unit_discard", src);
    assert!(status.success(), "non-zero: {:?}\n{}", status, stdout);
    assert!(stdout.contains("after-ok"), "got: {}", stdout);
    assert!(stdout.contains("after-fail-discard"), "got: {}", stdout);
}

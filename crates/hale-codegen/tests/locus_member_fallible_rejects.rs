//! Open-question #24 v0.2 (2026-05-25) — heap-bearing
//! fallible payloads on user-declared locus member fns. The
//! v0.1 ship gated `String` / `Bytes` / struct-with-heap-fields
//! types at codegen; v0.2 lifts that gate by wiring the same
//! TLS-snapshot caller_arena + `emit_method_return_deep_copy`
//! plumbing that non-fallible heap-returning locus methods
//! already use.
//!
//! These tests were the v0.1 rejection cases; v0.2 turns them
//! into positive cases — they should build cleanly + roundtrip
//! the heap-bearing payload correctly. The file name is kept
//! for git-blame continuity (the test bodies tell the story).

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
        "lt-locus-fallible-heap-{}-{}-{}.bin",
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
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status,
    )
}

#[test]
fn string_in_err_payload_now_works() {
    // v0.1 rejected this at codegen. v0.2 deep-copies the
    // String payload into caller_arena on the fail branch;
    // `err.msg` reads through the survived bytes.
    let src = r#"
        type E { msg: String; }
        locus L {
            fn check(x: Int) -> Int fallible(E) {
                if x < 0 { fail E { msg: "neg" }; }
                return x;
            }
            run() {
                let v = self.check(5) or 0;
                println("v=", v);
                let w = self.check(-1) or 99;
                println("w=", w);
                // Round-trip the heap-bearing payload through
                // a substitute that captures err and reads
                // err.msg — proves the String survived the
                // scratch-destroy + lands in caller_arena.
                let _ = self.check(-2) or self.echo(err.msg);
            }
            fn echo(s: String) -> Int {
                println("echo:", s);
                return 0;
            }
        }
        fn main() { L { }; }
    "#;
    let (stdout, status) = build_and_run("str_err", src);
    assert!(status.success(), "non-zero: {:?}\n{}", status, stdout);
    assert!(stdout.contains("v=5"), "got: {}", stdout);
    assert!(stdout.contains("w=99"), "got: {}", stdout);
    assert!(stdout.contains("echo:neg"), "got: {}", stdout);
}

#[test]
fn string_success_type_now_works() {
    // v0.1 rejected String returns from fallible methods.
    // v0.2 deep-copies the String into caller_arena on the
    // ok branch.
    let src = r#"
        type E { code: Int; }
        locus L {
            fn name(id: Int) -> String fallible(E) {
                if id < 0 { fail E { code: 1 }; }
                return "ok-name";
            }
            run() {
                let v = self.name(7) or "?";
                println("v=", v);
                let w = self.name(-1) or "fallback";
                println("w=", w);
            }
        }
        fn main() { L { }; }
    "#;
    let (stdout, status) = build_and_run("str_succ", src);
    assert!(status.success(), "non-zero: {:?}\n{}", status, stdout);
    assert!(stdout.contains("v=ok-name"), "got: {}", stdout);
    assert!(stdout.contains("w=fallback"), "got: {}", stdout);
}

#[test]
fn nested_string_field_in_err_struct_now_works() {
    // Recursive struct-with-String case. The err payload type
    // `E { code: Int; inner: Inner { tag: String; } }` carries
    // a heap pointer one nesting level deep; the deep-copy
    // recurses through TypeRef fields and anchors the String
    // in caller_arena.
    let src = r#"
        type Inner { tag: String; }
        type E { code: Int; inner: Inner; }
        locus L {
            fn check(x: Int) -> Int fallible(E) {
                if x < 0 {
                    fail E { code: 7, inner: Inner { tag: "nested" } };
                }
                return x;
            }
            run() {
                let v = self.check(5) or 0;
                println("v=", v);
                let w = self.check(-1) or 99;
                println("w=", w);
                let _ = self.check(-2) or self.echo(err.inner.tag);
            }
            fn echo(s: String) -> Int {
                println("echo:", s);
                return 0;
            }
        }
        fn main() { L { }; }
    "#;
    let (stdout, status) = build_and_run("nested_str", src);
    assert!(status.success(), "non-zero: {:?}\n{}", status, stdout);
    assert!(stdout.contains("v=5"), "got: {}", stdout);
    assert!(stdout.contains("w=99"), "got: {}", stdout);
    assert!(stdout.contains("echo:nested"), "got: {}", stdout);
}

#[test]
fn flat_struct_payload_still_works_post_v02() {
    // Regression guard. The v0.1-allowed flat-struct path
    // shouldn't have regressed under the v0.2 deep-copy
    // change. Scalars / no-heap fields pass through the
    // copy as identity stores.
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
    let (stdout, status) = build_and_run("flat", src);
    assert!(status.success(), "non-zero: {:?}\n{}", status, stdout);
    assert!(stdout.contains("v=7"), "got: {}", stdout);
    assert!(stdout.contains("w=42"), "got: {}", stdout);
}

#[test]
fn struct_success_type_with_string_field_now_works() {
    // Heap-bearing struct as the success type. The ok-branch
    // deep-copy anchors the inner String in caller_arena
    // before the callee's scratch goes away.
    let src = r#"
        type Reply { tag: String; n: Int; }
        type E { code: Int; }
        locus L {
            fn build(n: Int) -> Reply fallible(E) {
                if n < 0 { fail E { code: 1 }; }
                return Reply { tag: "built", n: n };
            }
            run() {
                let r = self.build(3) or Reply { tag: "fallback", n: 0 };
                println("tag=", r.tag);
                println("n=", r.n);
            }
        }
        fn main() { L { }; }
    "#;
    let (stdout, status) = build_and_run("struct_succ", src);
    assert!(status.success(), "non-zero: {:?}\n{}", status, stdout);
    assert!(stdout.contains("tag=built"), "got: {}", stdout);
    assert!(stdout.contains("n=3"), "got: {}", stdout);
}

//! FORM-vec hot-path inline regression.
//!
//! `@form(vec)`'s `.get` and `.push` no longer lower to opaque
//! `lotus_vec_get` / `lotus_vec_push` C calls on the common path —
//! they inline to a bounds-check + GEP + load/store, with the cold
//! cases (OOB on get, grow on push) keeping the original behavior.
//! These tests pin the observable semantics that must stay
//! identical after the inline: exact value round-trip, OOB raising,
//! growth across the doubling boundary, and heap-elem deep-copy of
//! a method-body push.

use std::process::Command;

use hale_codegen::build_executable;

fn build(name: &str, src: &str) -> std::path::PathBuf {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_test_form_vec_inline_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

fn run(name: &str, src: &str) -> (String, std::process::ExitStatus) {
    let bin = build(name, src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (String::from_utf8_lossy(&out.stdout).to_string(), out.status)
}

/// Push 1000 ints then get each back, summing. The inline load on
/// the in-bounds path must round-trip every element exactly.
#[test]
fn push_then_get_round_trip_exact() {
    let src = r#"
        @form(vec)
        locus IntVec {
            capacity { heap items of Int; }
        }
        fn main() {
            let v = IntVec { };
            let mut i = 0;
            while i < 1000 {
                v.push(i);
                i = i + 1;
            }
            let mut sum = 0;
            let mut j = 0;
            while j < 1000 {
                sum = sum + (v.get(j) or raise);
                j = j + 1;
            }
            println("sum=", to_string(sum));
        }
    "#;
    let (stdout, status) = run("round_trip", src);
    assert!(status.success(), "non-zero exit: {:?}", status);
    // 0 + 1 + ... + 999 = 499500
    assert!(stdout.contains("sum=499500"), "stdout: {:?}", stdout);
}

/// Out-of-bounds and negative indices both hit the cold OOB path
/// and raise an IndexError the `or { ... }` disposer recovers from
/// — no crash, the recovery block runs.
#[test]
fn oob_get_raises_and_recovers() {
    let src = r#"
        @form(vec)
        locus IntVec {
            capacity { heap items of Int; }
        }
        fn main() {
            let v = IntVec { };
            v.push(10);
            v.push(20);
            let past = v.get(99) or {
                println("past-recovered");
                -1
            };
            println("past=", to_string(past));
            let neg = v.get(-1) or {
                println("neg-recovered");
                -7
            };
            println("neg=", to_string(neg));
        }
    "#;
    let (stdout, status) = run("oob_recover", src);
    assert!(status.success(), "non-zero exit: {:?}", status);
    assert!(stdout.contains("past-recovered"), "stdout: {:?}", stdout);
    assert!(stdout.contains("past=-1"), "stdout: {:?}", stdout);
    assert!(stdout.contains("neg-recovered"), "stdout: {:?}", stdout);
    assert!(stdout.contains("neg=-7"), "stdout: {:?}", stdout);
}

/// Push past the initial capacity (4) so the doubling-grow cold
/// path fires repeatedly; every element must still read back after
/// the reallocs.
#[test]
fn push_past_capacity_grows_correctly() {
    let src = r#"
        @form(vec)
        locus IntVec {
            capacity { heap items of Int; }
        }
        fn main() {
            let v = IntVec { };
            let mut k = 0;
            while k < 100 {
                v.push(k * 3);
                k = k + 1;
            }
            println("len=", to_string(v.len()));
            let mut sum = 0;
            let mut m = 0;
            while m < 100 {
                sum = sum + (v.get(m) or raise);
                m = m + 1;
            }
            // 3 * (0 + 1 + ... + 99) = 3 * 4950 = 14850
            println("sum=", to_string(sum));
        }
    "#;
    let (stdout, status) = run("grow", src);
    assert!(status.success(), "non-zero exit: {:?}", status);
    assert!(stdout.contains("len=100"), "stdout: {:?}", stdout);
    assert!(stdout.contains("sum=14850"), "stdout: {:?}", stdout);
}

/// String element round-trip: heap-typed elem stores and reads back
/// the exact value through the inline load/store.
#[test]
fn string_elem_round_trip() {
    let src = r#"
        @form(vec)
        locus StrVec {
            capacity { heap items of String; }
        }
        fn main() {
            let s = StrVec { };
            s.push("alpha");
            s.push("beta");
            s.push("gamma");
            println("s0=", s.get(0) or raise);
            println("s2=", s.get(2) or raise);
        }
    "#;
    let (stdout, status) = run("string_elem", src);
    assert!(status.success(), "non-zero exit: {:?}", status);
    assert!(stdout.contains("s0=alpha"), "stdout: {:?}", stdout);
    assert!(stdout.contains("s2=gamma"), "stdout: {:?}", stdout);
}

/// Heap-typed struct element pushed from a method body (caller
/// scratch destroyed at method exit) is deep-copied into the
/// receiver arena, including across a grow. Reading back after the
/// scratch is gone must show the original String fields intact.
#[test]
fn method_body_push_deep_copies_heap_elem_across_grow() {
    let src = r#"
        type Entry { name: String; value: Int; }

        @form(vec)
        locus Registry {
            capacity { heap items of Entry; }

            fn add(n: String, v: Int) {
                self.push(Entry { name: n, value: v });
            }
        }
        fn main() {
            let r = Registry { };
            r.add("alpha", 1);
            r.add("beta", 2);
            r.add("gamma", 3);
            r.add("delta", 4);
            r.add("epsilon", 5);
            r.add("zeta", 6);
            println("len=", to_string(r.len()));
            let mut i = 0;
            while i < 6 {
                let e = r.get(i) or Entry { name: "FB", value: -1 };
                println(e.name, " ", to_string(e.value));
                i = i + 1;
            }
        }
    "#;
    let (stdout, status) = run("method_deep_copy", src);
    assert!(status.success(), "non-zero exit: {:?}", status);
    assert!(stdout.contains("len=6"), "stdout: {:?}", stdout);
    assert!(stdout.contains("alpha 1"), "stdout: {:?}", stdout);
    assert!(stdout.contains("zeta 6"), "stdout: {:?}", stdout);
    assert!(!stdout.contains("FB"), "fallback fired: {:?}", stdout);
}

//! FORM-vec hot-path inline regression.
//!
//! `@form(vec)`'s `.get`, `.push`, `.set` and `.pop` no longer lower
//! to opaque `lotus_vec_*` C calls on the common path — they inline
//! to a bounds-check + GEP + load/store, with the cold cases (OOB on
//! get/set, grow on push, empty on pop) keeping the original
//! behavior. These tests pin the observable semantics that must stay
//! identical after the inline: exact value round-trip, OOB raising,
//! growth across the doubling boundary, heap-elem deep-copy of a
//! method-body push, in-place `.set` mutation (incl. heap elems and
//! OOB), and LIFO `.pop` with empty-raise + len shrink.

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

/// Inline `.set`: in-bounds overwrites mutate in place; neighbours
/// are untouched. Pins the store-block path of the inlined set arm.
#[test]
fn set_inbounds_mutates_in_place() {
    let src = r#"
        @form(vec)
        locus IntVec {
            capacity { heap items of Int; }
        }
        fn main() {
            let v = IntVec { };
            let mut i = 0;
            while i < 10 {
                v.push(i);
                i = i + 1;
            }
            // Overwrite each slot with value*100.
            let mut j = 0;
            while j < 10 {
                v.set(j, j * 100) or raise;
                j = j + 1;
            }
            let mut sum = 0;
            let mut k = 0;
            while k < 10 {
                sum = sum + (v.get(k) or raise);
                k = k + 1;
            }
            // 100 * (0 + 1 + ... + 9) = 100 * 45 = 4500
            println("sum=", to_string(sum));
        }
    "#;
    let (stdout, status) = run("set_inbounds", src);
    assert!(status.success(), "non-zero exit: {:?}", status);
    assert!(stdout.contains("sum=4500"), "stdout: {:?}", stdout);
}

/// Inline `.set` OOB (and negative index) hit the cold oob block and
/// raise an IndexError the `or { ... }` disposer recovers from — no
/// store happens, the vec is unchanged.
#[test]
fn set_out_of_bounds_recovers_and_no_store() {
    let src = r#"
        @form(vec)
        locus IntVec {
            capacity { heap items of Int; }
        }
        fn main() {
            let v = IntVec { };
            v.push(1);
            v.push(2);
            v.set(99, 777) or {
                println("oob-recovered");
            };
            v.set(-1, 888) or {
                println("neg-recovered");
            };
            // Originals intact — no errant store landed.
            println("v0=", to_string(v.get(0) or raise));
            println("v1=", to_string(v.get(1) or raise));
        }
    "#;
    let (stdout, status) = run("set_oob", src);
    assert!(status.success(), "non-zero exit: {:?}", status);
    assert!(stdout.contains("oob-recovered"), "stdout: {:?}", stdout);
    assert!(stdout.contains("neg-recovered"), "stdout: {:?}", stdout);
    assert!(stdout.contains("v0=1"), "stdout: {:?}", stdout);
    assert!(stdout.contains("v1=2"), "stdout: {:?}", stdout);
}

/// Inline `.set` with a heap-typed (String) element: the deep-copied
/// value SSA is stored straight into `buf[idx]` and reads back exact.
#[test]
fn set_string_elem_in_place() {
    let src = r#"
        @form(vec)
        locus StrVec {
            capacity { heap items of String; }
        }
        fn main() {
            let s = StrVec { };
            s.push("one");
            s.push("two");
            s.push("three");
            s.set(1, "TWO") or raise;
            println("s0=", s.get(0) or raise);
            println("s1=", s.get(1) or raise);
            println("s2=", s.get(2) or raise);
        }
    "#;
    let (stdout, status) = run("set_string", src);
    assert!(status.success(), "non-zero exit: {:?}", status);
    assert!(stdout.contains("s0=one"), "stdout: {:?}", stdout);
    assert!(stdout.contains("s1=TWO"), "stdout: {:?}", stdout);
    assert!(stdout.contains("s2=three"), "stdout: {:?}", stdout);
}

/// Inline `.pop`: LIFO order, `len` shrinks per pop, and the
/// empty-vec pop hits the cold empty block raising an error the
/// fallback recovers from.
#[test]
fn pop_lifo_then_empty_raises() {
    let src = r#"
        @form(vec)
        locus IntVec {
            capacity { heap items of Int; }
        }
        fn main() {
            let v = IntVec { };
            v.push(7);
            v.push(8);
            v.push(9);
            println("len0=", to_string(v.len()));
            let a = v.pop() or raise;
            let b = v.pop() or raise;
            println("a=", to_string(a), " b=", to_string(b));
            println("len1=", to_string(v.len()));
            let c = v.pop() or raise;
            println("c=", to_string(c));
            println("len2=", to_string(v.len()));
            // Now empty — pop must take the cold empty path.
            let d = v.pop() or -1;
            println("d=", to_string(d));
            println("len3=", to_string(v.len()));
        }
    "#;
    let (stdout, status) = run("pop_lifo", src);
    assert!(status.success(), "non-zero exit: {:?}", status);
    assert!(stdout.contains("len0=3"), "stdout: {:?}", stdout);
    assert!(stdout.contains("a=9 b=8"), "stdout: {:?}", stdout);
    assert!(stdout.contains("len1=1"), "stdout: {:?}", stdout);
    assert!(stdout.contains("c=7"), "stdout: {:?}", stdout);
    assert!(stdout.contains("len2=0"), "stdout: {:?}", stdout);
    assert!(stdout.contains("d=-1"), "stdout: {:?}", stdout);
    assert!(stdout.contains("len3=0"), "stdout: {:?}", stdout);
}

/// Inline `.pop` of a heap-typed (String) element reads the exact
/// value out of `buf[new_len]` into the caller out-slot.
#[test]
fn pop_string_elem_exact() {
    let src = r#"
        @form(vec)
        locus StrVec {
            capacity { heap items of String; }
        }
        fn main() {
            let s = StrVec { };
            s.push("alpha");
            s.push("omega");
            let last = s.pop() or raise;
            let first = s.pop() or raise;
            println("last=", last);
            println("first=", first);
            println("len=", to_string(s.len()));
        }
    "#;
    let (stdout, status) = run("pop_string", src);
    assert!(status.success(), "non-zero exit: {:?}", status);
    assert!(stdout.contains("last=omega"), "stdout: {:?}", stdout);
    assert!(stdout.contains("first=alpha"), "stdout: {:?}", stdout);
    assert!(stdout.contains("len=0"), "stdout: {:?}", stdout);
}

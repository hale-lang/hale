//! Method-scratch elision (stage 1, 2026-06-28).
//!
//! Every locus method / mode / lifecycle hook used to call
//! `open_method_scratch` unconditionally — a `lotus_arena_create_subregion`
//! malloc (+ matching destroy) per call. For a method whose body provably
//! allocates nothing AND whose return is a by-value scalar (or Unit), that
//! scratch is dead substrate: `method_scratch_elidable` now skips the open,
//! leaving `current_method_scratch` None so the (absent) allocations fall
//! through to `self.__arena`, destroy/close no-op, and the return path takes
//! the no-deep-copy "store the pointer directly" branch.
//!
//! These tests pin the CORRECTNESS boundary the elision must respect — the
//! perf win itself is measured out-of-band, and ASan coverage of
//! misclassification leaks lives in `corpus_oracle`:
//!   * a numeric method computes correctly (the headline elided case);
//!   * a String-returning method is NOT elided — its returned heap value
//!     must survive across loop iterations (eliding would dangle the alias);
//!   * a heap self-store method is NOT elided — the stored String must read
//!     back intact;
//!   * a scalar self-write counter (elided) stays exact across a hot loop.

use std::process::Command;

use hale_codegen::build_executable;

fn build_and_run(name: &str, src: &str) -> String {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_method_scratch_{}_{}", name, std::process::id()));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "{name}: non-zero exit {:?}", out.status);
    String::from_utf8_lossy(&out.stdout).to_string()
}

#[test]
fn numeric_method_computes_correctly() {
    // The headline elided case: a method doing integer arithmetic over its
    // param (scratch skipped) — called in a hot loop, the sum must be exact.
    let src = r#"
        locus Counter {
            params { base: Int = 0; }
            fn step(x: Int) -> Int { return x * 2 + 1; }
        }
        fn main() {
            let c = Counter { };
            let mut acc: Int = 0;
            let mut i: Int = 0;
            while i < 1000 { acc = acc + c.step(i); i = i + 1; }
            println(acc);
        }
    "#;
    // sum_{i=0}^{999} (2i+1) = 999*1000 + 1000 = 1000000
    assert_eq!(build_and_run("numeric", src).trim(), "1000000");
}

#[test]
fn self_field_arithmetic_method_runs() {
    // A read-only self method doing arithmetic over numeric self fields —
    // `self.w * self.h` classifies as arithmetic (elided), result exact.
    let src = r#"
        locus Rect {
            params { w: Int = 6; h: Int = 7; }
            fn area() -> Int { return self.w * self.h; }
        }
        fn main() {
            let r = Rect { };
            println(r.area());
        }
    "#;
    assert_eq!(build_and_run("area", src).trim(), "42");
}

#[test]
fn string_return_method_not_elided() {
    // Correctness guard: a method building a fresh String MUST keep its
    // scratch (heap return → gate 1 fails). If it were wrongly elided, the
    // returned String would alias self.__arena and be clobbered across
    // calls — this loop, accumulating distinct results, would corrupt.
    let src = r#"
        locus Labeler {
            params { p: Int = 0; }
            fn label(n: Int) -> String { return "v=" + to_string(n); }
        }
        fn main() {
            let l = Labeler { };
            let mut i: Int = 0;
            while i < 4 { println(l.label(i * 10)); i = i + 1; }
        }
    "#;
    assert_eq!(
        build_and_run("strret", src).trim(),
        "v=0\nv=10\nv=20\nv=30"
    );
}

#[test]
fn heap_self_store_method_not_elided() {
    // A method storing a heap (String) value to a self field deep-copies
    // into self.__arena — it's classified allocating and keeps its scratch.
    // The stored value must read back intact.
    let src = r#"
        locus Box {
            params { name: String = "init"; }
            fn set(s: String) { self.name = s; }
            fn get() -> String { return self.name; }
        }
        fn main() {
            let b = Box { };
            b.set("hello-world");
            println(b.get());
        }
    "#;
    assert_eq!(build_and_run("heapstore", src).trim(), "hello-world");
}

#[test]
fn scalar_self_write_counter_exact() {
    // A scalar (Int) self-field write stores by value (no deep-copy), so the
    // counter method is elided. The final count must be exact across a hot
    // loop — proving the elided by-value self-write path is correct.
    let src = r#"
        locus Ctr {
            params { count: Int = 0; }
            fn bump() { self.count = self.count + 1; }
            fn get() -> Int { return self.count; }
        }
        fn main() {
            let c = Ctr { };
            let mut k: Int = 0;
            while k < 1000000 { c.bump(); k = k + 1; }
            println(c.get());
        }
    "#;
    assert_eq!(build_and_run("counter", src).trim(), "1000000");
}

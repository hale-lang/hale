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

/// Build with `LOTUS_DUMP_IR=1` and return the emitted LLVM IR text. Lets a
/// test assert directly whether a method body opened a scratch subregion.
fn dump_ir(name: &str, src: &str) -> String {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_ms_ir_{}_{}", name, std::process::id()));
    let ir = bin.with_extension("ll");
    std::env::set_var("LOTUS_DUMP_IR", "1");
    let result = build_executable(&program, &bin);
    std::env::remove_var("LOTUS_DUMP_IR");
    result.expect("build");
    let text = std::fs::read_to_string(&ir).expect("read IR");
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(&ir);
    text
}

/// Carve the IR body of a (possibly void / ptr / i64-returning) function.
fn carve_fn_body<'a>(ir: &'a str, name: &str) -> &'a str {
    let start = ir
        .find(&format!("define void @{}", name))
        .or_else(|| ir.find(&format!("define ptr @{}", name)))
        .or_else(|| ir.find(&format!("define i64 @{}", name)))
        .or_else(|| ir.find(&format!("define double @{}", name)))
        .unwrap_or_else(|| panic!("`{}` not defined in IR", name));
    let end = ir[start..].find("\n}").map(|i| start + i).unwrap_or(ir.len());
    &ir[start..end]
}

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

// ---- stage 2 (2026-06-28): same-locus `self.m()` calls --------------------
//
// A method that factors work into a sibling self-method used to keep its
// scratch (the `self.m()` call fell through to "allocating"). Stage 2 proves,
// per locus by a fixpoint, which `fn` methods are elidable — exactly the
// methods whose CALL allocates nothing — and lets a caller treat
// `self.m(args)` as non-allocating. These pin the correctness boundary.

#[test]
fn self_method_numeric_chain_computes_correctly() {
    // `outer` calls the elidable `inner` (the headline stage-2 win: outer now
    // also drops its scratch). The arithmetic over a hot loop must be exact.
    let src = r#"
        locus Compute {
            params { base: Int = 0; }
            fn inner(x: Int) -> Int { return x + 1; }
            fn outer(x: Int) -> Int { return self.inner(x) * 3; }
        }
        fn main() {
            let c = Compute { };
            let mut acc: Int = 0;
            let mut i: Int = 0;
            while i < 1000 { acc = acc + c.outer(i); i = i + 1; }
            println(acc);
        }
    "#;
    // sum_{i=0}^{999} (i+1)*3 = 3 * (1000*1001/2) = 3 * 500500 = 1501500
    assert_eq!(
        build_and_run("self_chain", src).trim(),
        "1501500"
    );
}

#[test]
fn self_method_three_deep_chain_correct() {
    // a -> b(self.a) -> c(self.b): a 3-deep self-method chain. Each link is
    // elidable, so the whole chain stays scratch-free; the result is exact.
    let src = r#"
        locus Chain {
            params { base: Int = 0; }
            fn a(x: Int) -> Int { return x + 1; }
            fn b(x: Int) -> Int { return self.a(x) + 2; }
            fn c(x: Int) -> Int { return self.b(x) * 2; }
        }
        fn main() {
            let k = Chain { };
            println(k.c(5));
        }
    "#;
    // c(5) = (((5+1)+2)*2) = 16
    assert_eq!(build_and_run("three_deep", src).trim(), "16");
}

#[test]
fn self_method_returning_heap_call_not_elided() {
    // SOUNDNESS: `wrap` calls `self.tag`, which returns a fresh String (heap).
    // `tag` fails gate 1 → never elidable → `self.tag(...)` stays allocating →
    // `wrap` keeps its scratch (and `wrap` itself returns String so is never
    // elided either). The distinct results accumulated across the loop must
    // not corrupt — proving the heap-returning self-call was NOT misclassified.
    let src = r#"
        locus Wrapper {
            params { p: Int = 0; }
            fn tag(n: Int) -> String { return "x" + to_string(n); }
            fn wrap(n: Int) -> String { return self.tag(n + 1); }
        }
        fn main() {
            let w = Wrapper { };
            let mut i: Int = 0;
            while i < 4 { println(w.wrap(i * 10)); i = i + 1; }
        }
    "#;
    assert_eq!(
        build_and_run("heap_self_call", src).trim(),
        "x1\nx11\nx21\nx31"
    );
}

#[test]
fn self_method_mutual_recursion_converges() {
    // Two numeric self-methods that call each other. The greatest-fixpoint
    // elidability analysis must converge (both start optimistically elidable,
    // neither demotes), and the computed values must be correct.
    let src = r#"
        locus MR {
            params { base: Int = 0; }
            fn even(n: Int) -> Int {
                if n <= 0 { return 0; }
                return self.odd(n - 1) + 1;
            }
            fn odd(n: Int) -> Int {
                if n <= 0 { return 0; }
                return self.even(n - 1) + 1;
            }
        }
        fn main() {
            let m = MR { };
            println(m.even(10));
            println(m.odd(7));
        }
    "#;
    // even(n)/odd(n) just count down to 0 → both return n.
    assert_eq!(
        build_and_run("mutual_rec", src).trim(),
        "10\n7"
    );
}

// ---- scalar-param-field reads (2026-06-30) --------------------------------
//
// A quiet handler whose only "unrecognized" expression was a scalar field
// read of a flat struct PARAM (`s.value` for `s: Sample`) used to keep its
// scratch: `self.sum + s.value` looked like a possible String concat because
// `s.value`'s scalar type wasn't modeled. The classifier now resolves the
// param's struct shape and recognizes a numeric-scalar field read as a
// non-allocating numeric scalar, so the `Add` is arithmetic and the method
// (Unit return, pure scalar self-mutation) drops its per-call scratch.

#[test]
fn scalar_param_field_handler_elides_scratch() {
    // The `Aggregator.on_sample(s: Sample)` ground-truth shape: pure scalar
    // self-mutation + scalar reads of the flat param `s`. It is genuinely
    // non-allocating; the only blocker was the `s.value` reads. The method's
    // IR must now contain NO scratch subregion open/destroy.
    let src = r#"
        type Sample { value: Float; }
        locus Aggregator {
            params { count: Int = 0; sum: Float = 0.0; min_v: Float = 1000000.0; }
            fn on_sample(s: Sample) {
                self.count = self.count + 1;
                self.sum = self.sum + s.value;
                if s.value < self.min_v { self.min_v = s.value; }
            }
            fn report() -> Float { return self.sum; }
        }
        fn main() {
            let a = Aggregator { };
            a.on_sample(Sample { value: 3.0 });
            a.on_sample(Sample { value: 4.0 });
            println(a.report());
        }
    "#;
    // Correctness: sum = 3.0 + 4.0 = 7.0.
    assert_eq!(build_and_run("agg_run", src).trim(), "7");

    let ir = dump_ir("agg_ir", src);
    let body = carve_fn_body(&ir, "Aggregator.on_sample");
    assert!(
        !body.contains("@lotus_arena_create_subregion"),
        "on_sample reads only scalar param fields + mutates scalar self \
         fields — its scratch must be ELIDED (no create_subregion); body:\n{}",
        body,
    );
    assert!(
        !body.contains("@lotus_arena_destroy"),
        "on_sample must not destroy a scratch it never opened; body:\n{}",
        body,
    );
}

#[test]
fn string_param_field_handler_keeps_scratch() {
    // NEGATIVE / soundness: a method whose body concatenates two STRING param
    // fields (`n.first + n.last`) genuinely allocates. String fields are
    // deliberately absent from the numeric-field map, so the `Add` stays
    // classified as allocating and the method KEEPS its scratch. A false
    // promotion here would strand the concat in self.__arena and leak (the
    // ASan corpus net would catch it; this pins the boundary directly).
    let src = r#"
        type Name { first: String; last: String; }
        locus Greeter {
            params { count: Int = 0; }
            fn greet(n: Name) {
                self.count = self.count + 1;
                let g = n.first + n.last;
                println(g);
            }
        }
        fn main() {
            let gr = Greeter { };
            gr.greet(Name { first: "ab", last: "cd" });
        }
    "#;
    // Correctness: prints the concatenation.
    assert_eq!(build_and_run("greet_run", src).trim(), "abcd");

    let ir = dump_ir("greet_ir", src);
    let body = carve_fn_body(&ir, "Greeter.greet");
    assert!(
        body.contains("@lotus_arena_create_subregion"),
        "greet concatenates two String param fields (allocating) — it must \
         KEEP its scratch (create_subregion present); body:\n{}",
        body,
    );
}

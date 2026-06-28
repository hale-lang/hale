//! `@form(vec)` bounds-check elimination (BCE) for `.get` inside
//! counted loops.
//!
//! A `@form(vec)` locus IS the vec; the common shape is a standalone
//! local instance:
//! ```text
//!   let v = IntVec { };
//!   ... v.push(x) ...
//!   for j in 0..v.len() { acc = acc ^ (v.get(j) or raise) }
//! ```
//! For `for VAR in 0..RECV.len()` (exclusive, lower bound literal 0)
//! over a vec instance `RECV` the body does not mutate, every
//! `RECV.get(VAR)` is provably in-bounds, so the per-element bounds
//! check + lazy-IndexError + `or` handler are dead and elided —
//! letting the loop body vectorize.
//!
//! These tests pin (a) correct results for the local-instance, self
//! and self-field receiver shapes; (b) that the bounds branch is
//! actually gone from the BCE loop-body IR; and (c) that BCE does NOT
//! fire when the body mutates the vec — the checked path is retained
//! and an intentional out-of-bounds access is still caught.

use std::process::Command;

use hale_codegen::build_executable;

fn build(name: &str, src: &str) -> std::path::PathBuf {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_test_form_vec_bce_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

fn run(name: &str, src: &str) -> (String, std::process::ExitStatus) {
    let bin = build(name, src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (String::from_utf8_lossy(&out.stdout).to_string(), out.status)
}

/// Build with `LOTUS_DUMP_IR` set so codegen writes the
/// pre-optimization LLVM IR alongside the binary, and return that IR
/// text. Pre-opt is sufficient for the BCE assertions: BCE elides the
/// per-iteration bounds *compare* and the *oob* block at emit time
/// (they never appear) and emits a constant `c_ret = 1`. The
/// downstream lazy-IndexError block is still textually present
/// pre-opt but statically dead (`is_err = 1 == 0`), and O2 removes
/// it (and vectorizes the loop) — so we assert on the emit-time facts.
fn build_dump_ir(name: &str, src: &str) -> String {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_test_form_vec_bce_{}", name));
    std::env::set_var("LOTUS_DUMP_IR", "1");
    build_executable(&program, &bin).expect("build");
    std::env::remove_var("LOTUS_DUMP_IR");
    let ll = bin.with_extension("ll");
    let ir = std::fs::read_to_string(&ll).expect("IR dump written");
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(&ll);
    ir
}

/// Slice out a single LLVM function definition body by a substring of
/// its (mangled) name: from the `define ... @<...name...>(` line to
/// the closing `}` at column 0. Pass a needle including the leading
/// `@` and trailing `(` (e.g. `"@main("`) when the bare name would
/// also match unrelated functions.
fn slice_fn<'a>(ir: &'a str, name_substr: &str) -> &'a str {
    let mut search_from = 0;
    while let Some(rel) = ir[search_from..].find("\ndefine ") {
        let def_start = search_from + rel + 1; // skip the leading '\n'
        let line_end = ir[def_start..]
            .find('\n')
            .map(|n| def_start + n)
            .unwrap_or(ir.len());
        let header = &ir[def_start..line_end];
        if header.contains(name_substr) {
            let after = line_end;
            let close = ir[after..]
                .find("\n}")
                .map(|n| after + n + 2)
                .unwrap_or(ir.len());
            return &ir[def_start..close];
        }
        search_from = line_end;
    }
    panic!("function containing {:?} not found in IR", name_substr);
}

/// Reference XOR fold of `f(i)` for `i in 0..n`, computed in Rust.
fn xor_fold(n: i64, f: impl Fn(i64) -> i64) -> i64 {
    (0..n).fold(0i64, |acc, i| acc ^ f(i))
}

/// PRIMARY BCE case (the bench shape): a local vec instance summed
/// (XOR) through `for j in 0..v.len() { v.get(j) }` in `main`. Result
/// must match the checked semantics exactly.
#[test]
fn bce_local_instance_sum_loop_correct() {
    let src = r#"
        @form(vec)
        locus IntVec {
            capacity { heap items of Int; }
        }
        fn main() {
            let v = IntVec { };
            let mut i = 0;
            while i < 1000 {
                v.push(i * 7 + 3);
                i = i + 1;
            }
            let mut acc = 0;
            for j in 0..v.len() {
                acc = acc ^ (v.get(j) or raise);
            }
            println("acc=", to_string(acc));
        }
    "#;
    let expected = xor_fold(1000, |i| i * 7 + 3);
    let (stdout, status) = run("local_sum", src);
    assert!(status.success(), "non-zero exit: {:?}", status);
    assert!(
        stdout.contains(&format!("acc={}", expected)),
        "stdout: {:?} (expected acc={})",
        stdout,
        expected
    );
}

/// IR proof on the local-instance loop in `main`: the BCE loop body
/// emits NO bounds compare / OOB block and carries the branch-free
/// `vec.get.bce` markers. A control function that reads the same vec
/// with a non-loop-var index keeps its bounds check.
#[test]
fn bce_loop_body_has_no_bounds_branch_in_ir() {
    let src = r#"
        @form(vec)
        locus IntVec {
            capacity { heap items of Int; }
        }
        locus Probe {
            params { data: IntVec = IntVec { }; }
            // Non-loop-var index: must keep the checked path.
            fn checked_one(k: Int) -> Int {
                return self.data.get(k) or -1;
            }
        }
        fn main() {
            let v = IntVec { };
            v.push(11);
            v.push(22);
            v.push(33);
            let mut acc = 0;
            for j in 0..v.len() {
                acc = acc ^ (v.get(j) or raise);
            }
            println("acc=", to_string(acc));
            let p = Probe { };
            println("k=", to_string(p.checked_one(0)));
        }
    "#;
    let ir = build_dump_ir("local_ir", src);

    // The BCE loop lives in `main` (distinct from is_main_fn).
    let body = slice_fn(&ir, "@main(");
    assert!(
        !body.contains("vec.get.inbounds"),
        "BCE loop body still has an inbounds compare:\n{}",
        body
    );
    assert!(
        !body.contains("vec.get.oob"),
        "BCE loop body still has an OOB block:\n{}",
        body
    );
    assert!(
        body.contains("vec.get.bce"),
        "BCE loop body missing the branch-free get markers:\n{}",
        body
    );

    // Control: checked_one (index is a param, not a loop var) keeps
    // the bounds branch and never takes the BCE path.
    let checked = slice_fn(&ir, "checked_one");
    assert!(
        checked.contains("vec.get.inbounds") || checked.contains("vec.get.oob"),
        "checked_one unexpectedly lost its bounds check:\n{}",
        checked
    );
    assert!(
        !checked.contains("vec.get.bce"),
        "checked_one unexpectedly took the BCE path:\n{}",
        checked
    );
}

/// BCE on the pattern-1 locus (the locus IS the vec, `self` receiver):
/// `for i in 0..self.len() { self.get(i) }` inside a method.
#[test]
fn bce_self_locus_sum_loop_correct() {
    let src = r#"
        @form(vec)
        locus IntVec {
            capacity { heap items of Int; }

            fn fill() {
                let mut i = 0;
                while i < 500 {
                    self.push(i);
                    i = i + 1;
                }
            }
            fn total() -> Int {
                let mut s = 0;
                for i in 0..self.len() {
                    s = s + (self.get(i) or raise);
                }
                return s;
            }
        }
        fn main() {
            let v = IntVec { };
            v.fill();
            println("sum=", to_string(v.total()));
        }
    "#;
    // 0 + 1 + ... + 499 = 124750
    let (stdout, status) = run("self_locus_sum", src);
    assert!(status.success(), "non-zero exit: {:?}", status);
    assert!(stdout.contains("sum=124750"), "stdout: {:?}", stdout);
}

/// BCE on a self-FIELD vec (`self.data` receiver):
/// `for i in 0..self.data.len() { self.data.get(i) }`.
#[test]
fn bce_self_field_sum_loop_correct() {
    let src = r#"
        @form(vec)
        locus IntVec {
            capacity { heap items of Int; }
        }
        locus Agg {
            params { data: IntVec = IntVec { }; }

            fn fill() {
                let mut i = 0;
                while i < 1000 {
                    self.data.push(i * 2 + 1);
                    i = i + 1;
                }
            }
            fn xor_fold() -> Int {
                let mut acc = 0;
                for i in 0..self.data.len() {
                    acc = acc ^ (self.data.get(i) or raise);
                }
                return acc;
            }
        }
        fn main() {
            let a = Agg { };
            a.fill();
            println("acc=", to_string(a.xor_fold()));
        }
    "#;
    let expected = xor_fold(1000, |i| i * 2 + 1);
    let (stdout, status) = run("self_field_sum", src);
    assert!(status.success(), "non-zero exit: {:?}", status);
    assert!(
        stdout.contains(&format!("acc={}", expected)),
        "stdout: {:?} (expected acc={})",
        stdout,
        expected
    );
}

/// Negative case: the loop body MUTATES the vec (`v.push`), so BCE
/// must NOT fire. Proven two ways: (1) IR — `main`'s loop keeps the
/// bounds compare / OOB block and takes NO branch-free BCE path;
/// (2) runtime — the result matches the checked semantics and an
/// intentionally out-of-bounds get takes its `or` fallback instead of
/// dereferencing past the buffer.
#[test]
fn bce_not_applied_when_body_mutates_vec() {
    let src = r#"
        @form(vec)
        locus IntVec {
            capacity { heap items of Int; }
        }
        fn main() {
            let v = IntVec { };
            v.push(100);
            let mut sum = 0;
            for j in 0..v.len() {
                sum = sum + (v.get(j) or raise);
                if j == 0 {
                    v.push(200);
                }
            }
            println("sum=", to_string(sum));
            // Intentionally out-of-bounds: must take the fallback,
            // proving the checked path is intact.
            let oob = v.get(9999) or -7;
            println("oob=", to_string(oob));
        }
    "#;
    // IR: the mutating loop kept its bounds check and took no BCE path.
    let ir = build_dump_ir("mutating_ir", src);
    let body = slice_fn(&ir, "@main(");
    assert!(
        !body.contains("vec.get.bce"),
        "mutating-body loop wrongly took the branch-free BCE path:\n{}",
        body
    );
    assert!(
        body.contains("vec.get.inbounds") || body.contains("vec.get.oob"),
        "checked path missing — BCE may have wrongly fired:\n{}",
        body
    );

    // Runtime: the for-range captures `len` once at entry (= 1 after
    // the seed push), so only j=0 runs — reads 100, pushes 200 (unseen
    // by this loop). sum = 100. The OOB get takes the fallback.
    let (stdout, status) = run("mutating_body", src);
    assert!(status.success(), "non-zero exit: {:?}", status);
    assert!(stdout.contains("sum=100"), "stdout: {:?}", stdout);
    assert!(stdout.contains("oob=-7"), "stdout: {:?}", stdout);
}

/// Negative case: index is `j + 1` (arithmetic, not the bare loop
/// var), so BCE must NOT fire — the index identity check requires
/// exactly `Expr::Ident(VAR)`. The last iteration reads out of bounds
/// and must take the fallback, not UB.
#[test]
fn bce_not_applied_for_offset_index() {
    let src = r#"
        @form(vec)
        locus IntVec {
            capacity { heap items of Int; }
        }
        fn main() {
            let v = IntVec { };
            v.push(10);
            v.push(20);
            v.push(30);
            let mut hits = 0;
            let mut misses = 0;
            for j in 0..v.len() {
                let x = v.get(j + 1) or -1;
                if x == -1 {
                    misses = misses + 1;
                } else {
                    hits = hits + 1;
                }
            }
            println("hits=", to_string(hits), " misses=", to_string(misses));
        }
    "#;
    // j=0->get(1)=20 hit; j=1->get(2)=30 hit; j=2->get(3) OOB miss.
    let (stdout, status) = run("offset_index", src);
    assert!(status.success(), "non-zero exit: {:?}", status);
    assert!(
        stdout.contains("hits=2 misses=1"),
        "stdout: {:?}",
        stdout
    );
}

/// Negative case: the body aliases the vec into another binding
/// (`let w = v;`). Even though this particular alias is harmless, the
/// walker conservatively bails (an alias could be mutated), so BCE
/// must NOT fire — the checked path is retained. Proven via IR: no
/// branch-free `vec.get.bce` path appears, and the bounds check
/// survives. Result must still be correct.
#[test]
fn bce_not_applied_when_vec_aliased_in_body() {
    let src = r#"
        @form(vec)
        locus IntVec {
            capacity { heap items of Int; }
        }
        fn main() {
            let v = IntVec { };
            v.push(5);
            v.push(6);
            v.push(7);
            let mut acc = 0;
            for j in 0..v.len() {
                // Alias v into w: BCE must bail for the whole loop,
                // even though the get below is on v. Without the
                // aliasing guard, v.get(j) would wrongly BCE.
                let w = v;
                acc = acc + (v.get(j) or raise) + (w.len() - w.len());
            }
            println("acc=", to_string(acc));
        }
    "#;
    let ir = build_dump_ir("aliased_ir", src);
    let body = slice_fn(&ir, "@main(");
    assert!(
        !body.contains("vec.get.bce"),
        "aliased-vec loop wrongly took the branch-free BCE path:\n{}",
        body
    );
    assert!(
        body.contains("vec.get.inbounds") || body.contains("vec.get.oob"),
        "aliased-vec loop lost its bounds check:\n{}",
        body
    );
    let (stdout, status) = run("aliased_body", src);
    assert!(status.success(), "non-zero exit: {:?}", status);
    assert!(stdout.contains("acc=18"), "stdout: {:?}", stdout);
}

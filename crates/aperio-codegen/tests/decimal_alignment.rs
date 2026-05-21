//! Regression test for the Decimal-field-in-struct segfault
//! (i128 alignment segfault, 2026-05-20). Root cause:
//! `lotus_arena_alloc` was aligning the offset within the chunk
//! rather than the actual returned pointer address. The chunk's
//! data region starts after a 24-byte header — 8-byte aligned
//! but NOT 16-byte aligned — so allocating a struct with i128
//! fields (Decimal) landed at an 8-byte address. LLVM's `movaps`
//! store of i128 into struct fields then trapped with SIGSEGV.
//!
//! Fix in two layers:
//!   - codegen `arena_alloc` now passes align=16 (covers the
//!     widest scalar — i128) instead of the previous 8.
//!   - C `lotus_arena_alloc` computes the offset as a function
//!     of the actual returned pointer address, not just the
//!     within-chunk offset. The chunk's `(chunk+1) + used` cursor
//!     gets aligned to `align`, then converted back to an offset.

use std::process::Command;

use aperio_codegen::build_executable;

fn build_and_run(name: &str, source: &str) -> std::process::Output {
    let program = aperio_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("aperio_test_dec_align_{}", name));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    out
}

#[test]
fn struct_with_decimal_default_does_not_segfault() {
    // The minimal repro. Pre-fix: SIGSEGV on the i128 store at
    // construction. Post-fix: exits cleanly.
    let src = r#"
        type X { p: Decimal = 0.0d; }
        fn main() {
            let x = X { };
            println("p=", x.p);
        }
    "#;
    let out = build_and_run("single_field", src);
    assert!(
        out.status.success(),
        "expected clean exit; status={:?} stderr={:?}",
        out.status,
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("p=0"),
        "got: {:?}",
        String::from_utf8_lossy(&out.stdout),
    );
}

#[test]
fn struct_with_many_decimal_fields_does_not_segfault() {
    // The original repro shape — a flat record with two Decimal
    // fields, used as the default-init type on a high-field-
    // count locus (20 such records sitting side by side).
    let src = r#"
        type Cell {
            a: Decimal = 0.0d;
            b: Decimal = 0.0d;
        }
        locus Grid {
            params {
                c01: Cell = Cell { };
                c02: Cell = Cell { };
                c03: Cell = Cell { };
                c04: Cell = Cell { };
                c05: Cell = Cell { };
                c06: Cell = Cell { };
                c07: Cell = Cell { };
                c08: Cell = Cell { };
                c09: Cell = Cell { };
                c10: Cell = Cell { };
                c11: Cell = Cell { };
                c12: Cell = Cell { };
                c13: Cell = Cell { };
                c14: Cell = Cell { };
                c15: Cell = Cell { };
                c16: Cell = Cell { };
                c17: Cell = Cell { };
                c18: Cell = Cell { };
                c19: Cell = Cell { };
                c20: Cell = Cell { };
            }
        }
        fn main() {
            let g = Grid { };
            println("ok");
        }
    "#;
    let out = build_and_run("grid_20", src);
    assert!(
        out.status.success(),
        "expected clean exit; status={:?} stderr={:?}",
        out.status,
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("ok"),
        "got: {:?}",
        String::from_utf8_lossy(&out.stdout),
    );
}

#[test]
fn multi_decimal_fallible_fn_returning_struct_does_not_segfault() {
    // F4: multi-Decimal flat-struct return from a fallible free
    // fn into a local binding. The friction noted F4 didn't
    // repro in a smoke test, only in a real-workload runtime
    // path. After the alignment fix this shape works end-to-end
    // — F4 and the in-struct movdqa segfault shared the same
    // root cause.
    let src = r#"
        type Record {
            label: String = "";
            x1: Decimal = 0.0d;
            x2: Decimal = 0.0d;
            x3: Decimal = 0.0d;
            x4: Decimal = 0.0d;
        }
        fn parse(s: String) -> Record fallible(ParseError) {
            return Record {
                label: s,
                x1: 100.5d,
                x2: 1.0d,
                x3: 101.5d,
                x4: 2.0d,
            };
        }
        fn main() {
            let r = parse("abc") or Record { };
            println("label=", r.label);
            println("x1=", r.x1);
            println("x3=", r.x3);
        }
    "#;
    let out = build_and_run("parse_record", src);
    assert!(
        out.status.success(),
        "expected clean exit; status={:?} stderr={:?}",
        out.status,
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("label=abc"), "got: {:?}", stdout);
    assert!(stdout.contains("x1=100.5"), "got: {:?}", stdout);
    assert!(stdout.contains("x3=101.5"), "got: {:?}", stdout);
}

#[test]
fn locus_with_decimal_param_default_does_not_segfault() {
    // Same shape via the locus-param-default path.
    let src = r#"
        locus Tracker {
            params {
                threshold: Decimal = 0.001d;
                total: Decimal = 0.0d;
            }
            fn show() {
                println("t=", self.threshold);
                println("s=", self.total);
            }
        }
        fn main() {
            let t = Tracker { };
            t.show();
        }
    "#;
    let out = build_and_run("locus_decimal_params", src);
    assert!(
        out.status.success(),
        "expected clean exit; status={:?} stderr={:?}",
        out.status,
        String::from_utf8_lossy(&out.stderr),
    );
}

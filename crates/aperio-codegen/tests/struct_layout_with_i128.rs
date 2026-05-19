//! Regression: struct sizeof() must respect target ABI alignment for
//! i128-typed fields (`Decimal`). Without an explicit datalayout on
//! the module, LLVM's fallback layout put i128 at align 8, sizing
//! `{ ptr, i128, ptr }` at 32 bytes; the GEP-derived offsets used
//! the same layout, but the post-O2 ConstantExpr `sizeof` was
//! re-evaluated against the natural x86_64 layout (i128 @ align 16,
//! struct size 48). The mismatch made the trailing ptr field land
//! past the allocation, so reads of a nested struct's String field
//! through that slot returned heap garbage.

use std::process::Command;

use aperio_codegen::build_executable;

fn build_and_run(name: &str, source: &str) -> (String, std::process::ExitStatus) {
    let program = aperio_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("lotus_test_{}", name));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        output.status,
    )
}

#[test]
fn nested_struct_string_after_decimal_reads_correctly() {
    // The pond/trade/backtest repro: `Tick` has a Decimal mid-struct
    // and a `Side` tail field whose String must read back as "bid".
    let src = r#"
        type Side { kind: String = "bid"; }
        type Tick {
            symbol: String  = "";
            price:  Decimal = 0.0d;
            qty:    Decimal = 0.0d;
            side:   Side    = Side { kind: "bid" };
        }
        fn main() {
            let t = Tick {
                symbol: "GOOG",
                price:  170.0d,
                qty:    100.0d,
                side:   Side { kind: "bid" },
            };
            println("t.symbol=", t.symbol,
                    " t.side.kind=[", t.side.kind, "]");
        }
    "#;
    let (stdout, status) = build_and_run("nested_string_after_decimal", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(
        stdout.contains("t.symbol=GOOG t.side.kind=[bid]"),
        "expected nested side.kind to read 'bid'; got: {:?}",
        stdout
    );
}

#[test]
fn struct_with_decimal_then_ptr_field_writes_in_bounds() {
    // Minimal: a struct with Decimal followed by a pointer-shaped
    // field (String here). Before the datalayout fix, the alloc was
    // sized for i128-align-8 (32 bytes), but the GEP for the tail
    // ptr put it at offset 32 — out of bounds. We confirm the tail
    // String reads as the literal we wrote.
    let src = r#"
        type Rec {
            n:    Decimal = 0.0d;
            tail: String  = "";
        }
        fn main() {
            let r = Rec { n: 1.5d, tail: "ok" };
            println("tail=[", r.tail, "]");
        }
    "#;
    let (stdout, status) = build_and_run("decimal_then_ptr", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(
        stdout.contains("tail=[ok]"),
        "expected tail string to round-trip; got: {:?}",
        stdout
    );
}

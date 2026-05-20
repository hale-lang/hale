//! Form H (2026-05-20): fixed-size arrays as bus payload fields.
//! Pre-fix, m70's wire codec rejected any array-typed payload
//! field with "arrays / tuples / enums cross-process are post-v1
//! polish". Fathom's SymbolBook (the canonical fixed-N-levels
//! order-book shape) needed this — workaround was hand-spelling
//! 20 numbered fields + N-way dispatch ladders in setter/getter
//! methods.
//!
//! Form H carves out the fixed-size case: `[T; N]` where T is a
//! primitive (Int / Float / Decimal / Bool / Duration / Time)
//! or a TypeRef (nested user-struct whose leaves are also
//! supported). Arrays of String / Bytes / nested-Arrays stay
//! deferred — the friction's "fixed-size case" framing.

use std::process::Command;

use aperio_codegen::build_executable;

fn build_and_run(name: &str, source: &str) -> (String, std::process::ExitStatus) {
    let program = aperio_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("aperio_test_bus_array_{}", name));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        output.status,
    )
}

#[test]
fn array_of_typeref_payload_roundtrips() {
    // fathom's SymbolBook-shape: [BookLevel; N] where BookLevel
    // has Decimal fields. The pre-Form-H rejection was at the
    // m70 wire-format walker on the publisher's serialize.
    let src = r#"
        type BookLevel {
            price: Decimal = 0.0d;
            qty: Decimal = 0.0d;
        }
        type SnapshotMsg {
            symbol: String = "";
            bids: [BookLevel; 5] = [BookLevel { }; 5];
        }
        topic Snapshots { payload: SnapshotMsg; }

        locus Subscriber {
            bus { subscribe Snapshots as h; }
            fn h(m: SnapshotMsg) {
                println("symbol=", m.symbol);
                println("bid0.price=", m.bids[0].price);
                println("bid0.qty=", m.bids[0].qty);
                println("bid1.price=", m.bids[1].price);
            }
        }
        locus Publisher {
            bus { publish Snapshots; }
            birth() {
                let m = SnapshotMsg {
                    symbol: "XBT/USD",
                    bids: [BookLevel { price: 100.5d, qty: 1.0d }; 5],
                };
                Snapshots <- m;
            }
        }
        fn main() { Subscriber { }; Publisher { }; }
    "#;
    let (stdout, status) = build_and_run("typeref_array", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("symbol=XBT/USD"), "got: {:?}", stdout);
    assert!(stdout.contains("bid0.price=100.5"), "got: {:?}", stdout);
    assert!(stdout.contains("bid0.qty=1"), "got: {:?}", stdout);
    assert!(stdout.contains("bid1.price=100.5"), "got: {:?}", stdout);
}

#[test]
fn array_of_int_payload_roundtrips() {
    // Primitive-element arrays take the single-memcpy fast path.
    let src = r#"
        type Frame { counts: [Int; 4] = [0; 4]; }
        topic Frames { payload: Frame; }

        locus Subscriber {
            bus { subscribe Frames as h; }
            fn h(f: Frame) {
                println("c0=", f.counts[0]);
                println("c3=", f.counts[3]);
            }
        }
        locus Publisher {
            bus { publish Frames; }
            birth() {
                let f = Frame { counts: [42; 4] };
                Frames <- f;
            }
        }
        fn main() { Subscriber { }; Publisher { }; }
    "#;
    let (stdout, status) = build_and_run("int_array", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("c0=42"), "got: {:?}", stdout);
    assert!(stdout.contains("c3=42"), "got: {:?}", stdout);
}

#[test]
fn array_of_decimal_payload_roundtrips() {
    // Decimal is i128 (16-byte aligned per the F.30b/G alignment
    // fix). The single-memcpy path on the wire side reads/writes
    // contiguous 16-byte elements.
    let src = r#"
        type Bar { prices: [Decimal; 3] = [0.0d; 3]; }
        topic Bars { payload: Bar; }

        locus Subscriber {
            bus { subscribe Bars as h; }
            fn h(b: Bar) {
                println("p0=", b.prices[0]);
                println("p1=", b.prices[1]);
                println("p2=", b.prices[2]);
            }
        }
        locus Publisher {
            bus { publish Bars; }
            birth() {
                let b = Bar { prices: [99.5d; 3] };
                Bars <- b;
            }
        }
        fn main() { Subscriber { }; Publisher { }; }
    "#;
    let (stdout, status) = build_and_run("decimal_array", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(stdout.contains("p0=99.5"), "got: {:?}", stdout);
    assert!(stdout.contains("p2=99.5"), "got: {:?}", stdout);
}

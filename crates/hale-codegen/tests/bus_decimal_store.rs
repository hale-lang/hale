//! Regression: a bus-received `Decimal` (i128, align-16) delivered to
//! a PINNED subscriber must be storable / usable in arithmetic without
//! a memory-safety fault.
//!
//! Root cause (fixed): the substrate bus cell's inline payload buffer
//! (`lotus_bus_cell_t.payload_inline`) had the struct's natural
//! alignment of only 8 (its widest member is a pointer / size_t). On
//! the PINNED mailbox path the cell is snapshotted onto the drain
//! thread's stack (`lotus_mailbox_drain_one`) and the handler reads the
//! payload straight out of that 8-aligned buffer. A payload struct
//! carrying a Decimal puts the i128 at a 16-aligned *offset*, but from
//! an 8-aligned base it lands at an 8-mod-16 address — and the codegen
//! reads it with an aligned SSE move (`vmovaps`), which #GP-traps.
//! Cooperative subscribers were unaffected (their drain copies into an
//! `aligned(16)` scratch buffer), which is why the pre-existing
//! cooperative `ws1_cross_seed_bus_decimal` test never caught it.
//!
//! The fix forces `payload_inline` (hence the whole cell) to 16-byte
//! alignment, so EVERY cell copy — stack snapshots, the mailbox ring
//! slot's embedded cell, the malloc'd queue array — keeps the inline
//! payload 16-aligned.
//!
//! Each test asserts the EXACT round-tripped / accumulated Decimal
//! values (not merely "did not crash"), so a partial i128 corruption
//! that survived the segfault would still fail the assertion. The
//! whole file must also run clean under `LOTUS_ASAN=1`.

use std::process::Command;

use hale_codegen::build_executable;

fn build_and_run(name: &str, source: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_test_bus_decimal_store_{}_{}", name, std::process::id()));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        output.status,
    )
}

/// The proven crasher: a 16-byte pure-Decimal payload copied WHOLE into
/// a `@form(vec)` slot on a pinned subscriber. The struct copy lowers to
/// an aligned `vmovaps` load off the 8-mod-16 mailbox buffer — SIGSEGV
/// pre-fix. Also accumulates the Decimals and reads them back to assert
/// the exact values survived the store.
#[test]
fn pinned_decimal_vec_push_and_accumulate() {
    let src = r#"
        type Money { v: Decimal; }

        @form(vec)
        locus VecSink {
            params { acc: Decimal = 0.0d; seen: Int = 0; }
            capacity { heap log of Money; }
            bus { subscribe "m" as on_m of type Money; }
            fn on_m(m: Money) {
                // whole-struct copy into the vec slot (aligned SSE move
                // off the bus payload buffer — the pre-fix trap site)
                self.push(m);
                self.acc = self.acc + m.v;
                self.seen = self.seen + 1;
                if self.seen == 3 {
                    let a = self.get(0) or raise;
                    let b = self.get(1) or raise;
                    let c = self.get(2) or raise;
                    println("v0=", to_string(a.v));
                    println("v1=", to_string(b.v));
                    println("v2=", to_string(c.v));
                    println("acc=", to_string(self.acc));
                }
            }
        }

        locus Pub {
            bus { publish "m" of type Money; }
            birth() {
                "m" <- Money { v: 12345.67d };
                "m" <- Money { v: 99999.99d };
                "m" <- Money { v: 0.000001d };
            }
        }

        main locus App {
            params {
                sink: VecSink = VecSink { };
                pub:  Pub     = Pub     { };
            }
            placement { sink: pinned; }
            run() {
                std::time::sleep(40ms);
                println("done");
            }
        }

        fn main() { App { }; }
    "#;
    let (stdout, status) = build_and_run("vec_push", src);
    assert!(
        status.success(),
        "pinned Decimal vec-push regressed (segfault / non-zero exit): \
         {:?} stdout={:?}",
        status,
        stdout
    );
    assert!(stdout.contains("v0=12345.67"), "v0 corrupted: {:?}", stdout);
    assert!(stdout.contains("v1=99999.99"), "v1 corrupted: {:?}", stdout);
    assert!(stdout.contains("v2=0.000001"), "v2 corrupted: {:?}", stdout);
    // 12345.67 + 99999.99 + 0.000001 = 112345.660001 — a partial i128
    // corruption that eluded the per-record eyeball still skews this.
    assert!(
        stdout.contains("acc=112345.660001"),
        "accumulated Decimal wrong: {:?}",
        stdout
    );
}

/// Store a bus-received Decimal into a `@form(hashmap)` cell on a pinned
/// subscriber, read it back, and accumulate. Exercises the cell-store
/// path the bug report flagged directly.
#[test]
fn pinned_decimal_hashmap_cell_store() {
    let src = r#"
        type Msg  { id: Int; amt: Decimal; }
        type Cell { key: Int; amt: Decimal; }

        @form(hashmap)
        locus MapSink {
            params { acc: Decimal = 0.0d; seen: Int = 0; }
            capacity { pool cells of Cell indexed_by key; }
            bus { subscribe "money" as on_money of type Msg; }
            fn on_money(m: Msg) {
                self.set(Cell { key: m.id, amt: m.amt });
                let c = self.get(m.id) or raise;
                self.acc = self.acc + c.amt;
                self.seen = self.seen + 1;
                println("cell id=", c.key, " amt=", to_string(c.amt));
                if self.seen == 3 { println("acc=", to_string(self.acc)); }
            }
        }

        locus Pub {
            bus { publish "money" of type Msg; }
            birth() {
                "money" <- Msg { id: 1, amt: 12345.67d };
                "money" <- Msg { id: 2, amt: 99999.99d };
                "money" <- Msg { id: 3, amt: 0.000001d };
            }
        }

        main locus App {
            params {
                sink: MapSink = MapSink { };
                pub:  Pub     = Pub     { };
            }
            placement { sink: pinned; }
            run() {
                std::time::sleep(40ms);
                println("done");
            }
        }

        fn main() { App { }; }
    "#;
    let (stdout, status) = build_and_run("hashmap_cell", src);
    assert!(
        status.success(),
        "pinned Decimal hashmap-cell store regressed: {:?} stdout={:?}",
        status,
        stdout
    );
    assert!(stdout.contains("cell id=1 amt=12345.67"), "got: {:?}", stdout);
    assert!(stdout.contains("cell id=2 amt=99999.99"), "got: {:?}", stdout);
    assert!(stdout.contains("cell id=3 amt=0.000001"), "got: {:?}", stdout);
    assert!(
        stdout.contains("acc=112345.660001"),
        "accumulated Decimal wrong: {:?}",
        stdout
    );
}

/// Plain `self` field store + arithmetic on a bus-received Decimal at a
/// pinned subscriber (no form). The minimal shape of the report.
#[test]
fn pinned_decimal_self_field_and_arithmetic() {
    let src = r#"
        type Msg { id: Int; amt: Decimal; }

        locus Sink {
            params { acc: Decimal = 0.0d; seen: Int = 0; }
            bus { subscribe "money" as on_money of type Msg; }
            fn on_money(m: Msg) {
                // to_string is expected safe even pre-fix (byte-wise read)
                println("got id=", m.id, " amt=", to_string(m.amt));
                self.acc = self.acc + m.amt;   // arithmetic + self store
                self.seen = self.seen + 1;
                if self.seen == 3 { println("acc=", to_string(self.acc)); }
            }
        }

        locus Pub {
            bus { publish "money" of type Msg; }
            birth() {
                "money" <- Msg { id: 1, amt: 12345.67d };
                "money" <- Msg { id: 2, amt: 99999.99d };
                "money" <- Msg { id: 3, amt: 0.000001d };
            }
        }

        main locus App {
            params {
                sink: Sink = Sink { };
                pub:  Pub  = Pub  { };
            }
            placement { sink: pinned; }
            run() {
                std::time::sleep(40ms);
                println("done");
            }
        }

        fn main() { App { }; }
    "#;
    let (stdout, status) = build_and_run("self_field", src);
    assert!(
        status.success(),
        "pinned Decimal self-field store regressed: {:?} stdout={:?}",
        status,
        stdout
    );
    assert!(stdout.contains("got id=1 amt=12345.67"), "got: {:?}", stdout);
    assert!(
        stdout.contains("acc=112345.660001"),
        "accumulated Decimal wrong: {:?}",
        stdout
    );
}

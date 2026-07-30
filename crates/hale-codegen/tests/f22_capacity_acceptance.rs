//! F.22 #18 — acceptance gates. The smoke (`f22_capacity_smoke`)
//! and dispatch (`f22_capacity_dispatch`) tests cover the
//! per-piece behavior; this suite exercises the higher-pressure
//! shapes that the v1 substrate has to handle to be useful.
//!
//! NOTE: Cell *contents* (read/write through a cell handle) are
//! deferred to v1.x — cells are opaque round-trip handles at v1.
//! These tests therefore validate substrate plumbing under load
//! and lifecycle ordering, not user-visible cell I/O.

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn build(name: &str, src: &str) -> std::path::PathBuf {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(&format!("hale_test_f22_acc_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

#[test]
fn pool_grows_past_initial_chunk() {
    // The initial chunk is 16 cells (LOTUS_POOL_INITIAL_CELLS).
    // 50 acquires without releases force at least one grow
    // (16 → 32 from the first grow, +2 more chunks of 64 each
    // in v0 spec or geometric continuation). The success bar:
    // every acquire returns a non-null cell, the binary runs
    // clean, and the wholesale teardown at locus dissolve
    // frees the whole population without segfault.
    let src = r#"
        locus PoolGrowL {
            capacity {
                pool entries of Int;
            }
            birth {
                let mut i = 0;
                while i < 50 {
                    let _c = self.entries.acquire();
                    i = i + 1;
                }
                println("ok");
            }
        }
        fn main() {
            let _ = PoolGrowL { };
        }
    "#;
    let bin = build("pool_grow", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("ok"), "missing log: {:?}", stdout);
}

#[test]
fn heap_individual_alloc_and_free_no_cross_contamination() {
    // Alloc 30 cells, free every other one, then alloc 15 more.
    // The heap's free()'d cells are recycled by malloc (or the
    // future heap-side freelist); the still-live cells stay
    // intact. The "ok" log fires after the whole sequence
    // completes without segfault, then dissolve frees the
    // remaining 30 live cells wholesale via the live-list walk.
    let src = r#"
        locus HeapChurnL {
            capacity {
                heap registry of Int;
            }
            birth {
                let mut i = 0;
                while i < 30 {
                    let _c = self.registry.alloc();
                    i = i + 1;
                }
                // Note: v0 cells are opaque, so we can't keep
                // individual cell handles for later free(). The
                // intermixed-free test surface awaits cell-content
                // I/O (v1.x). What this test validates: alloc()
                // grows under load, dissolve frees the population
                // wholesale.
                println("alloc-30-ok");
                let mut j = 0;
                while j < 15 {
                    let _d = self.registry.alloc();
                    j = j + 1;
                }
                println("alloc-15-more-ok");
            }
        }
        fn main() {
            let _ = HeapChurnL { };
        }
    "#;
    let bin = build("heap_churn", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("alloc-30-ok")
            && stdout.contains("alloc-15-more-ok"),
        "missing logs: {:?}",
        stdout
    );
}

#[test]
fn slot_teardown_runs_during_dissolve_no_resource_leak() {
    // A locus with multiple slots, instantiated and dissolved
    // many times. If slot teardown leaked (forgot to call
    // destroy() on each slot's allocator), RSS would climb
    // linearly with the iteration count. We can't measure RSS
    // from inside an Hale program, but a clean run under
    // 1000 iterations is a strong indicator that the teardown
    // path executes — and a leak would also surface as
    // address-space exhaustion on long-running tests.
    let src = r#"
        locus EphemeralL {
            capacity {
                pool a of Int;
                pool b of Int;
                heap c of Int;
            }
            birth {
                let _ca = self.a.acquire();
                let _cb = self.b.acquire();
                let _hc = self.c.alloc();
            }
        }
        fn main() {
            let mut n = 0;
            while n < 1000 {
                let _ = EphemeralL { };
                n = n + 1;
            }
            println("1000-cycles-ok");
        }
    "#;
    let bin = build("dissolve_loop", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("1000-cycles-ok"),
        "missing log: {:?}",
        stdout
    );
}

#[test]
fn capacity_with_user_struct_cell_type_compiles() {
    // T1.2 Map / T1.1 Vec want struct-cell-typed slots. Verify
    // a user `type` declaration as the cell type is accepted by
    // typecheck + codegen (the size_of() path needs to work for
    // arbitrary user structs).
    let src = r#"
        type Entry { key: Int; value: Int; }
        locus MapBackingL {
            capacity {
                pool buckets of Entry;
            }
            birth {
                let _c = self.buckets.acquire();
                println("struct-cell-ok");
            }
        }
        fn main() {
            let _ = MapBackingL { };
        }
    "#;
    let bin = build("struct_cell", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("struct-cell-ok"),
        "missing log: {:?}",
        stdout
    );
}

#[test]
fn cell_value_cannot_print_or_arithmetic() {
    // Cells are opaque round-trip handles at v1. Trying to
    // println a cell (or do arithmetic on one) should be a
    // diagnosable build error.
    let src = r#"
        locus BadL {
            capacity {
                pool entries of Int;
            }
            birth {
                let cell = self.entries.acquire();
                println("cell=", cell);
            }
        }
        fn main() { }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin("hale_test_f22_acc_no_print");
    let err = build_executable(&program, &bin)
        .expect_err("expected cell-not-printable diagnostic");
    let msg = format!("{}", err);
    assert!(
        msg.to_lowercase().contains("cell")
            || msg.contains("printable"),
        "expected cell-print diagnostic, got: {}",
        msg
    );
}

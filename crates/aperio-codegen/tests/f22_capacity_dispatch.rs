//! F.22 #17 — `self.X.acquire()` / `self.X.alloc()` method-shaped
//! dispatch. Exercises the cell round-trip surface from inside
//! locus method bodies (per spec §F.22 slot ABI example).

use std::process::Command;

use aperio_codegen::build_executable;

fn build(name: &str, src: &str) -> std::path::PathBuf {
    let program = aperio_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("aperio_test_f22_dispatch_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

#[test]
fn pool_acquire_release_round_trip_does_not_crash() {
    // Acquire a cell, release it. The simplest round-trip from the
    // spec example. Validates that the cell value typechecks
    // through the let binding and back into release().
    let src = r#"
        locus PoolHostL {
            capacity {
                pool entries of Int;
            }
            birth {
                let cell = self.entries.acquire();
                self.entries.release(cell);
                println("round-trip-ok");
            }
        }
        fn main() {
            let _ = PoolHostL { };
        }
    "#;
    let bin = build("pool_round_trip", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("round-trip-ok"),
        "missing round-trip log: {:?}",
        stdout
    );
}

#[test]
fn heap_alloc_free_round_trip_does_not_crash() {
    let src = r#"
        locus HeapHostL {
            capacity {
                heap registry of Int;
            }
            birth {
                let p = self.registry.alloc();
                self.registry.free(p);
                println("heap-round-trip-ok");
            }
        }
        fn main() {
            let _ = HeapHostL { };
        }
    "#;
    let bin = build("heap_round_trip", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("heap-round-trip-ok"),
        "missing log: {:?}",
        stdout
    );
}

#[test]
fn many_acquire_release_pairs_exercise_freelist_reuse() {
    // 100 cycles of acquire/release on a Pool. The C-side free-list
    // should recycle the same cell addresses without growing the
    // chunk count. We can't directly observe the address from
    // Aperio in v1 (cells are opaque), but a leak in the pool
    // primitive would surface as RSS growth that swamps the test
    // process. This is mostly a smoke for "doesn't crash under
    // load" — the more rigorous reuse test would be a C-side
    // unit test (deferred to a future runtime/test crate).
    let src = r#"
        locus ChurnL {
            capacity {
                pool entries of Int;
            }
            birth {
                let mut i = 0;
                while i < 100 {
                    let cell = self.entries.acquire();
                    self.entries.release(cell);
                    i = i + 1;
                }
                println("churn-ok");
            }
        }
        fn main() {
            let _ = ChurnL { };
        }
    "#;
    let bin = build("pool_churn", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("churn-ok"), "missing log: {:?}", stdout);
}

#[test]
fn pool_rejects_heap_methods() {
    // `alloc` / `free` are heap-only; calling them on a pool slot
    // should be a diagnosable error at build time.
    let src = r#"
        locus BadL {
            capacity {
                pool entries of Int;
            }
            birth {
                let p = self.entries.alloc();
            }
        }
        fn main() { }
    "#;
    let program = aperio_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push("aperio_test_f22_dispatch_pool_rejects_heap_methods");
    let err = build_executable(&program, &bin)
        .expect_err("expected pool-rejects-alloc diagnostic");
    let msg = format!("{}", err);
    assert!(
        msg.contains("pool")
            && (msg.contains("acquire") || msg.contains("alloc")),
        "expected pool-method diagnostic, got: {}",
        msg
    );
}

#[test]
fn heap_rejects_pool_methods() {
    let src = r#"
        locus BadL {
            capacity {
                heap registry of Int;
            }
            birth {
                let c = self.registry.acquire();
            }
        }
        fn main() { }
    "#;
    let program = aperio_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push("aperio_test_f22_dispatch_heap_rejects_pool_methods");
    let err = build_executable(&program, &bin)
        .expect_err("expected heap-rejects-acquire diagnostic");
    let msg = format!("{}", err);
    assert!(
        msg.contains("heap")
            && (msg.contains("acquire") || msg.contains("alloc")),
        "expected heap-method diagnostic, got: {}",
        msg
    );
}

#[test]
fn cross_slot_cell_release_rejected() {
    // v1.x-5: cells now carry slot origin. Releasing into a
    // different slot of the same element type is a hard
    // build-time error.
    let src = r#"
        locus CrossL {
            capacity {
                pool a of Int;
                pool b of Int;
            }
            birth {
                let cell = self.a.acquire();
                self.b.release(cell);
            }
        }
        fn main() { }
    "#;
    let program = aperio_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push("aperio_test_f22_dispatch_cross_slot");
    let err = build_executable(&program, &bin)
        .expect_err("v1.x-5 should reject cross-slot release");
    let msg = format!("{}", err);
    assert!(
        msg.contains("originated from") || msg.contains("CrossL.a"),
        "expected slot-of-origin diagnostic, got: {}",
        msg
    );
}

//! v1.x-1: F.22 capacity slots under `aperio run` (interpreter).
//! Codegen-side coverage lives in
//! `crates/aperio-codegen/tests/f22_capacity_*`. This file
//! validates parity — the same .ap source that works under
//! `aperio build` also works under `aperio run`.

use aperio_runtime::run_program;

fn run(src: &str) -> i32 {
    let program = aperio_syntax::parse_source(src)
        .map_err(|d| {
            d.iter()
                .map(|x| x.render(src))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .expect("parse");
    run_program(&program).expect("run")
}

#[test]
fn pool_slot_acquire_release_round_trip() {
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
    assert_eq!(run(src), 0);
}

#[test]
fn heap_slot_alloc_free_round_trip() {
    let src = r#"
        locus HeapHostL {
            capacity {
                heap registry of Int;
            }
            birth {
                let p = self.registry.alloc();
                self.registry.free(p);
                println("heap-ok");
            }
        }
        fn main() {
            let _ = HeapHostL { };
        }
    "#;
    assert_eq!(run(src), 0);
}

#[test]
fn pool_freelist_recycles_across_acquire_cycles() {
    // 100 acquire/release cycles. The interpreter's Pool free
    // list should keep peak size at 1 (each release returns
    // the cell before the next acquire). Functional test:
    // doesn't crash; no resource growth.
    let src = r#"
        locus ChurnL {
            capacity {
                pool entries of Int;
            }
            birth {
                let mut i = 0;
                while i < 100 {
                    let c = self.entries.acquire();
                    self.entries.release(c);
                    i = i + 1;
                }
                println("churn-ok");
            }
        }
        fn main() {
            let _ = ChurnL { };
        }
    "#;
    assert_eq!(run(src), 0);
}

#[test]
fn pool_method_on_heap_slot_rejected() {
    // Cross-kind method (acquire on heap) — interpreter raises
    // a runtime error rather than the codegen-build error.
    let src = r#"
        locus BadL {
            capacity {
                heap registry of Int;
            }
            birth {
                let c = self.registry.acquire();
            }
        }
        fn main() {
            let _ = BadL { };
        }
    "#;
    let program = aperio_syntax::parse_source(src).expect("parse");
    let res = run_program(&program);
    assert!(res.is_err(), "expected runtime error, got {:?}", res);
    let msg = format!("{:?}", res);
    assert!(
        msg.contains("heap") && msg.contains("acquire"),
        "expected diagnostic naming heap+acquire, got: {}",
        msg
    );
}

#[test]
fn struct_cell_field_io_round_trip() {
    // v1.x-2 parity: cell.field write + read works in the
    // interpreter the same way it works in codegen.
    let src = r#"
        type Entry { key: Int; value: Int; }
        locus MapL {
            capacity {
                pool entries of Entry;
            }
            birth {
                let cell = self.entries.acquire();
                cell.key = 42;
                cell.value = 99;
                if cell.key != 42 { return 1; }
                if cell.value != 99 { return 2; }
                println("ok");
            }
        }
        fn main() {
            let _ = MapL { };
        }
    "#;
    assert_eq!(run(src), 0);
}

#[test]
fn heap_cell_field_io_round_trip() {
    let src = r#"
        type Record { tag: Int; }
        locus HeapL {
            capacity {
                heap records of Record;
            }
            birth {
                let r = self.records.alloc();
                r.tag = 7;
                if r.tag != 7 { return 1; }
                println("ok");
            }
        }
        fn main() {
            let _ = HeapL { };
        }
    "#;
    assert_eq!(run(src), 0);
}

#[test]
fn cross_slot_release_rejected_at_runtime() {
    // v1.x-5 parity: interpreter enforces slot-of-origin at
    // release time, same as the codegen typecheck.
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
        fn main() {
            let _ = CrossL { };
        }
    "#;
    let program = aperio_syntax::parse_source(src).expect("parse");
    let res = run_program(&program);
    assert!(res.is_err(), "expected cross-slot release error");
    let msg = format!("{:?}", res);
    assert!(
        msg.contains("originated") || msg.contains("CrossL.a"),
        "expected slot-of-origin diagnostic, got: {}",
        msg
    );
}

#[test]
fn multiple_slots_coexist() {
    // Two pools + one heap on the same locus. All four method
    // dispatches work, instantiation and dissolve clean.
    let src = r#"
        locus MultiL {
            capacity {
                pool a of Int;
                pool b of Int;
                heap c of Int;
            }
            birth {
                let ca = self.a.acquire();
                let cb = self.b.acquire();
                let hc = self.c.alloc();
                self.a.release(ca);
                self.b.release(cb);
                self.c.free(hc);
                println("multi-ok");
            }
        }
        fn main() {
            let _ = MultiL { };
        }
    "#;
    assert_eq!(run(src), 0);
}

//! F.22 capacity-tuple — task #16 smoke (AST + typecheck + codegen
//! surface). Confirms a `capacity { pool X of T; heap Y of T; }`
//! block parses, typechecks, and round-trips through codegen
//! without crashing on slot init / dissolve. User-facing
//! `self.X.acquire()` / `.alloc()` dispatch lands in task #17;
//! these tests only validate the substrate scaffold.

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn build(name: &str, src: &str) -> std::path::PathBuf {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(&format!("hale_test_f22_smoke_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

#[test]
fn pool_slot_locus_instantiates_and_dissolves_cleanly() {
    // Slot declared, never used. Should:
    //   - parse the capacity block
    //   - extend the locus struct with __slot_entries: ptr
    //   - call lotus_pool_create at instantiation
    //   - call lotus_pool_destroy at dissolve
    // Crash-free run is the bar; nothing observable from the
    // user side yet — #17 wires acquire/release.
    let src = r#"
        locus PoolHostL {
            capacity {
                pool entries of Int;
            }
            birth { println("birth"); }
            dissolve { println("dissolve"); }
        }
        fn main() {
            let _ = PoolHostL { };
        }
    "#;
    let bin = build("pool_lifecycle", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("birth"), "missing birth: {:?}", stdout);
    assert!(stdout.contains("dissolve"), "missing dissolve: {:?}", stdout);
}

#[test]
fn heap_slot_locus_instantiates_and_dissolves_cleanly() {
    let src = r#"
        locus HeapHostL {
            capacity {
                heap registry of Int;
            }
            birth { println("birth"); }
            dissolve { println("dissolve"); }
        }
        fn main() {
            let _ = HeapHostL { };
        }
    "#;
    let bin = build("heap_lifecycle", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("birth"), "missing birth: {:?}", stdout);
    assert!(stdout.contains("dissolve"), "missing dissolve: {:?}", stdout);
}

#[test]
fn multiple_slots_init_in_declaration_order_destroy_in_reverse() {
    // Two pools + one heap; just confirm coexistence in one
    // locus. Order is implicit (the C-side allocators don't log)
    // but the spec mandates declaration-order init and reverse
    // destroy; a corrupted order would surface as a crash because
    // slot 0 (arena) is the last torn down and any slot teardown
    // touching it post-destroy would segfault.
    let src = r#"
        locus MultiSlotL {
            capacity {
                pool a of Int;
                heap b of Int;
                pool c of Int;
            }
            birth { println("ok"); }
        }
        fn main() {
            let _ = MultiSlotL { };
        }
    "#;
    let bin = build("multi_slots", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("ok"), "missing ok: {:?}", stdout);
}

#[test]
fn locus_typed_slot_rejected_by_typecheck() {
    // Restriction 1: cell type cannot be a locus reference.
    let src = r#"
        locus InnerL { birth { println("x"); } }
        locus OuterL {
            capacity {
                pool kids of InnerL;
            }
        }
        fn main() { }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin("hale_test_f22_smoke_locus_cell");
    let err = build_executable(&program, &bin).expect_err(
        "build should fail with F.22 restriction-1 diagnostic",
    );
    let msg = format!("{}", err);
    assert!(
        msg.contains("InnerL") || msg.contains("locus")
            || msg.to_lowercase().contains("capacity"),
        "expected restriction-1 diagnostic, got: {}",
        msg
    );
}

#[test]
fn duplicate_slot_name_rejected() {
    // Two slots sharing a name — the codegen-side check fires.
    let src = r#"
        locus DupL {
            capacity {
                pool entries of Int;
                heap entries of Int;
            }
        }
        fn main() { }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin("hale_test_f22_smoke_dup");
    let err = build_executable(&program, &bin).expect_err(
        "build should fail with duplicate-slot-name diagnostic",
    );
    let msg = format!("{}", err);
    assert!(
        msg.to_lowercase().contains("duplicate")
            || msg.contains("entries"),
        "expected duplicate-slot diagnostic, got: {}",
        msg
    );
}

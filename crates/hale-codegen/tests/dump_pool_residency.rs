//! F.35 Slice 4: `std::process::dump_pool_residency()` diagnostic.
//!
//! Writes one stderr line per cooperative pool with its I/O mode
//! (async_io / blocking), parked-coro count, and pending cell-
//! queue depth. Mirrors `dump_arena_residency` — callable from a
//! heartbeat tick on long-running daemons.

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

#[test]
fn dump_pool_residency_lists_each_pool_with_mode() {
    let src = r#"
        locus Worker { params { tag: String = "w"; } }

        main locus App {
            params {
                w1: Worker = Worker { };
                w2: Worker = Worker { };
            }
            placement {
                w1: cooperative(pool = io)  where async_io;
                w2: cooperative(pool = aux);
            }
            run() {
                std::process::dump_pool_residency();
            }
        }
        fn main() { App { }; }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin("hale_test_dump_pool_residency");
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "non-zero exit: {:?}", output.status);
    // Both pools should appear; one in async_io mode, one in blocking.
    assert!(
        stderr.contains("[io]") && stderr.contains("mode=async_io"),
        "missing io / async_io line: {:?}",
        stderr
    );
    assert!(
        stderr.contains("[aux]") && stderr.contains("mode=blocking"),
        "missing aux / blocking line: {:?}",
        stderr
    );
    assert!(
        stderr.contains("count=2"),
        "expected count=2 header: {:?}",
        stderr
    );
}

#[test]
fn dump_pool_residency_with_no_pools_emits_count_zero() {
    let src = r#"
        main locus App {
            params { tag: String = "x"; }
            run() {
                std::process::dump_pool_residency();
            }
        }
        fn main() { App { }; }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin("hale_test_dump_pool_residency_empty");
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "non-zero exit: {:?}", output.status);
    assert!(
        stderr.contains("count=0"),
        "expected count=0 with no placement entries: {:?}",
        stderr
    );
}

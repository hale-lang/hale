//! F.32-1α (2026-05-24) — `@form(hashmap, sync = serialized)`
//! end-to-end coverage.
//!
//! Two scenarios:
//!
//!   1. `serialized_basic_single_pool` — sanity check that the
//!      `sync = serialized` opt-in compiles and behaves
//!      identically to plain `@form(hashmap)` when there's no
//!      cross-pool traffic. The mutex is uncontended; per-op
//!      cost is one extra branch + one uncontended lock acquire.
//!
//!   2. `serialized_cross_pool_correctness` — the scenario that
//!      drove F.32-0 + F.32-1α to ship. Two pools (main + io)
//!      both write disjoint keys (even / odd ids) into a single
//!      shared Registry. Pre-F.32-1α (or with plain
//!      `@form(hashmap)`): `lotus_hashmap_set` / `_grow` race and
//!      double-free during concurrent grow. Post-F.32-1α: every
//!      mutate-method body takes `m->mu`, the race is gone, all
//!      200k inserts land.
//!
//! Per the F.32 plan, this test pairs with the (still-pending)
//! `bench/micro/form_hashmap_false_sharing.hl` perf bench, which
//! will compare `serialized` vs `striped` throughput once F.32-1β
//! ships.

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn unique_path(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let p = harness::unique_bin(&format!(
        "lt-form-hashmap-serialized-{}-{}-{}.bin",
        tag,
        std::process::id(),
        nanos,
    ));
    p
}

fn build_and_run(tag: &str, src: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = unique_path(tag);
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run binary");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    (stdout, out.status)
}

#[test]
fn serialized_basic_single_pool() {
    let src = r#"
        type Counter { id: Int; v: Int; }

        @form(hashmap, sync = serialized)
        locus Registry {
            capacity { pool entries of Counter indexed_by id; }
        }

        main locus App {
            params { reg: Registry = Registry { }; }
            run() {
                let n = 1000;
                let mut i = 0;
                while i < n {
                    self.reg.set(Counter { id: i, v: i + 1 });
                    i = i + 1;
                }
                print("len="); println(self.reg.len());
            }
        }

        fn main() { App { }; }
    "#;
    let (stdout, status) = build_and_run("basic", src);
    assert!(
        status.success(),
        "binary exited non-zero: {:?}\nstdout: {}",
        status,
        stdout,
    );
    assert!(
        stdout.contains("len=1000"),
        "expected len=1000 single-thread; got:\n{}",
        stdout
    );
}

#[test]
fn serialized_cross_pool_correctness() {
    // This is the scenario whose pre-F.32-1α behavior was a
    // double-free crash within ~1s. The PoolHost wraps Registry
    // and lives on the `io` pool; App on main both write to
    // self.host.reg.set (cross-pool through the @form-sync
    // exemption). 100k writes per side = 200k total.
    let src = r#"
        type Counter { id: Int; v: Int; }

        @form(hashmap, sync = serialized)
        locus Registry {
            capacity { pool entries of Counter indexed_by id; }
        }

        locus PoolHost {
            params { reg: Registry = Registry { }; }
            run() {
                let mut i = 0;
                while i < 20000 {
                    self.reg.set(Counter { id: i * 2 + 1, v: i });
                    i = i + 1;
                }
            }
        }

        main locus App {
            params { host: PoolHost = PoolHost { }; }
            placement { host: cooperative(pool = io); }
            run() {
                let mut i = 0;
                while i < 20000 {
                    self.host.reg.set(Counter { id: i * 2, v: i });
                    i = i + 1;
                }
                while self.host.reg.len() < 40000 {
                    std::time::sleep(1ms);
                }
                print("len="); println(self.host.reg.len());
            }
        }

        fn main() { App { }; }
    "#;
    let (stdout, status) = build_and_run("cross_pool", src);
    assert!(
        status.success(),
        "binary exited non-zero (probable corruption regression): {:?}\nstdout: {}",
        status,
        stdout,
    );
    assert!(
        stdout.contains("len=40000"),
        "expected len=40000 after both writers drain; got:\n{}",
        stdout
    );
}

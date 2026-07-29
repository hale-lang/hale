//! F.31 Phase 4b — cooperative-pool run() on the pool worker.
//!
//! Pre-Phase-4b: a main-locus params field placed on
//! `cooperative(pool = X)` had its `run()` called synchronously
//! on the parent's main thread. A long-running run() body
//! blocked the parent from progressing past the params-init
//! flow — the "long-running cooperative child blocks parent"
//! shape documented in spec/runtime.md.
//!
//! Post-Phase-4b: `run()` for non-main coop-pool loci is
//! posted to the pool's worker queue. The parent's params-init
//! continues immediately; the child's run() executes on the
//! pool's thread, in FIFO order with bus handler cells (so
//! birth runs synchronously on main, then run() lands on the
//! pool thread, then handler cells from cross-thread publishers
//! land behind it).

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
        "lt-coop-run-{}-{}-{}.bin",
        tag,
        std::process::id(),
        nanos,
    ));
    p
}

#[test]
fn coop_pool_run_does_not_block_main() {
    // Worker's run() sleeps 100ms before printing. Main's run()
    // prints immediately after the App { } instantiation
    // returns and then sleeps for the full window. Without
    // Phase 4b the synchronous run() call blocks main until
    // worker's sleep completes; with it, main prints first.
    let src = r#"
        locus Worker {
            run() {
                std::time::sleep(80ms);
                println("worker run done");
            }
        }

        main locus App {
            params {
                w: Worker = Worker { };
            }
            placement {
                w: cooperative(pool = io);
            }
            run() {
                println("main run start");
                std::time::sleep(200ms);
                println("main run end");
            }
        }

        fn main() {
            App { };
        }
    "#;

    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = unique_path("nonblock");
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);

    assert!(
        out.status.success(),
        "binary exited non-zero: {:?}\nstdout: {}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );

    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let lines: Vec<&str> = stdout.lines().collect();

    let start_idx = lines
        .iter()
        .position(|l| *l == "main run start")
        .unwrap_or_else(|| panic!("expected 'main run start'; got: {}", stdout));
    let worker_idx = lines
        .iter()
        .position(|l| *l == "worker run done")
        .unwrap_or_else(|| panic!("expected 'worker run done'; got: {}", stdout));
    let end_idx = lines
        .iter()
        .position(|l| *l == "main run end")
        .unwrap_or_else(|| panic!("expected 'main run end'; got: {}", stdout));

    // Phase 4b ordering: main run start → (worker fires async
    // on io thread) → main run end. Worker's print must land
    // BETWEEN start and end (proves both that worker ran AND
    // that main wasn't blocked waiting for it).
    assert!(
        start_idx < worker_idx,
        "'worker run done' appeared before 'main run start' \
         (idx {} vs {}); main blocked on worker before printing. \
         Full stdout:\n{}",
        worker_idx,
        start_idx,
        stdout
    );
    assert!(
        worker_idx < end_idx,
        "'worker run done' didn't fire before 'main run end' \
         (idx {} vs {}); pool worker didn't drain. \
         Full stdout:\n{}",
        worker_idx,
        end_idx,
        stdout
    );
}

#[test]
fn coop_pool_run_then_handler_in_fifo_order() {
    // Verify that posting run() to the pool queue followed by
    // bus handler cells preserves FIFO: run() lands first, then
    // any handler cells published during run() arrive after.
    let src = r#"
        type Tick { n: Int; }

        locus Sub {
            bus { subscribe "tick" as on_tick of type Tick; }
            fn on_tick(t: Tick) {
                println("tick ", t.n);
            }
            run() {
                println("sub run");
            }
        }

        main locus App {
            params {
                s: Sub = Sub { };
            }
            placement {
                s: cooperative(pool = io);
            }
            bus { publish "tick" of type Tick; }
            run() {
                std::time::sleep(20ms);
                "tick" <- Tick { n: 1 };
                "tick" <- Tick { n: 2 };
                std::time::sleep(50ms);
                println("main done");
            }
        }

        fn main() {
            App { };
        }
    "#;

    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = unique_path("fifo");
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);

    assert!(
        out.status.success(),
        "binary exited non-zero: {:?}\nstdout: {}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );

    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(stdout.contains("sub run"), "got: {:?}", stdout);
    assert!(stdout.contains("tick 1"), "got: {:?}", stdout);
    assert!(stdout.contains("tick 2"), "got: {:?}", stdout);
    assert!(stdout.contains("main done"), "got: {:?}", stdout);

    // FIFO check: sub run before tick 1 before tick 2.
    let lines: Vec<&str> = stdout.lines().collect();
    let sub_run = lines.iter().position(|l| *l == "sub run").unwrap();
    let tick1 = lines.iter().position(|l| *l == "tick 1").unwrap();
    let tick2 = lines.iter().position(|l| *l == "tick 2").unwrap();
    assert!(
        sub_run < tick1 && tick1 < tick2,
        "FIFO order broken: sub_run={} tick1={} tick2={}\nstdout:\n{}",
        sub_run,
        tick1,
        tick2,
        stdout
    );
}

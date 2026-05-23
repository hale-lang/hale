//! Cooperative `while { time::sleep(...); ... }` loops drain the
//! bus queue after each sleep returns, without the body needing
//! an explicit `yield;`.
//!
//! The pinned publisher MUST outlast the cooperative subscriber's
//! run() — otherwise the pinned thread's own
//! `flush_dissolve_frame` at run-exit drains `g_bus_queue` (and
//! dispatches cooperative subscribers' handlers on the pinned
//! thread, masking whether the cooperative-side fix is doing
//! anything). Pub does a long terminal sleep so its drain doesn't
//! fire until after Sub has fully exited.
//!
//! Sub's run() prints a sentinel as its last body statement.
//! With the sleep-folded drain, every "got tick" handler fires
//! mid-loop and prints BEFORE the sentinel. Without the fix the
//! cells sit in `g_bus_queue` until Sub's own flush_dissolve_frame
//! fires at run-exit — which runs AFTER the body's last println —
//! so "got tick" prints AFTER the sentinel.
//!
//! The assertion compares line positions in stdout.

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use aperio_codegen::build_executable;

fn unique_path(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "lt-sleep-drain-{}-{}-{}.bin",
        tag,
        std::process::id(),
        nanos,
    ));
    p
}

#[test]
#[ignore = "F.31 Phase 3b — requires placement-override codegen \
            to restore pinned dispatch so Pub runs on its own \
            thread; without it Pub runs synchronously and the \
            cross-thread-during-sleep scenario can't be exercised. \
            Re-enable once Phase 3b lands."]
fn sleep_drains_bus_queue_mid_loop() {
    let src = r#"
        type Tick { n: Int; }

        // Pinned publisher: own pthread. Brief warmup so Sub is
        // subscribed, drips 4 ticks, then a long terminal sleep
        // that outlasts Sub's run() — keeps the pinned thread's
        // own flush_dissolve_frame from draining the queue before
        // Sub gets the chance.
        //
        // F.31: pinned placement lives in main locus's
        // `placement { }` block rather than as a per-locus
        // `: schedule pinned` annotation.
        locus Pub {
            bus { publish "tick" of type Tick; }
            run() {
                std::time::sleep(20ms);
                let mut i = 1;
                while i <= 4 {
                    "tick" <- Tick { n: i };
                    std::time::sleep(5ms);
                    i = i + 1;
                }
                // Outlast Sub's run() (~150ms). When main exits
                // it pthread_joins Pub; the test wait is bounded
                // by this duration plus startup overhead.
                std::time::sleep(300ms);
            }
        }

        // Cooperative subscriber: run() loops with sleep, no
        // explicit yield. If sleep folds in the cooperative
        // bus drain, on_tick prints fire mid-loop, before the
        // body's last println.
        locus Sub {
            bus { subscribe "tick" as on_tick of type Tick; }
            fn on_tick(t: Tick) {
                println("got tick ", t.n);
            }
            run() {
                let mut i = 0;
                while i < 30 {
                    std::time::sleep(5ms);
                    i = i + 1;
                }
                println("subscriber.run done");
            }
        }

        main locus App {
            params {
                pub_: Pub = Pub { };
                sub: Sub = Sub { };
            }
            placement {
                pub_: pinned;
            }
        }

        fn main() {
            App { };
        }
    "#;

    let program = aperio_syntax::parse_source(src).expect("parse");
    let bin = unique_path("loop");
    build_executable(&program, &bin).expect("build");

    let out = Command::new(&bin)
        .output()
        .expect("run binary");

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

    let sentinel_idx = lines
        .iter()
        .position(|l| *l == "subscriber.run done")
        .unwrap_or_else(|| {
            panic!(
                "stdout should contain 'subscriber.run done'; got: {:?}",
                stdout
            );
        });

    let tick_lines: Vec<(usize, &&str)> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.starts_with("got tick "))
        .collect();

    assert!(
        tick_lines.len() >= 4,
        "expected 4 'got tick N' lines in stdout; got {}: {:?}",
        tick_lines.len(),
        stdout
    );

    // Every "got tick N" must precede the sentinel — proves sleep
    // is folding in the drain mid-loop. Without the fix, the
    // cells would sit in g_bus_queue until Sub's run() exits, and
    // flush_dissolve_frame's drain fires AFTER the body's last
    // println, putting tick lines AFTER the sentinel.
    for (idx, line) in &tick_lines {
        assert!(
            *idx < sentinel_idx,
            "'{}' at line {} appeared AFTER 'subscriber.run done' \
             at line {} — sleep is not draining the bus queue \
             mid-loop. Full stdout:\n{}",
            line,
            idx,
            sentinel_idx,
            stdout
        );
    }
}

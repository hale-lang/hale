//! F.31 Phase 4 — M:N cooperative pool worker thread.
//!
//! Verifies that a main-locus `params` field placed on a named
//! cooperative pool (other than "main") has its bus-subscribed
//! handler dispatched on the pool's worker thread. The test
//! publishes two ticks from main, then sleeps to let the io
//! pool's worker drain its queue, and asserts both handler
//! prints landed in stdout — proving the cross-thread routing
//! (main posts to pool queue, worker drains + invokes handler)
//! actually fires.

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
        "lt-coop-pool-{}-{}-{}.bin",
        tag,
        std::process::id(),
        nanos,
    ));
    p
}

#[test]
fn coop_pool_subscriber_delivery() {
    let src = r#"
        type Tick { n: Int; }

        locus Sub {
            bus { subscribe "tick" as on_tick of type Tick; }
            fn on_tick(t: Tick) {
                println("got tick ", t.n);
            }
        }

        main locus App {
            params {
                sub: Sub = Sub { };
            }
            placement {
                sub: cooperative(pool = io);
            }
            bus { publish "tick" of type Tick; }
            run() {
                "tick" <- Tick { n: 1 };
                "tick" <- Tick { n: 2 };
                // Sleep on main long enough for the io pool's
                // worker to drain its queue. At shutdown the
                // worker's join blocks until pending cells run,
                // so the sleep is belt-and-suspenders; the
                // assertion is whether the prints make it out
                // before main exits cleanly.
                std::time::sleep(100ms);
                println("main done");
            }
        }

        fn main() {
            App { };
        }
    "#;

    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = unique_path("basic");
    build_executable(&program, &bin).expect("build");

    let out = Command::new(&bin).output().expect("run binary");
    let _ = std::fs::remove_file(&bin);

    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();

    assert!(
        out.status.success(),
        "binary exited non-zero: {:?}\nstdout: {}\nstderr: {}",
        out.status,
        stdout,
        stderr,
    );

    assert!(
        stdout.contains("got tick 1"),
        "expected 'got tick 1' in stdout; full output:\n{}",
        stdout
    );
    assert!(
        stdout.contains("got tick 2"),
        "expected 'got tick 2' in stdout; full output:\n{}",
        stdout
    );
    assert!(
        stdout.contains("main done"),
        "expected 'main done' in stdout; full output:\n{}",
        stdout
    );
}

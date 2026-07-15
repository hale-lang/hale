//! Downstream handoff 2026-07-15: SIGSEGV under ingest load —
//! cross-pool bus-handler execution on the publisher's thread.
//!
//! The scope-exit flush emitted at the end of every fn/method body
//! calls `lotus_bus_queue_drain` on whatever thread ran the body. A
//! PINNED publisher that published to a main-pool cooperative
//! subscriber would then execute that subscriber's handler on its
//! own thread — and because the locked drain releases the queue
//! mutex before invoking each handler, main's sleep-slice drain
//! could be inside the SAME subscriber concurrently. Two threads in
//! one locus → concurrent arena clone/retire on the unlocked
//! anchor-retire freelist → freelist corruption → SIGSEGV/SIGABRT
//! in `lotus_retire_free_pop` (reproduced 2/3 pre-fix with the
//! string-keyed `indexed_by` churn below).
//!
//! Fix: the global cooperative queue records its owner thread at
//! creation (main's prelude); `lotus_bus_queue_drain` no-ops on any
//! other thread. Handler EXECUTION is owner-bound — matching the
//! pinned-mailbox and pool-ring channels, which were always
//! owner-executed — while cross-thread ENQUEUE stays as-is.

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use hale_codegen::build_executable;

fn unique_path(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "hale-xthread-drain-{}-{}-{}.bin",
        tag,
        std::process::id(),
        nanos,
    ));
    p
}

#[test]
fn pinned_flood_of_main_pool_subscriber_survives_and_delivers() {
    // The crash shape: a pinned flooder batch-publishes to a
    // string-keyed @form(hashmap) subscriber on pool main, calling a
    // method every 512 publishes (whose scope-exit flush used to
    // drain the queue on the pinned thread), while main's sleep
    // slices drain concurrently. Pre-fix this corrupts the
    // subscriber arena's retire freelist within seconds; post-fix
    // all cells drain on main only. Asserts BOTH no-crash and full
    // delivery.
    let src = r#"
        type Msg  { key: String; ts: String; n: Int; }
        type Cell { key: String; ts: String; }

        @form(hashmap)
        locus State {
            params { seen: Int = 0; }
            capacity { pool cells of Cell indexed_by key; }
            bus { subscribe "acc" as on_msg of type Msg; }
            fn on_msg(m: Msg) {
                // Same-key replace churn: retires the old cell's
                // key/ts clones and reuses them via the retire
                // freelist — the allocator the pre-fix race corrupted.
                self.set(Cell { key: m.key, ts: m.ts });
                self.seen = self.seen + 1;
            }
        }

        locus Flooder {
            params { n: Int = 0; }
            bus { publish "acc" of type Msg; }
            fn tick(i: Int) {
                // Allocating body: its scope-exit flush is exactly
                // the pre-fix cross-thread drain site.
                let s = "batch:" + "x";
                self.n = i;
            }
            run() {
                let mut i = 0;
                while i < 1000000 {
                    let k = "key-" + to_string(i - (i / 64) * 64);
                    "acc" <- Msg { key: k, ts: "2026-07-15T12:00:00Z", n: i };
                    i = i + 1;
                    if i - (i / 512) * 512 == 0 {
                        self.tick(i);
                    }
                }
                println("flooder done");
            }
        }

        main locus App {
            params {
                state: State = State { };
                flood: Flooder = Flooder { };
            }
            placement {
                state: cooperative(pool = main);
                flood: pinned;
            }
            run() {
                // Same-pool field read: poll until every cell is
                // delivered (bounded — ~60s worst case).
                let mut t = 0;
                while self.state.seen < 1000000 && t < 1200 {
                    std::time::sleep(50ms);
                    t = t + 1;
                }
                println("delivered=", self.state.seen);
            }
        }

        fn main() { App { }; }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = unique_path("flood");
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "flood run died (pre-fix: SIGSEGV/SIGABRT in the retire \
         freelist): {:?}\nstdout: {}\nstderr: {}",
        out.status,
        stdout,
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.contains("delivered=1000000"),
        "every cell must still be delivered (on main's drains); got: {}",
        stdout
    );
}

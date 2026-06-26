//! Drain<T> batch consumer (2026-06-26) — end-to-end test for the
//! shm_ring BATCH dispatch mode.
//!
//! A batch subscriber's handler takes `Drain<T>` instead of the
//! per-record payload `T`. The runtime calls the handler ONCE per
//! available batch with a `Drain<T>` handle; the handler loops over
//! the records inline with `for t in feed { ... }` — no per-record
//! function call. This is the perf path for cross-process delivery.
//!
//! The program (single process):
//!   - App binds `Quotes` to an shm_ring.
//!   - Agg subscribes `Quotes as on_quotes` with a `Drain<Tick>`
//!     param → registers through the BATCH reader thread.
//!   - Feed publishes three ticks (px = 10, 20, 30) in birth().
//!   - on_quotes sums px into self.total and prints `agg total=N`.
//!
//! We assert the final printed total == 60 (10 + 20 + 30), which
//! proves: the batch register path was taken, the batch reader
//! handed the handler a `{ring, start, end}` handle, and the inline
//! `for t in feed` read each record zero-copy through the slot
//! pointer (`t.px` GEPs into the mapped ring).

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use hale_codegen::build_executable;

fn unique_tag(label: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("drain-{}-{}-{}", label, std::process::id(), nanos)
}

fn build_binary(src: &str, label: &str) -> PathBuf {
    let prog = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("lotus_shm_drain_{}.bin", unique_tag(label)));
    build_executable(&prog, &bin).expect("build");
    bin
}

#[test]
fn drain_batch_handler_sums_records_inline() {
    let shm_name = format!("/hale-{}", unique_tag("e2e"));

    let src = format!(
        r#"
        type Tick {{ px: Int; sz: Int; }}
        topic Quotes {{ payload: Tick; subject: "quotes-{tag}"; }}

        locus Agg {{
            params {{ total: Int = 0; }}
            bus {{ subscribe Quotes as on_quotes; }}
            fn on_quotes(feed: Drain<Tick>) {{
                for t in feed {{
                    self.total = self.total + t.px;
                }}
                println("agg total=", self.total);
            }}
        }}

        locus Feed {{
            bus {{ publish Quotes; }}
            birth() {{
                Quotes <- Tick {{ px: 10, sz: 1 }};
                Quotes <- Tick {{ px: 20, sz: 2 }};
                Quotes <- Tick {{ px: 30, sz: 3 }};
            }}
        }}

        main locus App {{
            bindings {{
                Quotes: shm_ring("{shm_name}", slot_count: 1024, on_overflow: drop) where zero_copy;
            }}
        }}

        fn main() {{
            App {{ }};
            Agg {{ }};
            Feed {{ }};
            // Give the batch reader thread time to receive + dispatch
            // the published batch before the process exits.
            time::sleep(500ms);
        }}
    "#,
        tag = unique_tag("topic"),
        shm_name = shm_name,
    );

    let bin = build_binary(&src, "agg");
    let out = Command::new(&bin).output().expect("run drain example");
    let _ = std::fs::remove_file(&bin);

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "drain example failed: status={:?}\nstdout: {}\nstderr: {}",
        out.status,
        stdout,
        stderr,
    );

    // The handler may fire once per batch; the LAST printed total is
    // the cumulative sum. Assert the example observed total == 60.
    assert!(
        stdout.contains("agg total=60"),
        "expected `agg total=60` (10+20+30) in stdout; got:\n{}",
        stdout
    );

    // atexit cleanup: the ring's /dev/shm object is unlink'd on exit.
    let stripped = shm_name.trim_start_matches('/');
    let shm_path = PathBuf::from(format!("/dev/shm/{}", stripped));
    assert!(
        !shm_path.exists(),
        "atexit cleanup failed: `{}` persists in /dev/shm/",
        shm_name
    );
}

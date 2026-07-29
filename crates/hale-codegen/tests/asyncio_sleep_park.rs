//! Crumb batch-5 item 1 — `std::time::sleep` on a `where async_io`
//! pool must PARK the coro, not block the shared worker.
//!
//! Three dynamically-spawned children each sleep 400ms on ONE
//! async_io pool. Blocking-in-nanosleep serializes them (wakes at
//! 400/800/1200ms — the M3b symptom: one `await sleep(400)` in a JS
//! handler held the engine pool against unrelated requests); a
//! parked coro yields the worker, so all three wake at ~400ms.
//! The classic (non-async) chunked-nanosleep path — including the
//! ≤100ms slice + per-slice bus drain that keeps main-thread
//! handlers serviced — is unchanged; the park preflight only fires
//! in an async_io coro context.

use std::process::Command;

use hale_codegen::build_executable;

const SRC: &str = r#"
    type Go { n: Int; }

    locus Waiter {
        params { n: Int = 0; t0: Int = 0; }
        run() {
            std::time::sleep(400ms);
            println("woke ", self.n, " ",
                (std::time::monotonic_ns() - self.t0) / 1000000);
        }
    }

    locus Host {
        params { t0: Int = 0; }
        bus { subscribe "go" as on_go of type Go; }
        accept(c: Waiter) { }
        fn on_go(g: Go) {
            Waiter { n: g.n, t0: self.t0 };
        }
    }

    main locus App {
        params {
            t0: Int = 0;
            host: Host = Host { };
        }
        placement {
            host: cooperative(pool = web) where async_io;
        }
        bus { publish "go" of type Go; }
        run() {
            self.host.t0 = self.t0;
            "go" <- Go { n: 1 };
            "go" <- Go { n: 2 };
            "go" <- Go { n: 3 };
            std::time::sleep(1600ms);
        }
    }

    fn main() {
        App { t0: std::time::monotonic_ns() };
        return 0;
    }
"#;

#[test]
fn asyncio_sleeps_park_and_overlap() {
    let program = hale_syntax::parse_source(SRC).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_sleep_park_{}", std::process::id()));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let wakes: Vec<i64> = stdout
        .lines()
        .filter(|l| l.starts_with("woke "))
        .filter_map(|l| l.split_whitespace().last()?.parse().ok())
        .collect();
    assert_eq!(wakes.len(), 3, "expected 3 wakes; stdout: {:?}", stdout);
    // Parked: all ~400ms. Serialized: 400/800/1200. The bound is
    // deliberately loose against CI jitter but far below the
    // second serialized wake.
    for w in &wakes {
        assert!(
            *w >= 380 && *w < 700,
            "sleep did not park — wake at {}ms (serialized would be \
             400/800/1200): {:?}",
            w,
            wakes
        );
    }
}

//! GH #2183: the `async_io` pool's expiry sweep reads the clock once.
//!
//! The sweep resumes every timed park whose deadline has passed. It used
//! to read the clock again before each resume, so a coroutine that parked
//! again during the sweep was resumed again in the same sweep as soon as
//! its new deadline passed. Two coroutines whose round (work, then a short
//! sleep) outlasts their own sleep then pass the worker back and forth:
//! while one works, the other's sleep runs out. The sweep never ends, and
//! the pool never drains the cell their loops wait for.
//!
//! Here two `run()`s on one `async_io` pool each spin 3 ms and sleep 1 ms
//! until a `go` delivery (a cell on the same pool) sets their flag. With
//! the clock read once per sweep, each is resumed at most once per sweep,
//! so the pool drains `go` and both loops end. On the old runtime the
//! program never ends; the test bounds it and reads the missing lines.

use std::process::Command;
use std::time::Duration;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

const SRC: &str = r#"
    type Go { n: Int; }
    type Done { n: Int; }

    locus Spinner {
        params { n: Int = 0; go: Bool = false; }
        bus {
            subscribe "go" as on_go of type Go;
            publish "done" of type Done;
        }
        fn on_go(g: Go) { self.go = true; }
        run() {
            while !self.go {
                let t = std::time::monotonic_ns();
                while std::time::monotonic_ns() - t < 3000000 { }
                std::time::sleep(1ms);
            }
            println("done ", self.n);
            "done" <- Done { n: self.n };
        }
    }

    main locus App {
        params {
            a: Spinner = Spinner { n: 1 };
            b: Spinner = Spinner { n: 2 };
            finished: Int = 0;
        }
        placement {
            a: cooperative(pool = io) where async_io;
            b: cooperative(pool = io) where async_io;
        }
        bus {
            publish "go" of type Go;
            subscribe "done" as on_done of type Done;
        }
        fn on_done(d: Done) { self.finished = self.finished + 1; }
        run() {
            std::time::sleep(50ms);
            "go" <- Go { n: 1 };
            while self.finished < 2 { std::time::sleep(5ms); }
            println("both done");
        }
    }

    fn main() { App { }; }
"#;

#[test]
fn a_round_longer_than_its_sleep_lets_the_inbox_drain() {
    let bin = harness::unique_bin("hale_asyncio_sweep_once");
    build_opts::build_source(SRC, &bin, &build_opts::options()).expect("build");
    let out = harness::output_within(&mut Command::new(&bin), Duration::from_secs(20));
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    for line in ["done 1", "done 2", "both done"] {
        assert!(
            stdout.lines().any(|l| l == line),
            "`{line}` missing: the expiry sweep kept resuming the spinners \
             and never let the pool drain `go`\nstatus: {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}",
            out.status,
        );
    }
    assert!(out.status.success(), "status {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}", out.status);
}

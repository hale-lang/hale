//! GH #255 phase 2 — bounded topics + consumer shed bounds.
//!
//! Three contracts:
//!   1. `bounded(N, drop_old)` on a main-queue subscriber keeps
//!      the NEWEST N queued deliveries when a burst outruns the
//!      drain (ring semantics — the last event must survive).
//!   2. A topic `bounded(N) on_full: fail` publish with
//!      `or raise` refuses synchronously (BusFull panic) when a
//!      subscriber sits at capacity.
//!   3. The same burst with `or wait` parks the publisher until
//!      the drain frees space — nothing is shed, all events
//!      arrive.
//!
//! GH #2138: the subscriber is a main-queue one, which only main
//! drains (at its sleep slices), while the pinned publisher bursts
//! on its own thread, so whether a burst outran the drain was the
//! scheduler's choice: a main that drained between sends kept the
//! queue under the bound, and `on_full_fail_or_raise_refuses` saw
//! its burst survive. In the first two the subscriber's first
//! delivery now holds the queue: its handler waits on a latch the
//! publisher sets after its burst (`latch()`, a file named by the
//! pid; a sleep in a handler does not drain), and main's run() waits
//! for the same latch. Nothing drains during the burst, so the queue
//! is at its bound by the third send whatever the scheduler does.
//! The third test waits for its 50 deliveries instead of a fixed
//! 1.5 s, which a slow drain could outlast.

use std::path::PathBuf;
use std::process::Command;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;
#[path = "../../hale-types/tests/support/entries.rs"]
mod entries;

fn build(name: &str, src: &str) -> PathBuf {
    // The harness lowers without the check; each program here is held to
    // it as well, as `hale build` holds a user's. Until GH #2146 the
    // check refused `or raise` on an unkeyed `on_full: fail` topic, so
    // only this harness could build `on_full_fail_or_raise_refuses`.
    let program = hale_syntax::parse_source(src).expect("parses");
    let errors: Vec<String> = entries::check_program(&program)
        .into_iter()
        .filter(|d| d.is_error())
        .map(|d| d.message)
        .collect();
    assert!(errors.is_empty(), "`{name}` does not check: {errors:?}");
    let bin = harness::unique_bin(&format!("hale_test_busbound_{}", name));
    build_opts::build_source(src, &bin, &build_opts::options()).expect("build");
    bin
}

#[test]
fn drop_old_keeps_newest_under_burst() {
    // Pinned publisher bursts 100 events while the subscriber's
    // first delivery holds the queue (no drain); the
    // bounded(4, drop_old) subscriber must see the TAIL of the
    // burst — event 99 — and strictly fewer than 100 total.
    let src = r#"
        fn latch() -> String {
            return "/tmp/hale_test_busbound_latch_" + to_string(std::process::pid());
        }
        // up to 10 s, so a burst that never sets it still ends
        fn await_latch() {
            let deadline = std::time::monotonic_ns() + 10000000000;
            while !std::io::fs::file_exists(latch()) && std::time::monotonic_ns() < deadline {
                std::time::sleep(1ms);
            }
        }
        type E { n: Int = 0; }
        topic Evt { payload: E; subject: "evt"; }
        locus Tally {
            params { seen: Int = 0; last: Int = -1; }
            bus {
                subscribe Evt as on_e bounded(4, drop_old);
            }
            dissolve() {
                if self.last != 99 {
                    println("BAD last=", self.last);
                    std::process::exit(1);
                }
                if self.seen > 60 {
                    println("BAD no shedding, seen=", self.seen);
                    std::process::exit(1);
                }
                println("ok seen=", self.seen, " last=", self.last);
            }
            fn on_e(e: E) {
                // the first delivery holds the queue until the burst
                // is over (a sleep in a handler does not drain)
                if self.seen == 0 { await_latch(); }
                self.seen = self.seen + 1;
                self.last = e.n;
            }
        }
        locus Pusher {
            bus { publish Evt; }
            run() {
                std::time::sleep(250ms);
                let mut i = 0;
                while i < 100 {
                    Evt <- E { n: i };
                    i = i + 1;
                }
                std::io::fs::write_file(latch(), "") or discard;
            }
        }
        main locus App {
            params {
                tally: Tally = Tally { };
                p: Pusher = Pusher { };
            }
            placement { p: pinned(core = 0); }
            run() {
                await_latch();
                std::time::sleep(50ms);
                std::io::fs::unlink(latch()) or discard;
            }
        }
        fn main() { App { }; }
    "#;
    let bin = build("dropold", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("ok seen="),
        "stdout: {:?}\nstderr: {:?}",
        stdout,
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn on_full_fail_or_raise_refuses() {
    // The pinned publisher's third send finds the subscriber at its
    // bound — one delivery held in its handler, two queued (or two
    // queued and none taken yet) — and raises BusFull.
    let src = r#"
        fn latch() -> String {
            return "/tmp/hale_test_busbound_latch_" + to_string(std::process::pid());
        }
        // up to 10 s, so a burst that never sets it still ends
        fn await_latch() {
            let deadline = std::time::monotonic_ns() + 10000000000;
            while !std::io::fs::file_exists(latch()) && std::time::monotonic_ns() < deadline {
                std::time::sleep(1ms);
            }
        }
        type E { n: Int = 0; }
        topic Evt {
            payload: E;
            subject: "evt";
            bounded(2);
            on_full: fail;
        }
        locus Tally {
            params { seen: Int = 0; }
            bus { subscribe Evt as on_e; }
            fn on_e(e: E) {
                // the first delivery holds the queue until the burst
                // is over, so the bound is reached by the third send
                if self.seen == 0 { await_latch(); }
                self.seen = self.seen + 1;
            }
        }
        locus Pusher {
            bus { publish Evt; }
            run() {
                std::time::sleep(250ms);
                let mut i = 0;
                while i < 10 {
                    Evt <- E { n: i } or raise;
                    i = i + 1;
                }
                println("BURST SURVIVED (should have raised)");
                std::io::fs::write_file(latch(), "") or discard;
            }
        }
        main locus App {
            params {
                tally: Tally = Tally { };
                p: Pusher = Pusher { };
            }
            placement { p: pinned(core = 0); }
            run() {
                await_latch();
                std::io::fs::unlink(latch()) or discard;
            }
        }
        fn main() { App { }; }
    "#;
    let bin = build("fullraise", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success() && !stdout.contains("BURST SURVIVED"),
        "expected a BusFull raise.\nstdout: {:?}\nstderr: {:?}",
        stdout,
        stderr
    );
    assert!(
        stderr.contains("BusFull"),
        "expected the BusFull diagnostic.\nstderr: {:?}",
        stderr
    );
}

#[test]
fn on_full_or_wait_delivers_everything() {
    // Same shape, but the publisher waits for space: main's
    // sliced sleep drains between bursts, the parked publisher
    // resumes, and every event arrives — nothing shed, nothing
    // refused.
    let src = r#"
        type E { n: Int = 0; }
        topic Evt {
            payload: E;
            subject: "evt";
            bounded(10);
            on_full: fail;
        }
        locus Tally {
            params { seen: Int = 0; }
            bus { subscribe Evt as on_e; }
            dissolve() {
                if self.seen != 50 {
                    println("MISSED: saw ", self.seen, " of 50");
                    std::process::exit(1);
                }
                println("all 50 delivered");
            }
            fn on_e(e: E) { self.seen = self.seen + 1; }
        }
        locus Pusher {
            bus { publish Evt; }
            run() {
                std::time::sleep(200ms);
                let mut i = 0;
                while i < 50 {
                    Evt <- E { n: i } or wait;
                    i = i + 1;
                }
            }
        }
        main locus App {
            params {
                tally: Tally = Tally { };
                p: Pusher = Pusher { };
            }
            placement { p: pinned(core = 0); }
            run() {
                // until every event is delivered (up to 10 s), not a
                // fixed sleep a slow drain could outlast
                let deadline = std::time::monotonic_ns() + 10000000000;
                while self.tally.seen < 50 && std::time::monotonic_ns() < deadline {
                    std::time::sleep(10ms);
                }
            }
        }
        fn main() { App { }; }
    "#;
    let bin = build("fullwait", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("all 50 delivered"),
        "stdout: {:?}\nstderr: {:?}",
        stdout,
        String::from_utf8_lossy(&out.stderr)
    );
}

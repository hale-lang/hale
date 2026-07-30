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

use std::path::PathBuf;
use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn build(name: &str, src: &str) -> PathBuf {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(&format!("hale_test_busbound_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

#[test]
fn drop_old_keeps_newest_under_burst() {
    // Pinned publisher bursts 100 events while main sleeps (no
    // drain); the bounded(4, drop_old) subscriber must see the
    // TAIL of the burst — event 99 — and strictly fewer than
    // 100 total. (A drain slice may land mid-burst, so the exact
    // count is timing-dependent; the ring property is not.)
    let src = r#"
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
            }
        }
        main locus App {
            params {
                tally: Tally = Tally { };
                p: Pusher = Pusher { };
            }
            placement { p: pinned(core = 0); }
            run() {
                std::time::sleep(600ms);
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
    let src = r#"
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
            fn on_e(e: E) { self.seen = self.seen + 1; }
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
            }
        }
        main locus App {
            params {
                tally: Tally = Tally { };
                p: Pusher = Pusher { };
            }
            placement { p: pinned(core = 0); }
            run() {
                std::time::sleep(600ms);
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
                std::time::sleep(1500ms);
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

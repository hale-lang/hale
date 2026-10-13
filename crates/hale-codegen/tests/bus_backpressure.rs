//! GH #125 — bounded bus queue + backpressure.
//!
//! A producer that floods the cooperative bus (publishes a large batch
//! before the consumer drains) must not grow resident memory without
//! bound. Pre-#125 the queue doubled on every fill, so a `birth()` that
//! published 2M messages buffered the whole backlog (~1 GB, 2M cells ×
//! ~552 B). Now the single-threaded publisher BLOCKS once the queue hits
//! the cap — it inline-drains the queue (runs the oldest handlers) to make
//! space — so resident memory stays bounded while every message is still
//! delivered.

use std::process::Command;

/// Build, run, and read `final_rss_mb=` from stdout (MB). Panics if the
/// program crashes or never prints the line — which also asserts the flood
/// ran to completion (the line is only printed once the count reaches N).
#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

/// The megabytes the program reports for ITSELF, out of the
/// `/proc/self/statm` line it prints (`harness::statm_resident_bytes`).
/// `std::process::rss_bytes()` cannot be used: a spawned program
/// inherits its parent's RSS high-water mark through fork+exec, so
/// under `cargo test` these readings clamped to the libtest
/// harness's own footprint (~135 MB with several in-process LLVM
/// builds running) — two thirds of the 200 MB bound below, before
/// this program allocated anything. See GH #772.
fn build_and_rss(name: &str, src: &str) -> i64 {
    let bin = harness::unique_bin(&format!("hale_bus_bp_{}", name));
    build_opts::build_source(src, &bin, &build_opts::options()).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(output.status.success(), "{} crashed: {:?}", name, output.status);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let statm = stdout
        .lines()
        .find_map(|l| l.strip_prefix("rss_statm="))
        .unwrap_or_else(|| panic!("no rss_statm in {} stdout: {:?}", name, stdout));
    harness::statm_resident_bytes(statm) / 1048576
}

/// A `Source` whose `birth()` publishes 2M ticks before yielding — the
/// flooding producer. The `Sink` counts them and prints RSS once it has
/// seen all N (so the print also proves none were lost).
const FLOOD: &str = r#"
    type Tick { n: Int; }
    locus Sink {
        params { count: Int = 0; n: Int = 0; acc: Int = 0; }
        bus { subscribe "tick" as on_tick of type Tick; }
        fn on_tick(t: Tick) {
            self.acc = self.acc + t.n;
            self.count = self.count + 1;
            if self.count == self.n {
                print("rss_statm=");
                println(std::io::fs::read_file("/proc/self/statm") or "");
            }
        }
    }
    locus Source {
        params { n: Int = 0; }
        bus { publish "tick" of type Tick; }
        birth() { let mut i = 0; while i < self.n { "tick" <- Tick { n: i }; i = i + 1; } }
    }
    fn main() { Sink { n: 2000000 }; Source { n: 2000000 }; }
"#;

/// An `any -> pinned` flood: the main locus floods a **pinned** subscriber's
/// mailbox cross-thread (different OS threads). The mailbox is bounded and
/// the producer back-pressures — it blocks on the mailbox's `not_full`
/// condvar until the pinned consumer drains — so RSS stays bounded. The
/// `run()` waits for the count before printing, which also proves no
/// deadlock and that every message was delivered.
const PINNED_FLOOD: &str = r#"
    type Tick { n: Int; }
    locus Counter {
        params { count: Int = 0; }
        bus { subscribe "px" as on_tick of type Tick; }
        fn on_tick(t: Tick) { self.count = self.count + 1; }
    }
    main locus App {
        params { sub: Counter = Counter { }; }
        placement { sub: pinned; }
        bus { publish "px" of type Tick; }
        run() {
            let mut i = 0;
            while i < 2000000 { "px" <- Tick { n: i }; i = i + 1; }
            while self.sub.count < 2000000 { std::time::sleep(1ms); }
            print("rss_statm=");
            println(std::io::fs::read_file("/proc/self/statm") or "");
        }
    }
    fn main() { App { }; }
"#;

#[test]
fn flood_producer_does_not_grow_rss_unbounded() {
    let rss = build_and_rss("flood", FLOOD);
    // Unbounded (pre-#125) this floods to ~1 GB (2M cells × ~552 B). Bounded
    // at the default 8192-cell cap it stays well under 200 MB — the gap is
    // ~20x, so an absolute threshold is robust to build-config RSS variance.
    // Reaching the print at all also proves all 2M messages were delivered.
    assert!(
        rss < 200,
        "flood RSS {} MB — the bus queue did not stay bounded (expected < 200; \
         unbounded would be ~1 GB)",
        rss
    );
}

#[test]
fn cross_thread_flood_to_pinned_mailbox_is_bounded() {
    let rss = build_and_rss("pinned_flood", PINNED_FLOOD);
    // Unbounded the mailbox would flood to ~1 GB (2M cells × ~552 B).
    // Bounded + back-pressured it stays bounded; reaching the print also
    // proves no deadlock and all 2M messages delivered.
    assert!(
        rss < 200,
        "any->pinned flood RSS {} MB — the mailbox did not back-pressure \
         (expected < 200; unbounded would be ~1 GB)",
        rss
    );
}

/// A `pool -> main` flood: `Source`, on pool `side`, publishes 2M ticks to
/// a subscriber on main while main's `run()` spins with no drain point, so
/// main's cooperative queue is the only place the cells can wait. Main
/// spins until the source has sent a full queue's worth, then 300 ms more,
/// and prints its residency and the count sent so far while the backlog is
/// live; then it sleeps (a drain point) until all 2M arrived.
const POOL_TO_MAIN_FLOOD: &str = r#"
    type Tick { n: Int; }
    locus Counter {
        params { count: Int = 0; }
        bus { subscribe "pm" as on_tick of type Tick; }
        fn on_tick(t: Tick) { self.count = self.count + 1; }
    }
    locus Source {
        params { sent: Int = 0; }
        bus { publish "pm" of type Tick; }
        run() { while self.sent < 2000000 { "pm" <- Tick { n: self.sent }; self.sent = self.sent + 1; } }
    }
    main locus App {
        params { sub: Counter = Counter { }; src: Source = Source { }; }
        placement { src: cooperative(pool = side); }
        run() {
            let full = std::time::monotonic_ns() + 10000000000;
            while (self.src.sent < 8192 && std::time::monotonic_ns() < full) { }
            let more = std::time::monotonic_ns() + 300000000;
            while (std::time::monotonic_ns() < more) { }
            let statm = std::io::fs::read_file("/proc/self/statm") or "";
            println("sent_while_spinning=" + to_string(self.src.sent));
            print("rss_statm=");
            println(statm);
            while self.sub.count < 2000000 { std::time::sleep(1ms); }
            println("delivered=" + to_string(self.sub.count));
        }
    }
    fn main() { App { }; }
"#;

/// Build and run `src`, and read its `key=value` lines (the program prints
/// them while the memory being measured is live, then runs to the end).
fn run_fields(name: &str, src: &str) -> impl Fn(&str) -> String + use<> {
    let bin = harness::unique_bin(&format!("hale_bus_bp_{name}"));
    build_opts::build_source(src, &bin, &build_opts::options()).expect("build");
    let output = Command::new("timeout").arg("120").arg(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(output.status.success(), "{name}: {:?}\n{stdout}", output.status);
    let name = name.to_string();
    move |key: &str| {
        let prefix = format!("{key}=");
        stdout.lines().find_map(|l| l.strip_prefix(&prefix)).unwrap_or_else(|| panic!("{name}: no {key} in {stdout:?}")).to_string()
    }
}

/// Main's queue is bounded for a producer on another thread too. Before,
/// the cross-thread post grew main's queue without bound (the locked path
/// skipped the cap): the 2M ticks were all sent while main spun, and held
/// 1.1 GB. A spinning main drains nothing, so the source takes it for
/// blocked and goes on a queue's worth (8192) per 20 ms stall window:
/// about 115k ticks in the 300 ms spin, ~75 MB. Every tick is delivered
/// once main drains.
#[test]
fn cross_thread_flood_to_main_queue_is_bounded() {
    let field = run_fields("pool_to_main", POOL_TO_MAIN_FLOOD);
    let rss = harness::statm_resident_bytes(&field("rss_statm")) / 1048576;
    let sent: i64 = field("sent_while_spinning").parse().expect("a count");
    assert_eq!(field("delivered"), "2000000", "every tick is delivered");
    assert!(sent < 500_000, "the source ran on past main's bound: {sent} sent while main spun");
    assert!(rss < 200, "pool->main flood RSS {rss} MB with {sent} sent while main spun (expected < 200)");
}

/// A producer never waits on an owner that is not draining (PR #1552's
/// review). Main sends `go` over a socket and blocks reading the reply; the
/// pool's worker reads `go`, publishes 100000 ticks to a main subscriber
/// (more than main's queue holds), and only then replies `done`. Main's
/// read is no runtime wait, so a bound that held the worker until main's
/// next drain closed the cycle: neither side went on. Main's heartbeat
/// stands still, so the worker goes on and replies.
const OWNER_BLOCKED_IN_READ: &str = r#"
    type Tick { n: Int; }
    locus Counter {
        params { count: Int = 0; }
        bus { subscribe "bk" as on_tick of type Tick; }
        fn on_tick(t: Tick) { self.count = self.count + 1; }
    }
    locus Worker {
        params { port: Int = 0; }
        bus { publish "bk" of type Tick; }
        run() {
            let fd = std::io::tcp::connect("127.0.0.1", self.port) or raise;
            let s = std::io::tcp::Stream { conn_fd: fd };
            let go = s.recv(16) or raise;
            let mut i = 0;
            while i < 100000 { "bk" <- Tick { n: i }; i = i + 1; }
            s.send("done") or raise;
        }
    }
    main locus App {
        params { lfd: Int = 0; sub: Counter = Counter { }; w: Worker = Worker { }; }
        placement { w: cooperative(pool = side); }
        run() {
            let cfd = std::io::tcp::accept_one(self.lfd) or raise;
            let s = std::io::tcp::Stream { conn_fd: cfd };
            s.send("go") or raise;
            let reply = s.recv(16) or raise;
            println("reply=" + reply);
            let deadline = std::time::monotonic_ns() + 30000000000;
            while (self.sub.count < 100000 && std::time::monotonic_ns() < deadline) { std::time::sleep(1ms); }
            println("delivered=" + to_string(self.sub.count));
        }
    }
    fn main() {
        let lfd = std::io::tcp::listen_socket("127.0.0.1", 0) or raise;
        let port = std::io::tcp::__local_port(lfd);
        App { lfd: lfd, w: Worker { port: port } };
    }
"#;

#[test]
fn a_producer_never_waits_on_an_owner_blocked_in_a_read() {
    let field = run_fields("owner_in_read", OWNER_BLOCKED_IN_READ);
    assert_eq!(field("reply"), "done");
    assert_eq!(field("delivered"), "100000", "every tick is delivered");
}

/// The same for an owner blocked in an `@ffi` call, which the runtime
/// cannot see: main sleeps a second in `usleep` while the source publishes
/// 100000 ticks to its subscriber, and the source finishes inside that
/// second (a cap per 20 ms stall window), not at main's return.
const OWNER_BLOCKED_IN_FFI: &str = r#"
    @ffi("c") fn usleep(us: Int) -> Int;

    type Tick { n: Int; }
    locus Counter {
        params { count: Int = 0; }
        bus { subscribe "fb" as on_tick of type Tick; }
        fn on_tick(t: Tick) { self.count = self.count + 1; }
    }
    locus Source {
        params { sent: Int = 0; }
        bus { publish "fb" of type Tick; }
        run() { while self.sent < 100000 { "fb" <- Tick { n: self.sent }; self.sent = self.sent + 1; } }
    }
    main locus App {
        params { sub: Counter = Counter { }; src: Source = Source { }; }
        placement { src: cooperative(pool = side); }
        run() {
            let started = std::time::monotonic_ns() + 10000000000;
            while (self.src.sent == 0 && std::time::monotonic_ns() < started) { }
            usleep(1000000);
            println("sent_while_blocked=" + to_string(self.src.sent));
            let deadline = std::time::monotonic_ns() + 30000000000;
            while (self.sub.count < 100000 && std::time::monotonic_ns() < deadline) { std::time::sleep(1ms); }
            println("delivered=" + to_string(self.sub.count));
        }
    }
    fn main() { App { }; }
"#;

#[test]
fn a_producer_never_waits_on_an_owner_blocked_in_ffi() {
    let field = run_fields("owner_in_ffi", OWNER_BLOCKED_IN_FFI);
    assert_eq!(field("sent_while_blocked"), "100000", "the source waited on main's foreign call");
    assert_eq!(field("delivered"), "100000", "every tick is delivered");
}

/// A subscriber born on main while a pool floods its topic: its birth
/// spins 50 ms, so the flood parks a full queue's worth for it and waits
/// at the readiness cap. Main's post to its own queue never waits, so
/// readying it in rounds never ended while the source ran: each round
/// posted the list the source had parked during the last, and main's queue
/// took the whole flood (2M sent before `Counter { }` returned, 1.1 GB;
/// under a loaded box the same loop reached 16 GB in a failure-delivery
/// fixture). Readying it now posts the list and closes the window in one
/// hold, and the source then waits at main's full queue.
const READY_ON_MAIN_MID_FLOOD: &str = r#"
    type Tick { n: Int; }
    locus Counter {
        params { count: Int = 0; }
        bus { subscribe "rw" as on_tick of type Tick; }
        birth() {
            let until = std::time::monotonic_ns() + 50000000;
            while (std::time::monotonic_ns() < until) { }
        }
        fn on_tick(t: Tick) { self.count = self.count + 1; }
    }
    locus Source {
        params { sent: Int = 0; }
        bus { publish "rw" of type Tick; }
        run() { while self.sent < 2000000 { "rw" <- Tick { n: self.sent }; self.sent = self.sent + 1; } }
    }
    main locus App {
        params { src: Source = Source { }; }
        placement { src: cooperative(pool = side); }
        run() {
            let started = std::time::monotonic_ns() + 10000000000;
            while (self.src.sent == 0 && std::time::monotonic_ns() < started) { }
            let c = Counter { };
            let statm = std::io::fs::read_file("/proc/self/statm") or "";
            let after = self.src.sent;
            println("sent_after_birth=" + to_string(after));
            print("rss_statm=");
            println(statm);
            let deadline = std::time::monotonic_ns() + 60000000000;
            while (c.count < 2000000 - after && std::time::monotonic_ns() < deadline) { std::time::sleep(1ms); }
            println("missing=" + to_string(2000000 - after - c.count));
        }
    }
    fn main() { App { }; }
"#;

#[test]
fn a_subscriber_readied_on_main_mid_flood_closes_its_window() {
    let field = run_fields("ready_on_main", READY_ON_MAIN_MID_FLOOD);
    let rss = harness::statm_resident_bytes(&field("rss_statm")) / 1048576;
    let sent: i64 = field("sent_after_birth").parse().expect("a count");
    assert!(sent < 4 * 8192, "readying the subscriber went on with the flood: {sent} sent before it returned");
    assert!(rss < 200, "ready-on-main RSS {rss} MB with {sent} sent before readiness returned (expected < 200)");
    let missing: i64 = field("missing").parse().expect("a count");
    assert!(missing <= 0, "{missing} ticks sent after the subscriber's birth never reached it");
}

/// A flood whose handler publishes: the at-cap inline drain runs `Sink`'s
/// handler inside the `Source`'s post, and that handler publishes a
/// `Reply`. The nested publish used to clear the wire deserializer the
/// outer post had set before the outer cell read it (the cell then ran as
/// a materialized struct, its serialized bytes read as pointers), and to
/// re-encode into the thread's wire buffer that the outer post's small
/// payload, and the publisher's next subscriber, still read. Two sinks,
/// so a publisher fans one encode out past a post that drained inline.
/// `@PAD@` sets the body: a short one stays in the cell, a 700-byte one
/// spills to the heap.
const REENTRANT_FLOOD: &str = r#"
    type Tick { n: Int; body: String; }
    type Reply { body: String; }
    locus Sink {
        params { count: Int = 0; n: Int = 0; bad: Int = 0; pad: String = ""; tag: String = ""; }
        bus { subscribe "tick" as on_tick of type Tick; publish "reply" of type Reply; }
        fn on_tick(t: Tick) {
            "reply" <- Reply { body: "nested" };
            if t.body != self.pad + to_string(t.n) { self.bad = self.bad + 1; }
            self.count = self.count + 1;
            if self.count == self.n { println(self.tag + " ticks=" + to_string(self.count) + " bad=" + to_string(self.bad)); }
        }
    }
    locus Echo {
        params { count: Int = 0; bad: Int = 0; }
        bus { subscribe "reply" as on_reply of type Reply; }
        fn on_reply(r: Reply) {
            if r.body != "nested" { self.bad = self.bad + 1; }
            self.count = self.count + 1;
            if self.count == 20000 { println("replies=" + to_string(self.count) + " bad=" + to_string(self.bad)); }
        }
    }
    locus Source {
        params { n: Int = 0; pad: String = ""; }
        bus { publish "tick" of type Tick; }
        birth() { let mut i = 0; while i < self.n { "tick" <- Tick { n: i, body: self.pad + to_string(i) }; i = i + 1; } }
    }
    fn main() {
        let pad = @PAD@;
        Echo { };
        Sink { n: 10000, pad: pad, tag: "a" };
        Sink { n: 10000, pad: pad, tag: "b" };
        Source { n: 10000, pad: pad };
    }
"#;

/// Every body arrives intact, in both dispatch modes, plain and under ASan
/// with chunk pooling off (GH #816).
fn reentrant_flood_delivers_every_body(name: &str, pad: &str) {
    let src = REENTRANT_FLOOD.replace("@PAD@", pad);
    for asan in [false, true] {
        for no_bus_devirt in [false, true] {
            let what = format!("{name}: asan={asan}, no_bus_devirt={no_bus_devirt}");
            let bin = harness::unique_bin(&format!("hale_bus_bp_{name}_{asan}_{no_bus_devirt}"));
            let options = hale_codegen::BuildOptions { asan, no_bus_devirt, ..build_opts::options() };
            build_opts::build_source(&src, &bin, &options).unwrap_or_else(|e| panic!("{what}: build: {e:?}"));
            let out = Command::new("timeout")
                .arg("120")
                .arg(&bin)
                .env("ASAN_OPTIONS", "detect_leaks=0")
                .env("LOTUS_NO_CHUNK_POOL", "1")
                .output()
                .expect("run");
            let _ = std::fs::remove_file(&bin);
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            assert!(!stderr.contains("ERROR: AddressSanitizer"), "{what}: the sanitizer reported\n{stderr}");
            assert_eq!(out.status.code(), Some(0), "{what}: exit\n{stdout}\n{stderr}");
            let mut lines: Vec<&str> = stdout.lines().collect();
            lines.sort();
            assert_eq!(lines, ["a ticks=10000 bad=0", "b ticks=10000 bad=0", "replies=20000 bad=0"], "{what}\n{stderr}");
        }
    }
}

#[test]
fn a_flood_whose_handler_publishes_keeps_small_payloads_intact() {
    reentrant_flood_delivers_every_body("reentrant_small", "\"b\"");
}

#[test]
fn a_flood_whose_handler_publishes_keeps_spilled_payloads_intact() {
    reentrant_flood_delivers_every_body("reentrant_spilled", "std::str::repeat(\"x\", 700)");
}

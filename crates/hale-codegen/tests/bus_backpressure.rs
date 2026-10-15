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
    run_fields_env(name, src, &[])
}

/// [`run_fields`] with `env` set on the program.
fn run_fields_env(name: &str, src: &str, env: &[(&str, &str)]) -> impl Fn(&str) -> String + use<> {
    let bin = harness::unique_bin(&format!("hale_bus_bp_{name}"));
    build_opts::build_source(src, &bin, &build_opts::options()).expect("build");
    let output =
        Command::new("timeout").arg("120").arg(&bin).envs(env.iter().copied()).output().expect("run");
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

/// The stall window is the queue's, not each producer's (PR #1552's
/// review): four sources on four pools flood one main subscriber while
/// main sleeps a second in `usleep`, with a 64-cell queue. A raise of the
/// bound starts the window again for every producer waiting, so the queue
/// grows a cap per 20 ms in all. With a window per producer, each one whose
/// own clock had run out raised the bound again at once: about 11.5k sent
/// in the second, where the bound allows about 3.3k.
const FOUR_PRODUCERS_OWNER_IN_FFI: &str = r#"
    @ffi("c") fn usleep(us: Int) -> Int;

    type Tick { n: Int; }
    locus Counter {
        params { count: Int = 0; }
        bus { subscribe "f4" as on_tick of type Tick; }
        fn on_tick(t: Tick) { self.count = self.count + 1; }
    }
    locus Source {
        params { sent: Int = 0; }
        bus { publish "f4" of type Tick; }
        run() { while self.sent < 20000 { "f4" <- Tick { n: self.sent }; self.sent = self.sent + 1; } }
    }
    main locus App {
        params {
            sub: Counter = Counter { };
            a: Source = Source { };
            b: Source = Source { };
            c: Source = Source { };
            d: Source = Source { };
        }
        placement {
            a: cooperative(pool = pa);
            b: cooperative(pool = pb);
            c: cooperative(pool = pc);
            d: cooperative(pool = pd);
        }
        run() {
            let started = std::time::monotonic_ns() + 10000000000;
            while ((self.a.sent == 0 || self.b.sent == 0 || self.c.sent == 0 || self.d.sent == 0) && std::time::monotonic_ns() < started) { }
            let before = self.a.sent + self.b.sent + self.c.sent + self.d.sent;
            let t0 = std::time::monotonic_ns();
            usleep(1000000);
            let blocked_ms = (std::time::monotonic_ns() - t0) / 1000000;
            let after = self.a.sent + self.b.sent + self.c.sent + self.d.sent;
            println("sent_while_blocked=" + to_string(after - before));
            println("blocked_ms=" + to_string(blocked_ms));
            let deadline = std::time::monotonic_ns() + 30000000000;
            while (self.sub.count < 80000 && std::time::monotonic_ns() < deadline) { std::time::sleep(1ms); }
            println("delivered=" + to_string(self.sub.count));
        }
    }
    fn main() { App { }; }
"#;

#[test]
fn the_stall_window_is_the_queues_not_each_producers() {
    let field = run_fields_env("four_producers", FOUR_PRODUCERS_OWNER_IN_FFI, &[("LOTUS_BUS_QUEUE_CAP", "64")]);
    let sent: i64 = field("sent_while_blocked").parse().expect("a count");
    let blocked_ms: i64 = field("blocked_ms").parse().expect("a duration");
    // A cap (64) per 20 ms window over the blocked time, one more for the
    // window running when it began, and one for what was in flight.
    let allowed = (blocked_ms / 20 + 2) * 64;
    assert!(sent <= allowed, "{sent} sent in {blocked_ms} ms while main was blocked; the bound allows {allowed}");
    assert!(sent >= 64, "the sources waited on main's foreign call: {sent} sent in {blocked_ms} ms");
    assert_eq!(field("delivered"), "80000", "every tick is delivered");
}

/// A subscriber born on main while a pool floods its topic: its birth
/// opens the gate the source waits for and spins 50 ms, so the flood parks
/// a full queue's worth for it and waits at the readiness cap. Main's post
/// to its own queue never waits, so readying it in rounds never ended while
/// the source ran: each round posted the list the source had parked during
/// the last, and main's queue took the whole flood (2M sent before
/// `Counter { }` returned, 1.1 GB; under a loaded box the same loop reached
/// 16 GB in a failure-delivery fixture). Readying it now posts the list and
/// closes the window in one hold, and the source then waits at main's full
/// queue, which grows a queue's worth per 20 ms only while main drains
/// nothing (#1552): what the source has sent when main looks is bounded by
/// the time since the birth ended. The source starts at the gate, so
/// nothing it sends predates the subscription (a source flooding before
/// it, with no subscriber, was counted too, and failed the bound on a
/// loaded box).
const READY_ON_MAIN_MID_FLOOD: &str = r#"
    type Tick { n: Int; }
    fn gate() -> String { return "/tmp/hale-bp-main-gate-" + to_string(std::process::pid()); }
    locus Counter {
        params { count: Int = 0; born_end: Int = 0; }
        bus { subscribe "rw" as on_tick of type Tick; }
        birth() {
            std::io::fs::write_file(gate(), "go") or discard;
            let until = std::time::monotonic_ns() + 50000000;
            while (std::time::monotonic_ns() < until) { }
            self.born_end = std::time::monotonic_ns();
        }
        fn on_tick(t: Tick) { self.count = self.count + 1; }
    }
    locus Source {
        params { sent: Int = 0; }
        bus { publish "rw" of type Tick; }
        run() {
            let path = gate();
            let deadline = std::time::monotonic_ns() + 30000000000;
            while (!std::io::fs::file_exists(path) && std::time::monotonic_ns() < deadline) { std::time::sleep(1ms); }
            while self.sent < 2000000 { "rw" <- Tick { n: self.sent }; self.sent = self.sent + 1; }
        }
    }
    main locus App {
        params { src: Source = Source { }; }
        placement { src: cooperative(pool = side); }
        run() {
            let c = Counter { };
            let after = self.src.sent;
            let since_ms = (std::time::monotonic_ns() - c.born_end) / 1000000;
            println("sent_after_birth=" + to_string(after));
            println("ms_since_birth=" + to_string(since_ms));
            let deadline = std::time::monotonic_ns() + 60000000000;
            while (self.src.sent < 2000000 && std::time::monotonic_ns() < deadline) { std::time::sleep(1ms); }
            while (c.count < 2000000 && std::time::monotonic_ns() < deadline) { std::time::sleep(1ms); }
            println("heard=" + to_string(c.count));
            std::io::fs::unlink(gate()) or discard;
        }
    }
    fn main() { App { }; }
"#;

#[test]
fn a_subscriber_readied_on_main_mid_flood_closes_its_window() {
    let field = run_fields("ready_on_main", READY_ON_MAIN_MID_FLOOD);
    let sent: i64 = field("sent_after_birth").parse().expect("a count");
    let since_ms: i64 = field("ms_since_birth").parse().expect("a duration");
    // The parked queue's worth, what the source posts until main's queue
    // is full, one in flight, and a queue's worth per 20 ms main spent
    // after the birth without draining.
    let allowed = (3 + since_ms / 20) * 8192;
    assert!(
        sent <= allowed && sent < 2_000_000,
        "readying the subscriber went on with the flood: {sent} sent {since_ms} ms after the birth (bound {allowed})"
    );
    assert_eq!(field("heard"), "2000000", "every tick reaches the subscriber");
}

/// A subscriber readied on its own pool's worker, or its own pinned thread,
/// while another pool floods it: `Owner`'s run() builds a `Counter`, whose
/// birth opens the gate the source waits for and then spins 50 ms, so the
/// source parks a queue's worth for it and waits at the readiness cap.
/// `Owner` is on pool `p`, or pinned in the variant ([`placed`]).
/// Readiness posted in rounds, each the list parked during the last, and
/// the thread's posts to its own full ring or mailbox never wait (they go
/// to its overflow list), so it returned only when the 1M-cell flood
/// ended: on a pool, with the flood in the worker's overflow list
/// (608 MB); pinned, bounded only because its readiness drained the
/// mailbox between rounds. `Owner` and the source time readiness's return
/// and the flood's end on their own threads, so main's polling lag does
/// not enter the verdict. Every cell reaches the subscriber.
const READY_ON_OWN_THREAD_MID_FLOOD: &str = r#"
    type Tick { n: Int; }
    fn gate() -> String { return "/tmp/hale-bp-own-gate-" + to_string(std::process::pid()); }
    fn stop() -> String { return "/tmp/hale-bp-own-stop-" + to_string(std::process::pid()); }
    locus Counter {
        params { count: Int = 0; }
        bus { subscribe "ow" as on_tick of type Tick; }
        birth() {
            std::io::fs::write_file(gate(), "go") or discard;
            let until = std::time::monotonic_ns() + 50000000;
            while (std::time::monotonic_ns() < until) { }
        }
        fn on_tick(t: Tick) { self.count = self.count + 1; }
    }
    locus Source {
        params { sent: Int = 0; done: Int = 0; done_at: Int = 0; }
        bus { publish "ow" of type Tick; }
        run() {
            let path = gate();
            let deadline = std::time::monotonic_ns() + 30000000000;
            while (!std::io::fs::file_exists(path) && std::time::monotonic_ns() < deadline) { std::time::sleep(1ms); }
            while self.sent < 1000000 { "ow" <- Tick { n: self.sent }; self.sent = self.sent + 1; }
            self.done_at = std::time::monotonic_ns();
            self.done = 1;
        }
    }
    locus Owner {
        params { ready: Int = 0; ready_at: Int = 0; heard: Int = 0; finished: Int = 0; }
        run() {
            let path = stop();
            let c = Counter { };
            self.ready_at = std::time::monotonic_ns();
            self.ready = 1;
            let deadline = std::time::monotonic_ns() + 60000000000;
            while (!std::io::fs::file_exists(path) && std::time::monotonic_ns() < deadline) { std::time::sleep(1ms); }
            self.heard = c.count;
            self.finished = 1;
        }
    }
    main locus App {
        params { own: Owner = Owner { }; src: Source = Source { }; }
        placement { own: cooperative(pool = p); src: cooperative(pool = side); }
        run() {
            let deadline = std::time::monotonic_ns() + 60000000000;
            while (self.src.done == 0 && std::time::monotonic_ns() < deadline) { std::time::sleep(1ms); }
            while (self.own.ready == 0 && std::time::monotonic_ns() < deadline) { std::time::sleep(1ms); }
            let statm = std::io::fs::read_file("/proc/self/statm") or "";
            std::time::sleep(300ms);
            std::io::fs::write_file(stop(), "stop") or discard;
            while (self.own.finished == 0 && std::time::monotonic_ns() < deadline) { std::time::sleep(1ms); }
            if self.own.ready_at < self.src.done_at { println("ready=before-flood-ended"); } else { println("ready=after-flood-ended"); }
            print("rss_statm=");
            println(statm);
            println("sent=" + to_string(self.src.sent));
            println("heard=" + to_string(self.own.heard));
            std::io::fs::unlink(gate()) or discard;
            std::io::fs::unlink(stop()) or discard;
        }
    }
    fn main() { App { }; }
"#;

/// `src` as written (its locus on pool `p`), or with `field` pinned
/// instead: the embedded text stays a whole program the corpus parses.
fn placed(src: &str, field: &str, pinned: bool) -> String {
    let pool = format!("{field}: cooperative(pool = p);");
    assert!(src.contains(&pool), "the program places `{field}` on pool `p`");
    if pinned { src.replace(&pool, &format!("{field}: pinned;")) } else { src.to_string() }
}

fn ready_on_own_thread_mid_flood(name: &str, pinned: bool) {
    let src = placed(READY_ON_OWN_THREAD_MID_FLOOD, "own", pinned);
    let field = run_fields(name, &src);
    let rss = harness::statm_resident_bytes(&field("rss_statm")) / 1048576;
    // Timed on the subscriber's own thread and the source's: main's polling
    // lag (the worker drains on while main looks) does not enter it.
    assert_eq!(field("ready"), "before-flood-ended", "{name}: readying the subscriber went on with the flood");
    assert!(rss < 200, "{name}: RSS {rss} MB once the flood ended (expected < 200)");
    assert_eq!(field("sent"), "1000000", "{name}");
    assert_eq!(field("heard"), "1000000", "{name}: every cell reaches the subscriber");
}

#[test]
fn a_subscriber_readied_on_its_pools_worker_mid_flood_closes_its_window() {
    ready_on_own_thread_mid_flood("ready_on_pool", false);
}

#[test]
fn a_subscriber_readied_on_its_pinned_thread_mid_flood_closes_its_window() {
    ready_on_own_thread_mid_flood("ready_on_pinned", true);
}

/// A producer never waits on a ring whose consumer is not draining, as on
/// main's queue (#1552): main reads `go`, publishes 100000 ticks to a
/// subscriber of the worker on pool `p`, or pinned (more than its ring holds),
/// and only then replies `done`, while the worker blocks reading that
/// reply. A producer at a full ring waited for its consumer to free a slot,
/// so the two stood still; the worker's heartbeat stands still now, so main
/// spills behind the ring and replies.
const RING_CONSUMER_BLOCKED_IN_READ: &str = r#"
    type Tick { n: Int; }
    locus Counter {
        params { count: Int = 0; }
        bus { subscribe "rk" as on_tick of type Tick; }
        fn on_tick(t: Tick) { self.count = self.count + 1; }
    }
    locus Worker {
        params { port: Int = 0; sub: Counter = Counter { }; got: String = ""; }
        run() {
            let fd = std::io::tcp::connect("127.0.0.1", self.port) or raise;
            let s = std::io::tcp::Stream { conn_fd: fd };
            s.send("go") or raise;
            self.got = s.recv(16) or raise;
            let deadline = std::time::monotonic_ns() + 30000000000;
            while (self.sub.count < 100000 && std::time::monotonic_ns() < deadline) { std::time::sleep(1ms); }
        }
    }
    main locus App {
        params { lfd: Int = 0; w: Worker = Worker { }; }
        placement { w: cooperative(pool = p); }
        bus { publish "rk" of type Tick; }
        run() {
            let cfd = std::io::tcp::accept_one(self.lfd) or raise;
            let s = std::io::tcp::Stream { conn_fd: cfd };
            let go = s.recv(16) or raise;
            let mut i = 0;
            while i < 100000 { "rk" <- Tick { n: i }; i = i + 1; }
            s.send("done") or raise;
            println("sent=" + to_string(i));
            let deadline = std::time::monotonic_ns() + 30000000000;
            while (self.w.sub.count < 100000 && std::time::monotonic_ns() < deadline) { std::time::sleep(1ms); }
            println("delivered=" + to_string(self.w.sub.count));
        }
    }
    fn main() {
        let lfd = std::io::tcp::listen_socket("127.0.0.1", 0) or raise;
        let port = std::io::tcp::__local_port(lfd);
        App { lfd: lfd, w: Worker { port: port } };
    }
"#;

fn ring_consumer_blocked_in_read(name: &str, pinned: bool) {
    let field = run_fields(name, &placed(RING_CONSUMER_BLOCKED_IN_READ, "w", pinned));
    assert_eq!(field("sent"), "100000", "{name}");
    assert_eq!(field("delivered"), "100000", "{name}: every tick is delivered");
}

#[test]
fn a_producer_never_waits_on_a_pool_worker_blocked_in_a_read() {
    ring_consumer_blocked_in_read("pool_in_read", false);
}

#[test]
fn a_producer_never_waits_on_a_pinned_thread_blocked_in_a_read() {
    ring_consumer_blocked_in_read("pinned_in_read", true);
}

/// The spill behind a ring grows by a ring's worth per stall window, shared
/// by every producer, and keeps each publisher's order. Four sources on four
/// pools flood a subscriber on pool `p`, or pinned, with 64-cell rings; its
/// 100th handler blocks a second in `usleep`. While it does, the sources
/// spill a ring's worth per 20 ms in all (about 3.1k; with a window per
/// producer, 10.6k), and every tick arrives once it drains, each source's
/// in the order sent. Each tick carries the time it was sent, and the
/// subscriber counts the ticks sent inside its block as they arrive: main
/// counted the sources' sends between seeing the block begin and seeing it
/// end, so its lag after the block took in the ticks sent while the
/// subscriber was draining again (5489 against a bound of 3328, under
/// load).
const RING_SPILL_SHARED_WINDOW: &str = r#"
    @ffi("c") fn usleep(us: Int) -> Int;

    type Tick { src: Int; n: Int; at: Int; }
    locus Counter {
        params {
            count: Int = 0; bad: Int = 0; blocked_ms: Int = 0;
            from: Int = 0; until: Int = 0; during: Int = 0;
            a: Int = -1; b: Int = -1; c: Int = -1; d: Int = -1;
        }
        bus { subscribe "p4" as on_tick of type Tick; }
        fn on_tick(t: Tick) {
            if t.at >= self.from && t.at < self.until { self.during = self.during + 1; }
            let mut last = self.d;
            if t.src == 0 { last = self.a; }
            if t.src == 1 { last = self.b; }
            if t.src == 2 { last = self.c; }
            if t.n != last + 1 { self.bad = self.bad + 1; }
            if t.src == 0 { self.a = t.n; }
            if t.src == 1 { self.b = t.n; }
            if t.src == 2 { self.c = t.n; }
            if t.src == 3 { self.d = t.n; }
            self.count = self.count + 1;
            if self.count == 100 {
                self.from = std::time::monotonic_ns();
                usleep(1000000);
                self.until = std::time::monotonic_ns();
                self.blocked_ms = (self.until - self.from) / 1000000;
            }
        }
    }
    locus Source {
        params { id: Int = 0; sent: Int = 0; }
        bus { publish "p4" of type Tick; }
        run() {
            while self.sent < 20000 {
                "p4" <- Tick { src: self.id, n: self.sent, at: std::time::monotonic_ns() };
                self.sent = self.sent + 1;
            }
        }
    }
    main locus App {
        params {
            sub: Counter = Counter { };
            s0: Source = Source { id: 0 };
            s1: Source = Source { id: 1 };
            s2: Source = Source { id: 2 };
            s3: Source = Source { id: 3 };
        }
        placement {
            sub: cooperative(pool = p);
            s0: cooperative(pool = p0);
            s1: cooperative(pool = p1);
            s2: cooperative(pool = p2);
            s3: cooperative(pool = p3);
        }
        run() {
            let deadline = std::time::monotonic_ns() + 60000000000;
            while (self.sub.count < 80000 && std::time::monotonic_ns() < deadline) { std::time::sleep(1ms); }
            println("sent_while_blocked=" + to_string(self.sub.during));
            println("blocked_ms=" + to_string(self.sub.blocked_ms));
            println("delivered=" + to_string(self.sub.count));
            println("out_of_order=" + to_string(self.sub.bad));
        }
    }
    fn main() { App { }; }
"#;

fn ring_spill_shared_window(name: &str, pinned: bool) {
    let src = placed(RING_SPILL_SHARED_WINDOW, "sub", pinned);
    let field = run_fields_env(name, &src, &[("LOTUS_BUS_QUEUE_CAP", "64")]);
    let sent: i64 = field("sent_while_blocked").parse().expect("a count");
    let blocked_ms: i64 = field("blocked_ms").parse().expect("a duration");
    // A ring (64) per 20 ms window over the blocked time, one more for the
    // window running when it began and one for the ring's free slots then,
    // and a cell per source stamped inside the block but posted after it.
    let allowed = (blocked_ms / 20 + 2) * 64 + 4;
    assert!(sent <= allowed, "{name}: {sent} sent in {blocked_ms} ms while the consumer was blocked; the bound allows {allowed}");
    assert!(sent >= 64, "{name}: the sources waited on the blocked consumer: {sent} sent in {blocked_ms} ms");
    assert_eq!(field("delivered"), "80000", "{name}: every tick is delivered");
    assert_eq!(field("out_of_order"), "0", "{name}: a source's ticks arrive in the order sent");
}

#[test]
fn a_pool_rings_spill_grows_a_ring_per_window_in_order() {
    ring_spill_shared_window("spill_pool", false);
}

#[test]
fn a_pinned_mailboxs_spill_grows_a_ring_per_window_in_order() {
    ring_spill_shared_window("spill_pinned", true);
}

/// Readiness flushing its parked cells into its own FULL ring keeps them
/// ahead of what their publisher posts next (PR #1557's review). Main fills
/// `Owner`'s 64-cell ring with deliveries to `Filler`; `Owner` (on pool
/// `p`, or pinned) then builds a `Counter`, whose birth lets the source
/// start and spins, so the source's first 64 numbers park. Readiness posts
/// them; `Owner` sits 200 ms in a foreign call, so the source's next
/// numbers spill behind the full ring. The parked numbers went to the
/// worker's own overflow list, which it takes after the spill, so later
/// numbers arrived before earlier ones; they go to the spill now, ahead.
const READY_INTO_FULL_RING_KEEPS_ORDER: &str = r#"
    @ffi("c") fn usleep(us: Int) -> Int;

    type Fill { k: Int; }
    type Num { n: Int; }
    fn filled() -> String { return "/tmp/hale-bp-ord-filled-" + to_string(std::process::pid()); }
    fn born() -> String { return "/tmp/hale-bp-ord-born-" + to_string(std::process::pid()); }
    locus Filler {
        params { got: Int = 0; }
        bus { subscribe "of" as on_fill of type Fill; }
        fn on_fill(f: Fill) { self.got = self.got + 1; }
    }
    locus Counter {
        params { count: Int = 0; last: Int = -1; bad: Int = 0; }
        bus { subscribe "on" as on_num of type Num; }
        birth() {
            std::io::fs::write_file(born(), "go") or discard;
            let until = std::time::monotonic_ns() + 50000000;
            while (std::time::monotonic_ns() < until) { }
        }
        fn on_num(m: Num) {
            if m.n != self.last + 1 { self.bad = self.bad + 1; }
            self.last = m.n;
            self.count = self.count + 1;
        }
    }
    locus Source {
        params { sent: Int = 0; }
        bus { publish "on" of type Num; }
        run() {
            let path = born();
            let deadline = std::time::monotonic_ns() + 30000000000;
            while (!std::io::fs::file_exists(path) && std::time::monotonic_ns() < deadline) { std::time::sleep(1ms); }
            while self.sent < 1000 { "on" <- Num { n: self.sent }; self.sent = self.sent + 1; }
        }
    }
    locus Owner {
        params { fill: Filler = Filler { }; count: Int = 0; bad: Int = 0; finished: Int = 0; }
        run() {
            let path = filled();
            let deadline = std::time::monotonic_ns() + 30000000000;
            while (!std::io::fs::file_exists(path) && std::time::monotonic_ns() < deadline) { }
            let c = Counter { };
            usleep(200000);
            while (c.count < 1000 && std::time::monotonic_ns() < deadline) { std::time::sleep(1ms); }
            self.count = c.count;
            self.bad = c.bad;
            self.finished = 1;
        }
    }
    main locus App {
        params { own: Owner = Owner { }; src: Source = Source { }; }
        placement { own: cooperative(pool = p); src: cooperative(pool = side); }
        bus { publish "of" of type Fill; }
        run() {
            let mut i = 0;
            while i < 64 { "of" <- Fill { k: i }; i = i + 1; }
            std::io::fs::write_file(filled(), "go") or discard;
            let deadline = std::time::monotonic_ns() + 60000000000;
            while (self.own.finished == 0 && std::time::monotonic_ns() < deadline) { std::time::sleep(1ms); }
            println("delivered=" + to_string(self.own.count));
            println("out_of_order=" + to_string(self.own.bad));
            std::io::fs::unlink(filled()) or discard;
            std::io::fs::unlink(born()) or discard;
        }
    }
    fn main() { App { }; }
"#;

fn ready_into_full_ring_keeps_order(name: &str, pinned: bool) {
    let src = placed(READY_INTO_FULL_RING_KEEPS_ORDER, "own", pinned);
    let field = run_fields_env(name, &src, &[("LOTUS_BUS_QUEUE_CAP", "64")]);
    assert_eq!(field("delivered"), "1000", "{name}: every number is delivered");
    assert_eq!(field("out_of_order"), "0", "{name}: the numbers arrive in the order sent");
}

#[test]
fn readiness_into_its_pools_full_ring_keeps_order() {
    ready_into_full_ring_keeps_order("ready_full_pool", false);
}

#[test]
fn readiness_into_its_pinned_full_mailbox_keeps_order() {
    ready_into_full_ring_keeps_order("ready_full_pinned", true);
}

/// A failed dequeue does not mean an empty ring (PR #1557's review): a
/// producer claims its slot before it publishes it. Two sources flood a
/// subscriber on pool `p` (or pinned) through a 64-cell ring, and
/// `LOTUS_BUS_TEST_RING_CLAIM_STALL_US` holds the 100th claim unpublished for
/// 100 ms: the consumer cannot take the ring's head, so its heartbeat stops,
/// the other source fills the ring and spills. The consumer took the spill
/// whenever its dequeue failed, so spilled numbers passed their source's
/// earlier ones still in the ring (3 out of order); it waits for the claim
/// to be published now.
const UNPUBLISHED_HEAD_KEEPS_ORDER: &str = r#"
    type Num { src: Int; n: Int; }
    locus Counter {
        params { count: Int = 0; a: Int = -1; b: Int = -1; bad: Int = 0; }
        bus { subscribe "rs" as on_num of type Num; }
        fn on_num(m: Num) {
            let mut last = self.b;
            if m.src == 0 { last = self.a; }
            if m.n != last + 1 { self.bad = self.bad + 1; }
            if m.src == 0 { self.a = m.n; } else { self.b = m.n; }
            self.count = self.count + 1;
        }
    }
    locus Source {
        params { id: Int = 0; sent: Int = 0; }
        bus { publish "rs" of type Num; }
        run() { while self.sent < 5000 { "rs" <- Num { src: self.id, n: self.sent }; self.sent = self.sent + 1; } }
    }
    main locus App {
        params { sub: Counter = Counter { }; s0: Source = Source { id: 0 }; s1: Source = Source { id: 1 }; }
        placement {
            sub: cooperative(pool = p);
            s0: cooperative(pool = pa);
            s1: cooperative(pool = pb);
        }
        run() {
            let deadline = std::time::monotonic_ns() + 60000000000;
            while (self.sub.count < 10000 && std::time::monotonic_ns() < deadline) { std::time::sleep(1ms); }
            println("delivered=" + to_string(self.sub.count));
            println("out_of_order=" + to_string(self.sub.bad));
        }
    }
    fn main() { App { }; }
"#;

fn unpublished_head_keeps_order(name: &str, pinned: bool) {
    let src = placed(UNPUBLISHED_HEAD_KEEPS_ORDER, "sub", pinned);
    let field = run_fields_env(
        name,
        &src,
        &[("LOTUS_BUS_QUEUE_CAP", "64"), ("LOTUS_BUS_TEST_RING_CLAIM_STALL_US", "100000")],
    );
    assert_eq!(field("delivered"), "10000", "{name}: every number is delivered");
    assert_eq!(field("out_of_order"), "0", "{name}: each source's numbers arrive in the order sent");
}

#[test]
fn a_pool_ring_takes_its_spill_only_once_its_head_is_published() {
    unpublished_head_keeps_order("unpublished_head_pool", false);
}

#[test]
fn a_pinned_mailbox_takes_its_spill_only_once_its_head_is_published() {
    unpublished_head_keeps_order("unpublished_head_pinned", true);
}

/// A bound raised while the consumer drained nothing is headroom for that
/// stall only. Four sources flood main's subscriber through a 64-cell
/// queue; its 100th handler blocks 300 ms in `usleep`, so the sources take
/// main for blocked and raise the bound about fifteen times. Then main
/// drains again, a little slower than the sources send (each handler spins
/// 20 us), and samples the backlog (what the sources sent less what it
/// took). The raised bound used to persist until main drained the queue to
/// half the cap, which sources sending faster never let happen: the backlog
/// stayed at the stall's level (about 900). It shrinks with every take now,
/// so the sources wait at what is queued until main is back under the cap.
const HEADROOM_ON_MAIN: &str = r#"
    @ffi("c") fn usleep(us: Int) -> Int;

    type Tick { n: Int; }
    fn stop() -> String { return "/tmp/hale-bp-hr-main-" + to_string(std::process::pid()); }
    locus Source {
        params { sent: Int = 0; done: Int = 0; }
        bus { publish "hm" of type Tick; }
        run() {
            let path = stop();
            let mut going = true;
            while going {
                "hm" <- Tick { n: self.sent };
                self.sent = self.sent + 1;
                if self.sent % 64 == 0 { going = !std::io::fs::file_exists(path); }
            }
            self.done = 1;
        }
    }
    main locus App {
        params {
            count: Int = 0; resumed_at: Int = 0; samples: Int = 0; least: Int = -1; stopped: Int = 0;
            s0: Source = Source { };
            s1: Source = Source { };
            s2: Source = Source { };
            s3: Source = Source { };
        }
        placement {
            s0: cooperative(pool = p0);
            s1: cooperative(pool = p1);
            s2: cooperative(pool = p2);
            s3: cooperative(pool = p3);
        }
        bus { subscribe "hm" as on_tick of type Tick; }
        fn on_tick(t: Tick) {
            self.count = self.count + 1;
            if self.count == 100 {
                usleep(300000);
                self.resumed_at = std::time::monotonic_ns();
            }
            let busy = std::time::monotonic_ns() + 20000;
            while (std::time::monotonic_ns() < busy) { }
            if self.resumed_at > 0 && self.samples < 200 && self.count % 16 == 0
                && std::time::monotonic_ns() - self.resumed_at > 50000000 {
                let backlog = self.s0.sent + self.s1.sent + self.s2.sent + self.s3.sent - self.count;
                if self.least < 0 || backlog < self.least { self.least = backlog; }
                self.samples = self.samples + 1;
            }
            // The sources stop after the 200th sample, or 5 s after the
            // stall whatever was sampled, so a regression fails, never hangs.
            if self.stopped == 0 && self.resumed_at > 0
                && (self.samples >= 200 || std::time::monotonic_ns() - self.resumed_at > 5000000000) {
                std::io::fs::write_file(stop(), "stop") or discard;
                self.stopped = 1;
            }
        }
        run() {
            let deadline = std::time::monotonic_ns() + 60000000000;
            while (self.stopped == 0 && std::time::monotonic_ns() < deadline) { std::time::sleep(1ms); }
            std::io::fs::write_file(stop(), "stop") or discard;
            // The marker stays until every source has seen it: one
            // descheduled past its last check would otherwise find it
            // gone and flood on.
            while ((self.s0.done + self.s1.done + self.s2.done + self.s3.done) < 4 && std::time::monotonic_ns() < deadline) { std::time::sleep(1ms); }
            println("samples=" + to_string(self.samples));
            println("least_backlog=" + to_string(self.least));
            println("sources_done=" + to_string(self.s0.done + self.s1.done + self.s2.done + self.s3.done));
            std::io::fs::unlink(stop()) or discard;
        }
    }
    fn main() { App { }; }
"#;

#[test]
fn a_bound_raised_on_mains_queue_is_headroom_for_the_stall_only() {
    let field = run_fields_env("headroom_main", HEADROOM_ON_MAIN, &[("LOTUS_BUS_QUEUE_CAP", "64")]);
    assert_eq!(field("samples"), "200", "main sampled its backlog 200 times after the stall");
    let least: i64 = field("least_backlog").parse().expect("a count");
    assert!(least <= 3 * 64, "main's backlog stayed at {least} after the stall (expected at most 3 caps of 64)");
    assert_eq!(field("sources_done"), "4", "every source saw the stop marker before main removed it");
}

/// The same for a ring's spill: a subscriber on pool `p` (or pinned) blocks
/// its 100th handler 300 ms while four sources flood it, then drains a
/// little slower than they send, and main samples the backlog. The spill's
/// raised bound persisted the same way (about 900); it shrinks with every
/// take from the spill now. Every tick is delivered.
const HEADROOM_ON_RING: &str = r#"
    @ffi("c") fn usleep(us: Int) -> Int;

    type Tick { n: Int; }
    fn stop() -> String { return "/tmp/hale-bp-hr-ring-" + to_string(std::process::pid()); }
    locus Counter {
        params { count: Int = 0; resumed_at: Int = 0; }
        bus { subscribe "hr" as on_tick of type Tick; }
        fn on_tick(t: Tick) {
            self.count = self.count + 1;
            if self.count == 100 {
                usleep(300000);
                self.resumed_at = std::time::monotonic_ns();
            }
            let busy = std::time::monotonic_ns() + 20000;
            while (std::time::monotonic_ns() < busy) { }
        }
    }
    locus Source {
        params { sent: Int = 0; done: Int = 0; }
        bus { publish "hr" of type Tick; }
        run() {
            let path = stop();
            let mut going = true;
            while going {
                "hr" <- Tick { n: self.sent };
                self.sent = self.sent + 1;
                if self.sent % 64 == 0 { going = !std::io::fs::file_exists(path); }
            }
            self.done = 1;
        }
    }
    main locus App {
        params {
            sub: Counter = Counter { };
            s0: Source = Source { };
            s1: Source = Source { };
            s2: Source = Source { };
            s3: Source = Source { };
        }
        placement {
            sub: cooperative(pool = p);
            s0: cooperative(pool = p0);
            s1: cooperative(pool = p1);
            s2: cooperative(pool = p2);
            s3: cooperative(pool = p3);
        }
        run() {
            let deadline = std::time::monotonic_ns() + 60000000000;
            while (self.sub.resumed_at == 0 && std::time::monotonic_ns() < deadline) { std::time::sleep(1ms); }
            std::time::sleep(50ms);
            let mut least = -1;
            let mut i = 0;
            while i < 200 {
                let got = self.sub.count;
                let backlog = self.s0.sent + self.s1.sent + self.s2.sent + self.s3.sent - got;
                if least < 0 || backlog < least { least = backlog; }
                std::time::sleep(1ms);
                i = i + 1;
            }
            std::io::fs::write_file(stop(), "stop") or discard;
            while ((self.s0.done + self.s1.done + self.s2.done + self.s3.done) < 4 && std::time::monotonic_ns() < deadline) { std::time::sleep(1ms); }
            let total = self.s0.sent + self.s1.sent + self.s2.sent + self.s3.sent;
            while (self.sub.count < total && std::time::monotonic_ns() < deadline) { std::time::sleep(1ms); }
            println("least_backlog=" + to_string(least));
            println("missing=" + to_string(total - self.sub.count));
            std::io::fs::unlink(stop()) or discard;
        }
    }
    fn main() { App { }; }
"#;

fn headroom_on_ring(name: &str, pinned: bool) {
    let src = placed(HEADROOM_ON_RING, "sub", pinned);
    let field = run_fields_env(name, &src, &[("LOTUS_BUS_QUEUE_CAP", "64")]);
    let least: i64 = field("least_backlog").parse().expect("a count");
    assert!(least <= 3 * 64, "{name}: the backlog stayed at {least} after the stall (expected at most 3 caps of 64)");
    assert_eq!(field("missing"), "0", "{name}: every tick is delivered");
}

#[test]
fn a_bound_raised_on_a_pool_ring_is_headroom_for_the_stall_only() {
    headroom_on_ring("headroom_pool", false);
}

#[test]
fn a_bound_raised_on_a_pinned_mailbox_is_headroom_for_the_stall_only() {
    headroom_on_ring("headroom_pinned", true);
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

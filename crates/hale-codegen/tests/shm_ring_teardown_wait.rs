//! What a program's teardown does with shm ring readers and the waits on
//! them: every wait ends at a defined step of the teardown spine, and none
//! on a timer (spec/runtime.md § Lifecycle obligations, line 7).
//!
//! The readers stop at the spine's reader stop, after the wait-abort and
//! the pool join and before the pinned joins and the dissolve cascade: a
//! reader's handlers run on its own thread and touch main's loci, so it
//! stops before they die; and it lives while anything it serves can still
//! produce, so a pool's job is joined against it. A wait that only
//! teardown ends takes the wait-abort's raise, `BusWaitAborted`: an
//! `or wait` a reader's handler is parked in, and an `on_overflow: block`
//! claim on a ring whose reader in this process has stopped. Each program
//! here must end within a deadline, in a plain build and under
//! AddressSanitizer.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

const PROGRAM: &str = r#"
type Tick { n: Int = 0; }
type Note { text: String = ""; }
topic Ticks { payload: Tick; }
topic Notes {
    payload: Note;
    subject: "notes";
    bounded(2);
    on_full: fail;
}
locus Fwd {
    bus {
        subscribe Ticks as on_tick;
        publish Notes;
    }
    fn on_tick(t: Tick) {
        Notes <- Note { text: "tick " + to_string(t.n) } or wait;
    }
}
locus Tally {
    params { seen: Int = 0; }
    bus { subscribe Notes as on_note; }
    fn on_note(n: Note) { self.seen = self.seen + 1; }
    dissolve() { println("notes seen: ", self.seen); }
}
locus Producer {
    bus { publish Ticks; }
    birth() {
        let mut i = 0;
        while i < 20 {
            Ticks <- Tick { n: i };
            i = i + 1;
        }
    }
}
main locus App {
    params {
        tally: Tally = Tally { };
        fwd: Fwd = Fwd { };
    }
    bindings { Ticks: shm_ring("__NAME__", slot_count: 64, on_overflow: drop) where zero_copy; }
    run() {
        Producer { };
        std::time::sleep(20ms);
    }
}
fn main() { App { }; }
"#;

fn shm_name(tag: &str) -> String {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("/hale-teardown-wait-{tag}-{}-{nanos}", std::process::id())
}

struct Ended {
    status: std::process::ExitStatus,
    stdout: String,
    stderr: String,
}

const ABORTED: &str = "Hale panic: unhandled BusWaitAborted";

/// Run `bin`; it must end within the deadline, and AddressSanitizer must
/// have nothing to say.
fn ends(bin: &Path, env: &[(&str, &str)]) -> Ended {
    let mut child = Command::new(bin).envs(env.iter().copied()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().expect("run");
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(s) = child.try_wait().expect("wait") {
            break s;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("the program did not end within 30 s: a wait at teardown that no step of the spine ends");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let out = child.wait_with_output().expect("output");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("AddressSanitizer"), "{stderr}");
    Ended { status, stdout: stdout.into_owned(), stderr: stderr.into_owned() }
}

/// The forwarding program ended: cleanly, with Tally's count, or with the
/// handler's `or wait` aborted at the wait-abort, which comes before the
/// reader stop (line 7: a wait only teardown ends raises).
fn forwarded(e: &Ended) {
    if e.status.success() {
        assert!(e.stdout.contains("notes seen: "), "Tally said nothing:\n{}", e.stdout);
    } else {
        assert_eq!(e.status.code(), Some(1), "{:?}\n{}", e.status, e.stderr);
        assert!(e.stderr.contains(ABORTED), "a failure other than the abort: {}", e.stderr);
    }
}

/// The publisher saw its claim end as an aborted wait.
fn aborted_claim(e: &Ended) {
    assert_eq!(e.status.code(), Some(1), "{:?}\nstdout:\n{}\nstderr:\n{}", e.status, e.stdout, e.stderr);
    assert!(e.stderr.contains("BusWaitAborted (an `on_overflow: block` ring claim"), "{}", e.stderr);
    assert!(!e.stdout.contains("published"), "the publisher went on past an aborted claim: {}", e.stdout);
}

#[test]
fn a_reader_parked_in_or_wait_does_not_hang_the_exit() {
    let bin = harness::unique_bin("shm_teardown_wait");
    let src = PROGRAM.replace("__NAME__", &shm_name("plain"));
    build_opts::build_source(&src, &bin, &build_opts::options()).expect("build");
    forwarded(&ends(&bin, &[]));
}

#[test]
fn a_reader_parked_in_or_wait_ends_clean_under_asan() {
    let bin = harness::unique_bin("shm_teardown_wait_asan");
    let src = PROGRAM.replace("__NAME__", &shm_name("asan"));
    harness::build_source_asan(&src, &bin);
    forwarded(&ends(&bin, &[("LOTUS_NO_CHUNK_POOL", "1")]));
}

/// A finite producer still publishing when main returns, onto a two-slot
/// `on_overflow: block` ring whose one reader is in this process: a job of
/// a classic cooperative pool sleeps 1 s, then publishes ten, while main's
/// run() returns after 20 ms (review finding). Stopping the readers before
/// the pool's jobs are joined stranded the job's third claim (no reader to
/// advance the cursor) and main's join of the pool with it. The reader
/// stop now follows the pool join, so the job finishes against a live
/// reader and everything it published is delivered.
const BLOCK_RING: &str = r#"
type M { n: Int = 0; }
topic Ms { payload: M; }
locus Sink {
    params { got: Int = 0; }
    bus { subscribe Ms as on_m; }
    fn on_m(m: M) { self.got = self.got + 1; }
    dissolve() { println("got ", self.got); }
}
locus Burst {
    params { _u: Int = 0; }
    bus { publish Ms; }
    run() {
        std::time::sleep(1s);
        let mut i = 0;
        while i < 10 {
            Ms <- M { n: i };
            i = i + 1;
        }
        println("published");
    }
}
main locus App {
    params {
        sink: Sink = Sink { };
        burst: Burst = Burst { };
    }
    placement { burst: __PLACE__; }
    bindings { Ms: shm_ring("__NAME__", slot_count: 2, on_overflow: block) where zero_copy; }
    run() { std::time::sleep(20ms); }
}
fn main() { App { }; }
"#;

fn block_ring(tag: &str, place: &str) -> String {
    BLOCK_RING.replace("__PLACE__", place).replace("__NAME__", &shm_name(tag))
}

#[test]
fn a_pool_job_publishing_onto_a_block_ring_at_exit_finishes_and_is_delivered() {
    let bin = harness::unique_bin("shm_teardown_block_pool");
    build_opts::build_source(&block_ring("pool", "cooperative(pool = jobs)"), &bin, &build_opts::options()).expect("build");
    let e = ends(&bin, &[]);
    assert!(e.status.success(), "{:?}\n{}", e.status, e.stderr);
    assert!(e.stdout.contains("published") && e.stdout.contains("got 10"), "every publish of the joined job is delivered: {}", e.stdout);
}

#[test]
fn a_pool_job_publishing_onto_a_block_ring_at_exit_runs_clean_under_asan() {
    let bin = harness::unique_bin("shm_teardown_block_pool_asan");
    harness::build_source_asan(&block_ring("poolasan", "cooperative(pool = jobs)"), &bin);
    let e = ends(&bin, &[("LOTUS_NO_CHUNK_POOL", "1")]);
    assert!(e.status.success(), "{:?}\n{}", e.status, e.stderr);
    assert!(e.stdout.contains("published") && e.stdout.contains("got 10"), "{}", e.stdout);
}

/// The same producer on a pinned thread, which is joined after the reader
/// stop: its third claim waits on a reader that has stopped, which only
/// teardown ends, so it takes the wait-abort's raise, promptly, and the
/// program ends instead of hanging the pinned join.
#[test]
fn a_pinned_producer_blocked_on_a_stopped_reader_sees_the_abort() {
    let bin = harness::unique_bin("shm_teardown_block_pinned");
    build_opts::build_source(&block_ring("pinned", "pinned"), &bin, &build_opts::options()).expect("build");
    aborted_claim(&ends(&bin, &[]));
}

#[test]
fn a_pinned_producer_blocked_on_a_stopped_reader_sees_the_abort_under_asan() {
    let bin = harness::unique_bin("shm_teardown_block_pinned_asan");
    harness::build_source_asan(&block_ring("pinnedasan", "pinned"), &bin);
    aborted_claim(&ends(&bin, &[("LOTUS_NO_CHUNK_POOL", "1")]));
}

/// A main locus built twice, each instance subscribing to a two-slot
/// `on_overflow: block` ring and publishing ten records onto it (review
/// finding): the first instance's teardown stops its reader, and the
/// second instance's fresh reader is live. The publisher handle, registered
/// once in main's prelude, used to keep a stopped-consumer flag from the
/// first teardown, so the second instance's full-ring claim refused and
/// raised BusWaitAborted though its reader would advance the cursor. A
/// claim now refuses only when no local reader of the ring is live: each
/// reader counts itself in at registration and out at its stop.
const TWICE: &str = r#"
type M { n: Int = 0; }
topic Ms { payload: M; }
locus Sink {
    params { got: Int = 0; }
    bus { subscribe Ms as on_m; }
    fn on_m(m: M) { self.got = self.got + 1; }
    dissolve() { println("got ", self.got); }
}
main locus App {
    params { sink: Sink = Sink { }; }
    bus { publish Ms; }
    bindings { Ms: shm_ring("__NAME__", slot_count: 2, on_overflow: block) where zero_copy; }
    run() {
        let mut i = 0;
        while i < 10 {
            Ms <- M { n: i };
            i = i + 1;
        }
        println("published");
    }
}
fn main() { App { }; App { }; }
"#;

/// Both instances published all ten and their readers delivered them (the
/// second instance's fresh reader may also replay the two records the
/// first left in the ring).
fn both_delivered(e: &Ended) {
    assert!(e.status.success(), "{:?}\nstdout:\n{}\nstderr:\n{}", e.status, e.stdout, e.stderr);
    assert_eq!(e.stdout.matches("published").count(), 2, "{}", e.stdout);
    let got: Vec<i64> = e.stdout.lines().filter_map(|l| l.strip_prefix("got ")).filter_map(|n| n.trim().parse().ok()).collect();
    assert_eq!(got.len(), 2, "{}", e.stdout);
    assert!(got.iter().all(|&n| n >= 10), "each instance's reader took all ten: {got:?}");
}

#[test]
fn a_second_main_instance_waits_on_its_own_live_reader() {
    let bin = harness::unique_bin("shm_teardown_twice");
    build_opts::build_source(&TWICE.replace("__NAME__", &shm_name("twice")), &bin, &build_opts::options()).expect("build");
    both_delivered(&ends(&bin, &[]));
}

#[test]
fn a_second_main_instance_waits_on_its_own_live_reader_under_asan() {
    let bin = harness::unique_bin("shm_teardown_twice_asan");
    harness::build_source_asan(&TWICE.replace("__NAME__", &shm_name("twiceasan")), &bin);
    both_delivered(&ends(&bin, &[("LOTUS_NO_CHUNK_POOL", "1")]));
}

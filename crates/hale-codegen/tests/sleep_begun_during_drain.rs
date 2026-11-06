//! A timed wait a watching `run()` begins during a drain returns at once
//! (spec/semantics.md § "Drain cascade (whole-process)", step 2).
//!
//! The drain cut short only a sleep already in progress when it began:
//! `std::time::sleep` snapshotted `draining` at its entry and was cut only
//! when the snapshot was 0. So a `while !self.draining { work();
//! sleep(30s); }` loop that was mid-`work()` when SIGTERM landed began a
//! full 30 s sleep after the drain, outlasted the 5 s grace, and the
//! process died by the signal (143) with its `drain()` hook unrun
//! (reported from hale-lang/voice). Now a sleep begun during the drain in
//! the run() of a locus that reads `draining` returns at once, until that
//! run() has read it as true, on every placement: main, a classic pool,
//! an `async_io` pool (where the sleep is a timed park) and a pinned
//! thread. What did not change is held too: a sleep in progress is still
//! cut, and a wait begun after the run() has seen the drain (a final
//! flush's pacing after its loop) or in a `dissolve()` body runs its full
//! length.

use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[path = "support/build.rs"]
mod build_opts;
#[path = "support/harness.rs"]
mod harness;

struct Ran {
    code: Option<i32>,
    stopped_in: Duration,
    stdout: String,
}

/// Build `src`, wait for its stdout line `mark`, send SIGTERM and collect
/// how it ended and how long after the signal it took.
fn term_at(tag: &str, src: &str, mark: &str) -> Ran {
    let bin = harness::unique_bin(tag);
    build_opts::build_source(src, &bin, &build_opts::options()).expect("build");
    let mut child = Command::new(&bin)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn");
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    let want = mark.to_string();
    let reader = std::thread::spawn(move || {
        let mut r = BufReader::new(stdout);
        let mut seen = String::new();
        let mut line = String::new();
        loop {
            line.clear();
            match r.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    seen.push_str(&line);
                    if line.trim() == want {
                        let _ = tx.send(());
                        break;
                    }
                }
            }
        }
        let mut rest = String::new();
        let _ = r.read_to_string(&mut rest);
        seen + &rest
    });
    rx.recv_timeout(Duration::from_secs(20))
        .unwrap_or_else(|_| panic!("the program prints `{mark}` once it runs"));
    let sent = Instant::now();
    let killed = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .expect("kill");
    assert!(killed.success(), "sent SIGTERM");
    let status = child.wait().expect("wait");
    let stopped_in = sent.elapsed();
    let stdout = reader.join().expect("reader");
    let _ = std::fs::remove_file(&bin);
    Ran { code: status.code(), stopped_in, stdout }
}

/// Busy work the signal lands in: no wait, so nothing for the drain to cut.
const BUSY: &str = r#"
fn busy(ms: Int) {
    let t0 = std::time::monotonic_ns();
    while std::time::monotonic_ns() - t0 < ms * 1000000 { }
}
"#;

/// The reported shape on the child `Worker`, placed by `placement`: SIGTERM
/// lands while its run() is in `busy`, so the sleep after it begins during
/// the drain. It returns at once, the loop sees the drain, the hook runs
/// and the process exits 0, well inside the 5 s grace.
fn assert_worker_stops(tag: &str, placement: &str) {
    let src = format!(
        r#"{BUSY}
locus Worker {{
    params {{ _u: Int = 0; }}
    run() {{
        while !self.draining {{
            println("working");
            busy(600);
            std::time::sleep(30s);
        }}
        println("worker sees drain");
    }}
    drain() {{ println("worker drain hook"); }}
}}
main locus App {{
    params {{ w: Worker = Worker {{ }}; }}
    {placement}
    run() {{ while !self.draining {{ std::time::sleep(10ms); }} }}
}}
fn main() {{ App {{ }}; }}
"#
    );
    let ran = term_at(tag, &src, "working");
    assert_eq!(ran.code, Some(0), "a clean drain, not the signal ({:?}):\n{}", ran.stopped_in, ran.stdout);
    assert!(ran.stopped_in < Duration::from_secs(2), "stopped in {:?}, not after a 30 s sleep:\n{}", ran.stopped_in, ran.stdout);
    assert!(ran.stdout.contains("worker sees drain"), "the loop saw the drain:\n{}", ran.stdout);
    assert!(ran.stdout.contains("worker drain hook"), "the drain() hook ran:\n{}", ran.stdout);
}

#[test]
fn a_sleep_begun_during_the_drain_on_main_returns_at_once() {
    let src = format!(
        r#"{BUSY}
main locus App {{
    params {{ _u: Int = 0; }}
    run() {{
        while !self.draining {{
            println("working");
            busy(600);
            std::time::sleep(30s);
        }}
        println("run sees drain");
    }}
    drain() {{ println("drain hook"); }}
}}
fn main() {{ App {{ }}; }}
"#
    );
    let ran = term_at("sleep_drain_main", &src, "working");
    assert_eq!(ran.code, Some(0), "a clean drain, not the signal ({:?}):\n{}", ran.stopped_in, ran.stdout);
    assert!(ran.stopped_in < Duration::from_secs(2), "stopped in {:?}:\n{}", ran.stopped_in, ran.stdout);
    assert!(ran.stdout.contains("run sees drain"), "{}", ran.stdout);
    assert!(ran.stdout.contains("drain hook"), "the drain() hook ran:\n{}", ran.stdout);
}

#[test]
fn a_sleep_begun_during_the_drain_on_a_classic_pool_returns_at_once() {
    assert_worker_stops("sleep_drain_pool", "placement { w: cooperative(pool = work); }");
}

#[test]
fn a_sleep_begun_during_the_drain_on_an_async_io_pool_returns_at_once() {
    assert_worker_stops("sleep_drain_async", "placement { w: cooperative(pool = work) where async_io; }");
}

#[test]
fn a_sleep_begun_during_the_drain_on_a_pinned_thread_returns_at_once() {
    assert_worker_stops("sleep_drain_pinned", "placement { w: pinned; }");
}

/// What the drain already did: a sleep in progress when it begins returns
/// at its next slice.
#[test]
fn a_sleep_in_progress_is_still_cut() {
    let ran = term_at(
        "sleep_drain_in_progress",
        r#"
main locus App {
    params { _u: Int = 0; }
    run() {
        while !self.draining {
            println("sleeping");
            std::time::sleep(30s);
        }
        println("run sees drain");
    }
    drain() { println("drain hook"); }
}
fn main() { App { }; }
"#,
        "sleeping",
    );
    assert_eq!(ran.code, Some(0), "({:?}):\n{}", ran.stopped_in, ran.stdout);
    assert!(ran.stopped_in < Duration::from_secs(2), "stopped in {:?}:\n{}", ran.stopped_in, ran.stdout);
    assert!(ran.stdout.contains("drain hook"), "{}", ran.stdout);
}

/// A wait begun once the run() has seen the drain is the run()'s own: a
/// final flush paced after the loop, and a `dissolve()` body pacing its
/// own, each run their full length.
#[test]
fn a_wait_after_the_run_has_seen_the_drain_runs_its_length() {
    let ran = term_at(
        "sleep_drain_seen",
        r#"
locus Flusher {
    params { _u: Int = 0; }
    dissolve() {
        let t0 = std::time::monotonic_ns();
        std::time::sleep(200ms);
        if std::time::monotonic_ns() - t0 >= 190000000 { println("hook paced"); } else { println("hook cut"); }
    }
}
main locus App {
    params { f: Flusher = Flusher { }; }
    run() {
        println("up");
        while !self.draining { std::time::sleep(10ms); }
        let t0 = std::time::monotonic_ns();
        std::time::sleep(300ms);
        if std::time::monotonic_ns() - t0 >= 290000000 { println("flush paced"); } else { println("flush cut"); }
    }
}
fn main() { App { }; }
"#,
        "up",
    );
    assert_eq!(ran.code, Some(0), "({:?}):\n{}", ran.stopped_in, ran.stdout);
    assert!(ran.stdout.contains("flush paced"), "the pacing after the loop ran its length:\n{}", ran.stdout);
    assert!(ran.stdout.contains("hook paced"), "the dissolve() body's sleep ran its length:\n{}", ran.stdout);
}

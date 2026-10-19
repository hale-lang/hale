//! When the drain grace expires, the one line the runtime prints says
//! what the drain was still waiting on.
//!
//! A SIGTERM begins a drain; a program that has not exited within the
//! grace (`LOTUS_DRAIN_GRACE_MS`, 5000 ms by default) ends as the
//! signal's default action would. The line it printed then named no one
//! — "a run() that never reads self.draining?" — and a CI run that
//! reached it (the head's stop, GH #1148's neighbour) left nothing to
//! say WHICH run(). It now adds what the runtime knows: each cooperative
//! pool with its mode, whether a worker is mid-iteration and in which
//! locus, and how many cells queue; then the live loci that read
//! `draining`, by name. Nothing else changes: the process still ends
//! the same way, at the same time.

use std::io::{BufRead, BufReader};
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;


#[path = "support/build.rs"]
mod build_opts;
#[path = "support/harness.rs"]
mod harness;

const GRACE_MS: u64 = 400;

struct Ran {
    status: std::process::ExitStatus,
    stderr: String,
}

/// Build `src`, wait for its first stdout line ("up"), send SIGTERM and
/// collect how it ended.
fn term_after_up(tag: &str, src: &str) -> Ran {
    let bin = harness::unique_bin(tag);
    build_opts::build_source(src, &bin, &build_opts::options()).expect("build");
    let mut child = Command::new(&bin)
        .env("LOTUS_DRAIN_GRACE_MS", GRACE_MS.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let _ = BufReader::new(stdout).read_line(&mut line);
        let _ = tx.send(line);
    });
    let line = rx
        .recv_timeout(Duration::from_secs(20))
        .expect("the program prints `up` once it runs");
    assert_eq!(line.trim(), "up");
    let killed = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .expect("kill");
    assert!(killed.success(), "sent SIGTERM");
    let out = child.wait_with_output().expect("wait");
    let _ = std::fs::remove_file(&bin);
    Ran {
        status: out.status,
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// A run() on a cooperative pool that never returns: the pool's worker
/// is mid-iteration in `Stuck` when the grace runs out.
#[test]
fn a_run_blocking_a_pool_worker_past_the_grace_is_named() {
    let ran = term_after_up(
        "drain_grace_pool",
        r#"
locus Stuck {
    params { _u: Int = 0; }
    run() {
        println("up");
        let mut i = 0;
        while i < 100000 { std::time::sleep(20ms); i = i + 1; }
    }
}
main locus Root {
    params { s: Stuck = Stuck { }; }
    placement { s: cooperative(pool = io); }
    run() { while !self.draining { std::time::sleep(10ms); } }
}
fn main() { Root { }; }
"#,
    );
    let e = &ran.stderr;
    assert_eq!(
        ran.status.signal(),
        Some(15),
        "the process still ends as SIGTERM's default action would: {:?}\n{e}",
        ran.status
    );
    assert!(e.contains(&format!("did not finish within {GRACE_MS} ms")), "{e}");
    assert!(
        e.contains("still waiting on: pool `io` (blocking): a worker is mid-iteration, 0 queued in locus `Stuck`"),
        "names the pool and the locus its worker is inside: {e}"
    );
    // `Root` answered: its loop ended on the drain and its run()
    // returned, so it is no longer one the drain waits on.
    assert!(
        e.contains("live loci that read draining: none named"),
        "names no locus that has already answered: {e}"
    );
    assert_eq!(
        e.matches("did not finish within").count(),
        1,
        "one line, not several: {e}"
    );
}

/// A signal that lands while the root's params are still being built
/// finds the root live. A param's run() starts as the params are built
/// (a pool's is posted), so it can say "up" before the root's own birth;
/// the root used to be counted as a drain observer only after its params,
/// and a SIGTERM in that window found nothing that could answer a drain
/// and took the default action at once, with no line (a CI flake of the
/// test above). Here a second param's birth holds the window open for a
/// second: the drain begins, waits out the grace on the stuck pool, and
/// says so.
#[test]
fn a_signal_while_the_params_are_built_finds_the_root_live() {
    let ran = term_after_up(
        "drain_grace_params",
        r#"
locus Stuck {
    params { _u: Int = 0; }
    run() {
        println("up");
        let mut i = 0;
        while i < 100000 { std::time::sleep(20ms); i = i + 1; }
    }
}
locus Slow {
    params { _u: Int = 0; }
    birth() { std::time::sleep(1s); }
}
main locus Root {
    params {
        s: Stuck = Stuck { };
        slow: Slow = Slow { };
    }
    placement { s: cooperative(pool = io); }
    run() { while !self.draining { std::time::sleep(10ms); } }
}
fn main() { Root { }; }
"#,
    );
    let e = &ran.stderr;
    assert_eq!(ran.status.signal(), Some(15), "{:?}\n{e}", ran.status);
    assert!(
        e.contains(&format!("did not finish within {GRACE_MS} ms")),
        "the drain began and waited out its grace; it did not end at once as if nothing could answer it: {e}"
    );
}

/// The same wait on a pinned thread, which is on no pool: the report says
/// so and still names the locus that reads `draining` and has not ended.
#[test]
fn a_pinned_locus_that_outlasts_the_grace_is_named() {
    let ran = term_after_up(
        "drain_grace_pinned",
        r#"
locus Napper {
    params { _u: Int = 0; }
    run() {
        println("up");
        // a wait a drain cannot cut short (a `sleep` is a timed park, which
        // the drain expires): a subprocess the loop is inside of. The
        // subprocess runs BEFORE `draining` is first tested, so a SIGTERM
        // that lands between `up` and here still finds this locus inside
        // a 3 s wait, past the grace, and never ends the loop early.
        let mut going = true;
        while going {
            let r = std::process::run("sleep\n3")
                or std::process::ProcessOutput { code: -1, signal: 0, stdout: "", stderr: "" };
            going = !self.draining;
        }
    }
}
main locus Root {
    params { n: Napper = Napper { }; }
    placement { n: pinned; }
    run() { while !self.draining { std::time::sleep(10ms); } }
}
fn main() { Root { }; }
"#,
    );
    let e = &ran.stderr;
    assert_eq!(ran.status.signal(), Some(15), "{:?}\n{e}", ran.status);
    assert!(
        e.contains("still waiting on: no cooperative pool; live loci that read draining:"),
        "{e}"
    );
    assert!(e.contains("`Napper` x1"), "names the pinned locus: {e}");
}

/// The control: a drain that finishes inside the grace says nothing.
#[test]
fn a_drain_that_finishes_prints_no_report() {
    let src = r#"
main locus Root {
    params { _u: Int = 0; }
    run() { println("up"); while !self.draining { std::time::sleep(10ms); } }
}
fn main() { Root { }; }
"#;
    let bin = harness::unique_bin("drain_grace_quiet");
    build_opts::build_source(src, &bin, &build_opts::options()).expect("build");
    let mut child = Command::new(&bin)
        .env("LOTUS_DRAIN_GRACE_MS", GRACE_MS.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    let mut first = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut first)
        .expect("up");
    Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .expect("kill");
    let out = child.wait_with_output().expect("wait");
    let _ = std::fs::remove_file(&bin);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{:?}\n{stderr}", out.status);
    assert!(!stderr.contains("did not finish"), "{stderr}");
}

/// Build `src`, wait for its "up" line, send SIGTERM, and return how it
/// ended and how long after the signal, under the default grace.
fn term_after_up_timed(tag: &str, src: &str) -> (Ran, Duration) {
    let bin = harness::unique_bin(tag);
    build_opts::build_source(src, &bin, &build_opts::options()).expect("build");
    let mut child = Command::new(&bin)
        .env("LOTUS_DRAIN_GRACE_MS", "5000")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    while line.trim() != "up" {
        line.clear();
        assert!(out.read_line(&mut line).expect("read") > 0, "the program prints `up` once it runs");
    }
    let sent = std::time::Instant::now();
    Command::new("kill").args(["-TERM", &child.id().to_string()]).status().expect("kill");
    let out = child.wait_with_output().expect("wait");
    let took = sent.elapsed();
    let _ = std::fs::remove_file(&bin);
    (Ran { status: out.status, stderr: String::from_utf8_lossy(&out.stderr).into_owned() }, took)
}

/// A finished locus has nothing to drain. Pinned loci that read
/// `draining` in their run() alone, whose run() has returned, held a
/// SIGTERM's drain for the whole grace (5 s): each was counted as a live
/// drain observer until its parent tore it down, which a parent that
/// never ends did not do. Counted out once their run() returns, they
/// leave nothing that could answer the drain, so the signal takes its
/// default action at once.
#[test]
fn pinned_loci_whose_run_returned_do_not_hold_the_drain() {
    let (ran, took) = term_after_up_timed(
        "drain_finished_pinned",
        r#"
locus Worker {
    params { n: Int = 0; }
    run() {
        if self.draining { return; }
        println("worker done");
    }
}
main locus Root {
    params { a: Worker = Worker { n: 1 }; b: Worker = Worker { n: 2 }; }
    placement { a: pinned(core = 0); b: pinned(core = 1); }
    run() {
        std::time::sleep(300ms);
        println("up");
        while true { std::time::sleep(50ms); }
    }
}
fn main() { Root { }; }
"#,
    );
    let e = &ran.stderr;
    assert!(took < Duration::from_millis(2000), "ended {took:?} after the signal, not after the 5 s grace: {e}");
    assert_eq!(ran.status.signal(), Some(15), "nothing left could answer the drain: {:?}\n{e}", ran.status);
    assert!(!e.contains("did not finish"), "{e}");
}

/// The other side of the rule above: a locus whose `draining` read is
/// in a method its parent calls still answers a drain after its run()
/// has returned, so it stays counted, and the parent's loop ends the
/// drain the ordinary way.
#[test]
fn a_finished_locus_read_through_a_method_still_answers_the_drain() {
    let (ran, took) = term_after_up_timed(
        "drain_finished_method",
        r#"
locus Gate {
    params { n: Int = 0; }
    fn stopping() -> Bool { return self.draining; }
    run() { println("gate done"); }
}
main locus Root {
    params { g: Gate = Gate { }; }
    run() {
        println("up");
        while !self.g.stopping() { std::time::sleep(20ms); }
        println("drained");
    }
}
fn main() { Root { }; }
"#,
    );
    let e = &ran.stderr;
    assert!(ran.status.success(), "the parent's loop answered the drain: {:?}\n{e}", ran.status);
    assert!(took < Duration::from_millis(2000), "{took:?}: {e}");
}

/// The drain observer table, driven through its test surface
/// (`lotus_drain_observer_step`: 0 enter, 1 finished, 2 teardown, 4 how
/// many the table holds). An instance leaves the count exactly once,
/// whichever of its two ends comes first. Teardown's latch can come
/// before its run's end, since an admitted run outlives it under its run hold, and
/// `leave` then `finished` used to count the instance out twice, taking
/// a second, live observer with it: 2 → 1 → 0, and a signal took the
/// default action at once instead of giving that observer its grace.
/// Growth and removal run over 200 instances, every entry gone at the
/// end; under ASan too.
const OBSERVER_STEPS: &str = r#"
@ffi("c") fn lotus_drain_observer_step(op: Int, instance: Int) -> Int;
locus Never {
    params { n: Int = 0; }
    fn stopping() -> Bool { return self.draining; }
}
fn main() {
    // teardown first, then the run's end: one count out
    lotus_drain_observer_step(0, 4096);
    lotus_drain_observer_step(0, 8192);
    let left = lotus_drain_observer_step(2, 4096);
    let finished = lotus_drain_observer_step(1, 4096);
    // the run's end first, then teardown: one count out
    lotus_drain_observer_step(0, 12288);
    let run_end = lotus_drain_observer_step(1, 12288);
    let again = lotus_drain_observer_step(1, 12288);
    let torn = lotus_drain_observer_step(2, 12288);
    println("orders left=", left, " finished=", finished, " run_end=", run_end, " again=", again, " torn=", torn);
    // many instances, every other one finished first, then all torn down
    let mut i = 1;
    while i <= 200 { lotus_drain_observer_step(0, 65536 + i * 64); i = i + 1; }
    let held = lotus_drain_observer_step(4, 0);
    i = 1;
    while i <= 200 { if i % 2 == 0 { lotus_drain_observer_step(1, 65536 + i * 64); } i = i + 1; }
    i = 1;
    while i <= 200 { lotus_drain_observer_step(2, 65536 + i * 64); i = i + 1; }
    println("many held=", held, " live=", lotus_drain_observer_step(3, 0), " tracked=", lotus_drain_observer_step(4, 0));
    println("up");
    // 8192 is still live: a signal now waits out the grace
    while true { std::time::sleep(20ms); }
}
"#;

#[test]
fn a_drain_observer_is_counted_out_once_in_either_order() {
    for asan in [false, true] {
        let bin = harness::unique_bin(if asan { "drain_observer_steps_asan" } else { "drain_observer_steps" });
        let options = hale_codegen::BuildOptions { asan, ..build_opts::options() };
        build_opts::build_source(OBSERVER_STEPS, &bin, &options).expect("build");
        let mut child = Command::new(&bin)
            .env("LOTUS_DRAIN_GRACE_MS", GRACE_MS.to_string())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn");
        let mut out = BufReader::new(child.stdout.take().unwrap());
        let mut lines = Vec::new();
        loop {
            let mut line = String::new();
            assert!(out.read_line(&mut line).expect("read") > 0, "asan={asan}: the program prints `up`: {lines:?}");
            if line.trim() == "up" {
                break;
            }
            lines.push(line.trim().to_string());
        }
        let sent = std::time::Instant::now();
        Command::new("kill").args(["-TERM", &child.id().to_string()]).status().expect("kill");
        let done = child.wait_with_output().expect("wait");
        let took = sent.elapsed();
        let _ = std::fs::remove_file(&bin);
        let stderr = String::from_utf8_lossy(&done.stderr).into_owned();
        assert_eq!(
            lines,
            ["orders left=1 finished=1 run_end=1 again=1 torn=1", "many held=201 live=1 tracked=1"],
            "asan={asan}: {stderr}"
        );
        assert!(
            stderr.contains(&format!("did not finish within {GRACE_MS} ms")),
            "asan={asan}: the live observer kept the drain's grace: {stderr}"
        );
        assert!(took >= Duration::from_millis(GRACE_MS - 50), "asan={asan}: {took:?}");
        assert_eq!(done.status.signal(), Some(15), "asan={asan}: {:?}\n{stderr}", done.status);
        assert!(!stderr.contains("AddressSanitizer"), "{stderr}");
    }
}

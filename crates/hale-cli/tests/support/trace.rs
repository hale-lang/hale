//! Where a DNA integration test's wall time goes.
//!
//! These tests spend their minutes in three places: `hale` invocations
//! that run to completion, processes they spawn and leave running, and
//! loops that poll for a condition (a socket appearing, a row landing).
//! This module times all three and prints a per-test summary, so a slow
//! test can be explained instead of guessed at.
//!
//! It is silent unless `HALE_DNA_TEST_TRACE=1`, so it costs nothing in
//! CI: one relaxed `bool` read per event.
//!
//! ```text
//! HALE_DNA_TEST_TRACE=1 cargo test --release -p hale-cli \
//!     --test dna_design -- --nocapture
//! ```
//!
//! Every line is prefixed `[dnatrace]`, so a run's events can be summed
//! on their own:
//!
//! ```text
//! grep '^\[dnatrace\] ' out.log | awk -F'\t' '{s[$2]+=$3; n[$2]++} END {for (k in s) printf "%8.1fs %4d  %s\n", s[k], n[k], k}' | sort -rn
//! ```
#![allow(dead_code)]

use std::cell::RefCell;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

pub fn on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("HALE_DNA_TEST_TRACE").is_ok_and(|v| v != "0" && !v.is_empty()))
}

struct Event {
    kind: &'static str,
    label: String,
    took: Duration,
}

thread_local! {
    static EVENTS: RefCell<Vec<Event>> = const { RefCell::new(Vec::new()) };
}

/// Record one finished interval. `kind` buckets it in the summary
/// (`hale`, `spawn`, `wait`, `sleep`, `git`, …); `label` names it.
pub fn record(kind: &'static str, label: impl Into<String>, took: Duration) {
    if !on() {
        return;
    }
    let label = label.into();
    eprintln!("[dnatrace] {kind}\t{:.3}\t{label}", took.as_secs_f64());
    EVENTS.with(|e| e.borrow_mut().push(Event { kind, label, took }));
}

/// An interval that ends when the span is dropped, so
/// `let _s = Span::new("hale", args);` times the rest of the scope.
pub struct Span {
    kind: &'static str,
    label: Option<String>,
    t0: Instant,
}

impl Span {
    pub fn new(kind: &'static str, label: impl Into<String>) -> Span {
        Span { kind, label: Some(label.into()), t0: Instant::now() }
    }

    /// Ends it here rather than at the end of the scope.
    pub fn end(self) -> Duration {
        self.t0.elapsed()
    }
}

impl Drop for Span {
    fn drop(&mut self) {
        if let Some(label) = self.label.take() {
            record(self.kind, label, self.t0.elapsed());
        }
    }
}

/// Time `f` and record it.
pub fn timed<T>(kind: &'static str, label: impl Into<String>, f: impl FnOnce() -> T) -> T {
    let s = Span::new(kind, label);
    let out = f();
    s.end();
    out
}

/// Poll `cond` until it holds or `within` elapses, and record how long
/// the wait actually took.
///
/// The gap between polls starts at 20ms and grows by half each time up
/// to `poll_cap`: a condition that lands quickly — a socket binding, a
/// row a running organism is about to append — is seen within tens of
/// milliseconds instead of at the next fixed 250-500ms edge, while a
/// condition that takes a minute is still only asked for a few times a
/// second. Every one of these tests waits for a *positive* condition,
/// so a tighter first poll cannot change an outcome, only when it is
/// noticed.
pub fn wait_until(label: impl Into<String>, within: Duration, poll_cap: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let label = label.into();
    let t0 = Instant::now();
    let dl = t0 + within;
    let mut gap = Duration::from_millis(20).min(poll_cap);
    loop {
        if cond() {
            record("wait", label, t0.elapsed());
            return true;
        }
        let now = Instant::now();
        if now >= dl {
            record("wait", format!("{label} [TIMED OUT]"), t0.elapsed());
            return false;
        }
        std::thread::sleep(gap.min(dl - now));
        gap = (gap + gap / 2).min(poll_cap);
    }
}

/// A fixed sleep, named so the summary shows what it bought.
pub fn sleep(label: impl Into<String>, d: Duration) {
    std::thread::sleep(d);
    record("sleep", label, d);
}

/// Prints the per-kind and per-label breakdown when the test ends.
/// `let _t = trace::test("name");` at the top of a test is enough.
pub struct TestTrace {
    name: &'static str,
    t0: Instant,
}

pub fn test(name: &'static str) -> TestTrace {
    TestTrace { name, t0: Instant::now() }
}

impl Drop for TestTrace {
    fn drop(&mut self) {
        if !on() {
            return;
        }
        let wall = self.t0.elapsed();
        EVENTS.with(|e| {
            let events = e.borrow();
            let mut kinds: Vec<(&'static str, Duration, usize)> = Vec::new();
            for ev in events.iter() {
                match kinds.iter_mut().find(|k| k.0 == ev.kind) {
                    Some(k) => {
                        k.1 += ev.took;
                        k.2 += 1;
                    }
                    None => kinds.push((ev.kind, ev.took, 1)),
                }
            }
            kinds.sort_by_key(|k| std::cmp::Reverse(k.1));
            let mut labels: Vec<(String, Duration, usize)> = Vec::new();
            for ev in events.iter() {
                let key = format!("{} {}", ev.kind, ev.label);
                match labels.iter_mut().find(|l| l.0 == key) {
                    Some(l) => {
                        l.1 += ev.took;
                        l.2 += 1;
                    }
                    None => labels.push((key, ev.took, 1)),
                }
            }
            labels.sort_by_key(|l| std::cmp::Reverse(l.1));
            eprintln!("[dnatrace] ---- {} : {:.1}s wall ----", self.name, wall.as_secs_f64());
            for (kind, took, n) in &kinds {
                eprintln!("[dnatrace] ---- {:>7.1}s  {:>3}x  {}", took.as_secs_f64(), n, kind);
            }
            for (label, took, n) in labels.iter().take(15) {
                eprintln!("[dnatrace] ----   {:>7.1}s  {:>3}x  {}", took.as_secs_f64(), n, label);
            }
        });
    }
}

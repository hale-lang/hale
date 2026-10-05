//! A mixed ownership edge (the placement correspondence's U-1, F.40
//! phase 3, P1 3 of 6).
//!
//! One enclosing locus, `Worker`, with two instances in two domains: a
//! root field on main, and a field of `Owner`, which the root places
//! `pinned`. Each `Worker`'s handler writes `I { n: self.k };`, and `I`
//! bubbles to the root, which accepts it. The ownership graph's edge is
//! `Mixed`: the owner is resolved (the root, a singleton on main) for
//! both instances, and the mechanism is chosen per instance at the
//! literal, the same-tower bubble on main and the cross-pool post off
//! it. A transient birth would drop the owner, and is never the
//! fallback.
//!
//! Both instances are held to the same lifetime: the root accepts the
//! child, its child count rises by exactly one per construction, the
//! child is retained after the handler that wrote it has returned, it
//! is born on the root's thread (its `birth()` records `pthread_self()`),
//! and the root's teardown dissolves each child exactly once. Counts are
//! read only after the message the child's own `birth()` sends, never
//! after a sleep window: every wait is a condition loop with a 10 s
//! deadline that prints `TIMEOUT` and exits non-zero.
//!
//! **The control.** Under `BuildOptions::no_ownership_bubble` the plan
//! is empty and both children are transient: the root accepts neither,
//! and the pinned instance's child is born on the pinned thread. The
//! test asserts that outcome, so the main arm's assertions are shown to
//! be able to fail. Each arm runs with devirtualization on and off, and
//! once under ASan (chunk recycling off).
//!
//! `Go` is published by a `Relay`, not by the root: the root has one
//! `Worker` field, and the intra-locus rewrite would turn a root
//! publish of `Go` into a direct call to that field alone, dropping the
//! nested instance's delivery (a type-level read in the desugar, found
//! here and left to the desugar's switch to the table).
//!
//! A child released early is not exercised: a child born by a
//! cross-pool post may declare no subscription and no non-empty `run()`
//! (`crosspool_child_shape_ok`), which are the two ways a resident or a
//! flow child ends before its owner does.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use hale_codegen::BuildOptions;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

const MIXED: &str = r#"
@ffi("c") fn pthread_self() -> Int;

type GoP { k: Int; }
type BornP { n: Int; tid: Int; }
topic Go { payload: GoP; }
topic Born { payload: BornP; }
topic Kick { payload: GoP; }

locus I {
    params { n: Int = 0; }
    bus { publish Born; }
    birth() { Born <- BornP { n: self.n, tid: pthread_self() }; }
    dissolve() { println("DISSOLVE n=" + to_string(self.n)); }
}

locus Worker {
    params { k: Int = 0; }
    bus { subscribe Go as on_go; }
    fn on_go(g: GoP) {
        if g.k == self.k {
            I { n: self.k };
        }
    }
}

locus Owner {
    params { w: Worker = Worker { k: 2 }; }
}

locus Relay {
    bus { subscribe Kick as on_kick; publish Go; }
    fn on_kick(g: GoP) { Go <- GoP { k: g.k }; }
}

main locus App {
    params { r: Relay = Relay { }; a: Worker = Worker { k: 1 }; o: Owner = Owner { }; born: Int = 0; }
    placement { o: pinned; }
    accept(c: I) { }
    bus { publish Kick; subscribe Born as on_born; }
    fn on_born(b: BornP) {
        println("BORN n=" + to_string(b.n) + " tid=" + to_string(b.tid));
        self.born = self.born + 1;
    }
    mode harmonic() -> Int {
        let mut n: Int = 0;
        for child in self.children { n = n + 1; }
        return n;
    }
    run() {
        println("MAIN tid=" + to_string(pthread_self()));
        let deadline = std::time::monotonic_ns() + 10000000000;
        Kick <- GoP { k: 1 };
        while self.born < 1 && std::time::monotonic_ns() < deadline { std::time::sleep(1ms); }
        if self.born < 1 { println("TIMEOUT born=" + to_string(self.born)); std::process::exit(3); }
        println("CHILDREN after=1 n=" + to_string(self.harmonic()));
        Kick <- GoP { k: 2 };
        while self.born < 2 && std::time::monotonic_ns() < deadline { std::time::sleep(1ms); }
        if self.born < 2 { println("TIMEOUT born=" + to_string(self.born)); std::process::exit(4); }
        println("CHILDREN after=2 n=" + to_string(self.harmonic()));
        println("DONE");
    }
}

fn main() { App { }; }
"#;

/// What one run printed, and how it ended.
struct Run {
    stdout: String,
    stderr: String,
    status: Option<i32>,
}

fn run(opts: &BuildOptions, tag: &str) -> Run {
    let bin = harness::unique_bin(&format!("ownership_bubble_mixed_{tag}"));
    build_opts::build_source(MIXED, &bin, opts).unwrap_or_else(|e| panic!("build: {e:?}"));
    // The ASan arm checks memory safety, not leaks: a child born by a
    // cross-pool post leaks its arena record at teardown (216 bytes, in
    // the post's dispatcher, `__xpool_dispatch_<I>_<A>`), with or without
    // a mixed edge: `ownership_bubble_crosspool.rs`'s own program does
    // the same under ASan. That leak is the cross-pool path's, not this
    // edge's.
    let mut child = Command::new(&bin)
        .env("ASAN_OPTIONS", "detect_leaks=0")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    let mut out = child.stdout.take().expect("stdout");
    let mut err = child.stderr.take().expect("stderr");
    let reader = std::thread::spawn(move || {
        use std::io::Read;
        let mut s = String::new();
        let _ = out.read_to_string(&mut s);
        s
    });
    let err_reader = std::thread::spawn(move || {
        use std::io::Read;
        let mut s = String::new();
        let _ = err.read_to_string(&mut s);
        s
    });
    // The program's own deadline is 10 s; this one catches a hang in
    // teardown, which the program cannot report itself.
    let limit = Instant::now() + Duration::from_secs(60);
    let status = loop {
        if let Some(s) = child.try_wait().expect("wait") {
            break s.code();
        }
        if Instant::now() > limit {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let _ = std::fs::remove_file(&bin);
    Run { stdout: reader.join().unwrap_or_default(), stderr: err_reader.join().unwrap_or_default(), status }
}

fn field(line: &str, key: &str) -> Option<i64> {
    line.split_whitespace()
        .find_map(|w| w.strip_prefix(key).and_then(|v| v.strip_prefix('=')))
        .and_then(|v| v.parse().ok())
}

fn lines<'a>(r: &'a Run, prefix: &'a str) -> impl Iterator<Item = &'a str> {
    r.stdout.lines().filter(move |l| l.starts_with(prefix))
}

/// The owner's thread, each child's birth thread by `n`, the child
/// counts sampled after each birth, and the dissolves by `n`.
fn observed(r: &Run) -> (Option<i64>, Vec<(i64, i64)>, Vec<i64>, Vec<i64>) {
    let main = lines(r, "MAIN ").next().and_then(|l| field(l, "tid"));
    let born = lines(r, "BORN ").filter_map(|l| Some((field(l, "n")?, field(l, "tid")?))).collect();
    let counts = lines(r, "CHILDREN ").filter_map(|l| field(l, "n")).collect();
    let mut dissolved: Vec<i64> = lines(r, "DISSOLVE ").filter_map(|l| field(l, "n")).collect();
    dissolved.sort();
    (main, born, counts, dissolved)
}

fn check_owned(r: &Run, arm: &str) {
    assert!(!r.stderr.contains("AddressSanitizer"), "{arm}: ASan reported:\n{}", r.stderr);
    assert_eq!(r.status, Some(0), "{arm}: the run did not finish\n{}\n{}", r.stdout, r.stderr);
    assert!(r.stdout.lines().any(|l| l == "DONE"), "{arm}: no DONE\n{}", r.stdout);
    let (main, born, counts, dissolved) = observed(r);
    let main = main.expect("the owner's thread");
    assert_eq!(
        born,
        vec![(1, main), (2, main)],
        "{arm}: each child is born once, on the owner's thread, the pinned instance's by the cross-pool post\n{}",
        r.stdout
    );
    assert_eq!(
        counts,
        vec![1, 2],
        "{arm}: the owner accepts each child, one per construction, and keeps the first after its handler \
         returned\n{}",
        r.stdout
    );
    assert_eq!(dissolved, vec![1, 2], "{arm}: the owner's teardown dissolves each child exactly once\n{}", r.stdout);
}

#[test]
fn a_mixed_edge_keeps_its_owner_for_both_instances() {
    for (tag, no_bus_devirt) in [("devirt", false), ("nodevirt", true)] {
        let r = run(&BuildOptions { no_bus_devirt, ..build_opts::options() }, tag);
        check_owned(&r, tag);
    }
}

#[test]
fn a_mixed_edge_keeps_its_owner_under_asan() {
    let r = run(&BuildOptions { asan: true, ..build_opts::options() }, "asan");
    check_owned(&r, "asan");
}

/// The differential control: with the bubble off, both children are
/// transient. The owner accepts neither, and the pinned instance's
/// child is born on the pinned thread, not the owner's.
#[test]
fn without_the_bubble_both_children_are_transient() {
    for (tag, no_bus_devirt) in [("control_devirt", false), ("control_nodevirt", true)] {
        let r = run(&BuildOptions { no_ownership_bubble: true, no_bus_devirt, ..build_opts::options() }, tag);
        assert_eq!(r.status, Some(0), "{tag}: the run did not finish\n{}\n{}", r.stdout, r.stderr);
        let (main, born, counts, dissolved) = observed(&r);
        let main = main.expect("the owner's thread");
        assert_eq!(counts, vec![0, 0], "{tag}: the owner accepts no transient child\n{}", r.stdout);
        assert_eq!(born.iter().map(|(n, _)| *n).collect::<Vec<_>>(), vec![1, 2], "{tag}\n{}", r.stdout);
        assert_eq!(born[0].1, main, "{tag}: the main instance's child is born on main\n{}", r.stdout);
        assert_ne!(born[1].1, main, "{tag}: the pinned instance's child is born on its thread\n{}", r.stdout);
        assert_eq!(dissolved, vec![1, 2], "{tag}: each transient child dissolves once\n{}", r.stdout);
    }
}

/// U-1's located refusal: where an instance of the enclosing locus runs
/// off the owner's thread its birth is a fire-and-forget post, so a
/// value use of the literal cannot be lowered, and is refused at the
/// literal with both instances named.
#[test]
fn a_value_use_at_a_mixed_site_is_refused_at_the_literal() {
    let src = MIXED.replace("            I { n: self.k };", "            let c = I { n: self.k };");
    let bin = harness::unique_bin("ownership_bubble_mixed_value_use");
    let err = build_opts::build_source(&src, &bin, &build_opts::options())
        .expect_err("a value use at a mixed site must not build");
    let _ = std::fs::remove_file(&bin);
    let hale_codegen::CodegenError::UnsupportedAt(msg, span) = &err else {
        panic!("the refusal is located: {err:?}");
    };
    let at = src.find("I { n: self.k }").expect("the literal");
    assert_eq!(span.start.0 as usize, at, "the refusal points at the literal: {msg}");
    assert_eq!(
        msg,
        "`I { }` is born to `App`, but `Worker` runs both on `App`'s thread and off it (App.a on main; \
         App.o.w on the pinned thread of App.o; the owner App on main), so its birth is a same-tower \
         allocation in some instances and a cross-pool post in others; a cross-pool birth is \
         fire-and-forget, so the literal cannot be used as a value. Write it as a bare statement \
         (`I { ... };`).",
    );
}

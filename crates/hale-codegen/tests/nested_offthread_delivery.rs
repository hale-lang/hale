//! The receiving thread of a nested off-thread subscriber, measured
//! (F.40 phase 3, P1 3 of 6; the placement correspondence's § 4).
//!
//! A locus nested under a root field placed off main runs on that
//! field's thread: its methods, and so its bus handlers, run where its
//! owner runs (spec/semantics.md § Placement block, nested instances
//! inherit; spec/types.md, the pool belongs to the instance). The legacy
//! placement labels (the bus graph's, until it read the placement table,
//! and the ownership graph's copy) read only the root's `placement { }`
//! entries, by the written type of the field, so a nested locus was
//! labelled `SameThread` (row B-1), and so was an adapter in `bindings
//! { }`, whose instance runs on a thread of its own (row B-7). This file
//! measures what a handler actually sees.
//!
//! **The witness is the thread id, recorded inside the handler.** A
//! deliver record is no witness: the runtime records a delivery at post
//! time, on the publisher's thread. So every receiving handler calls
//! `pthread_self()` itself, as
//! `bus_adapter_inbound::adapter_subscription_runs_on_the_adapters_own_thread`
//! does, and reports it by message.
//!
//! **The fixture is synchronized throughout.** The anchor of each case
//! reports `Ready { tid }` from its own thread (its `run()`; an adapter,
//! built before the root, answers a `Hello` instead). The collector, a
//! root field declared first so no report is lost to birth order,
//! gathers it, and only then does the root publish `Go`, which starts
//! the five `Tick`s. Every receiving handler reports
//! `Seen { who, seq, tid }`, and the receiver answers an `AskCount` from
//! its own domain with its count and its thread, by message: no field is
//! read across threads. The collector's handlers print, so its own
//! subjects are never a direct call and are drained on main. Every wait
//! is a loop on a condition with a 10 s deadline on
//! `std::time::monotonic_ns`; on expiry the program prints `TIMEOUT`
//! with what it waited for and exits non-zero.
//!
//! **Two variants.** A handler that calls `pthread_self()` is not quiet,
//! so it never reaches the direct-call gate. The witnessing variant pins
//! the receiving domain on the deferred route. The quiet variant
//! (`self.got = self.got + 1; self.last = t.seq;`) reaches the gate; its
//! count goes through the same `AskCount` round trip, polled until 5 or
//! the deadline, and its thread is pinned at plan level: its subject's
//! flavor is not `static_direct`, the only lowering that runs a handler
//! on the publisher's thread.
//!
//! **Both arms.** Each run is built with devirtualization on and with
//! `BuildOptions::no_bus_devirt`, and held to the same counts and the
//! same receiving domain in each.
//!
//! **A negative control** proves the oracle can fail: a bound adapter's
//! own subscription runs on the adapter's thread (GH #1032), and the
//! oracle, asked to find it on main, must report the mismatch.
//!
//! The registration route that puts a nested handler on its anchor's
//! thread (the correspondence's U-6) is pinned at IR level, and its
//! lifetime by a teardown under ASan.
//!
//! The outcomes measured today that contradict the spec are listed in
//! [`KNOWN_OPEN`] and [`KNOWN_OPEN_FLAVORS`], each asserted to FAIL in
//! exactly the way it fails today; when a fix closes one, its run goes
//! green, the assertion fails, and the entry has to go.

use std::collections::BTreeMap;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use hale_codegen::{build_executable_with_options, BuildOptions};
use hale_frontend::snapshot::{Config, Snapshot, Target};

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

/// The topics and payloads every case shares.
const HEADER: &str = r#"
@ffi("c") fn pthread_self() -> Int;

type TickP { seq: Int; }
type ReadyP { who: Int; tid: Int; }
type SeenP { who: Int; seq: Int; tid: Int; }
type GoP { n: Int; }
type AskP { who: Int; }
type CountP { who: Int; n: Int; tid: Int; }

topic Tick { payload: TickP; }
topic Ready { payload: ReadyP; }
topic Seen { payload: SeenP; }
topic Go { payload: GoP; }
topic AskCount { payload: AskP; }
topic Counted { payload: CountP; }
"#;

/// The collector, a root field declared FIRST: its subscriptions are
/// registered before any later field's thread can publish, so no report
/// is lost to birth order. Each handler prints, so none of its subjects
/// is a direct call, and all of them are drained on main.
const COLLECTOR: &str = r#"
locus Collector {
    params { ready: Int = 0; seen: Int = 0; counted: Int = 0; count: Int = 0; }
    bus { subscribe Ready as on_ready; subscribe Seen as on_seen; subscribe Counted as on_counted; }
    fn on_ready(r: ReadyP) {
        println("READY who=" + to_string(r.who) + " tid=" + to_string(r.tid));
        self.ready = self.ready + 1;
    }
    fn on_seen(s: SeenP) {
        println("SEEN who=" + to_string(s.who) + " seq=" + to_string(s.seq) + " tid=" + to_string(s.tid));
        self.seen = self.seen + 1;
    }
    fn on_counted(c: CountP) {
        println("COUNT who=" + to_string(c.who) + " n=" + to_string(c.n) + " tid=" + to_string(c.tid));
        self.counted = self.counted + 1;
        self.count = c.n;
    }
}
"#;

/// The receiver that witnesses its thread in every handler.
const RECV_WITNESS: &str = r#"
locus Recv {
    params { got: Int = 0; }
    bus { subscribe Tick as on_tick; subscribe AskCount as on_ask; publish Seen; publish Counted; }
    fn on_tick(t: TickP) {
        self.got = self.got + 1;
        Seen <- SeenP { who: 1, seq: t.seq, tid: pthread_self() };
    }
    fn on_ask(a: AskP) { Counted <- CountP { who: 1, n: self.got, tid: pthread_self() }; }
}
"#;

/// The receiver whose `Tick` handler is quiet, so it reaches the gate.
const RECV_QUIET: &str = r#"
locus Recv {
    params { got: Int = 0; last: Int = 0; }
    bus { subscribe Tick as on_tick; subscribe AskCount as on_ask; publish Counted; }
    fn on_tick(t: TickP) { self.got = self.got + 1; self.last = t.seq; }
    fn on_ask(a: AskP) { Counted <- CountP { who: 1, n: self.got, tid: pthread_self() }; }
}
"#;

/// The root's `run()`: the handshake, the five ticks, the reports and
/// the count round trip, each wait bounded by one 10 s deadline.
/// `@HELLO@` asks an anchor that cannot report unprompted for its
/// `Ready`; `@SEEN@` is how many `Seen` reports the case expects.
const DRIVER: &str = r#"
    run() {
        println("MAIN tid=" + to_string(pthread_self()));
        let deadline = std::time::monotonic_ns() + 10000000000;
        @HELLO@
        while self.c.ready < 1 && std::time::monotonic_ns() < deadline { std::time::sleep(1ms); }
        if self.c.ready < 1 { println("TIMEOUT ready saw=" + to_string(self.c.ready)); std::process::exit(3); }
        Go <- GoP { n: 5 };
        while self.c.seen < @SEEN@ && std::time::monotonic_ns() < deadline { std::time::sleep(1ms); }
        if self.c.seen < @SEEN@ { println("TIMEOUT seen saw=" + to_string(self.c.seen)); std::process::exit(4); }
        let mut asked = 0;
        while self.c.count != 5 && std::time::monotonic_ns() < deadline {
            if self.c.counted == asked {
                AskCount <- AskP { who: 1 };
                asked = asked + 1;
            }
            std::time::sleep(1ms);
        }
        if self.c.count != 5 { println("TIMEOUT count saw=" + to_string(self.c.count)); std::process::exit(5); }
        println("DONE");
    }
"#;

/// The shapes § 4 names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Case {
    /// A pinned owner nests the subscriber; a root sibling on main
    /// publishes.
    A,
    /// A pool owner (`cooperative(pool = io)`) nests the subscriber; a
    /// root sibling on main publishes.
    B,
    /// A pinned owner nests the publisher; the subscriber is a root
    /// field on main.
    C,
    /// An adapter in the root's `bindings { }` publishes; the subscriber
    /// is a root field on main (rows B-7, O-7). The adapter is built in
    /// the bindings prelude, before the root, so it reports its `Ready`
    /// when asked (`Hello`), from its own subscription, which runs on
    /// its own thread (GH #1032).
    Adapter,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Variant {
    Witness,
    Quiet,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Arm {
    Devirt,
    NoDevirt,
}

/// The domain a receiver must run in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Expect {
    /// The anchor that reported `Ready` (who = 1).
    Anchor,
    /// The root's thread.
    Main,
}

fn expected(case: Case) -> Expect {
    match case {
        Case::A | Case::B => Expect::Anchor,
        Case::C | Case::Adapter => Expect::Main,
    }
}

fn program(case: Case, variant: Variant) -> String {
    let recv = match variant {
        Variant::Witness => RECV_WITNESS,
        Variant::Quiet => RECV_QUIET,
    };
    // The adapter's negative control adds one report of its own.
    let seen = match (case, variant) {
        (Case::Adapter, Variant::Witness) => 6,
        (Case::Adapter, Variant::Quiet) => 1,
        (_, Variant::Witness) => 5,
        (_, Variant::Quiet) => 0,
    };
    let hello = if case == Case::Adapter { "Hello <- AskP { who: 1 };" } else { "" };
    let driver = DRIVER.replace("@SEEN@", &seen.to_string()).replace("@HELLO@", hello);
    let shape = match case {
        Case::A | Case::B => {
            let placement = if case == Case::A { "pinned" } else { "cooperative(pool = io)" };
            format!(
                r#"
locus Feeder {{
    bus {{ subscribe Go as on_go; publish Tick; }}
    fn on_go(g: GoP) {{
        let mut i = 1;
        while i <= g.n {{ Tick <- TickP {{ seq: i }}; i = i + 1; }}
    }}
}}

locus Owner {{
    params {{ r: Recv = Recv {{ }}; }}
    bus {{ publish Ready; }}
    run() {{ Ready <- ReadyP {{ who: 1, tid: pthread_self() }}; }}
}}

main locus App {{
    params {{ c: Collector = Collector {{ }}; f: Feeder = Feeder {{ }}; o: Owner = Owner {{ }}; }}
    placement {{ o: {placement}; }}
    bus {{ publish Go; publish AskCount; }}
{driver}
}}
"#
            )
        }
        Case::C => format!(
            r#"
locus Pub {{
    bus {{ publish Tick; }}
    fn fire(n: Int) {{
        let mut i = 1;
        while i <= n {{ Tick <- TickP {{ seq: i }}; i = i + 1; }}
    }}
}}

locus Owner {{
    params {{ p: Pub = Pub {{ }}; }}
    bus {{ subscribe Go as on_go; publish Ready; }}
    run() {{ Ready <- ReadyP {{ who: 1, tid: pthread_self() }}; }}
    fn on_go(g: GoP) {{ self.p.fire(g.n); }}
}}

main locus App {{
    params {{ c: Collector = Collector {{ }}; s: Recv = Recv {{ }}; o: Owner = Owner {{ }}; }}
    placement {{ o: pinned; }}
    bus {{ publish Go; publish AskCount; }}
{driver}
}}
"#
        ),
        Case::Adapter => format!(
            r#"
type Wire {{ n: Int; }}
topic Beat {{ payload: Wire; subject: "beat"; }}
topic Hello {{ payload: AskP; }}

locus Probe {{
    bus {{ subscribe Hello as on_hello; subscribe Go as on_go; publish Tick; publish Ready; publish Seen; }}
    fn send(subject: String, bytes: Bytes) {{ }}
    fn on_hello(h: AskP) {{ Ready <- ReadyP {{ who: 1, tid: pthread_self() }}; }}
    fn on_go(g: GoP) {{
        Seen <- SeenP {{ who: 9, seq: 0, tid: pthread_self() }};
        let mut i = 1;
        while i <= g.n {{ Tick <- TickP {{ seq: i }}; i = i + 1; }}
    }}
}}

main locus App {{
    params {{ c: Collector = Collector {{ }}; s: Recv = Recv {{ }}; }}
    bindings {{ Beat: Probe {{ }}; }}
    bus {{ publish Beat; publish Hello; publish Go; publish AskCount; }}
{driver}
}}
"#
        ),
    };
    format!("{HEADER}\n{COLLECTOR}\n{recv}\n{shape}\nfn main() {{ App {{ }}; }}\n")
}

/// What one run printed, parsed.
#[derive(Debug, Default)]
struct Observed {
    main: Option<i64>,
    ready: BTreeMap<i64, i64>,
    seen: Vec<(i64, i64, i64)>,
    counts: Vec<(i64, i64, i64)>,
    done: bool,
    timeout: Option<String>,
    status: Option<i32>,
}

fn field(line: &str, key: &str) -> Option<i64> {
    line.split_whitespace()
        .find_map(|w| w.strip_prefix(key).and_then(|v| v.strip_prefix('=')))
        .and_then(|v| v.parse().ok())
}

fn parse(stdout: &str, status: Option<i32>) -> Observed {
    let mut o = Observed { status, ..Observed::default() };
    for line in stdout.lines() {
        if line.starts_with("MAIN ") {
            o.main = field(line, "tid");
        } else if line.starts_with("READY ") {
            if let (Some(w), Some(t)) = (field(line, "who"), field(line, "tid")) {
                o.ready.insert(w, t);
            }
        } else if line.starts_with("SEEN ") {
            if let (Some(w), Some(s), Some(t)) = (field(line, "who"), field(line, "seq"), field(line, "tid")) {
                o.seen.push((w, s, t));
            }
        } else if line.starts_with("COUNT ") {
            if let (Some(w), Some(n), Some(t)) = (field(line, "who"), field(line, "n"), field(line, "tid")) {
                o.counts.push((w, n, t));
            }
        } else if line == "DONE" {
            o.done = true;
        } else if line.starts_with("TIMEOUT") {
            o.timeout = Some(line.to_string());
        }
    }
    o
}

/// Build `case`/`variant` in `arm`, run it under a deadline, and parse
/// what it printed.
fn run(case: Case, variant: Variant, arm: Arm) -> Observed {
    let src = program(case, variant);
    let program = hale_syntax::parse_source(&src).unwrap_or_else(|e| panic!("parse: {e:?}\n{src}"));
    let bin = harness::unique_bin(&format!("nested_offthread_{case:?}_{variant:?}_{arm:?}").to_lowercase());
    let opts = BuildOptions { no_bus_devirt: arm == Arm::NoDevirt, ..build_opts::options() };
    build_executable_with_options(&program, &bin, &[], &opts).unwrap_or_else(|e| panic!("build: {e:?}"));
    let mut child = Command::new(&bin).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().expect("spawn");
    // Drained while the program runs: a run that polls its count until
    // the deadline prints more than a pipe holds, and would block on it.
    let mut out = child.stdout.take().expect("piped stdout");
    let reader = std::thread::spawn(move || {
        use std::io::Read;
        let mut s = String::new();
        let _ = out.read_to_string(&mut s);
        s
    });
    // The program's own deadline is 10 s; this one only catches a hang in
    // teardown, which the program cannot report itself.
    let limit = Instant::now() + Duration::from_secs(30);
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
    let stdout = reader.join().unwrap_or_default();
    let _ = std::fs::remove_file(&bin);
    parse(&stdout, status)
}

/// The thread a report came from, named relative to the run's anchor
/// and main.
fn where_ran(o: &Observed, tid: i64) -> &'static str {
    if Some(tid) == o.main {
        "main"
    } else if Some(&tid) == o.ready.get(&1) {
        "the anchor"
    } else {
        "another thread"
    }
}

fn expected_tid(o: &Observed, expect: Expect) -> Option<i64> {
    match expect {
        Expect::Anchor => o.ready.get(&1).copied(),
        Expect::Main => o.main,
    }
}

fn expect_name(expect: Expect) -> &'static str {
    match expect {
        Expect::Anchor => "the anchor",
        Expect::Main => "main",
    }
}

/// The oracle: the run finished, every `seq` in `1..=5` reached the
/// receiver exactly once, and every report the receiver sent (each
/// `Seen`, and its answer to `AskCount`) came from the expected domain,
/// with the count 5. The first departure is the verdict.
fn judge(o: &Observed, variant: Variant, expect: Expect) -> Result<(), String> {
    if o.status != Some(0) || !o.done {
        return Err(format!("the run did not finish: exit {:?}, {:?}", o.status, o.timeout));
    }
    let Some(want) = expected_tid(o, expect) else {
        return Err("the run never reported its expected thread".into());
    };
    if o.ready.get(&1) == o.main.as_ref() {
        return Err("the anchor reported main's thread: the case does not separate the domains".into());
    }
    if variant == Variant::Witness {
        let mine: Vec<&(i64, i64, i64)> = o.seen.iter().filter(|(w, _, _)| *w == 1).collect();
        for seq in 1..=5 {
            let n = mine.iter().filter(|(_, s, _)| *s == seq).count();
            if n != 1 {
                return Err(format!("seq {seq} was delivered {n} times"));
            }
        }
        if mine.len() != 5 {
            return Err(format!("{} deliveries, expected 5", mine.len()));
        }
        if let Some((_, seq, tid)) = mine.iter().find(|(_, _, t)| *t != want) {
            return Err(format!(
                "the receiver ran on {}, expected {} (seq {seq})",
                where_ran(o, *tid),
                expect_name(expect)
            ));
        }
    }
    let Some(&(_, n, tid)) = o.counts.iter().filter(|(w, _, _)| *w == 1).last() else {
        return Err("no count was answered".into());
    };
    if n != 5 {
        return Err(format!("the receiver counted {n}, expected 5"));
    }
    if tid != want {
        return Err(format!("the count was answered on {}, expected {}", where_ran(o, tid), expect_name(expect)));
    }
    Ok(())
}

/// The flavor the lowering view's dispatch plan gives the quiet
/// receiver's `Tick` subject.
fn quiet_flavor(case: Case) -> &'static str {
    let src = program(case, Variant::Quiet);
    let program = hale_syntax::parse_source(&src).expect("parse");
    let Ok(snap) = Snapshot::from_program(program, Vec::new(), Config::harness(Target::host())) else {
        panic!("the program does not load");
    };
    let view = snap.demand_lowering().unwrap_or_else(|b| panic!("lowering blocked: {:?}", b.refused));
    let row = view
        .plan
        .subjects
        .iter()
        .find(|s| s.subscribers.iter().any(|(l, h)| l == "Recv" && h == "on_tick"))
        .expect("the quiet receiver's subject has a plan row");
    row.flavor.as_str()
}

/// What each run fails with today, against the spec: nothing. A nested
/// subscriber registers with its anchor's route (U-6), so every
/// deferred delivery runs on the anchor's thread, and the bus graph's
/// labels read the placement table (B-1, B-7), so no subject with an
/// off-main end is a direct call. A run that regresses fails by name;
/// an outcome found open again is listed here, asserted to fail exactly
/// as it does.
const KNOWN_OPEN: &[(Case, Variant, Arm, &str)] = &[];

/// The quiet receivers' plan flavor where it is still the direct call:
/// none. Measured on the parent of the route correction, all four were
/// `static_direct` (the legacy label called the nested receiver of A and
/// B, the nested publisher of C and the adapter `SameThread`); reading
/// the table, all four are `static_bucket`.
const KNOWN_OPEN_FLAVORS: &[(Case, &str)] = &[];

fn check_case(case: Case) {
    let mut failures = Vec::new();
    for variant in [Variant::Witness, Variant::Quiet] {
        for arm in [Arm::Devirt, Arm::NoDevirt] {
            let o = run(case, variant, arm);
            let verdict = judge(&o, variant, expected(case));
            eprintln!("MEASURE {case:?} {variant:?} {arm:?}: {verdict:?} {o:?}");
            let open = KNOWN_OPEN.iter().find(|(c, v, a, _)| (*c, *v, *a) == (case, variant, arm));
            match (open, verdict) {
                (None, Ok(())) => {}
                (None, Err(e)) => failures.push(format!("{case:?} {variant:?} {arm:?}: {e}")),
                (Some((.., known)), Err(e)) if e == *known => {}
                (Some((.., known)), Err(e)) => failures.push(format!(
                    "{case:?} {variant:?} {arm:?}: known open as `{known}`, but failed with `{e}`"
                )),
                (Some(_), Ok(())) => failures.push(format!(
                    "{case:?} {variant:?} {arm:?} now passes: remove its KNOWN_OPEN entry"
                )),
            }
        }
    }
    let flavor = quiet_flavor(case);
    eprintln!("MEASURE {case:?} quiet flavor: {flavor}");
    match (KNOWN_OPEN_FLAVORS.iter().find(|(c, _)| *c == case), flavor) {
        (None, "static_direct") => failures.push(format!(
            "{case:?}: the quiet receiver's subject is a direct call, which runs it on the publisher's thread"
        )),
        (None, _) => {}
        (Some((_, known)), f) if f == *known => {}
        (Some(_), f) => failures.push(format!(
            "{case:?}: the quiet receiver's flavor is now `{f}`: remove its KNOWN_OPEN_FLAVORS entry"
        )),
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn a_nested_subscriber_under_a_pinned_owner_receives_on_the_owners_thread() {
    check_case(Case::A);
}

#[test]
fn a_nested_subscriber_under_a_pool_owner_receives_on_the_pools_worker() {
    check_case(Case::B);
}

#[test]
fn a_nested_publisher_under_a_pinned_owner_is_received_on_main() {
    check_case(Case::C);
}

#[test]
fn an_adapters_publication_is_received_on_main() {
    check_case(Case::Adapter);
}

/// The negative control: the adapter's own `Go` handler runs on the
/// adapter's thread (GH #1032), and the oracle, told to expect main,
/// reports the mismatch by name.
#[test]
fn the_oracle_reports_a_handler_on_the_wrong_thread() {
    let o = run(Case::Adapter, Variant::Witness, Arm::Devirt);
    let control: Vec<i64> = o.seen.iter().filter(|(w, _, _)| *w == 9).map(|(_, _, t)| *t).collect();
    assert_eq!(control.len(), 1, "the control reported once: {o:?}");
    let as_run = Observed {
        seen: (1..=5).map(|seq| (1, seq, control[0])).collect(),
        counts: vec![(1, 5, control[0])],
        main: o.main,
        ready: o.ready.clone(),
        done: o.done,
        timeout: o.timeout.clone(),
        status: o.status,
    };
    assert_eq!(
        judge(&as_run, Variant::Witness, Expect::Main),
        Err("the receiver ran on the anchor, expected main (seq 1)".to_string()),
        "a handler on the adapter's thread must fail an expectation of main: {o:?}"
    );
}

/// The route's lifetime (U-6): a pinned anchor whose only subscriber is
/// nested, torn down while cells for it are still queued. Each handler
/// sleeps, so the root's fifty publications are still in the anchor's
/// mailbox when the root's `run()` returns. The join shuts the mailbox
/// down, the anchor's thread drains every queued cell (the count is 50)
/// and dissolves the tree, and only then is the mailbox destroyed; the
/// root's `dissolve()` publishes once more after that, to a subscriber
/// that is gone. Run under ASan, which turns chunk recycling off, so a
/// cell posted to a freed mailbox or a handler run on a freed arena is a
/// report, not a silent read. Both arms.
const ROUTE_TEARDOWN: &str = r#"
type TickP { seq: Int; }
topic Tick { payload: TickP; }

locus Kid {
    params { got: Int = 0; tag: String = "k"; }
    bus { subscribe Tick as on_tick; }
    fn on_tick(t: TickP) { std::time::sleep(1ms); self.got = self.got + 1; self.tag = self.tag + "."; }
    dissolve() { println("KID got=" + to_string(self.got) + " tag=" + self.tag); }
}

locus Owner {
    params { k: Kid = Kid { }; }
}

main locus App {
    params { o: Owner = Owner { }; }
    placement { o: pinned; }
    bus { publish Tick; }
    run() {
        let mut i = 1;
        while i <= 50 { Tick <- TickP { seq: i }; i = i + 1; }
    }
    dissolve() {
        Tick <- TickP { seq: 0 };
        println("APP dissolved");
    }
}

fn main() { App { }; }
"#;

#[test]
fn a_nested_subscribers_route_outlives_its_queued_cells() {
    let program = hale_syntax::parse_source(ROUTE_TEARDOWN).expect("parse");
    for arm in [Arm::Devirt, Arm::NoDevirt] {
        let bin = harness::unique_bin(&format!("nested_offthread_teardown_{arm:?}").to_lowercase());
        let opts = BuildOptions { asan: true, no_bus_devirt: arm == Arm::NoDevirt, ..build_opts::options() };
        build_executable_with_options(&program, &bin, &[], &opts).unwrap_or_else(|e| panic!("build: {e:?}"));
        let out = Command::new("timeout").arg("60").arg(&bin).output().expect("run");
        let _ = std::fs::remove_file(&bin);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(!stderr.contains("AddressSanitizer"), "{arm:?}: ASan reported:\n{stderr}");
        assert_eq!(out.status.code(), Some(0), "{arm:?}: exit\n{stdout}\n{stderr}");
        assert_eq!(
            stdout.lines().filter(|l| l.starts_with("KID ") || l.starts_with("APP ")).collect::<Vec<_>>(),
            [format!("KID got=50 tag=k{}", ".".repeat(50)).as_str(), "APP dissolved"],
            "{arm:?}: every queued cell is handled before the tree dissolves, and the root outlives it\n{stderr}"
        );
    }
}

/// The IR of `case`'s witnessing program.
fn ir_of(case: Case) -> String {
    let src = program(case, Variant::Witness);
    let program = hale_syntax::parse_source(&src).expect("parse");
    let bin = harness::unique_bin(&format!("nested_offthread_ir_{case:?}").to_lowercase());
    let ll = bin.with_extension("ll");
    let opts = BuildOptions { dump_ir: Some(ll.clone()), ..build_opts::options() };
    build_executable_with_options(&program, &bin, &[], &opts).unwrap_or_else(|e| panic!("build: {e:?}"));
    let ir = std::fs::read_to_string(&ll).expect("read IR");
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(&ll);
    ir
}

/// The line of `ir` registering `handler`, and its position.
fn registration(ir: &str, handler: &str) -> (usize, String) {
    let needle = format!("ptr @{handler}, ");
    ir.lines()
        .enumerate()
        .find(|(_, l)| l.contains("@lotus_bus_register") && l.contains(&needle))
        .map(|(i, l)| (i, l.to_string()))
        .unwrap_or_else(|| panic!("no registration of `{handler}`"))
}

/// The registration route's IR, pinned (U-6). Under a pinned anchor the
/// anchor's mailbox is created before its params are initialized, and
/// the nested receiver's registrations carry it; the join retires the
/// registrations routed to it before destroying it. Under a pool anchor
/// the nested receiver's registrations carry the pool, looked up by
/// name, instead of the registering thread's (none, on main). A program
/// with no route anchor declares no retire.
#[test]
fn a_nested_registration_carries_its_anchors_route() {
    let a = ir_of(Case::A);
    let created = a
        .lines()
        .position(|l| l.contains("%Owner.mailbox.create = call ptr @lotus_mailbox_create()"))
        .expect("the pinned anchor creates its mailbox");
    for handler in ["__hwrap_Recv_on_tick", "__hwrap_Recv_on_ask"] {
        let (at, line) = registration(&a, handler);
        assert!(at > created, "`{handler}` registers after the anchor's mailbox exists:\n{line}");
        assert!(line.contains("ptr %Owner.mailbox.create, "), "`{handler}` routes to the anchor's mailbox:\n{line}");
    }
    assert!(
        a.contains("call void @lotus_bus_retire_mailbox(ptr %mailbox.destroy.load)"),
        "the anchor's join retires the registrations routed to its mailbox"
    );

    let b = ir_of(Case::B);
    for handler in ["__hwrap_Recv_on_tick", "__hwrap_Recv_on_ask"] {
        let (_, line) = registration(&b, handler);
        assert!(
            line.contains("ptr null, ") && line.contains("ptr %coop_pool.lookup"),
            "`{handler}` routes to the anchor's pool, by name:\n{line}"
        );
    }

    let c = ir_of(Case::C);
    assert!(!c.contains("lotus_bus_retire_mailbox"), "no route anchor, no retire");
}

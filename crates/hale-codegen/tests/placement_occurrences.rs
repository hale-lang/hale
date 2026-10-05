//! Domains are templates (F.40 phase 3, P1 5 of 6; the placement
//! correspondence's § 9, checkpoint 1).
//!
//! A `DomainId` belongs to a template, as the key that anchors it does:
//! one construction literal of the root, one pinned field, one domain in
//! the table. A pinned domain anchored by a construction template has one
//! anchor per live occurrence of that template, and so one thread per
//! live occurrence. Nothing maps a `DomainId` to one physical thread id
//! that occurrences share.
//!
//! The program builds its root at one site, `make`'s literal, and its
//! one field is placed `pinned` and nests a subscriber. A pinned anchor
//! is joined when the scope that built it exits, and a pinned locus in
//! a loop is refused, so two occurrences of one site are live together
//! only when the site's scope is entered again before it exits: `make`
//! recurses once, and the inner call drives while both occurrences
//! stand. The table has one template and one pinned domain; at runtime
//! each occurrence's anchor reports `Ready { who, tid }` from its own
//! thread, every `Seen` a nested subscriber reports carries its own
//! occurrence's anchor's thread, never the other's, and the two
//! anchors' threads differ, neither of them main. The witness is the
//! thread id recorded inside the handler (`nested_offthread_delivery.rs`),
//! each wait a loop on a condition with a 10 s deadline.
//!
//! The budget multiplies the template's anchors by its bound: two
//! threads for a site a fn reaches twice (`AtMost(2)`), and for the
//! recursive site no static bound, an uncertainty with its reason.

use std::collections::BTreeMap;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use hale_codegen::BuildOptions;
use hale_frontend::snapshot::{Config, Snapshot};
use hale_types::placement::{Bound, DomainKind, Origin};

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

const SRC: &str = r#"
@ffi("c") fn pthread_self() -> Int;

type TickP { seq: Int; }
type ReadyP { who: Int; tid: Int; }
type SeenP { who: Int; seq: Int; tid: Int; }
type AskP { who: Int; }
type CountP { who: Int; n: Int; tid: Int; }

topic Tick { payload: TickP; }
topic Ready { payload: ReadyP; }
topic Seen { payload: SeenP; }
topic Hello { payload: AskP; }
topic AskCount { payload: AskP; }
topic Counted { payload: CountP; }

locus Collector {
    params { ready: Int = 0; seen: Int = 0; counted: Int = 0; fives: Int = 0; }
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
        if c.n == 5 { self.fives = self.fives + 1; }
    }
}

locus Recv {
    params { id: Int = 0; got: Int = 0; }
    bus { subscribe Tick as on_tick; subscribe AskCount as on_ask; publish Seen; publish Counted; }
    fn on_tick(t: TickP) {
        self.got = self.got + 1;
        Seen <- SeenP { who: self.id, seq: t.seq, tid: pthread_self() };
    }
    fn on_ask(a: AskP) { Counted <- CountP { who: self.id, n: self.got, tid: pthread_self() }; }
}

locus Owner {
    params { id: Int = 0; r: Recv = Recv { }; }
    bus { subscribe Hello as on_hello; publish Ready; }
    fn on_hello(h: AskP) { Ready <- ReadyP { who: self.id, tid: pthread_self() }; }
}

main locus App {
    params { id: Int = 0; o: Owner = Owner { }; }
    placement { o: pinned; }
    run() { make(self.id + 1); }
}

fn make(n: Int) {
    if n > 2 {
        Driver { };
        return;
    }
    App { id: n, o: Owner { id: n, r: Recv { id: n } } };
}

locus Driver {
    params { c: Collector = Collector { }; }
    bus { publish Hello; publish Tick; publish AskCount; }
    run() {
        println("MAIN tid=" + to_string(pthread_self()));
        let deadline = std::time::monotonic_ns() + 10000000000;
        Hello <- AskP { who: 0 };
        while self.c.ready < 2 && std::time::monotonic_ns() < deadline { std::time::sleep(1ms); }
        if self.c.ready < 2 { println("TIMEOUT ready saw=" + to_string(self.c.ready)); std::process::exit(3); }
        let mut i = 1;
        while i <= 5 { Tick <- TickP { seq: i }; i = i + 1; }
        while self.c.seen < 10 && std::time::monotonic_ns() < deadline { std::time::sleep(1ms); }
        if self.c.seen < 10 { println("TIMEOUT seen saw=" + to_string(self.c.seen)); std::process::exit(4); }
        AskCount <- AskP { who: 0 };
        while self.c.counted < 2 && std::time::monotonic_ns() < deadline { std::time::sleep(1ms); }
        if self.c.fives < 2 { println("TIMEOUT count saw=" + to_string(self.c.fives)); std::process::exit(5); }
        println("DONE");
    }
}

fn main() {
    make(1);
}
"#;

/// The same root built at one site by a fn called twice: two
/// occurrences, `AtMost(2)`, which run one after the other (each
/// anchor is joined when `make` returns).
const TWICE: &str = r#"
locus Worker {
    run() { }
}

main locus App {
    params { o: Worker = Worker { }; }
    placement { o: pinned; }
}

fn make() {
    App { };
}

fn main() {
    make();
    make();
}
"#;

fn field(line: &str, key: &str) -> Option<i64> {
    line.split_whitespace()
        .find_map(|w| w.strip_prefix(key).and_then(|v| v.strip_prefix('=')))
        .and_then(|v| v.parse().ok())
}

/// Build the program (devirtualized, or with `no_bus_devirt`), run it
/// under a deadline, and return what it printed with its exit status.
fn run(no_bus_devirt: bool) -> (String, Option<i32>) {
    let bin = harness::unique_bin(&format!("placement_occurrences_{no_bus_devirt}"));
    let opts = BuildOptions { no_bus_devirt, ..build_opts::options() };
    build_opts::build_source(SRC, &bin, &opts).unwrap_or_else(|e| panic!("build: {e:?}"));
    let mut child = Command::new(&bin).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().expect("spawn");
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
    (stdout, status)
}

/// `src`'s snapshot, checked clean.
fn snapshot(src: &str) -> Snapshot {
    let program = hale_syntax::parse_source(src).expect("parse");
    let Ok(s) = Snapshot::from_program(program, Vec::new(), Config::check(false, false)) else {
        panic!("the program does not load")
    };
    let checked = s.demand_check().unwrap_or_else(|_| panic!("the check is blocked"));
    let errors: Vec<&str> = checked.diags.iter().filter(|d| d.is_error()).map(|d| d.message.as_str()).collect();
    assert!(errors.is_empty(), "the program checks clean: {errors:?}");
    s
}

/// The table of `s`: its one construction's bound, its pinned domains
/// (each anchored in that construction), and the budget's thread count.
fn template(s: &Snapshot) -> (Bound, usize, Result<usize, String>) {
    let t = s.demand_placement().unwrap_or_else(|_| panic!("placement is blocked"));
    let root = t.root.as_ref().expect("a root");
    assert_eq!(root.constructions.len(), 1, "one construction literal, `make`'s");
    let pinned: Vec<_> = t.domains.iter().filter(|d| matches!(d.kind, DomainKind::Pinned { .. })).collect();
    for d in &pinned {
        let DomainKind::Pinned { anchor, .. } = &d.kind else { unreachable!() };
        assert_eq!(anchor.origin, Origin::Construction(root.constructions[0].literal));
    }
    let summary = s.demand_alloc_summary().unwrap_or_else(|_| panic!("no allocation summary"));
    let budget = hale_types::resource_budget::budget_for_programs(&s.bundle(), t, summary);
    (root.constructions[0].bound.clone(), pinned.len(), budget.threads())
}

/// The static half: a template's pinned field is one domain however many
/// of its occurrences run, and the budget multiplies the template's
/// anchors by its bound. Built by a fn called twice, two threads; built
/// by a recursive fn, as the runtime half's is, no static bound, so the
/// count is an uncertainty with the reason.
#[test]
fn a_template_is_one_domain_and_the_budget_multiplies_it_by_its_bound() {
    let (bound, domains, threads) = template(&snapshot(TWICE));
    assert_eq!(bound, Bound::AtMost(2), "`make` is called twice");
    assert_eq!(domains, 1, "one domain for the template's one pinned field");
    assert_eq!(threads, Ok(2), "the template's one anchor times its bound");

    let (bound, domains, threads) = template(&snapshot(SRC));
    let Bound::Unbounded(why) = bound else { panic!("`make` is reached from the root's own body: {bound:?}") };
    assert_eq!(domains, 1, "one domain, whatever number of occurrences run");
    assert_eq!(threads, Err(why));
}

/// The runtime half: each occurrence has its own anchor thread, and its
/// nested subscriber receives on it, in both devirtualization arms.
#[test]
fn each_live_occurrence_of_a_template_runs_on_its_own_anchor() {
    for no_bus_devirt in [false, true] {
        occurrences_in(no_bus_devirt);
    }
}

fn occurrences_in(no_bus_devirt: bool) {
    let (stdout, status) = run(no_bus_devirt);
    let stdout = format!("(no_bus_devirt = {no_bus_devirt})\n{stdout}");
    let mut main = None;
    let mut ready: BTreeMap<i64, i64> = BTreeMap::new();
    let mut seen: Vec<(i64, i64, i64)> = Vec::new();
    let mut counts: BTreeMap<i64, i64> = BTreeMap::new();
    for line in stdout.lines() {
        if line.starts_with("MAIN ") {
            main = field(line, "tid");
        } else if line.starts_with("READY ") {
            ready.insert(field(line, "who").expect("who"), field(line, "tid").expect("tid"));
        } else if line.starts_with("SEEN ") {
            seen.push((field(line, "who").expect("who"), field(line, "seq").expect("seq"), field(line, "tid").expect("tid")));
        } else if line.starts_with("COUNT ") {
            counts.insert(field(line, "who").expect("who"), field(line, "n").expect("n"));
        }
    }
    assert_eq!(status, Some(0), "the run completes:\n{stdout}");
    assert!(stdout.lines().any(|l| l == "DONE"), "{stdout}");
    let main = main.expect("main's thread");
    let (t1, t2) = (ready[&1], ready[&2]);
    assert_ne!(t1, t2, "two live occurrences, two anchor threads:\n{stdout}");
    assert!(t1 != main && t2 != main, "neither anchor is main:\n{stdout}");
    for who in [1, 2] {
        let mine: Vec<_> = seen.iter().filter(|(w, _, _)| *w == who).collect();
        let seqs: Vec<i64> = {
            let mut v: Vec<i64> = mine.iter().map(|(_, s, _)| *s).collect();
            v.sort();
            v
        };
        assert_eq!(seqs, [1, 2, 3, 4, 5], "occurrence {who} receives each tick once:\n{stdout}");
        assert!(
            mine.iter().all(|(_, _, tid)| *tid == ready[&who]),
            "occurrence {who}'s subscriber receives on its own anchor's thread, never the other's:\n{stdout}"
        );
        assert_eq!(counts.get(&who), Some(&5), "occurrence {who} counted five:\n{stdout}");
    }
}

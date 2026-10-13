//! A static tree's subscriptions are reserved before its first birth
//! (#1534; spec/runtime.md § "Lifecycle obligations", line 6; friction
//! sweep 2026-10-12, downstream handoff).
//!
//! A tree's nodes register their subscriptions as each is built, so what
//! an earlier params sibling's `birth()` or inline `run()` published to a
//! later one reached no subscription of it and was lost. The root now
//! reserves, before its params are built, every subject one node of its
//! tree subscribes to and another may publish before that subscriber
//! registers; each registration claims what was held for it, and its
//! readiness delivers it once its `birth()` completes. These hold the
//! shapes to every cell, delivered once and in order: siblings in one
//! seed and across seeds, a nested subscriber, a devirtualized (quiet)
//! subscriber, keyed subscribers (scalar and String keys), a pinned
//! anchor, the subtrees a pinned and a pool-placed field build on their
//! own threads, the topic's bound, and a publisher on another thread
//! hammering while the tree is built — with and without a stall that
//! makes a park land after the claim it raced (the forward path), and
//! under ASan. The IR tests pin the cost: a tree that needs no
//! reservation emits none, and the publish site is the same IR whether
//! or not one exists, so the direct-call fast path is unchanged.

use std::path::Path;
use std::process::Command;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

/// Build `src` (optionally under ASan), run it with `env`, and return
/// (stdout lines sorted, stderr). Fails on a sanitizer report or a
/// non-zero exit.
fn run_env(tag: &str, src: &str, asan: bool, env: &[(&str, &str)]) -> (Vec<String>, String) {
    let bin = harness::unique_bin(tag);
    if asan {
        harness::build_source_asan(src, &bin);
    } else {
        build_opts::build_source(src, &bin, &build_opts::options()).expect("build");
    }
    run_bin(&bin, env)
}

fn run_bin(bin: &Path, env: &[(&str, &str)]) -> (Vec<String>, String) {
    let mut cmd = Command::new(bin);
    cmd.env("LOTUS_NO_CHUNK_POOL", "1");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run");
    let _ = std::fs::remove_file(bin);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    for bad in ["AddressSanitizer", "LeakSanitizer", "SUMMARY:"] {
        assert!(!stderr.contains(bad), "{bad} in:\n{stderr}");
    }
    assert!(out.status.success(), "{:?}\n{stdout}\n{stderr}", out.status);
    let mut lines: Vec<String> = stdout.lines().map(str::to_string).collect();
    lines.sort();
    (lines, stderr)
}

fn run(tag: &str, src: &str) -> Vec<String> {
    run_env(tag, src, false, &[]).0
}

const SIBLINGS: &str = r#"
    type Note { n: Int; }
    topic Fact { payload: Note; }
    locus Teller { params { x: Int = 0; } bus { publish Fact; } birth() { Fact <- Note { n: 7 }; } }
    locus Runner { params { x: Int = 0; } bus { publish Fact; } run() { Fact <- Note { n: 30 }; } }
    locus Ear {
        params { tag: String = "?"; born: Bool = false; }
        bus { subscribe Fact as on_fact; }
        birth() { self.born = true; }
        fn on_fact(f: Note) { println(self.tag + " " + to_string(f.n) + " born " + to_string(self.born)); }
    }
    locus Shell { params { inner: Ear = Ear { tag: "inner" }; } }
    main locus Root {
        params {
            first: Ear = Ear { tag: "first" };
            t: Teller = Teller { };
            r: Runner = Runner { };
            later: Ear = Ear { tag: "later" };
            s: Shell = Shell { };
        }
        run() { yield; std::time::sleep(20ms); }
    }
    fn main() { Root { }; }
"#;

const SIBLINGS_HEARD: [&str; 6] = [
    "first 30 born true",
    "first 7 born true",
    "inner 30 born true",
    "inner 7 born true",
    "later 30 born true",
    "later 7 born true",
];

#[test]
fn an_earlier_siblings_birth_and_run_reach_later_siblings() {
    // A subscriber before the publishers, one after them, and one nested
    // a level down: each hears the birth's 7 and the inline run()'s 30
    // once, after its own birth.
    assert_eq!(run("siblings", SIBLINGS), SIBLINGS_HEARD);
}

#[test]
fn an_earlier_siblings_publish_reaches_a_later_sibling_under_asan() {
    let (out, _) = run_env("siblings_asan", SIBLINGS, true, &[]);
    assert_eq!(out, SIBLINGS_HEARD);
}

#[test]
fn a_quiet_later_subscriber_hears_an_earlier_sibling() {
    // A handler that only folds into its own state is a direct call when
    // nothing is held; the reservation holds a window open, so the
    // publish takes the helper and parks.
    let src = r#"
        type Note { n: Int; }
        topic Fact { payload: Note; }
        locus Teller { params { x: Int = 0; } bus { publish Fact; } birth() { Fact <- Note { n: 7 }; Fact <- Note { n: 100 }; } }
        locus Tally {
            params { sum: Int = 0; hits: Int = 0; }
            bus { subscribe Fact as on_fact; }
            fn on_fact(f: Note) { self.sum = self.sum + f.n; self.hits = self.hits + 1; }
            run() { std::time::sleep(30ms); println("sum " + to_string(self.sum) + " hits " + to_string(self.hits)); }
        }
        main locus Root {
            params { t: Teller = Teller { }; y: Tally = Tally { }; }
            run() { std::time::sleep(60ms); }
        }
        fn main() { Root { }; }
    "#;
    assert_eq!(run("quiet", src), ["sum 107 hits 2"]);
}

#[test]
fn keyed_later_subscribers_take_their_key_at_the_claim() {
    let src = r#"
        type Tick { lane: Int; v: Int; }
        type Memo { who: String; v: Int; }
        topic Ticks { payload: Tick; subject: "rsv.ticks"; keyed_by lane; }
        topic Memos { payload: Memo; subject: "rsv.memos"; keyed_by who; }
        locus Source {
            params { x: Int = 0; }
            bus { publish Ticks; publish Memos; }
            birth() {
                Ticks <- Tick { lane: 1, v: 10 };
                Ticks <- Tick { lane: 2, v: 20 };
                Memos <- Memo { who: "ann", v: 1 };
                Memos <- Memo { who: "bob", v: 2 };
            }
        }
        locus Lane {
            params { lane: Int = 0; }
            bus { subscribe Ticks as on_tick where key == self.lane; }
            fn on_tick(t: Tick) { println("lane " + to_string(self.lane) + " " + to_string(t.v)); }
        }
        locus Desk {
            params { who: String = ""; }
            bus { subscribe Memos as on_memo where key == self.who; }
            fn on_memo(m: Memo) { println("desk " + self.who + " " + to_string(m.v)); }
        }
        locus Every {
            params { x: Int = 0; }
            bus { subscribe Ticks as on_tick; }
            fn on_tick(t: Tick) { println("every " + to_string(t.v)); }
        }
        main locus Root {
            params {
                s: Source = Source { };
                l1: Lane = Lane { lane: 1 };
                l2: Lane = Lane { lane: 2 };
                d: Desk = Desk { who: "bob" };
                e: Every = Every { };
            }
            run() { yield; std::time::sleep(20ms); }
        }
        fn main() { Root { }; }
    "#;
    assert_eq!(run("keyed", src), ["desk bob 2", "every 10", "every 20", "lane 1 10", "lane 2 20"]);
}

#[test]
fn a_keyed_parent_hears_its_childrens_birth_once() {
    // A parent's keyed subscription registers after its params (its key
    // may read them), so its children's births publish before it exists:
    // the root's reservation holds them, and the parent's claim takes the
    // one its key names, once — the other key's is not its, and nothing
    // reaches it twice (the shape a DNA workflow's fixture builds: Works
    // asking their state at birth, the fixture answering the left one).
    let src = r#"
        type Ask { scope: String; n: Int; }
        topic Asked { payload: Ask; subject: "rsv.asked"; keyed_by scope; }
        locus Work {
            params { scope: String = ""; n: Int = 0; }
            bus { publish Asked; }
            birth() { Asked <- Ask { scope: self.scope, n: self.n }; }
        }
        main locus Fixture {
            params {
                left: Work = Work { scope: "left", n: 1 };
                right: Work = Work { scope: "right", n: 2 };
                heard: Int = 0;
            }
            bus { subscribe Asked as on_ask where key == "left"; }
            fn on_ask(a: Ask) { self.heard = self.heard + 1; println("asked " + a.scope + " " + to_string(a.n)); }
            run() { yield; std::time::sleep(20ms); println("heard " + to_string(self.heard)); }
        }
        fn main() { Fixture { }; }
    "#;
    let (out, stderr) = run_env("keyed_parent", src, false, &[("LOTUS_BUS_LOG_RESERVE", "1")]);
    assert_eq!(out, ["asked left 1", "heard 1"], "{stderr}");
    assert!(stderr.contains("reservation `rsv.asked` closed: parked 2 copied 1 forwarded 0 shed 0 claimants 1 missing 0"), "{stderr}");
}

#[test]
fn a_pinned_anchor_hears_an_earlier_sibling() {
    let src = r#"
        type Note { n: Int; }
        topic Fact { payload: Note; subject: "rsv.pinned"; }
        locus Teller { params { x: Int = 0; } bus { publish Fact; } birth() { Fact <- Note { n: 7 }; } }
        locus Far {
            params { x: Int = 0; }
            bus { subscribe Fact as on_fact; }
            fn on_fact(f: Note) { println("far " + to_string(f.n)); }
        }
        main locus Root {
            params { t: Teller = Teller { }; f: Far = Far { }; }
            placement { f: pinned(core = 0); }
            run() { std::time::sleep(50ms); }
        }
        fn main() { Root { }; }
    "#;
    assert_eq!(run("pinned", src), ["far 7"]);
}

#[test]
fn subtrees_built_on_an_anchors_thread_take_part() {
    // A pinned field's params and a pool-placed field's are built on
    // that field's own thread, in an init the root's thread waits for:
    // their subscribers claim from there, and what they publish there
    // reaches a later sibling.
    let src = r#"
        type Note { n: Int; }
        topic Fact { payload: Note; subject: "rsv.anchor.fact"; }
        topic Echo { payload: Note; subject: "rsv.anchor.echo"; }
        locus Teller { params { x: Int = 0; } bus { publish Fact; } birth() { Fact <- Note { n: 7 }; } }
        locus Ear {
            params { tag: String = "?"; }
            bus { subscribe Fact as on_fact; }
            fn on_fact(f: Note) { println(self.tag + " " + to_string(f.n)); }
        }
        locus Flood {
            params { x: Int = 0; }
            bus { publish Echo; }
            birth() { let mut i = 0; while i < 5000 { Echo <- Note { n: i }; i = i + 1; } }
        }
        locus Hold { params { e: Ear = Ear { tag: "pinned.inner" }; f: Flood = Flood { }; } }
        locus PoolHold { params { e: Ear = Ear { tag: "pool.inner" }; } }
        locus Count {
            params { n: Int = 0; }
            bus { subscribe Echo as on_echo; }
            fn on_echo(f: Note) { self.n = self.n + 1; if f.n == 4999 { println("echo " + to_string(self.n)); } }
        }
        main locus Root {
            params { t: Teller = Teller { }; h: Hold = Hold { }; p: PoolHold = PoolHold { }; c: Count = Count { }; }
            placement { h: pinned(core = 0); p: cooperative(pool = io); }
            run() { std::time::sleep(100ms); }
        }
        fn main() { Root { }; }
    "#;
    let (out, stderr) = run_env("anchors", src, false, &[("LOTUS_BUS_LOG_RESERVE", "1")]);
    assert_eq!(out, ["echo 5000", "pinned.inner 7", "pool.inner 7"], "{stderr}");
    assert!(stderr.contains("reservation `rsv.anchor.echo` closed: parked 5000 copied 5000"), "{stderr}");
}

#[test]
fn an_imported_seeds_publisher_reaches_a_later_sibling() {
    let dir = harness::unique_dir("cross_seed");
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    std::fs::create_dir_all(dir.join("app")).unwrap();
    std::fs::write(
        dir.join("lib").join("lib.hl"),
        r#"
type Beat { n: Int = 0; }
topic BeatT { payload: Beat; subject: "rsv.beat"; }
locus Drum { bus { publish BeatT; } run() { BeatT <- Beat { n: 1 }; } }
"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("app").join("main.hl"),
        r#"
import "../lib" as l;
locus Listener { bus { subscribe l::BeatT as on_beat; } fn on_beat(b: l::Beat) { println("heard " + to_string(b.n)); } }
main locus Root { params { d: l::Drum = l::Drum { }; h: Listener = Listener { }; } run() { std::time::sleep(20ms); } }
fn main() { Root { }; }
"#,
    )
    .unwrap();
    let bin = harness::unique_bin("cross_seed_bin");
    build_opts::build_seed_dir(&dir.join("app").join("main.hl"), &bin, &build_opts::options()).expect("build");
    let (out, _) = run_bin(&bin, &[]);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(out, ["heard 1"]);
}

#[test]
fn the_topics_bound_applies_to_the_reservation() {
    // `on_full: fail` has no publisher left to raise into once the
    // subscriber exists: the reservation keeps its first three and sheds
    // the rest, counted, saying so once. A `drop_old` bound keeps the
    // newest four.
    let src = r#"
        type Ev { n: Int = 0; }
        topic Strict { payload: Ev; subject: "rsv.strict"; bounded(3); on_full: fail; }
        topic Loose { payload: Ev; subject: "rsv.loose"; }
        locus Burst {
            params { x: Int = 0; }
            bus { publish Strict; publish Loose; }
            birth() {
                let mut i = 0;
                while i < 10 { Strict <- Ev { n: i } or wait; Loose <- Ev { n: i }; i = i + 1; }
            }
        }
        locus Gauge {
            params { x: Int = 0; }
            bus { subscribe Strict as on_strict; subscribe Loose as on_loose bounded(4, drop_old); }
            fn on_strict(e: Ev) { println("strict " + to_string(e.n)); }
            fn on_loose(e: Ev) { println("loose " + to_string(e.n)); }
        }
        main locus Root {
            params { b: Burst = Burst { }; g: Gauge = Gauge { }; }
            run() { yield; std::time::sleep(20ms); }
        }
        fn main() { Root { }; }
    "#;
    let (out, stderr) = run_env("bound", src, false, &[("LOTUS_BUS_LOG_RESERVE", "1")]);
    assert_eq!(
        out,
        ["loose 6", "loose 7", "loose 8", "loose 9", "strict 0", "strict 1", "strict 2"],
        "{stderr}"
    );
    assert_eq!(stderr.matches("bounded with `on_full: fail`").count(), 1, "{stderr}");
    assert!(stderr.contains("reservation `rsv.strict` closed: parked 10 copied 3 forwarded 0 shed 7"), "{stderr}");
    assert!(stderr.contains("reservation `rsv.loose` closed: parked 10 copied 4 forwarded 0 shed 6"), "{stderr}");
}

#[test]
fn a_construction_a_failure_leaves_closes_its_reservation() {
    // The failure propagates out of the tree's params part-way: the
    // reservation it opened closes on that exit, saying what it held for
    // a subscriber that was never built, and holds nothing after — a
    // later tree built elsewhere is untouched by it.
    let src = r#"
        type Note { n: Int; }
        type Oops { why: String; }
        topic Fact { payload: Note; subject: "rsv.left"; }
        fn pick(ok: Bool) -> Int fallible(Oops) { if ok { return 1; } fail Oops { why: "no" }; }
        locus Teller { params { x: Int = 0; } bus { publish Fact; } birth() { Fact <- Note { n: 7 }; } }
        locus Ear { params { v: Int = 0; } bus { subscribe Fact as on_fact; } fn on_fact(f: Note) { println("ear " + to_string(f.n)); } }
        locus Tree { params { t: Teller = Teller { }; v: Int = pick(false) or raise; e: Ear = Ear { }; } }
        fn build() -> Int fallible(Oops) { let t = Tree { }; return 1; }
        fn main() { let r = build() or 0; println("built " + to_string(r)); let e = Ear { }; Teller { }; }
    "#;
    let (out, stderr) = run_env("abandoned", src, false, &[("LOTUS_BUS_LOG_RESERVE", "1")]);
    assert_eq!(out, ["built 0", "ear 7"], "{stderr}");
    assert!(stderr.contains("reservation `rsv.left` closed: parked 1 copied 0 forwarded 0 shed 0 claimants 0 missing 1"), "{stderr}");
    assert!(stderr.contains("1 subscription(s) of `rsv.left` reserved for a statically built tree never registered"), "{stderr}");
}

#[test]
fn a_publisher_never_waits_on_a_reservation_and_the_drain_completes() {
    // The tree's reservation stays open while `Slow`'s birth waits for the
    // drain, and a pinned publisher floods the subject meanwhile. Nothing
    // waits on a reservation: past a queue's worth it sheds, counted and
    // said once, so the publisher keeps running, sees the drain, and the
    // process ends within the grace. (When a publisher on another thread
    // waited at the cap for the reservation to close, it stood still at a
    // queue's worth and could not see the drain while it waited.)
    use std::io::{BufRead, BufReader};
    use std::process::Stdio;
    use std::sync::mpsc;
    use std::time::Duration;
    let src = r#"
        type Note { n: Int; }
        topic Feed { payload: Note; subject: "rsv.drain"; }
        locus Pump {
            params { sent: Int = 0; }
            bus { publish Feed; }
            run() {
                while !self.draining { Feed <- Note { n: self.sent }; self.sent = self.sent + 1; }
                println("pump sent " + to_string(self.sent));
            }
        }
        locus Slow {
            params { x: Int = 0; }
            birth() {
                println("slow waiting");
                while !self.draining && self.x < 2000 { std::time::sleep(10ms); self.x = self.x + 1; }
            }
        }
        locus Sink {
            params { got: Int = 0; }
            bus { subscribe Feed as on_feed; }
            fn on_feed(f: Note) { self.got = self.got + 1; }
        }
        main locus Root {
            params { p: Pump = Pump { }; w: Slow = Slow { }; s: Sink = Sink { }; }
            placement { p: pinned; }
            run() { while !self.draining { std::time::sleep(10ms); } }
        }
        fn main() { Root { }; }
    "#;
    let bin = harness::unique_bin("drain_open");
    build_opts::build_source(src, &bin, &build_opts::options()).expect("build");
    let mut child = Command::new(&bin)
        .env("LOTUS_DRAIN_GRACE_MS", "3000")
        .env("LOTUS_BUS_LOG_RESERVE", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut first = String::new();
        let _ = stdout.read_line(&mut first);
        let _ = tx.send(first.clone());
        let mut rest = String::new();
        let _ = std::io::Read::read_to_string(&mut stdout, &mut rest);
        let _ = tx.send(rest);
    });
    let first = rx.recv_timeout(Duration::from_secs(30)).expect("the tree starts");
    assert_eq!(first.trim(), "slow waiting");
    // the pump floods well past a queue's worth while the tree is built
    std::thread::sleep(Duration::from_millis(300));
    let killed = Command::new("kill").args(["-TERM", &child.id().to_string()]).status().expect("kill");
    assert!(killed.success());
    let rest = rx.recv_timeout(Duration::from_secs(20)).expect("the process ends");
    let out = child.wait_with_output().expect("wait");
    let _ = std::fs::remove_file(&bin);
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(!stderr.contains("did not finish within"), "the drain hit the grace:\n{stderr}");
    assert!(out.status.success(), "{:?}\n{rest}\n{stderr}", out.status);
    let sent: u64 = rest
        .lines()
        .find_map(|l| l.strip_prefix("pump sent "))
        .and_then(|n| n.trim().parse().ok())
        .unwrap_or_else(|| panic!("the pump never stopped:\n{rest}\n{stderr}"));
    assert!(sent > 4 * 8192, "the pump stood still at the reservation's cap: sent {sent}\n{stderr}");
    assert_eq!(stderr.matches("are held for it, a queue's worth; further ones are shed").count(), 1, "{stderr}");
    assert!(stderr.contains("copied 8192 forwarded 0"), "{stderr}");
}

/// Two later subscribers bound the subject their own way: `First` keeps
/// the oldest two (`drop_new`), `Last` the newest two (`drop_old`). The
/// reservation retains what both need, and each claim applies its own
/// bound, whichever field comes first.
fn private_bounds(fields: &str) -> String {
    r#"
    type Ev { n: Int; }
    topic Evs { payload: Ev; subject: "rsv.private"; }
    locus Burst { params { x: Int = 0; } bus { publish Evs; } birth() { let mut i = 0; while i < 4 { Evs <- Ev { n: i }; i = i + 1; } } }
    locus First {
        params { x: Int = 0; }
        bus { subscribe Evs as on_ev bounded(2, drop_new); }
        fn on_ev(e: Ev) { println("first " + to_string(e.n)); }
    }
    locus Last {
        params { x: Int = 0; }
        bus { subscribe Evs as on_ev bounded(2, drop_old); }
        fn on_ev(e: Ev) { println("last " + to_string(e.n)); }
    }
    main locus Root { params { b: Burst = Burst { }; f: First = First { }; l: Last = Last { }; } run() { yield; std::time::sleep(20ms); } }
    fn main() { Root { }; }
    "#
    .replace("b: Burst = Burst { }; f: First = First { }; l: Last = Last { };", fields)
}

#[test]
fn each_claimant_keeps_its_own_bound() {
    for fields in [
        "b: Burst = Burst { }; f: First = First { }; l: Last = Last { };",
        "b: Burst = Burst { }; l: Last = Last { }; f: First = First { };",
    ] {
        let (out, stderr) = run_env("private_bounds", &private_bounds(fields), false, &[]);
        assert_eq!(out, ["first 0", "first 1", "last 2", "last 3"], "{fields}\n{stderr}");
    }
}

const KEYED_BOUNDS: &str = r#"
    type Ev { lane: Int; n: Int; }
    topic Lanes { payload: Ev; subject: "rsv.lanes"; keyed_by lane; }
    locus Burst {
        params { x: Int = 0; }
        bus { publish Lanes; }
        birth() {
            Lanes <- Ev { lane: 0, n: 0 };
            Lanes <- Ev { lane: 0, n: 1 };
            Lanes <- Ev { lane: 1, n: 0 };
            Lanes <- Ev { lane: 1, n: 1 };
        }
    }
    locus Ear {
        params { lane: Int = 0; }
        bus { subscribe Lanes as on_ev bounded(2, drop_new) where key == self.lane; }
        fn on_ev(e: Ev) { println("ear " + to_string(self.lane) + " " + to_string(e.n)); }
    }
    main locus Root {
        params { b: Burst = Burst { }; e0: Ear = Ear { lane: 0 }; e1: Ear = Ear { lane: 1 }; }
        run() { yield; std::time::sleep(20ms); }
    }
    fn main() { Root { }; }
"#;

#[test]
fn a_keyed_claimants_bound_is_over_its_own_keys_cells() {
    // Two cells for each key; each Ear's bound of two is over its own
    // key's cells, so neither loses one to the other key's, in either
    // policy.
    for policy in ["drop_new", "drop_old"] {
        let src = KEYED_BOUNDS.replace("bounded(2, drop_new)", &format!("bounded(2, {policy})"));
        let (out, stderr) = run_env("keyed_bounds", &src, false, &[]);
        assert_eq!(out, ["ear 0 0", "ear 0 1", "ear 1 0", "ear 1 1"], "{policy}\n{stderr}");
    }
}

const MANY_KEYS: &str = r#"
    type Ev { lane: Int; n: Int; }
    topic Lanes { payload: Ev; subject: "rsv.many"; keyed_by lane; }
    locus Burst {
        params { x: Int = 0; }
        bus { publish Lanes; }
        birth() { let mut i = 0; while i < 100000 { Lanes <- Ev { lane: i, n: i }; i = i + 1; } }
    }
    locus Ear {
        params { lane: Int = 5; }
        bus { subscribe Lanes as on_ev bounded(2, drop_new) where key == self.lane; }
        fn on_ev(e: Ev) { println("ear " + to_string(e.n)); }
    }
    main locus Root {
        params { b: Burst = Burst { }; e: Ear = Ear { }; }
        run() { yield; std::time::sleep(20ms); }
    }
    fn main() { Root { }; }
"#;

const MANY_STRING_KEYS: &str = r#"
    type Ev { lane: String; n: Int; }
    topic Lanes { payload: Ev; subject: "rsv.many.str"; keyed_by lane; }
    locus Burst {
        params { x: Int = 0; }
        bus { publish Lanes; }
        birth() { let mut i = 0; while i < 100000 { Lanes <- Ev { lane: "l" + to_string(i), n: i }; i = i + 1; } }
    }
    locus Ear {
        params { lane: String = "l5"; }
        bus { subscribe Lanes as on_ev bounded(2, drop_new) where key == self.lane; }
        fn on_ev(e: Ev) { println("ear " + to_string(e.n)); }
    }
    main locus Root {
        params { b: Burst = Burst { }; e: Ear = Ear { }; }
        run() { yield; std::time::sleep(20ms); }
    }
    fn main() { Root { }; }
"#;

#[test]
fn a_flood_of_keys_holds_a_queues_worth_of_records() {
    // 100,000 distinct keys before their one subscriber claims: the
    // reservation keeps records only for keys it retains a cell of, never
    // more than a queue's worth, found by hash; a key past the cap is shed
    // with no record. The subscriber's own key came early and is kept.
    for (tag, src, subject) in [("many_keys", MANY_KEYS, "rsv.many"), ("many_str_keys", MANY_STRING_KEYS, "rsv.many.str")] {
        let started = std::time::Instant::now();
        let (out, stderr) = run_env(tag, src, false, &[("LOTUS_BUS_LOG_RESERVE", "1")]);
        assert_eq!(out, ["ear 5"], "{tag}\n{stderr}");
        let counts = closed_counts(&stderr, subject);
        let get = |k: &str| counts.iter().find(|(n, _)| n == k).map(|(_, v)| *v).unwrap_or(u64::MAX);
        assert_eq!(get("parked"), 100000, "{stderr}");
        assert_eq!(get("keys"), 8192, "one record per retained key, a queue's worth: {stderr}");
        assert_eq!(get("shed"), 100000 - 8192, "{stderr}");
        assert!(started.elapsed() < std::time::Duration::from_secs(60), "{tag} took {:?}", started.elapsed());
    }
}

/// A publisher on its own thread, started early in the tree, sends
/// `count` numbered cells (a short pause every ten) while the rest of the
/// tree is built, slowly; every subscriber, built before it or after,
/// must see each number once, in order.
fn hammer(count: u32) -> String {
    r#"
    type Seq { n: Int; last: Bool; }
    topic Feed { payload: Seq; subject: "rsv.feed"; }
    locus Hammer {
        params { count: Int = 3000; }
        bus { publish Feed; }
        run() {
            let mut i = 0;
            while i < self.count {
                Feed <- Seq { n: i, last: i == self.count - 1 };
                i = i + 1;
                if i % 10 == 0 { std::time::sleep(100us); }
            }
        }
    }
    locus Sink {
        params { tag: String = "?"; next: Int = 0; bad: Int = 0; }
        bus { subscribe Feed as on_feed; }
        fn on_feed(s: Seq) {
            if s.n != self.next { self.bad = self.bad + 1; }
            self.next = s.n + 1;
            if s.last { println(self.tag + " next " + to_string(self.next) + " bad " + to_string(self.bad)); }
        }
    }
    locus Slow { params { x: Int = 0; } birth() { std::time::sleep(3ms); } }
    locus Pair { params { a: Sink = Sink { tag: "pair.a" }; s: Slow = Slow { }; b: Sink = Sink { tag: "pair.b" }; } }
    main locus Root {
        params {
            early: Sink = Sink { tag: "early" };
            h: Hammer = Hammer { };
            s0: Slow = Slow { };
            p1: Pair = Pair { };
            s1: Slow = Slow { };
            late: Sink = Sink { tag: "late" };
            p2: Pair = Pair { };
            p3: Pair = Pair { };
        }
        placement { h: pinned(core = 0); }
        run() { std::time::sleep(500ms); }
    }
    fn main() { Root { }; }
    "#
    .replace("count: Int = 3000;", &format!("count: Int = {count};"))
}

fn hammer_heard(count: u32) -> Vec<String> {
    let line = |tag: &str| format!("{tag} next {count} bad 0");
    let mut v = vec![line("early"), line("late")];
    for _ in 0..3 {
        v.push(line("pair.a"));
        v.push(line("pair.b"));
    }
    v.sort();
    v
}

/// The reservation's closing counts, from `LOTUS_BUS_LOG_RESERVE`.
fn closed_counts(stderr: &str, subject: &str) -> Vec<(String, u64)> {
    let head = format!("reservation `{subject}` closed: ");
    let line = stderr.lines().find_map(|l| l.split_once(&head).map(|(_, rest)| rest.to_string()));
    let line = line.unwrap_or_else(|| panic!("no closing line for `{subject}` in:\n{stderr}"));
    let words: Vec<&str> = line.split_whitespace().collect();
    words.chunks(2).map(|kv| (kv[0].to_string(), kv[1].parse().unwrap_or(0))).collect()
}

#[test]
fn a_cross_thread_publisher_during_construction_delivers_each_cell_once() {
    let (out, stderr) = run_env("hammer", &hammer(3000), false, &[("LOTUS_BUS_LOG_RESERVE", "1")]);
    assert_eq!(out, hammer_heard(3000), "{stderr}");
    let counts = closed_counts(&stderr, "rsv.feed");
    let get = |k: &str| counts.iter().find(|(n, _)| n == k).map(|(_, v)| *v).unwrap_or(0);
    assert_eq!(get("missing"), 0, "{stderr}");
    assert_eq!(get("claimants"), 8, "{stderr}");
}

#[test]
fn a_park_after_the_claim_it_raced_is_forwarded_once() {
    // The stall holds each cross-thread publish between the view it
    // walked and its park, so claims land in between: those cells reach
    // the claimants the publish did not see by the forward, once.
    let (out, stderr) = run_env(
        "hammer_stall",
        &hammer(3000),
        false,
        &[("LOTUS_BUS_LOG_RESERVE", "1"), ("LOTUS_BUS_TEST_RESERVE_STALL_US", "300")],
    );
    assert_eq!(out, hammer_heard(3000), "{stderr}");
    let counts = closed_counts(&stderr, "rsv.feed");
    let get = |k: &str| counts.iter().find(|(n, _)| n == k).map(|(_, v)| *v).unwrap_or(0);
    assert!(get("forwarded") > 0, "the stall forced no forward:\n{stderr}");
    assert!(get("copied") > 0, "{stderr}");
}

#[test]
fn a_cross_thread_publisher_during_construction_under_asan() {
    let (out, stderr) = run_env(
        "hammer_asan",
        &hammer(1500),
        true,
        &[("LOTUS_BUS_TEST_RESERVE_STALL_US", "200")],
    );
    assert_eq!(out, hammer_heard(1500), "{stderr}");
}

/// The body of the function `@name` in `ir`.
fn body<'a>(ir: &'a str, name: &str) -> &'a str {
    let head = format!("@{name}(");
    let start = ir
        .lines()
        .scan(0usize, |at, l| {
            let here = *at;
            *at += l.len() + 1;
            Some((here, l))
        })
        .find(|(_, l)| l.starts_with("define") && l.contains(&head))
        .map(|(at, _)| at)
        .unwrap_or_else(|| panic!("no function {name} in the IR"));
    let rest = &ir[start..];
    &rest[..rest.find("\n}\n").map(|e| e + 2).unwrap_or(rest.len())]
}

fn ir(tag: &str, src: &str) -> String {
    let bin = harness::unique_bin(tag);
    let text = harness::build_source_ir_text(src, &bin).expect("build");
    let _ = std::fs::remove_file(&bin);
    text
}

/// A quiet subscriber and its publisher, in the order `fields` builds
/// them.
fn direct_program(fields: &str) -> String {
    r#"
    type Note { n: Int; }
    topic Fact { payload: Note; subject: "rsv.direct"; }
    locus Teller { params { x: Int = 0; } bus { publish Fact; } birth() { Fact <- Note { n: 7 }; } }
    locus Tally {
        params { sum: Int = 0; }
        bus { subscribe Fact as on_fact; }
        fn on_fact(f: Note) { self.sum = self.sum + f.n; }
    }
    main locus Root { params { t: Teller = Teller { }; y: Tally = Tally { }; } run() { } }
    fn main() { Root { }; }
    "#
    .replace("t: Teller = Teller { }; y: Tally = Tally { };", fields)
}

#[test]
fn the_direct_call_fast_path_is_unchanged_without_a_reservation() {
    // The subscriber first: nothing is published before it registers, so
    // nothing is reserved and no reservation call is emitted at all.
    let without = ir("direct_without", &direct_program("y: Tally = Tally { }; t: Teller = Teller { };"));
    assert!(!without.contains("@lotus_bus_reserve_begin"), "a tree needing no reservation emitted one");
    // The publisher first: the root reserves `rsv.direct`.
    let with = ir("direct_with", &direct_program("t: Teller = Teller { }; y: Tally = Tally { };"));
    assert!(with.contains("call void @lotus_bus_reserve("), "the sibling shape reserved nothing");
    // The publish site is the baked direct call either way, behind the
    // one readiness load it always had: the reservation is the runtime's,
    // consulted through the window count the guard already reads.
    let site_without = body(&without, "Teller.birth");
    let site_with = body(&with, "Teller.birth");
    assert!(site_without.contains("@lotus_bus_static_direct_count("), "not the baked direct path:\n{site_without}");
    assert!(site_without.contains("@lotus_bus_unready_count"), "{site_without}");
    assert!(!site_without.contains("@lotus_bus_reserve_"), "{site_without}");
    assert_eq!(site_without, site_with, "a reservation changed the publish site's IR");
}

#[test]
fn a_parent_only_subscriber_needs_no_reservation() {
    // A parent registers before its params (#1533), so its children's
    // publishes reach it without one.
    let src = r#"
        type Note { n: Int; }
        topic Fact { payload: Note; }
        locus Teller { params { x: Int = 0; } bus { publish Fact; } birth() { Fact <- Note { n: 7 }; } }
        main locus Root {
            params { t: Teller = Teller { }; got: Int = 0; }
            bus { subscribe Fact as on_fact; }
            fn on_fact(f: Note) { self.got = f.n; }
            run() { yield; }
        }
        fn main() { Root { }; }
    "#;
    assert!(!ir("parent_only", src).contains("@lotus_bus_reserve_begin"));
}

#[test]
fn reservations_bracket_the_params_and_close_before_the_roots_birth() {
    let text = ir("bracket", SIBLINGS);
    let at = |needle: &str| {
        text.lines()
            .position(|l| l.contains(needle))
            .unwrap_or_else(|| panic!("no `{needle}` in the IR"))
    };
    let begin = at("call ptr @lotus_bus_reserve_begin(");
    let reserve = at("call void @lotus_bus_reserve(");
    let first_claim = at("call void @lotus_bus_reserve_claim(");
    let end = at("call void @lotus_bus_reserve_end(");
    let teller_birth = at("call void @Teller.birth(");
    let root_birth = text
        .lines()
        .position(|l| l.contains("call void @Root.birth(") || l.contains("call void @Root.__birth"))
        .unwrap_or(usize::MAX);
    assert!(begin < reserve && reserve < first_claim, "begin {begin} reserve {reserve} claim {first_claim}");
    assert!(reserve < teller_birth, "the reservation is made before the publisher's birth");
    assert!(first_claim < end, "claims precede the close");
    assert!(root_birth == usize::MAX || end < root_birth, "the scope closes before the root's birth");
    // Three claimants of the one subject: `first`, `later`, `inner`.
    assert_eq!(text.matches("call void @lotus_bus_reserve_claim(").count(), 3);
}

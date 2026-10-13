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
//! anchor, the topic's bound, and a publisher on another thread
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
    assert!(stderr.contains("reservation `rsv.strict` closed: parked 3 copied 3 forwarded 0 shed 7"), "{stderr}");
    assert!(stderr.contains("reservation `rsv.loose` closed: parked 10 copied 4 forwarded 0 shed 6"), "{stderr}");
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
        params { count: Int = COUNT; }
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
    .replace("COUNT", &count.to_string())
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
    main locus Root { params { FIELDS } run() { } }
    fn main() { Root { }; }
    "#
    .replace("FIELDS", fields)
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

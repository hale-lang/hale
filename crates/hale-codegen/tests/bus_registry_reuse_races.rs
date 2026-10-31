//! The bus registry reuses a dissolved subscriber's entry (#1634). Two
//! races a reuse opens, each held open by a test hook that stalls one
//! thread inside the window, while main dissolves a subscriber and builds
//! the next, whose registration takes the dissolved one's entry:
//!
//! - a table walk that matched an entry's subject delivers only if the
//!   entry is still the registration it matched
//!   (`LOTUS_BUS_TEST_WALK_STALL_US`, between the match and the walk's
//!   version recheck): a keyed message for `alpha.t` never reaches the
//!   `beta.t` subscriber that took the entry meanwhile;
//! - a bounded subscriber's cells in flight are counted for the
//!   incarnation the cell belongs to (`LOTUS_BUS_TEST_ACCT_STALL_US`,
//!   between finding the bounded registration and charging the cell): a
//!   cell built for the dissolved subscriber never charges the one that
//!   took its entry, whose bound of one would otherwise shed every
//!   later delivery.
//!
//! Main spins without draining while it holds the first subscriber, and
//! returns with `return;`, so nothing on main takes the queue's lock
//! inside the window.

#![cfg(target_os = "linux")]

use std::process::Command;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

fn run(tag: &str, src: &str, knob: &str) -> String {
    let bin = harness::unique_bin(tag);
    build_opts::build_source(src, &bin, &build_opts::options()).expect("build");
    let out = Command::new(&bin).env(knob, "300000").env("LOTUS_BUS_TABLE_STATS", "1").output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(out.status.success(), "{tag}: {:?}\n{stdout}\n{stderr}", out.status);
    // the second subscriber took the first one's entry
    assert!(stderr.contains("[bus_table] entries=1 "), "{tag}: no reuse happened:\n{stderr}");
    stdout
}

const SUBJECT_RACE: &str = r#"
type Msg { k: Int; n: Int; }
topic AlphaT { payload: Msg; subject: "alpha.t"; keyed_by k; }
topic BetaT { payload: Msg; subject: "beta.t"; keyed_by k; }
locus Alpha {
    params { got: Int = 0; }
    bus { subscribe AlphaT as on_a where key == 1; }
    fn on_a(m: Msg) { self.got = self.got + 1; println("alpha got " + to_string(m.n)); }
}
locus Beta {
    params { got: Int = 0; }
    bus { subscribe BetaT as on_b where key == 1; }
    fn on_b(m: Msg) { self.got = self.got + 1; println("beta got " + to_string(m.n)); }
}
locus Pub {
    params { x: Int = 0; }
    bus { publish AlphaT; publish BetaT; }
    run() {
        std::time::sleep(50ms);
        AlphaT <- Msg { k: 1, n: 1 };
        std::time::sleep(300ms);
        BetaT <- Msg { k: 1, n: 2 };
    }
}
main locus App {
    params { p: Pub = Pub { }; t0: Duration = 0ns; }
    placement { p: cooperative(pool = pa); }
    fn hold() {
        let a = Alpha { };
        let mut go = true;
        while go { go = std::time::monotonic() - self.t0 < 150ms; }
        return;
    }
    run() {
        self.t0 = std::time::monotonic();
        self.hold();
        let b = Beta { };
        std::time::sleep(1500ms);
        println("beta saw " + to_string(b.got));
    }
}
fn main() { App { }; }
"#;

/// The walk for `alpha.t` matched the alpha subscriber's entry, then
/// stalled; meanwhile the entry became the beta subscriber's. The walk
/// skips it: the beta subscriber hears only its own message.
#[test]
fn a_walk_never_delivers_through_an_entry_reused_after_its_match() {
    let out = run("reuse_subject_race", SUBJECT_RACE, "LOTUS_BUS_TEST_WALK_STALL_US");
    assert_eq!(out.lines().collect::<Vec<_>>(), ["beta got 2", "beta saw 1"], "{out}");
}

const ACCOUNTING_RACE: &str = r#"
type Ev { n: Int; }
topic T { payload: Ev; subject: "acct.t"; }
locus Ear {
    params { got: Int = 0; }
    bus { subscribe T as on_e bounded(1, drop_new); }
    fn on_e(e: Ev) { self.got = self.got + 1; println("ear got " + to_string(e.n)); }
}
locus Pub {
    params { x: Int = 0; }
    bus { publish T; }
    run() {
        std::time::sleep(50ms);
        T <- Ev { n: 1 };
        std::time::sleep(100ms);
        T <- Ev { n: 2 };
        std::time::sleep(100ms);
        T <- Ev { n: 3 };
    }
}
main locus App {
    params { p: Pub = Pub { }; t0: Duration = 0ns; }
    placement { p: cooperative(pool = pa); }
    fn hold() {
        let a = Ear { };
        let mut go = true;
        while go { go = std::time::monotonic() - self.t0 < 150ms; }
        return;
    }
    run() {
        self.t0 = std::time::monotonic();
        self.hold();
        let b = Ear { };
        std::time::sleep(2000ms);
        println("second ear saw " + to_string(b.got));
    }
}
fn main() { App { }; }
"#;

/// The first publish found the first ear's bounded registration, then
/// stalled; meanwhile the entry became the second ear's, whose bound is
/// one. The stale cell charges nothing to the second ear (and is dropped
/// as the first ear's), so the second ear hears both later publishes.
#[test]
fn a_reused_entry_is_never_charged_for_the_previous_subscribers_cell() {
    let out = run("reuse_accounting_race", ACCOUNTING_RACE, "LOTUS_BUS_TEST_ACCT_STALL_US");
    // (a starved pool can publish the first only once the second ear is
    // up, which then hears it as well: that run proves nothing, and fails
    // nothing)
    let lines: Vec<&str> = out.lines().collect();
    assert!(lines.contains(&"ear got 2") && lines.contains(&"ear got 3"), "the second ear was shed:\n{out}");
    assert!(matches!(lines.last(), Some(&"second ear saw 2") | Some(&"second ear saw 3")), "{out}");
}

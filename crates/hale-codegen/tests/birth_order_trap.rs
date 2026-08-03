//! Params are born in declaration order, and a cooperative child's
//! `run()` runs INLINE during its own birth — so a child whose
//! `run()` does not return blocks every param declared after it from
//! ever being born.
//!
//! ## What this file used to claim, and why it was wrong
//!
//! This was `coop_child_handler_cadence.rs`: a downstream handoff
//! reported "a bus handler's write to a `self` param isn't observed
//! by `run()`", we correctly re-diagnosed it as ordering rather than
//! coherence, and then mis-filed it a second time as a *handler
//! cadence* question — "only the main locus is serviced from inside
//! its own `run()`" — with the reproducer `#[ignore]`d pending a
//! model decision.
//!
//! There was no model decision to make. The cooperative child's
//! sleep-slice drain services its handlers perfectly well. The
//! reproducer declared the SUBSCRIBER before the PUBLISHER, so the
//! publisher had not been born yet — its `run()` could not have
//! published, because it did not exist. Swap the two declarations
//! and the handler fires mid-loop exactly like the main locus's.
//! `cooperative_child_handler_fires_during_its_own_run` below is
//! that same reproducer with the declarations swapped, and it now
//! runs live instead of `#[ignore]`d.
//!
//! ## The real defect
//!
//! `later_params_are_not_born_until_a_child_run_returns` pins the
//! actual trap. With the usual `while true` keep-alive shape the
//! later params are never born AT ALL — their `birth()` never runs,
//! so subscriptions they register and sockets they bind silently
//! never exist, and the process looks like it booted and idled.
//!
//! Measured across all four placements (2026-08-03): default and
//! `cooperative(pool = main)` block; `cooperative(pool = io)`
//! (posted to a worker) and `pinned` (own thread) do not. The LATER
//! field's own placement is irrelevant — instantiation itself runs
//! inline on main, so even a `pinned` sibling declared after the
//! blocker is stuck. `check::check_cooperative_pool_blocking` warns
//! on the provable shape; see `birth_order_trap_diagnostic.rs`.

use std::process::Command;

#[path = "support/harness.rs"]
mod harness;

use hale_codegen::build_executable;

fn run_src(name: &str, src: &str) -> String {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(name);
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// Publisher `p` declared FIRST, so it is born (and pinned onto its
/// own thread) before the subscriber's loop starts.
const CHILD_SUBSCRIBER: &str = r#"
    type Req { v: Int; }
    topic R { payload: Req; subject: "cadence.r"; }
    locus Worker {
        params { pending: Int = 0; }
        bus { subscribe R as on_r; }
        fn on_r(r: Req) { println("HANDLER"); self.pending = r.v; }
        run() {
            let mut i = 0;
            while i < 6 {
                std::time::sleep(60ms);
                println("TICK ", self.pending);
                i = i + 1;
            }
        }
    }
    locus P {
        bus { publish R; }
        run() { std::time::sleep(120ms); R <- Req { v: 42 }; }
    }
    main locus App {
        params { p: P = P { }; w: Worker = Worker { }; }
        placement { p: pinned; }
    }
    fn main() { App { }; }
"#;

const MAIN_SUBSCRIBER: &str = r#"
    type Req { v: Int; }
    topic R { payload: Req; subject: "cadence.m"; }
    locus P {
        bus { publish R; }
        run() { std::time::sleep(120ms); R <- Req { v: 42 }; }
    }
    main locus App {
        params { pending: Int = 0; p: P = P { }; }
        placement { p: pinned; }
        bus { subscribe R as on_r; }
        fn on_r(r: Req) { println("HANDLER"); self.pending = r.v; }
        run() {
            let mut i = 0;
            while i < 6 {
                std::time::sleep(60ms);
                println("TICK ", self.pending);
                i = i + 1;
            }
        }
    }
    fn main() { App { }; }
"#;

/// A cooperative child's `run()` loop IS serviced from inside its own
/// sleep slices: the handler lands mid-loop and the loop observes the
/// write on the next tick. This is the test that was `#[ignore]`d as
/// a pending model decision; the only change is that the publisher is
/// declared before the subscriber so that it exists.
#[test]
fn cooperative_child_handler_fires_during_its_own_run() {
    let out = run_src("birth_order_child", CHILD_SUBSCRIBER);
    let lines: Vec<&str> = out.lines().collect();
    let handler = lines
        .iter()
        .position(|l| l.starts_with("HANDLER"))
        .unwrap_or_else(|| panic!("handler never ran:\n{}", out));
    assert!(
        handler < lines.len() - 1,
        "the handler must fire DURING the child's loop, not after it:\n{}",
        out
    );
    assert!(
        lines.iter().any(|l| l.trim() == "TICK 42"),
        "the child's own `run()` must observe its handler's write:\n{}",
        out
    );
}

/// The control: the main locus behaves identically. Kept so that a
/// regression in either one is attributable.
#[test]
fn main_locus_handler_fires_during_its_own_run() {
    let out = run_src("birth_order_main", MAIN_SUBSCRIBER);
    let lines: Vec<&str> = out.lines().collect();
    let handler = lines
        .iter()
        .position(|l| l.starts_with("HANDLER"))
        .unwrap_or_else(|| panic!("handler never ran:\n{}", out));
    assert!(
        handler < lines.len() - 1,
        "the handler must fire DURING the loop, not after it:\n{}",
        out
    );
    assert!(
        lines.iter().any(|l| l.trim() == "TICK 42"),
        "the loop must observe the handler's write:\n{}",
        out
    );
}

/// The actual trap: a child's `run()` runs inline during its own
/// birth, so a later sibling is not born until it returns. Uses a
/// BOUNDED loop so the test terminates — with the `while true`
/// keep-alive that real programs use, `LATER BORN` never prints at
/// all.
#[test]
fn later_params_are_not_born_until_a_child_run_returns() {
    const SRC: &str = r#"
        locus Blocker {
            run() {
                let mut i = 0;
                while i < 3 { std::time::sleep(40ms); i = i + 1; }
                println("BLOCKER done");
            }
        }
        locus Later { birth() { println("LATER BORN"); } }
        main locus App {
            params { f: Blocker = Blocker { }; l: Later = Later { }; }
        }
        fn main() { App { }; }
    "#;
    let out = run_src("birth_order_blocks", SRC);
    let lines: Vec<&str> = out.lines().map(|l| l.trim()).collect();
    let blocker = lines.iter().position(|l| *l == "BLOCKER done");
    let later = lines.iter().position(|l| *l == "LATER BORN");
    let (blocker, later) = match (blocker, later) {
        (Some(b), Some(l)) => (b, l),
        _ => panic!("expected both lines:\n{}", out),
    };
    assert!(
        blocker < later,
        "`Later` must not be born until `Blocker.run()` returns — that \
         is the trap this test exists to pin. If this ever reverses, \
         the birth-order model changed and the diagnostic in \
         check::check_cooperative_pool_blocking is now wrong:\n{}",
        out
    );
}

/// The blocker's placement is what matters, and moving it off the
/// main thread lifts the block. The later sibling's OWN placement
/// does not help (instantiation runs inline on main regardless) —
/// which is why the diagnostic only inspects the blocker.
#[test]
fn pinning_the_blocker_lets_later_params_be_born_first() {
    const SRC: &str = r#"
        locus Blocker {
            run() {
                let mut i = 0;
                while i < 3 { std::time::sleep(40ms); i = i + 1; }
                println("BLOCKER done");
            }
        }
        locus Later { birth() { println("LATER BORN"); } }
        main locus App {
            params { f: Blocker = Blocker { }; l: Later = Later { }; }
            placement { f: pinned; }
        }
        fn main() { App { }; }
    "#;
    let out = run_src("birth_order_pinned", SRC);
    let lines: Vec<&str> = out.lines().map(|l| l.trim()).collect();
    let blocker = lines
        .iter()
        .position(|l| *l == "BLOCKER done")
        .unwrap_or_else(|| panic!("blocker never finished:\n{}", out));
    let later = lines
        .iter()
        .position(|l| *l == "LATER BORN")
        .unwrap_or_else(|| panic!("later never born:\n{}", out));
    assert!(
        later < blocker,
        "with the blocker pinned to its own thread, `Later` must be \
         born while it still runs:\n{}",
        out
    );
}

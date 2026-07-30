//! When does a cooperative CHILD's bus handler run relative to its
//! own `run()` loop?
//!
//! fathom (FRICTION) reported this as "a bus handler's write to a
//! `self` param isn't observed by `run()`" and read it as a memory
//! visibility problem between the handler context and the loop.
//!
//! It is not. Instrumenting both sides shows the write is perfectly
//! visible — the handler simply does not run until `run()` has
//! finished. Ordering, not coherence. The distinction matters because
//! the two have completely different fixes, and "add a barrier" would
//! have chased nothing.
//!
//! ## The shape that works, and the one that doesn't
//!
//! Subscriber is the MAIN locus, sleeping in its own `run()`:
//! the handler fires mid-loop and the loop observes the write
//! immediately. `main_locus_handler_fires_during_its_own_run` pins
//! this, and it is what makes the case below a real asymmetry rather
//! than "the bus is just asynchronous".
//!
//! Subscriber is a cooperative CHILD on pool `main`, same sleeps,
//! same publisher: the handler does not fire until the child's
//! `run()` returns. Publisher placement is irrelevant — pinned and
//! `cooperative(pool = io)` behave identically.
//!
//! ## Why it is `#[ignore]`d rather than fixed here
//!
//! Whether this is a defect depends on a model decision the
//! substrate has not made explicitly. A cooperative locus is
//! serviced between its scheduler turns; a `run()` that loops holds
//! its turn, so its own handlers wait. That is consistent with the
//! documented cooperative-pool starvation model — but `sleep`'s
//! per-slice drain deliberately services the main-pool queue
//! ~10×/s, which sets up the expectation that sleeping yields, and
//! for the main locus it does.
//!
//! So either sleep's drain should reach a cooperative child's
//! handlers too, or the asymmetry needs documenting as a rule
//! ("only the main locus is serviced from inside its own `run()`").
//! Running as a live test would assert one answer before it has been
//! chosen; `--ignored` keeps the reproducer exact and runnable.

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
        params { w: Worker = Worker { }; p: P = P { }; }
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

/// The control, and it must keep passing: the main locus IS serviced
/// from inside its own `run()`, so the handler lands mid-loop and the
/// loop sees the write on the next tick.
#[test]
fn main_locus_handler_fires_during_its_own_run() {
    let out = run_src("cadence_main", MAIN_SUBSCRIBER);
    let lines: Vec<&str> = out.lines().collect();
    let handler = lines.iter().position(|l| l.starts_with("HANDLER"));
    let handler = handler.unwrap_or_else(|| panic!("handler never ran:\n{}", out));
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

/// The asymmetry. Ignored because the intended model is undecided —
/// see the module docs.
#[test]
#[ignore = "cooperative-child handler cadence: model decision pending"]
fn cooperative_child_handler_fires_during_its_own_run() {
    let out = run_src("cadence_child", CHILD_SUBSCRIBER);
    assert!(
        out.lines().any(|l| l.trim() == "TICK 42"),
        "a cooperative child's own `run()` never observes its handler's \
         write, because the handler does not run until the loop \
         returns. The write itself is fine — the ordering is the \
         issue, and the main-locus control proves the asymmetry:\n{}",
        out
    );
}

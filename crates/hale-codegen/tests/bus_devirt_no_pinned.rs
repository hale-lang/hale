//! Static-devirt #3 (static-pinned) — the no_pinned predicate must be
//! the EXACT negation of codegen's `lotus_bus_mark_pinned` condition.
//!
//! Regression for the merge-blocker race: `no_pinned` was derived from
//! a placement-only probe that missed the inline-adapter `bindings {}`
//! term of `lotus_bus_mark_pinned`. A program with an adapter binding
//! (→ a transport recv-thread that enqueues to the cooperative queue
//! using the LOCKED enqueue, because mark_pinned fired) PLUS a separate
//! plain local eligible subject would then dispatch that subject with
//! `no_pinned=1` → the unlocked `lotus_bus_queue_enqueue_st` on the
//! SAME queue. Unlocked-vs-locked concurrent enqueue = data race, and
//! a timing race is invisible to the stdout differential.
//!
//! The fix routes BOTH the mark_pinned emission and the no_pinned flag
//! through one `program_has_offthread` value, so `no_pinned ==
//! !program_has_offthread` by construction. This test pins that down at
//! the IR level: the eligible subject in an adapter-bearing program is
//! dispatched with `no_pinned=0`; in an otherwise-identical program
//! WITHOUT the adapter it is `no_pinned=1`.
//!
//! The IR is dumped via `LOTUS_DUMP_IR=1` BEFORE the optimization
//! pipeline runs, so the `lotus_bus_dispatch_static` call carries the
//! flag as a literal i32 constant. Single-threaded test (env toggles).

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use hale_codegen::build_executable;

fn unique_path(tag: &str, ext: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut p = std::env::temp_dir();
    p.push(format!("lt-nopin-{}-{}-{}.{}", tag, std::process::id(), nanos, ext));
    p
}

/// Compile `src` with the IR dumped, return the `.ll` text.
fn build_ir(tag: &str, src: &str) -> String {
    let bin = unique_path(tag, "bin");
    let ir = bin.with_extension("ll");
    let program = hale_syntax::parse_source(src).expect("parse");
    // SAFETY: single test thread; no concurrent env mutation.
    std::env::set_var("LOTUS_DUMP_IR", "1");
    let result = build_executable(&program, &bin);
    std::env::remove_var("LOTUS_DUMP_IR");
    result.expect("build");
    let text = std::fs::read_to_string(&ir).expect("read IR");
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(&ir);
    text
}

/// Parse the `no_pinned` (final i32) argument of every
/// `call void @lotus_bus_dispatch_static(...)` site in the IR. Each
/// call ends `..., i32 <flat>, i32 <no_pinned>)`.
fn no_pinned_args(ir: &str) -> Vec<i64> {
    ir.lines()
        .filter(|l| l.contains("call void @lotus_bus_dispatch_static("))
        .map(|l| {
            let args = l.rsplit_once('(').expect("call args").1;
            let last = args
                .rsplit_once("i32 ")
                .expect("final i32 arg")
                .1
                .trim_end()
                .trim_end_matches(')')
                .trim();
            last.parse::<i64>().unwrap_or_else(|_| {
                panic!("could not parse no_pinned from line: {l}")
            })
        })
        .collect()
}

/// Eligible local subject `local`, no off-thread anything → single-
/// threaded bus → no_pinned MUST be 1.
const CONTROL_SINGLE_THREADED: &str = r#"
type Local { v: Int; }
locus Sink {
    bus { subscribe "local" as on_local of type Local; }
    fn on_local(l: Local) { println("local ", l.v); }
}
main locus App {
    bus { publish "local" of type Local; }
    run() {
        Sink { };
        "local" <- Local { v: 1 };
    }
}
fn main() { App { }; }
"#;

/// SAME eligible local subject `local`, but an inline adapter
/// `bindings { }` on a DIFFERENT subject makes the runtime set
/// g_bus_has_pinned (mark_pinned fires) → a transport recv-thread can
/// enqueue concurrently → no_pinned MUST be 0 (locked enqueue).
const ADAPTER_PLUS_ELIGIBLE: &str = r#"
type Ext { n: Int; }
type Local { v: Int; }
topic ExtTopic { payload: Ext; subject: "ext"; }
locus Adapter { fn send(subject: String, bytes: Bytes) { } }
locus Sink {
    bus { subscribe "local" as on_local of type Local; }
    fn on_local(l: Local) { println("local ", l.v); }
}
main locus App {
    bindings { ExtTopic: Adapter { }; }
    bus { publish "local" of type Local; }
    run() {
        Sink { };
        "local" <- Local { v: 1 };
    }
}
fn main() { App { }; }
"#;

#[test]
fn single_threaded_program_uses_no_pinned_1() {
    let ir = build_ir("ctl", CONTROL_SINGLE_THREADED);
    let flags = no_pinned_args(&ir);
    assert!(
        !flags.is_empty(),
        "expected an eligible static dispatch in the control program"
    );
    assert!(
        flags.iter().all(|&f| f == 1),
        "single-threaded program must dispatch with no_pinned=1; got {flags:?}"
    );
}

#[test]
fn adapter_binding_forces_no_pinned_0_on_eligible_subject() {
    // The fix: the adapter `bindings {}` term of mark_pinned is now
    // reflected in no_pinned, so the eligible `local` subject — which
    // would otherwise look single-threaded by placement — is dispatched
    // with the LOCKED enqueue (no_pinned=0). This is the race fix.
    let ir = build_ir("adapter", ADAPTER_PLUS_ELIGIBLE);
    let flags = no_pinned_args(&ir);
    assert!(
        !flags.is_empty(),
        "expected an eligible static dispatch for `local` in the \
         adapter-bearing program"
    );
    assert!(
        flags.iter().all(|&f| f == 0),
        "an adapter binding makes the bus multithreaded — the eligible \
         subject MUST be dispatched with no_pinned=0 (locked enqueue), \
         else it races the adapter recv-thread on the cooperative queue; \
         got {flags:?}"
    );
}

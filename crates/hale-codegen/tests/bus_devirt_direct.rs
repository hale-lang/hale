//! Direct-call devirtualization (build #1b slice-2) — IR-level gate.
//!
//! The differential harness (`bus_devirt_differential.rs`) proves the
//! direct-call lowering is BEHAVIORALLY identical to the all-dynamic
//! one. This test is the complementary positive/negative check that the
//! CLASSIFIER actually engages: a quiet + flat + same-thread subject
//! lowers to `lotus_bus_dispatch_static_direct`, while any handler with
//! an effect (a `println` call, a republish, a helper call) or a
//! non-flat payload or an off-thread subscriber stays on the deferred
//! `lotus_bus_dispatch_static` enqueue.
//!
//! The IR is dumped via `LOTUS_DUMP_IR=1` BEFORE the optimization
//! pipeline, so each publish site is a literal call to one of the two
//! runtime entry points. Single-threaded test (env toggle), repo
//! default `--test-threads=1`.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use hale_codegen::build_executable;

fn unique_path(tag: &str, ext: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut p = std::env::temp_dir();
    p.push(format!("lt-direct-{}-{}-{}.{}", tag, std::process::id(), nanos, ext));
    p
}

fn build_ir(tag: &str, src: &str) -> String {
    let bin = unique_path(tag, "bin");
    let ir = bin.with_extension("ll");
    let program = hale_syntax::parse_source(src).expect("parse");
    // SAFETY: single test thread; no concurrent env mutation.
    std::env::set_var("LOTUS_DUMP_IR", "1");
    std::env::remove_var("LOTUS_NO_BUS_DEVIRT");
    let result = build_executable(&program, &bin);
    std::env::remove_var("LOTUS_DUMP_IR");
    result.expect("build");
    let text = std::fs::read_to_string(&ir).expect("read IR");
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(&ir);
    text
}

/// Slice-3 INLINE form: a single-subscriber-handler direct subject
/// walks the bucket itself via the `pure` accessors and bakes the
/// handler as a DIRECT call. The count-accessor call is the
/// unambiguous signature of the inline lowering.
fn calls_inline_direct(ir: &str) -> bool {
    ir.contains("call i64 @lotus_bus_static_direct_count(")
}
/// Slice-2 HELPER form: the non-inlined `lotus_bus_dispatch_static_direct`
/// runtime helper — kept for MULTI-distinct-handler direct subjects,
/// which can't bake one constant.
fn calls_direct_helper(ir: &str) -> bool {
    ir.contains("call void @lotus_bus_dispatch_static_direct(")
}
fn calls_deferred_static(ir: &str) -> bool {
    // The non-`_direct` static enqueue. Match the exact `(` so the
    // `_direct` suffix doesn't false-positive this probe.
    ir.contains("call void @lotus_bus_dispatch_static(")
}

/// QUIET handler (self-field accumulation + pure-cond if), FLAT Int
/// payload, same-thread → MUST direct-call, NOT deferred-enqueue.
const QUIET_FLAT: &str = r#"
type Sample { value: Int; }
locus Aggregator {
    params { count: Int = 0; sum: Int = 0; min_v: Int = 999; }
    bus { subscribe "s" as on_s of type Sample; }
    fn on_s(s: Sample) {
        self.count = self.count + 1;
        self.sum = self.sum + s.value;
        if s.value < self.min_v { self.min_v = s.value; }
    }
}
locus Producer {
    bus { publish "s" of type Sample; }
    run() { "s" <- Sample { value: 7 }; }
}
fn main() { let a = Aggregator { }; Producer { }; print("c="); println(a.count); }
"#;

/// PRINTING handler (a `println` CALL) → non-quiet → deferred enqueue,
/// NEVER direct.
const PRINTING_HANDLER: &str = r#"
type Tick { n: Int; }
locus Logger {
    bus { subscribe "t" as on_t of type Tick; }
    fn on_t(t: Tick) { println(t.n); }
}
locus Producer {
    bus { publish "t" of type Tick; }
    run() { "t" <- Tick { n: 1 }; }
}
fn main() { Logger { }; Producer { }; }
"#;

/// QUIET handler but NON-FLAT (String) payload → flat leg fails →
/// deferred static enqueue (managed-payload wire path), NEVER direct.
const QUIET_NONFLAT: &str = r#"
type Msg { text: String; len: Int; }
locus Keeper {
    params { last_len: Int = 0; }
    bus { subscribe "m" as on_m of type Msg; }
    fn on_m(m: Msg) { self.last_len = m.len; }
}
locus Producer {
    bus { publish "m" of type Msg; }
    run() { "m" <- Msg { text: "hi", len: 2 }; }
}
fn main() { let k = Keeper { }; Producer { }; print("l="); println(k.last_len); }
"#;

/// QUIET + FLAT but the subscriber is CROSS-POOL (off-thread) → can't
/// direct-call (must enqueue to the pool worker) → deferred static.
const QUIET_FLAT_CROSS_POOL: &str = r#"
type Tick { n: Int; }
locus Sub {
    params { total: Int = 0; }
    bus { subscribe "tk" as on_tk of type Tick; }
    fn on_tk(t: Tick) { self.total = self.total + t.n; }
    run() { }
}
main locus App {
    params { s: Sub = Sub { }; }
    placement { s: cooperative(pool = io); }
    bus { publish "tk" of type Tick; }
    run() {
        std::time::sleep(10ms);
        "tk" <- Tick { n: 1 };
        std::time::sleep(20ms);
    }
}
fn main() { App { }; }
"#;

/// MULTI-distinct-handler quiet+flat+same-thread subject: two DIFFERENT
/// subscriber locus-types subscribe one subject, so the publish site
/// cannot bake a single constant handler → it keeps the slice-2
/// `lotus_bus_dispatch_static_direct` HELPER (which dispatches each
/// `e->handler` indirectly), NOT the inline baked call.
const MULTI_HANDLER_QUIET_FLAT: &str = r#"
type Ev { v: Int; }
locus SinkA {
    params { sum: Int = 0; }
    bus { subscribe "ev" as on_ev of type Ev; }
    fn on_ev(e: Ev) { self.sum = self.sum + e.v; }
}
locus SinkB {
    params { total: Int = 0; }
    bus { subscribe "ev" as on_ev of type Ev; }
    fn on_ev(e: Ev) { self.total = self.total + e.v; }
}
locus Producer {
    bus { publish "ev" of type Ev; }
    run() { "ev" <- Ev { v: 5 }; }
}
fn main() {
    let a = SinkA { };
    let b = SinkB { };
    Producer { };
    print("a="); println(a.sum);
    print("b="); println(b.total);
}
"#;

#[test]
fn quiet_flat_same_thread_lowers_to_inline_direct_call() {
    let ir = build_ir("quiet", QUIET_FLAT);
    assert!(
        calls_inline_direct(&ir),
        "a quiet + flat + same-thread SINGLE-handler subject must lower to \
         the INLINE baked direct call (walks the bucket via \
         lotus_bus_static_direct_count/_selfptr)"
    );
    assert!(
        !calls_direct_helper(&ir),
        "the single-handler case must NOT use the lotus_bus_dispatch_static_direct \
         helper — it bakes the handler inline instead"
    );
    assert!(
        !calls_deferred_static(&ir),
        "the quiet subject must NOT also take the deferred static \
         enqueue path"
    );
}

#[test]
fn multi_handler_direct_keeps_helper() {
    let ir = build_ir("multi", MULTI_HANDLER_QUIET_FLAT);
    assert!(
        calls_direct_helper(&ir),
        "a direct-call-eligible subject with TWO distinct subscriber \
         handlers can't bake one constant — it must keep the \
         lotus_bus_dispatch_static_direct helper"
    );
    assert!(
        !calls_inline_direct(&ir),
        "the multi-distinct-handler case must NOT emit the inline baked \
         walk (no single handler to bake)"
    );
}

#[test]
fn printing_handler_stays_deferred() {
    let ir = build_ir("printing", PRINTING_HANDLER);
    assert!(
        !calls_inline_direct(&ir) && !calls_direct_helper(&ir),
        "a handler that calls println is NOT quiet — it must never \
         direct-call (that would run I/O during the publish loop)"
    );
    assert!(
        calls_deferred_static(&ir),
        "the printing-handler subject is still #1b-eligible, so it must \
         take the deferred static enqueue"
    );
}

#[test]
fn non_flat_payload_stays_deferred() {
    let ir = build_ir("nonflat", QUIET_NONFLAT);
    assert!(
        !calls_inline_direct(&ir) && !calls_direct_helper(&ir),
        "a non-flat (String-bearing) payload must not direct-call — \
         managed payloads keep the wire / per-subscriber-arena path"
    );
    assert!(
        calls_deferred_static(&ir),
        "the quiet-but-non-flat subject is #1b-eligible → deferred static"
    );
}

#[test]
fn cross_pool_subscriber_stays_deferred() {
    let ir = build_ir("crosspool", QUIET_FLAT_CROSS_POOL);
    assert!(
        !calls_inline_direct(&ir) && !calls_direct_helper(&ir),
        "an off-thread (cross-pool) subscriber cannot be direct-called \
         — it must enqueue to the pool worker"
    );
}

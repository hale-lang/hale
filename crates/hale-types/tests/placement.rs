//! F.31 (2026-05-23) — placement block typecheck rules.
//!
//! The parser already enforces "main-only" and Ident keying.
//! Typecheck-side validation adds:
//!   1. Field exists in this locus's params block.
//!   2. Field type is a locus type.
//!   3. No duplicate field keys across placement entries.
//! Pinned-class restrictions (no accept(), no closures on
//! placed-pinned loci) move to codegen-time in Phase 3.

use hale_syntax::parse_source;
use hale_types::check_program;

fn check(src: &str) -> Vec<String> {
    let prog = parse_source(src).expect("parse failed");
    check_program(&prog)
        .into_iter()
        .map(|d| d.message)
        .collect()
}

#[test]
fn canonical_placement_typechecks_clean() {
    let src = r#"
locus Worker { run() { } }

main locus App {
    params {
        w: Worker = Worker { };
    }
    placement {
        w: pinned;
    }
}

fn main() { App { }; }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().all(|m| !m.contains("placement")),
        "expected clean placement typecheck, got: {:?}",
        msgs
    );
}

#[test]
fn placement_with_unknown_field_rejected() {
    let src = r#"
locus Worker { run() { } }

main locus App {
    params {
        w: Worker = Worker { };
    }
    placement {
        missing: pinned;
    }
}

fn main() { App { }; }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().any(|m|
            m.contains("placement") && m.contains("missing")
            && m.contains("params")),
        "expected diagnostic about unknown field `missing`, got: {:?}",
        msgs
    );
}

#[test]
fn placement_on_non_locus_field_rejected() {
    let src = r#"
main locus App {
    params {
        n: Int = 0;
    }
    placement {
        n: pinned;
    }
}

fn main() { App { }; }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().any(|m|
            m.contains("placement") && m.contains("not a locus type")),
        "expected diagnostic about non-locus type, got: {:?}",
        msgs
    );
}

#[test]
fn placement_duplicate_field_rejected() {
    let src = r#"
locus Worker { run() { } }

main locus App {
    params {
        w: Worker = Worker { };
    }
    placement {
        w: pinned;
        w: cooperative;
    }
}

fn main() { App { }; }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().any(|m|
            m.contains("duplicate") && m.contains("w")),
        "expected diagnostic about duplicate field, got: {:?}",
        msgs
    );
}

#[test]
fn placement_two_siblings_distinct_placements_clean() {
    let src = r#"
locus Worker { run() { } }

main locus App {
    params {
        a: Worker = Worker { };
        b: Worker = Worker { };
    }
    placement {
        a: pinned(core = 1);
        b: pinned(core = 2);
    }
}

fn main() { App { }; }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().all(|m| !m.contains("placement")),
        "expected two-sibling placement to typecheck clean, got: {:?}",
        msgs
    );
}

#[test]
fn placement_cooperative_with_pool_clean() {
    let src = r#"
locus Worker { run() { } }

main locus App {
    params {
        w: Worker = Worker { };
    }
    placement {
        w: cooperative(pool = io);
    }
}

fn main() { App { }; }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().all(|m| !m.contains("placement")),
        "expected cooperative-with-pool placement to typecheck clean, got: {:?}",
        msgs
    );
}

#[test]
fn placement_unspecified_field_uses_default() {
    // A locus without a placement entry doesn't need one; it
    // defaults to cooperative(pool = main) at codegen time.
    // Typecheck should not require placement coverage.
    let src = r#"
locus Worker { run() { } }

main locus App {
    params {
        a: Worker = Worker { };
        b: Worker = Worker { };
    }
    placement {
        a: pinned;
        // b deliberately not mentioned
    }
}

fn main() { App { }; }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().all(|m| !m.contains("placement")),
        "expected partial placement coverage to typecheck clean, got: {:?}",
        msgs
    );
}

// ---- F.31 Phase 5: single-threaded-method invariant ----

#[test]
fn cross_pool_self_field_call_rejected() {
    // `self.db.query()` invoked from main locus's body. main is
    // on `cooperative(main)` by default; `db` is placed pinned,
    // so it owns its own thread. The direct call crosses pools
    // and must be rejected.
    let src = r#"
locus DB {
    fn query() { }
}

main locus App {
    params {
        db: DB = DB { };
    }
    placement {
        db: pinned;
    }
    run() {
        self.db.query();
    }
}

fn main() { App { }; }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().any(|m| m.contains("cross-pool method call")),
        "expected cross-pool diagnostic, got: {:?}",
        msgs
    );
}

#[test]
fn same_pool_self_field_call_accepted() {
    // Both main (App) and `db` are on the default `cooperative(main)`
    // pool — App declares no placement entry for db, so it inherits.
    // The direct call is intra-pool and must typecheck clean.
    let src = r#"
locus DB {
    fn query() { }
}

main locus App {
    params {
        db: DB = DB { };
    }
    run() {
        self.db.query();
    }
}

fn main() { App { }; }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().all(|m| !m.contains("cross-pool")),
        "expected same-pool call to typecheck clean, got: {:?}",
        msgs
    );
}

#[test]
fn different_named_cooperative_pools_rejected() {
    // App on default `cooperative(main)`, db on
    // `cooperative(pool = io)`. Different named pools → different
    // OS threads under M:N scheduling → cross-pool call.
    let src = r#"
locus DB {
    fn query() { }
}

main locus App {
    params {
        db: DB = DB { };
    }
    placement {
        db: cooperative(pool = io);
    }
    run() {
        self.db.query();
    }
}

fn main() { App { }; }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().any(|m| m.contains("cross-pool method call")),
        "expected cross-pool diagnostic between named pools, got: {:?}",
        msgs
    );
}

#[test]
fn bus_send_does_not_trigger_cross_pool_check() {
    // `"subject" <- value;` is the legal cross-pool path. It must
    // not trigger a cross-pool diagnostic — bus dispatch handles
    // the boundary.
    let src = r#"
type Ping { n: Int; }

topic tick { payload: Ping; }

locus DB {
    bus { subscribe "tick" as on_tick of type Ping; }
    fn on_tick(p: Ping) { }
}

main locus App {
    params {
        db: DB = DB { };
    }
    placement {
        db: pinned;
    }
    bus { publish "tick" of type Ping; }
    run() {
        "tick" <- Ping { n: 1 };
    }
}

fn main() { App { }; }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().all(|m| !m.contains("cross-pool")),
        "expected bus send to be exempt from cross-pool check, got: {:?}",
        msgs
    );
}

#[test]
fn cross_pool_call_on_plain_form_locus_rejected_with_upgrade_hint() {
    // F.32-0 (2026-05-24): plain `@form(...)` loci no longer
    // get the cross-pool exemption. The 3ec6391 first-cut
    // assumed the form ABI serialized cell access; bench-prep
    // for F.32-1 surfaced that the runtime has no
    // synchronization on `lotus_hashmap_set` / `_grow` and
    // concurrent writers double-free during grow. F.32-0
    // restores single-pool default for plain `@form(...)`;
    // the diagnostic now points authors at the upgrade path
    // (`sync = serialized` or `sync = striped`, lands in
    // F.32-1α/β).
    //
    // Pre-F.32-0 behavior (kept for history): the diagnostic
    // was skipped for any `@form(...)` receiver. Test was
    // named `cross_pool_call_on_form_bearing_locus_accepted`.
    let src = r#"
type Counter { name: String; v: Int = 0; }

@form(hashmap)
locus Registry {
    capacity { pool counters of Counter indexed_by name; }
    fn render() { }
}

main locus App {
    params {
        registry: Registry = Registry { };
    }
    placement {
        registry: pinned;
    }
    run() {
        self.registry.render();
    }
}

fn main() { App { }; }
"#;
    let msgs = check(src);
    let cross_pool: Vec<_> = msgs.iter()
        .filter(|m| m.contains("cross-pool"))
        .collect();
    assert_eq!(
        cross_pool.len(),
        1,
        "expected exactly one cross-pool diagnostic; got msgs: {:?}",
        msgs
    );
    let msg = cross_pool[0];
    assert!(
        msg.contains("self.registry.render"),
        "diagnostic should name the offending call site: {}",
        msg
    );
    assert!(
        msg.contains("`Registry` is `@form(...)`"),
        "diagnostic should flag the receiver as form-bearing: {}",
        msg
    );
    assert!(
        msg.contains("sync = serialized") && msg.contains("sync = striped"),
        "upgrade hint should name both serialized and striped: {}",
        msg
    );
    assert!(
        msg.contains("F.32"),
        "upgrade hint should reference the F.32 delivery plan: {}",
        msg
    );
}

// F.32-1∞ (2026-05-25): when the offending cross-pool call IS
// one of the synthesized hashmap methods, the diagnostic should
// substitute the inference-specific hint (naming the picked
// discipline + the observed writer/reader pools) for the
// generic "choose serialized or striped" hint.

#[test]
fn cross_pool_set_call_carries_inferred_sync_hint() {
    // Two pools (io + compute) each fire `self.reg.set(...)`
    // inside a bus handler (`on_tick`). Inference: 2 writer
    // pools, hot-path (in `on_*` handlers) → striped.
    let src = r#"
type Entry { k: Int; v: Int; }
type Tick { n: Int; }

@form(hashmap)
locus Registry {
    capacity { pool entries of Entry indexed_by k; }
}

locus IoWorker {
    params { reg: Registry = Registry { }; }
    bus { subscribe "tick" as on_tick of type Tick; }
    fn on_tick(t: Tick) {
        self.reg.set(Entry { k: t.n, v: 1 });
    }
}

locus CompWorker {
    params { reg: Registry = Registry { }; }
    bus { subscribe "tick" as on_tick of type Tick; }
    fn on_tick(t: Tick) {
        self.reg.set(Entry { k: t.n, v: 2 });
    }
}

main locus App {
    params {
        io: IoWorker = IoWorker { };
        cpu: CompWorker = CompWorker { };
    }
    placement {
        io: cooperative(pool = io);
        cpu: cooperative(pool = compute);
    }
    bus { publish "tick" of type Tick; }
    run() { }
}

fn main() { App { }; }
"#;
    let msgs = check(src);
    let cross_pool: Vec<_> =
        msgs.iter().filter(|m| m.contains("cross-pool")).collect();
    assert!(
        !cross_pool.is_empty(),
        "expected cross-pool diagnostic; got: {:?}",
        msgs
    );
    let msg = cross_pool[0];
    assert!(
        msg.contains("inferred sync (F.32-1∞)"),
        "diagnostic should carry the inferred-sync banner: {}",
        msg
    );
    assert!(
        msg.contains("sync = striped"),
        "inference should pick striped (2 writer pools, hot-path): {}",
        msg
    );
    assert!(
        msg.contains("hot-path: yes"),
        "hot-path detection should fire (call inside on_tick): {}",
        msg
    );
    // Should NOT carry the generic "choose serialized or striped"
    // fallback hint — the inference picked one, the diagnostic
    // names it specifically.
    assert!(
        !msg.contains("sync = serialized")
            || msg.contains("`sync = striped` for `Registry`"),
        "diagnostic should not fall back to the generic both-options hint: {}",
        msg
    );
}

#[test]
fn cross_pool_set_call_one_writer_picks_serialized() {
    // 1 writer pool (io), 2 reader pools (io, compute) → the
    // 2-vs-1 rule fires `serialized`. Concrete shape: each
    // worker holds its own `reg` field; pool propagation
    // first-wins puts Registry on `io` (from IoWriter).
    // CompReader on `compute` calling `self.reg.has(...)` is
    // the cross-pool call site; the diagnostic includes the
    // inference banner.
    let src = r#"
type Entry { k: Int; v: Int; }
type Tick { n: Int; }

@form(hashmap)
locus Registry {
    capacity { pool entries of Entry indexed_by k; }
}

locus IoWriter {
    params { reg: Registry = Registry { }; }
    bus { subscribe "tick" as on_tick of type Tick; }
    fn on_tick(t: Tick) {
        self.reg.set(Entry { k: t.n, v: 1 });
        let _ = self.reg.has(t.n);
    }
}

locus CompReader {
    params { reg: Registry = Registry { }; }
    bus { subscribe "tick" as on_tick of type Tick; }
    fn on_tick(t: Tick) {
        let _ = self.reg.has(t.n);
    }
}

main locus App {
    params {
        io: IoWriter = IoWriter { };
        cpu: CompReader = CompReader { };
    }
    placement {
        io: cooperative(pool = io);
        cpu: cooperative(pool = compute);
    }
    bus { publish "tick" of type Tick; }
    run() { }
}

fn main() { App { }; }
"#;
    let msgs = check(src);
    let cross_pool: Vec<_> =
        msgs.iter().filter(|m| m.contains("cross-pool")).collect();
    assert!(
        !cross_pool.is_empty(),
        "expected cross-pool diagnostic; got: {:?}",
        msgs
    );
    let combined = cross_pool
        .iter()
        .map(|m| m.as_str())
        .collect::<Vec<&str>>()
        .join("\n");
    assert!(
        combined.contains("sync = serialized"),
        "inference should pick serialized (1 writer, multi reader): {}",
        combined
    );
}

// ---------------------------------------------------------------
// Dead bus receiver (fathom handoff 2026-06-02): a locus that
// subscribes to the bus but is placed cooperative on a non-main
// pool never receives a cell — only main-cooperative or pinned
// loci get delivery. The toolchain used to accept it silently.
// These lock in the diagnostic and its precision (no false
// positives on pinned / main / non-subscribing / intra-locus).
// ---------------------------------------------------------------

// Corrected 2026-06-03 (fathom over-fire handoff): a non-main
// cooperative subscriber is a dead receiver only when its run() ALSO
// makes a blocking call that starves the pool thread. Placement alone
// over-fired on event-driven subscribers (PriceView/WsDispatcher),
// which receive fine. The error message no longer claims "will never
// fire" flatly.
const DEAD_RX: &str = "monopolizes the pool's thread";

fn dead_receiver_src(placement_spec: &str, run_body: &str) -> String {
    format!(
        r#"
type Tick {{ n: Int; }}

locus Gateway {{
    bus {{ subscribe "tick" as on_tick of type Tick; }}
    fn on_tick(t: Tick) {{ }}
    run() {{ {run_body} }}
}}

locus Feed {{
    bus {{ publish "tick" of type Tick; }}
    run() {{ "tick" <- Tick {{ n: 1 }}; }}
}}

main locus App {{
    params {{
        gw: Gateway = Gateway {{ }};
        feed: Feed  = Feed {{ }};
    }}
    placement {{
        gw: {placement_spec};
    }}
}}

fn main() {{ App {{ }}; }}
"#
    )
}

const BLOCKING_RUN: &str = "let n = std::io::tls::recv_into(0, 0, 64);";

#[test]
fn cooperative_nonmain_subscriber_blocking_rejected() {
    // The gateway shape: non-main cooperative subscriber whose run()
    // blocks — still rejected (its blocking call starves the dispatch).
    let msgs = check(&dead_receiver_src("cooperative(pool = ws)", BLOCKING_RUN));
    assert!(
        msgs.iter().any(|m| m.contains(DEAD_RX)),
        "a non-main cooperative subscriber with a blocking run() is a dead \
         receiver and must be rejected; got: {:?}",
        msgs
    );
}

#[test]
fn cooperative_nonmain_subscriber_event_driven_compiles() {
    // The PriceView shape: non-main cooperative subscriber that does
    // NOT block (handlers + a sleep loop) — receives fine, must NOT be
    // rejected. This is the over-fire the correction fixes.
    let msgs = check(&dead_receiver_src(
        "cooperative(pool = prices)",
        "std::time::sleep(60s);",
    ));
    assert!(
        !msgs.iter().any(|m| m.contains(DEAD_RX)),
        "an event-driven (non-blocking) non-main cooperative subscriber \
         receives fine and must not be rejected; got: {:?}",
        msgs
    );
}

#[test]
fn pinned_subscriber_not_rejected() {
    // Pinned owns its thread (+ mailbox) — never a dead receiver, even
    // with a blocking run().
    let msgs = check(&dead_receiver_src("pinned", BLOCKING_RUN));
    assert!(
        !msgs.iter().any(|m| m.contains(DEAD_RX)),
        "pinned subscribers receive bus cells (per-locus mailbox); must not \
         be flagged: {:?}",
        msgs
    );
}

#[test]
fn main_cooperative_subscriber_not_rejected() {
    // main-pool cooperative subscriber, even blocking, is not the
    // dead-receiver error (main's sliced sleep drains; at most a
    // blocking warning).
    let msgs = check(&dead_receiver_src("cooperative(pool = main)", BLOCKING_RUN));
    assert!(
        !msgs.iter().any(|m| m.contains(DEAD_RX)),
        "main-pool cooperative subscribers are not the dead-receiver error: {:?}",
        msgs
    );
}

#[test]
fn cooperative_nonmain_nonsubscriber_not_rejected() {
    // A non-main cooperative locus that does NOT subscribe is fine.
    let src = r#"
locus Worker { run() { } }

main locus App {
    params { w: Worker = Worker { }; }
    placement { w: cooperative(pool = compute); }
}

fn main() { App { }; }
"#;
    let msgs = check(src);
    assert!(
        !msgs.iter().any(|m| m.contains(DEAD_RX)),
        "a non-subscribing locus must not be flagged regardless of pool: {:?}",
        msgs
    );
}

#[test]
fn intra_locus_self_pub_sub_not_rejected() {
    // Publishes AND subscribes the same topic itself → the intra-locus
    // optimization devirtualizes it to a direct self.handler() call,
    // which delivers on any pool. Flagging it would be a false positive.
    let src = r#"
type Ev { n: Int; }

locus SelfLoop {
    bus {
        publish   "ev" of type Ev;
        subscribe "ev" as on_ev of type Ev;
    }
    fn on_ev(e: Ev) { }
    run() { "ev" <- Ev { n: 1 }; }
}

main locus App {
    params { s: SelfLoop = SelfLoop { }; }
    placement { s: cooperative(pool = ws); }
}

fn main() { App { }; }
"#;
    let msgs = check(src);
    assert!(
        !msgs.iter().any(|m| m.contains(DEAD_RX)),
        "intra-locus self-publish→self-subscribe is devirtualized and \
         delivers on any pool; must not be flagged: {:?}",
        msgs
    );
}

// ---------------------------------------------------------------
// Blocking-syscall-on-a-cooperative-pool (fathom handoff #2). A
// cooperative (non-async_io) locus that calls a known-blocking
// stdlib op in run() stalls co-scheduled loci. hale's first
// WARNING (non-fatal) — a single-purpose blocking server is
// legitimate, so it's surfaced, not rejected.
// ---------------------------------------------------------------

const BLOCKS_WARN: &str = "holds the pool's OS thread";

fn blocking_src(placement_spec: &str, run_body: &str) -> String {
    format!(
        r#"
locus Gateway {{
    run() {{ {run_body} }}
}}

main locus App {{
    params {{ gw: Gateway = Gateway {{ }}; }}
    placement {{ gw: {placement_spec}; }}
}}

fn main() {{ App {{ }}; }}
"#
    )
}

#[test]
fn cooperative_blocking_run_warns() {
    let msgs = check(&blocking_src(
        "cooperative(pool = ws)",
        "let n = std::io::tls::recv_into(0, 0, 64);",
    ));
    assert!(
        msgs.iter().any(|m| m.contains(BLOCKS_WARN)),
        "expected a blocking-on-cooperative-pool warning; got: {:?}",
        msgs
    );
}

#[test]
fn pinned_blocking_run_not_warned() {
    let msgs = check(&blocking_src(
        "pinned",
        "let n = std::io::tls::recv_into(0, 0, 64);",
    ));
    assert!(
        !msgs.iter().any(|m| m.contains(BLOCKS_WARN)),
        "pinned owns its own thread; blocking is fine: {:?}",
        msgs
    );
}

#[test]
fn async_io_blocking_run_not_warned() {
    let msgs = check(&blocking_src(
        "cooperative(pool = ws) where async_io",
        "let n = std::io::tls::recv_into(0, 0, 64);",
    ));
    assert!(
        !msgs.iter().any(|m| m.contains(BLOCKS_WARN)),
        "async_io parks on I/O readiness; must not warn: {:?}",
        msgs
    );
}

#[test]
fn cooperative_nonblocking_run_not_warned() {
    let msgs = check(&blocking_src("cooperative(pool = ws)", "let x = 1 + 1;"));
    assert!(
        !msgs.iter().any(|m| m.contains(BLOCKS_WARN)),
        "no blocking call in run(); must not warn: {:?}",
        msgs
    );
}

#[test]
fn blocking_inside_while_loop_warns() {
    // The blocking call is nested in `while true { ... }` — exercises
    // the full-recursion walk into loop bodies.
    let msgs = check(&blocking_src(
        "cooperative(pool = ws)",
        "while true { let n = std::io::tls::recv_into(0, 0, 64); }",
    ));
    assert!(
        msgs.iter().any(|m| m.contains(BLOCKS_WARN)),
        "a blocking call inside a loop in run() must warn: {:?}",
        msgs
    );
}

// ---------------------------------------------------------------
// Interprocedural deepening (2026-06-04): the warning now follows
// the call graph, so a run() that blocks through a helper fn or a
// `self.method` is flagged just like a literal blocking op. The
// dead-receiver ERROR deliberately stays direct-call-only.
// ---------------------------------------------------------------

#[test]
fn blocking_via_free_fn_helper_warns() {
    // run() itself has no stdlib blocking op — it calls a free fn
    // that does. The interprocedural walk must still warn.
    let src = r#"
fn pump(fd: Int) -> Int {
    return std::io::tcp::recv_into(fd, 0, 64);
}

locus Gateway {
    run() { let n = pump(0); }
}

main locus App {
    params { gw: Gateway = Gateway { }; }
    placement { gw: cooperative(pool = ws); }
}

fn main() { App { }; }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().any(|m| m.contains(BLOCKS_WARN) && m.contains("pump()")),
        "blocking reached through a free-fn helper must warn (naming the \
         helper); got: {:?}",
        msgs
    );
}

#[test]
fn blocking_via_self_method_warns() {
    // run() calls `self.pull()`, whose body blocks. The intra-locus
    // method call graph must propagate the block to run().
    let src = r#"
locus Gateway {
    fn pull() { let n = std::io::tcp::recv_into(0, 0, 64); }
    run() { self.pull(); }
}

main locus App {
    params { gw: Gateway = Gateway { }; }
    placement { gw: cooperative(pool = ws); }
}

fn main() { App { }; }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter()
            .any(|m| m.contains(BLOCKS_WARN) && m.contains("self.pull()")),
        "blocking reached through a self-method must warn; got: {:?}",
        msgs
    );
}

#[test]
fn blocking_via_transitive_free_fn_warns() {
    // run() -> outer() -> inner() (blocks). Two hops; the fixpoint
    // must taint `outer` from `inner`, then flag run()'s `outer()`.
    let src = r#"
fn inner(fd: Int) -> Int { return std::io::tcp::recv_into(fd, 0, 64); }
fn outer(fd: Int) -> Int { return inner(fd); }

locus Gateway {
    run() { let n = outer(0); }
}

main locus App {
    params { gw: Gateway = Gateway { }; }
    placement { gw: cooperative(pool = ws); }
}

fn main() { App { }; }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().any(|m| m.contains(BLOCKS_WARN) && m.contains("outer()")),
        "blocking two fn-hops deep must warn; got: {:?}",
        msgs
    );
}

#[test]
fn nonblocking_helper_not_warned() {
    // run() calls a helper that does NOT block — no false positive.
    let src = r#"
fn compute(x: Int) -> Int { return x + x; }

locus Gateway {
    run() { let n = compute(21); }
}

main locus App {
    params { gw: Gateway = Gateway { }; }
    placement { gw: cooperative(pool = ws); }
}

fn main() { App { }; }
"#;
    let msgs = check(src);
    assert!(
        !msgs.iter().any(|m| m.contains(BLOCKS_WARN)),
        "a non-blocking helper must not produce a false warning; got: {:?}",
        msgs
    );
}

#[test]
fn dead_receiver_stays_direct_only_helper_blocking_warns_not_errors() {
    // A non-main cooperative SUBSCRIBER whose run() blocks only via a
    // helper fn. Per the scope decision, the dead-receiver ERROR stays
    // direct-call-only (no error here), but the pool-stall WARNING
    // does fire interprocedurally.
    let src = r#"
type Tick { n: Int; }

fn pump(fd: Int) -> Int { return std::io::tcp::recv_into(fd, 0, 64); }

locus Gateway {
    bus { subscribe "tick" as on_tick of type Tick; }
    fn on_tick(t: Tick) { }
    run() { let n = pump(0); }
}

locus Feed {
    bus { publish "tick" of type Tick; }
    run() { "tick" <- Tick { n: 1 }; }
}

main locus App {
    params {
        gw: Gateway = Gateway { };
        feed: Feed  = Feed { };
    }
    placement { gw: cooperative(pool = ws); }
}

fn main() { App { }; }
"#;
    let msgs = check(src);
    assert!(
        !msgs.iter().any(|m| m.contains(DEAD_RX)),
        "the dead-receiver ERROR is direct-call-only; blocking via a helper \
         must NOT raise it; got: {:?}",
        msgs
    );
    assert!(
        msgs.iter().any(|m| m.contains(BLOCKS_WARN)),
        "blocking via a helper on a cooperative subscriber must still warn; \
         got: {:?}",
        msgs
    );
}

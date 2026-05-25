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

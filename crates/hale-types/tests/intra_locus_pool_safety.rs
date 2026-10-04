//! F.31 pool-safety guard on the intra-locus publish→direct-call
//! optimization (`desugar_intra_locus_topics`), through the frontend's
//! load and the lowering view, as every verb reaches it.
//!
//! The optimization rewrites a 1-publisher / 1-subscriber publish
//! into a direct, *synchronous* method call on the publisher's own
//! thread. That is correct only when publisher and subscriber share
//! an execution context. When the subscriber field is placed on a
//! separate cooperative pool (or pinned thread), the direct call
//! would run the handler on the publisher's thread (e.g. main) —
//! violating the single-threaded-pool invariant and dropping the
//! pool context that any locus the handler instantiates needs to
//! inherit (an accept'd child's run() would go synchronous and its
//! subscriptions would register on the global queue; observed as a
//! per-connection handler blocking the main thread in accept()).
//!
//! So: an off-thread subscriber must keep its publish on the bus
//! dispatch path (Send preserved), while an on-thread subscriber
//! (pool = main / no placement) still gets the optimization
//! (Send rewritten to a method-call Stmt::Expr). Which fields are off
//! their owner's thread is the placement table's answer (F.40 phase 3,
//! P1: `PlacementTable::off_owner_fields`, the rows whose
//! `owner_relative` is `OffOwner`), handed to the rewrite by
//! `resolve_program`.

use hale_frontend::snapshot::{Config, Snapshot};
use hale_syntax::ast::{Block, LifecycleKind, LocusMember, Stmt, TopDecl};

/// The `run()` body of the named locus in the lowering view's merged
/// program.
fn run_body<'a>(program: &'a hale_syntax::ast::Program, locus: &str) -> &'a Block {
    for item in &program.items {
        if let TopDecl::Locus(l) = item {
            if l.name.name != locus {
                continue;
            }
            for m in &l.members {
                if let LocusMember::Lifecycle(d) = m {
                    if d.kind == LifecycleKind::Run {
                        return &d.body;
                    }
                }
            }
        }
    }
    panic!("no run() body found for locus {locus}");
}

/// `true` if the run body still contains a `Stmt::Send` (publish
/// left on the bus path); `false` if every Send was rewritten to a
/// method-call Stmt::Expr by the optimization.
fn has_send(body: &Block) -> bool {
    body.stmts.iter().any(|s| matches!(s, Stmt::Send { .. }))
}

const SRC_TEMPLATE: &str = r#"
    type Ping { n: Int = 0; }
    topic PingT { payload: Ping; subject: "p.ping"; }

    locus Worker {
        bus { subscribe PingT as on_ping; }
        fn on_ping(p: Ping) { println("ping ", p.n); }
    }

    main locus App {
        params { w: Worker = Worker { }; }
        placement { __PLACEMENT__ }
        bus { publish PingT; }
        run() {
            PingT <- Ping { n: 1 };
        }
    }

    fn main() { App { }; }
"#;

/// Whether the lowering view still publishes from `App.run()`, and how
/// many sends the rewrite replaced.
fn lowered(src: &str) -> (bool, usize) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let s = Snapshot::from_program(program, Vec::new(), Config::check(false, false)).unwrap_or_else(|_| panic!("load"));
    let view = s.demand_lowering().unwrap_or_else(|b| panic!("lowering is blocked: {:?}", b.because));
    (has_send(run_body(&view.merged, "App")), view.intra_locus.len())
}

fn with_placement(placement: &str) -> (bool, usize) {
    lowered(&SRC_TEMPLATE.replace("__PLACEMENT__", placement))
}

#[test]
fn off_thread_pool_subscriber_keeps_publish_on_bus() {
    // Worker on a named non-main pool → the publish must NOT be
    // devirtualized to a direct call on main.
    assert_eq!(
        with_placement("w: cooperative(pool = io);"),
        (true, 0),
        "publish to a pool-placed subscriber was rewritten to a direct \
         call — it must stay on the bus dispatch path so the handler \
         runs on the pool's worker"
    );
}

#[test]
fn async_io_pool_subscriber_keeps_publish_on_bus() {
    // The exact shape that wedged the main thread: an async_io pool.
    assert_eq!(
        with_placement("w: cooperative(pool = io) where async_io;"),
        (true, 0),
        "publish to an async_io-pool subscriber was rewritten to a direct call"
    );
}

#[test]
fn pinned_subscriber_keeps_publish_on_bus() {
    assert_eq!(
        with_placement("w: pinned;"),
        (true, 0),
        "publish to a pinned subscriber was rewritten to a direct call"
    );
}

#[test]
fn pool_main_subscriber_still_optimizes() {
    // Explicit `pool = main` keeps Worker on the publisher's thread,
    // so the optimization is safe and must still fire.
    assert_eq!(
        with_placement("w: cooperative(pool = main);"),
        (false, 1),
        "publish to a same-thread (pool = main) subscriber should still \
         be optimized to a direct call"
    );
}

#[test]
fn unplaced_subscriber_still_optimizes() {
    // No placement block at all → Worker is on main with the
    // publisher → optimization must still fire.
    assert_eq!(
        lowered(&SRC_TEMPLATE.replace("placement { __PLACEMENT__ }", "")),
        (false, 1),
        "publish to an unplaced (main-thread) subscriber should still be \
         optimized to a direct call"
    );
}

/// The set the rewrite reads is the table's: the root field its entry
/// pins, by the root's name, and nothing else.
#[test]
fn the_off_owner_set_is_the_tables_rows() {
    let src = r#"
        type Ping { n: Int = 0; }
        topic PingT { payload: Ping; subject: "p.ping"; }
        locus Worker {
            bus { subscribe PingT as on_ping; }
            fn on_ping(p: Ping) { println("ping ", p.n); }
        }
        main locus App {
            params { w: Worker = Worker { }; }
            placement { w: pinned; }
            bus { publish PingT; }
            run() { PingT <- Ping { n: 1 }; }
        }
        fn main() { App { }; }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let s = Snapshot::from_program(program, Vec::new(), Config::check(false, false)).unwrap_or_else(|_| panic!("load"));
    let table = s.demand_placement().unwrap_or_else(|_| panic!("placement is blocked"));
    let want: std::collections::BTreeSet<(String, String)> = [("App".to_string(), "w".to_string())].into();
    assert_eq!(table.off_owner_fields(), want);
}

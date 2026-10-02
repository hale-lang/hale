//! The intra-locus publish→direct-call optimization
//! (`desugar_intra_locus_topics`) over the set of fields it is handed as
//! off their owner's thread.
//!
//! Which fields those are is the placement table's answer (F.40 phase 3,
//! P1), and `hale-types`'s `tests/intra_locus_pool_safety.rs` checks the
//! F.31 pool-safety guard end to end, through the frontend's load and the
//! lowering view. Here the set is written by hand: a field in it keeps
//! its publish on the bus, and the rewrite returns what it rewrote.

use std::collections::BTreeSet;

use hale_syntax::ast::{Block, LifecycleKind, LocusMember, Stmt, TopDecl};
use hale_syntax::desugar::desugar_intra_locus_topics;

/// Pull the `run()` body of the named locus out of a parsed +
/// desugared program.
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

const SRC: &str = r#"
    type Ping { n: Int = 0; }
    topic PingT { payload: Ping; subject: "p.ping"; }

    locus Worker {
        bus { subscribe PingT as on_ping; }
        fn on_ping(p: Ping) { println("ping ", p.n); }
    }

    main locus App {
        params { w: Worker = Worker { }; }
        bus { publish PingT; }
        run() {
            PingT <- Ping { n: 1 };
        }
    }

    fn main() { App { }; }
"#;

#[test]
fn a_field_off_its_owners_thread_keeps_its_publish_on_the_bus() {
    let mut program = hale_syntax::parse_source(SRC).expect("parse");
    let off: BTreeSet<(String, String)> = [("App".to_string(), "w".to_string())].into();
    assert!(desugar_intra_locus_topics(&mut program, &off).is_empty());
    assert!(
        has_send(run_body(&program, "App")),
        "publish to an off-thread subscriber was rewritten to a direct call"
    );
}

#[test]
fn a_field_on_its_owners_thread_is_optimized() {
    let mut program = hale_syntax::parse_source(SRC).expect("parse");
    desugar_intra_locus_topics(&mut program, &BTreeSet::new());
    assert!(
        !has_send(run_body(&program, "App")),
        "publish to a same-thread subscriber should be optimized to a direct call"
    );
}

/// Boundary 7 (F.40 phase 1.5): the rewrite returns what it erased —
/// the send's identity, the publishing locus, the topic and the handler
/// the call names — and the call keeps the send's identity, so a second
/// run finds nothing to rewrite and returns nothing.
#[test]
fn the_rewrite_returns_what_it_rewrote_once() {
    use hale_syntax::ast::{Expr, NodeId};
    use hale_syntax::desugar::IntraLocusRewrite;
    let mut program = hale_syntax::parse_source(SRC).expect("parse");
    for item in &mut program.items {
        let TopDecl::Locus(l) = item else { continue };
        for m in &mut l.members {
            let LocusMember::Lifecycle(d) = m else { continue };
            for s in &mut d.body.stmts {
                if let Stmt::Send { id, .. } = s {
                    *id = NodeId(42);
                }
            }
        }
    }
    let rewrites = desugar_intra_locus_topics(&mut program, &BTreeSet::new());
    assert_eq!(
        rewrites,
        vec![IntraLocusRewrite {
            send: NodeId(42),
            locus: "App".to_string(),
            subject: "PingT".to_string(),
            handler: "on_ping".to_string(),
        }]
    );
    let call_id = run_body(&program, "App").stmts.iter().find_map(|s| match s {
        Stmt::Expr(Expr::Call { id, .. }) => Some(*id),
        _ => None,
    });
    assert_eq!(call_id, Some(NodeId(42)), "the direct call carries the send's identity");
    assert!(
        desugar_intra_locus_topics(&mut program, &BTreeSet::new()).is_empty(),
        "a second run rewrites nothing"
    );
}

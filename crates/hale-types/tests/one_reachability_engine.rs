//! Reachability is derived in exactly one place.
//!
//! GH #408. There used to be two walks: the application checker's,
//! and a second one the fleet tier wrote against the topology
//! artifact. The second had to remember every rule the first had
//! learned, and it forgot several — through-stdlib edges, unknown
//! propagation, the zero-length overlap case, and the label on each
//! hop of a witness. Every one of those was a fail-open or a
//! misleading diagnostic, and each was fixed separately before the
//! shared engine existed.
//!
//! Both tiers now go through `model_graph::search`, which owns the
//! queue, the visited set, the parent tree, root seeding, masking and
//! the step ceiling. A third walk would reopen the same class, so it
//! fails here rather than in someone's deployment.
//!
//! The check is deliberately narrow: `pop_front` is the tell for a
//! hand-rolled breadth-first queue, and the engine is the only place
//! that should have one.

use std::path::Path;

/// The two claim evaluators. Anything else in the tree may walk
/// however it likes — this is about the two that must agree.
const EVALUATORS: [&str; 2] =
    ["src/claims.rs", "../hale-cli/src/fleet.rs"];

#[test]
fn neither_claim_evaluator_rolls_its_own_search() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut offenders = Vec::new();
    for rel in EVALUATORS {
        let p = root.join(rel);
        let src = std::fs::read_to_string(&p)
            .unwrap_or_else(|e| panic!("read {}: {e}", p.display()));
        for (i, line) in src.lines().enumerate() {
            if line.contains("pop_front()") {
                offenders.push(format!(
                    "{rel}:{}: {}",
                    i + 1,
                    line.trim()
                ));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "a hand-rolled traversal in a claim evaluator.\n\nReachability \
         belongs to `model_graph::search`, which both tiers share so \
         that a rule learned in one cannot go missing in the other. \
         If this genuinely is not a reachability walk, narrow the \
         check rather than deleting it.\n\n{}\n",
        offenders.join("\n")
    );
}

/// The engine is where the queue lives, so the lint above would be
/// vacuous if it also covered the engine — and vacuous if `pop_front`
/// stopped being how the engine is written. Pin both facts.
#[test]
fn the_engine_is_the_one_place_that_has_a_queue() {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/model_graph.rs");
    let src = std::fs::read_to_string(&p).expect("read model_graph.rs");
    assert!(
        src.contains("pop_front()"),
        "the shared engine no longer contains the pattern the lint \
         looks for, so that lint now proves nothing — update both"
    );
}

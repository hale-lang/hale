//! Boolean reachability is derived in one place.
//!
//! GH #408. There used to be two unweighted reachability walks: the
//! application checker's, and a second one the fleet tier wrote
//! against the topology artifact. The second had to remember every
//! rule the first had learned, and it forgot several — through-stdlib
//! edges, unknown propagation, the zero-length overlap case, and the
//! label on each hop of a witness. Every one was a fail-open or a
//! misleading diagnostic.
//!
//! Both prohibition checks now go through `model_graph::search`.
//!
//! **Scope, stated precisely.** This is about BOOLEAN reachability,
//! which is what `forbid reaches` and `only edges` ask. `claims.rs`
//! still contains `site_count`, a recursive weighted traversal with
//! its own memoization, cycle handling and step ceiling, because
//! `bound` computes a quantitative semiring rather than a yes/no
//! answer. That is a legitimately different algorithm over the same
//! edges, not a duplicate of this one — but it does mean this file
//! must not claim the evaluators contain no traversals at all.
//!
//! The checks below are textual and therefore heuristic. They are
//! worth having anyway: the failure they guard against is someone
//! writing a fresh queue rather than importing one, and that is
//! exactly what a grep can see.

use std::path::{Path, PathBuf};

fn read(rel: &str) -> String {
    let p: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&p)
        .unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

const CLAIMS: &str = "src/claims.rs";
const FLEET: &str = "../hale-cli/src/fleet.rs";
const ENGINE: &str = "src/model_graph.rs";

/// Neither prohibition evaluator may stand up its own BFS queue.
#[test]
fn no_prohibition_evaluator_defines_a_private_bfs() {
    let mut offenders = Vec::new();
    for rel in [CLAIMS, FLEET] {
        let src = read(rel);
        for (i, line) in src.lines().enumerate() {
            let l = line.trim();
            if l.starts_with("//") {
                continue;
            }
            // The tells for a hand-rolled frontier.
            if l.contains("pop_front()")
                || l.contains("pop_back()")
                || l.contains("VecDeque")
            {
                offenders.push(format!("{rel}:{}: {l}", i + 1));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "a hand-rolled frontier in a prohibition evaluator.\n\nBoolean \
         reachability belongs to `model_graph::search`, which both \
         tiers share so that a rule learned in one cannot go missing \
         in the other.\n\n{}\n",
        offenders.join("\n")
    );
}

/// ...and both must actually call the shared engine, or the check
/// above is satisfied by an evaluator that simply stopped doing
/// reachability the supported way.
#[test]
fn both_prohibition_evaluators_call_the_shared_engine() {
    assert!(
        read(CLAIMS).contains("model_graph::search("),
        "`claims.rs` no longer calls the shared search"
    );
    assert!(
        read(FLEET).contains("model_graph::ModelGraph")
            || read(FLEET).contains("ModelGraph::new"),
        "`fleet.rs` no longer uses the shared graph"
    );
    assert!(
        read(ENGINE).contains("search("),
        "`ModelGraph::reaches` no longer delegates to `search`"
    );
}

/// The engine holds exactly one frontier.
///
/// "At least one" would be satisfied by an engine that had grown a
/// second walk of its own, which is the same defect one level in.
#[test]
fn the_engine_holds_exactly_one_frontier() {
    let src = read(ENGINE);
    let n = src
        .lines()
        .filter(|l| !l.trim().starts_with("//"))
        .filter(|l| l.contains("pop_front()"))
        .count();
    assert_eq!(
        n, 1,
        "expected exactly one queue in the shared engine, found {n} — \
         either a second traversal appeared, or the first stopped \
         being written the way the lint above looks for, and that \
         lint is now vacuous"
    );
}

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
//! Change 10 moved the application-tier end of this: the evaluator
//! in `claims.rs` is deleted, and the prohibition it used to walk is
//! `judgment.rs`'s. The invariant is unchanged and so is the file it
//! guards against — a fresh queue instead of an import.
//!
//! **Scope, stated precisely.** This centralizes unweighted
//! TRANSITIVE reachability, which is what `forbid reaches` asks at
//! both tiers. Two neighbours are deliberately outside it:
//!
//!  * `only edges` enumerates DIRECT crossing edges — a cut-edge
//!    subset query, with no transitive walk at either tier;
//!  * `bound` keeps `site_count`, a recursive WEIGHTED traversal with
//!    its own memoization, cycle handling and step ceiling, because a
//!    quantitative semiring is a different algorithm over the same
//!    edges rather than a duplicate of this one.
//!
//! So this file must not claim the evaluators contain no traversals
//! at all. It claims something narrower and checkable: neither
//! prohibition evaluator stands up its own frontier.
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

const JUDGMENT: &str = "src/judgment.rs";
const FLEET: &str = "../hale-cli/src/fleet.rs";
const ENGINE: &str = "src/model_graph.rs";

/// Neither prohibition evaluator may stand up its own BFS queue.
#[test]
fn no_prohibition_evaluator_defines_a_private_bfs() {
    let mut offenders = Vec::new();
    for rel in [JUDGMENT, FLEET] {
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
        read(JUDGMENT).contains("model_graph::search("),
        "`judgment.rs` no longer calls the shared search"
    );
    // Mentioning or constructing a graph is not using it: a private
    // BFS could be restored while an unrelated `ModelGraph` reference
    // stayed behind. Pin the CALL.
    assert!(
        read(FLEET).contains("match graph.reaches("),
        "`fleet.rs` no longer evaluates prohibitions through \
         `ModelGraph::reaches`"
    );
    // Likewise `search(` alone is vacuous here — this module's own
    // tests call it, so the substring survives `reaches` ceasing to
    // delegate. Pin the call site inside `reaches`.
    assert!(
        read(ENGINE).contains("let out = search("),
        "`ModelGraph::reaches` no longer delegates to `search`"
    );
}

/// The engine holds exactly one frontier.
///
/// "At least one queue" would be satisfied by an engine that had
/// grown a second walk of its own — the same defect one level in. All
/// three counts are pinned, because a second frontier could otherwise
/// hide behind `pop_back` or a second `VecDeque` consumed by a
/// helper.
#[test]
fn the_engine_holds_exactly_one_frontier() {
    let src = read(ENGINE);
    let count = |needle: &str| {
        src.lines()
            .filter(|l| !l.trim().starts_with("//"))
            .filter(|l| l.contains(needle))
            .count()
    };
    assert_eq!(
        count("VecDeque::new()"),
        1,
        "expected exactly one queue in the shared engine"
    );
    assert_eq!(
        count("pop_front()"),
        1,
        "expected exactly one frontier pop — if the queue is now \
         drained some other way, the sibling lint that greps for \
         `pop_front` is vacuous and must change with this one"
    );
    assert_eq!(
        count("pop_back()"),
        0,
        "a second frontier hiding behind `pop_back`"
    );
}

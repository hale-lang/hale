//! One reachability engine over a normalized model.
//!
//! GH #408: the fleet tier read the topology artifact and rebuilt a
//! smaller graph of its own, and the omissions were not random. It
//! dropped `calls_via_stdlib`, so a user→stdlib→user path the
//! application checker walks disappeared when the same components
//! were composed. It collected `unknowns` into its output and never
//! consulted them, so an indirect call could remove the only modeled
//! path to a target and the prohibition reported `holds` — an
//! absence certified by not looking.
//!
//! Both are the same bug: a second derivation of "what reaches what"
//! that has to remember every rule the first one learned. This type
//! is that derivation, once. A caller supplies edges and says which
//! vertices have INCOMPLETE outgoing edge sets; it answers with a
//! path, a certified absence, or a refusal to certify.
//!
//! It deliberately knows nothing about instances, routes, claims or
//! artifacts — a route id rides along as an opaque label on an edge
//! so a witness can name it. That keeps it usable from the
//! application tier, whose edges are ordinary calls and bus hops,
//! when that tier moves onto the shared model.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// What a caller knows about one vertex's outgoing edges.
///
/// `edges` are the successors it CAN enumerate. `hole` is set when
/// that list is incomplete — the honest shape, because an incomplete
/// vertex usually has some known edges too. Reporting a hole is the
/// caller's only obligation; what happens next is [`HolePolicy`], and
/// the search guarantees the hole is never simply forgotten.
pub struct Visit<V, E, H> {
    pub edges: Vec<(V, E)>,
    pub hole: Option<H>,
}

impl<V, E, H> Visit<V, E, H> {
    /// A fully-known vertex.
    pub fn edges(edges: Vec<(V, E)>) -> Self {
        Self { edges, hole: None }
    }
    /// A vertex with known edges AND an incomplete edge set.
    pub fn partial(edges: Vec<(V, E)>, hole: H) -> Self {
        Self { edges, hole: Some(hole) }
    }
    /// A vertex whose edges cannot be enumerated at all.
    pub fn hole(hole: H) -> Self {
        Self { edges: Vec::new(), hole: Some(hole) }
    }
}

/// What an incomplete vertex means to this caller.
///
/// Both tiers of this compiler want a different answer and both are
/// right, so the choice belongs to the caller — but the consequence
/// belongs to the engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HolePolicy {
    /// Stop at the first hole. The application checker wants this:
    /// the repair is to make that edge resolvable, and the
    /// diagnostic names the edge.
    Halt,
    /// Keep walking known edges; a concrete counterexample beats a
    /// refusal, and the hole only decides the answer if no path is
    /// found. The fleet checker wants this: a cross-binary path is
    /// worth more than "cannot tell".
    PathWins,
}

/// The outcome of one search.
///
/// Every variant except `NotFound` carries the parent tree, so a
/// caller can render the path to wherever the walk stopped — "here is
/// how the source reaches the edge I could not follow" is far more
/// useful than naming the vertex alone.
pub enum Search<V, E, H> {
    /// The frontier was exhausted, over a graph with no relevant
    /// holes. This is the ONLY variant that proves an absence.
    NotFound,
    /// `hit` is in the target set; `parent` is the tree the caller's
    /// own witness renderer walks back through. Returning the tree
    /// rather than a path is what lets both tiers keep the witness
    /// code they already have.
    Found { hit: V, parent: BTreeMap<V, (V, E)> },
    /// A reachable vertex had an incomplete edge set, so no absence
    /// can be claimed past it.
    Uncertified { at: V, hole: H, parent: BTreeMap<V, (V, E)> },
    /// The step ceiling tripped. Distinct from `NotFound` on purpose:
    /// search EXHAUSTION can prove an absence, search ABANDONMENT
    /// never can, and collapsing the two is a fail-open.
    Saturated { parent: BTreeMap<V, (V, E)> },
}

/// The traversal both tiers share.
///
/// It owns exactly the bookkeeping that is easy to get subtly wrong
/// and identical everywhere — the queue, the visited set, the parent
/// tree, root seeding, masking, the step ceiling, and the propagation
/// of incomplete vertices. Everything that differs between an
/// application and a fleet — what a vertex is, which edges exist,
/// what counts as the target — stays with the caller.
///
/// Three rules are baked in because they are semantics, not
/// mechanism, and each one was a shipped bug before it lived here:
///
///  * **roots are tested.** A vertex in both the source and target
///    sets is a zero-length path, which is a real boundary confusion
///    a prohibition should surface rather than skip.
///  * **masked vertices are neither tested nor traversed**, so "no
///    path avoids the gate" is the interposition proof.
///  * **a reachable hole is never dropped.** A caller reports it; the
///    engine decides the verdict. Forgetting to consult it is not a
///    mistake this API allows.
///
/// `max_steps` is `None` for "walk until exhausted". It is not a
/// large number: a ceiling that can be reached must be able to say
/// so, and `u32::MAX` would overflow the counter before tripping.
pub fn search<V, E, H>(
    roots: impl IntoIterator<Item = V>,
    mut successors: impl FnMut(&V) -> Visit<V, E, H>,
    is_dst: impl Fn(&V) -> bool,
    is_masked: impl Fn(&V) -> bool,
    max_steps: Option<u32>,
    policy: HolePolicy,
) -> Search<V, E, H>
where
    V: Ord + Clone,
{
    let mut parent: BTreeMap<V, (V, E)> = BTreeMap::new();
    let mut seen: BTreeSet<V> = BTreeSet::new();
    let mut queue: VecDeque<V> = VecDeque::new();
    // The first hole met under `PathWins`, kept so a fruitless search
    // can still refuse rather than certify.
    let mut pending: Option<(V, H)> = None;
    for r in roots {
        if is_masked(&r) {
            continue;
        }
        if seen.insert(r.clone()) {
            queue.push_back(r);
        }
    }
    let mut steps: u32 = 0;
    while let Some(k) = queue.pop_front() {
        steps += 1;
        if max_steps.is_some_and(|m| steps > m) {
            return Search::Saturated { parent };
        }
        if is_dst(&k) {
            return Search::Found { hit: k, parent };
        }
        let Visit { edges, hole } = successors(&k);
        if let Some(h) = hole {
            match policy {
                HolePolicy::Halt => {
                    return Search::Uncertified { at: k, hole: h, parent }
                }
                HolePolicy::PathWins => {
                    if pending.is_none() {
                        pending = Some((k.clone(), h));
                    }
                }
            }
        }
        for (next, label) in edges {
            if is_masked(&next) {
                continue;
            }
            if seen.insert(next.clone()) {
                parent.insert(next.clone(), (k.clone(), label));
                queue.push_back(next);
            }
        }
    }
    match pending {
        Some((at, hole)) => Search::Uncertified { at, hole, parent },
        None => Search::NotFound,
    }
}

/// An edge, with an optional label the witness renderer can name
/// (the fleet tier puts a route id here; the application tier has
/// nothing to say and passes `None`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edge {
    pub to: String,
    pub via: Option<String>,
}

/// Why a vertex's outgoing edges are not fully known.
///
/// Recorded per vertex so the refusal can say which hole stopped it —
/// "cannot certify" is only actionable if the reader learns where to
/// look.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hole {
    pub at: String,
    pub why: String,
}

/// The result of asking whether one set reaches another.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reach {
    /// No path, over a graph with no relevant holes: a real absence.
    None,
    /// A concrete path, as `(vertex, edge label used to enter it)`.
    /// The first element's label is always `None`.
    Path(Vec<(String, Option<String>)>),
    /// No path was found, but a vertex reachable from the source has
    /// incomplete outgoing edges, so the absence is not proved. This
    /// is `uncertified`, not `holds` — and not `violated` either,
    /// since nothing was disproved.
    Uncertified(Hole),
}

/// A directed graph plus the set of vertices whose out-edges are
/// incomplete.
#[derive(Default, Debug)]
pub struct ModelGraph {
    edges: BTreeMap<String, Vec<Edge>>,
    holes: BTreeMap<String, String>,
}

impl ModelGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_edge(
        &mut self,
        from: impl Into<String>,
        to: impl Into<String>,
        via: Option<String>,
    ) {
        self.edges.entry(from.into()).or_default().push(Edge {
            to: to.into(),
            via,
        });
    }

    /// Mark a vertex as having outgoing edges this model cannot see.
    pub fn add_hole(
        &mut self,
        at: impl Into<String>,
        why: impl Into<String>,
    ) {
        self.holes.insert(at.into(), why.into());
    }

    pub fn has_holes(&self) -> bool {
        !self.holes.is_empty()
    }

    /// Does any vertex in `src` reach any vertex in `dst`?
    ///
    /// `masked` vertices are removed from the graph entirely, which
    /// is how interposition is asked: "no path avoids the gate".
    ///
    /// A found path always wins over a hole. A counterexample is
    /// definitive, and reporting `uncertified` when a real violation
    /// exists would hide the more useful answer.
    pub fn reaches(
        &self,
        src: &BTreeSet<String>,
        dst: &BTreeSet<String>,
        masked: &BTreeSet<String>,
    ) -> Reach {
        // Hole propagation is the engine's job now: this tier just
        // reports whether a vertex is incomplete and picks the
        // policy. The application tier picks `Halt` instead — see
        // `claims.rs`, where stopping at the edge is what makes the
        // diagnostic actionable.
        let out = search(
            src.iter().cloned(),
            |v: &String| Visit {
                edges: self
                    .edges
                    .get(v)
                    .into_iter()
                    .flatten()
                    .map(|e| (e.to.clone(), e.via.clone()))
                    .collect(),
                hole: self.holes.get(v.as_str()).cloned(),
            },
            // Roots are tested. A vertex in both sets is a
            // zero-length path — the source already IS the forbidden
            // destination — and suppressing that here is what made
            // `forbid_reaches(g, g)` report a false absence.
            |v| dst.contains(v),
            |v| masked.contains(v),
            None,
            HolePolicy::PathWins,
        );

        match out {
            Search::Found { hit: h, parent } => {
                // parent maps a node to (predecessor, label of the
                // edge INTO it), so walking back already yields the
                // label each hop was entered by.
                let mut rev: Vec<(String, Option<String>)> =
                    vec![(h.clone(), None)];
                let mut cur = h;
                while let Some((prev, via)) = parent.get(&cur) {
                    rev.last_mut().expect("non-empty").1 = via.clone();
                    rev.push((prev.clone(), None));
                    cur = prev.clone();
                }
                rev.reverse();
                Reach::Path(rev)
            }
            Search::Uncertified { at, hole, .. } => {
                Reach::Uncertified(Hole { at, why: hole })
            }
            // Abandonment never proves an absence.
            Search::Saturated { .. } => {
                Reach::Uncertified(Hole {
                    at: String::new(),
                    why: "the reachability walk hit its step ceiling \
                          before exhausting the graph"
                        .to_string(),
                })
            }
            Search::NotFound => Reach::None,
        }
    }
}

/// Which artifact `unknowns` kinds actually mean "edges are missing".
///
/// `uninhabited_interface_call` is recorded as an unknown but is NOT
/// fail-closed: in a closed world an interface with no conformers has
/// no values, so the call site is dead and the walker treats it as
/// such (see `topology.rs`). Reading every unknown as a hole would
/// make a dead site refuse to certify anything downstream of it,
/// which is both wrong and loud.
pub fn kind_hides_edges(kind: &str) -> bool {
    match kind {
        "indirect_call" => true,
        "computed_publish" => true,
        k if k.starts_with("untyped_receiver_call") => true,
        k if k.starts_with("uninhabited_interface_call") => false,
        // Unrecognized kinds fail CLOSED. A newer compiler emitting a
        // hole this build has never heard of must not be read as an
        // absence of holes.
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(xs: &[&str]) -> BTreeSet<String> {
        xs.iter().map(|s| s.to_string()).collect()
    }

    fn chain() -> ModelGraph {
        let mut g = ModelGraph::new();
        g.add_edge("a", "b", Some("r1".into()));
        g.add_edge("b", "c", Some("r2".into()));
        g
    }

    #[test]
    fn a_path_names_the_label_of_each_hop() {
        let g = chain();
        match g.reaches(&set(&["a"]), &set(&["c"]), &BTreeSet::new()) {
            Reach::Path(p) => {
                assert_eq!(p[0], ("a".into(), None));
                assert_eq!(p[1], ("b".into(), Some("r1".into())));
                assert_eq!(p[2], ("c".into(), Some("r2".into())));
            }
            other => panic!("expected a path, got {other:?}"),
        }
    }

    #[test]
    fn absence_over_a_complete_graph_is_certified() {
        let mut g = ModelGraph::new();
        g.add_edge("a", "b", None);
        assert_eq!(
            g.reaches(&set(&["a"]), &set(&["z"]), &BTreeSet::new()),
            Reach::None
        );
    }

    #[test]
    fn a_reachable_hole_refuses_to_certify_an_absence() {
        let mut g = ModelGraph::new();
        g.add_edge("a", "b", None);
        g.add_hole("b", "indirect_call");
        match g.reaches(&set(&["a"]), &set(&["z"]), &BTreeSet::new()) {
            Reach::Uncertified(h) => {
                assert_eq!(h.at, "b");
                assert_eq!(h.why, "indirect_call");
            }
            other => panic!("expected uncertified, got {other:?}"),
        }
    }

    /// The rule that keeps this usable: an unknown somewhere else in
    /// the model is not evidence about THIS claim.
    #[test]
    fn an_unreachable_hole_does_not_poison_the_claim() {
        let mut g = ModelGraph::new();
        g.add_edge("a", "b", None);
        g.add_hole("q", "indirect_call");
        assert_eq!(
            g.reaches(&set(&["a"]), &set(&["z"]), &BTreeSet::new()),
            Reach::None
        );
    }

    /// A counterexample is definitive; a hole elsewhere must not
    /// downgrade it to "cannot tell".
    #[test]
    fn a_real_path_wins_over_a_hole() {
        let mut g = chain();
        g.add_hole("b", "indirect_call");
        assert!(matches!(
            g.reaches(&set(&["a"]), &set(&["c"]), &BTreeSet::new()),
            Reach::Path(_)
        ));
    }

    #[test]
    fn masking_the_gate_removes_the_path() {
        let g = chain();
        assert_eq!(
            g.reaches(&set(&["a"]), &set(&["c"]), &set(&["b"])),
            Reach::None
        );
    }

    /// A hole behind the mask is not reachable, so it cannot conceal
    /// anything the claim is about.
    #[test]
    fn masking_also_hides_the_holes_behind_it() {
        let mut g = chain();
        g.add_hole("b", "indirect_call");
        assert_eq!(
            g.reaches(&set(&["a"]), &set(&["c"]), &set(&["b"])),
            Reach::None
        );
    }

    // ---- the wrapper's two fixed fail-opens -------------------

    /// A vertex in both the source and target sets already sits in
    /// the forbidden set without going anywhere. The wrapper used to
    /// pass `dst.contains(v) && !src.contains(v)`, which disabled the
    /// engine's root test and answered `None` — the exact false
    /// absence the engine documents itself as preventing.
    #[test]
    fn source_target_overlap_is_a_zero_length_path() {
        let g = ModelGraph::new();
        assert_eq!(
            g.reaches(&set(&["a"]), &set(&["a"]), &BTreeSet::new()),
            Reach::Path(vec![("a".into(), None)])
        );
    }

    /// Search EXHAUSTION can prove an absence. Search ABANDONMENT
    /// cannot, and the wrapper used to map both to `Reach::None`.
    #[test]
    fn abandoning_the_walk_never_certifies_an_absence() {
        let out: Search<&str, (), String> = search(
            ["a"],
            |_| Visit::edges(vec![("b", ()), ("c", ())]),
            |_| false,
            |_| false,
            Some(1),
            HolePolicy::PathWins,
        );
        assert!(
            matches!(out, Search::Saturated { .. }),
            "a tripped ceiling is not `NotFound`"
        );
    }

    // ---- the generic engine's own contract ---------------------

    #[test]
    fn a_root_that_is_the_target_is_found_without_expanding() {
        let mut expanded = false;
        let out: Search<&str, (), ()> = search(
            ["a"],
            |_| {
                expanded = true;
                Visit::edges(vec![])
            },
            |v| *v == "a",
            |_| false,
            None,
            HolePolicy::Halt,
        );
        assert!(matches!(out, Search::Found { .. }));
        assert!(!expanded, "the target test precedes expansion");
    }

    #[test]
    fn a_masked_root_is_neither_tested_nor_traversed() {
        let out: Search<&str, (), ()> = search(
            ["a"],
            |_| Visit::edges(vec![]),
            |v| *v == "a",
            |v| *v == "a",
            None,
            HolePolicy::Halt,
        );
        assert!(matches!(out, Search::NotFound));
    }

    #[test]
    fn halt_reports_the_vertex_and_the_reason() {
        let out: Search<&str, (), &str> = search(
            ["a"],
            |_| Visit::hole("indirect"),
            |_| false,
            |_| false,
            None,
            HolePolicy::Halt,
        );
        match out {
            Search::Uncertified { at, hole, .. } => {
                assert_eq!(at, "a");
                assert_eq!(hole, "indirect");
            }
            _ => panic!("expected uncertified"),
        }
    }

    /// A vertex can have known edges AND an incomplete set. Under
    /// `PathWins` the known edges are still walked, and the hole only
    /// decides the answer if nothing is found.
    #[test]
    fn a_partial_vertex_still_contributes_its_known_edges() {
        let out: Search<&str, (), &str> = search(
            ["a"],
            |v| match *v {
                "a" => Visit::partial(vec![("b", ())], "indirect"),
                _ => Visit::edges(vec![]),
            },
            |v| *v == "b",
            |_| false,
            None,
            HolePolicy::PathWins,
        );
        assert!(
            matches!(out, Search::Found { .. }),
            "the known edge reaches the target, which beats the hole"
        );
    }

    #[test]
    fn several_roots_are_all_seeded() {
        let out: Search<&str, (), ()> = search(
            ["a", "b"],
            |_| Visit::edges(vec![]),
            |v| *v == "b",
            |_| false,
            None,
            HolePolicy::Halt,
        );
        assert!(matches!(out, Search::Found { hit: "b", .. }));
    }

    /// Breadth-first: the parent tree records the SHORTEST route to
    /// each vertex, so a witness is the shortest counterexample
    /// rather than whichever one the walk stumbled into.
    #[test]
    fn the_parent_tree_records_the_shortest_route() {
        let mut g = ModelGraph::new();
        g.add_edge("a", "mid", Some("long1".into()));
        g.add_edge("mid", "z", Some("long2".into()));
        g.add_edge("a", "z", Some("short".into()));
        match g.reaches(&set(&["a"]), &set(&["z"]), &BTreeSet::new()) {
            Reach::Path(p) => {
                assert_eq!(p.len(), 2, "a -> z directly: {p:?}");
                assert_eq!(p[1].1, Some("short".into()));
            }
            other => panic!("expected a path, got {other:?}"),
        }
    }

    #[test]
    fn unknown_kinds_are_classified_by_whether_they_hide_edges() {
        assert!(kind_hides_edges("indirect_call"));
        assert!(kind_hides_edges("computed_publish"));
        assert!(kind_hides_edges("untyped_receiver_call:foo"));
        // dead in a closed world, not a hole
        assert!(!kind_hides_edges("uninhabited_interface_call:I.m"));
        // a kind from a newer compiler fails closed
        assert!(kind_hides_edges("some_future_hole"));
    }
}

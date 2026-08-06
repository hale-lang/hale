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
pub enum Visit<V, E> {
    /// The successors, each with the label to record for the witness.
    Edges(Vec<(V, E)>),
    /// This vertex's edges cannot be enumerated, so the search must
    /// not report an absence past it. The caller has already said why
    /// (it owns the diagnostic); the search just stops.
    Halt,
}

/// The outcome of one search.
pub enum Search<V, E> {
    /// The frontier was exhausted without reaching the target.
    NotFound,
    /// `hit` is in the target set; `parent` is the tree the caller's
    /// own witness renderer walks back through. Returning the tree
    /// rather than a path is what lets both tiers keep the witness
    /// code they already have.
    Found { hit: V, parent: BTreeMap<V, (V, E)> },
    /// The caller refused to enumerate this vertex's edges.
    Halted(V),
    /// The step ceiling tripped.
    Saturated,
}

/// The traversal both tiers share.
///
/// It owns exactly the bookkeeping that is easy to get subtly wrong
/// and identical everywhere — the queue, the visited set, the parent
/// tree, the root seeding, the mask, and the step ceiling. Everything
/// that differs between an application and a fleet — what a vertex
/// is, which edges exist, what counts as the target, when to refuse —
/// stays with the caller as a closure.
///
/// Two rules are deliberately baked in because they are semantics
/// rather than mechanism:
///
///  * **roots are tested.** A vertex in both the source and target
///    sets is a zero-length path, which is a real boundary confusion
///    a prohibition should surface rather than skip. The fleet tier
///    lacked this rule and reported `forbid_reaches(g, g)` as holding.
///  * **masked vertices are neither tested nor traversed**, so "no
///    path avoids the gate" is the interposition proof.
pub fn search<V, E>(
    roots: impl IntoIterator<Item = V>,
    mut successors: impl FnMut(&V) -> Visit<V, E>,
    is_dst: impl Fn(&V) -> bool,
    is_masked: impl Fn(&V) -> bool,
    max_steps: u32,
) -> Search<V, E>
where
    V: Ord + Clone,
    E: Clone,
{
    let mut parent: BTreeMap<V, (V, E)> = BTreeMap::new();
    let mut seen: BTreeSet<V> = BTreeSet::new();
    let mut queue: VecDeque<V> = VecDeque::new();
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
        if steps > max_steps {
            return Search::Saturated;
        }
        if is_dst(&k) {
            return Search::Found { hit: k, parent };
        }
        let edges = match successors(&k) {
            Visit::Halt => return Search::Halted(k),
            Visit::Edges(e) => e,
        };
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
    Search::NotFound
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
        // Holes seen while walking, in deterministic order so a
        // refusal names the same vertex on every machine.
        //
        // This tier walks PAST a hole rather than stopping at it: a
        // concrete counterexample is more useful than "cannot tell",
        // so the refusal is only reported if the search finds no path
        // at all. The application tier makes the opposite choice —
        // see `claims.rs`, where the diagnostic explains which edge
        // could not be followed and the walk ends there.
        let mut hit: BTreeSet<String> = BTreeSet::new();
        let out = search(
            src.iter().cloned(),
            |v: &String| {
                if self.holes.contains_key(v.as_str()) {
                    hit.insert(v.clone());
                }
                Visit::Edges(
                    self.edges
                        .get(v)
                        .into_iter()
                        .flatten()
                        .map(|e| (e.to.clone(), e.via.clone()))
                        .collect(),
                )
            },
            |v| dst.contains(v) && !src.contains(v),
            |v| masked.contains(v),
            u32::MAX,
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
            // Only a hole the source could actually have walked to
            // can conceal a path — an unrelated unknown elsewhere in
            // the fleet must not poison every claim.
            Search::NotFound | Search::Saturated => match hit.first() {
                Some(at) => Reach::Uncertified(Hole {
                    at: at.clone(),
                    why: self.holes[at.as_str()].clone(),
                }),
                None => Reach::None,
            },
            Search::Halted(at) => Reach::Uncertified(Hole {
                at: at.clone(),
                why: self.holes.get(&at).cloned().unwrap_or_default(),
            }),
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

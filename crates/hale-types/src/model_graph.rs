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
        let mut parent: BTreeMap<&str, (&str, Option<String>)> =
            BTreeMap::new();
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        let mut queue: VecDeque<&str> = VecDeque::new();
        // Holes seen while walking, kept in deterministic order so a
        // refusal names the same vertex on every machine.
        let mut hit: BTreeSet<&str> = BTreeSet::new();

        for v in src {
            if masked.contains(v) {
                continue;
            }
            if seen.insert(v) {
                queue.push_back(v);
                if self.holes.contains_key(v.as_str()) {
                    hit.insert(v);
                }
            }
        }

        while let Some(cur) = queue.pop_front() {
            if dst.contains(cur) && !src.contains(cur) {
                let mut path = vec![(cur.to_string(), None)];
                let mut at = cur;
                while let Some((prev, via)) = parent.get(at) {
                    path.push((prev.to_string(), via.clone()));
                    at = prev;
                }
                path.reverse();
                // Each node is paired with the label of its OUTGOING
                // edge by the walk above; the renderer wants the one
                // it was entered by, so shift every label forward.
                let shifted: Vec<(String, Option<String>)> = path
                    .iter()
                    .enumerate()
                    .map(|(i, (n, _))| {
                        (
                            n.clone(),
                            if i == 0 {
                                None
                            } else {
                                path[i - 1].1.clone()
                            },
                        )
                    })
                    .collect();
                return Reach::Path(shifted);
            }
            for e in self.edges.get(cur).into_iter().flatten() {
                if masked.contains(&e.to) {
                    continue;
                }
                if seen.insert(&e.to) {
                    parent.insert(&e.to, (cur, e.via.clone()));
                    queue.push_back(&e.to);
                    if self.holes.contains_key(e.to.as_str()) {
                        hit.insert(&e.to);
                    }
                }
            }
        }

        // No path. Only a hole the source could actually have walked
        // to can conceal one — an unrelated unknown elsewhere in the
        // fleet must not poison every claim.
        match hit.first() {
            Some(at) => Reach::Uncertified(Hole {
                at: (*at).to_string(),
                why: self.holes[*at].clone(),
            }),
            None => Reach::None,
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

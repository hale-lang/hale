//! Exact declaration-layer unit relationships (GH #1076, #1212).
//!
//! An equation says that one `from` unit equals `factor` `to` units.
//! Closing the catalogue proves path independence and records a forest
//! of declaration witnesses. Queries consume that closure; they never
//! choose a semantic base unit or round a factor to a machine number.
//!
//! This is the unit dialect's semantic core. Parsing quantity/point
//! declarations, solving expression flows, and replacing the current
//! `Time`/`Duration` primitive handling are separate consumers still to
//! be connected. In particular, this module grants no new source syntax.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use hale_graph::ids::SiteId;
use num_bigint::BigInt;
use num_integer::Integer;
use num_rational::BigRational;

/// A positive, reduced, exact conversion factor. Zero and negative
/// factors cannot describe unit equivalence. Point origins belong to
/// the quantity/point rows, not to this multiplicative graph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ratio(BigRational);

impl Ratio {
    pub fn new(numerator: BigInt, denominator: BigInt) -> Option<Self> {
        if numerator <= BigInt::from(0) || denominator <= BigInt::from(0) {
            return None;
        }
        Some(Self(BigRational::new(numerator, denominator)))
    }

    pub fn one() -> Self {
        Self(BigRational::from_integer(BigInt::from(1)))
    }

    pub fn numerator(&self) -> &BigInt {
        self.0.numer()
    }

    pub fn denominator(&self) -> &BigInt {
        self.0.denom()
    }

    pub fn reciprocal(&self) -> Self {
        Self(self.0.recip())
    }

    pub fn times(&self, other: &Self) -> Self {
        Self(&self.0 * &other.0)
    }

    pub fn divided_by(&self, other: &Self) -> Self {
        Self(&self.0 / &other.0)
    }

    /// Greatest positive rational dividing both inputs by integers.
    fn gcd(&self, other: &Self) -> Self {
        Self(BigRational::new(
            self.numerator().gcd(other.numerator()),
            self.denominator().lcm(other.denominator()),
        ))
    }

    /// Exactness of denomination conversion only. A runtime range or
    /// representation-width obligation is a separate check.
    pub fn is_integral(&self) -> bool {
        self.denominator() == &BigInt::from(1)
    }
}

impl std::fmt::Display for Ratio {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// Resolved declaration identities, never display names or source spans.
#[derive(Clone, Debug)]
pub struct Equation {
    pub site: SiteId,
    pub from: SiteId,
    pub to: SiteId,
    pub factor: Ratio,
}

/// One equation used forward or backward. The renderer looks its site
/// up in the snapshot's provenance rather than reconstructing a path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step {
    pub equation: SiteId,
    pub reversed: bool,
}

impl Step {
    fn reversed(self) -> Self {
        Self {
            reversed: !self.reversed,
            ..self
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CatalogueError {
    DuplicateUnit(SiteId),
    DuplicateEquation(SiteId),
    UnknownUnit {
        equation: SiteId,
        unit: SiteId,
    },
    /// Follow the forest from `from` to `to`, then the inconsistent
    /// equation backward. The witness's product is not one.
    InconsistentCycle {
        equation: SiteId,
        claimed: Ratio,
        implied: Ratio,
        cycle: Vec<Step>,
    },
}

#[derive(Clone, Debug)]
struct Node {
    component: SiteId,
    /// Relative to an arbitrary traversal root; never exposed as a
    /// base-unit choice. Only quotients and gcds leave the producer.
    scale: Ratio,
    /// Parent and the equation direction from parent to this node.
    parent: Option<(SiteId, Step)>,
}

/// An immutable, consistent catalogue. Failed closure returns errors,
/// never a graph from which a consumer could read an invented ratio.
#[derive(Clone, Debug)]
pub struct UnitGraph {
    nodes: BTreeMap<SiteId, Node>,
    equations: BTreeMap<SiteId, Equation>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conversion {
    pub factor: Ratio,
    pub witness: Vec<Step>,
}

impl Conversion {
    /// A reduced denominator greater than one requires an explicit
    /// loss decision. This does not choose that decision or a policy.
    pub fn requires_loss_policy(&self) -> bool {
        !self.factor.is_integral()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MeetError {
    NoInputs,
    UnknownUnit { input: usize, unit: SiteId },
    DifferentComponents { first: usize, other: usize },
}

/// A solved denomination, borrowing the catalogue that gives it meaning.
/// It need not have a declared spelling: the rational gcd of two nodes
/// is not necessarily another declared node.
#[derive(Debug)]
pub struct Denomination<'g> {
    graph: &'g UnitGraph,
    component: SiteId,
    scale: Ratio,
    /// Input positions: deterministic and irredundant. A gcd can need
    /// more than two inputs (6, 10, 15); never drop a necessary witness.
    pub witnesses: Vec<usize>,
}

impl Denomination<'_> {
    /// All declared names for this exact denomination, in identity order.
    pub fn named_units(&self) -> impl Iterator<Item = SiteId> + '_ {
        self.graph.nodes.iter().filter_map(|(id, n)| {
            (n.component == self.component && n.scale == self.scale).then_some(*id)
        })
    }

    /// The factor at a boundary with a pinned target denomination.
    /// Returns none for an unknown or disconnected unit.
    pub fn factor_to(&self, target: SiteId) -> Option<Ratio> {
        let n = self.graph.nodes.get(&target)?;
        (n.component == self.component).then(|| self.scale.divided_by(&n.scale))
    }
}

impl UnitGraph {
    pub fn close(
        units: impl IntoIterator<Item = SiteId>,
        equations: impl IntoIterator<Item = Equation>,
    ) -> Result<Self, Vec<CatalogueError>> {
        let mut errors = Vec::new();
        let mut unit_set = BTreeSet::new();
        for unit in units {
            if !unit_set.insert(unit) {
                errors.push(CatalogueError::DuplicateUnit(unit));
            }
        }
        let mut rows = BTreeMap::new();
        for equation in equations {
            for unit in [equation.from, equation.to] {
                if !unit_set.contains(&unit) {
                    errors.push(CatalogueError::UnknownUnit {
                        equation: equation.site,
                        unit,
                    });
                }
            }
            let site = equation.site;
            if rows.insert(site, equation).is_some() {
                errors.push(CatalogueError::DuplicateEquation(site));
            }
        }
        if !errors.is_empty() {
            return Err(errors);
        }

        let mut neighbours: BTreeMap<SiteId, Vec<(SiteId, Step, Ratio)>> =
            unit_set.iter().map(|id| (*id, Vec::new())).collect();
        for equation in rows.values() {
            let step = Step {
                equation: equation.site,
                reversed: false,
            };
            neighbours.get_mut(&equation.from).unwrap().push((
                equation.to,
                step,
                equation.factor.clone(),
            ));
            neighbours.get_mut(&equation.to).unwrap().push((
                equation.from,
                step.reversed(),
                equation.factor.reciprocal(),
            ));
        }

        let mut graph = Self {
            nodes: BTreeMap::new(),
            equations: rows,
        };
        let mut bad_equations = BTreeSet::new();
        for root in unit_set {
            if graph.nodes.contains_key(&root) {
                continue;
            }
            graph.nodes.insert(
                root,
                Node {
                    component: root,
                    scale: Ratio::one(),
                    parent: None,
                },
            );
            let mut pending = VecDeque::from([root]);
            while let Some(from) = pending.pop_front() {
                let from_scale = graph.nodes[&from].scale.clone();
                for (to, step, factor) in &neighbours[&from] {
                    let expected = from_scale.divided_by(factor);
                    if let Some(known) = graph.nodes.get(to) {
                        if known.scale != expected && bad_equations.insert(step.equation) {
                            // Render the equation in its written direction,
                            // independent of how the traversal reached it.
                            let e = &graph.equations[&step.equation];
                            let implied = graph.nodes[&e.from]
                                .scale
                                .divided_by(&graph.nodes[&e.to].scale);
                            let mut cycle = graph.path(e.from, e.to);
                            cycle.push(Step {
                                equation: e.site,
                                reversed: true,
                            });
                            errors.push(CatalogueError::InconsistentCycle {
                                equation: e.site,
                                claimed: e.factor.clone(),
                                implied,
                                cycle,
                            });
                        }
                    } else {
                        graph.nodes.insert(
                            *to,
                            Node {
                                component: root,
                                scale: expected,
                                parent: Some((from, *step)),
                            },
                        );
                        pending.push_back(*to);
                    }
                }
            }
        }
        if errors.is_empty() {
            Ok(graph)
        } else {
            Err(errors)
        }
    }

    pub fn equation(&self, site: SiteId) -> Option<&Equation> {
        self.equations.get(&site)
    }

    pub fn conversion(&self, from: SiteId, to: SiteId) -> Option<Conversion> {
        let a = self.nodes.get(&from)?;
        let b = self.nodes.get(&to)?;
        (a.component == b.component).then(|| Conversion {
            factor: a.scale.divided_by(&b.scale),
            witness: self.path(from, to),
        })
    }

    /// Coarsest denomination into which every input widens exactly.
    /// This solves an unpinned flow; it never changes a boundary's pin.
    pub fn meet(&self, inputs: &[SiteId]) -> Result<Denomination<'_>, MeetError> {
        let first = *inputs.first().ok_or(MeetError::NoInputs)?;
        let first_node = self.nodes.get(&first).ok_or(MeetError::UnknownUnit {
            input: 0,
            unit: first,
        })?;
        let mut scales = Vec::with_capacity(inputs.len());
        let mut scale = first_node.scale.clone();
        let mut witnesses = vec![0usize];
        for (input, unit) in inputs.iter().enumerate() {
            let n = self
                .nodes
                .get(unit)
                .ok_or(MeetError::UnknownUnit { input, unit: *unit })?;
            if n.component != first_node.component {
                return Err(MeetError::DifferentComponents {
                    first: 0,
                    other: input,
                });
            }
            scales.push(&n.scale);
            let next = scale.gcd(&n.scale);
            if next != scale {
                witnesses.push(input);
                scale = next;
            }
        }
        // Inputs that never refined the running meet cannot become
        // necessary later. Prune only the contributing inputs, so a
        // flow with thousands of repeated literals is not quadratic.
        // Removing an input can only coarsen the meet. Once a witness
        // is necessary, removing more inputs cannot make it redundant.
        // Prefer earlier source inputs when several explanations exist.
        for position in (0..witnesses.len()).rev() {
            if witnesses.len() == 1 {
                break;
            }
            let remove = witnesses[position];
            let mut remaining = witnesses.iter().copied().filter(|i| *i != remove);
            let candidate = remaining
                .next()
                .map(|first| remaining.fold(scales[first].clone(), |a, i| a.gcd(scales[i])));
            if candidate.as_ref() == Some(&scale) {
                witnesses.retain(|i| *i != remove);
            }
        }
        Ok(Denomination {
            graph: self,
            component: first_node.component,
            scale,
            witnesses,
        })
    }

    fn path(&self, from: SiteId, to: SiteId) -> Vec<Step> {
        let mut up = Vec::new();
        let mut ancestors = BTreeMap::from([(from, 0usize)]);
        let mut cur = from;
        while let Some((parent, step)) = self.nodes[&cur].parent {
            up.push(step.reversed());
            cur = parent;
            ancestors.insert(cur, up.len());
        }
        let mut down = Vec::new();
        cur = to;
        while !ancestors.contains_key(&cur) {
            let (parent, step) = self.nodes[&cur].parent.expect("same component");
            down.push(step);
            cur = parent;
        }
        up.truncate(ancestors[&cur]);
        up.extend(down.into_iter().rev());
        up
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hale_graph::ids::SeedId;

    fn unit(n: u32) -> SiteId {
        SiteId::new(SeedId(0), n)
    }
    fn equation_site(n: u32) -> SiteId {
        SiteId::new(SeedId(1), n)
    }
    fn ratio(n: u64, d: u64) -> Ratio {
        Ratio::new(n.into(), d.into()).unwrap()
    }
    fn equation(n: u32, from: u32, to: u32, numerator: u64, denominator: u64) -> Equation {
        Equation {
            site: equation_site(n),
            from: unit(from),
            to: unit(to),
            factor: ratio(numerator, denominator),
        }
    }

    fn walk(rows: &[Equation], mut at: SiteId, steps: &[Step]) -> (SiteId, Ratio) {
        let mut factor = Ratio::one();
        for step in steps {
            let row = rows.iter().find(|e| e.site == step.equation).unwrap();
            if step.reversed {
                assert_eq!(at, row.to);
                at = row.from;
                factor = factor.times(&row.factor.reciprocal());
            } else {
                assert_eq!(at, row.from);
                at = row.to;
                factor = factor.times(&row.factor);
            }
        }
        (at, factor)
    }

    #[test]
    fn conversion_direction_exactness_and_witness_agree() {
        // s -> ms -> us -> ns; the node numbering deliberately makes
        // ms, not ns, the traversal root.
        let rows = vec![
            equation(0, 3, 0, 1000, 1),
            equation(1, 0, 2, 1000, 1),
            equation(2, 2, 1, 1000, 1),
        ];
        let graph = UnitGraph::close((0..4).map(unit), rows.clone()).unwrap();
        let wide = graph.conversion(unit(3), unit(1)).unwrap();
        assert_eq!(wide.factor, ratio(1_000_000_000, 1));
        assert!(!wide.requires_loss_policy());
        assert_eq!(walk(&rows, unit(3), &wide.witness), (unit(1), wide.factor));
        let narrow = graph.conversion(unit(1), unit(3)).unwrap();
        assert_eq!(narrow.factor, ratio(1, 1_000_000_000));
        assert!(narrow.requires_loss_policy());
        assert_eq!(
            walk(&rows, unit(1), &narrow.witness),
            (unit(3), narrow.factor)
        );
    }

    #[test]
    fn a_consistent_cycle_does_not_add_a_second_answer() {
        let rows = vec![
            equation(0, 0, 1, 6, 1),
            equation(1, 1, 2, 5, 1),
            equation(2, 0, 2, 30, 1),
        ];
        let graph = UnitGraph::close((0..3).map(unit), rows.clone()).unwrap();
        for a in 0..3 {
            for b in 0..3 {
                let c = graph.conversion(unit(a), unit(b)).unwrap();
                assert_eq!(walk(&rows, unit(a), &c.witness), (unit(b), c.factor));
            }
        }
    }

    #[test]
    fn inconsistent_cycles_carry_a_closed_nonidentity_product() {
        let rows = vec![
            equation(0, 0, 1, 6, 1),
            equation(1, 1, 2, 5, 1),
            equation(2, 0, 2, 31, 1),
        ];
        let errors = UnitGraph::close((0..3).map(unit), rows.clone()).unwrap_err();
        assert_eq!(errors.len(), 1);
        let CatalogueError::InconsistentCycle {
            equation,
            claimed,
            implied,
            cycle,
        } = &errors[0]
        else {
            panic!("{errors:?}")
        };
        assert_ne!(claimed, implied);
        let row = rows.iter().find(|e| e.site == *equation).unwrap();
        let (end, product) = walk(&rows, row.from, cycle);
        assert_eq!(end, row.from);
        assert_ne!(product, Ratio::one());
        assert_eq!(product, implied.divided_by(claimed));
        assert_eq!(cycle.len(), 3);
    }

    #[test]
    fn self_equations_obey_the_same_cycle_law() {
        assert!(UnitGraph::close([unit(0)], [equation(0, 0, 0, 1, 1)]).is_ok());
        let rows = vec![equation(0, 0, 0, 2, 1)];
        let errors = UnitGraph::close([unit(0)], rows.clone()).unwrap_err();
        let CatalogueError::InconsistentCycle { cycle, .. } = &errors[0] else {
            panic!("{errors:?}")
        };
        assert_eq!(walk(&rows, unit(0), cycle), (unit(0), ratio(1, 2)));
    }

    #[test]
    fn a_catalogue_can_have_several_disconnected_quantities() {
        let graph = UnitGraph::close(
            (0..5).map(unit),
            [equation(0, 0, 1, 1000, 1), equation(1, 2, 3, 1024, 1)],
        )
        .unwrap();
        assert!(graph.conversion(unit(0), unit(2)).is_none());
        assert!(graph.conversion(unit(0), unit(99)).is_none());
        assert!(matches!(
            graph.meet(&[unit(0), unit(2)]),
            Err(MeetError::DifferentComponents { first: 0, other: 1 })
        ));
        assert!(matches!(graph.meet(&[]), Err(MeetError::NoInputs)));
        assert!(matches!(
            graph.meet(&[unit(0), unit(99)]),
            Err(MeetError::UnknownUnit { input: 1, .. })
        ));
        assert_eq!(
            graph.conversion(unit(4), unit(4)).unwrap().factor,
            Ratio::one()
        );
        assert_eq!(
            graph
                .meet(&[unit(4)])
                .unwrap()
                .named_units()
                .collect::<Vec<_>>(),
            [unit(4)]
        );
    }

    #[test]
    fn duplicate_and_unknown_identities_are_not_silently_merged() {
        assert_eq!(
            UnitGraph::close([unit(0), unit(0)], []).unwrap_err(),
            [CatalogueError::DuplicateUnit(unit(0))]
        );
        let e = equation(0, 0, 1, 3, 1);
        assert_eq!(
            UnitGraph::close([unit(0), unit(1)], [e.clone(), e.clone()]).unwrap_err(),
            [CatalogueError::DuplicateEquation(e.site)]
        );
        assert_eq!(
            UnitGraph::close([unit(0)], [e.clone()]).unwrap_err(),
            [CatalogueError::UnknownUnit {
                equation: e.site,
                unit: unit(1)
            }]
        );
        // Two units with identical local indices but different seeds
        // remain disconnected; SiteId equality, never NodeId equality.
        let other = SiteId::new(SeedId(2), 0);
        let graph = UnitGraph::close([unit(0), other], []).unwrap();
        assert!(graph.conversion(unit(0), other).is_none());
    }

    #[test]
    fn no_zero_negative_or_unreduced_factors_enter_the_graph() {
        for (n, d) in [(0, 1), (1, 0), (-1, 1), (1, -1), (-1, -1)] {
            assert!(Ratio::new(n.into(), d.into()).is_none());
        }
        assert_eq!(ratio(10, 20), ratio(1, 2));
        assert_eq!(ratio(10, 20).to_string(), "1/2");
    }

    #[test]
    fn catalogue_products_are_not_limited_to_runtime_integer_widths() {
        let rows: Vec<_> = (0..200).map(|n| equation(n, n, n + 1, 1000, 1)).collect();
        let graph = UnitGraph::close((0..201).map(unit), rows.clone()).unwrap();
        let c = graph.conversion(unit(0), unit(200)).unwrap();
        assert_eq!(c.factor.numerator(), &BigInt::from(1000).pow(200));
        assert_eq!(c.factor.denominator(), &BigInt::from(1));
        assert_eq!(walk(&rows, unit(0), &c.witness), (unit(200), c.factor));
        let back = graph.conversion(unit(200), unit(0)).unwrap();
        assert_eq!(back.factor.denominator(), &BigInt::from(1000).pow(200));
    }

    #[test]
    fn meet_tracks_the_finest_necessary_input_and_the_boundary_pin() {
        let graph = UnitGraph::close(
            (0..4).map(unit),
            [
                equation(0, 0, 1, 60, 1),
                equation(1, 1, 2, 1000, 1),
                equation(2, 2, 3, 1000, 1),
            ],
        )
        .unwrap();
        // min, s, ms => ms, witnessed by the ms input only.
        let d = graph.meet(&[unit(0), unit(1), unit(2)]).unwrap();
        assert_eq!(d.named_units().collect::<Vec<_>>(), [unit(2)]);
        assert_eq!(d.witnesses, [2]);
        assert_eq!(d.factor_to(unit(1)), Some(ratio(1, 1000)));
        assert_eq!(d.factor_to(unit(3)), Some(ratio(1000, 1)));
        // Introducing us changes the unpinned flow; the conversion to
        // the pinned seconds boundary is still explicit narrowing.
        let finer = graph.meet(&[unit(0), unit(1), unit(2), unit(3)]).unwrap();
        assert_eq!(finer.witnesses, [3]);
        assert_eq!(finer.factor_to(unit(1)), Some(ratio(1, 1_000_000)));
    }

    #[test]
    fn some_meets_require_three_witnesses() {
        let graph = UnitGraph::close(
            (0..4).map(unit),
            [
                equation(0, 1, 0, 6, 1),
                equation(1, 2, 0, 10, 1),
                equation(2, 3, 0, 15, 1),
            ],
        )
        .unwrap();
        let inputs = [unit(1), unit(2), unit(3)];
        let d = graph.meet(&inputs).unwrap();
        assert_eq!(d.named_units().collect::<Vec<_>>(), [unit(0)]);
        assert_eq!(d.witnesses, [0, 1, 2]);
        for omit in 0..3 {
            let fewer: Vec<_> = inputs
                .iter()
                .enumerate()
                .filter_map(|(i, u)| (i != omit).then_some(*u))
                .collect();
            assert_ne!(
                graph.meet(&fewer).unwrap().factor_to(unit(0)),
                d.factor_to(unit(0))
            );
        }
        let repeated = graph.meet(&[unit(1), unit(2), unit(3), unit(1)]).unwrap();
        assert_eq!(repeated.witnesses, [0, 1, 2]);
    }

    #[test]
    fn rational_meet_can_have_no_declared_spelling() {
        let graph = UnitGraph::close(
            (0..3).map(unit),
            [equation(0, 1, 0, 3, 2), equation(1, 2, 0, 5, 2)],
        )
        .unwrap();
        let d = graph.meet(&[unit(1), unit(2)]).unwrap();
        assert_eq!(d.factor_to(unit(0)), Some(ratio(1, 2)));
        assert_eq!(d.named_units().count(), 0);
        assert_eq!(d.witnesses, [0, 1]);
        for u in [unit(1), unit(2)] {
            // Every input divided by the meet is an integer.
            assert!(d.factor_to(u).unwrap().reciprocal().is_integral());
        }
    }

    #[test]
    fn repeated_flow_inputs_do_not_duplicate_the_explanation() {
        let graph = UnitGraph::close([unit(0), unit(1)], [equation(0, 0, 1, 1000, 1)]).unwrap();
        let mut inputs = vec![unit(0); 10_000];
        inputs.push(unit(1));
        let d = graph.meet(&inputs).unwrap();
        assert_eq!(d.witnesses, [10_000]);
        assert_eq!(d.factor_to(unit(1)), Some(Ratio::one()));
    }

    #[test]
    fn traversal_root_and_declaration_order_do_not_choose_the_denomination() {
        let weights = [2, 3, 5, 7, 11];
        for shift in 0..weights.len() {
            let id = |i: usize| unit(((i + shift) % weights.len()) as u32);
            let mut rows = Vec::new();
            for i in 0..weights.len() {
                for j in (i + 1)..weights.len() {
                    rows.push(Equation {
                        site: equation_site(rows.len() as u32),
                        from: id(i),
                        to: id(j),
                        factor: ratio(weights[i], weights[j]),
                    });
                }
            }
            rows.reverse();
            let graph = UnitGraph::close((0..weights.len()).rev().map(id), rows).unwrap();
            for i in 0..weights.len() {
                for j in 0..weights.len() {
                    assert_eq!(
                        graph.conversion(id(i), id(j)).unwrap().factor,
                        ratio(weights[i], weights[j])
                    );
                }
            }
            let d = graph
                .meet(&(0..weights.len()).map(id).collect::<Vec<_>>())
                .unwrap();
            assert_eq!(d.factor_to(id(0)), Some(ratio(1, weights[0])));
            assert_eq!(d.witnesses, [0, 1]);
        }
    }
}

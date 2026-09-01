//! Positive completeness — what the model can promise is exact.
//!
//! A consumer asks "is this model adequate for my judgment?" by
//! reading capabilities, never by reverse-engineering the absence of
//! particular strings. The dual-bookkeeping law: a capability may
//! not claim exactness while any hole hides that relation family —
//! [`ApplicationModel::validate`] rejects the contradiction, so the
//! two accounts cannot drift.
//!
//! [`ApplicationModel::validate`]: crate::application::ApplicationModel::validate

use crate::hole::RelationSet;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Capabilities {
    /// The call graph is complete: no unresolved callee anywhere
    /// reachable.
    pub exact_calls: bool,
    /// Publish and subscribe completeness are INDEPENDENT facts
    /// (round 3): a set-level hole can hide one without the other,
    /// and per-family adequacy must not couple them.
    pub exact_publishes: bool,
    /// Every subscription is known.
    pub exact_subscribes: bool,
    /// Every key predicate's coverage is decidable.
    pub exact_key_filters: bool,
    /// The ownership tree is complete.
    pub exact_ownership: bool,
    /// Every instance's thread domain is known statically.
    pub exact_placement: bool,
    /// Every transport route is known.
    pub exact_routes: bool,
    /// Every effect is classified.
    pub exact_effects: bool,
    /// Instance counts are statically exact.
    pub exact_cardinality: bool,
    /// Delivery semantics are known for every binding.
    pub exact_delivery_guarantees: bool,
    /// Per-call cost facts (allocation sites, frame sizes, blocking
    /// points) are complete for every analyzed function — what the
    /// quantitative `@budget` laws count over (Change 5h).
    pub exact_costs: bool,
}

impl Capabilities {
    /// The relation families each capability vouches for — the
    /// contradiction check in `validate` walks this mapping. EVERY
    /// flag participates: a capability with no mapped family would
    /// be unfalsifiable, which is exactly the drift this law
    /// exists to prevent.
    pub fn vouched_families(self) -> Vec<(&'static str, bool, RelationSet)> {
        vec![
            ("exact_calls", self.exact_calls, RelationSet::CALLS),
            (
                "exact_publishes",
                self.exact_publishes,
                RelationSet::PUBLISHES,
            ),
            (
                "exact_subscribes",
                self.exact_subscribes,
                RelationSet::SUBSCRIBES,
            ),
            (
                "exact_key_filters",
                self.exact_key_filters,
                RelationSet::KEY_FILTERS,
            ),
            ("exact_ownership", self.exact_ownership, RelationSet::OWNS),
            ("exact_placement", self.exact_placement, RelationSet::PLACED),
            ("exact_routes", self.exact_routes, RelationSet::ROUTES),
            ("exact_effects", self.exact_effects, RelationSet::EFFECTS),
            (
                "exact_cardinality",
                self.exact_cardinality,
                RelationSet::CARDINALITY,
            ),
            (
                "exact_delivery_guarantees",
                self.exact_delivery_guarantees,
                RelationSet::DELIVERY,
            ),
            ("exact_costs", self.exact_costs, RelationSet::COSTS),
        ]
    }
}

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
    pub exact_calls: bool,
    pub exact_bus_endpoints: bool,
    pub exact_key_filters: bool,
    pub exact_ownership: bool,
    pub exact_placement: bool,
    pub exact_routes: bool,
    pub exact_effects: bool,
    pub exact_cardinality: bool,
    pub exact_delivery_guarantees: bool,
}

impl Capabilities {
    /// The relation families each capability vouches for — the
    /// contradiction check in `validate` walks this mapping. EVERY
    /// flag participates: a capability with no mapped family would
    /// be unfalsifiable, which is exactly the drift this law
    /// exists to prevent.
    pub fn vouched_families(
        self,
    ) -> Vec<(&'static str, bool, RelationSet)> {
        vec![
            ("exact_calls", self.exact_calls, RelationSet::CALLS),
            (
                "exact_bus_endpoints",
                self.exact_bus_endpoints,
                RelationSet::PUBLISHES.union(RelationSet::SUBSCRIBES),
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
        ]
    }
}

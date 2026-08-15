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
    /// contradiction check in `validate` walks this mapping.
    pub fn vouched_families(self) -> Vec<(bool, RelationSet)> {
        vec![
            (self.exact_calls, RelationSet::CALLS),
            (
                self.exact_bus_endpoints,
                RelationSet::PUBLISHES.union(RelationSet::SUBSCRIBES),
            ),
            (self.exact_ownership, RelationSet::OWNS),
            (self.exact_placement, RelationSet::PLACED),
            (self.exact_routes, RelationSet::ROUTES),
            (self.exact_effects, RelationSet::EFFECTS),
        ]
    }
}

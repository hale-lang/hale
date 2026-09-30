//! Identity mechanics (F.40 phase 1.1a).
//!
//! Two identity laws, inherited from the model (GH #476) and kept:
//!
//! 1. **No semantic string joins.** Canonical declaration identity,
//!    author-facing display spelling, wire subject identity,
//!    payload-shape identity and deployed-instance identity are
//!    separate fields. Nothing decides two things are the same
//!    because display strings happen to match.
//! 2. **Internal density, external stability.** A table id is a dense
//!    index for in-memory speed; serialized identities are stable
//!    canonical names, never these numbers. An id is meaningless
//!    outside the value that minted it.
//!
//! And the snapshot identity F.40 adds: a [`SiteId`] is `(seed,
//! index)`, minted once after desugar for every semantic site of the
//! merged program (phase 1.1b mints; this module is the type). It
//! has real equality, ordering and hashing, unlike the AST's
//! structural `NodeId`, which compares equal to everything on purpose
//! so syntax trees can be compared for shape. Addresses are not
//! identities (declarations are cloned) and spans are not (the
//! stdlib's coordinates overlap user files; desugars stamp one span
//! on several declarations), which is why neither is the key.

/// A dense typed id: an index into its owning table, wrapped in a
/// newtype so tables cannot be confused.
#[macro_export]
macro_rules! table_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
        pub struct $name(pub u32);

        impl $name {
            /// The index into the owning table.
            pub fn index(self) -> usize {
                self.0 as usize
            }
        }
    };
}

table_id!(
    /// A source seed (a compilation unit set): the seed half of a
    /// [`SiteId`], and the model's seed sort.
    SeedId
);

/// The identity of one semantic site in a snapshot: the seed it came
/// from and its index in that seed's pre-order numbering, minted once
/// after desugar. Snapshot-local: two snapshots number independently,
/// and a shadow compares them through an explicit correspondence,
/// never by these numbers. Not a persistent deployment identity.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct SiteId {
    /// The seed the site belongs to.
    pub seed: SeedId,
    /// The site's index in the seed's numbering.
    pub index: u32,
}

impl SiteId {
    pub const fn new(seed: SeedId, index: u32) -> SiteId {
        SiteId { seed, index }
    }
}

impl std::fmt::Display for SiteId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.seed.0, self.index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_site_id_orders_by_seed_then_index_and_renders_as_one_name() {
        let a = SiteId::new(SeedId(0), 7);
        let b = SiteId::new(SeedId(0), 8);
        let c = SiteId::new(SeedId(1), 0);
        assert!(a < b && b < c);
        assert_eq!(a.to_string(), "0:7");
        assert_eq!(a, SiteId::new(SeedId(0), 7));
        assert_ne!(a, b);
        let mut set = std::collections::BTreeSet::new();
        set.insert(a);
        set.insert(SiteId::new(SeedId(0), 7));
        assert_eq!(set.len(), 1, "real equality and hashing");
        assert_eq!(SeedId(3).index(), 3);
    }
}

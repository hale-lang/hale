//! The snapshot (F.40 phase 1.1b-ii): identity minted once, after
//! desugar, for every semantic site of the program.
//!
//! A site's identity is a `SiteId { seed, index }`
//! (`hale_graph::ids`). The index is one counter over the whole
//! merged program, so it is unique across seeds: F.39's owner table
//! keys rows by the bare index (`ExprId(u32)`), and a per-seed
//! numbering would collide there. The seed is the source unit the
//! site's span falls in (the bundle's `sources`, whose `base`/`len`
//! give each unit's range in the bundle-global space), or the
//! program's own ordinal when the caller has no source map.
//!
//! Minting is idempotent: a site that already carries an id keeps
//! it, and the counter continues past the largest id present. That
//! is what lets the F.39 pre-pass (`resolve_owners`), which numbers
//! `Struct` and `Call` nodes it finds unnumbered, run after the
//! snapshot without renumbering anything, and keep numbering on the
//! harness paths that build without a snapshot.
//!
//! What this pass records is one row per site: its id, its kind and
//! its span. Tables key on the id; witnesses render from the span
//! through the provenance store. Generated declarations (the JSON
//! parsers, the api surface, the topic desugars) are minted like any
//! other site, since they exist in the program by the time the
//! sequence has run; their origin as generated sites is recorded
//! separately (phase 1.1b-iii).

use hale_graph::ids::{SeedId, SiteId};
use hale_syntax::ast::{NodeId, Program};
use hale_syntax::sites::{for_each_site, for_each_site_mut, SiteKind};
use hale_syntax::Span;

use crate::symbol::SourceFile;

/// One minted site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Site {
    pub id: SiteId,
    pub kind: SiteKind,
    pub span: Span,
}

/// The identities of one snapshot, in minting order.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    /// Every site, ordered by index.
    pub sites: Vec<Site>,
    /// The seeds, by `SeedId` index: a source unit's path, or the
    /// program's ordinal rendered as text when no source map was
    /// given.
    pub seeds: Vec<String>,
}

impl Snapshot {
    /// The site with this id, if the snapshot minted it.
    pub fn site(&self, id: SiteId) -> Option<&Site> {
        self.sites
            .binary_search_by_key(&id.index, |s| s.id.index)
            .ok()
            .map(|i| &self.sites[i])
            .filter(|s| s.id == id)
    }

    pub fn len(&self) -> usize {
        self.sites.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sites.is_empty()
    }
}

/// The seed of a span: the source unit whose bundle-global range holds
/// its start, if the source map has one.
fn seed_of(sources: &[SourceFile], span: Span) -> Option<SeedId> {
    sources
        .iter()
        .position(|u| span.start.0 >= u.base && span.start.0 < u.base.saturating_add(u.len))
        .map(|i| SeedId(i as u32))
}

/// Mint an identity for every site of `programs` that has none, and
/// return every site's row. `sources` is the bundle's source map (the
/// seed of a site is the unit its span falls in); when it is empty,
/// each program is its own seed, in the order given.
pub fn mint<'a>(
    programs: impl IntoIterator<Item = (&'a str, &'a mut Program)>,
    sources: &[SourceFile],
) -> Snapshot {
    let programs: Vec<(&str, &mut Program)> = programs.into_iter().collect();
    // The counter continues past the largest id already present, so
    // minting twice, or after a pre-pass numbered some nodes, keeps
    // every existing id.
    let mut next: u32 = 0;
    for (_, p) in programs.iter() {
        for_each_site(p, &mut |_, _, id| {
            if !id.is_none() {
                next = next.max(id.0 + 1);
            }
        });
    }
    let mut seeds: Vec<String> = sources.iter().map(|u| u.path.clone()).collect();
    let mut sites = Vec::new();
    for (ordinal, (path, p)) in programs.into_iter().enumerate() {
        let own_seed = if sources.is_empty() {
            seeds.push(path.to_string());
            Some(SeedId(ordinal as u32))
        } else {
            None
        };
        for_each_site_mut(p, &mut |kind, span, id| {
            if id.is_none() {
                *id = NodeId(next);
                next += 1;
            }
            let seed = own_seed
                .or_else(|| seed_of(sources, span))
                .unwrap_or(SeedId(0));
            sites.push(Site { id: SiteId::new(seed, id.0), kind, span });
        });
    }
    sites.sort_by_key(|s| s.id.index);
    sites.dedup_by_key(|s| s.id.index);
    Snapshot { sites, seeds }
}

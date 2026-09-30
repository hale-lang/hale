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
//! parsers, the api surface) are minted like any other site, since
//! they exist in the program by the time the sequence has run; which
//! desugar generated a site is a second table, `origins` (phase
//! 1.1b-iii), read off the marker each desugar leaves.

use std::collections::BTreeMap;

use hale_graph::ids::{SeedId, SiteId};
use hale_syntax::ast::{NodeId, Program, TopDecl};
use hale_syntax::sites::{
    for_each_named_site, for_each_site, for_each_site_in_item, for_each_site_mut, SiteKind,
};
use hale_syntax::Span;

use crate::symbol::SourceFile;

/// Which desugar synthesized a site. `mint` records one for every site
/// it can tell was generated, by the marker the desugar leaves:
///
/// - `JsonParsers`: `json_gen`'s `__json_parse_<T>` / `__json_to_json_<T>`
///   fns (the name prefix) and its `JsonError` (`TypeDecl::synthetic`,
///   since a program may declare a `JsonError` of its own). The api
///   codecs' `JsonError` comes from the same generator and is recorded
///   here too.
/// - `ApiSurface`: `api_gen` parses everything it generates at
///   `API_SYNTH_BASE`, so any site whose span starts there; and the api
///   codecs, which `json_gen` parses at 0, by their `__api_decode_` /
///   `__api_encode_` prefix.
/// - `OmittedRun`: the `run` `desugar_omitted_run` adds carries its
///   locus's own span, which no written lifecycle can.
/// - `ChainDesugar`: the `let`s and assignments the chains rewrite
///   introduces bind `__hale_`-prefixed names. The calls it builds
///   (`.get(i)`, `.len()`) carry the chain's span and no marker, so
///   they have no row.
/// - `TopicDesugar`, `IntraLocusRewrite`, `ReprAccessors`: no row. Those
///   passes rewrite subjects and expressions in place and synthesize no
///   declaration; the calls the latter two build carry the rewritten
///   node's span and no marker. They also run only inside codegen,
///   after every entry point has minted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Origin {
    JsonParsers,
    ApiSurface,
    TopicDesugar,
    IntraLocusRewrite,
    ReprAccessors,
    OmittedRun,
    ChainDesugar,
}

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
    /// The generated sites and the desugar that generated each,
    /// ordered by index. A site with no row was written.
    pub origins: Vec<(SiteId, Origin)>,
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

    /// The desugar that generated this site, if one did.
    pub fn origin(&self, id: SiteId) -> Option<Origin> {
        self.origins
            .binary_search_by_key(&id.index, |(s, _)| s.index)
            .ok()
            .map(|i| self.origins[i])
            .filter(|(s, _)| *s == id)
            .map(|(_, o)| o)
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
/// return every site's row and every generated site's origin.
/// `sources` is the bundle's source map (the seed of a site is the unit
/// its span falls in); when it is empty, each program is its own seed,
/// in the order given.
pub fn mint<'a>(
    programs: impl IntoIterator<Item = (&'a str, &'a mut Program)>,
    sources: &[SourceFile],
) -> Snapshot {
    let mut programs: Vec<(&str, &mut Program)> = programs.into_iter().collect();
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
    for (ordinal, (path, p)) in programs.iter_mut().enumerate() {
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
    let mut snapshot = Snapshot { sites, seeds, origins: Vec::new() };

    let mut by_index: BTreeMap<u32, Origin> = BTreeMap::new();
    for (_, p) in programs.iter() {
        origins_of(p, &mut by_index);
    }
    snapshot.origins = by_index
        .into_iter()
        .filter_map(|(index, origin)| {
            let i = snapshot
                .sites
                .binary_search_by_key(&index, |s| s.id.index)
                .ok()?;
            Some((snapshot.sites[i].id, origin))
        })
        .collect();
    snapshot
}

/// Record the origin of every generated site of `p`, by index. A
/// declaration's origin covers every site inside it and is recorded
/// first; the per-site markers only fill what that left.
fn origins_of(p: &Program, out: &mut BTreeMap<u32, Origin>) {
    fn items(decls: &[TopDecl], out: &mut BTreeMap<u32, Origin>) {
        for item in decls {
            let origin = match item {
                TopDecl::Fn(fd)
                    if fd.name.name.starts_with("__json_parse_")
                        || fd.name.name.starts_with("__json_to_json_") =>
                {
                    Some(Origin::JsonParsers)
                }
                TopDecl::Fn(fd)
                    if fd.name.name.starts_with("__api_decode_")
                        || fd.name.name.starts_with("__api_encode_") =>
                {
                    Some(Origin::ApiSurface)
                }
                TopDecl::Type(t) if t.synthetic && t.name.name == "JsonError" => {
                    Some(Origin::JsonParsers)
                }
                TopDecl::Module(md) => {
                    items(&md.items, out);
                    None
                }
                _ => None,
            };
            if let Some(origin) = origin {
                for_each_site_in_item(item, &mut |_, _, id| {
                    out.entry(id.0).or_insert(origin);
                });
            }
        }
    }
    items(&p.items, out);

    let mut locus_span: Option<Span> = None;
    for_each_named_site(p, &mut |kind, span, name, id| {
        // Pre-order: a locus is visited before its members.
        if kind == SiteKind::Locus {
            locus_span = Some(span);
        }
        let origin = if span.start.0 >= hale_syntax::api_gen::API_SYNTH_BASE {
            Some(Origin::ApiSurface)
        } else {
            match kind {
                SiteKind::Lifecycle if Some(span) == locus_span => Some(Origin::OmittedRun),
                SiteKind::Let | SiteKind::Assign | SiteKind::For
                    if name.is_some_and(|n| n.starts_with("__hale_")) =>
                {
                    Some(Origin::ChainDesugar)
                }
                _ => None,
            }
        };
        if let Some(origin) = origin {
            out.entry(id.0).or_insert(origin);
        }
    });
}

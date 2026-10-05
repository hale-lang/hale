//! The lowering view's correspondence (F.40 phase 3, C5): which site of
//! the checked programs, or of the bundled stdlib, each site of the
//! merged program is.
//!
//! The lowering view ([`crate::resolved`]) lowers a program the check
//! never saw: the checked program after the two lowering rewrites (the
//! intra-locus rewrite and the topic rewrite), with the bundled stdlib
//! appended, minted once more. The snapshot's families are derived over
//! the checked programs, and the stdlib's over its analysis copy
//! ([`crate::stdlib_bodies`]); lowering reads them for the merged
//! program. That is sound only if every merged site is one of those
//! sites, which this module states and checks.
//!
//! The merged program's sites are the checked programs' sites plus the
//! stdlib's:
//!
//! - a user site keeps the id the check's mint gave it (the merged mint
//!   keeps every id it finds), so its image is the checked site at the
//!   same index, of the same kind and span ([`Image::Checked`]);
//! - the call the intra-locus rewrite puts in a send's place carries the
//!   send's id, so its image is the checked send, through the rewrite's
//!   relation ([`Image::RewrittenSend`]);
//! - a stdlib site pairs, in walk order, with the analysis copy's: the
//!   two are clones of one parsed program that no pass touches between
//!   the clone and the mint ([`Image::Stdlib`]);
//! - the one site a rewrite generates: the `std::api::local_context()`
//!   call the intra-locus rewrite passes to a handler that takes a
//!   `std::api::Context` (GH #1108), at its send's span, which its
//!   relation flags (`IntraLocusRewrite::context`;
//!   [`Image::Generated`]). It has no checked image, and no family has a
//!   row for it.
//!
//! The law ([`correspond`]): every merged site has exactly one image,
//! no two merged sites share one, and every checked site and every
//! analysis-copy site is the image of exactly one merged site, except
//! the sites the rewrites erased. A rewrite erases one kind of site, the
//! subject of a send: an identifier expression (a `Use`) the topic
//! rewrite replaces with its wire literal, or the intra-locus rewrite
//! drops with the send, and each relation records it
//! (`TopicRewrite::erased`, `IntraLocusRewrite::erased`;
//! [`Erased`]). Beside the context call, the rewrites generate no site,
//! so the merged program holds none the correspondence cannot place.

use std::collections::{BTreeMap, BTreeSet};

use hale_graph::ids::SiteId;
use hale_syntax::ast::{NodeId, Program, TopDecl};
use hale_syntax::desugar::{IntraLocusRewrite, TopicRewrite};
use hale_syntax::sites::{for_each_site_in_item, SiteKind};
use hale_syntax::Span;

use crate::snapshot::{Snapshot, STDLIB_SEED};

/// What a merged site is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Image {
    /// The checked programs' site at the same index, of the same kind
    /// and span: the merged mint kept its id.
    Checked(SiteId),
    /// The checked send the intra-locus rewrite replaced with the direct
    /// call at this site, which keeps the send's id
    /// (`IntraLocusRewrite::send`): a `Call` whose image is a `Send`.
    RewrittenSend(SiteId),
    /// The stdlib analysis copy's site in the same walk position
    /// ([`crate::stdlib_bodies::identities`]).
    Stdlib(SiteId),
    /// The `std::api::local_context()` call the intra-locus rewrite
    /// generated for the checked send `send`, whose handler takes a
    /// context (`IntraLocusRewrite::context`): a `Call` at the send's
    /// span with no checked image.
    Generated { send: SiteId },
}

/// A checked site no merged site is: what a rewrite erased, and the send
/// whose subject it was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Erased {
    /// The subject of a send the topic rewrite turned into its wire
    /// literal (`TopicRewrite::erased`).
    TopicSubject { send: SiteId },
    /// The subject of a send the intra-locus rewrite replaced with a
    /// direct call (`IntraLocusRewrite::erased`).
    IntraLocusSubject { send: SiteId },
}

/// The correspondence of one lowering view: every merged site's image,
/// and every checked site the rewrites erased.
#[derive(Debug, Clone, Default)]
pub struct Correspondence {
    /// Merged site index → its image.
    images: BTreeMap<u32, Image>,
    /// Analysis-copy site index → the merged site it is.
    stdlib: BTreeMap<u32, SiteId>,
    /// Checked site index → the rewrite that erased it.
    erased: BTreeMap<u32, Erased>,
}

impl Correspondence {
    /// The image of the merged site `merged` carries, if the merged mint
    /// minted it.
    pub fn image(&self, merged: NodeId) -> Option<Image> {
        self.images.get(&merged.0).copied()
    }

    /// The checked site a merged node is: its own, or for the call that
    /// replaced a send, the send. `None` for a stdlib site and for the
    /// context call a rewrite generated.
    pub fn checked(&self, merged: NodeId) -> Option<SiteId> {
        match self.image(merged)? {
            Image::Checked(id) | Image::RewrittenSend(id) => Some(id),
            Image::Stdlib(_) | Image::Generated { .. } => None,
        }
    }

    /// The merged site the analysis copy's site `analysis` is.
    pub fn merged_of_stdlib(&self, analysis: SiteId) -> Option<SiteId> {
        self.stdlib.get(&analysis.index).copied()
    }

    /// Every merged site's image, by the merged site's index.
    pub fn images(&self) -> impl Iterator<Item = (u32, Image)> + '_ {
        self.images.iter().map(|(i, image)| (*i, *image))
    }

    /// Every checked site the rewrites erased, by its index.
    pub fn erased(&self) -> impl Iterator<Item = (u32, Erased)> + '_ {
        self.erased.iter().map(|(i, e)| (*i, *e))
    }
}

/// Derive the correspondence of a merged program, and hold it to the
/// law (module docs).
///
/// `checked` is the identities of the programs the verb checked (the
/// snapshot's mint); `merged` and `merged_ids` the lowering view's
/// program and its mint, whose stdlib is its tail; `intra_locus` and
/// `topic_rewrites` the two rewrites' relations. A site the law cannot
/// place is a compiler bug, returned as the message naming it.
pub fn correspond(
    checked: &Snapshot,
    merged: &Program,
    merged_ids: &Snapshot,
    intra_locus: &[IntraLocusRewrite],
    topic_rewrites: &[TopicRewrite],
) -> Result<Correspondence, String> {
    let mut out = Correspondence::default();
    let stdlib_seed = merged_ids.seeds.iter().position(|s| s == STDLIB_SEED);
    let rewritten_sends: BTreeSet<u32> =
        intra_locus.iter().filter(|r| !r.send.is_none()).map(|r| r.send.0).collect();

    // The context calls the intra-locus rewrite generated: each at its
    // send's span, which the call that replaced the send keeps.
    let mut context_calls: BTreeMap<(u32, u32), SiteId> = BTreeMap::new();
    for rw in intra_locus.iter().filter(|r| r.context && !r.send.is_none()) {
        let at = merged_ids.site_id(rw.send).and_then(|id| merged_ids.site(id));
        let (Some(at), Some(send)) = (at, checked.site_id(rw.send)) else {
            return Err(format!("the rewritten send {} has no call in the merged program", rw.send.0));
        };
        context_calls.insert((at.span.start.0, at.span.end.0), send);
    }

    // The user's sites: the checked site at the same index.
    let mut taken: BTreeSet<u32> = BTreeSet::new();
    for site in &merged_ids.sites {
        if stdlib_seed == Some(site.id.seed.0 as usize) {
            continue;
        }
        let Some(own) = checked.site_id(NodeId(site.id.index)).and_then(|id| checked.site(id)) else {
            match context_calls.remove(&(site.span.start.0, site.span.end.0)) {
                Some(send) if site.kind == SiteKind::Call => {
                    out.images.insert(site.id.index, Image::Generated { send });
                    continue;
                }
                _ => {
                    return Err(format!(
                        "the merged program's {:?} at {:?} (site {}) is no site of the checked program",
                        site.kind, site.span, site.id.index
                    ))
                }
            }
        };
        let image = if own.kind == site.kind && own.span == site.span {
            Image::Checked(own.id)
        } else if own.kind == SiteKind::Send && site.kind == SiteKind::Call && rewritten_sends.contains(&site.id.index) {
            Image::RewrittenSend(own.id)
        } else {
            return Err(format!(
                "the merged site {} is a {:?} at {:?}, not the {:?} at {:?} the check minted",
                site.id.index, site.kind, site.span, own.kind, own.span
            ));
        };
        taken.insert(own.id.index);
        out.images.insert(site.id.index, image);
    }
    if let Some((span, send)) = context_calls.into_iter().next() {
        return Err(format!("the context call of the rewritten send {send:?} at {span:?} is not in the merged program"));
    }

    // What the rewrites erased: each relation names its send's subject.
    let mut erased: BTreeMap<u32, Erased> = BTreeMap::new();
    let send_of = |id: NodeId| checked.site_id(id);
    for rw in topic_rewrites.iter().filter(|r| !r.erased.is_none()) {
        if let Some(send) = send_of(rw.site) {
            erased.insert(rw.erased.0, Erased::TopicSubject { send });
        }
    }
    for rw in intra_locus.iter().filter(|r| !r.erased.is_none()) {
        if let Some(send) = send_of(rw.send) {
            erased.insert(rw.erased.0, Erased::IntraLocusSubject { send });
        }
    }
    for site in &checked.sites {
        if taken.contains(&site.id.index) {
            continue;
        }
        match erased.get(&site.id.index) {
            Some(e) if site.kind == SiteKind::Use => {
                out.erased.insert(site.id.index, *e);
            }
            _ => {
                return Err(format!(
                    "the checked {:?} at {:?} (site {}) is no site of the merged program, and no rewrite erased it",
                    site.kind, site.span, site.id.index
                ))
            }
        }
    }

    // The stdlib's sites: the analysis copy's, by walk position.
    let pairing = stdlib_pairing(merged, merged_ids)?;
    let stdlib_sites = merged_ids.sites.iter().filter(|s| stdlib_seed == Some(s.id.seed.0 as usize)).count();
    if pairing.len() != stdlib_sites {
        return Err(format!(
            "the merged program has {stdlib_sites} stdlib sites, and {} of them pair with the analysis copy",
            pairing.len()
        ));
    }
    for (analysis, there) in pairing {
        if stdlib_seed != Some(there.seed.0 as usize) {
            return Err(format!("the stdlib's site {analysis:?} pairs with the user's merged site {there:?}"));
        }
        out.images.insert(there.index, Image::Stdlib(analysis));
        out.stdlib.insert(analysis.index, there);
    }
    Ok(out)
}

/// The analysis copy's sites paired, in walk order, with the merged
/// program's stdlib tail: analysis id → merged id.
fn stdlib_pairing(merged: &Program, merged_ids: &Snapshot) -> Result<BTreeMap<SiteId, SiteId>, String> {
    let (Some(analysis), Some(analysis_ids)) =
        (crate::stdlib_bodies::program(), crate::stdlib_bodies::identities())
    else {
        return Err("the stdlib analysis copy did not parse".to_string());
    };
    let n = analysis.items.len();
    if merged.items.len() < n {
        return Err(format!("the merged program holds {} items, fewer than the stdlib's {n}", merged.items.len()));
    }
    let walk = |items: &[TopDecl]| {
        let mut sites: Vec<(SiteKind, Span, NodeId)> = Vec::new();
        for item in items {
            for_each_site_in_item(item, &mut |kind, span, id| sites.push((kind, span, id)));
        }
        sites
    };
    let ours = walk(&analysis.items);
    let theirs = walk(&merged.items[merged.items.len() - n..]);
    if ours.len() != theirs.len() {
        return Err(format!(
            "the merged program's stdlib tail does not pair with the analysis copy: {} sites against {}",
            theirs.len(),
            ours.len()
        ));
    }
    let mut out = BTreeMap::new();
    for ((k1, s1, a), (k2, s2, m)) in ours.iter().zip(&theirs) {
        if k1 != k2 || s1 != s2 {
            return Err(format!("the stdlib pairing diverges: a {k1:?} at {s1:?} against a {k2:?} at {s2:?}"));
        }
        let (Some(a), Some(m)) = (analysis_ids.site_id(*a), merged_ids.site_id(*m)) else { continue };
        out.insert(a, m);
    }
    Ok(out)
}

//! The `placement` family (F.40 phase 3, P1): which thread domain each
//! instance runs in, as one table per snapshot.
//!
//! The design is `notes/f40-placement-correspondence.md` (hale-lang/hale#1296):
//! its § 1 fixes the schema this module states, its § 2 reads each of the
//! eight legacy producers the registry lists under `placement`, and its § 9
//! states the checkpoints a producer is verified against.
//!
//! ## Two universes
//!
//! Two stores mint the sites the table names: the snapshot's identities
//! (the seed's files and the imports linked into them) and the stdlib
//! analysis copy's ([`crate::stdlib_bodies::identities`]), which is minted
//! alone. Each numbers its seeds from 0 and its sites from 0, so a user
//! declaration and a stdlib locus can carry the same [`SiteId`]. A bare id
//! names a site only with the store that minted it, so every site the
//! table holds is a [`SiteRef`]: its universe beside its id, compared
//! universe first. No bare `SiteId` leaves the producer, and provenance
//! dispatches on the universe ([`provenance`]), never the other way.
//!
//! One stdlib declaration has three identities, and none stands in for
//! another: its **analysis** identity (the copy's `SiteId`, meaningful in
//! that store only), its **placement** identity (that id qualified by
//! [`SiteUniverse::StdlibAnalysis`], what the table's rows hold and every
//! consumer compares), and its **lowering** identity ([`DeclRef::lowered`],
//! the `__Std…` name lowering keys on). Lowering's own mint numbers the
//! merged program's stdlib past the user's sites, a third numbering;
//! [`join_lowering`] resolves the table's refs into it once, totally and
//! injectively.
//!
//! ## Templates, occurrences, incarnations
//!
//! An [`InstanceKey`] names a template: a construction literal of the
//! root (or an adapter's binding entry), the fields from its top, a
//! replica. An occurrence is one execution of the template's literal; the
//! table counts them ([`Construction::bound`], [`DynamicSite::bound`]) and
//! never keys them. An incarnation is the runtime's, and the table never
//! mints one.

use std::collections::{BTreeMap, BTreeSet};

use hale_graph::ids::SiteId;
use hale_syntax::ast::{flat_decls, NodeId, Program, TopDecl};
use hale_syntax::sites::{for_each_site_in_item, SiteKind};

use crate::entry::MainLocus;
use crate::snapshot::{Site, Snapshot};
use crate::ty::Ty;

// ------------------------------------------------------------ identity

/// Which store minted a site. The two stores number independently, each
/// from seed 0 and index 0, so a `SiteId` names a site only with its
/// universe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SiteUniverse {
    /// The snapshot's identities: the seed's files and the imports linked
    /// in.
    User,
    /// The stdlib analysis copy's ([`crate::stdlib_bodies::identities`]).
    StdlibAnalysis,
}

/// A site the table names: the universe that minted it and its id there.
/// Every site in the table is one. Equality, ordering and hashing compare
/// the universe first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SiteRef {
    pub universe: SiteUniverse,
    pub id: SiteId,
}

impl SiteRef {
    pub fn user(id: SiteId) -> SiteRef {
        SiteRef { universe: SiteUniverse::User, id }
    }

    pub fn stdlib(id: SiteId) -> SiteRef {
        SiteRef { universe: SiteUniverse::StdlibAnalysis, id }
    }
}

/// Provenance (kind, span, origin) of a site, from the store its universe
/// names and from no other. `user` is the snapshot's identities, `stdlib`
/// the analysis copy's.
pub fn provenance<'a>(site: SiteRef, user: &'a Snapshot, stdlib: &'a Snapshot) -> Option<&'a Site> {
    match site.universe {
        SiteUniverse::User => user.site(site.id),
        SiteUniverse::StdlibAnalysis => stdlib.site(site.id),
    }
}

/// Static instance identity: the scope that constructs the tree, the
/// field path from its top, the replica. A key names a template, never a
/// runtime occurrence.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InstanceKey {
    pub origin: Origin,
    /// The fields from the origin's top; empty is the top itself.
    pub path: Vec<Step>,
    /// `Some(i)` on a `replicas = K > 1` field and on every row nested
    /// under it.
    pub replica: Option<u32>,
}

/// The scope that constructs a static tower.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Origin {
    /// A root literal (`App { … }`): one template per literal site, the
    /// root at `[]`.
    Construction(SiteRef),
    /// An adapter literal in the root's `bindings { }`, by its binding
    /// entry's site (the adapter literal has no site of its own): built
    /// once, in the bindings prelude, however often the root is
    /// constructed.
    Binding(SiteRef),
}

/// One field of a key's path.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Step {
    pub field: String,
    /// The literal taken, when the field's initializer chooses among
    /// literals; `None` when it has one.
    pub alternative: Option<SiteRef>,
}

/// Where an instance comes from: a template of the static tower, or a
/// literal outside it (a method or fn body, a let-bound literal, an
/// accepted child), every occurrence of which shares the literal's rows.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Template {
    Static(InstanceKey),
    Dynamic { literal: SiteRef },
}

/// The locus declaration an instance realizes: an override literal's,
/// not the field's declared type.
///
/// Its canonical key is the declaration's site and the lowered name:
/// two specializations of one generic share [`DeclRef::site`] and differ
/// in [`DeclRef::lowered`], which the substitution decides. Equality,
/// ordering and hashing read those two and never [`DeclRef::args`],
/// whose type carries no total order.
#[derive(Debug, Clone)]
pub struct DeclRef {
    /// The locus declaration; for a monomorph, the template's.
    pub site: SiteRef,
    /// The substitution; empty unless generic.
    pub args: Vec<Ty>,
    /// The name lowering keys on (`__StdIoTcpListener`, `Cache_Int_String`).
    pub lowered: String,
}

impl PartialEq for DeclRef {
    fn eq(&self, other: &DeclRef) -> bool {
        self.site == other.site && self.lowered == other.lowered
    }
}

impl Eq for DeclRef {}

impl PartialOrd for DeclRef {
    fn partial_cmp(&self, other: &DeclRef) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for DeclRef {
    fn cmp(&self, other: &DeclRef) -> std::cmp::Ordering {
        (self.site, &self.lowered).cmp(&(other.site, &other.lowered))
    }
}

impl std::hash::Hash for DeclRef {
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        self.site.hash(h);
        self.lowered.hash(h);
    }
}

// ------------------------------------------------------------- bounds

/// How many occurrences of a template can be live at once.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Bound {
    /// At most one.
    Once,
    /// At most `n`.
    AtMost(u32),
    /// No static bound; the reason, which a count over the table renders
    /// as its uncertainty.
    Unbounded(String),
}

/// One literal of the root declaration: a construction template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Construction {
    /// The root literal; `Origin::Construction` of every key under it.
    pub literal: SiteRef,
    /// How many occurrences of this template can be live at once.
    pub bound: Bound,
}

// ------------------------------------------------------------ domains

/// A domain's index in [`PlacementTable::domains`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DomainId(pub u32);

/// A resolved set of logical CPUs, sorted and deduplicated: a `cores`
/// spec expanded, or a `node` / `l3` affinity resolved against the
/// root's `topology { }`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CoreSet(pub Vec<i64>);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainKind {
    /// The program's main thread; adds no thread.
    Main,
    /// One worker per name. A pool carries at most one affinity (rule 16).
    Pool { name: String, async_io: bool, affinity: Option<CoreSet> },
    /// One thread per anchor (per replica); the anchor's origin is the
    /// scope that creates it.
    Pinned { anchor: InstanceKey, affinity: Option<CoreSet>, numa_node: Option<i64> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Domain {
    pub id: DomainId,
    pub kind: DomainKind,
}

// --------------------------------------------------------------- rows

/// What decided a row's domain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// The root's `placement { }` entry for this field. The block is not
    /// a site of its own, so `decl` is the declaration that holds it (the
    /// root) and `entry` the entry's site.
    Entry { decl: SiteRef, entry: SiteRef },
    /// An adapter locus inline in `bindings { }`: pinned-equivalent.
    Binding { entry: SiteRef },
    /// Nested: the owner's domain.
    Inherited { from: InstanceKey },
    /// A root field with no entry, and the root itself: pool main.
    Default,
}

/// Where a row runs relative to its owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OwnerRelative {
    SameAsOwner,
    OffOwner,
}

/// One static instance of a construction template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceRow {
    /// The declaration actually built (an override literal's, not the
    /// field's declared type). `None` is a hole (invariant 6): the row
    /// still has its domain, and [`PlacementTable::holes`] says why.
    pub realizes: Option<DeclRef>,
    /// The literal that builds it (default or override); `None` = a hole.
    pub literal: Option<SiteRef>,
    /// `None` only at an origin's top (`[]` path).
    pub owner: Option<InstanceKey>,
    pub domain: DomainId,
    pub decided_by: Decision,
    pub owner_relative: OwnerRelative,
    /// On or under a step with an `alternative`: live only in occurrences
    /// that took it.
    pub guarded: bool,
}

/// The root lowering deploys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootRow {
    /// `EntryRow::lowering_root`, never `EntryRow::entry`.
    pub decl: MainLocus,
    /// The declaration, qualified as the user's.
    pub realizes: DeclRef,
    /// False when lowering deploys a module-nested `main`.
    pub is_entry: bool,
    /// Every literal of the root declaration, each a template.
    pub constructions: Vec<Construction>,
}

/// The scope that encloses a dynamic literal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Enclosing {
    /// A locus member's body (a method, a hook, a handler).
    Locus(DeclRef),
    /// A free fn, `fn main` included, by its declaration's site.
    Fn(SiteRef),
}

/// A locus literal outside the static tower: in a method or fn body,
/// let-bound, an `accept`ed child, or a root literal's own nested
/// literal. Its domains are the domains its enclosing scope runs in;
/// empty is unknown, never defaulted to main.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynamicSite {
    pub literal: SiteRef,
    /// The declaration the literal builds; `None` when it names none the
    /// producer resolves (a hole).
    pub realizes: Option<DeclRef>,
    pub enclosing: Enclosing,
    pub domains: BTreeSet<DomainId>,
    pub bound: Bound,
}

/// Where a hole sits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HoleAt {
    Instance(InstanceKey),
    Dynamic(SiteRef),
    Entry(SiteRef),
}

/// What the producer could not decide.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HoleKind {
    /// The realized declaration does not resolve (invariant 6); the
    /// written type or literal path.
    UnresolvedDeclaration { written: String },
    /// The field's initializer is not a literal, nor a choice among
    /// literals (a call, a name): its literal is unknown.
    UnenumerableInitializer,
    /// A generic declaration realized with no substitution the producer
    /// could read.
    UnresolvedArguments,
    /// A root `placement { }` entry that decides no field family.
    EntryDecidesNothing { field: String },
    /// A dynamic site whose enclosing scope's domains are unknown.
    UnknownDomains { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hole {
    pub at: HoleAt,
    pub kind: HoleKind,
}

impl HoleKind {
    /// The policy a consumer applies (the correspondence's invariant 6
    /// and § 8): unknown disables what needs it and is never a default.
    pub fn policy(&self) -> &'static str {
        match self {
            HoleKind::UnresolvedDeclaration { .. } | HoleKind::UnresolvedArguments => {
                "the row keeps the domain its owner or its entry gives it; only `realizes` is unknown, and a \
                 consumer that keys on the declaration treats the row as a hole of its own"
            }
            HoleKind::UnenumerableInitializer => {
                "the row keeps its domain; its literal is unknown, so nothing below it is enumerated, and a \
                 count over the subtree is an uncertainty"
            }
            HoleKind::EntryDecidesNothing { .. } => {
                "the entry is kept as a hole, never dropped: it names no field family a template builds"
            }
            HoleKind::UnknownDomains { .. } => {
                "unknown is never main: the direct-call gate stays deferred, the intra-locus rewrite treats \
                 the site as off its owner's thread, sync inference counts it a domain of its own, the \
                 ownership graph proves no same-tower edge, F.31 compares known domains only, the budget \
                 renders an uncertainty, and the model holes it"
            }
        }
    }
}

/// The placement table of one snapshot.
#[derive(Debug, Clone, Default)]
pub struct PlacementTable {
    /// `None`: lowering deploys no main locus.
    pub root: Option<RootRow>,
    /// Indexed by [`DomainId`]. `domains[0]` is always main.
    pub domains: Vec<Domain>,
    pub instances: BTreeMap<InstanceKey, InstanceRow>,
    /// Loci instantiated outside the static tower.
    pub dynamic: Vec<DynamicSite>,
    /// What the producer could not decide, each with its policy.
    pub holes: Vec<Hole>,
}

impl PlacementTable {
    /// The main domain.
    pub const MAIN: DomainId = DomainId(0);

    pub fn domain(&self, id: DomainId) -> &Domain {
        &self.domains[id.0 as usize]
    }
}

// ------------------------------------------------- the lowering join

/// A ref the lowering view resolves: a declaration, which joins by its
/// lowered name, or any other site, which joins by position.
#[derive(Debug, Clone, Copy)]
pub enum LoweringRef<'a> {
    Decl(&'a DeclRef),
    Site(SiteRef),
}

impl LoweringRef<'_> {
    fn site(&self) -> SiteRef {
        match self {
            LoweringRef::Decl(d) => d.site,
            LoweringRef::Site(s) => *s,
        }
    }
}

/// Resolve the table's refs into lowering's merged mint, once.
///
/// A `User` site joins directly: the resolved program keeps the ids the
/// bundle minted, so the merged mint holds the same index with the same
/// kind. A `StdlibAnalysis` site joins by position: the analysis copy and
/// the merged program's stdlib tail are clones of one parsed program
/// that no pass touches between the clone and the mint, so the two walks
/// visit the same sites in the same order. Each pair's kind and span are
/// asserted equal, and a `StdlibAnalysis` declaration must also join by
/// its lowered name to the same merged site. The resolution must be
/// total and injective over `refs`: a ref the merged program lacks, or
/// two refs resolving to one merged site, is a compiler bug, returned as
/// the message naming the ref.
///
/// `user` is the snapshot's identities; `merged` and `merged_ids` the
/// lowering view's program and mint.
pub fn join_lowering(
    refs: &[LoweringRef<'_>],
    user: &Snapshot,
    merged: &Program,
    merged_ids: &Snapshot,
) -> Result<BTreeMap<SiteRef, SiteId>, String> {
    let pairing = if refs.iter().any(|r| r.site().universe == SiteUniverse::StdlibAnalysis) {
        Some(stdlib_pairing(merged, merged_ids)?)
    } else {
        None
    };
    let mut out: BTreeMap<SiteRef, SiteId> = BTreeMap::new();
    let mut taken: BTreeMap<SiteId, SiteRef> = BTreeMap::new();
    for r in refs {
        let site = r.site();
        let resolved = match site.universe {
            SiteUniverse::User => {
                let own = user
                    .site(site.id)
                    .ok_or_else(|| format!("{site:?}: the snapshot did not mint this site"))?;
                let there = merged_ids
                    .site_id(NodeId(site.id.index))
                    .and_then(|id| merged_ids.site(id))
                    .ok_or_else(|| format!("{site:?}: the merged program has no site at this index"))?;
                if there.kind != own.kind || there.span != own.span {
                    return Err(format!(
                        "{site:?}: the merged site at this index is a {:?} at {:?}, not the {:?} at {:?} the \
                         snapshot minted",
                        there.kind, there.span, own.kind, own.span
                    ));
                }
                there.id
            }
            SiteUniverse::StdlibAnalysis => {
                let pairing = pairing.as_ref().expect("built for a stdlib ref");
                let by_position = *pairing
                    .get(&site.id)
                    .ok_or_else(|| format!("{site:?}: the analysis copy's site has no merged counterpart"))?;
                if let LoweringRef::Decl(d) = r {
                    let by_name = stdlib_decl_named(merged, merged_ids, &d.lowered).ok_or_else(|| {
                        format!("{site:?}: the merged stdlib declares no locus `{}`", d.lowered)
                    })?;
                    if by_name != by_position {
                        return Err(format!(
                            "{site:?}: `{}` joins by name to {by_name:?} and by position to {by_position:?}",
                            d.lowered
                        ));
                    }
                }
                by_position
            }
        };
        if let Some(prev) = out.get(&site) {
            if *prev != resolved {
                return Err(format!("{site:?} resolved twice, to {prev:?} and {resolved:?}"));
            }
            continue;
        }
        if let Some(other) = taken.insert(resolved, site) {
            return Err(format!("{other:?} and {site:?} both resolve to the merged site {resolved:?}"));
        }
        out.insert(site, resolved);
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
        let mut sites: Vec<(SiteKind, hale_syntax::Span, NodeId)> = Vec::new();
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

/// The merged site of the stdlib locus lowering names `lowered`.
fn stdlib_decl_named(merged: &Program, merged_ids: &Snapshot, lowered: &str) -> Option<SiteId> {
    let stdlib_seed = merged_ids.seeds.iter().position(|s| s == crate::snapshot::STDLIB_SEED)?;
    flat_decls(&merged.items).find_map(|item| match item {
        TopDecl::Locus(l) if l.name.name == lowered => {
            merged_ids.site_id(l.id).filter(|id| id.seed.0 as usize == stdlib_seed)
        }
        _ => None,
    })
}

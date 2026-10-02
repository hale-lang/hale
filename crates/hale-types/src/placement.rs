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
//! An [`InstanceKey`] names a template: a construction literal (the
//! root's, or one `fn main` builds), the entry's implicit construction of
//! the root, or an adapter's binding entry; the fields from its top; a
//! replica. An occurrence is one execution of the template's literal; the
//! table counts them ([`Construction::bound`], [`DynamicSite::bound`]) and
//! never keys them. An incarnation is the runtime's, and the table never
//! mints one.
//!
//! ## The producer
//!
//! [`derive_placement`] runs once per snapshot (`Snapshot::demand_placement`,
//! counted as `placement`), after the desugar sequence and the mint, so every
//! site it names is one a mint numbered. It is seeded from the entry row's
//! lowering root, never its entry: the table describes what lowering deploys.
//! Per construction literal of the root it walks the static tower (the
//! root's params fields as that literal builds them, then their params
//! fields, each with the literal that built it), with the adapters of the
//! root's `bindings { }` as origins of their own; then it lists every locus
//! literal outside the tower as a dynamic site, with the domains its
//! enclosing scope runs in and how many occurrences can be live.
//!
//! **The entry is a construction scope.** A root no literal builds (a
//! `main locus` that only carries claims, a library seed checked alone, a
//! `fn main` that builds other loci by verb) is the entry's implicit
//! template, [`Origin::Entry`], bound `Once`, its tower enumerated from the
//! declaration's defaults exactly as under a literal. The literals directly
//! in `fn main`'s body are templates too ([`PlacementTable::entry_literals`]),
//! not dynamic sites: `fn main` runs once, on main, so each is bound by its
//! statement's loop context alone.
//!
//! The pre-mint pool map sync inference reads
//! ([`crate::check::compute_pool_of_locus_type`], run per program before the
//! sequence and the mint) is untouched and never converted into rows.
//!
//! ## Where the schema departs from the design's § 1
//!
//! The tree forced these, and they are accepted:
//!
//! - [`Decision::Entry`] carries `{ decl, entry }`, not `{ block, entry }`:
//!   a `placement { }` block is not a minted site, so the declaration that
//!   holds it stands in.
//! - [`InstanceRow::realizes`] is an `Option`: invariant 6 needs a hole
//!   there, and `None` is that hole.
//! - An adapter's [`InstanceRow::literal`] is its binding entry's site, since
//!   the adapter literal has no site of its own; the entry's implicit
//!   template's top likewise carries the entry's site.
//! - [`Origin::Entry`] and [`PlacementTable::entry_literals`] (the entry as a
//!   construction scope), [`HoleKind::Reuse`] (a field that holds an
//!   instance built elsewhere claims none), and [`InstanceRow::built_by`]
//!   (a held instance's subtree is its source's rows projected into its
//!   holder's domain, each recording the source row it was built as) are
//!   the driver's rulings on
//!   what the shadow found, not in the design's text.
//!
//! The design's case 6 (a generic locus as a params field) never reaches
//! the producer's consumers: the checker refuses the shape, so G-2 is a
//! checker gap, not a correction. The table still keys the two
//! specializations apart over the refused program.

use std::collections::{BTreeMap, BTreeSet};

use hale_graph::ids::SiteId;
use hale_syntax::ast::{
    flat_decls, Block, ElseBranch, Expr, IfStmt, LValueSeg, LocusDecl, LocusMember, MatchArmBody, NodeId,
    OrDisposition, ParamInit, Pattern, PinAffinity, PlacementConstraint, PlacementSpec, Program, RecoveryModifier, Stmt,
    StructInit, TopDecl, TopologyBlock, TransportSpec, TypeDeclBody, TypeExpr,
};
use hale_syntax::sites::{for_each_site_in_item, SiteKind};

use crate::entry::{EntryRow, MainLocus};
use crate::resolve::TopScope;
use crate::snapshot::{Site, Snapshot};
use crate::symbol::Bundle;
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
    /// A literal: one of the root's (`App { … }`), one template per
    /// literal site with the root at `[]`, or one the entry `fn main`
    /// builds directly ([`PlacementTable::entry_literals`]), its locus at
    /// `[]`.
    Construction(SiteRef),
    /// An adapter literal in the root's `bindings { }`, by its binding
    /// entry's site (the adapter literal has no site of its own): built
    /// once, in the bindings prelude, however often the root is
    /// constructed.
    Binding(SiteRef),
    /// The entry itself, constructing a root no literal builds: the
    /// root's implicit template, by the entry's site (the top-level user
    /// `fn main`'s when there is one, else the root declaration's). Its
    /// tower is enumerated exactly as under a literal, from the
    /// declaration's defaults, and its bound is `Once`: the entry runs
    /// once per process.
    Entry(SiteRef),
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
    /// A root field with no entry, and a template's top: pool main.
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
    /// A held instance's row (a [`HoleKind::Reuse`]) or a row under one:
    /// the source template's row it was built as, on the domain where it
    /// was built, for the retention question. The rows under a held row
    /// are its source's rows projected (each realizes what its source row
    /// realizes), and they exist only where the source is linked. The
    /// instance moved into its holder's domain on the handoff, so this
    /// row's domain is the holder's, and the source's row
    /// ([`PlacementTable::handed_off`]) answers where it was built, not
    /// where it runs. `None` for a row its own template builds, and for a
    /// held row whose source is no template's row (a parameter, a field
    /// of `self`, a dynamic literal), which has nothing below it.
    pub built_by: Option<InstanceKey>,
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
    /// Every literal of the root declaration, each a template. Empty when
    /// no literal builds the root: its one template is then the entry's
    /// ([`Origin::Entry`]).
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
/// empty is unknown, never defaulted to main. Its declaration's params
/// subtree is not enumerated: a locus built only here has no rows, and
/// what its fields run on is the site's domains, under the same policy.
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
    /// literals, nor a name (a call): its literal is unknown.
    UnenumerableInitializer,
    /// The field is initialized from an existing instance (`self.roles`,
    /// a local name), the source expression as written: the row is the
    /// field's hold on an instance built elsewhere, not a new one. The
    /// hole sits on the held row alone; the rows under it are its
    /// source's actual rows, projected into the holder's domain, where
    /// the source is linked, and there are none where it is not.
    Reuse { source: String },
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
            HoleKind::Reuse { .. } => {
                "no new instance: the row claims none and anchors no domain of its own (it keeps its owner's, \
                 an entry naming the field decides nothing); the held instance moved into that domain on the \
                 handoff, so where the source template is linked its actual rows (declarations, overrides, \
                 descendants) are projected under the row, inherited, each naming the source row it was built \
                 as (`built_by`), and where it is not, the subtree is unknown and nothing below the row is \
                 enumerated (an instance of it runs in an unknown domain, which disables a proof or an \
                 optimization and is never main or pinned by default); a domain question skips the source's rows, and a count over the table skips the \
                 held row and its subtree"
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
    /// The templates the entry builds besides the root: every literal
    /// directly in the top-level user `fn main`'s body that names a locus
    /// other than the root and is no field of another literal there. Each
    /// is `Origin::Construction` of the keys under it, its locus at `[]`
    /// on main, and its bound is its statement's loop context.
    pub entry_literals: Vec<Construction>,
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

    /// The rows a held instance was built as: every key some row names
    /// as its [`InstanceRow::built_by`]. Each answers where its instance
    /// was built (the retention question); where the instance runs is
    /// its held row's domain, so a domain question skips these.
    pub fn handed_off(&self) -> BTreeSet<&InstanceKey> {
        self.instances.values().filter_map(|r| r.built_by.as_ref()).collect()
    }
}

// ----------------------------------------------------- the producer

/// The `placement` family's producer: one table over the bundle's checked
/// programs and the stdlib analysis copy, seeded from the entry row's
/// lowering root. `bundle` must be minted (its identities hold the
/// programs' sites); a bundle nothing minted names no site and gets an
/// empty table.
pub fn derive_placement(bundle: &Bundle<'_>, top: &TopScope, entry: &EntryRow) -> PlacementTable {
    build(bundle, top, entry)
}

fn build<'a>(bundle: &'a Bundle<'a>, top: &'a TopScope, entry: &EntryRow) -> PlacementTable {
    let stdlib = match (crate::stdlib_bodies::program(), crate::stdlib_bodies::identities()) {
        (Some(p), Some(ids)) => Some((p, ids)),
        _ => None,
    };
    let decls = Decls::of(bundle, stdlib);
    let scopes = Scopes::of(bundle, stdlib, &decls);
    let mut b = Builder {
        decls: &decls,
        top,
        user_ids: &bundle.snapshot,
        table: PlacementTable {
            domains: vec![Domain { id: PlacementTable::MAIN, kind: DomainKind::Main }],
            ..PlacementTable::default()
        },
        pools: BTreeMap::new(),
        static_literals: BTreeSet::new(),
        held: Vec::new(),
    };
    if let Some(root) = entry.lowering_root.as_ref() {
        b.root(bundle, entry, root, &scopes);
    }
    b.entry_literals(&scopes);
    b.handoffs();
    b.dynamic_sites(&scopes);
    b.table
}

/// One locus declaration the producer can realize.
struct DeclEntry<'a> {
    decl: &'a LocusDecl,
    site: SiteRef,
    /// The modules that enclose it, outermost first.
    module: Vec<String>,
}

/// What a written name denotes.
enum Named<'d, 'a> {
    Locus(&'d DeclEntry<'a>),
    /// An interface (or perspective): a contract-typed slot, which holds
    /// whichever implementation was built.
    Contract,
    /// A type, a primitive spelled as a name, or anything else that is no
    /// locus: not a placement fact at all.
    NotALocus,
    /// A name the producer cannot resolve.
    Unknown,
}

/// The declarations of both universes, by the names lowering reads.
struct Decls<'a> {
    user: Vec<DeclEntry<'a>>,
    stdlib: Vec<DeclEntry<'a>>,
    user_by_name: BTreeMap<&'a str, Vec<usize>>,
    stdlib_by_name: BTreeMap<&'a str, usize>,
    /// `type A = T;` targets, each universe's.
    user_aliases: BTreeMap<&'a str, &'a TypeExpr>,
    stdlib_aliases: BTreeMap<&'a str, &'a TypeExpr>,
    /// Interface and perspective names, both universes.
    contracts: BTreeSet<&'a str>,
    /// Every other declared type name (struct and enum types), both
    /// universes: a literal naming one builds no locus.
    types: BTreeSet<&'a str>,
    /// `alias::Name` → the merged declaration name.
    renames: BTreeMap<String, String>,
    /// Every free fn, by universe and name.
    fns: BTreeSet<(SiteUniverse, String)>,
}

impl<'a> Decls<'a> {
    fn of(bundle: &Bundle<'a>, stdlib: Option<(&'a Program, &'a Snapshot)>) -> Decls<'a> {
        let mut d = Decls {
            user: Vec::new(),
            stdlib: Vec::new(),
            user_by_name: BTreeMap::new(),
            stdlib_by_name: BTreeMap::new(),
            user_aliases: BTreeMap::new(),
            stdlib_aliases: BTreeMap::new(),
            contracts: BTreeSet::new(),
            types: BTreeSet::new(),
            renames: bundle.import_renames.iter().map(|(segs, m)| (segs.join("::"), m.clone())).collect(),
            fns: BTreeSet::new(),
        };
        for program in bundle.programs.values() {
            d.collect(&program.items, &bundle.snapshot, SiteUniverse::User, &mut Vec::new());
        }
        if let Some((program, ids)) = stdlib {
            d.collect(&program.items, ids, SiteUniverse::StdlibAnalysis, &mut Vec::new());
        }
        d
    }

    fn collect(&mut self, items: &'a [TopDecl], ids: &Snapshot, universe: SiteUniverse, module: &mut Vec<String>) {
        for item in items {
            match item {
                TopDecl::Locus(l) => {
                    let Some(id) = ids.site_id(l.id) else { continue };
                    let entry = DeclEntry { decl: l, site: SiteRef { universe, id }, module: module.clone() };
                    match universe {
                        SiteUniverse::User => {
                            self.user_by_name.entry(l.name.name.as_str()).or_default().push(self.user.len());
                            self.user.push(entry);
                        }
                        SiteUniverse::StdlibAnalysis => {
                            self.stdlib_by_name.entry(l.name.name.as_str()).or_insert(self.stdlib.len());
                            self.stdlib.push(entry);
                        }
                    }
                }
                TopDecl::Type(t) => match &t.body {
                    TypeDeclBody::Alias(te) if t.generics.is_empty() => {
                        let aliases = match universe {
                            SiteUniverse::User => &mut self.user_aliases,
                            SiteUniverse::StdlibAnalysis => &mut self.stdlib_aliases,
                        };
                        aliases.entry(t.name.name.as_str()).or_insert(te);
                    }
                    _ => {
                        self.types.insert(t.name.name.as_str());
                    }
                },
                TopDecl::Interface(i) => {
                    self.contracts.insert(i.name.name.as_str());
                }
                TopDecl::Fn(fd) => {
                    self.fns.insert((universe, fd.name.name.clone()));
                }
                TopDecl::Perspective(p) => {
                    self.contracts.insert(p.name.name.as_str());
                }
                TopDecl::Module(m) => {
                    module.push(m.name.name.clone());
                    self.collect(&m.items, ids, universe, module);
                    module.pop();
                }
                _ => {}
            }
        }
    }

    fn entry(&self, universe: SiteUniverse, i: usize) -> &DeclEntry<'a> {
        match universe {
            SiteUniverse::User => &self.user[i],
            SiteUniverse::StdlibAnalysis => &self.stdlib[i],
        }
    }

    /// The user declaration named `name`: the top-level one when a module
    /// declares the name too, as the flat scope reads it.
    fn user_named(&self, name: &str) -> Option<&DeclEntry<'a>> {
        let all = self.user_by_name.get(name)?;
        all.iter().map(|i| &self.user[*i]).min_by_key(|e| e.module.len())
    }

    fn stdlib_named(&self, name: &str) -> Option<&DeclEntry<'a>> {
        self.stdlib_by_name.get(name).map(|i| &self.stdlib[*i])
    }

    /// What `segs` denotes, written in `from`'s universe.
    fn resolve(&self, segs: &[&str], from: SiteUniverse) -> Named<'_, 'a> {
        self.resolve_at(segs, from, 0)
    }

    fn resolve_at(&self, segs: &[&str], from: SiteUniverse, depth: u32) -> Named<'_, 'a> {
        if depth > 16 {
            return Named::Unknown;
        }
        match segs {
            [] => Named::Unknown,
            ["std", ..] => match crate::ownership::stdlib_mangled_for_path(segs) {
                Some(mangled) => match self.stdlib_named(mangled) {
                    Some(e) => Named::Locus(e),
                    None if self.contracts.contains(mangled) => Named::Contract,
                    None => Named::NotALocus,
                },
                None => Named::Unknown,
            },
            [name] => {
                let aliases = match from {
                    SiteUniverse::User => &self.user_aliases,
                    SiteUniverse::StdlibAnalysis => &self.stdlib_aliases,
                };
                if let Some(TypeExpr::Named { path, .. }) = aliases.get(name) {
                    let target: Vec<&str> = path.segments.iter().map(|s| s.name.as_str()).collect();
                    return self.resolve_at(&target, from, depth + 1);
                }
                let found = match from {
                    SiteUniverse::User => self.user_named(name).or_else(|| self.stdlib_named(name)),
                    SiteUniverse::StdlibAnalysis => self.stdlib_named(name).or_else(|| self.user_named(name)),
                };
                match found {
                    Some(e) => Named::Locus(e),
                    None if self.contracts.contains(name) => Named::Contract,
                    None if self.types.contains(name) || aliases.contains_key(name) => Named::NotALocus,
                    None => Named::Unknown,
                }
            }
            _ => {
                if let Some(mangled) = self.renames.get(&segs.join("::")) {
                    let mangled = mangled.clone();
                    return self.resolve_at(&[mangled.as_str()], SiteUniverse::User, depth + 1);
                }
                // A module-qualified path: the declaration of that name
                // whose enclosing modules end with the path's head.
                let (last, head) = segs.split_last().expect("non-empty");
                let found = self.user_by_name.get(last).and_then(|all| {
                    all.iter().map(|i| &self.user[*i]).find(|e| {
                        e.module.len() >= head.len()
                            && e.module[e.module.len() - head.len()..].iter().map(String::as_str).eq(head.iter().copied())
                    })
                });
                match found {
                    Some(e) => Named::Locus(e),
                    None => Named::Unknown,
                }
            }
        }
    }
}

fn segments(path: &hale_syntax::ast::QualifiedName) -> Vec<&str> {
    path.segments.iter().map(|s| s.name.as_str()).collect()
}

fn written(segs: &[&str]) -> String {
    segs.join("::")
}

/// `ty` with `subst`'s type parameters replaced.
fn substitute(ty: &TypeExpr, subst: &BTreeMap<String, TypeExpr>) -> TypeExpr {
    match ty {
        TypeExpr::Named { path, generic_args, .. }
            if path.segments.len() == 1 && generic_args.is_empty() && subst.contains_key(&path.segments[0].name) =>
        {
            subst[&path.segments[0].name].clone()
        }
        TypeExpr::Named { path, generic_args, span } => TypeExpr::Named {
            path: path.clone(),
            generic_args: generic_args.iter().map(|a| substitute(a, subst)).collect(),
            span: *span,
        },
        other => other.clone(),
    }
}

/// The literals an initializer chooses among: the literal itself, or
/// the literal arm of each branch of an `if` / `match` (and a block's
/// tail). `None` when any arm is something else (a call, a name).
fn alternatives(e: &Expr) -> Option<Vec<&Expr>> {
    fn block<'e>(b: &'e Block, out: &mut Vec<&'e Expr>) -> Option<()> {
        if !b.stmts.is_empty() {
            return None;
        }
        arms(b.tail.as_deref()?, out)
    }
    fn if_chain<'e>(i: &'e IfStmt, out: &mut Vec<&'e Expr>) -> Option<()> {
        block(&i.then_block, out)?;
        match i.else_block.as_deref()? {
            ElseBranch::Else(b) => block(b, out),
            ElseBranch::ElseIf(n) => if_chain(n, out),
        }
    }
    fn arms<'e>(e: &'e Expr, out: &mut Vec<&'e Expr>) -> Option<()> {
        match e {
            Expr::Struct { .. } => {
                out.push(e);
                Some(())
            }
            Expr::Block(b) => block(b, out),
            Expr::If(i) => if_chain(i, out),
            Expr::Match(m) => {
                for arm in &m.arms {
                    match &arm.body {
                        MatchArmBody::Expr(e) => arms(e, out)?,
                        MatchArmBody::Block(b) => block(b, out)?,
                    }
                }
                Some(())
            }
            _ => None,
        }
    }
    let mut out = Vec::new();
    arms(e, &mut out)?;
    Some(out)
}

/// The existing instance an initializer names, as written: a name or a
/// field chain on one (`self.roles`, `r`). `None` for anything else.
fn reused_source(e: &Expr) -> Option<String> {
    match e {
        Expr::Ident(i) => Some(i.name.clone()),
        Expr::KwSelf(_) => Some("self".to_string()),
        Expr::Field { receiver, name, .. } => Some(format!("{}.{}", reused_source(receiver)?, name.name)),
        _ => None,
    }
}

/// A resolved CPU set for an affinity, against the root's topology.
fn core_set(affinity: &PinAffinity, topology: Option<&TopologyBlock>) -> Option<CoreSet> {
    let cores = match affinity {
        PinAffinity::Any => return None,
        PinAffinity::Cores(spec) => spec.expand(),
        PinAffinity::Node(n) => topology?.node_cores(*n)?,
        PinAffinity::L3(name) => topology?.l3_cores(&name.name)?,
    };
    Some(CoreSet(cores))
}

/// The NUMA node an affinity binds a pinned instance's arena to.
fn numa_node(affinity: &PinAffinity, topology: Option<&TopologyBlock>) -> Option<i64> {
    match affinity {
        PinAffinity::Node(n) => topology?.node_cores(*n).map(|_| *n),
        PinAffinity::L3(name) => topology?.node_of_l3(&name.name),
        PinAffinity::Any | PinAffinity::Cores(_) => None,
    }
}

/// The root's placement decisions, by field.
struct RootEntries<'a> {
    decl: SiteRef,
    entries: BTreeMap<&'a str, (&'a hale_syntax::ast::PlacementEntry, SiteRef)>,
    topology: Option<&'a TopologyBlock>,
}

/// One field's position in the walk: the owner's key and domain, and
/// whether the owner is live only under an alternative.
struct Owner<'k> {
    key: &'k InstanceKey,
    domain: DomainId,
    guarded: bool,
}

struct Builder<'d, 'a> {
    decls: &'d Decls<'a>,
    top: &'d TopScope,
    user_ids: &'d Snapshot,
    table: PlacementTable,
    /// Pool domains by name.
    pools: BTreeMap<String, DomainId>,
    /// Every literal the static tower visited: not a dynamic site.
    static_literals: BTreeSet<SiteRef>,
    /// Each held row whose source names a literal, with that literal's
    /// template top: projected once every template is built ([`Builder::handoffs`]).
    held: Vec<(InstanceKey, InstanceKey)>,
}

/// The names a scope binds: each to the locus literal it is bound to,
/// when bound once, immutably, to one; `None` otherwise.
type Lets = BTreeMap<String, Option<SiteRef>>;

impl<'d, 'a> Builder<'d, 'a> {
    fn new_domain(&mut self, kind: DomainKind) -> DomainId {
        let id = DomainId(self.table.domains.len() as u32);
        self.table.domains.push(Domain { id, kind });
        id
    }

    fn pool(&mut self, name: &str, async_io: bool, affinity: Option<CoreSet>) -> DomainId {
        if let Some(id) = self.pools.get(name).copied() {
            if let DomainKind::Pool { async_io: a, affinity: aff, .. } = &mut self.table.domains[id.0 as usize].kind {
                *a |= async_io;
                if aff.is_none() {
                    *aff = affinity;
                }
            }
            return id;
        }
        let id = self.new_domain(DomainKind::Pool { name: name.to_string(), async_io, affinity });
        self.pools.insert(name.to_string(), id);
        id
    }

    fn decl_ref(&self, e: &DeclEntry<'a>, declared: Option<&TypeExpr>) -> (DeclRef, bool) {
        let name = e.decl.name.name.clone();
        if e.decl.generics.is_empty() {
            return (DeclRef { site: e.site, args: Vec::new(), lowered: name }, true);
        }
        let args: Vec<TypeExpr> = match declared {
            Some(TypeExpr::Named { generic_args, .. }) if generic_args.len() == e.decl.generics.len() => {
                generic_args.clone()
            }
            _ => return (DeclRef { site: e.site, args: Vec::new(), lowered: name }, false),
        };
        let lowered = crate::mangle::mangle_generic_name(&name, &args).unwrap_or(name);
        let tys = args.iter().map(|a| crate::resolve::resolve_type_expr(a, &self.top.names)).collect();
        (DeclRef { site: e.site, args: tys, lowered }, true)
    }

    fn hole(&mut self, at: HoleAt, kind: HoleKind) {
        self.table.holes.push(Hole { at, kind });
    }

    fn root(&mut self, bundle: &Bundle<'a>, entry: &EntryRow, root: &MainLocus, scopes: &Scopes<'a>) {
        let Some(site) = root.site.map(SiteRef::user) else { return };
        let Some(decl) = self.decls.user.iter().find(|e| e.site == site) else { return };
        let decl: &'d DeclEntry<'a> = decl;
        let l = decl.decl;
        let is_entry = entry.entry().is_some_and(|e| e.site == root.site);
        let mut entries = RootEntries { decl: site, entries: BTreeMap::new(), topology: None };
        for m in &l.members {
            match m {
                LocusMember::Placement(pb) => {
                    for e in &pb.entries {
                        if let Some(id) = bundle.snapshot.site_id(e.id) {
                            entries.entries.entry(e.field.name.as_str()).or_insert((e, SiteRef::user(id)));
                        }
                    }
                }
                LocusMember::Topology(tb) => entries.topology = Some(tb),
                _ => {}
            }
        }
        let mut constructions: Vec<(SiteRef, &'a [StructInit], Bound, &Lets)> = Vec::new();
        for s in &scopes.scopes {
            for lit in &s.literals {
                if lit.decl == Some(site) && s.universe == SiteUniverse::User {
                    constructions.push((lit.site, lit.inits, scopes.bound(s, lit.in_loop), &s.lets));
                }
            }
        }
        constructions.sort_by_key(|(s, _, _, _)| *s);
        let realizes = DeclRef { site, args: Vec::new(), lowered: l.name.name.clone() };
        self.table.root = Some(RootRow {
            decl: root.clone(),
            realizes: realizes.clone(),
            is_entry,
            constructions: constructions
                .iter()
                .map(|(literal, _, bound, _)| Construction { literal: *literal, bound: bound.clone() })
                .collect(),
        });
        // A root no literal builds is the entry's implicit template, from
        // the declaration's defaults; its top's literal is the entry's
        // site, as an adapter's is its binding entry's.
        let templates: Vec<(Origin, SiteRef, &'a [StructInit], Option<&Lets>)> = if constructions.is_empty() {
            let entry_site = scopes.entry_fn().unwrap_or(site);
            vec![(Origin::Entry(entry_site), entry_site, &[], None)]
        } else {
            constructions
                .iter()
                .map(|(literal, inits, _, lets)| (Origin::Construction(*literal), *literal, *inits, Some(*lets)))
                .collect()
        };
        for (origin, literal, inits, lets) in templates {
            if matches!(origin, Origin::Construction(_)) {
                self.static_literals.insert(literal);
            }
            let key = InstanceKey { origin, path: Vec::new(), replica: None };
            self.top(&key, decl, realizes.clone(), literal, inits, Some(&entries), lets);
            // Invariant 5: every entry decides a field family in this
            // template, or it is a hole.
            for (field, (_, entry_site)) in &entries.entries {
                let decided = self.table.instances.iter().any(|(k, r)| {
                    k.origin == origin
                        && k.path.len() == 1
                        && k.path[0].field == *field
                        && matches!(r.decided_by, Decision::Entry { .. })
                });
                if !decided && !self.table.holes.iter().any(|h| h.at == HoleAt::Entry(*entry_site)) {
                    self.hole(HoleAt::Entry(*entry_site), HoleKind::EntryDecidesNothing { field: field.to_string() });
                }
            }
        }
        // The adapters of the root's `bindings { }`: an origin each,
        // built once in the bindings prelude, pinned-equivalent.
        for m in &l.members {
            let LocusMember::Bindings(bb) = m else { continue };
            for e in &bb.entries {
                let TransportSpec::Adapter { locus, inits, .. } = &e.transport else { continue };
                let Some(entry_site) = bundle.snapshot.site_id(e.id).map(SiteRef::user) else { continue };
                let key = InstanceKey { origin: Origin::Binding(entry_site), path: Vec::new(), replica: None };
                let domain = self.new_domain(DomainKind::Pinned { anchor: key.clone(), affinity: None, numa_node: None });
                let realized = match self.decls.resolve(&[locus.name.as_str()], SiteUniverse::User) {
                    Named::Locus(d) => Some(d),
                    _ => None,
                };
                if realized.is_none() {
                    self.hole(
                        HoleAt::Instance(key.clone()),
                        HoleKind::UnresolvedDeclaration { written: locus.name.clone() },
                    );
                }
                self.table.instances.insert(
                    key.clone(),
                    InstanceRow {
                        realizes: realized.map(|d| self.decl_ref(d, None).0),
                        literal: Some(entry_site),
                        owner: None,
                        domain,
                        decided_by: Decision::Binding { entry: entry_site },
                        owner_relative: OwnerRelative::SameAsOwner,
                        guarded: false,
                        built_by: None,
                    },
                );
                if let Some(d) = realized {
                    let owner = Owner { key: &key, domain, guarded: false };
                    let mut stack = vec![d.site];
                    self.fields(d, inits, &BTreeMap::new(), &owner, None, None, &mut stack);
                }
            }
        }
    }

    /// A template's top row on main, and its tower below it. `root` is set
    /// for the root's templates, whose fields a `placement { }` entry
    /// decides; `lets` are the names bound in the scope the literal is
    /// written in.
    #[allow(clippy::too_many_arguments)]
    fn top(
        &mut self,
        key: &InstanceKey,
        decl: &'d DeclEntry<'a>,
        realizes: DeclRef,
        literal: SiteRef,
        inits: &'a [StructInit],
        root: Option<&RootEntries<'a>>,
        lets: Option<&Lets>,
    ) {
        self.table.instances.insert(
            key.clone(),
            InstanceRow {
                realizes: Some(realizes),
                literal: Some(literal),
                owner: None,
                domain: PlacementTable::MAIN,
                decided_by: Decision::Default,
                owner_relative: OwnerRelative::SameAsOwner,
                guarded: false,
                built_by: None,
            },
        );
        let owner = Owner { key, domain: PlacementTable::MAIN, guarded: false };
        let mut stack = vec![decl.site];
        self.fields(decl, inits, &BTreeMap::new(), &owner, root, lets, &mut stack);
    }

    /// The templates the entry builds besides the root: each locus
    /// literal directly in the top-level user `fn main`'s body that no
    /// template has visited already (the root's literals, a field of an
    /// earlier literal there), bound by its statement's loop context.
    fn entry_literals(&mut self, scopes: &Scopes<'a>) {
        let Some(main) = scopes.scopes.iter().find(|s| matches!(s.kind, ScopeKind::Fn { is_main: true, .. })) else {
            return;
        };
        for lit in &main.literals {
            if self.static_literals.contains(&lit.site) {
                continue;
            }
            let Some(d) = lit.decl.and_then(|d| scopes.decl_entry(self.decls, d)) else { continue };
            self.static_literals.insert(lit.site);
            let key = InstanceKey { origin: Origin::Construction(lit.site), path: Vec::new(), replica: None };
            let (realizes, args_known) = self.decl_ref(d, None);
            if !args_known {
                self.hole(HoleAt::Instance(key.clone()), HoleKind::UnresolvedArguments);
            }
            self.top(&key, d, realizes, lit.site, lit.inits, None, Some(&main.lets));
            self.table.entry_literals.push(Construction { literal: lit.site, bound: scopes.bound(main, lit.in_loop) });
        }
    }

    /// Each held row whose source names a template's literal becomes the
    /// projection of that template's rows into its holder's domain: the
    /// held row realizes what the source's top realizes, and every row of
    /// the source is copied under it (its declaration, its literal, its
    /// holes and its descendants), inherited, each naming its own source
    /// row as `built_by`. A held row with no template to project keeps its
    /// `Reuse` hole and nothing below it. Run once every template is
    /// built: a source is usually one of `fn main`'s literals, which are
    /// built after the root's. A source that holds an instance itself is
    /// projected after that instance is, so its copy carries the subtree.
    fn handoffs(&mut self) {
        let mut pending = std::mem::take(&mut self.held);
        while let Some(i) =
            pending.iter().position(|(_, source)| !pending.iter().any(|(h, _)| h.origin == source.origin))
        {
            let (held, source) = pending.remove(i);
            self.project(&held, &source);
        }
    }

    /// The rows of `source`'s template, projected under `held`.
    fn project(&mut self, held: &InstanceKey, source: &InstanceKey) {
        let Some(top) = self.table.instances.get(source).cloned() else { return };
        let Some(holder) = self.table.instances.get(held).cloned() else { return };
        let at = |k: &InstanceKey| InstanceKey {
            origin: held.origin,
            path: held.path.iter().chain(&k.path[source.path.len()..]).cloned().collect(),
            replica: k.replica.or(held.replica),
        };
        let rows: Vec<(InstanceKey, InstanceRow)> = self
            .table
            .instances
            .iter()
            .filter(|(k, _)| k.origin == source.origin && k.path.len() > source.path.len() && k.path.starts_with(&source.path))
            .map(|(k, r)| (k.clone(), r.clone()))
            .collect();
        // The held row's own holes are the declared type's reading; the
        // source's top says what was built.
        self.table.holes.retain(|h| {
            h.at != HoleAt::Instance(held.clone()) || matches!(h.kind, HoleKind::Reuse { .. })
        });
        let mut holes: Vec<Hole> = Vec::new();
        for (from, to) in std::iter::once((source.clone(), held.clone())).chain(rows.iter().map(|(k, _)| (k.clone(), at(k)))) {
            for h in &self.table.holes {
                if h.at == HoleAt::Instance(from.clone()) {
                    holes.push(Hole { at: HoleAt::Instance(to.clone()), kind: h.kind.clone() });
                }
            }
        }
        self.table.holes.extend(holes);
        if let Some(r) = self.table.instances.get_mut(held) {
            r.realizes = top.realizes;
            r.built_by = Some(source.clone());
        }
        for (k, r) in rows {
            let owner = r.owner.as_ref().map(at).unwrap_or_else(|| held.clone());
            self.table.instances.insert(
                at(&k),
                InstanceRow {
                    realizes: r.realizes,
                    literal: r.literal,
                    decided_by: Decision::Inherited { from: owner.clone() },
                    owner: Some(owner),
                    domain: holder.domain,
                    owner_relative: OwnerRelative::SameAsOwner,
                    guarded: holder.guarded || r.guarded,
                    built_by: Some(k),
                },
            );
        }
    }

    /// The params fields of `decl` as a literal with `inits` builds it,
    /// each a row under `owner`, and their fields below them. `root` is
    /// set when `decl` is the root and `owner` a construction's top: its
    /// fields are the ones a `placement { }` entry decides. `lets` is set
    /// while `inits` were written in the template's scope, so a name among
    /// them is one of its locals.
    ///
    /// A field held from an existing instance ([`HoleKind::Reuse`]) is a
    /// row with no literal in its owner's domain, and the walk stops
    /// there: the instance was built elsewhere, so its declaration's
    /// defaults say nothing about it. A source that names a literal is
    /// recorded for [`Builder::handoffs`], which projects that template's
    /// rows under the held row.
    #[allow(clippy::too_many_arguments)]
    fn fields(
        &mut self,
        decl: &'d DeclEntry<'a>,
        inits: &'a [StructInit],
        subst: &BTreeMap<String, TypeExpr>,
        owner: &Owner<'_>,
        root: Option<&RootEntries<'a>>,
        lets: Option<&Lets>,
        stack: &mut Vec<SiteRef>,
    ) {
        if stack.len() > 64 {
            return;
        }
        let universe = decl.site.universe;
        for m in &decl.decl.members {
            let LocusMember::Params(pb) = m else { continue };
            for p in &pb.params {
                let field = p.name.name.as_str();
                let written_here = inits.iter().find(|i| i.name.name == field).map(|i| &i.value);
                let init: Option<&'a Expr> = written_here.or(match &p.init {
                    ParamInit::Value(e) => Some(e),
                    ParamInit::Inferred => None,
                });
                let declared: Option<TypeExpr> = p.ty.as_ref().map(|t| substitute(t, subst));
                let declared_named = match &declared {
                    Some(TypeExpr::Named { path, .. }) => self.decls.resolve(&segments(path), universe),
                    _ => Named::NotALocus,
                };
                let declared_is_slot = matches!(declared_named, Named::Locus(_) | Named::Contract);
                let alts = init.and_then(alternatives);
                let reused = if alts.is_none() { init.and_then(reused_source) } else { None };
                // The literal a held instance was built from: a local of
                // the template's scope bound to one.
                let source = match (&reused, written_here, lets) {
                    (Some(name), Some(_), Some(lets)) => lets.get(name).copied().flatten(),
                    _ => None,
                };
                // Each alternative: (its literal's site, the declaration
                // it names, its inits, the path as written).
                let mut built: Vec<(Option<SiteRef>, Option<&'d DeclEntry<'a>>, &'a [StructInit], String)> = Vec::new();
                match &alts {
                    Some(lits) => {
                        let mut any_locus = false;
                        let mut any_unknown = false;
                        for lit in lits {
                            let Expr::Struct { path, inits, id, .. } = lit else { continue };
                            let segs = segments(path);
                            let site = self.site_of(*id, universe);
                            match self.decls.resolve(&segs, universe) {
                                Named::Locus(d) => {
                                    any_locus = true;
                                    built.push((site, Some(d), inits, written(&segs)));
                                }
                                Named::Unknown | Named::Contract => {
                                    any_unknown = true;
                                    built.push((site, None, inits, written(&segs)));
                                }
                                Named::NotALocus => {}
                            }
                        }
                        if !any_locus && !(any_unknown && declared_is_slot) {
                            continue;
                        }
                    }
                    None if declared_is_slot => {}
                    None => continue,
                }
                let choice = built.len() > 1;
                let entry = root.and_then(|r| r.entries.get(field).map(|(e, s)| (r, *e, *s)));
                if alts.is_none() {
                    built.push((None, None, &[], String::new()));
                }
                for (literal, realized, lit_inits, path_written) in built {
                    if let Some(l) = literal {
                        self.static_literals.insert(l);
                    }
                    let step = Step { field: field.to_string(), alternative: if choice { literal } else { None } };
                    let guarded = owner.guarded || choice;
                    // The field's family: one row, or one per replica.
                    let mut family: Vec<(Option<u32>, DomainId, Decision)> = Vec::new();
                    match entry {
                        // A held instance anchors nothing: it keeps its
                        // owner's domain whatever an entry says.
                        _ if reused.is_some() => {
                            family.push((owner.key.replica, owner.domain, Decision::Inherited { from: owner.key.clone() }))
                        }
                        Some((r, e, entry_site)) => {
                            let decision = Decision::Entry { decl: r.decl, entry: entry_site };
                            match &e.spec {
                                PlacementSpec::Pinned { affinity, replicas } => {
                                    let k = replicas.unwrap_or(1).max(1) as u32;
                                    let cores = core_set(affinity, r.topology);
                                    let node = numa_node(affinity, r.topology);
                                    for i in 0..k {
                                        let replica = (k > 1).then_some(i);
                                        let affinity = match (&cores, k > 1) {
                                            (Some(CoreSet(c)), true) if !c.is_empty() => {
                                                Some(CoreSet(vec![c[i as usize % c.len()]]))
                                            }
                                            (cores, _) => cores.clone(),
                                        };
                                        let anchor = InstanceKey {
                                            origin: owner.key.origin,
                                            path: vec![step.clone()],
                                            replica,
                                        };
                                        let d = self.new_domain(DomainKind::Pinned { anchor, affinity, numa_node: node });
                                        family.push((replica, d, decision.clone()));
                                    }
                                }
                                PlacementSpec::Cooperative { pool, affinity } => {
                                    let d = match pool.as_ref().map(|p| p.name.as_str()) {
                                        None | Some("main") => PlacementTable::MAIN,
                                        Some(name) => {
                                            let async_io =
                                                e.constraints.iter().any(|c| matches!(c.kind, PlacementConstraint::AsyncIo));
                                            let cores = core_set(affinity, r.topology).filter(|c| !c.0.is_empty());
                                            self.pool(name, async_io, cores)
                                        }
                                    };
                                    family.push((None, d, decision));
                                }
                            }
                        }
                        None if root.is_some() => family.push((None, PlacementTable::MAIN, Decision::Default)),
                        None => family.push((owner.key.replica, owner.domain, Decision::Inherited { from: owner.key.clone() })),
                    }
                    for (replica, domain, decided_by) in family {
                        let mut path = owner.key.path.clone();
                        path.push(step.clone());
                        let key = InstanceKey { origin: owner.key.origin, path, replica: replica.or(owner.key.replica) };
                        let (realizes, args_known) = match realized {
                            Some(d) => {
                                let (r, known) = self.decl_ref(d, declared.as_ref());
                                (Some(r), known)
                            }
                            None => (None, true),
                        };
                        if literal.is_none() {
                            let kind = match &reused {
                                Some(source) => HoleKind::Reuse { source: source.clone() },
                                None => HoleKind::UnenumerableInitializer,
                            };
                            self.hole(HoleAt::Instance(key.clone()), kind);
                        }
                        // A literal naming nothing resolvable, or no literal at
                        // all: the declared type is what is left to read.
                        let realizes = match (realizes, literal.is_none(), &declared_named) {
                            (Some(r), _, _) => Some(r),
                            (None, true, Named::Locus(d)) => Some(self.decl_ref(d, declared.as_ref()).0),
                            (None, _, _) => {
                                let w = if path_written.is_empty() {
                                    match &declared {
                                        Some(TypeExpr::Named { path, .. }) => written(&segments(path)),
                                        _ => String::from("an unnamed type"),
                                    }
                                } else {
                                    path_written.clone()
                                };
                                self.hole(HoleAt::Instance(key.clone()), HoleKind::UnresolvedDeclaration { written: w });
                                None
                            }
                        };
                        if !args_known {
                            self.hole(HoleAt::Instance(key.clone()), HoleKind::UnresolvedArguments);
                        }
                        let owner_relative =
                            if domain == owner.domain { OwnerRelative::SameAsOwner } else { OwnerRelative::OffOwner };
                        self.table.instances.insert(
                            key.clone(),
                            InstanceRow {
                                realizes: realizes.clone(),
                                literal,
                                owner: Some(owner.key.clone()),
                                domain,
                                decided_by,
                                owner_relative,
                                guarded,
                                built_by: None,
                            },
                        );
                        if let Some(lit) = source {
                            let top = InstanceKey { origin: Origin::Construction(lit), path: Vec::new(), replica: None };
                            self.held.push((key.clone(), top));
                        }
                        // Below a literal the producer resolved, the walk
                        // goes on; any hole stops it (nothing below an
                        // unknown literal is enumerated, and below a held
                        // instance only its source's rows, projected).
                        let (Some(d), Some(_)) = (realized, literal) else { continue };
                        if stack.contains(&d.site) {
                            continue;
                        }
                        let below: BTreeMap<String, TypeExpr> = match &realizes {
                            Some(r) if !r.args.is_empty() => match &declared {
                                Some(TypeExpr::Named { generic_args, .. }) => d
                                    .decl
                                    .generics
                                    .iter()
                                    .map(|g| g.name.name.clone())
                                    .zip(generic_args.iter().cloned())
                                    .collect(),
                                _ => BTreeMap::new(),
                            },
                            _ => BTreeMap::new(),
                        };
                        stack.push(d.site);
                        let next = Owner { key: &key, domain, guarded };
                        let lets_below = if written_here.is_some() { lets } else { None };
                        self.fields(d, lit_inits, &below, &next, None, lets_below, stack);
                        stack.pop();
                    }
                }
            }
        }
    }

    fn site_of(&self, id: NodeId, universe: SiteUniverse) -> Option<SiteRef> {
        let ids = match universe {
            SiteUniverse::User => self.user_ids,
            SiteUniverse::StdlibAnalysis => crate::stdlib_bodies::identities()?,
        };
        ids.site_id(id).map(|id| SiteRef { universe, id })
    }

    /// Every locus literal outside the static tower, with the domains its
    /// enclosing scope runs in and its bound. A scope of the stdlib is
    /// listed only once the program reaches it: a stdlib locus some row
    /// or reached literal realizes, a stdlib fn a reached scope calls.
    fn dynamic_sites(&mut self, scopes: &Scopes<'a>) {
        let n = scopes.scopes.len();
        // The domains each scope runs in; `None` once unknown.
        let mut dom: Vec<Result<BTreeSet<DomainId>, String>> = vec![Ok(BTreeSet::new()); n];
        let mut reached: Vec<bool> = scopes.scopes.iter().map(|s| s.universe == SiteUniverse::User).collect();
        for (i, s) in scopes.scopes.iter().enumerate() {
            match &s.kind {
                ScopeKind::Fn { is_main: true, .. } => {
                    dom[i] = Ok([PlacementTable::MAIN].into_iter().collect());
                }
                ScopeKind::Locus { decl } => {
                    let set: BTreeSet<DomainId> = self
                        .table
                        .instances
                        .values()
                        .filter(|r| r.realizes.as_ref().is_some_and(|d| d.site == *decl))
                        .map(|r| r.domain)
                        .collect();
                    if !set.is_empty() {
                        reached[i] = true;
                    }
                    dom[i] = Ok(set);
                }
                ScopeKind::Fn { .. } => {}
            }
            if !s.escapes.is_empty() {
                // handled below, per target
            }
        }
        for s in &scopes.scopes {
            for f in &s.escapes {
                if let Some(t) = scopes.fn_named(s.universe, f) {
                    dom[t] = Err(format!("`{f}` is passed as a value, so its callers are not all known"));
                }
            }
        }
        // The domains the static tower assigned each literal it visited.
        let mut placed: BTreeMap<SiteRef, BTreeSet<DomainId>> = BTreeMap::new();
        for r in self.table.instances.values() {
            if let Some(l) = r.literal.filter(|l| self.static_literals.contains(l)) {
                placed.entry(l).or_default().insert(r.domain);
            }
        }
        // To a fixpoint: a literal's declaration runs where the literal
        // runs; a callee runs where its caller does. A literal of the
        // static tower runs where its rows were placed, whatever scope it
        // is written in: an explicit field initializer in `fn main` builds
        // a pinned field, not a main-thread instance.
        loop {
            let mut changed = false;
            for (i, s) in scopes.scopes.iter().enumerate() {
                if !reached[i] {
                    continue;
                }
                let here = dom[i].clone();
                let targets = s
                    .literals
                    .iter()
                    .filter_map(|l| {
                        let t = l.decl.and_then(|d| scopes.locus_scope(d))?;
                        let add = match placed.get(&l.site) {
                            Some(set) => Ok(set.clone()),
                            None if self.static_literals.contains(&l.site) => Ok(BTreeSet::new()),
                            None => here.clone(),
                        };
                        Some((t, add))
                    })
                    .chain(s.calls.iter().filter_map(|(f, _)| Some((scopes.fn_named(s.universe, f)?, here.clone()))));
                for (t, add) in targets.collect::<Vec<_>>() {
                    if !reached[t] {
                        reached[t] = true;
                        changed = true;
                    }
                    let next = match (&dom[t], &add) {
                        (Err(_), _) => continue,
                        (Ok(_), Err(why)) => Err(why.clone()),
                        (Ok(have), Ok(add)) => {
                            if add.is_subset(have) {
                                continue;
                            }
                            Ok(have.union(add).copied().collect())
                        }
                    };
                    dom[t] = next;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        for (i, s) in scopes.scopes.iter().enumerate() {
            if !reached[i] {
                continue;
            }
            let (domains, unknown) = match &dom[i] {
                Ok(set) if !set.is_empty() => (set.clone(), None),
                Ok(_) => (
                    BTreeSet::new(),
                    Some(match &s.kind {
                        ScopeKind::Locus { .. } => "the enclosing locus has no instance the table places".to_string(),
                        ScopeKind::Fn { .. } => "the enclosing fn has no caller the table places".to_string(),
                    }),
                ),
                Err(why) => (BTreeSet::new(), Some(why.clone())),
            };
            for l in &s.literals {
                if self.static_literals.contains(&l.site) || l.decl.is_none() && !l.unknown {
                    continue;
                }
                if self.table.root.as_ref().is_some_and(|r| r.constructions.iter().any(|c| c.literal == l.site)) {
                    continue;
                }
                let realizes = l.decl.and_then(|d| scopes.decl_entry(self.decls, d)).map(|e| self.decl_ref(e, None).0);
                if realizes.is_none() {
                    self.hole(HoleAt::Dynamic(l.site), HoleKind::UnresolvedDeclaration { written: l.written.clone() });
                }
                if let Some(why) = &unknown {
                    self.hole(HoleAt::Dynamic(l.site), HoleKind::UnknownDomains { reason: why.clone() });
                }
                let enclosing = match &s.kind {
                    ScopeKind::Fn { site, .. } => Enclosing::Fn(*site),
                    ScopeKind::Locus { decl } => match scopes.decl_entry(self.decls, *decl) {
                        Some(e) => Enclosing::Locus(self.decl_ref(e, None).0),
                        None => continue,
                    },
                };
                self.table.dynamic.push(DynamicSite {
                    literal: l.site,
                    realizes,
                    enclosing,
                    domains: domains.clone(),
                    bound: scopes.bound(s, l.in_loop),
                });
            }
        }
        self.table.dynamic.sort_by_key(|d| d.literal);
    }
}

// ------------------------------------------------- scopes and bounds

/// A locus literal in a body.
struct Literal<'a> {
    site: SiteRef,
    /// The locus declaration it names, when it names one.
    decl: Option<SiteRef>,
    /// It names nothing the producer resolves (neither a locus nor a
    /// type): a hole, not a skipped struct literal.
    unknown: bool,
    inits: &'a [StructInit],
    in_loop: bool,
    written: String,
}

enum ScopeKind {
    /// A free fn: its name, its site, whether it is a top-level `fn main`.
    Fn { name: String, site: SiteRef, is_main: bool },
    /// Every body of one locus declaration: they run where its
    /// instances do.
    Locus { decl: SiteRef },
}

/// One scope's bodies: the literals in them, the free fns they call,
/// the free fns they name as values, and the names they bind.
struct Scope<'a> {
    universe: SiteUniverse,
    kind: ScopeKind,
    literals: Vec<Literal<'a>>,
    calls: Vec<(String, bool)>,
    escapes: BTreeSet<String>,
    lets: Lets,
}

/// How many times a scope can run: finite, or unbounded with a reason.
#[derive(Clone)]
enum Count {
    Finite(u32),
    Unbounded(String),
}

struct Scopes<'a> {
    scopes: Vec<Scope<'a>>,
    fns: BTreeMap<(SiteUniverse, String), usize>,
    loci: BTreeMap<SiteRef, usize>,
    counts: Vec<Count>,
}

impl<'a> Scopes<'a> {
    fn of(
        bundle: &'a Bundle<'a>,
        stdlib: Option<(&'a Program, &'a Snapshot)>,
        decls: &Decls<'a>,
    ) -> Scopes<'a> {
        let mut s = Scopes { scopes: Vec::new(), fns: BTreeMap::new(), loci: BTreeMap::new(), counts: Vec::new() };
        for program in bundle.programs.values() {
            s.collect(&program.items, &bundle.snapshot, SiteUniverse::User, decls, true);
        }
        if let Some((program, ids)) = stdlib {
            s.collect(&program.items, ids, SiteUniverse::StdlibAnalysis, decls, true);
        }
        let mut callers: BTreeMap<(SiteUniverse, &str), Vec<(usize, bool)>> = BTreeMap::new();
        for (j, caller) in s.scopes.iter().enumerate() {
            for (callee, in_loop) in &caller.calls {
                callers.entry((caller.universe, callee.as_str())).or_default().push((j, *in_loop));
            }
        }
        let mut memo: Vec<Option<Count>> = vec![None; s.scopes.len()];
        for i in 0..s.scopes.len() {
            s.count(i, &callers, &mut memo, &mut Vec::new());
        }
        s.counts = memo.into_iter().map(|c| c.expect("every scope counted")).collect();
        s
    }

    fn collect(&mut self, items: &'a [TopDecl], ids: &Snapshot, universe: SiteUniverse, decls: &Decls<'a>, top: bool) {
        for item in items {
            match item {
                TopDecl::Fn(fd) => {
                    let Some(id) = ids.site_id(fd.id) else { continue };
                    let site = SiteRef { universe, id };
                    let mut w = BodyWalk::new(ids, universe, decls);
                    for p in &fd.params {
                        w.bind(&p.name.name, None);
                        if let Some(d) = &p.default {
                            w.expr(d);
                        }
                    }
                    w.block(&fd.body);
                    let is_main = top && universe == SiteUniverse::User && fd.name.name == "main";
                    self.fns.entry((universe, fd.name.name.clone())).or_insert(self.scopes.len());
                    self.scopes.push(w.finish(ScopeKind::Fn { name: fd.name.name.clone(), site, is_main }));
                }
                TopDecl::Locus(l) => {
                    let Some(id) = ids.site_id(l.id) else { continue };
                    let decl = SiteRef { universe, id };
                    let mut w = BodyWalk::new(ids, universe, decls);
                    for m in &l.members {
                        match m {
                            LocusMember::Fn(fd) => {
                                for p in &fd.params {
                                    w.bind(&p.name.name, None);
                                    if let Some(d) = &p.default {
                                        w.expr(d);
                                    }
                                }
                                w.block(&fd.body);
                            }
                            LocusMember::Lifecycle(ld) => w.block(&ld.body),
                            LocusMember::Mode(md) => w.block(&md.body),
                            LocusMember::Failure(fd) => w.block(&fd.body),
                            LocusMember::BirthCheck(bc) => {
                                w.expr(&bc.cond);
                                if let Some(p) = &bc.payload {
                                    w.expr(p);
                                }
                            }
                            _ => {}
                        }
                    }
                    self.loci.insert(decl, self.scopes.len());
                    self.scopes.push(w.finish(ScopeKind::Locus { decl }));
                }
                TopDecl::Module(m) => self.collect(&m.items, ids, universe, decls, false),
                _ => {}
            }
        }
    }

    /// The top-level user `fn main`'s site, when there is one.
    fn entry_fn(&self) -> Option<SiteRef> {
        self.scopes.iter().find_map(|s| match s.kind {
            ScopeKind::Fn { is_main: true, site, .. } => Some(site),
            _ => None,
        })
    }

    fn fn_named(&self, universe: SiteUniverse, name: &str) -> Option<usize> {
        self.fns.get(&(universe, name.to_string())).copied()
    }

    fn locus_scope(&self, decl: SiteRef) -> Option<usize> {
        self.loci.get(&decl).copied()
    }

    fn decl_entry<'d>(&self, decls: &'d Decls<'a>, decl: SiteRef) -> Option<&'d DeclEntry<'a>> {
        let all = match decl.universe {
            SiteUniverse::User => &decls.user,
            SiteUniverse::StdlibAnalysis => &decls.stdlib,
        };
        all.iter().position(|e| e.site == decl).map(|i| decls.entry(decl.universe, i))
    }

    /// How many times scope `i` can run: `fn main` once; a fn the sum of
    /// its call sites' counts; a locus body, a fn named as a value, a
    /// recursive fn or a call in a loop without bound.
    fn count(
        &self,
        i: usize,
        callers: &BTreeMap<(SiteUniverse, &str), Vec<(usize, bool)>>,
        memo: &mut Vec<Option<Count>>,
        visiting: &mut Vec<usize>,
    ) -> Count {
        if let Some(c) = &memo[i] {
            return c.clone();
        }
        let s = &self.scopes[i];
        let c = match &s.kind {
            ScopeKind::Fn { is_main: true, .. } => Count::Finite(1),
            ScopeKind::Locus { .. } => {
                Count::Unbounded("built in a locus body, which can run any number of times".to_string())
            }
            ScopeKind::Fn { name, .. } => {
                if self.scopes.iter().any(|o| o.universe == s.universe && o.escapes.contains(name)) {
                    Count::Unbounded(format!("built in `{name}`, which is passed as a value"))
                } else if visiting.contains(&i) {
                    // Not memoized: the answer belongs to the cycle's entry.
                    return Count::Unbounded(format!("built in `{name}`, which is recursive"));
                } else {
                    visiting.push(i);
                    let mut total = Count::Finite(0);
                    for (j, in_loop) in callers.get(&(s.universe, name.as_str())).into_iter().flatten() {
                        let add = if *in_loop {
                            Count::Unbounded(format!("built in `{name}`, which is called in a loop"))
                        } else {
                            self.count(*j, callers, memo, visiting)
                        };
                        total = match (total, add) {
                            (Count::Finite(a), Count::Finite(b)) => Count::Finite(a.saturating_add(b)),
                            (Count::Unbounded(why), _) | (_, Count::Unbounded(why)) => {
                                total = Count::Unbounded(why);
                                break;
                            }
                        };
                    }
                    visiting.pop();
                    total
                }
            }
        };
        memo[i] = Some(c.clone());
        c
    }

    fn bound(&self, s: &Scope<'a>, in_loop: bool) -> Bound {
        if in_loop {
            return Bound::Unbounded("built in a loop".to_string());
        }
        let i = self
            .scopes
            .iter()
            .position(|o| std::ptr::eq(o, s))
            .expect("a scope of this table");
        match &self.counts[i] {
            Count::Finite(0) => Bound::AtMost(0),
            Count::Finite(1) => Bound::Once,
            Count::Finite(n) => Bound::AtMost(*n),
            Count::Unbounded(why) => Bound::Unbounded(why.clone()),
        }
    }
}

/// One scope's walk over its bodies.
struct BodyWalk<'a, 'd> {
    ids: &'d Snapshot,
    universe: SiteUniverse,
    decls: &'d Decls<'a>,
    loop_depth: u32,
    literals: Vec<Literal<'a>>,
    calls: Vec<(String, bool)>,
    escapes: BTreeSet<String>,
    lets: Lets,
}

impl<'a, 'd> BodyWalk<'a, 'd> {
    fn new(ids: &'d Snapshot, universe: SiteUniverse, decls: &'d Decls<'a>) -> Self {
        BodyWalk {
            ids,
            universe,
            decls,
            loop_depth: 0,
            literals: Vec::new(),
            calls: Vec::new(),
            escapes: BTreeSet::new(),
            lets: BTreeMap::new(),
        }
    }

    fn finish(self, kind: ScopeKind) -> Scope<'a> {
        Scope {
            universe: self.universe,
            kind,
            literals: self.literals,
            calls: self.calls,
            escapes: self.escapes,
            lets: self.lets,
        }
    }

    /// `name` bound to `to`: a second binding of one name, anywhere in the
    /// scope, binds it to nothing the producer follows.
    fn bind(&mut self, name: &str, to: Option<SiteRef>) {
        self.lets.entry(name.to_string()).and_modify(|b| *b = None).or_insert(to);
    }

    /// A match arm's bindings, each a name bound to nothing followed.
    fn pattern(&mut self, p: &Pattern) {
        match p {
            Pattern::Binding(i) => self.bind(&i.name, None),
            Pattern::Constructor { args, .. } | Pattern::Tuple(args, _) => {
                for a in args {
                    self.pattern(a);
                }
            }
            Pattern::Literal(..) | Pattern::Wildcard(_) => {}
        }
    }

    /// The locus literal `e` is, when it is one.
    fn locus_literal(&self, e: &Expr) -> Option<SiteRef> {
        let Expr::Struct { path, id, .. } = e else { return None };
        match self.decls.resolve(&segments(path), self.universe) {
            Named::Locus(_) => self.ids.site_id(*id).map(|id| SiteRef { universe: self.universe, id }),
            _ => None,
        }
    }

    fn block(&mut self, b: &'a Block) {
        for s in &b.stmts {
            self.stmt(s);
        }
        if let Some(t) = &b.tail {
            self.expr(t);
        }
    }

    fn looped(&mut self, b: &'a Block) {
        self.loop_depth += 1;
        self.block(b);
        self.loop_depth -= 1;
    }

    fn if_chain(&mut self, i: &'a IfStmt) {
        self.expr(&i.cond);
        self.block(&i.then_block);
        match i.else_block.as_deref() {
            Some(ElseBranch::Else(b)) => self.block(b),
            Some(ElseBranch::ElseIf(n)) => self.if_chain(n),
            None => {}
        }
    }

    fn disposition(&mut self, d: &'a OrDisposition) {
        match d {
            OrDisposition::Substitute(e) | OrDisposition::Fail(e, _) => self.expr(e),
            OrDisposition::Raise(_) | OrDisposition::Discard(_) | OrDisposition::Wait(_) => {}
        }
    }

    fn stmt(&mut self, s: &'a Stmt) {
        match s {
            Stmt::Let { is_mut, name, value, .. } => {
                let to = if *is_mut { None } else { self.locus_literal(value) };
                self.bind(&name.name, to);
                self.expr(value);
            }
            Stmt::LetTuple { names, value, .. } => {
                for n in names {
                    self.bind(&n.name, None);
                }
                self.expr(value);
            }
            Stmt::Assign { target, value, .. } => {
                self.expr(value);
                for seg in &target.tail {
                    if let LValueSeg::Index(ix) = seg {
                        self.expr(ix);
                    }
                }
            }
            Stmt::If(i) => self.if_chain(i),
            Stmt::Match(m) => {
                self.expr(&m.scrutinee);
                for arm in &m.arms {
                    self.pattern(&arm.pattern);
                    if let Some(g) = &arm.guard {
                        self.expr(g);
                    }
                    match &arm.body {
                        MatchArmBody::Expr(e) => self.expr(e),
                        MatchArmBody::Block(b) => self.block(b),
                    }
                }
            }
            Stmt::For { name, iter, body, .. } => {
                self.bind(&name.name, None);
                self.expr(iter);
                self.looped(body);
            }
            Stmt::While { cond, body, .. } => {
                self.loop_depth += 1;
                self.expr(cond);
                self.block(body);
                self.loop_depth -= 1;
            }
            Stmt::Return(Some(e), _) | Stmt::Fail { value: e, .. } | Stmt::Expr(e) => self.expr(e),
            Stmt::Block(b) => self.block(b),
            Stmt::Recovery { args, modifier, .. } => {
                for a in args {
                    self.expr(a);
                }
                if let Some(RecoveryModifier::For(e) | RecoveryModifier::Until(e)) = modifier {
                    self.expr(e);
                }
            }
            Stmt::Violate { payload: Some(p), .. } => self.expr(p),
            Stmt::Send { subject, value, or_disposition, .. } => {
                self.expr(subject);
                self.expr(value);
                if let Some(d) = or_disposition {
                    self.disposition(d);
                }
            }
            Stmt::ShmWrite { max, body, .. } => {
                self.expr(max);
                self.block(body);
            }
            _ => {}
        }
    }

    fn expr(&mut self, e: &'a Expr) {
        match e {
            Expr::Ident(i) => {
                // A name that is a free fn, read as a value.
                if self.decls_has_fn(&i.name) {
                    self.escapes.insert(i.name.clone());
                }
            }
            Expr::Literal(..) | Expr::Path(_) | Expr::KwSelf(_) => {}
            Expr::Binary { left, right, .. } => {
                self.expr(left);
                self.expr(right);
            }
            Expr::Unary { operand, .. } => self.expr(operand),
            Expr::Call { callee, args, .. } => {
                match &**callee {
                    Expr::Ident(i) => self.calls.push((i.name.clone(), self.loop_depth > 0)),
                    Expr::Path(qn) => {
                        if let Some(last) = qn.segments.last() {
                            self.calls.push((last.name.clone(), self.loop_depth > 0));
                        }
                    }
                    other => self.expr(other),
                }
                for a in args {
                    self.expr(a);
                }
            }
            Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. } => self.expr(receiver),
            Expr::Index { receiver, index, .. } => {
                self.expr(receiver);
                self.expr(index);
            }
            Expr::Tuple(parts, _) | Expr::Array(parts, _) => {
                for p in parts {
                    self.expr(p);
                }
            }
            Expr::Struct { path, inits, id, .. } => {
                let segs = segments(path);
                let (decl, unknown) = match self.decls.resolve(&segs, self.universe) {
                    Named::Locus(d) => (Some(d.site), false),
                    Named::Unknown => (None, true),
                    Named::Contract | Named::NotALocus => (None, false),
                };
                if let Some(id) = self.ids.site_id(*id) {
                    if decl.is_some() || unknown {
                        self.literals.push(Literal {
                            site: SiteRef { universe: self.universe, id },
                            decl,
                            unknown,
                            inits,
                            in_loop: self.loop_depth > 0,
                            written: written(&segs),
                        });
                    }
                }
                for i in inits {
                    self.expr(&i.value);
                }
            }
            Expr::Block(b) => self.block(b),
            Expr::If(i) => self.if_chain(i),
            Expr::Match(m) => {
                self.expr(&m.scrutinee);
                for arm in &m.arms {
                    self.pattern(&arm.pattern);
                    if let Some(g) = &arm.guard {
                        self.expr(g);
                    }
                    match &arm.body {
                        MatchArmBody::Expr(e) => self.expr(e),
                        MatchArmBody::Block(b) => self.block(b),
                    }
                }
            }
            Expr::Sum(inner, _) | Expr::Prod(inner, _) => self.expr(inner),
            Expr::Approx { left, right, tolerance, .. } => {
                self.expr(left);
                self.expr(right);
                self.expr(tolerance);
            }
            Expr::Range { lo, hi, .. } => {
                self.expr(lo);
                self.expr(hi);
            }
            Expr::ArrayRepeat { val, .. } => self.expr(val),
            Expr::Or { inner, disposition, .. } => {
                self.expr(inner);
                self.disposition(disposition);
            }
        }
    }

    fn decls_has_fn(&self, name: &str) -> bool {
        self.decls.fns.contains(&(self.universe, name.to_string()))
    }
}

/// Legacy producers of the family that nothing outside their modules
/// calls, reachable for the placement shadow (`tests/shadow_placement.rs`)
/// alone: test support, not an API.
#[doc(hidden)]
pub mod legacy {
    use std::collections::BTreeMap;

    use hale_syntax::ast::LocusDecl;

    use crate::bus_graph::Placement;
    use crate::check::PoolId;
    use crate::symbol::Bundle;

    /// F.31's owner-relative answer at `self.f`.
    pub fn enclosing_field_placement(enclosing_locus: &LocusDecl, field_name: &str) -> Option<PoolId> {
        crate::check::enclosing_field_placement(enclosing_locus, field_name)
    }

    /// The ownership graph's per-type labels.
    pub fn collect_placements(bundle: &Bundle<'_>) -> BTreeMap<String, Placement> {
        crate::ownership_graph::collect_placements(bundle)
    }
}

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

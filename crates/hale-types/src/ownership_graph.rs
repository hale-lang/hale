//! Authoritative locus-ownership graph (ANALYSIS ONLY).
//!
//! Hale's ownership topology is, like the bus topology, fully
//! *static*: a locus declares the child types it owns with the
//! `accept(c: ChildType) { ... }` lifecycle hook, and it gives birth
//! to children by writing a locus-typed literal (`ChildType { ... }`)
//! in one of its method bodies. There is no runtime `attach()` /
//! `adopt()` construct — the set of ownership edges is therefore
//! statically enumerable, which is the premise that makes ancestor
//! resolution sound.
//!
//! This module is the structural twin of [`crate::bus_graph`]. Where
//! `build_bus_graph` reifies publisher→subscriber edges and gates each
//! subject for devirtualization, [`build_ownership_graph`] reifies
//! instantiation→owner edges and, for every instantiation *site*,
//! resolves which ancestor locus **owns** the new locus and classifies
//! the resulting edge.
//!
//! The headline capability over today's direct-parent case is
//! **bubbling**: an instantiation `I{}` inside locus `B` where `B`
//! does NOT itself accept `I` resolves to the nearest *ancestor* of
//! `B` that accepts `I` (innermost-wins). When the set of ancestors
//! disagrees on the owner across distinct instantiation paths the site
//! is `PerPath`; when a path climbs to a root with no acceptor it is
//! `Orphan`.
//!
//! Build is PURE ANALYSIS: nothing here changes codegen, emits a
//! diagnostic, or mutates the AST. `Orphan` is a *resolved property*,
//! not an error — a future pass consumes the graph; this pass only
//! computes it. The classification defaults conservative: any shape
//! this pass cannot resolve (open world, cross-seed) is
//! `Unanalyzable` / `EdgeClass::Open`, mirroring the bus gate's
//! default-to-ineligible stance.

use std::collections::{BTreeMap, BTreeSet};

use hale_graph::ids::SiteId;
use hale_syntax::ast::*;
use hale_syntax::Span;

use crate::handler_routing::{child_locus_name, resolve_locus_type, ChildRef, DeclAt, DeclaredNames};
use crate::placement::{Enclosing, HoleAt, HoleKind, Origin, PerUsePosition, PlacementTable, SiteRef, SiteUniverse};
use crate::resolve::TopScope;
use crate::symbol::Bundle;

// === Public graph =================================================

/// How the owner of an instantiation site was resolved. Mirrors the
/// `StaticIneligible` shape in `bus_graph`: a positive resolution
/// (`SelfOwned` / `Ancestor`) or one of the fall-through properties
/// (`PerPath` / `Orphan` / `Unanalyzable`). Defaults conservative —
/// anything the walk cannot close over lands in `Unanalyzable`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnerResolution {
    /// The enclosing locus itself declares `accept(_: I)` — today's
    /// direct-parent case, innermost-most possible. The owner *is* the
    /// enclosing locus.
    SelfOwned(String),
    /// A UNIQUE accepting ancestor above the enclosing locus owns the
    /// site (bubbling). The `String` is that owner locus type.
    Ancestor(String),
    /// The owner locus type differs across distinct instantiation
    /// paths (a shared intermediary reached from two different
    /// acceptors). Carries the distinct owner types, sorted.
    PerPath(Vec<String>),
    /// At least one instantiation path climbs to a root (a `main
    /// locus`, a locus only born at `fn main`, or an uninstantiated
    /// root) WITHOUT hitting an acceptor of `I`. A resolved property,
    /// NOT an error.
    Orphan,
    /// Open-world / cross-seed / otherwise unresolvable. Carries a
    /// human-readable reason.
    Unanalyzable(String),
}

impl OwnerResolution {
    /// A short tag for classification summaries / test assertions.
    pub fn tag(&self) -> &'static str {
        match self {
            OwnerResolution::SelfOwned(_) => "SelfOwned",
            OwnerResolution::Ancestor(_) => "Ancestor",
            OwnerResolution::PerPath(_) => "PerPath",
            OwnerResolution::Orphan => "Orphan",
            OwnerResolution::Unanalyzable(_) => "Unanalyzable",
        }
    }
    /// The single resolved owner type, when there is exactly one
    /// (`SelfOwned` / `Ancestor`). `None` for `PerPath` / `Orphan` /
    /// `Unanalyzable`.
    pub fn owner(&self) -> Option<&str> {
        match self {
            OwnerResolution::SelfOwned(o) | OwnerResolution::Ancestor(o) => {
                Some(o)
            }
            _ => None,
        }
    }
}

/// A best-effort classification of the owner instance. `DirectParent`
/// is the `SelfOwned` case; `SingletonConst` is when the owner is a
/// provably-unique instance (a `main locus` or a wasm `@export`
/// locus), so a future pass could constant-fold the owner pointer.
/// `Ancestor` covers every other resolved owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnerKind {
    DirectParent,
    Ancestor,
    SingletonConst,
}

/// Where the owner runs relative to the enclosing (instantiating)
/// locus, read from the placement table: the domains of every instance
/// of the enclosing locus against the owner's. `SameTower` — every
/// enclosing instance runs in the owner's one domain (or neither is
/// built); `CrossPool` — none does, and the owner has one known domain;
/// `Mixed` — some do and some do not, or an instance runs where the
/// table cannot say, so the delivery mechanism differs per enclosing
/// instance (the placement correspondence's U-1: the resolved owner is
/// kept, and only the mechanism varies); `Open` — not a closed world,
/// or no single owner to compare, so no tower relation can be asserted.
/// Conservative: an unresolved owner is `Open`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EdgeClass {
    SameTower,
    CrossPool,
    Mixed,
    Open,
}

/// One resolved instantiation site: `enclosing_locus` gives birth to
/// `child_ty` in a member body or params initializer, and the pass has
/// resolved which ancestor owns it.
#[derive(Debug, Clone)]
pub struct OwnedSite {
    /// The locus type being instantiated (`I`).
    pub child_ty: String,
    /// The locus whose member or params initializer writes `I { ... }` (`B`).
    pub enclosing_locus: String,
    /// The resolved owner + how it was found.
    pub resolution: OwnerResolution,
    /// Best-effort classification of the owner instance.
    pub owner_kind: OwnerKind,
    /// Same-thread vs cross-pool vs mixed vs open, per the domains of
    /// the enclosing locus's instances and the owner's.
    pub edge_class: EdgeClass,
    /// For a `Mixed` edge: every instance of the enclosing locus, with
    /// the domain it runs in (`App.w on main`, `App.p.w on pinned:App.p`,
    /// `a literal in Spawner on an unknown domain`), the owner's last;
    /// what a refusal names. Empty for every other class.
    pub instances: Vec<String>,
    /// The projection class declared on the *owner* (the acceptor),
    /// when the owner is resolved and annotates one. Informational —
    /// it does not affect resolution.
    pub owner_projection: Option<ProjectionClass>,
    /// The span of the `I { ... }` literal.
    pub span: Span,
    /// The declaration whose body holds the literal: an index into
    /// [`OwnershipGraph::declarations`]. `enclosing_locus` is its name.
    pub enclosing_decl: usize,
    /// The `fn` whose body holds the literal; `None` for a lifecycle, a
    /// mode, a failure handler or a params default.
    pub member: Option<String>,
    /// The declaration the literal names, an index into
    /// [`OwnershipGraph::declarations`], joined by the resolved declaration
    /// identity (by name only for unminted input). `None` for a locus
    /// declared outside this graph, such as bundled stdlib analysis.
    pub child_decl: Option<usize>,
    /// The literal's child in the terms of [`OwnershipGraph::accepts`]:
    /// the locus `child_locus_name` resolves it to, a generic template
    /// specialized by the binding's or field's declared type. `None`
    /// where a generic template has no declared type to specialize it.
    /// `child_ty` uses this resolved name too (the template
    /// name where no specialization is known), so bubbling never keys
    /// an imported or aliased birth by its written final segment.
    pub child_key: Option<String>,
    /// This literal was walked inside a params initializer, including
    /// nested expressions. Structural provenance survives desugars that
    /// copy an expression with its original source span (F.40 C3).
    pub params_default: bool,
    /// The literal itself is a discarded expression statement. A
    /// nested initializer or a block tail is a value use even when an
    /// enclosing expression's result is discarded.
    pub bare_statement: bool,
    /// The fields the literal writes out. Every other param of the
    /// child takes its default, which lowering expands where this
    /// literal is lowered.
    pub supplied: BTreeSet<String>,
    /// For a `params_default` site, the param whose default holds it.
    pub params_field: Option<String>,
    /// The literal sits in the default of a method's argument: lowering
    /// never lowers it in this locus's body, only at a call that leaves
    /// the argument to its default, under the caller's locus.
    pub arg_default: Option<ArgDefault>,
}

/// A position in the default of a fn's or a locus method's argument
/// (`fn take(s: Ship = Ship { })`). Lowering expands the default at each
/// call that leaves the argument out, in the caller's scope and under
/// the caller's locus (C3 rest, the review of #1351).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArgDefault {
    /// The declaring fn's id (`FnDecl::id`), as the typed-body table's
    /// `omitted_args` column names a call's callee.
    pub callee: u32,
    /// The argument's index.
    pub index: usize,
    /// The fn as a diagnostic names it: `take`, or `Holder.take` for a
    /// method.
    pub fn_name: String,
    /// The argument's name.
    pub param: String,
}

/// A locus birth in a free function. Collected by the same walk as
/// [`OwnershipGraph::sites`], without an enclosing owning locus.
#[derive(Debug, Clone)]
pub struct FreeFnSite {
    pub child_ty: String,
    pub span: Span,
    /// The declaration the literal names, in the graph's declaration
    /// table, using the same resolver as [`OwnedSite::child_decl`].
    pub child_decl: Option<usize>,
    pub child_key: Option<String>,
    /// See [`OwnedSite::supplied`].
    pub supplied: BTreeSet<String>,
    /// See [`OwnedSite::bare_statement`].
    pub bare_statement: bool,
    /// See [`OwnedSite::arg_default`]: a literal in a free fn's argument
    /// default is lowered at the calls that leave it out, under their
    /// locus, not in the free fn.
    pub arg_default: Option<ArgDefault>,
}

/// A call written where default expansion reads it: in a locus's member
/// body (lowered under that locus), in a locus's params default, or in an
/// argument's default. Calls in a free fn's body or a binding entry are
/// lowered under no locus and are not kept.
#[derive(Debug, Clone)]
pub struct CallSite {
    /// The call's id (`Expr::Call`'s), the typed-body table's key.
    pub id: u32,
    pub span: Span,
    /// The locus declaration whose member or params default holds the
    /// call; `None` for one in a free fn's argument default.
    pub enclosing_decl: Option<usize>,
    /// The call sits in this param's default of `enclosing_decl`.
    pub params_field: Option<String>,
    /// The call sits in this argument's default.
    pub arg_default: Option<ArgDefault>,
    /// The call sits in a position lowering emits at every use, under
    /// whichever locus uses it (`enclosing_decl` is then `None`).
    pub per_use: Option<PerUsePosition>,
}

/// Where a literal or a call outside the member-body walk is lowered
/// (C3 rest, the review of #1351).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OtherPosition {
    /// A closure's assertion, evaluated under its locus: a member body
    /// of that locus, as default expansion reads one.
    Closure,
    /// A `const`'s value or a type's field default, emitted at every use
    /// under the locus of that use, which no row relates to the position.
    PerUse(PerUsePosition),
}

/// A locus literal at an [`OtherPosition`].
#[derive(Debug, Clone)]
pub struct OtherSite {
    /// The literal: its resolved child, the fields it supplies, its span.
    pub site: FreeFnSite,
    /// The locus declaration whose member holds it, `None` for a
    /// top-level `const` or `type`.
    pub enclosing_decl: Option<usize>,
    pub position: OtherPosition,
}

/// One literal lowering builds from a default, in a context
/// ([`OwnershipGraph::expansions`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Expansion {
    pub literal: ExpandedLiteral,
    /// The locus lowering expands it under: an index into
    /// [`OwnershipGraph::declarations`].
    pub context: usize,
    /// Where the context's own body starts the expansion: the literal
    /// that leaves a param to its default, or the call that leaves an
    /// argument to its default, through any chain of defaults.
    pub root: Span,
    /// The root sits in a position lowering emits at every use: the
    /// context is every locus, since none is known to be the one.
    pub per_use: Option<PerUsePosition>,
}

/// A literal default expansion visits, in [`OwnershipGraph::sites`],
/// [`OwnershipGraph::free_fn_sites`] or [`OwnershipGraph::other_sites`]
/// (the last only as a root, never in a default).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ExpandedLiteral {
    Owned(usize),
    Free(usize),
    Other(usize),
}

/// One locus declaration, in the bundle's declaration order (programs
/// in bundle order, then items in source order, modules flattened).
/// Two declarations may share a name; each is its own row.
#[derive(Debug, Clone)]
pub struct LocusDeclRow {
    pub name: String,
    pub span: Span,
    /// The declaration's snapshot identity (`None` when unminted).
    pub id: Option<SiteId>,
    /// `locus L<T>`: a template, specialized at its use sites.
    pub generic: bool,
    /// The fns its `bus { subscribe … as h }` entries name: its bus
    /// handlers.
    pub bus_handlers: BTreeSet<String>,
}

/// The whole-bundle ownership graph.
#[derive(Debug, Clone, Default)]
pub struct OwnershipGraph {
    /// Every resolved instantiation site, in walk order.
    pub sites: Vec<OwnedSite>,
    /// Free-function births from the graph's one walk, in source order.
    pub free_fn_sites: Vec<FreeFnSite>,
    /// The calls default expansion reads, in walk order.
    pub calls: Vec<CallSite>,
    /// The locus literals at the other positions default expansion reads
    /// (a closure's assertion, a const's value, a type's field default),
    /// apart from the births: their ownership is not this graph's.
    pub other_sites: Vec<OtherSite>,
    /// Construction context, from the same walk and child resolution.
    /// This includes binding adapters, whose births do not participate
    /// in the legacy body-bubbling relation.
    births: Vec<BirthRow>,
    /// Every locus declaration, in declaration order.
    pub declarations: Vec<LocusDeclRow>,
    /// locus type → the child types it declares `accept(_: T)` for, each
    /// the locus `child_locus_name` resolves it to: an alias followed,
    /// generic arguments mangled, a `std::` or cross-seed path renamed.
    pub accepts: BTreeMap<String, BTreeSet<String>>,
    /// child locus type → the set of locus types that instantiate it
    /// in a method body (the ancestor-edge relation).
    pub instantiated_by: BTreeMap<String, BTreeSet<String>>,
    /// The `accept` rows by the declaring locus's identity, each with
    /// its param type as written: what a specialization of a generic
    /// template accepts ([`AcceptRows::specialize`]).
    pub accept_rows: AcceptRows,
    /// The rows the graph is assembled from ([`OwnershipRows`]).
    pub rows: OwnershipRows,
}

/// A resolved construction and the defaults it overrides. The model
/// asks the graph which of these constructions occur outside its
/// arrangement; it never infers execution context from source spans.
#[derive(Debug, Clone)]
pub(crate) struct BirthRow {
    pub child_decl: Option<usize>,
    pub span: Span,
    literal: Option<SiteId>,
    supplied: BTreeSet<String>,
    context: BirthContext,
}

#[derive(Debug, Clone)]
enum BirthContext {
    Body,
    Default { owner: usize, field: String },
    Binding(Option<SiteId>),
}

/// One `accept` param, by the locus that declares it.
#[derive(Debug, Clone)]
pub struct AcceptRow {
    /// The declaring locus's identity (a monomorph keeps its
    /// template's).
    pub owner_id: NodeId,
    pub owner: String,
    /// The param's type as written: a generic template's names its
    /// parameters.
    pub ty: TypeExpr,
    /// The `accept` declaration's span: the unminted fallback's
    /// containment test.
    pub span: Span,
}

/// Every locus's `accept` rows, with what their types resolve against.
#[derive(Debug, Clone, Default)]
pub struct AcceptRows {
    rows: Vec<AcceptRow>,
    declared: DeclaredNames,
    renames: Vec<(Vec<String>, String)>,
}

impl AcceptRows {
    pub fn rows(&self) -> &[AcceptRow] {
        &self.rows
    }

    /// The child loci a specialization of `template` accepts: the
    /// template's rows, asked for by its identity, each type passed
    /// through `substitute` (the consumer's substitution of the
    /// template's parameters by the specialization's arguments) and
    /// resolved by `child_locus_name` the way a concrete locus's are. A
    /// template no entry point minted is matched by its name and span.
    pub fn specialize(
        &self,
        template: &LocusDecl,
        substitute: impl Fn(&TypeExpr) -> TypeExpr,
    ) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for r in &self.rows {
            let same = if r.owner_id.is_none() || template.id.is_none() {
                r.owner == template.name.name
                    && template.span.start <= r.span.start
                    && r.span.end <= template.span.end
            } else {
                r.owner_id.0 == template.id.0
            };
            if !same {
                continue;
            }
            if let ChildRef::Locus(name) = child_locus_name(&substitute(&r.ty), &self.declared, &self.renames) {
                out.insert(name);
            }
        }
        out
    }
}

impl OwnershipGraph {
    /// Births not represented by the model's arrangement. A default
    /// executes only when a particular construction leaves its field
    /// unsupplied. The same default literal may therefore be arranged
    /// for one construction and dynamic for another. Each (literal,
    /// context) is visited at most once, even for recursive templates.
    pub(crate) fn unarranged_births(
        &self,
        placement: &PlacementTable,
        represented: &BTreeSet<SiteId>,
    ) -> Vec<&BirthRow> {
        let mut defaults: BTreeMap<usize, Vec<(usize, &str)>> = BTreeMap::new();
        for (i, row) in self.births.iter().enumerate() {
            if let BirthContext::Default { owner, field } = &row.context {
                defaults.entry(*owner).or_default().push((i, field));
            }
        }
        let covered = |row: &BirthRow| row.literal.is_some_and(|id| represented.contains(&id));
        let bindings: BTreeSet<SiteId> = placement.instances.keys().filter_map(|k| match k.origin {
            Origin::Binding(site) if site.universe == SiteUniverse::User => Some(site.id),
            _ => None,
        }).collect();
        let mut pending: Vec<(usize, bool)> = self.births.iter().enumerate().filter_map(|(i, row)| {
            match &row.context {
                BirthContext::Body => Some((i, !covered(row))),
                BirthContext::Binding(Some(id)) if bindings.contains(id) => Some((i, !covered(row))),
                _ => None,
            }
        }).collect();
        // An implicit entry has no source literal to seed its defaults.
        if let Some(root) = placement.root.as_ref().filter(|r| r.constructions.is_empty()) {
            if root.realizes.site.universe == SiteUniverse::User {
                if let Some(owner) = self.declarations.iter().position(|d| d.id == Some(root.realizes.site.id)) {
                    for (i, _) in defaults.get(&owner).into_iter().flatten() {
                        pending.push((*i, !covered(&self.births[*i])));
                    }
                }
            }
        }
        let mut seen = BTreeSet::new();
        let mut outside = BTreeSet::new();
        while let Some((i, dynamic)) = pending.pop() {
            if !seen.insert((i, dynamic)) { continue; }
            let row = &self.births[i];
            if dynamic { outside.insert(i); }
            let Some(owner) = row.child_decl else { continue };
            for (child, field) in defaults.get(&owner).into_iter().flatten() {
                if !row.supplied.contains(*field) {
                    pending.push((*child, dynamic || !covered(&self.births[*child])));
                }
            }
        }
        outside.into_iter().map(|i| &self.births[i]).collect()
    }

    /// Count of sites resolved to a positive owner (`SelfOwned` or
    /// `Ancestor`).
    pub fn resolved_count(&self) -> usize {
        self.sites
            .iter()
            .filter(|s| s.resolution.owner().is_some())
            .count()
    }
    /// Histogram of sites by resolution tag.
    pub fn by_resolution(&self) -> BTreeMap<&'static str, usize> {
        let mut h: BTreeMap<&'static str, usize> = BTreeMap::new();
        for s in &self.sites {
            *h.entry(s.resolution.tag()).or_insert(0) += 1;
        }
        h
    }
    /// Every site instantiating `child_ty`.
    pub fn sites_for<'g>(&'g self, child_ty: &str) -> Vec<&'g OwnedSite> {
        self.sites.iter().filter(|s| s.child_ty == child_ty).collect()
    }

    /// Who owns the child `site` gives birth to, judged by its
    /// declaration identity: the accepting ancestors are found for
    /// [`OwnedSite::child_key`], so an `accept` naming the child through
    /// an alias, an import path or a generic specialization owns it, and
    /// one naming only its last segment or its template does not.
    ///
    /// An ancestor owns the birth only if one accepts the child on EVERY
    /// construction path of the enclosing locus ([`Self::construction_paths`],
    /// asked for only when the enclosing locus does not accept the child
    /// itself): a path that reaches a root, a hole of the placement table
    /// or no construction at all with no acceptor on it makes the site
    /// `Orphan`, and [`SiteOwnership::unowned`] names that path. A type
    /// having an accepting parent somewhere is no proof that every
    /// instance has one.
    ///
    /// `None` where the graph cannot decide: an open world (no entry
    /// point, so a consumer may complete the tower), or a child the
    /// graph cannot identify. Unknown ownership is not proven absence.
    pub fn owner_of_site<'p>(
        &self,
        site: &OwnedSite,
        paths: impl FnOnce() -> &'p ConstructionPaths,
    ) -> Option<SiteOwnership> {
        if matches!(site.resolution, OwnerResolution::Unanalyzable(_)) {
            return None;
        }
        let key = site.child_key.as_deref()?;
        let enclosing = &self.declarations[site.enclosing_decl].name;
        if self.rows.accepts_ancestor(enclosing, key) {
            return Some(SiteOwnership { resolution: OwnerResolution::SelfOwned(enclosing.clone()), unowned: None });
        }
        let mut climb = PathClimb { graph: self, paths: paths(), child: key, owners: BTreeSet::new(), unowned: None };
        climb.up(&mut vec![site.enclosing_decl]);
        let resolution = match (&climb.unowned, climb.owners.len()) {
            (Some(_), _) | (None, 0) => OwnerResolution::Orphan,
            (None, 1) => OwnerResolution::Ancestor(climb.owners.into_iter().next().expect("one owner")),
            (None, _) => OwnerResolution::PerPath(climb.owners.into_iter().collect()),
        };
        Some(SiteOwnership { resolution, unowned: climb.unowned })
    }

    /// Every construction path of each declaration, for
    /// [`Self::owner_of_site`]: how an instance of it comes to exist.
    ///
    /// The placement table supplies the paths the graph's own walk cannot
    /// see: every instance row (a params field of its owner row's
    /// declaration; at a template's top, a root: a literal directly in
    /// `fn main`, the root's construction or the entry's implicit one, an
    /// adapter of the root's `bindings { }`) and every dynamic site (a
    /// literal in a locus's bodies, or in a free fn, whose callers the
    /// climb does not follow). What the table records as a hole is a path
    /// with no ancestor: a field held from an instance built elsewhere
    /// that the table does not link, a field whose initializer is no
    /// literal, a dynamic site whose domains are unknown, an owner row
    /// whose declaration does not resolve. A held row the table links
    /// is its source row's projection and adds no path: the source row is
    /// the construction.
    ///
    /// The walk's own edges stay paths too (a literal in a locus's bodies
    /// or params defaults is built within that locus): the table does not
    /// enumerate the params subtree of a locus built only dynamically.
    /// Every path the union adds can only take a proof away, never give
    /// one.
    pub fn construction_paths(&self, table: &PlacementTable, bundle: &Bundle<'_>) -> ConstructionPaths {
        let ids = &bundle.snapshot;
        let decl_of = |r: &SiteRef| {
            (r.universe == SiteUniverse::User)
                .then(|| self.declarations.iter().position(|d| d.id == Some(r.id)))
                .flatten()
        };
        let span_of = |r: Option<SiteRef>, or: usize| {
            r.filter(|r| r.universe == SiteUniverse::User)
                .and_then(|r| ids.site(r.id))
                .map_or(self.declarations[or].span, |s| s.span)
        };
        let mut paths = ConstructionPaths { of: vec![Vec::new(); self.declarations.len()] };
        for s in &self.sites {
            if let Some(child) = s.child_decl {
                paths.add(child, ConstructionPath::Within(s.enclosing_decl));
            }
        }
        let entry_literals: BTreeSet<SiteRef> = table.entry_literals.iter().map(|c| c.literal).collect();
        let hole_at = |at: HoleAt| table.holes.iter().filter(move |h| h.at == at).map(|h| &h.kind);
        for (key, row) in &table.instances {
            if row.built_by.is_some() {
                continue;
            }
            let Some(d) = row.realizes.as_ref().and_then(|r| decl_of(&r.site)) else { continue };
            let at = span_of(row.literal, d);
            let hole = hole_at(HoleAt::Instance(key.clone())).find_map(|k| match k {
                HoleKind::Reuse { source } => Some(format!(
                    "held from `{source}`, an instance built elsewhere that the placement table does not link"
                )),
                HoleKind::UnenumerableInitializer => {
                    Some("as a field whose initializer is no literal the placement table can read".to_string())
                }
                _ => None,
            });
            let path = match (hole, &row.owner) {
                (Some(what), _) => ConstructionPath::Hole { what, at },
                (None, Some(owner)) => {
                    match table.instances.get(owner).and_then(|o| o.realizes.as_ref()).and_then(|r| decl_of(&r.site)) {
                        Some(p) => ConstructionPath::Within(p),
                        None => ConstructionPath::Hole {
                            what: "as a field of an instance whose declaration the placement table does not resolve"
                                .to_string(),
                            at,
                        },
                    }
                }
                (None, None) => ConstructionPath::Root {
                    what: match key.origin {
                        Origin::Construction(l) if entry_literals.contains(&l) => "directly in `fn main`",
                        Origin::Construction(_) => "as the program's root",
                        Origin::Entry(_) => "as the program's root, by the entry",
                        Origin::Binding(_) => "as an adapter of the root's `bindings { }`",
                    }
                    .to_string(),
                    at,
                },
            };
            paths.add(d, path);
        }
        let fns = free_fn_names(bundle);
        for site in &table.dynamic {
            let Some(d) = site.realizes.as_ref().and_then(|r| decl_of(&r.site)) else { continue };
            let at = span_of(Some(site.literal), d);
            let path = match &site.enclosing {
                Enclosing::Fn(f) => ConstructionPath::Hole {
                    what: match fns.get(&f.id).filter(|_| f.universe == SiteUniverse::User) {
                        Some(name) => format!("in the free fn `{name}`, whose callers the ownership graph does not follow"),
                        None => "in a free fn, whose callers the ownership graph does not follow".to_string(),
                    },
                    at,
                },
                Enclosing::Locus(p) if site.domains.is_empty() => ConstructionPath::Hole {
                    what: format!(
                        "in `{}`, where the placement table knows no domain: {}",
                        p.lowered,
                        hole_at(HoleAt::Dynamic(site.literal))
                            .find_map(|k| match k {
                                HoleKind::UnknownDomains { reason } => Some(reason.as_str()),
                                _ => None,
                            })
                            .unwrap_or("its domains are unknown")
                    ),
                    at,
                },
                Enclosing::Locus(p) => match decl_of(&p.site) {
                    Some(p) => ConstructionPath::Within(p),
                    None => ConstructionPath::Hole {
                        what: format!("in `{}`, a declaration the ownership graph does not hold", p.lowered),
                        at,
                    },
                },
            };
            paths.add(d, path);
        }
        paths
    }

    /// Interest-based ownership, artifact #2b — **owner-forwarding sets**.
    ///
    /// #2 bubbled a child `I` to a *singleton* ancestor `A` by
    /// constant-folding `A`'s pointer through a global. #2b generalizes
    /// to a NON-singleton ancestor whose instance pointer cannot be a
    /// global constant (two `A`s, two subtrees): the owner pointer must
    /// be **threaded** down the birth chain via a hidden per-locus
    /// `__owner_for_<I>` field, so each `A` collects only the `I`s born
    /// in its own subtree (instance isolation).
    ///
    /// This computes, per locus type, the set of interest-types it must
    /// *carry* that field for. For every site resolving to a
    /// NON-singleton `Ancestor(A)` (`OwnerKind::Ancestor`, skipping
    /// `SingletonConst` which stays on #2's global path) with
    /// `EdgeClass::SameTower`, EVERY intermediary on EVERY instantiation
    /// path from the enclosing locus UP to — but excluding — `A` gets
    /// `site.child_ty` added to its set. The enclosing locus is included
    /// (it holds the field it reads at the bubble site); `A` itself is
    /// excluded (it *accepts* `I` — it is the owner, not a forwarder).
    ///
    /// Because the site resolved to a UNIQUE `Ancestor(A)` with no orphan
    /// path, no intermediary between the enclosing locus and `A` accepts
    /// `I` (else the climb would have stopped there), and every branch
    /// reaches `A` (else it would be `Orphan`/`PerPath`) — so the walk
    /// never climbs above `A` and never over-includes an off-path node.
    /// Cycle-safe via a per-site visited set, mirroring [`climb`].
    pub fn compute_forwarding_sets(
        &self,
    ) -> BTreeMap<String, BTreeSet<String>> {
        let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for site in &self.sites {
            let owner = match &site.resolution {
                OwnerResolution::Ancestor(a) => a.as_str(),
                // SelfOwned / PerPath / Orphan / Unanalyzable never thread.
                _ => continue,
            };
            // Singletons keep #2's global-constant path; only genuine
            // multi-instance ancestors thread. Cross-pool is #3.
            if site.owner_kind != OwnerKind::Ancestor {
                continue;
            }
            if site.edge_class != EdgeClass::SameTower {
                continue;
            }
            let mut visited: BTreeSet<String> = BTreeSet::new();
            collect_forwarding(
                &site.enclosing_locus,
                owner,
                &site.child_ty,
                &self.instantiated_by,
                &mut visited,
                &mut out,
            );
        }
        out
    }

    /// The bubble plans lowering acts on: the graph's sites distilled to
    /// the ones a bubble moves, plus [`Self::compute_forwarding_sets`].
    /// Resolution depends only on `(enclosing_locus, child_ty)` (the
    /// climb walks the static instantiated-by relation, not a runtime
    /// path), so every plan keys on that pair.
    pub fn bubble_plans(&self) -> BubblePlans {
        let mut plan: BTreeMap<(String, String), String> = BTreeMap::new();
        let mut nonsingleton: BTreeMap<(String, String), String> =
            BTreeMap::new();
        // Interest-based ownership #3: the cross-pool twin. A site
        // resolving to `Ancestor(A)` with `OwnerKind::SingletonConst`
        // AND `EdgeClass::CrossPool` (A a program-start singleton on a
        // different thread than the enclosing locus) lands here — the
        // child is born on A's thread via the async post+dispatch path.
        // Non-singleton cross-pool has no compile-time pool handle for A
        // → NOT admitted (stays transient, deferred).
        let mut crosspool: BTreeMap<(String, String), String> =
            BTreeMap::new();
        // U-1: a `Mixed` edge keeps its resolved owner; the mechanism is
        // chosen per enclosing instance where lowering can emit both
        // arms (a singleton owner: same-tower on its thread, a cross-pool
        // post off it), and refused, located, where it cannot.
        let mut mixed: BTreeMap<(String, String), MixedPlan> = BTreeMap::new();
        for site in &self.sites {
            if let OwnerResolution::Ancestor(owner) = &site.resolution {
                let key =
                    (site.enclosing_locus.clone(), site.child_ty.clone());
                match (&site.edge_class, &site.owner_kind) {
                    (EdgeClass::SameTower, OwnerKind::SingletonConst) => {
                        plan.insert(key, owner.clone());
                    }
                    (EdgeClass::SameTower, OwnerKind::Ancestor) => {
                        nonsingleton.insert(key, owner.clone());
                    }
                    (EdgeClass::CrossPool, OwnerKind::SingletonConst) => {
                        crosspool.insert(key, owner.clone());
                    }
                    (EdgeClass::Mixed, kind) => {
                        mixed.insert(
                            key,
                            MixedPlan {
                                owner: owner.clone(),
                                singleton: *kind == OwnerKind::SingletonConst,
                                instances: site.instances.clone(),
                            },
                        );
                    }
                    // CrossPool + non-singleton (no static pool handle),
                    // Open, per-path, orphan: stay transient.
                    _ => {}
                }
            }
        }
        let forwarding = self.compute_forwarding_sets();
        BubblePlans {
            singleton: plan,
            nonsingleton,
            crosspool,
            mixed,
            forwarding,
        }
    }

    /// Every literal lowering builds from a default, with the locus it
    /// is lowered under (F.40 phase 3, C3 rest, and its review). A
    /// default is not lowered where it is written: a params default where
    /// a literal leaves its field unsupplied, an argument default at each
    /// call that leaves the argument out (`omitted`, the typed-body
    /// table's `omitted_args`; a call that supplies it expands nothing),
    /// each in the scope that holds that literal or call and under that
    /// scope's locus. So a default's context is a locus whose own member
    /// bodies hold a literal or a call reaching it, through any chain of
    /// defaults of either kind: a default that leaves a param of the
    /// locus it builds to that param's default, or calls a fn that leaves
    /// an argument to its own. Each (literal, context, root) once, where
    /// the root is the literal or call in the context's body that starts
    /// the chain; the default's literals only, never a body's own.
    ///
    /// A closure's assertion is one of its locus's member bodies here. A
    /// `const`'s value and a type's field default are lowered at every use,
    /// under the locus of the use, which no row relates to the position, so
    /// what they hold is expanded under every locus, with the position
    /// recorded ([`Expansion::per_use`]). A literal or call in a free fn's
    /// body (`fn main` included) or a binding entry is lowered under no
    /// locus, so what it reaches has no context here. `posted` is the
    /// cross-pool plan: a literal it holds
    /// for its context is posted to its owner's thread before its params
    /// are built, so no params default beneath it is expanded under that
    /// context.
    pub fn expansions(
        &self,
        posted: &BTreeMap<(String, String), String>,
        omitted: &crate::typed_bodies::OmittedArgsByCall,
    ) -> Vec<Expansion> {
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
        enum Node {
            Literal(ExpandedLiteral),
            Call(usize),
        }
        // What each default holds: a params default by (declaration,
        // field), an argument default by (callee, index).
        let mut params: BTreeMap<(usize, &str), Vec<Node>> = BTreeMap::new();
        let mut args: BTreeMap<u32, Vec<(usize, Node)>> = BTreeMap::new();
        for (i, site) in self.sites.iter().enumerate() {
            let node = Node::Literal(ExpandedLiteral::Owned(i));
            match (&site.params_field, &site.arg_default) {
                (Some(field), _) if site.params_default => {
                    params.entry((site.enclosing_decl, field.as_str())).or_default().push(node)
                }
                (_, Some(arg)) => args.entry(arg.callee).or_default().push((arg.index, node)),
                _ => {}
            }
        }
        for (i, site) in self.free_fn_sites.iter().enumerate() {
            if let Some(arg) = &site.arg_default {
                args.entry(arg.callee).or_default().push((arg.index, Node::Literal(ExpandedLiteral::Free(i))));
            }
        }
        for (c, call) in self.calls.iter().enumerate() {
            match (call.enclosing_decl, &call.params_field, &call.arg_default) {
                (Some(decl), Some(field), _) => params.entry((decl, field.as_str())).or_default().push(Node::Call(c)),
                (_, _, Some(arg)) => args.entry(arg.callee).or_default().push((arg.index, Node::Call(c))),
                _ => {}
            }
        }
        // The roots: what a locus's own member bodies lower (a closure's
        // assertion among them), each under that locus; and what a
        // position lowering emits at every use lowers, under every locus,
        // since no row says which uses it.
        let mut pending: Vec<(Node, usize, Span, Option<PerUsePosition>)> = Vec::new();
        for (i, site) in self.sites.iter().enumerate() {
            if !site.params_default && site.arg_default.is_none() {
                pending.push((Node::Literal(ExpandedLiteral::Owned(i)), site.enclosing_decl, site.span, None));
            }
        }
        let every = 0..self.declarations.len();
        for (i, other) in self.other_sites.iter().enumerate() {
            let node = Node::Literal(ExpandedLiteral::Other(i));
            match (other.position, other.enclosing_decl) {
                (OtherPosition::Closure, Some(decl)) => pending.push((node, decl, other.site.span, None)),
                (OtherPosition::Closure, None) => {}
                (OtherPosition::PerUse(p), _) => {
                    pending.extend(every.clone().map(|d| (node, d, other.site.span, Some(p))))
                }
            }
        }
        for (c, call) in self.calls.iter().enumerate() {
            match (call.enclosing_decl, &call.params_field, &call.arg_default, call.per_use) {
                (Some(decl), None, None, None) => pending.push((Node::Call(c), decl, call.span, None)),
                (None, None, None, Some(p)) => pending.extend(every.clone().map(|d| (Node::Call(c), d, call.span, Some(p)))),
                _ => {}
            }
        }
        // Every node reached from a root is in a default: a root's own
        // literal is never reported, only what it expands.
        let mut seen: BTreeSet<(Node, usize, u32, u32)> = BTreeSet::new();
        let mut out = Vec::new();
        while let Some((node, context, root, per_use)) = pending.pop() {
            if !seen.insert((node, context, root.start.0, root.end.0)) {
                continue;
            }
            let reached: Vec<Node> = match node {
                Node::Literal(literal) => {
                    let s = self.literal(literal);
                    let (child_ty, child_decl, supplied) = (&s.child_ty, s.child_decl, &s.supplied);
                    if posted.contains_key(&(self.declarations[context].name.clone(), child_ty.clone())) {
                        Vec::new()
                    } else if let Some(child) = child_decl {
                        params
                            .range((child, "")..)
                            .take_while(|((d, _), _)| *d == child)
                            .filter(|((_, field), _)| !supplied.contains(*field))
                            .flat_map(|(_, nodes)| nodes.iter().copied())
                            .collect()
                    } else {
                        Vec::new()
                    }
                }
                Node::Call(c) => omitted
                    .get(&self.calls[c].id)
                    .into_iter()
                    .flatten()
                    .flat_map(|o| {
                        args.get(&o.callee).into_iter().flatten().filter(move |(i, _)| *i >= o.from).map(|(_, n)| *n)
                    })
                    .collect(),
            };
            for next in reached {
                if let Node::Literal(literal) = next {
                    out.push(Expansion { literal, context, root, per_use });
                }
                pending.push((next, context, root, per_use));
            }
        }
        out.sort_by_key(|e| (e.literal, e.context, e.root.start.0, e.root.end.0));
        out.dedup();
        out
    }

    /// The literal an [`ExpandedLiteral`] names, as a free-standing site
    /// (an owned site's child, supplied fields, bareness, span and
    /// argument default).
    pub fn literal(&self, literal: ExpandedLiteral) -> FreeFnSite {
        match literal {
            ExpandedLiteral::Owned(i) => {
                let s = &self.sites[i];
                FreeFnSite {
                    child_ty: s.child_ty.clone(),
                    span: s.span,
                    child_decl: s.child_decl,
                    child_key: s.child_key.clone(),
                    supplied: s.supplied.clone(),
                    bare_statement: s.bare_statement,
                    arg_default: s.arg_default.clone(),
                }
            }
            ExpandedLiteral::Free(i) => self.free_fn_sites[i].clone(),
            ExpandedLiteral::Other(i) => self.other_sites[i].site.clone(),
        }
    }
}

/// The bubble plans lowering reads, projected from the graph by
/// [`OwnershipGraph::bubble_plans`]. The four plans key on
/// `(enclosing locus, child type)` and carry the owner locus type `A`;
/// they are DISJOINT (a site has one edge class and one owner kind).
/// Every other resolution (SelfOwned direct-parent, non-singleton
/// cross-pool, per-path, orphan, open) is in none of them and stays
/// transient; a `Mixed` edge never does (its plan is `mixed`). `BubblePlans::default()` is the empty plan: no bubble,
/// no threading field, nothing stitched — the differential control
/// arm codegen's `LOTUS_NO_OWNERSHIP_BUBBLE=1` selects.
#[derive(Debug, Clone, Default)]
pub struct BubblePlans {
    /// Interest-based ownership #2, the SameTower + SingletonConst plan:
    /// an `I{}` born deep inside locus `B` that resolves to a UNIQUE
    /// accepting ancestor `A` where `A` is a `main locus` / `@export`
    /// singleton on the same OS thread as `B`. `A`'s pointer folds to
    /// a global, so the child bubbles to it directly.
    pub singleton: BTreeMap<(String, String), String>,
    /// #2b, the SameTower + Ancestor plan: the same bubble to an `A`
    /// with MULTIPLE instances, whose pointer cannot be a constant and
    /// is threaded down the birth chain in hidden `__owner_for_<I>`
    /// fields (see `forwarding`).
    pub nonsingleton: BTreeMap<(String, String), String>,
    /// #3, the CrossPool + SingletonConst plan: `A` a singleton on a
    /// DIFFERENT pool/thread than `B`, so the child is born on `A`'s
    /// thread through the async post + dispatch path (a bare `I{};`
    /// statement only).
    pub crosspool: BTreeMap<(String, String), String>,
    /// U-1, the `Mixed` plan: some instances of `B` run on `A`'s thread
    /// and some do not (or run where the table cannot say). `A` stays
    /// the owner of every one. For a singleton `A` a bare `I{};` takes
    /// the same-tower bubble on `A`'s thread and the cross-pool post off
    /// it, chosen at the site; a value use, or a non-singleton `A` (no
    /// static handle for the post), is refused at the literal.
    pub mixed: BTreeMap<(String, String), MixedPlan>,
    /// #2b's forwarding sets ([`OwnershipGraph::compute_forwarding_sets`]):
    /// locus type → the interest types `I` it carries an
    /// `__owner_for_I` field for.
    pub forwarding: BTreeMap<String, BTreeSet<String>>,
}

// === Construction paths (type-check rule 20) ======================

/// One way an instance of a locus declaration comes to exist
/// ([`OwnershipGraph::construction_paths`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConstructionPath {
    /// Built within an instance of this declaration (an index into
    /// [`OwnershipGraph::declarations`]): a params field of it, or a
    /// literal in one of its bodies.
    Within(usize),
    /// Built where no locus encloses it, so nothing above it can accept
    /// what it births: `what` says where, `at` is the literal (the
    /// entry's site for the entry's implicit construction).
    Root { what: String, at: Span },
    /// A path the placement table records as a hole: it proves no
    /// owner, so it counts as a path with none.
    Hole { what: String, at: Span },
}

/// Every declaration's construction paths, indexed like
/// [`OwnershipGraph::declarations`]. A declaration with none is built
/// nowhere the graph or the placement table sees.
#[derive(Debug, Clone, Default)]
pub struct ConstructionPaths {
    pub of: Vec<Vec<ConstructionPath>>,
}

impl ConstructionPaths {
    fn add(&mut self, decl: usize, path: ConstructionPath) {
        let paths = &mut self.of[decl];
        if !paths.contains(&path) {
            paths.push(path);
        }
    }
}

/// What [`OwnershipGraph::owner_of_site`] decides for a site.
#[derive(Debug, Clone)]
pub struct SiteOwnership {
    pub resolution: OwnerResolution,
    /// For an `Orphan`, the first construction path found with no
    /// acceptor on it.
    pub unowned: Option<UnownedPath>,
}

/// A construction path of a site's enclosing locus that no ancestor
/// accepting the child lies on.
#[derive(Debug, Clone)]
pub struct UnownedPath {
    /// The declarations climbed, the enclosing locus first, each built
    /// within the next.
    pub through: Vec<usize>,
    /// How the last of them is built: a `Root` or a `Hole`. `None` when
    /// it has no construction path, or every one closes a cycle.
    pub end: Option<ConstructionPath>,
}

/// The climb up the construction paths from a site's enclosing locus:
/// the nearest acceptor of `child` on each path, and the first path that
/// reaches none. Once one is found the site is `Orphan`, so the climb
/// stops.
struct PathClimb<'g> {
    graph: &'g OwnershipGraph,
    paths: &'g ConstructionPaths,
    child: &'g str,
    owners: BTreeSet<String>,
    unowned: Option<UnownedPath>,
}

impl PathClimb<'_> {
    fn accepts(&self, decl: usize) -> bool {
        self.graph.rows.accepts_ancestor(&self.graph.declarations[decl].name, self.child)
    }

    /// `chain` is the path climbed so far, its last entry the declaration
    /// whose paths are read; a path back onto `chain` is a cycle, which
    /// finds no new acceptor.
    fn up(&mut self, chain: &mut Vec<usize>) {
        let node = *chain.last().expect("the climb starts at the enclosing locus");
        let paths = self.paths.of.get(node).map(Vec::as_slice).unwrap_or(&[]);
        let fresh: Vec<&ConstructionPath> = paths
            .iter()
            .filter(|p| !matches!(p, ConstructionPath::Within(d) if chain.contains(d)))
            .collect();
        if fresh.is_empty() {
            self.unowned = Some(UnownedPath { through: chain.clone(), end: None });
            return;
        }
        for path in fresh {
            if self.unowned.is_some() {
                return;
            }
            match path {
                ConstructionPath::Within(p) if self.accepts(*p) => {
                    self.owners.insert(self.graph.declarations[*p].name.clone());
                }
                ConstructionPath::Within(p) => {
                    chain.push(*p);
                    self.up(chain);
                    chain.pop();
                }
                ConstructionPath::Root { .. } | ConstructionPath::Hole { .. } => {
                    self.unowned = Some(UnownedPath { through: chain.clone(), end: Some(path.clone()) });
                }
            }
        }
    }
}

/// Every free fn's name, by its site: a dynamic site's enclosing fn.
fn free_fn_names(bundle: &Bundle<'_>) -> BTreeMap<SiteId, String> {
    fn walk(items: &[TopDecl], ids: &crate::snapshot::Snapshot, out: &mut BTreeMap<SiteId, String>) {
        for item in items {
            match item {
                TopDecl::Fn(f) => {
                    if let Some(id) = ids.site_id(f.id) {
                        out.insert(id, f.name.name.clone());
                    }
                }
                TopDecl::Module(m) => walk(&m.items, ids, out),
                _ => {}
            }
        }
    }
    let mut out = BTreeMap::new();
    for program in bundle.programs.values() {
        walk(&program.items, &bundle.snapshot, &mut out);
    }
    out
}

/// One `Mixed` edge's plan ([`BubblePlans::mixed`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MixedPlan {
    /// The resolved owner, `A`.
    pub owner: String,
    /// `A` is a program-start singleton, so a post has a static target.
    pub singleton: bool,
    /// The enclosing instances and their domains, then the owner's
    /// ([`OwnedSite::instances`]): what a refusal names.
    pub instances: Vec<String>,
}

/// DFS upward from `node` toward `owner` via `instantiated_by`, adding
/// `child` to the forwarding set of every intermediary reached (the
/// enclosing locus and every locus above it up to, but excluding,
/// `owner`). `visited` guards instantiation cycles.
fn collect_forwarding(
    node: &str,
    owner: &str,
    child: &str,
    instantiated_by: &BTreeMap<String, BTreeSet<String>>,
    visited: &mut BTreeSet<String>,
    out: &mut BTreeMap<String, BTreeSet<String>>,
) {
    // `owner` accepts `child` — it is the owner, not a forwarder. Stop
    // (and never climb above it).
    if node == owner {
        return;
    }
    if !visited.insert(node.to_string()) {
        return;
    }
    out.entry(node.to_string()).or_default().insert(child.to_string());
    if let Some(parents) = instantiated_by.get(node) {
        for p in parents {
            collect_forwarding(
                p,
                owner,
                child,
                instantiated_by,
                visited,
                out,
            );
        }
    }
}

// === Shared walk ==================================================

/// One locus's ownership-relevant facts, collected in a single walk.
#[derive(Debug, Clone, Default)]
struct LocusFacts {
    /// Child types this locus declares `accept(_: T)` for, resolved by
    /// `child_locus_name` (a type that names no locus is accepted by
    /// no one).
    accepts: BTreeSet<String>,
    /// Locus-typed literals born in this locus's members or params defaults.
    instantiates: Vec<RawSite>,
    /// Projection class from a `: projection …` annotation, if any.
    projection: Option<ProjectionClass>,
    /// `main locus` / `@export locus` — a provably-unique instance.
    singleton: bool,
}

/// A single instantiation literal captured during the walk.
#[derive(Debug, Clone)]
struct RawSite {
    id: NodeId,
    supplied: BTreeSet<String>,
    params_field: Option<String>,
    binding: Option<NodeId>,
    // Filled once by `identify_child` after the structural walk, when
    // the literal's declared binding type is also known.
    child_ty: String,
    child_decl: Option<usize>,
    child_key: Option<String>,
    span: Span,
    /// The literal's path as written.
    path: Vec<String>,
    /// The type the literal's binding or field declares, when the
    /// literal is a `let`'s or a params default's whole value.
    declared: Option<TypeExpr>,
    /// See [`OwnedSite::enclosing_decl`] and [`OwnedSite::member`].
    enclosing_decl: usize,
    member: Option<String>,
    params_default: bool,
    bare_statement: bool,
    /// See [`OwnedSite::arg_default`].
    arg_default: Option<ArgDefault>,
    /// The position is a call, not a literal: kept for default
    /// expansion ([`CallSite`]), never resolved as a birth.
    call: bool,
    /// A position outside the member-body walk ([`OtherPosition`]).
    other: Option<OtherPosition>,
}

/// The rows a graph is assembled from (F.40 phase 3, C5): the product of
/// one structural walk and shared child resolution, per-locus facts,
/// declaration identities and the closed-world entry flag. Rows over two
/// programs concatenate into rows over both ([`OwnershipRows::then`]):
/// lowering's graph is the snapshot's rows followed by the stdlib's
/// ([`lowering_ownership_graph`]).
#[derive(Debug, Clone, Default)]
pub struct OwnershipRows {
    facts: BTreeMap<String, LocusFacts>,
    declarations: Vec<LocusDeclRow>,
    has_entry_point: bool,
    accept_rows: AcceptRows,
    free_fn_sites: Vec<RawSite>,
    binding_sites: Vec<RawSite>,
    /// The calls default expansion reads: a locus's (in its member
    /// bodies and params defaults) and any argument default's.
    calls: Vec<RawSite>,
    /// The literals of the other positions ([`OtherPosition`]).
    other: Vec<RawSite>,
}

/// Collect `e`'s literals and calls into `other`, at `position`, under
/// the locus declaration `decl` (`usize::MAX` for none).
fn collect_other(e: &Expr, decl: usize, position: OtherPosition, other: &mut Vec<RawSite>) {
    let at = other.len();
    collect_sites_expr(e, other);
    for s in &mut other[at..] {
        s.other = Some(position);
        s.enclosing_decl = decl;
    }
}

/// A type's field defaults, each lowered at every literal of the type
/// that leaves the field.
fn collect_type_defaults(td: &TypeDecl, decl: usize, other: &mut Vec<RawSite>) {
    if let TypeDeclBody::Struct(fields) = &td.body {
        for f in fields {
            if let Some(d) = &f.default {
                collect_other(d, decl, OtherPosition::PerUse(PerUsePosition::TypeFieldDefault), other);
            }
        }
    }
}

impl OwnershipRows {
    /// The rows of a bundle: the walk its graph is assembled from
    /// ([`build_ownership_graph`]). What a bundle no snapshot holds reads
    /// the accept relation from ([`crate::build_rule_diags`]'s caller);
    /// every verb reads its snapshot's graph's.
    pub fn of(bundle: &Bundle<'_>) -> OwnershipRows {
        collect_ownership_walk(bundle, None, Some(&crate::entry::entry_row(bundle)))
    }

    /// `accepts_ancestor`: whether the locus `ancestor` declares an
    /// `accept` for `child`, the child named as the graph resolves it
    /// (`child_locus_name`). The relation the climb finds an owner by
    /// ([`OwnershipGraph::owner_of_site`]) and the borrow-lifetime law
    /// asks of a bare literal's enclosing locus.
    pub fn accepts_ancestor(&self, ancestor: &str, child: &str) -> bool {
        self.facts.get(ancestor).is_some_and(|f| f.accepts.contains(child))
    }

    /// These rows followed by `next`'s, as one walk over both programs
    /// would collect them: `next`'s declarations after these, its sites'
    /// declaration indices shifted past them, a locus both declare
    /// merged as the walk merges two declarations of one name (this
    /// one's projection first), and `next`'s name table, which is
    /// `next`'s program's whole (the stdlib's rows answer over the merged
    /// program's). A literal of these rows that names a locus they do not
    /// declare (a stdlib locus, from a checked program) names `next`'s
    /// one declaration of that name, as the walk over both resolves it.
    fn then(mut self, next: OwnershipRows) -> OwnershipRows {
        let shift = self.declarations.len();
        // `usize::MAX` marks a position no locus declaration holds (a
        // call in a free fn's argument default, a top-level const's or
        // type's literal): it names no declaration to shift.
        let moved = |mut s: RawSite| {
            if s.enclosing_decl != usize::MAX {
                s.enclosing_decl += shift;
            }
            s.child_decl = s.child_decl.map(|d| d + shift);
            s
        };
        let declared_next = |s: &mut RawSite| {
            let mut named = next.declarations.iter().enumerate().filter(|(_, d)| !d.generic && d.name == s.child_ty);
            if let (None, Some((i, _)), None) = (s.child_decl, named.next(), named.next()) {
                s.child_decl = Some(shift + i);
            }
        };
        for s in self.facts.values_mut().flat_map(|f| f.instantiates.iter_mut()) {
            declared_next(s);
        }
        for s in self.free_fn_sites.iter_mut().chain(self.binding_sites.iter_mut()).chain(self.other.iter_mut()) {
            declared_next(s);
        }
        for (name, f) in next.facts {
            let entry = self.facts.entry(name).or_default();
            entry.accepts.extend(f.accepts);
            entry.instantiates.extend(f.instantiates.into_iter().map(moved));
            entry.projection = entry.projection.or(f.projection);
            entry.singleton |= f.singleton;
        }
        self.declarations.extend(next.declarations);
        self.has_entry_point |= next.has_entry_point;
        self.accept_rows.rows.extend(next.accept_rows.rows);
        self.accept_rows.declared = next.accept_rows.declared;
        self.free_fn_sites.extend(next.free_fn_sites.into_iter().map(moved));
        self.binding_sites.extend(next.binding_sites.into_iter().map(moved));
        self.calls.extend(next.calls.into_iter().map(moved));
        self.other.extend(next.other.into_iter().map(moved));
        self
    }
}

/// Walk every locus and free function once, collecting accepts,
/// instantiations and their birth context, projection and singleton
/// facts, plus declaration identities and the closed-world entry flag.
/// This is the single source of truth `build_ownership_graph` consumes.
///
/// `tail` walks only those items of the bundle's program, the stdlib's
/// in a merged program ([`stdlib_ownership_rows`]): names still resolve against
/// the whole bundle, and the rows have no entry point (`entry` is `None`).
fn collect_ownership_walk(
    bundle: &Bundle<'_>,
    tail: Option<&[TopDecl]>,
    entry: Option<&crate::entry::EntryRow>,
) -> OwnershipRows {
    // Closed-world gate, mirroring `build_bus_graph`'s
    // `has_entry_point`: a bare top-level `fn main` OR an entry (the
    // entry row's) makes the ownership DAG complete (no dynamic attach
    // construct).
    let has_entry_point = entry.is_some_and(|entry| {
        entry.entry().is_some()
            || bundle
                .programs
                .values()
                .any(|p| p.items.iter().any(|i| matches!(i, TopDecl::Fn(f) if f.name.name == "main")))
    });

    // The child an `accept` names is resolved by the one resolver the
    // handler rows use, so an alias, generic arguments or a `std::` path
    // name the locus lowering resolves (F.40 phase 1.4).
    let programs: Vec<&Program> = bundle.programs.values().copied().collect();
    let declared = DeclaredNames::of(&programs);
    let renames = bundle.import_renames.as_slice();

    // Collect candidates before resolving them: their whole binding
    // supplies any specialization, and all declarations are then known.
    let mut facts: BTreeMap<String, LocusFacts> = BTreeMap::new();
    let mut declarations: Vec<LocusDeclRow> = Vec::new();
    let mut accept_rows: Vec<AcceptRow> = Vec::new();
    let mut free_fn_sites: Vec<RawSite> = Vec::new();
    let mut binding_sites: Vec<RawSite> = Vec::new();
    let mut other: Vec<RawSite> = Vec::new();
    struct WalkCx<'a> {
        declared: &'a DeclaredNames,
        renames: &'a [(Vec<String>, String)],
        snapshot: &'a crate::snapshot::Snapshot,
    }
    #[allow(clippy::too_many_arguments)]
    fn walk(
        items: &[TopDecl],
        cx: &WalkCx<'_>,
        facts: &mut BTreeMap<String, LocusFacts>,
        declarations: &mut Vec<LocusDeclRow>,
        accept_rows: &mut Vec<AcceptRow>,
        free_fn_sites: &mut Vec<RawSite>,
        binding_sites: &mut Vec<RawSite>,
        other: &mut Vec<RawSite>,
    ) {
        let (declared, renames) = (cx.declared, cx.renames);
        for item in items {
            match item {
                TopDecl::Locus(l) => {
                    let decl = declarations.len();
                    declarations.push(LocusDeclRow {
                        name: l.name.name.clone(),
                        span: l.span,
                        id: cx.snapshot.site_id(l.id),
                        generic: !l.generics.is_empty(),
                        bus_handlers: l
                            .members
                            .iter()
                            .filter_map(|m| match m {
                                LocusMember::Bus(b) => Some(&b.members),
                                _ => None,
                            })
                            .flatten()
                            .filter_map(|bm| match bm {
                                BusMember::Subscribe { handler, .. } => Some(handler.name.clone()),
                                _ => None,
                            })
                            .collect(),
                    });
                    let entry = facts.entry(l.name.name.clone()).or_default();
                    let first_site = entry.instantiates.len();
                    entry.singleton |= l.is_main || l.export;
                    if entry.projection.is_none() {
                        for ann in &l.annotations {
                            if let LocusAnnotation::Projection(pc) = ann {
                                entry.projection = Some(*pc);
                            }
                        }
                    }
                    for m in &l.members {
                        match m {
                            LocusMember::Lifecycle(ld) => {
                                if ld.kind == LifecycleKind::Accept {
                                    for p in &ld.params {
                                        accept_rows.push(AcceptRow {
                                            owner_id: l.id,
                                            owner: l.name.name.clone(),
                                            ty: p.ty.clone(),
                                            span: ld.span,
                                        });
                                        if let ChildRef::Locus(name) =
                                            child_locus_name(&p.ty, declared, renames)
                                        {
                                            entry.accepts.insert(name);
                                        }
                                    }
                                }
                                collect_sites_block(
                                    &ld.body,
                                    &mut entry.instantiates,
                                );
                            }
                            LocusMember::Fn(fd) => {
                                let from = entry.instantiates.len();
                                collect_sites_fn(fd, &mut entry.instantiates);
                                for s in &mut entry.instantiates[from..] {
                                    s.member = Some(fd.name.name.clone());
                                    if let Some(arg) = &mut s.arg_default {
                                        arg.fn_name = format!("{}.{}", l.name.name, fd.name.name);
                                    }
                                }
                            }
                            LocusMember::Mode(md) => collect_sites_block(
                                &md.body,
                                &mut entry.instantiates,
                            ),
                            LocusMember::Failure(fl) => collect_sites_block(
                                &fl.body,
                                &mut entry.instantiates,
                            ),
                            LocusMember::BirthCheck(bc) => {
                                collect_sites_expr(&bc.cond, &mut entry.instantiates);
                                if let Some(payload) = &bc.payload {
                                    collect_sites_expr(payload, &mut entry.instantiates);
                                }
                            }
                            // The positions default expansion reads apart
                            // from the birth walk (C3 rest, the review of
                            // #1351): a closure's assertion, evaluated
                            // under this locus, and the positions lowering
                            // emits at every use.
                            LocusMember::Closure(c) => {
                                if let Some(a) = &c.assertion {
                                    for e in [&a.left, &a.right, &a.tolerance] {
                                        collect_other(e, decl, OtherPosition::Closure, other);
                                    }
                                }
                            }
                            LocusMember::Const(c) => {
                                collect_other(&c.value, decl, OtherPosition::PerUse(PerUsePosition::Const), other)
                            }
                            LocusMember::Type(td) => collect_type_defaults(td, decl, other),
                            // A params default `child: C = C { ... }` is
                            // a real ownership edge (this locus gives
                            // birth to `C` as its own initial state) —
                            // the same edge the placement block pins on
                            // a `main locus` field. Collecting it lets
                            // the ancestor climb connect a pool-placed
                            // consumer up to its owner (interest #3's
                            // cross-pool bubble), and it only ADDS edges
                            // (Orphan→Ancestor), never removes them.
                            LocusMember::Params(pb) => {
                                for p in &pb.params {
                                    if let ParamInit::Value(e) = &p.init {
                                        let at = entry.instantiates.len();
                                        collect_sites_expr(
                                            e,
                                            &mut entry.instantiates,
                                        );
                                        declare_site(
                                            &mut entry.instantiates,
                                            at,
                                            e,
                                            p.ty.as_ref(),
                                        );
                                        for site in &mut entry.instantiates[at..] {
                                            site.params_default = true;
                                            site.params_field = Some(p.name.name.clone());
                                        }
                                    }
                                }
                            }
                            LocusMember::Bindings(bb) => {
                                for binding in &bb.entries {
                                    let TransportSpec::Adapter { locus, inits, .. } = &binding.transport else { continue };
                                    let at = binding_sites.len();
                                    binding_sites.push(RawSite {
                                        id: binding.id,
                                        supplied: inits.iter().map(|i| i.name.name.clone()).collect(),
                                        params_field: None,
                                        binding: Some(binding.id),
                                        child_ty: String::new(), child_decl: None, child_key: None,
                                        span: binding.span, path: vec![locus.name.clone()], declared: None,
                                        enclosing_decl: decl, member: None, params_default: false, bare_statement: false,
                                        arg_default: None, call: false, other: None,
                                    });
                                    for init in inits { collect_sites_expr(&init.value, binding_sites); }
                                    for site in &mut binding_sites[at..] {
                                        site.binding = Some(binding.id);
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    for s in &mut entry.instantiates[first_site..] {
                        s.enclosing_decl = decl;
                    }
                }
                TopDecl::Fn(f) => collect_sites_fn(f, free_fn_sites),
                TopDecl::Const(c) => {
                    collect_other(&c.value, usize::MAX, OtherPosition::PerUse(PerUsePosition::Const), other)
                }
                TopDecl::Type(td) => collect_type_defaults(td, usize::MAX, other),
                TopDecl::Module(m) => {
                    walk(&m.items, cx, facts, declarations, accept_rows, free_fn_sites, binding_sites, other)
                }
                _ => {}
            }
        }
    }
    let cx = WalkCx { declared: &declared, renames, snapshot: &bundle.snapshot };
    let walked: Vec<&[TopDecl]> = match tail {
        Some(items) => vec![items],
        None => programs.iter().map(|p| p.items.as_slice()).collect(),
    };
    for items in walked {
        walk(
            items,
            &cx,
            &mut facts,
            &mut declarations,
            &mut accept_rows,
            &mut free_fn_sites,
            &mut binding_sites,
            &mut other,
        );
    }

    // The calls leave the literal positions here: a locus's, with its
    // declaration, a free fn's argument defaults', and the other
    // positions' (the rest of a free fn's, and a binding entry's, are
    // lowered under no locus). `usize::MAX` marks a position no locus
    // declaration holds.
    let mut calls: Vec<RawSite> = Vec::new();
    for facts in facts.values_mut() {
        let (c, literals): (Vec<RawSite>, Vec<RawSite>) =
            std::mem::take(&mut facts.instantiates).into_iter().partition(|s| s.call);
        calls.extend(c);
        facts.instantiates = literals;
    }
    let (c, literals): (Vec<RawSite>, Vec<RawSite>) = free_fn_sites.into_iter().partition(|s| s.call);
    calls.extend(c.into_iter().filter(|s| s.arg_default.is_some()).map(|s| RawSite { enclosing_decl: usize::MAX, ..s }));
    let mut free_fn_sites = literals;
    let (c, mut other): (Vec<RawSite>, Vec<RawSite>) = other.into_iter().partition(|s| s.call);
    calls.extend(c);
    binding_sites.retain(|s| !s.call);
    for facts in facts.values_mut() {
        facts.instantiates.retain_mut(|s| identify_child(s, &declarations, &declared, renames, &bundle.snapshot));
    }
    free_fn_sites.retain_mut(|s| identify_child(s, &declarations, &declared, renames, &bundle.snapshot));
    binding_sites.retain_mut(|s| identify_child(s, &declarations, &declared, renames, &bundle.snapshot));
    other.retain_mut(|s| identify_child(s, &declarations, &declared, renames, &bundle.snapshot));
    OwnershipRows {
        free_fn_sites,
        binding_sites,
        calls,
        other,
        facts,
        declarations,
        has_entry_point,
        accept_rows: AcceptRows {
            rows: accept_rows,
            declared,
            renames: renames.to_vec(),
        },
    }
}

/// Record `declared` as the type of the site `value` pushed at `at`, when
/// `value` is itself the literal (not a literal nested inside it).
fn declare_site(out: &mut [RawSite], at: usize, value: &Expr, declared: Option<&TypeExpr>) {
    let (Some(te), Expr::Struct { .. }) = (declared, value) else { return };
    // The walk pushes the whole literal before its initializer values,
    // even when that literal later resolves to a plain record. A nested
    // locus cannot inherit the record's type through an overlapping span.
    if let Some(site) = out.get_mut(at) {
        site.declared = Some(te.clone());
    }
}

/// [`OwnedSite::child_decl`] and [`OwnedSite::child_key`] for a raw
/// site: the declaration the literal names and the child an `accept`
/// would have to name to own it.
fn identify_child(
    site: &mut RawSite,
    declarations: &[LocusDeclRow],
    declared: &DeclaredNames,
    renames: &[(Vec<String>, String)],
    snapshot: &crate::snapshot::Snapshot,
) -> bool {
    let written = TypeExpr::Named {
        path: QualifiedName {
            segments: site.path.iter().map(|s| Ident::new(s.clone(), site.span)).collect(),
            span: site.span,
        },
        generic_args: Vec::new(),
        span: site.span,
    };
    // The same resolver as accept and handler types follows aliases,
    // imports and stdlib paths. Plain records and unresolved paths are
    // not ownership births, even if their final segment names a locus.
    let Some(resolved) = resolve_locus_type(&written, declared, renames) else { return false };
    let child_decl = match resolved.at {
        Some(DeclAt::Program(node)) if !node.is_none() => snapshot.site_id(node)
            .and_then(|id| declarations.iter().position(|d| d.id == Some(id))),
        Some(DeclAt::Program(_)) => declarations.iter().position(|d| d.name == resolved.declaration),
        Some(DeclAt::Stdlib(_)) | None => None,
    };
    let generic = child_decl.is_some_and(|d| declarations[d].generic);
    let key = if generic && resolved.name == resolved.declaration {
        // A bare template needs its whole binding's declared type. An
        // alias that already resolves to a specialization keeps that
        // answer; it does not need an additional type annotation.
        site.declared.as_ref()
            .and_then(|te| resolve_locus_type(te, declared, renames))
            .filter(|r| r.declaration == resolved.declaration && r.name != r.declaration)
            .map(|r| r.name)
    } else {
        Some(resolved.name)
    };
    site.child_ty = key.clone().unwrap_or(resolved.declaration);
    site.child_decl = child_decl;
    site.child_key = key;
    true
}

// === Build ========================================================

/// Build the authoritative [`OwnershipGraph`] for a bundle.
///
/// `_top` is accepted for API symmetry with
/// [`crate::bus_graph::build_bus_graph`] (both are the post-typecheck
/// analysis passes over a bundle); ownership resolution needs no
/// resolved-type scope, so it is unused today. Structural twin of
/// `build_bus_graph`: one shared walk ([`collect_ownership_walk`]),
/// then per-site owner resolution + edge classification. `placement`
/// is the snapshot's table (a bundle no snapshot holds reads
/// [`crate::placement::bundle_placement`]): each edge's class compares
/// the domains of the enclosing locus's instances with the owner's.
/// Resolution never reads it: who owns a child is a fact of the site,
/// whatever thread delivers it (U-1).
pub fn build_ownership_graph(
    bundle: &Bundle<'_>,
    _top: &TopScope,
    placement: &crate::placement::PlacementTable,
    entry: &crate::entry::EntryRow,
) -> OwnershipGraph {
    assemble(collect_ownership_walk(bundle, None, Some(entry)), &bundle.snapshot, placement)
}

/// The rows of the stdlib's loci and free fns in a merged program:
/// `stdlib` is the stdlib's items, the tail of the program `bundle`
/// holds. The snapshot's rows are derived over the checked programs,
/// which hold no stdlib, so the stdlib's are the one part of lowering's
/// graph the merged program answers itself ([`lowering_ownership_graph`]); their
/// names resolve against the whole merged program, as they did when the
/// graph was built over it.
pub fn stdlib_ownership_rows(bundle: &Bundle<'_>, stdlib: &[TopDecl]) -> OwnershipRows {
    collect_ownership_walk(bundle, Some(stdlib), None)
}

/// Lowering's ownership graph (F.40 phase 3, C5): the snapshot's rows,
/// read for the program lowering walks through the view's
/// correspondence, followed by the stdlib's ([`stdlib_ownership_rows`]), assembled
/// with the placement table as the snapshot's graph is.
///
/// Every user site the rows name keeps its identity in the merged
/// program ([`crate::correspondence::Image::Checked`]): a literal, a
/// binding entry, a locus declaring an `accept`. Neither rewrite touches
/// a locus literal or an `accept`, so each row answers for the merged
/// program as for the checked one. `ids` is the merged program's
/// identities, which the births' literals are read under.
pub fn lowering_ownership_graph(
    snapshot: &OwnershipGraph,
    stdlib: OwnershipRows,
    ids: &crate::snapshot::Snapshot,
    correspondence: &crate::correspondence::Correspondence,
    placement: &crate::placement::PlacementTable,
) -> Result<OwnershipGraph, String> {
    use crate::correspondence::Image;
    let rows = &snapshot.rows;
    let sites = rows.facts.values().flat_map(|f| &f.instantiates).chain(&rows.free_fn_sites).chain(&rows.binding_sites);
    let ids_named = sites.map(|s| s.id).chain(rows.accept_rows.rows.iter().map(|r| r.owner_id));
    for id in ids_named.filter(|id| !id.is_none()) {
        if !matches!(correspondence.image(id), Some(Image::Checked(_))) {
            return Err(format!("the ownership row's site {} has no checked image in the merged program", id.0));
        }
    }
    Ok(assemble(rows.clone().then(stdlib), ids, placement))
}

/// The graph its rows assemble into: per-site owner resolution and edge
/// classification, with the placement table.
fn assemble(walk: OwnershipRows, ids: &crate::snapshot::Snapshot, placement: &crate::placement::PlacementTable) -> OwnershipGraph {
    let domains = placement.domains_by_type();
    let births = walk.facts.values().flat_map(|f| &f.instantiates)
        .chain(&walk.free_fn_sites).chain(&walk.binding_sites).map(|site| BirthRow {
            child_decl: site.child_decl,
            span: site.span,
            literal: ids.site_id(site.id),
            supplied: site.supplied.clone(),
            context: match (&site.params_field, site.binding) {
                (Some(field), _) => BirthContext::Default { owner: site.enclosing_decl, field: field.clone() },
                (_, Some(entry)) => BirthContext::Binding(ids.site_id(entry)),
                _ => BirthContext::Body,
            },
        }).collect();

    // The ancestor-edge relation: child locus type → the set of locus
    // types that instantiate it in a method body.
    let mut instantiated_by: BTreeMap<String, BTreeSet<String>> =
        BTreeMap::new();
    for (locus, f) in &walk.facts {
        for site in &f.instantiates {
            instantiated_by
                .entry(site.child_ty.clone())
                .or_default()
                .insert(locus.clone());
        }
    }

    // Public accepts map (owned copy).
    let accepts: BTreeMap<String, BTreeSet<String>> = walk
        .facts
        .iter()
        .map(|(k, v)| (k.clone(), v.accepts.clone()))
        .collect();

    let singletons: BTreeSet<String> = walk
        .facts
        .iter()
        .filter(|(_, f)| f.singleton)
        .map(|(k, _)| k.clone())
        .collect();

    let mut sites: Vec<OwnedSite> = Vec::new();
    // Deterministic order: `facts` is a BTreeMap (by enclosing locus),
    // then walk order within a locus.
    for (enclosing, f) in &walk.facts {
        for site in &f.instantiates {
            let child = &site.child_ty;
            let child_decl = site.child_decl;
            let child_key = site.child_key.clone();

            // Open world: the DAG may be completed by a downstream
            // consumer — resolve everything conservatively.
            if !walk.has_entry_point {
                sites.push(OwnedSite {
                    child_ty: child.clone(),
                    enclosing_locus: enclosing.clone(),
                    resolution: OwnerResolution::Unanalyzable(
                        "open world: bundle has no entry point (`fn main` \
                         or `main locus`)"
                            .to_string(),
                    ),
                    owner_kind: OwnerKind::Ancestor,
                    edge_class: EdgeClass::Open,
                    instances: Vec::new(),
                    owner_projection: None,
                    span: site.span,
                    enclosing_decl: site.enclosing_decl,
                    member: site.member.clone(),
                    child_decl,
                    child_key,
                    params_default: site.params_default,
                    bare_statement: site.bare_statement,
                    supplied: site.supplied.clone(),
                    params_field: site.params_field.clone(),
                    arg_default: site.arg_default.clone(),
                });
                continue;
            }

            let resolution =
                resolve_owner(enclosing, child, &accepts, &instantiated_by);
            let owner_kind = classify_owner_kind(&resolution, &singletons);
            let edge_class = classify_edge(enclosing, &resolution, placement, &domains);
            let instances = match (&edge_class, resolution.owner()) {
                (EdgeClass::Mixed, Some(owner)) => {
                    let mut v = placement.instances_of(enclosing);
                    v.extend(placement.instances_of(owner).into_iter().map(|i| format!("the owner {i}")));
                    v
                }
                _ => Vec::new(),
            };
            let owner_projection = resolution
                .owner()
                .and_then(|o| walk.facts.get(o).and_then(|of| of.projection));

            sites.push(OwnedSite {
                child_ty: child.clone(),
                enclosing_locus: enclosing.clone(),
                resolution,
                owner_kind,
                edge_class,
                instances,
                owner_projection,
                span: site.span,
                enclosing_decl: site.enclosing_decl,
                member: site.member.clone(),
                child_decl,
                child_key,
                params_default: site.params_default,
                bare_statement: site.bare_statement,
                supplied: site.supplied.clone(),
                params_field: site.params_field.clone(),
                arg_default: site.arg_default.clone(),
            });
        }
    }

    let free_site = |site: &RawSite| FreeFnSite {
        child_ty: site.child_ty.clone(), span: site.span,
        child_decl: site.child_decl, child_key: site.child_key.clone(),
        supplied: site.supplied.clone(), bare_statement: site.bare_statement,
        arg_default: site.arg_default.clone(),
    };
    let free_fn_sites = walk.free_fn_sites.iter().map(free_site).collect();
    // A position no locus declaration holds (a free fn's argument
    // default, a top-level const or type) is marked `usize::MAX` by the
    // walk, and a position lowered at every use has no locus either.
    let decl_of = |site: &RawSite| (site.enclosing_decl != usize::MAX).then_some(site.enclosing_decl);
    let calls = walk.calls.iter().map(|site| {
        let per_use = match site.other {
            Some(OtherPosition::PerUse(p)) => Some(p),
            _ => None,
        };
        CallSite {
            id: site.id.0,
            span: site.span,
            enclosing_decl: decl_of(site).filter(|_| per_use.is_none()),
            params_field: site.params_field.clone(),
            arg_default: site.arg_default.clone(),
            per_use,
        }
    }).collect();
    let other_sites = walk.other.iter().filter_map(|site| Some(OtherSite {
        site: free_site(site),
        enclosing_decl: decl_of(site),
        position: site.other?,
    })).collect();
    OwnershipGraph {
        sites,
        births,
        free_fn_sites,
        calls,
        other_sites,
        declarations: walk.declarations.clone(),
        accepts,
        instantiated_by,
        accept_rows: walk.accept_rows.clone(),
        rows: walk,
    }
}

/// Resolve the owner of `child` instantiated inside `enclosing`.
///
/// 1. If `enclosing` itself accepts `child` → `SelfOwned` (today's
///    direct-parent case, innermost-most possible).
/// 2. Else climb the `instantiated_by` closure. The owner on a path is
///    the FIRST (nearest, innermost-wins) ancestor that accepts
///    `child`. Collect the distinct owner types + whether any path
///    reaches a root with no acceptor. Cycle-safe via a visited set.
fn resolve_owner(
    enclosing: &str,
    child: &str,
    accepts: &BTreeMap<String, BTreeSet<String>>,
    instantiated_by: &BTreeMap<String, BTreeSet<String>>,
) -> OwnerResolution {
    if accepts
        .get(enclosing)
        .map(|a| a.contains(child))
        .unwrap_or(false)
    {
        return OwnerResolution::SelfOwned(enclosing.to_string());
    }

    let mut owners: BTreeSet<String> = BTreeSet::new();
    let mut orphan = false;
    let mut visited: BTreeSet<String> = BTreeSet::new();
    visited.insert(enclosing.to_string());
    climb(
        enclosing,
        child,
        accepts,
        instantiated_by,
        &mut visited,
        &mut owners,
        &mut orphan,
    );

    // Any orphan path dominates: the site is unowned on at least one
    // instantiation path.
    if orphan {
        return OwnerResolution::Orphan;
    }
    match owners.len() {
        0 => OwnerResolution::Orphan, // no acceptor reachable anywhere
        1 => OwnerResolution::Ancestor(owners.into_iter().next().unwrap()),
        _ => OwnerResolution::PerPath(owners.into_iter().collect()),
    }
}

/// DFS up the `instantiated_by` closure from `node`, accumulating the
/// nearest accepting ancestor per branch (into `owners`) and whether
/// any branch dead-ends at a root without an acceptor (`orphan`).
/// `visited` prevents infinite recursion on instantiation cycles.
#[allow(clippy::too_many_arguments)]
fn climb(
    node: &str,
    child: &str,
    accepts: &BTreeMap<String, BTreeSet<String>>,
    instantiated_by: &BTreeMap<String, BTreeSet<String>>,
    visited: &mut BTreeSet<String>,
    owners: &mut BTreeSet<String>,
    orphan: &mut bool,
) {
    let Some(parents) = instantiated_by.get(node) else {
        // No locus instantiates `node` → it is a root (a `main locus`,
        // a locus born only at `fn main`, or uninstantiated). No
        // acceptor above → this path is orphan.
        *orphan = true;
        return;
    };
    // Parents not already on the current path (cycle guard).
    let fresh: Vec<&String> =
        parents.iter().filter(|p| !visited.contains(*p)).collect();
    if fresh.is_empty() {
        // Every parent is a cycle back onto the current path — no NEW
        // acceptor is reachable up this branch. Conservatively an
        // orphan path (no owner found before the cycle closed).
        *orphan = true;
        return;
    }
    for p in fresh {
        if accepts.get(p).map(|a| a.contains(child)).unwrap_or(false) {
            // Nearest acceptor on this branch — innermost wins, stop.
            owners.insert(p.clone());
        } else {
            visited.insert(p.clone());
            climb(p, child, accepts, instantiated_by, visited, owners, orphan);
            visited.remove(p);
        }
    }
}

/// Best-effort owner-instance classification.
fn classify_owner_kind(
    resolution: &OwnerResolution,
    singletons: &BTreeSet<String>,
) -> OwnerKind {
    match resolution {
        OwnerResolution::SelfOwned(_) => OwnerKind::DirectParent,
        OwnerResolution::Ancestor(o) => {
            if singletons.contains(o) {
                OwnerKind::SingletonConst
            } else {
                OwnerKind::Ancestor
            }
        }
        OwnerResolution::PerPath(os) => {
            if !os.is_empty() && os.iter().all(|o| singletons.contains(o)) {
                OwnerKind::SingletonConst
            } else {
                OwnerKind::Ancestor
            }
        }
        // No single owner — kind is not meaningful; label conservatively.
        OwnerResolution::Orphan | OwnerResolution::Unanalyzable(_) => {
            OwnerKind::Ancestor
        }
    }
}

/// Classify the edge by comparing where the enclosing locus's instances
/// run with where the owner's do, both read from the placement table
/// (F.40 phase 3, P1, rows O-1 to O-7). Per instance, never per type: an
/// enclosing locus nested under a field placed off main runs on that
/// field's thread (O-1, O-2), an adapter on its own (O-7), and a locus
/// born in a method body where its enclosing scope runs. Conservative:
/// an unresolved owner yields `Open`, and an instance the table cannot
/// place is never taken for the owner's thread.
fn classify_edge(
    enclosing: &str,
    resolution: &OwnerResolution,
    placement: &crate::placement::PlacementTable,
    domains: &BTreeMap<String, crate::placement::TypeDomains>,
) -> EdgeClass {
    match resolution {
        // Owner == enclosing: always the same thread.
        OwnerResolution::SelfOwned(_) => EdgeClass::SameTower,
        OwnerResolution::Ancestor(o) => relate(enclosing, o, placement, domains),
        OwnerResolution::PerPath(os) => {
            let classes: Vec<EdgeClass> = os.iter().map(|o| relate(enclosing, o, placement, domains)).collect();
            if classes.iter().all(|c| *c == EdgeClass::SameTower) {
                EdgeClass::SameTower
            } else if classes.iter().all(|c| *c == EdgeClass::CrossPool) {
                EdgeClass::CrossPool
            } else {
                EdgeClass::Mixed
            }
        }
        // No single owner to compare against.
        OwnerResolution::Orphan | OwnerResolution::Unanalyzable(_) => {
            EdgeClass::Open
        }
    }
}

/// One enclosing locus against one owner, per instance: each row of the
/// enclosing locus is paired with the row that owns it, the nearest row
/// above it in its template realizing the owner, and each instance the
/// table has no such row for (a literal in a body, a row below no owner
/// row) with every domain the owner runs in. `SameTower` when every pair
/// shares its domain (or nothing is built), `CrossPool` when none does
/// and the owner runs in one known domain, `Mixed` otherwise (U-1), and
/// whenever an instance runs where the table cannot say.
fn relate(
    enclosing: &str,
    owner: &str,
    placement: &crate::placement::PlacementTable,
    domains: &BTreeMap<String, crate::placement::TypeDomains>,
) -> EdgeClass {
    use crate::placement::{DomainId, InstanceKey};
    let of = |t: &str| domains.get(t).cloned().unwrap_or_default();
    let (e, o) = (of(enclosing), of(owner));
    if e.unknown || o.unknown {
        return EdgeClass::Mixed;
    }
    let realizes = |k: &InstanceKey, t: &str| {
        placement.instances.get(k).and_then(|r| r.realizes.as_ref()).is_some_and(|d| d.lowered == t)
    };
    let handed_off = placement.handed_off();
    let mut pairs: Vec<(DomainId, DomainId)> = Vec::new();
    for (k, r) in &placement.instances {
        if handed_off.contains(k) || !realizes(k, enclosing) {
            continue;
        }
        let mut up = r.owner.clone();
        let mut found = None;
        while let Some(key) = up {
            if realizes(&key, owner) {
                found = placement.instances.get(&key).map(|a| a.domain);
                break;
            }
            up = placement.instances.get(&key).and_then(|a| a.owner.clone());
        }
        match found {
            Some(a) => pairs.push((r.domain, a)),
            None => pairs.extend(o.known.iter().map(|a| (r.domain, *a))),
        }
    }
    for s in &placement.dynamic {
        if s.realizes.as_ref().is_some_and(|d| d.lowered == enclosing) {
            for d in &s.domains {
                pairs.extend(o.known.iter().map(|a| (*d, *a)));
            }
        }
    }
    if pairs.iter().all(|(d, a)| d == a) {
        return EdgeClass::SameTower;
    }
    match o.known.iter().collect::<Vec<_>>().as_slice() {
        [a] if pairs.iter().all(|(d, _)| d != *a) => EdgeClass::CrossPool,
        _ => EdgeClass::Mixed,
    }
}

// === Instantiation-literal walk ===================================

/// Collect every literal candidate in fn defaults and bodies. The shared
/// resolver later retains the locus births; the walk itself makes no
/// name-based classification. What an argument's default holds is marked
/// with the argument ([`ArgDefault`]): it is lowered at the calls that
/// leave the argument out, not here.
fn collect_sites_fn(f: &FnDecl, out: &mut Vec<RawSite>) {
    for (index, param) in f.params.iter().enumerate() {
        if let Some(value) = &param.default {
            let at = out.len();
            collect_sites_expr(value, out);
            declare_site(out, at, value, Some(&param.ty));
            let arg = ArgDefault {
                callee: f.id.0,
                index,
                fn_name: f.name.name.clone(),
                param: param.name.name.clone(),
            };
            for site in &mut out[at..] {
                site.arg_default = Some(arg.clone());
            }
        }
    }
    collect_sites_block(&f.body, out);
}

fn collect_sites_block(
    b: &Block,
    out: &mut Vec<RawSite>,
) {
    for s in &b.stmts {
        collect_sites_stmt(s, out);
    }
    if let Some(tail) = &b.tail {
        collect_sites_expr(tail, out);
    }
}

fn collect_sites_stmt(
    s: &Stmt,
    out: &mut Vec<RawSite>,
) {
    match s {
        Stmt::Let { value, ty, .. } => {
            let at = out.len();
            collect_sites_expr(value, out);
            declare_site(out, at, value, ty.as_ref());
        }
        Stmt::LetTuple { value, .. } => {
            collect_sites_expr(value, out)
        }
        Stmt::Assign { target, value, .. } => {
            for seg in &target.tail {
                if let LValueSeg::Index(e) = seg {
                    collect_sites_expr(e, out);
                }
            }
            collect_sites_expr(value, out);
        }
        Stmt::If(ifstmt) => collect_sites_if(ifstmt, out),
        Stmt::Match(m) => collect_sites_match(m, out),
        Stmt::For { iter, body, .. } => {
            collect_sites_expr(iter, out);
            collect_sites_block(body, out);
        }
        Stmt::While { cond, body, .. } => {
            collect_sites_expr(cond, out);
            collect_sites_block(body, out);
        }
        Stmt::Return(opt, _) => {
            if let Some(e) = opt {
                collect_sites_expr(e, out);
            }
        }
        Stmt::Fail { value, .. } => {
            collect_sites_expr(value, out)
        }
        Stmt::Recovery { args, .. } => {
            for a in args {
                collect_sites_expr(a, out);
            }
        }
        Stmt::Violate { payload, .. } => {
            if let Some(e) = payload {
                collect_sites_expr(e, out);
            }
        }
        Stmt::Send { subject, value, .. } => {
            collect_sites_expr(subject, out);
            collect_sites_expr(value, out);
        }
        Stmt::ShmWrite { max, body, .. } => {
            collect_sites_expr(max, out);
            collect_sites_block(body, out);
        }
        Stmt::Block(b) => collect_sites_block(b, out),
        Stmt::Expr(e) => {
            let at = out.len();
            collect_sites_expr(e, out);
            if matches!(e, Expr::Struct { .. }) {
                out[at].bare_statement = true;
            }
        }
        // No sub-expressions to walk.
        Stmt::Reperspective { .. }
        | Stmt::Yield(_)
        | Stmt::Break(_)
        | Stmt::Continue(_)
        | Stmt::Terminate(_) => {}
    }
}

fn collect_sites_if(
    ifstmt: &IfStmt,
    out: &mut Vec<RawSite>,
) {
    collect_sites_expr(&ifstmt.cond, out);
    collect_sites_block(&ifstmt.then_block, out);
    match ifstmt.else_block.as_deref() {
        None => {}
        Some(ElseBranch::Else(b)) => collect_sites_block(b, out),
        Some(ElseBranch::ElseIf(inner)) => {
            collect_sites_if(inner, out)
        }
    }
}

fn collect_sites_match(
    m: &MatchStmt,
    out: &mut Vec<RawSite>,
) {
    collect_sites_expr(&m.scrutinee, out);
    for arm in &m.arms {
        if let Some(g) = &arm.guard {
            collect_sites_expr(g, out);
        }
        match &arm.body {
            MatchArmBody::Expr(e) => collect_sites_expr(e, out),
            MatchArmBody::Block(b) => collect_sites_block(b, out),
        }
    }
}

fn collect_sites_expr(
    e: &Expr,
    out: &mut Vec<RawSite>,
) {
    match e {
        Expr::Struct { path, inits, span, id, .. } => {
            out.push(RawSite {
                id: *id,
                supplied: inits.iter().map(|i| i.name.name.clone()).collect(),
                params_field: None,
                binding: None,
                child_ty: String::new(),
                child_decl: None,
                child_key: None,
                span: *span,
                path: path.segments.iter().map(|s| s.name.clone()).collect(),
                declared: None,
                enclosing_decl: 0,
                member: None,
                params_default: false,
                bare_statement: false,
                arg_default: None,
                call: false,
                other: None,
            });
            for init in inits {
                collect_sites_expr(&init.value, out);
            }
        }
        Expr::Binary { left, right, .. } => {
            collect_sites_expr(left, out);
            collect_sites_expr(right, out);
        }
        Expr::Unary { operand, .. } => {
            collect_sites_expr(operand, out)
        }
        Expr::Call { callee, args, span, id } => {
            // A call's position, for default expansion: the defaults it
            // leaves are lowered where it is (`CallSite`).
            out.push(RawSite {
                id: *id,
                supplied: BTreeSet::new(),
                params_field: None,
                binding: None,
                child_ty: String::new(),
                child_decl: None,
                child_key: None,
                span: *span,
                path: Vec::new(),
                declared: None,
                enclosing_decl: 0,
                member: None,
                params_default: false,
                bare_statement: false,
                arg_default: None,
                call: true,
                other: None,
            });
            collect_sites_expr(callee, out);
            for a in args {
                collect_sites_expr(a, out);
            }
        }
        Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. } => {
            collect_sites_expr(receiver, out)
        }
        Expr::Index { receiver, index, .. } => {
            collect_sites_expr(receiver, out);
            collect_sites_expr(index, out);
        }
        Expr::Tuple(items, _) | Expr::Array(items, _) => {
            for it in items {
                collect_sites_expr(it, out);
            }
        }
        Expr::Block(b) => collect_sites_block(b, out),
        Expr::If(ifstmt) => collect_sites_if(ifstmt, out),
        Expr::Match(m) => collect_sites_match(m, out),
        Expr::Sum(inner, _) | Expr::Prod(inner, _) => {
            collect_sites_expr(inner, out)
        }
        Expr::Approx {
            left,
            right,
            tolerance,
            ..
        } => {
            collect_sites_expr(left, out);
            collect_sites_expr(right, out);
            collect_sites_expr(tolerance, out);
        }
        Expr::Range { lo, hi, .. } => {
            collect_sites_expr(lo, out);
            collect_sites_expr(hi, out);
        }
        Expr::ArrayRepeat { val, .. } => {
            collect_sites_expr(val, out)
        }
        Expr::Or {
            inner, disposition, ..
        } => {
            collect_sites_expr(inner, out);
            match disposition {
                OrDisposition::Substitute(e) | OrDisposition::Fail(e, _) => {
                    collect_sites_expr(e, out)
                }
                OrDisposition::Raise(_)
                | OrDisposition::Discard(_)
                | OrDisposition::Wait(_) => {}
            }
        }
        // Leaves — no sub-expressions.
        Expr::Literal(_, _)
        | Expr::Ident(_)
        | Expr::Path(_)
        | Expr::KwSelf(_) => {}
    }
}

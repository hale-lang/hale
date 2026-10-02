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

use crate::bus_graph::Placement;
use crate::handler_routing::{child_locus_name, ChildRef, DeclaredNames};
use crate::placement::{Enclosing, HoleAt, HoleKind, Origin, PlacementTable, SiteRef, SiteUniverse};
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
/// locus. `SameTower` — same OS thread (both same-thread, or
/// identically placed); `CrossPool` — different thread placement (a
/// pinned / non-`main` cooperative pool on one side); `Open` — not a
/// closed world, or no single owner to compare, so no tower relation
/// can be asserted. Conservative: an unresolved owner is `Open`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EdgeClass {
    SameTower,
    CrossPool,
    Open,
}

/// One resolved instantiation site: `enclosing_locus` gives birth to
/// `child_ty` in one of its method bodies, and the pass has resolved
/// which ancestor owns it.
#[derive(Debug, Clone)]
pub struct OwnedSite {
    /// The locus type being instantiated (`I`).
    pub child_ty: String,
    /// The locus whose method body writes `I { ... }` (`B`).
    pub enclosing_locus: String,
    /// The resolved owner + how it was found.
    pub resolution: OwnerResolution,
    /// Best-effort classification of the owner instance.
    pub owner_kind: OwnerKind,
    /// Same-thread vs cross-pool vs open, per the owner's placement.
    pub edge_class: EdgeClass,
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
    /// [`OwnershipGraph::declarations`]: the first of its name in
    /// declaration order, a qualified path resolved as an `accept`'s
    /// type is. `None` for a path that names no declaration here.
    pub child_decl: Option<usize>,
    /// The literal's child in the terms of [`OwnershipGraph::accepts`]:
    /// the locus `child_locus_name` resolves it to, a generic template
    /// specialized by the binding's or field's declared type. `None`
    /// where the graph cannot say which locus is born: a qualified path
    /// that resolves to none, a generic template no declared type
    /// specializes. `child_ty` and `resolution` stay keyed by the
    /// literal's last segment, as lowering and the model read them.
    pub child_key: Option<String>,
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
        if self.accepts.get(enclosing).is_some_and(|a| a.contains(key)) {
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
            forwarding,
        }
    }
}

/// The bubble plans lowering reads, projected from the graph by
/// [`OwnershipGraph::bubble_plans`]. The three plans key on
/// `(enclosing locus, child type)` and carry the owner locus type `A`;
/// they are DISJOINT (a site has one edge class and one owner kind).
/// Every other resolution (SelfOwned direct-parent, non-singleton
/// cross-pool, per-path, orphan, open) is in none of them and stays
/// transient. `BubblePlans::default()` is the empty plan: no bubble,
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
        self.graph.accepts.get(&self.graph.declarations[decl].name).is_some_and(|a| a.contains(self.child))
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
#[derive(Default)]
struct LocusFacts {
    /// Child types this locus declares `accept(_: T)` for, resolved by
    /// `child_locus_name` (a type that names no locus is accepted by
    /// no one).
    accepts: BTreeSet<String>,
    /// Locus-typed literals born in this locus's method bodies.
    instantiates: Vec<RawSite>,
    /// Projection class from a `: projection …` annotation, if any.
    projection: Option<ProjectionClass>,
    /// `main locus` / `@export locus` — a provably-unique instance.
    singleton: bool,
}

/// A single instantiation literal captured during the walk.
struct RawSite {
    child_ty: String,
    span: Span,
    /// The literal's path as written.
    path: Vec<String>,
    /// The type the literal's binding or field declares, when the
    /// literal is a `let`'s or a params default's whole value.
    declared: Option<TypeExpr>,
    /// See [`OwnedSite::enclosing_decl`] and [`OwnedSite::member`].
    enclosing_decl: usize,
    member: Option<String>,
}

/// The product of one walk: per-locus facts + the whole-bundle set of
/// locus type names (so a literal can be told apart from a plain
/// struct literal) + the closed-world entry-point flag.
struct OwnershipWalk {
    facts: BTreeMap<String, LocusFacts>,
    declarations: Vec<LocusDeclRow>,
    declared: DeclaredNames,
    has_entry_point: bool,
    accept_rows: AcceptRows,
}

/// Walk every locus once, collecting accepts + method-body
/// instantiations + projection + singleton-ness, plus the set of all
/// locus type names and the closed-world entry-point flag. This is the
/// single source of truth `build_ownership_graph` consumes.
fn collect_ownership_walk(bundle: &Bundle<'_>) -> OwnershipWalk {
    // Pass 1: gather every locus type name, so pass 2 can tell a
    // locus-instantiation literal apart from a plain struct literal.
    let mut locus_types: BTreeSet<String> = BTreeSet::new();
    fn names(items: &[TopDecl], out: &mut BTreeSet<String>) {
        for item in items {
            match item {
                TopDecl::Locus(l) => {
                    out.insert(l.name.name.clone());
                }
                TopDecl::Module(m) => names(&m.items, out),
                _ => {}
            }
        }
    }
    for program in bundle.programs.values() {
        names(&program.items, &mut locus_types);
    }

    // Closed-world gate, mirroring `build_bus_graph`'s
    // `has_entry_point`: a bare top-level `fn main` OR a `main locus`
    // makes the ownership DAG complete (no dynamic attach construct).
    let has_entry_point = bundle.programs.values().any(|p| {
        p.items.iter().any(|i| {
            matches!(i, TopDecl::Locus(l) if l.is_main)
                || matches!(i, TopDecl::Fn(f) if f.name.name == "main")
        })
    });

    // The child an `accept` names is resolved by the one resolver the
    // handler rows use, so an alias, generic arguments or a `std::` path
    // name the locus lowering resolves (F.40 phase 1.4).
    let programs: Vec<&Program> = bundle.programs.values().copied().collect();
    let declared = DeclaredNames::of(&programs);
    let renames = bundle.import_renames.as_slice();

    // Pass 2: per-locus facts.
    let mut facts: BTreeMap<String, LocusFacts> = BTreeMap::new();
    let mut declarations: Vec<LocusDeclRow> = Vec::new();
    let mut accept_rows: Vec<AcceptRow> = Vec::new();
    struct WalkCx<'a> {
        locus_types: &'a BTreeSet<String>,
        declared: &'a DeclaredNames,
        renames: &'a [(Vec<String>, String)],
        snapshot: &'a crate::snapshot::Snapshot,
    }
    fn walk(
        items: &[TopDecl],
        cx: &WalkCx<'_>,
        facts: &mut BTreeMap<String, LocusFacts>,
        declarations: &mut Vec<LocusDeclRow>,
        accept_rows: &mut Vec<AcceptRow>,
    ) {
        let (locus_types, declared, renames) = (cx.locus_types, cx.declared, cx.renames);
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
                                    locus_types,
                                    &mut entry.instantiates,
                                );
                            }
                            LocusMember::Fn(fd) => {
                                let from = entry.instantiates.len();
                                collect_sites_block(
                                    &fd.body,
                                    locus_types,
                                    &mut entry.instantiates,
                                );
                                for s in &mut entry.instantiates[from..] {
                                    s.member = Some(fd.name.name.clone());
                                }
                            }
                            LocusMember::Mode(md) => collect_sites_block(
                                &md.body,
                                locus_types,
                                &mut entry.instantiates,
                            ),
                            LocusMember::Failure(fl) => collect_sites_block(
                                &fl.body,
                                locus_types,
                                &mut entry.instantiates,
                            ),
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
                                            locus_types,
                                            &mut entry.instantiates,
                                        );
                                        declare_site(
                                            &mut entry.instantiates,
                                            at,
                                            e,
                                            p.ty.as_ref(),
                                        );
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
                TopDecl::Module(m) => walk(&m.items, cx, facts, declarations, accept_rows),
                _ => {}
            }
        }
    }
    let cx = WalkCx { locus_types: &locus_types, declared: &declared, renames, snapshot: &bundle.snapshot };
    for program in &programs {
        walk(&program.items, &cx, &mut facts, &mut declarations, &mut accept_rows);
    }

    OwnershipWalk {
        facts,
        declarations,
        declared: declared.clone(),
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
    let (Some(te), Expr::Struct { span, .. }) = (declared, value) else {
        return;
    };
    if let Some(site) = out.get_mut(at).filter(|s| s.span == *span) {
        site.declared = Some(te.clone());
    }
}

/// [`OwnedSite::child_decl`] and [`OwnedSite::child_key`] for a raw
/// site: the declaration the literal names and the child an `accept`
/// would have to name to own it.
fn identify_child(
    site: &RawSite,
    declarations: &[LocusDeclRow],
    declared: &DeclaredNames,
    renames: &[(Vec<String>, String)],
) -> (Option<usize>, Option<String>) {
    let first = |name: &str| declarations.iter().position(|d| d.name == name);
    if site.path.len() > 1 {
        // A qualified literal names what the same path names in an
        // `accept`: an import or a `std::` path, resolved.
        let te = TypeExpr::Named {
            path: QualifiedName {
                segments: site.path.iter().map(|s| Ident::new(s.clone(), site.span)).collect(),
                span: site.span,
            },
            generic_args: Vec::new(),
            span: site.span,
        };
        return match child_locus_name(&te, declared, renames) {
            ChildRef::Locus(name) => (first(&name), Some(name)),
            ChildRef::External(_) => (None, None),
        };
    }
    let decl = first(&site.child_ty);
    let generic = decl.is_some_and(|d| declarations[d].generic);
    if !generic {
        return (decl, Some(site.child_ty.clone()));
    }
    // A template is born as the specialization its binding or field
    // declares; without one the graph cannot say which.
    let key = site.declared.as_ref().and_then(|te| match child_locus_name(te, declared, renames) {
        ChildRef::Locus(name) => Some(name),
        ChildRef::External(_) => None,
    });
    (decl, key)
}

// === Build ========================================================

/// Build the authoritative [`OwnershipGraph`] for a bundle.
///
/// `_top` is accepted for API symmetry with
/// [`crate::bus_graph::build_bus_graph`] (both are the post-typecheck
/// analysis passes over a bundle); ownership resolution needs no
/// resolved-type scope, so it is unused today. Structural twin of
/// `build_bus_graph`: one shared walk ([`collect_ownership_walk`]),
/// then per-site owner resolution + edge classification.
pub fn build_ownership_graph(
    bundle: &Bundle<'_>,
    _top: &TopScope,
) -> OwnershipGraph {
    let walk = collect_ownership_walk(bundle);
    let placement_of = collect_placements(bundle);

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
            let (child_decl, child_key) = identify_child(
                site,
                &walk.declarations,
                &walk.declared,
                &bundle.import_renames,
            );

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
                    owner_projection: None,
                    span: site.span,
                    enclosing_decl: site.enclosing_decl,
                    member: site.member.clone(),
                    child_decl,
                    child_key,
                });
                continue;
            }

            let resolution =
                resolve_owner(enclosing, child, &accepts, &instantiated_by);
            let owner_kind = classify_owner_kind(&resolution, &singletons);
            let edge_class =
                classify_edge(enclosing, &resolution, &placement_of);
            let owner_projection = resolution
                .owner()
                .and_then(|o| walk.facts.get(o).and_then(|of| of.projection));

            sites.push(OwnedSite {
                child_ty: child.clone(),
                enclosing_locus: enclosing.clone(),
                resolution,
                owner_kind,
                edge_class,
                owner_projection,
                span: site.span,
                enclosing_decl: site.enclosing_decl,
                member: site.member.clone(),
                child_decl,
                child_key,
            });
        }
    }

    OwnershipGraph {
        sites,
        declarations: walk.declarations,
        accepts,
        instantiated_by,
        accept_rows: walk.accept_rows,
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

/// Classify the edge by comparing the owner's placement to the
/// enclosing locus's placement. Conservative: an unresolved owner
/// yields `Open`; an unknown placement defaults to `SameThread`
/// (matching how an unplaced locus runs on the owner's thread).
fn classify_edge(
    enclosing: &str,
    resolution: &OwnerResolution,
    placement_of: &BTreeMap<String, Placement>,
) -> EdgeClass {
    let pe = placement_of
        .get(enclosing)
        .cloned()
        .unwrap_or(Placement::SameThread);
    match resolution {
        // Owner == enclosing: always the same thread.
        OwnerResolution::SelfOwned(_) => EdgeClass::SameTower,
        OwnerResolution::Ancestor(o) => {
            let po =
                placement_of.get(o).cloned().unwrap_or(Placement::SameThread);
            if po == pe {
                EdgeClass::SameTower
            } else {
                EdgeClass::CrossPool
            }
        }
        OwnerResolution::PerPath(os) => {
            let all_same = os.iter().all(|o| {
                placement_of
                    .get(o)
                    .cloned()
                    .unwrap_or(Placement::SameThread)
                    == pe
            });
            if all_same {
                EdgeClass::SameTower
            } else {
                EdgeClass::CrossPool
            }
        }
        // No single owner to compare against.
        OwnerResolution::Orphan | OwnerResolution::Unanalyzable(_) => {
            EdgeClass::Open
        }
    }
}

// === Placement (the bus graph's former per-type labels) ============

/// Map each locus *type* to the [`Placement`] it receives where placed
/// as a `main locus` field: a `placement { }` entry keys on the owner's
/// `params` field name, and that field's declared type names the placed
/// child locus. First-placement-wins. The bus graph's copy of this walk
/// was replaced by the placement table (F.40 phase 3, P1 3 of 6).
pub(crate) fn collect_placements(bundle: &Bundle<'_>) -> BTreeMap<String, Placement> {
    let mut out: BTreeMap<String, Placement> = BTreeMap::new();

    fn walk(items: &[TopDecl], out: &mut BTreeMap<String, Placement>) {
        for item in items {
            match item {
                TopDecl::Locus(l) => {
                    let mut field_ty: BTreeMap<String, String> =
                        BTreeMap::new();
                    for member in &l.members {
                        if let LocusMember::Params(pb) = member {
                            for p in &pb.params {
                                if let Some(ty) = &p.ty {
                                    if let Some(name) = named_type(ty) {
                                        field_ty
                                            .insert(p.name.name.clone(), name);
                                    }
                                }
                            }
                        }
                    }
                    for member in &l.members {
                        if let LocusMember::Placement(pb) = member {
                            for e in &pb.entries {
                                let Some(child_ty) = field_ty.get(&e.field.name)
                                else {
                                    continue;
                                };
                                let placement = match &e.spec {
                                    PlacementSpec::Cooperative { pool, .. } => {
                                        match pool {
                                            Some(p) if p.name != "main" => {
                                                Placement::CrossPool(
                                                    p.name.clone(),
                                                )
                                            }
                                            _ => Placement::SameThread,
                                        }
                                    }
                                    PlacementSpec::Pinned { .. } => {
                                        Placement::Pinned
                                    }
                                };
                                out.entry(child_ty.clone())
                                    .or_insert(placement);
                            }
                        }
                    }
                }
                TopDecl::Module(m) => walk(&m.items, out),
                _ => {}
            }
        }
    }
    for program in bundle.programs.values() {
        walk(&program.items, &mut out);
    }
    out
}

// === Instantiation-literal walk ===================================

/// The single named type a param's `TypeExpr` denotes (a bare `Named`
/// path's last segment), else `None`.
fn named_type(ty: &TypeExpr) -> Option<String> {
    match ty {
        TypeExpr::Named { path, .. } => {
            path.segments.last().map(|s| s.name.clone())
        }
        _ => None,
    }
}

/// GH #476 Change 8: locus births in FREE functions — `fn main() {
/// EchoL { }; }` and friends.
///
/// The ownership graph deliberately walks locus MEMBER bodies only:
/// its question is "which owning locus does this child bubble to",
/// and a free function has no owner to bubble toward. The model's
/// arrangement asks a different question — "is this instance in the
/// static arrangement, or does it appear at runtime?" — and a
/// free-function birth is emphatically the latter. Without this
/// walk, a whole program whose loci are all born in `fn main` would
/// model zero instances while claiming exact placement.
///
/// Returns `(locus type, literal span)` per site, in source order.
pub fn free_fn_birth_sites(
    bundle: &Bundle<'_>,
) -> Vec<(String, Span)> {
    let mut locus_types: BTreeSet<String> = BTreeSet::new();
    fn names(items: &[TopDecl], out: &mut BTreeSet<String>) {
        for item in items {
            match item {
                TopDecl::Locus(l) => {
                    out.insert(l.name.name.clone());
                }
                TopDecl::Module(m) => names(&m.items, out),
                _ => {}
            }
        }
    }
    for program in bundle.programs.values() {
        names(&program.items, &mut locus_types);
    }
    let mut out: Vec<RawSite> = Vec::new();
    fn walk(
        items: &[TopDecl],
        locus_types: &BTreeSet<String>,
        out: &mut Vec<RawSite>,
    ) {
        for item in items {
            match item {
                TopDecl::Fn(f) => {
                    collect_sites_block(&f.body, locus_types, out)
                }
                TopDecl::Module(m) => walk(&m.items, locus_types, out),
                _ => {}
            }
        }
    }
    for program in bundle.programs.values() {
        walk(&program.items, &locus_types, &mut out);
    }
    out.into_iter().map(|s| (s.child_ty, s.span)).collect()
}

/// Collect every locus-instantiation literal (`I { ... }` where `I` is
/// a known locus type) reachable from a block, walking every
/// sub-statement and sub-expression.
fn collect_sites_block(
    b: &Block,
    locus_types: &BTreeSet<String>,
    out: &mut Vec<RawSite>,
) {
    for s in &b.stmts {
        collect_sites_stmt(s, locus_types, out);
    }
    if let Some(tail) = &b.tail {
        collect_sites_expr(tail, locus_types, out);
    }
}

fn collect_sites_stmt(
    s: &Stmt,
    locus_types: &BTreeSet<String>,
    out: &mut Vec<RawSite>,
) {
    match s {
        Stmt::Let { value, ty, .. } => {
            let at = out.len();
            collect_sites_expr(value, locus_types, out);
            declare_site(out, at, value, ty.as_ref());
        }
        Stmt::LetTuple { value, .. } => {
            collect_sites_expr(value, locus_types, out)
        }
        Stmt::Assign { target, value, .. } => {
            for seg in &target.tail {
                if let LValueSeg::Index(e) = seg {
                    collect_sites_expr(e, locus_types, out);
                }
            }
            collect_sites_expr(value, locus_types, out);
        }
        Stmt::If(ifstmt) => collect_sites_if(ifstmt, locus_types, out),
        Stmt::Match(m) => collect_sites_match(m, locus_types, out),
        Stmt::For { iter, body, .. } => {
            collect_sites_expr(iter, locus_types, out);
            collect_sites_block(body, locus_types, out);
        }
        Stmt::While { cond, body, .. } => {
            collect_sites_expr(cond, locus_types, out);
            collect_sites_block(body, locus_types, out);
        }
        Stmt::Return(opt, _) => {
            if let Some(e) = opt {
                collect_sites_expr(e, locus_types, out);
            }
        }
        Stmt::Fail { value, .. } => {
            collect_sites_expr(value, locus_types, out)
        }
        Stmt::Recovery { args, .. } => {
            for a in args {
                collect_sites_expr(a, locus_types, out);
            }
        }
        Stmt::Violate { payload, .. } => {
            if let Some(e) = payload {
                collect_sites_expr(e, locus_types, out);
            }
        }
        Stmt::Send { subject, value, .. } => {
            collect_sites_expr(subject, locus_types, out);
            collect_sites_expr(value, locus_types, out);
        }
        Stmt::ShmWrite { max, body, .. } => {
            collect_sites_expr(max, locus_types, out);
            collect_sites_block(body, locus_types, out);
        }
        Stmt::Block(b) => collect_sites_block(b, locus_types, out),
        Stmt::Expr(e) => collect_sites_expr(e, locus_types, out),
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
    locus_types: &BTreeSet<String>,
    out: &mut Vec<RawSite>,
) {
    collect_sites_expr(&ifstmt.cond, locus_types, out);
    collect_sites_block(&ifstmt.then_block, locus_types, out);
    match ifstmt.else_block.as_deref() {
        None => {}
        Some(ElseBranch::Else(b)) => collect_sites_block(b, locus_types, out),
        Some(ElseBranch::ElseIf(inner)) => {
            collect_sites_if(inner, locus_types, out)
        }
    }
}

fn collect_sites_match(
    m: &MatchStmt,
    locus_types: &BTreeSet<String>,
    out: &mut Vec<RawSite>,
) {
    collect_sites_expr(&m.scrutinee, locus_types, out);
    for arm in &m.arms {
        if let Some(g) = &arm.guard {
            collect_sites_expr(g, locus_types, out);
        }
        match &arm.body {
            MatchArmBody::Expr(e) => collect_sites_expr(e, locus_types, out),
            MatchArmBody::Block(b) => collect_sites_block(b, locus_types, out),
        }
    }
}

fn collect_sites_expr(
    e: &Expr,
    locus_types: &BTreeSet<String>,
    out: &mut Vec<RawSite>,
) {
    match e {
        Expr::Struct { path, inits, span, .. } => {
            if let Some(name) = path.segments.last().map(|s| &s.name) {
                if locus_types.contains(name) {
                    out.push(RawSite {
                        child_ty: name.clone(),
                        span: *span,
                        path: path.segments.iter().map(|s| s.name.clone()).collect(),
                        declared: None,
                        enclosing_decl: 0,
                        member: None,
                    });
                }
            }
            for init in inits {
                collect_sites_expr(&init.value, locus_types, out);
            }
        }
        Expr::Binary { left, right, .. } => {
            collect_sites_expr(left, locus_types, out);
            collect_sites_expr(right, locus_types, out);
        }
        Expr::Unary { operand, .. } => {
            collect_sites_expr(operand, locus_types, out)
        }
        Expr::Call { callee, args, .. } => {
            collect_sites_expr(callee, locus_types, out);
            for a in args {
                collect_sites_expr(a, locus_types, out);
            }
        }
        Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. } => {
            collect_sites_expr(receiver, locus_types, out)
        }
        Expr::Index { receiver, index, .. } => {
            collect_sites_expr(receiver, locus_types, out);
            collect_sites_expr(index, locus_types, out);
        }
        Expr::Tuple(items, _) | Expr::Array(items, _) => {
            for it in items {
                collect_sites_expr(it, locus_types, out);
            }
        }
        Expr::Block(b) => collect_sites_block(b, locus_types, out),
        Expr::If(ifstmt) => collect_sites_if(ifstmt, locus_types, out),
        Expr::Match(m) => collect_sites_match(m, locus_types, out),
        Expr::Sum(inner, _) | Expr::Prod(inner, _) => {
            collect_sites_expr(inner, locus_types, out)
        }
        Expr::Approx {
            left,
            right,
            tolerance,
            ..
        } => {
            collect_sites_expr(left, locus_types, out);
            collect_sites_expr(right, locus_types, out);
            collect_sites_expr(tolerance, locus_types, out);
        }
        Expr::Range { lo, hi, .. } => {
            collect_sites_expr(lo, locus_types, out);
            collect_sites_expr(hi, locus_types, out);
        }
        Expr::ArrayRepeat { val, .. } => {
            collect_sites_expr(val, locus_types, out)
        }
        Expr::Or {
            inner, disposition, ..
        } => {
            collect_sites_expr(inner, locus_types, out);
            match disposition {
                OrDisposition::Substitute(e) | OrDisposition::Fail(e, _) => {
                    collect_sites_expr(e, locus_types, out)
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

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

use hale_syntax::ast::*;
use hale_syntax::Span;

use crate::handler_routing::{child_locus_name, ChildRef, DeclaredNames};
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
}

/// The whole-bundle ownership graph.
#[derive(Debug, Clone, Default)]
pub struct OwnershipGraph {
    /// Every resolved instantiation site, in walk order.
    pub sites: Vec<OwnedSite>,
    /// locus type → the child types it declares `accept(_: T)` for, each
    /// the locus `child_locus_name` resolves it to: an alias followed,
    /// generic arguments mangled, a `std::` or cross-seed path renamed.
    pub accepts: BTreeMap<String, BTreeSet<String>>,
    /// child locus type → the set of locus types that instantiate it
    /// in a method body (the ancestor-edge relation).
    pub instantiated_by: BTreeMap<String, BTreeSet<String>>,
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
}

/// The product of one walk: per-locus facts + the whole-bundle set of
/// locus type names (so a literal can be told apart from a plain
/// struct literal) + the closed-world entry-point flag.
struct OwnershipWalk {
    facts: BTreeMap<String, LocusFacts>,
    has_entry_point: bool,
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
    fn walk(
        items: &[TopDecl],
        locus_types: &BTreeSet<String>,
        declared: &DeclaredNames,
        renames: &[(Vec<String>, String)],
        facts: &mut BTreeMap<String, LocusFacts>,
    ) {
        for item in items {
            match item {
                TopDecl::Locus(l) => {
                    let entry = facts.entry(l.name.name.clone()).or_default();
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
                            LocusMember::Fn(fd) => collect_sites_block(
                                &fd.body,
                                locus_types,
                                &mut entry.instantiates,
                            ),
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
                                        collect_sites_expr(
                                            e,
                                            locus_types,
                                            &mut entry.instantiates,
                                        );
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                TopDecl::Module(m) => {
                    walk(&m.items, locus_types, declared, renames, facts)
                }
                _ => {}
            }
        }
    }
    for program in &programs {
        walk(&program.items, &locus_types, &declared, renames, &mut facts);
    }

    OwnershipWalk {
        facts,
        has_entry_point,
    }
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
) -> OwnershipGraph {
    let walk = collect_ownership_walk(bundle);
    let domains = placement.domains_by_type();

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
            });
        }
    }

    OwnershipGraph {
        sites,
        accepts,
        instantiated_by,
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
        Stmt::Let { value, .. } | Stmt::LetTuple { value, .. } => {
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

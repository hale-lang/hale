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

use crate::bus_graph::Placement;
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
}

/// The whole-bundle ownership graph.
#[derive(Debug, Clone, Default)]
pub struct OwnershipGraph {
    /// Every resolved instantiation site, in walk order.
    pub sites: Vec<OwnedSite>,
    /// locus type → the child types it declares `accept(_: T)` for.
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
}

// === Shared walk ==================================================

/// One locus's ownership-relevant facts, collected in a single walk.
#[derive(Default)]
struct LocusFacts {
    /// Child types this locus declares `accept(_: T)` for.
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

    // Pass 2: per-locus facts.
    let mut facts: BTreeMap<String, LocusFacts> = BTreeMap::new();
    fn walk(
        items: &[TopDecl],
        locus_types: &BTreeSet<String>,
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
                                        if let Some(name) = named_type(&p.ty) {
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
                            _ => {}
                        }
                    }
                }
                TopDecl::Module(m) => walk(&m.items, locus_types, facts),
                _ => {}
            }
        }
    }
    for program in bundle.programs.values() {
        walk(&program.items, &locus_types, &mut facts);
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

// === Placement (mirrors bus_graph::collect_subscriber_placements) ==

/// Map each locus *type* to the [`Placement`] it receives where placed
/// as a `main locus` field. Same shape as
/// `bus_graph::collect_subscriber_placements`: a `placement { }` entry
/// keys on the owner's `params` field name, and that field's declared
/// type names the placed child locus. First-placement-wins.
fn collect_placements(bundle: &Bundle<'_>) -> BTreeMap<String, Placement> {
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
                                    PlacementSpec::Cooperative { pool } => {
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
        Stmt::Yield(_)
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
        Expr::Struct { path, inits, span } => {
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
                OrDisposition::Raise(_) | OrDisposition::Discard(_) => {}
            }
        }
        // Leaves — no sub-expressions.
        Expr::Literal(_, _)
        | Expr::Ident(_)
        | Expr::Path(_)
        | Expr::KwSelf(_) => {}
    }
}

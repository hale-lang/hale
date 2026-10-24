//! Source instance contexts, before lifecycle obligations or emission plans.
//!
//! Placement supplies templates, domains and bounds. Original declarations
//! supply construction position, accepted-child membership and dynamic fields.
//! This table owns no AST and contains no plan rows or runtime instance IDs.

pub mod model;

use std::collections::{BTreeMap, BTreeSet};
use hale_model::LifecycleStage;
pub use hale_model::construction::{LiteralPosition, Construction, Contribution};
use hale_syntax::ast::{Block, ElseBranch, Expr, IfStmt, LifecycleKind, LocusDecl, LocusMember, MatchArmBody, ParamInit, Stmt, TopDecl, TypeExpr};
use crate::flow_instances::FlowInstances;
use crate::ownership_graph::OwnershipGraph;
use crate::placement::{Bound, Decision, DeclRef, DomainId, DomainKind, Enclosing, HoleAt, HoleKind, InstanceKey, LocusIndex, Origin, PlacementTable, SiteRef, SiteUniverse, Template};
use crate::snapshot::Snapshot;
use crate::symbol::Bundle;

/// Declaration specialization and construction template, shared with the plan.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceSite {
    pub decl: DeclRef,
    pub template: Template,
}

/// Source authorities required to derive an instance's contexts.
pub struct LifecycleContextInputs<'a> {
    pub bundle: &'a Bundle<'a>,
    pub placement: &'a PlacementTable,
    pub flows: &'a FlowInstances,
    pub ownership: &'a OwnershipGraph,
}

/// One row per source instance template, in construction order. Owner and
/// parent-context indices address rows of this same table.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LifecycleContexts {
    pub instances: Vec<LifecycleContext>,
}

/// Derive once from source families; consumers do not reconstruct these facts
/// from lifecycle plans or rendered topology artifacts.
pub fn derive_lifecycle_contexts(inputs: &LifecycleContextInputs<'_>) -> LifecycleContexts {
    let index = LocusIndex::of(inputs.bundle);
    let literals = literal_positions(inputs.bundle);
    LifecycleContexts { instances: contexts(inputs, &index, &literals) }
}

// ------------------------------------------------------- the literals

/// Where a locus literal is written: its statement position and the
/// body that holds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LiteralAt {
    built: LiteralPosition,
    member: Member,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Member {
    /// The top-level user `fn main`.
    FnMain,
    Fn,
    /// An `on_failure` body: runs only when the handler does.
    Handler,
    /// A lifecycle body with a row of its locus's own (`birth`, `run`,
    /// `drain`, `dissolve`): the row whose end follows each statement.
    Lifecycle(LifecycleStage),
    Other,
}

/// Every locus literal of both universes, by site.
fn literal_positions(bundle: &Bundle<'_>) -> BTreeMap<SiteRef, LiteralAt> {
    let mut w = LiteralWalk { out: BTreeMap::new(), ids: &bundle.snapshot, universe: SiteUniverse::User, member: Member::Other };
    for program in bundle.programs.values() {
        w.items(&program.items, true);
    }
    if let (Some(p), Some(ids)) = (crate::stdlib_bodies::program(), crate::stdlib_bodies::identities()) {
        w.ids = ids;
        w.universe = SiteUniverse::StdlibAnalysis;
        w.items(&p.items, false);
    }
    w.out
}

struct LiteralWalk<'s> {
    out: BTreeMap<SiteRef, LiteralAt>,
    ids: &'s Snapshot,
    universe: SiteUniverse,
    member: Member,
}

impl LiteralWalk<'_> {
    fn items(&mut self, items: &[TopDecl], top: bool) {
        for item in items {
            match item {
                TopDecl::Fn(f) => {
                    let main = top && self.universe == SiteUniverse::User && f.name.name == "main";
                    self.member = if main { Member::FnMain } else { Member::Fn };
                    self.block(&f.body);
                }
                TopDecl::Locus(l) => {
                    for m in &l.members {
                        match m {
                            LocusMember::Params(pb) => {
                                self.member = Member::Other;
                                for p in &pb.params {
                                    if let ParamInit::Value(e) = &p.init {
                                        self.expr(e, LiteralPosition::Nested);
                                    }
                                }
                            }
                            LocusMember::Lifecycle(d) => {
                                self.member = match d.kind {
                                    LifecycleKind::Birth => Member::Lifecycle(LifecycleStage::Birth),
                                    LifecycleKind::Run => Member::Lifecycle(LifecycleStage::Run),
                                    LifecycleKind::Drain => Member::Lifecycle(LifecycleStage::Drain),
                                    LifecycleKind::Dissolve => Member::Lifecycle(LifecycleStage::Dissolve),
                                    LifecycleKind::Accept | LifecycleKind::Release => Member::Other,
                                };
                                self.block(&d.body);
                            }
                            LocusMember::Fn(f) => {
                                self.member = Member::Other;
                                self.block(&f.body);
                            }
                            LocusMember::Failure(f) => {
                                self.member = Member::Handler;
                                self.block(&f.body);
                            }
                            LocusMember::Mode(md) => {
                                self.member = Member::Other;
                                self.block(&md.body);
                            }
                            _ => {}
                        }
                    }
                }
                TopDecl::Module(m) => self.items(&m.items, false),
                _ => {}
            }
        }
    }

    fn block(&mut self, b: &Block) {
        for s in &b.stmts {
            self.stmt(s);
        }
        if let Some(t) = &b.tail {
            self.expr(t, LiteralPosition::Nested);
        }
    }

    fn if_chain(&mut self, i: &IfStmt) {
        self.expr(&i.cond, LiteralPosition::Nested);
        self.block(&i.then_block);
        match i.else_block.as_deref() {
            Some(ElseBranch::Else(b)) => self.block(b),
            Some(ElseBranch::ElseIf(n)) => self.if_chain(n),
            None => {}
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Let { value, .. } => self.expr(value, LiteralPosition::Let),
            Stmt::LetTuple { value, .. } => self.expr(value, LiteralPosition::Nested),
            Stmt::Assign { value, .. } => self.expr(value, LiteralPosition::Nested),
            Stmt::Expr(e) => self.expr(e, LiteralPosition::Statement),
            Stmt::If(i) => self.if_chain(i),
            Stmt::Match(m) => {
                self.expr(&m.scrutinee, LiteralPosition::Nested);
                for arm in &m.arms {
                    match &arm.body {
                        MatchArmBody::Expr(e) => self.expr(e, LiteralPosition::Statement),
                        MatchArmBody::Block(b) => self.block(b),
                    }
                }
            }
            Stmt::For { iter, body, .. } => {
                self.expr(iter, LiteralPosition::Nested);
                self.block(body);
            }
            Stmt::While { cond, body, .. } => {
                self.expr(cond, LiteralPosition::Nested);
                self.block(body);
            }
            Stmt::Return(Some(e), _) | Stmt::Fail { value: e, .. } => self.expr(e, LiteralPosition::Nested),
            Stmt::Block(b) => self.block(b),
            Stmt::Send { value, .. } => self.expr(value, LiteralPosition::Nested),
            _ => {}
        }
    }

    /// `e`, written at `built`: a literal records it; anything inside
    /// it is nested.
    fn expr(&mut self, e: &Expr, built: LiteralPosition) {
        match e {
            Expr::Struct { inits, id, .. } => {
                if let Some(site) = self.ids.site_id(*id) {
                    self.out.insert(SiteRef { universe: self.universe, id: site }, LiteralAt { built, member: self.member });
                }
                for i in inits {
                    self.expr(&i.value, LiteralPosition::Nested);
                }
            }
            Expr::Binary { left, right, .. } => {
                self.expr(left, LiteralPosition::Nested);
                self.expr(right, LiteralPosition::Nested);
            }
            Expr::Unary { operand, .. } => self.expr(operand, LiteralPosition::Nested),
            Expr::Call { callee, args, .. } => {
                self.expr(callee, LiteralPosition::Nested);
                for a in args {
                    self.expr(a, LiteralPosition::Nested);
                }
            }
            Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. } => self.expr(receiver, LiteralPosition::Nested),
            Expr::Index { receiver, index, .. } => {
                self.expr(receiver, LiteralPosition::Nested);
                self.expr(index, LiteralPosition::Nested);
            }
            Expr::Tuple(parts, _) | Expr::Array(parts, _) => {
                for p in parts {
                    self.expr(p, LiteralPosition::Nested);
                }
            }
            Expr::Block(b) => self.block(b),
            Expr::If(i) => self.if_chain(i),
            Expr::Match(m) => {
                self.expr(&m.scrutinee, LiteralPosition::Nested);
                for arm in &m.arms {
                    match &arm.body {
                        MatchArmBody::Expr(e) => self.expr(e, LiteralPosition::Nested),
                        MatchArmBody::Block(b) => self.block(b),
                    }
                }
            }
            Expr::Or { inner, .. } => self.expr(inner, LiteralPosition::Nested),
            _ => {}
        }
    }
}

// ------------------------------------------------------- the instances

/// One instance template: the declaration it realizes, where it sits,
/// and every parent context it is reached through.
#[derive(Debug, Clone, PartialEq)]
pub struct LifecycleContext {
    pub site: SourceSite,
    pub universe: SiteUniverse,
    pub how: Construction,
    /// Every parent context, in construction order: one for a static
    /// row, an adapter, a template's top and a literal in a free fn; one
    /// per contribution of each owner for a dynamic literal's field,
    /// which is one template however many constructions of its owner's
    /// declaration reach it, and for a body or accepted literal, whose
    /// owners are every template of its enclosing locus. Nothing about
    /// an occurrence is read from one contribution alone.
    pub contributions: Vec<Contribution>,
    /// Unresolved ownership coverage; never interpreted as an ownerless object.
    pub owner_gaps: Vec<OwnerGap>,
    /// How many occurrences: the table's for a static row and a literal
    /// (it covers every scope the literal is written in), its owners'
    /// summed for a field.
    pub bound: Bound,
    /// Built in an `on_failure` body under every contribution: it exists
    /// only on a path where the handler runs.
    pub in_handler: bool,
    /// Built by a statement of its enclosing locus's lifecycle body: the
    /// kind of that body's row, which ends after the statement's
    /// teardown (C13).
    pub statement_of: Option<LifecycleStage>,
    /// A field's name in its owner's params: where it falls in the
    /// owner's declaration order, the order the cascade walks fields in.
    pub field: Option<String>,
    /// Its domain was decided for it (a root entry, a binding), not
    /// inherited from its owner.
    pub placed: bool,
}

/// A source ownership edge that cannot supply a concrete parent context.
/// These are coverage gaps, not evidence that the object has no runtime owner.
#[derive(Debug, Clone, PartialEq)]
pub enum OwnerGap {
    /// This candidate edge closes a recursive construction/ownership chain.
    Recursive { candidate: usize },
    /// No represented template realizes the enclosing source declaration.
    UnrepresentedEnclosing { declaration: DeclRef },
    /// Placement names an owner excluded from this table's source horizon.
    MissingPlacedOwner { key: InstanceKey },
}

/// Pinned and pool anchors initialize their params on their own domain
/// (C49/C50). Their literal and, for a pool anchor, their own birth still
/// run on the instantiating thread.
pub(crate) fn initialization_domain(t: &PlacementTable, placed: bool, c: &Contribution) -> Option<DomainId> {
    if placed && c.own.is_some_and(|d| matches!(t.domains[d.0 as usize].kind, DomainKind::Pinned { .. } | DomainKind::Pool { .. })) {
        c.own
    } else {
        c.it
    }
}

/// Where a template's contributions come from.
enum Source<'t> {
    /// A static row or a literal in a free fn: the one the table gives.
    Fixed,
    /// A body or accepted literal: one per contribution of each template
    /// of its enclosing locus, on the domain that occurrence runs (one of
    /// the table's `domains` for the scope).
    Enclosed { decl: DeclRef, domains: &'t BTreeSet<DomainId>, in_handler: bool },
    /// A dynamic literal's field: one per contribution of each template
    /// its literal is a default of.
    Field { parents: Vec<usize> },
}

/// Whether `to` is `from` or an owner of it, through any owner.
fn reaches(owners: &[Vec<usize>], from: usize, to: usize) -> bool {
    let mut seen = BTreeSet::new();
    let mut stack = vec![from];
    while let Some(o) = stack.pop() {
        if o == to {
            return true;
        }
        if seen.insert(o) {
            stack.extend(owners[o].iter().copied());
        }
    }
    false
}

/// `c` at the end of `v`, where `v` does not hold it yet.
fn push_unique(v: &mut Vec<Contribution>, c: Contribution) {
    if !v.contains(&c) {
        v.push(c);
    }
}

/// The first parent contribution for each execution context. Consumers of
/// `under` read the parent's instantiating and queue domains, never its
/// ancestry. Fields also inherit whether construction is in a handler.
/// Retain those distinctions without expanding every path through shared
/// owners: a chain of two literals per level otherwise doubles its contexts
/// at every level, although all paths on one thread make the same claims.
fn parent_contexts(all: &[Contribution]) -> impl Iterator<Item = (usize, &Contribution)> {
    let mut seen = BTreeSet::new();
    all.iter().enumerate().filter(move |(_, c)| seen.insert((c.it, c.own, c.in_handler)))
}

/// A template's contributions from its owners' current ones; `cut` are
/// the owners whose edge closed a cycle, built under none of theirs.
fn derived_contributions(
    out: &[LifecycleContext],
    source: &Source<'_>,
    owners: &[usize],
    cut: &[usize],
) -> Vec<Contribution> {
    let mut v = Vec::new();
    match source {
        Source::Fixed => unreachable!("a fixed template keeps the table's contribution"),
        Source::Enclosed { domains, in_handler, .. } => {
            // One domain for the scope is every occurrence's; otherwise the
            // enclosing occurrence's own, where the table names it.
            let one = (domains.len() == 1).then(|| *domains.iter().next().expect("one"));
            let on = |d: Option<DomainId>| one.or(d.filter(|d| domains.contains(d)));
            for &o in owners {
                for (k, c) in parent_contexts(&out[o].contributions) {
                    let d = on(c.own);
                    v.push(Contribution { owner: Some(o), under: Some(k), it: d, own: d, in_handler: *in_handler });
                }
            }
            if owners.is_empty() || !cut.is_empty() {
                push_unique(&mut v, Contribution { owner: None, under: None, it: one, own: one, in_handler: *in_handler });
            }
        }
        // Built inline in the owner's params loop: on the owner's threads,
        // under that owner.
        Source::Field { .. } => {
            for &p in owners {
                for (k, c) in parent_contexts(&out[p].contributions) {
                    v.push(Contribution { owner: Some(p), under: Some(k), ..c.clone() });
                }
            }
            for &p in cut {
                for c in &out[p].contributions {
                    push_unique(&mut v, Contribution { owner: None, under: None, ..c.clone() });
                }
            }
        }
    }
    v
}

fn contexts<'a>(
    inputs: &LifecycleContextInputs<'a>,
    index: &LocusIndex<'a>,
    literals: &BTreeMap<SiteRef, LiteralAt>,
) -> Vec<LifecycleContext> {
    let t = inputs.placement;
    let held: BTreeSet<&InstanceKey> = t
        .holes
        .iter()
        .filter_map(|h| match (&h.at, &h.kind) {
            (HoleAt::Instance(k), HoleKind::Reuse { .. }) => Some(k),
            _ => None,
        })
        .collect();
    let main_locus = t.root.as_ref().map(|r| r.realizes.clone());
    let bound_of = |origin: &Origin| -> Bound {
        let lit = match origin {
            Origin::Construction(l) => *l,
            Origin::Entry(_) | Origin::Binding(_) => return Bound::Once,
        };
        t.root
            .iter()
            .flat_map(|r| &r.constructions)
            .chain(&t.entry_literals)
            .find(|c| c.literal == lit)
            .map(|c| c.bound.clone())
            .unwrap_or(Bound::Once)
    };
    let mut out: Vec<LifecycleContext> = Vec::new();
    let mut by_key: BTreeMap<&InstanceKey, usize> = BTreeMap::new();
    for (key, row) in &t.instances {
        if row.built_by.is_some() || held.contains(key) {
            continue;
        }
        let (Some(realizes), Some(_)) = (row.realizes.clone(), row.realizes.as_ref().and_then(|d| index.decl(d.site)))
        else {
            continue;
        };
        let how = if key.path.is_empty() {
            match key.origin {
                Origin::Binding(_) => Construction::Adapter,
                Origin::Entry(_) => {
                    Construction::Top { built: LiteralPosition::Statement, main_locus: main_locus.as_ref() == Some(&realizes), in_fn_main: true }
                }
                Origin::Construction(lit) => Construction::Top {
                    built: literals.get(&lit).map(|l| l.built).unwrap_or(LiteralPosition::Statement),
                    main_locus: main_locus.as_ref() == Some(&realizes),
                    in_fn_main: literals.get(&lit).is_none_or(|l| l.member == Member::FnMain),
                },
            }
        } else {
            Construction::Field
        };
        let owner = row.owner.as_ref().and_then(|k| by_key.get(k).copied());
        let owner_gaps = row.owner.as_ref().filter(|_| owner.is_none())
            .map(|key| vec![OwnerGap::MissingPlacedOwner { key: key.clone() }]).unwrap_or_default();
        let it = owner.map_or(Some(PlacementTable::MAIN), |o| {
            initialization_domain(t, out[o].placed, &out[o].contributions[0])
        });
        by_key.insert(key, out.len());
        let bound = bound_of(&key.origin);
        out.push(LifecycleContext {
            site: SourceSite { decl: realizes.clone(), template: Template::Static(key.clone()) },
            universe: realizes.site.universe,
            how,
            contributions: vec![Contribution {
                owner,
                under: None,
                it,
                own: Some(row.domain),
                in_handler: false,
            }],
            bound,
            owner_gaps,
            in_handler: false,
            statement_of: None,
            field: (how == Construction::Field).then(|| key.path.last().map(|s| s.field.clone())).flatten(),
            placed: matches!(row.decided_by, Decision::Entry { .. } | Decision::Binding { .. }),
        });
    }
    // Every dynamic template first: a body literal's owners are its
    // enclosing locus's templates, which may be dynamic sites listed after
    // it or their fields, so contributions are derived once all exist.
    let mut sources: Vec<Source<'a>> = out.iter().map(|_| Source::Fixed).collect();
    for d in &t.dynamic {
        let (Some(realizes), Some(_)) = (d.realizes.clone(), d.realizes.as_ref().and_then(|r| index.decl(r.site))) else {
            continue;
        };
        let at = literals.get(&d.literal).copied().unwrap_or(LiteralAt { built: LiteralPosition::Nested, member: Member::Other });
        let domain = (d.domains.len() == 1).then(|| *d.domains.iter().next().expect("one"));
        let in_handler = at.member == Member::Handler;
        let i = out.len();
        let how = match &d.enclosing {
            Enclosing::Locus(enclosing) => {
                sources.push(Source::Enclosed { decl: enclosing.clone(), domains: &d.domains, in_handler });
                let acceptor = index.decl(enclosing.site);
                // A concrete `accept` names the child's declaration; an
                // interface's admits it in the graph's `accepts`.
                let accepts = acceptor.is_some_and(|a| {
                    accept_types(a).any(|ty| index.names(ty, enclosing.site.universe) == Some(realizes.site))
                        || (accept_types(a).any(|ty| index.names(ty, enclosing.site.universe).is_none())
                            && inputs.ownership.accepts.get(&a.name.name).is_some_and(|c| c.contains(&realizes.lowered)))
                });
                if accepts {
                    Construction::Accepted { flow: inputs.flows.is_flow(&realizes.lowered) }
                } else {
                    Construction::Body { built: at.built, in_fn_main: false }
                }
            }
            Enclosing::Fn(_) => {
                sources.push(Source::Fixed);
                Construction::Body { built: at.built, in_fn_main: at.member == Member::FnMain }
            }
        };
        out.push(LifecycleContext {
            site: SourceSite { decl: realizes.clone(), template: Template::Dynamic { literal: d.literal } },
            universe: realizes.site.universe,
            how,
            contributions: vec![Contribution { owner: None, under: None, it: domain, own: domain, in_handler }],
            owner_gaps: Vec::new(),
            bound: d.bound.clone(),
            in_handler,
            statement_of: match (how, at.member) {
                (Construction::Body { built: LiteralPosition::Statement, .. }, Member::Lifecycle(k)) => Some(k),
                _ => None,
            },
            field: None,
            placed: false,
        });
        dynamic_fields(&mut out, &mut sources, i, index, &inputs.bundle.snapshot);
    }
    // Each template's owners: a static row's from the table, a body
    // literal's every template of its enclosing locus, in the order they
    // are listed, a field's every template it is a default of.
    let mut owners: Vec<Vec<usize>> = (0..out.len())
        .map(|i| match &sources[i] {
            Source::Fixed => out[i].contributions.iter().filter_map(|c| c.owner).collect(),
            Source::Enclosed { decl, .. } => (0..out.len()).filter(|&j| j != i && out[j].site.decl == *decl).collect(),
            Source::Field { parents } => parents.clone(),
        })
        .collect();
    for i in 0..out.len() {
        if let Source::Enclosed { decl, .. } = &sources[i] {
            if owners[i].is_empty() {
                out[i].owner_gaps.push(OwnerGap::UnrepresentedEnclosing { declaration: decl.clone() });
            }
        }
    }
    // A locus that builds itself (`B { }` in B's run, or two loci that
    // build each other) owns no instance of its own chain: the owner
    // closing a cycle of owners is cut, its occurrences built under none.
    let mut cut: Vec<Vec<usize>> = vec![Vec::new(); out.len()];
    for i in 0..out.len() {
        let mut k = 0;
        while k < owners[i].len() {
            if reaches(&owners, owners[i][k], i) {
                let candidate = owners[i].remove(k);
                out[i].owner_gaps.push(OwnerGap::Recursive { candidate });
                cut[i].push(candidate);
            } else {
                k += 1;
            }
        }
    }
    for i in 0..out.len() {
        if matches!(sources[i], Source::Fixed) {
            for c in &mut out[i].contributions {
                c.owner = c.owner.filter(|o| !cut[i].contains(o));
            }
        } else {
            out[i].contributions.clear();
        }
    }
    // Every contribution from its owners', to a fixpoint: with the cycles
    // cut the owners are acyclic, so each template's count is bounded.
    loop {
        let mut changed = false;
        for i in 0..out.len() {
            if matches!(sources[i], Source::Fixed) {
                continue;
            }
            let next = derived_contributions(&out, &sources[i], &owners[i], &cut[i]);
            if next != out[i].contributions {
                out[i].contributions = next;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    // A field's bound is its owners' summed (a cut owner's without its own
    // cycle); a template is built in a handler only where every
    // contribution is.
    fn bound_of_field(
        i: usize,
        out: &[LifecycleContext],
        sources: &[Source<'_>],
        owners: &[Vec<usize>],
        cut: Option<&[Vec<usize>]>,
    ) -> Bound {
        if !matches!(sources[i], Source::Field { .. }) {
            return out[i].bound.clone();
        }
        let from_owners = owners[i].iter().map(|&o| bound_of_field(o, out, sources, owners, cut));
        let from_cut = cut.into_iter().flat_map(|c| &c[i]).map(|&o| bound_of_field(o, out, sources, owners, None));
        from_owners.chain(from_cut).reduce(|a, b| add_bounds(&a, &b)).unwrap_or(Bound::Once)
    }
    let bounds: Vec<Bound> = (0..out.len()).map(|i| bound_of_field(i, &out, &sources, &owners, Some(&cut))).collect();
    for (s, bound) in out.iter_mut().zip(bounds) {
        s.bound = bound;
        s.in_handler = !s.contributions.is_empty() && s.contributions.iter().all(|c| c.in_handler);
    }
    out
}

/// The locus fields a dynamic literal's declaration builds from its
/// defaults, each one template of its field literal however many
/// templates it is a default of, recursively; a field literal already
/// a template is not re-entered, which a cycle of defaults ends at.
fn dynamic_fields<'a>(
    out: &mut Vec<LifecycleContext>,
    sources: &mut Vec<Source<'a>>,
    parent: usize,
    index: &LocusIndex<'a>,
    ids: &Snapshot,
) {
    let universe = out[parent].universe;
    if universe != SiteUniverse::User {
        return;
    }
    let Some(decl) = index.decl(out[parent].site.decl.site) else { return };
    for m in &decl.members {
        let LocusMember::Params(pb) = m else { continue };
        for p in &pb.params {
            let ParamInit::Value(Expr::Struct { path: lit_path, id, .. }) = &p.init else { continue };
            let ty = TypeExpr::Named { path: lit_path.clone(), generic_args: Vec::new(), span: lit_path.span };
            let Some(site) = index.names(&ty, universe) else { continue };
            let Some(field_decl) = index.decl(site) else { continue };
            let Some(literal) = ids.site_id(*id).map(SiteRef::user) else { continue };
            let realizes = DeclRef { site, args: Vec::new(), lowered: field_decl.name.name.clone(), template: field_decl.name.name.clone() };
            let field = p.name.name.clone();
            // Every instance of a literal shares its rows: a field literal
            // reached under several of its declaration's templates is one
            // template, with each as a parent.
            let template = Template::Dynamic { literal };
            match out.iter().position(|s| s.site.template == template) {
                Some(j) => {
                    if let Source::Field { parents } = &mut sources[j] {
                        if !parents.contains(&parent) {
                            parents.push(parent);
                        }
                    }
                }
                None => {
                    out.push(LifecycleContext {
                        field: Some(field),
                        placed: false,
                        site: SourceSite { decl: realizes, template },
                        universe: site.universe,
                        how: Construction::Field,
                        contributions: Vec::new(),
                        owner_gaps: Vec::new(),
                        bound: Bound::Once,
                        in_handler: false,
                        statement_of: None,
                    });
                    sources.push(Source::Field { parents: vec![parent] });
                    dynamic_fields(out, sources, out.len() - 1, index, ids);
                }
            }
        }
    }
}

/// How many occurrences two sources of one template give it together.
fn add_bounds(a: &Bound, b: &Bound) -> Bound {
    let n = |x: &Bound| match x {
        Bound::Once => Some(1u32),
        Bound::AtMost(k) => Some(*k),
        Bound::Unbounded(_) => None,
    };
    match (n(a), n(b)) {
        (Some(x), Some(y)) => x.checked_add(y).map(Bound::AtMost)
            .unwrap_or_else(|| Bound::Unbounded("source construction count exceeds u32".to_string())),
        _ => match (a, b) {
            (Bound::Unbounded(why), _) | (_, Bound::Unbounded(why)) => Bound::Unbounded(why.clone()),
            _ => unreachable!("a bound without a count is unbounded"),
        },
    }
}

fn accept_types(decl: &LocusDecl) -> impl Iterator<Item = &TypeExpr> {
    decl.members.iter().filter_map(|m| match m {
        LocusMember::Lifecycle(d) if d.kind == LifecycleKind::Accept => d.params.first().map(|p| &p.ty),
        _ => None,
    })
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn construction_count_overflow_is_not_a_finite_bound() {
        assert_eq!(add_bounds(&Bound::AtMost(u32::MAX - 1), &Bound::Once), Bound::AtMost(u32::MAX));
        assert!(matches!(add_bounds(&Bound::AtMost(u32::MAX), &Bound::Once), Bound::Unbounded(_)));
        assert!(matches!(add_bounds(&Bound::AtMost(u32::MAX), &Bound::AtMost(u32::MAX)), Bound::Unbounded(_)));
        assert_eq!(add_bounds(&Bound::Once, &Bound::Once), Bound::AtMost(2));
    }

    #[test]
    fn an_unbounded_source_remains_unbounded_when_summed() {
        let unbounded = Bound::Unbounded("recursive source".into());
        assert_eq!(add_bounds(&unbounded, &Bound::Once), unbounded);
        assert_eq!(add_bounds(&Bound::Once, &unbounded), unbounded);
    }
}

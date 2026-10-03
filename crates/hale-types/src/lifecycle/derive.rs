//! The `lifecycle_order` family's producer (F.40 phase 3, L1): one plan
//! of obligations per snapshot, `Snapshot::demand_lifecycle`, counted as
//! `lifecycle_order`.
//!
//! [`derive_lifecycle`] reads four families and the declarations they
//! name, and derives nothing those families already answer:
//!
//! - the **placement table** (P1): every instance template, static
//!   (an [`InstanceKey`] of the root's, the entry's, an adapter's tower,
//!   replicas apart) or dynamic (a literal outside the tower, with the
//!   domains its scope runs in and its bound), and the domains. A held
//!   row and the rows projected under it are their source's instances
//!   and owe nothing of their own; a hole names no declaration and gets
//!   no rows. A dynamic literal's own locus fields are not rows of the
//!   table (P1 enumerates no subtree it cannot key), so they are
//!   instances here, each its field literal's template, in its owner's
//!   domains. A field literal reached through several constructions of
//!   its owner's declaration is one template that keeps each one's
//!   contribution (owner, instantiating domain, own domain), a nested
//!   field the product of its owner's with its own placement, and its
//!   bound is their sum. A body or accepted literal keeps every template
//!   of its enclosing locus as an owner the same way, each occurrence on
//!   the domain its enclosing occurrence runs it on, with the table's
//!   bound, which covers every enclosing scope. A claim is one domain
//!   where the contributions agree and their set where they do not,
//!   never one owner's alone.
//! - the **handler rows**: which `on_failure` an instance's owner runs
//!   for it, and the recovery ops it can invoke (a restart).
//! - the **flow rows**: whether an accepted child is reclaimed at the
//!   end of its `run()` (a flow) or by its owner's cascade (a resident).
//! - the **bus graph**: which instances subscribe (registration,
//!   readiness, the teardown delivery contract).
//!
//! Each instance owes its rows in the order its domain performs them:
//! params settle (when its declaration brackets), accept, subscribe and
//! readiness, birth, the run's admission and the run, the closures, a
//! failure's rows per source it can raise, then drain, dissolve, the
//! pinned join and the reclaim. A failure's rows are guarded: each
//! source (a birth-epoch closure, the `birth_check`, a `violate` in
//! `run()` or in a handler or in `drain()`, a dissolve-epoch closure)
//! has its delivery on its own path, a held alternative where the
//! owner's params can still be open when it is raised, and, when the
//! owner's handler can restart, the recovery decision and the restart,
//! performed or refused under teardown. The process owes the spines'
//! own steps: the pre-drain, the wait-abort and the pool join of the
//! main locus's eager teardown and of `fn main`'s exit, and the signal
//! path's cooperative drain.
//!
//! A row's status is its decision line's ([`super::DECISION_LINES`]):
//! the line that makes it exist, the rule that places each edge and the
//! rule behind each domain claim each carry their own, so a row whose
//! existence is shipped can carry a claim that is known open (decision
//! L0-1, inventory C36) or pending (line 3).

use std::collections::{BTreeMap, BTreeSet};

use hale_syntax::ast::{
    Block, ElseBranch, Expr, IfStmt, LifecycleKind, LocusDecl, LocusMember, MatchArmBody, ParamInit, RecoveryOp,
    Stmt, TopDecl, TypeExpr, UnaryOp,
};

use super::{
    DomainRole, Edges, Epoch, Event, FailureSource, Holder, Instance, LifecyclePlan, Multiplicity, NotStarted, Obligation,
    ObligationId, ObligationKind as K, PathGuard, Point, Prerequisite, Progress, ProgressRule, Resource, Retention,
    Rule, RunsOn, ShutdownCause, SourceSite, Spine, Status, Terminal,
};
use crate::bus_graph::BusGraph;
use crate::flows::{is_flow, FlowRows};
use crate::handler_routing::HandlerRouting;
use crate::placement::{
    Bound, Decision, DeclRef, DomainId, DomainKind, Enclosing, HoleAt, HoleKind, InstanceKey, LocusIndex, Origin,
    PlacementTable, SiteRef, SiteUniverse, Template,
};
use crate::snapshot::Snapshot;
use crate::symbol::Bundle;

/// What the producer reads: the snapshot's placement table, handler
/// rows, flow rows and bus graph, over the bundle they were derived
/// from.
pub struct LifecycleInputs<'a> {
    pub bundle: &'a Bundle<'a>,
    pub placement: &'a PlacementTable,
    pub handlers: &'a HandlerRouting,
    pub flows: &'a FlowRows,
    pub bus: &'a BusGraph,
}

/// The `lifecycle_order` family's producer: every obligation of every
/// instance template the placement table names, and the process's.
pub fn derive_lifecycle(inputs: &LifecycleInputs<'_>) -> LifecyclePlan {
    let index = LocusIndex::of(inputs.bundle);
    let literals = literal_positions(inputs.bundle);
    let subjects = subjects(inputs, &index, &literals);
    let mut b = Builder {
        inputs,
        subjects: &subjects,
        facts: subjects.iter().map(|s| Facts::of(s.decl, s.universe, &index, inputs.bus)).collect(),
        plan: LifecyclePlan {
            obligations: Vec::new(),
            instances: subjects
                .iter()
                .map(|s| Instance { site: s.site.clone(), bound: s.bound.clone(), in_handler: s.in_handler })
                .collect(),
            domains: inputs.placement.domains.clone(),
        },
        rows: Vec::new(),
    };
    for i in 0..subjects.len() {
        let rows = b.instance(i);
        b.rows.push(rows);
    }
    for i in 0..subjects.len() {
        b.tree_edges(i);
    }
    b.process();
    b.plan
}

// ------------------------------------------------------- the literals

/// Where a locus literal is written: its statement position and the
/// body that holds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LiteralAt {
    built: Built,
    member: Member,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Built {
    /// `Kid { };` — torn down where the statement ends (the eager spine).
    Statement,
    /// `let k = Kid { };` — torn down at the scope's exit (a frame entry).
    Let,
    /// Inside another expression (a receiver, an argument, a field
    /// default): torn down with the frame, as a let-bound one is.
    Nested,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Member {
    /// The top-level user `fn main`.
    FnMain,
    Fn,
    /// An `on_failure` body: runs only when the handler does.
    Handler,
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
                                        self.expr(e, Built::Nested);
                                    }
                                }
                            }
                            LocusMember::Lifecycle(d) => {
                                self.member = Member::Other;
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
            self.expr(t, Built::Nested);
        }
    }

    fn if_chain(&mut self, i: &IfStmt) {
        self.expr(&i.cond, Built::Nested);
        self.block(&i.then_block);
        match i.else_block.as_deref() {
            Some(ElseBranch::Else(b)) => self.block(b),
            Some(ElseBranch::ElseIf(n)) => self.if_chain(n),
            None => {}
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Let { value, .. } => self.expr(value, Built::Let),
            Stmt::LetTuple { value, .. } => self.expr(value, Built::Nested),
            Stmt::Assign { value, .. } => self.expr(value, Built::Nested),
            Stmt::Expr(e) => self.expr(e, Built::Statement),
            Stmt::If(i) => self.if_chain(i),
            Stmt::Match(m) => {
                self.expr(&m.scrutinee, Built::Nested);
                for arm in &m.arms {
                    match &arm.body {
                        MatchArmBody::Expr(e) => self.expr(e, Built::Statement),
                        MatchArmBody::Block(b) => self.block(b),
                    }
                }
            }
            Stmt::For { iter, body, .. } => {
                self.expr(iter, Built::Nested);
                self.block(body);
            }
            Stmt::While { cond, body, .. } => {
                self.expr(cond, Built::Nested);
                self.block(body);
            }
            Stmt::Return(Some(e), _) | Stmt::Fail { value: e, .. } => self.expr(e, Built::Nested),
            Stmt::Block(b) => self.block(b),
            Stmt::Send { value, .. } => self.expr(value, Built::Nested),
            _ => {}
        }
    }

    /// `e`, written at `built`: a literal records it; anything inside
    /// it is nested.
    fn expr(&mut self, e: &Expr, built: Built) {
        match e {
            Expr::Struct { inits, id, .. } => {
                if let Some(site) = self.ids.site_id(*id) {
                    self.out.insert(SiteRef { universe: self.universe, id: site }, LiteralAt { built, member: self.member });
                }
                for i in inits {
                    self.expr(&i.value, Built::Nested);
                }
            }
            Expr::Binary { left, right, .. } => {
                self.expr(left, Built::Nested);
                self.expr(right, Built::Nested);
            }
            Expr::Unary { operand, .. } => self.expr(operand, Built::Nested),
            Expr::Call { callee, args, .. } => {
                self.expr(callee, Built::Nested);
                for a in args {
                    self.expr(a, Built::Nested);
                }
            }
            Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. } => self.expr(receiver, Built::Nested),
            Expr::Index { receiver, index, .. } => {
                self.expr(receiver, Built::Nested);
                self.expr(index, Built::Nested);
            }
            Expr::Tuple(parts, _) | Expr::Array(parts, _) => {
                for p in parts {
                    self.expr(p, Built::Nested);
                }
            }
            Expr::Block(b) => self.block(b),
            Expr::If(i) => self.if_chain(i),
            Expr::Match(m) => {
                self.expr(&m.scrutinee, Built::Nested);
                for arm in &m.arms {
                    match &arm.body {
                        MatchArmBody::Expr(e) => self.expr(e, Built::Nested),
                        MatchArmBody::Block(b) => self.block(b),
                    }
                }
            }
            Expr::Or { inner, .. } => self.expr(inner, Built::Nested),
            _ => {}
        }
    }
}

// ------------------------------------------------------- the instances

/// One instance template: the declaration it realizes, where it sits,
/// and every parent context it is reached through.
struct Subject<'a> {
    site: SourceSite,
    decl: &'a LocusDecl,
    universe: SiteUniverse,
    how: How,
    /// Every parent context, in construction order: one for a static
    /// row, an adapter, a template's top and a literal in a free fn; one
    /// per contribution of each owner for a dynamic literal's field,
    /// which is one template however many constructions of its owner's
    /// declaration reach it, and for a body or accepted literal, whose
    /// owners are every template of its enclosing locus. Nothing about
    /// an occurrence is read from one contribution alone.
    contributions: Vec<Contribution>,
    /// How many occurrences: the table's for a static row and a literal
    /// (it covers every scope the literal is written in), its owners'
    /// summed for a field.
    bound: Bound,
    /// Built in an `on_failure` body under every contribution: it exists
    /// only on a path where the handler runs.
    in_handler: bool,
    /// A field whose declared type is a contract (an interface, a
    /// perspective) the literal implements: the cascade tears it down
    /// through its recorded reclaim (inventory C32), after its owner's
    /// dissolve.
    contract: bool,
    /// Its domain was decided for it (a root entry, a binding), not
    /// inherited from its owner.
    placed: bool,
}

/// One parent context of a template: the owner it is built under there,
/// and the domains it is built and runs on.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Contribution {
    /// The instance whose `on_failure` it fails to and whose arena holds
    /// it: a field's owner, an accepted child's acceptor, a body
    /// literal's enclosing locus. `None` for a template's top, a literal
    /// in a free fn, and the occurrences of a cycle of owners.
    owner: Option<usize>,
    /// The owner's contribution this one is built under: for a field,
    /// its owner's context with the field's own placement; for a body or
    /// accepted literal, the enclosing occurrence that runs it. `None`
    /// pairs it with every contribution of the owner.
    under: Option<usize>,
    /// The instantiating thread: the domain running the code that holds
    /// the literal. `None` when the scope's domains are unknown or many.
    it: Option<DomainId>,
    /// The queue owner: where its `run()` runs and its cells land.
    own: Option<DomainId>,
    in_handler: bool,
}

/// Pinned and pool anchors initialize their params on their own domain
/// (C49/C50). Their literal and, for a pool anchor, their own birth still
/// run on the instantiating thread.
fn initialization_domain(t: &PlacementTable, placed: bool, c: &Contribution) -> Option<DomainId> {
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

/// A template's contributions from its owners' current ones; `cut` are
/// the owners whose edge closed a cycle, built under none of theirs.
fn derived_contributions(
    out: &[Subject<'_>],
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
                for (k, c) in out[o].contributions.iter().enumerate() {
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
                for (k, c) in out[p].contributions.iter().enumerate() {
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

/// One claim from every contribution's: the rule they share, over the
/// domains they name together. None where one of them names none (that
/// occurrence's domain is not known) or two state different rules.
fn combine(claims: impl IntoIterator<Item = Option<RunsOn>>) -> Option<RunsOn> {
    let mut out: Option<RunsOn> = None;
    for c in claims {
        let c = c?;
        match &mut out {
            None => out = Some(c),
            Some(o) if o.rule == c.rule => o.domains.extend(c.domains),
            Some(_) => return None,
        }
    }
    out
}

/// Whether `owner`'s field `field` is declared with a type that names no
/// locus (a contract the literal implements).
fn contract_field(owner: &LocusDecl, field: &str, universe: SiteUniverse, index: &LocusIndex<'_>) -> bool {
    owner
        .members
        .iter()
        .filter_map(|m| match m {
            LocusMember::Params(pb) => pb.params.iter().find(|p| p.name.name == field),
            _ => None,
        })
        .next()
        .and_then(|p| p.ty.as_ref())
        .is_some_and(|ty| index.names(ty, universe).is_none())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum How {
    /// A template's top: a root or entry literal in `fn main`, or the
    /// entry's implicit construction of the root.
    Top { built: Built, main_locus: bool },
    /// A params field of its owner.
    Field,
    /// An adapter of the root's `bindings { }`.
    Adapter,
    /// A literal its enclosing locus accepts.
    Accepted { flow: bool },
    /// A literal its scope tears down.
    Body { built: Built, in_fn_main: bool },
}

fn subjects<'a>(
    inputs: &LifecycleInputs<'a>,
    index: &LocusIndex<'a>,
    literals: &BTreeMap<SiteRef, LiteralAt>,
) -> Vec<Subject<'a>> {
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
    let mut out: Vec<Subject<'a>> = Vec::new();
    let mut by_key: BTreeMap<&InstanceKey, usize> = BTreeMap::new();
    for (key, row) in &t.instances {
        if row.built_by.is_some() || held.contains(key) {
            continue;
        }
        let (Some(realizes), Some(decl)) = (row.realizes.clone(), row.realizes.as_ref().and_then(|d| index.decl(d.site)))
        else {
            continue;
        };
        let how = if key.path.is_empty() {
            match key.origin {
                Origin::Binding(_) => How::Adapter,
                Origin::Entry(_) => How::Top { built: Built::Statement, main_locus: main_locus.as_ref() == Some(&realizes) },
                Origin::Construction(lit) => How::Top {
                    built: literals.get(&lit).map(|l| l.built).unwrap_or(Built::Statement),
                    main_locus: main_locus.as_ref() == Some(&realizes),
                },
            }
        } else {
            How::Field
        };
        let owner = row.owner.as_ref().and_then(|k| by_key.get(k).copied());
        let it = owner.map_or(Some(PlacementTable::MAIN), |o| {
            initialization_domain(t, out[o].placed, &out[o].contributions[0])
        });
        let contract = match (owner, key.path.last()) {
            (Some(o), Some(step)) => contract_field(out[o].decl, &step.field, out[o].universe, index),
            _ => false,
        };
        by_key.insert(key, out.len());
        let bound = bound_of(&key.origin);
        out.push(Subject {
            site: SourceSite { decl: realizes.clone(), template: Template::Static(key.clone()) },
            decl,
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
            in_handler: false,
            contract,
            placed: matches!(row.decided_by, Decision::Entry { .. } | Decision::Binding { .. }),
        });
    }
    // Every dynamic template first: a body literal's owners are its
    // enclosing locus's templates, which may be dynamic sites listed after
    // it or their fields, so contributions are derived once all exist.
    let mut sources: Vec<Source<'a>> = out.iter().map(|_| Source::Fixed).collect();
    for d in &t.dynamic {
        let (Some(realizes), Some(decl)) = (d.realizes.clone(), d.realizes.as_ref().and_then(|r| index.decl(r.site))) else {
            continue;
        };
        let at = literals.get(&d.literal).copied().unwrap_or(LiteralAt { built: Built::Nested, member: Member::Other });
        let domain = (d.domains.len() == 1).then(|| *d.domains.iter().next().expect("one"));
        let in_handler = at.member == Member::Handler;
        let i = out.len();
        let how = match &d.enclosing {
            Enclosing::Locus(enclosing) => {
                sources.push(Source::Enclosed { decl: enclosing.clone(), domains: &d.domains, in_handler });
                let acceptor = index.decl(enclosing.site);
                let accepts = acceptor.is_some_and(|a| {
                    accept_types(a).any(|ty| index.names(ty, enclosing.site.universe) == Some(realizes.site))
                });
                if accepts {
                    How::Accepted { flow: is_flow(inputs.flows, &realizes.lowered) }
                } else {
                    How::Body { built: at.built, in_fn_main: false }
                }
            }
            Enclosing::Fn(_) => {
                sources.push(Source::Fixed);
                How::Body { built: at.built, in_fn_main: at.member == Member::FnMain }
            }
        };
        out.push(Subject {
            site: SourceSite { decl: realizes.clone(), template: Template::Dynamic { literal: d.literal } },
            decl,
            universe: realizes.site.universe,
            how,
            contributions: vec![Contribution { owner: None, under: None, it: domain, own: domain, in_handler }],
            bound: d.bound.clone(),
            in_handler,
            contract: false,
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
    // A locus that builds itself (`B { }` in B's run, or two loci that
    // build each other) owns no instance of its own chain: the owner
    // closing a cycle of owners is cut, its occurrences built under none.
    let mut cut: Vec<Vec<usize>> = vec![Vec::new(); out.len()];
    for i in 0..out.len() {
        let mut k = 0;
        while k < owners[i].len() {
            if reaches(&owners, owners[i][k], i) {
                cut[i].push(owners[i].remove(k));
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
        out: &[Subject<'_>],
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
    out: &mut Vec<Subject<'a>>,
    sources: &mut Vec<Source<'a>>,
    parent: usize,
    index: &LocusIndex<'a>,
    ids: &Snapshot,
) {
    let universe = out[parent].universe;
    if universe != SiteUniverse::User {
        return;
    }
    let decl = out[parent].decl;
    for m in &decl.members {
        let LocusMember::Params(pb) = m else { continue };
        for p in &pb.params {
            let ParamInit::Value(Expr::Struct { path: lit_path, id, .. }) = &p.init else { continue };
            let ty = TypeExpr::Named { path: lit_path.clone(), generic_args: Vec::new(), span: lit_path.span };
            let Some(site) = index.names(&ty, universe) else { continue };
            let Some(field_decl) = index.decl(site) else { continue };
            let Some(literal) = ids.site_id(*id).map(SiteRef::user) else { continue };
            let realizes = DeclRef { site, args: Vec::new(), lowered: field_decl.name.name.clone() };
            let contract = p.ty.as_ref().is_some_and(|ty| index.names(ty, universe).is_none());
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
                    out.push(Subject {
                        contract,
                        placed: false,
                        site: SourceSite { decl: realizes, template },
                        decl: field_decl,
                        universe,
                        how: How::Field,
                        contributions: Vec::new(),
                        bound: Bound::Once,
                        in_handler: false,
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
        (Some(x), Some(y)) => Bound::AtMost(x.saturating_add(y)),
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

// ------------------------------------------------- what a declaration has

/// What a declaration's members make an instance owe.
struct Facts {
    /// A `run()` the author wrote (the empty one the desugar gives a
    /// locus that declares none runs nothing the trace sees).
    run: bool,
    /// The epochs of its closures, each once.
    closure_epochs: BTreeSet<Epoch>,
    birth_check: bool,
    /// Where its `violate` statements are.
    violates: BTreeSet<FailureSource>,
    subscribes: bool,
    /// It declares an `on_failure`.
    handles: bool,
    /// The params bracket: a handler, and a field holding an instance or
    /// a default that is computed (inventory C3).
    brackets: bool,
}

impl Facts {
    fn of(decl: &LocusDecl, universe: SiteUniverse, index: &LocusIndex<'_>, bus: &BusGraph) -> Facts {
        let mut f = Facts {
            run: false,
            closure_epochs: BTreeSet::new(),
            birth_check: false,
            violates: BTreeSet::new(),
            subscribes: bus.subjects.values().any(|s| s.subscribers.iter().any(|x| x.locus == decl.name.name)),
            handles: false,
            brackets: false,
        };
        let handlers: BTreeSet<&str> = decl
            .members
            .iter()
            .filter_map(|m| match m {
                LocusMember::Bus(b) => Some(b.members.iter().filter_map(|bm| match bm {
                    hale_syntax::ast::BusMember::Subscribe { handler, .. } => Some(handler.name.as_str()),
                    _ => None,
                })),
                _ => None,
            })
            .flatten()
            .collect();
        let mut holds = false;
        let mut computed = false;
        for m in &decl.members {
            match m {
                LocusMember::Params(pb) => {
                    for p in &pb.params {
                        if p.ty.as_ref().is_some_and(|t| !plain_value(t, universe, index)) {
                            holds = true;
                        }
                        match &p.init {
                            ParamInit::Value(Expr::Struct { .. }) => holds = true,
                            ParamInit::Value(e) if !constant(e) => computed = true,
                            _ => {}
                        }
                    }
                }
                LocusMember::Lifecycle(d) => {
                    let source = match d.kind {
                        LifecycleKind::Run => {
                            f.run |= !d.synthesized;
                            FailureSource::Run
                        }
                        LifecycleKind::Drain => FailureSource::Drain,
                        LifecycleKind::Dissolve => FailureSource::Dissolve,
                        LifecycleKind::Birth => FailureSource::BirthClosure,
                        LifecycleKind::Accept | LifecycleKind::Release => FailureSource::Run,
                    };
                    if violates(&d.body) {
                        f.violates.insert(source);
                    }
                }
                LocusMember::Fn(fd) => {
                    if violates(&fd.body) {
                        f.violates.insert(if handlers.contains(fd.name.name.as_str()) {
                            FailureSource::Handler
                        } else {
                            FailureSource::Run
                        });
                    }
                }
                LocusMember::Closure(c) => {
                    for clause in &c.clauses {
                        if let hale_syntax::ast::ClosureClause::Epoch(e) = clause {
                            f.closure_epochs.insert(match e {
                                hale_syntax::ast::EpochSpec::Tick => Epoch::Tick,
                                hale_syntax::ast::EpochSpec::Duration(_) => Epoch::Duration,
                                hale_syntax::ast::EpochSpec::Birth => Epoch::Birth,
                                hale_syntax::ast::EpochSpec::Dissolve => Epoch::Dissolve,
                                hale_syntax::ast::EpochSpec::Explicit | hale_syntax::ast::EpochSpec::Inline => {
                                    Epoch::Inline
                                }
                            });
                        }
                    }
                }
                LocusMember::BirthCheck(_) => f.birth_check = true,
                LocusMember::Failure(_) => f.handles = true,
                _ => {}
            }
        }
        f.brackets = f.handles && (holds || computed);
        f
    }

    /// Every failure an instance can raise, in the order its epochs come.
    fn sources(&self) -> Vec<FailureSource> {
        let mut out = Vec::new();
        if self.closure_epochs.contains(&Epoch::Birth) || self.violates.contains(&FailureSource::BirthClosure) {
            out.push(FailureSource::BirthClosure);
        }
        if self.birth_check {
            out.push(FailureSource::BirthCheck);
        }
        if self.violates.contains(&FailureSource::Run)
            || self.closure_epochs.contains(&Epoch::Tick)
            || self.closure_epochs.contains(&Epoch::Duration)
        {
            out.push(FailureSource::Run);
        }
        if self.violates.contains(&FailureSource::Handler) {
            out.push(FailureSource::Handler);
        }
        if self.violates.contains(&FailureSource::Drain) {
            out.push(FailureSource::Drain);
        }
        if self.closure_epochs.contains(&Epoch::Dissolve) || self.violates.contains(&FailureSource::Dissolve) {
            out.push(FailureSource::Dissolve);
        }
        out
    }
}

/// A field type whose value is a plain value (inventory C3's
/// `is_plain_value_ty`): a primitive, a declared record or enum, an
/// array of one. A locus, a contract, a generic container is not.
fn plain_value(t: &TypeExpr, universe: SiteUniverse, index: &LocusIndex<'_>) -> bool {
    match t {
        TypeExpr::Primitive(..) => true,
        TypeExpr::Named { path, generic_args, .. } => {
            generic_args.is_empty() && path.segments.len() == 1 && !index.holds_instance(t, universe)
        }
        TypeExpr::Array { elem, .. } | TypeExpr::Bounded { elem, .. } => plain_value(elem, universe, index),
        _ => false,
    }
}

/// A default lowering stores as a constant (a literal, a negated one).
fn constant(e: &Expr) -> bool {
    match e {
        Expr::Literal(..) | Expr::Path(_) => true,
        Expr::Unary { op: UnaryOp::Neg, operand, .. } => matches!(**operand, Expr::Literal(..)),
        _ => false,
    }
}

/// Whether a body holds a `violate` statement.
fn violates(b: &Block) -> bool {
    fn stmt(s: &Stmt) -> bool {
        match s {
            Stmt::Violate { .. } => true,
            Stmt::If(i) => if_chain(i),
            Stmt::Match(m) => m.arms.iter().any(|a| match &a.body {
                MatchArmBody::Block(b) => violates(b),
                MatchArmBody::Expr(_) => false,
            }),
            Stmt::For { body, .. } | Stmt::While { body, .. } | Stmt::Block(body) => violates(body),
            _ => false,
        }
    }
    fn if_chain(i: &IfStmt) -> bool {
        violates(&i.then_block)
            || match i.else_block.as_deref() {
                Some(ElseBranch::Else(b)) => violates(b),
                Some(ElseBranch::ElseIf(n)) => if_chain(n),
                None => false,
            }
    }
    b.stmts.iter().any(stmt)
}

// ------------------------------------------------------------ the rows

/// The rows one instance owes, by what the tree's edges join.
#[derive(Default, Clone)]
struct Rows {
    params_settle: Option<ObligationId>,
    birth: Option<ObligationId>,
    run: Option<ObligationId>,
    drain: Option<ObligationId>,
    dissolve: Option<ObligationId>,
    pinned_join: Option<ObligationId>,
    reclaim: Option<ObligationId>,
    /// Each failure's delivery, raised by the instance.
    deliveries: Vec<ObligationId>,
}

struct Builder<'b, 'a> {
    inputs: &'b LifecycleInputs<'a>,
    subjects: &'b [Subject<'a>],
    facts: Vec<Facts>,
    plan: LifecyclePlan,
    rows: Vec<Rows>,
}

const fn shipped(line: &'static str) -> Rule {
    Rule::line(line, Status::Shipped)
}

const fn open(line: &'static str, row: &'static str) -> Rule {
    Rule::line(line, Status::KnownOpen { inventory_row: row })
}

const POOL_OWNER: &str = "the construction-time domain for an owner placed on a cooperative pool: decision L0-1 names the pool's worker, spec/semantics.md the settling thread";
const NO_OPTION: &str = "the wave-2 decisions frame lines 1-3 as one protocol and choose no option for this line";

fn local() -> Progress {
    Progress { rule: ProgressRule::Local, status: Status::Shipped }
}

fn after(obligation: ObligationId, point: Point, rule: Rule) -> Prerequisite {
    Prerequisite { event: Event { obligation, point }, rule }
}

impl<'b, 'a> Builder<'b, 'a> {
    fn push(&mut self, o: Obligation) -> ObligationId {
        let id = ObligationId(self.plan.obligations.len() as u32);
        self.plan.obligations.push(o);
        id
    }

    fn get(&mut self, id: ObligationId) -> &mut Obligation {
        &mut self.plan.obligations[id.0 as usize]
    }

    fn kind(&self, d: DomainId) -> &DomainKind {
        &self.plan.domains[d.0 as usize].kind
    }

    fn contributions(&self, i: usize) -> &'b [Contribution] {
        &self.subjects[i].contributions
    }

    /// Some contribution of `i`'s template answers yes.
    fn any(&self, i: usize, f: impl Fn(&Contribution) -> bool) -> bool {
        self.contributions(i).iter().any(f)
    }

    /// Every contribution of `i`'s template answers yes.
    fn all(&self, i: usize, f: impl Fn(&Contribution) -> bool) -> bool {
        self.contributions(i).iter().all(f)
    }

    /// One claim from each contribution's (see [`combine`]).
    fn claim(&self, i: usize, f: impl Fn(&Contribution) -> Option<RunsOn>) -> Option<RunsOn> {
        combine(self.contributions(i).iter().map(f))
    }

    /// Each instance `i` is built under, once, in construction order.
    fn owners(&self, i: usize) -> Vec<usize> {
        let mut out: Vec<usize> = Vec::new();
        for o in self.contributions(i).iter().filter_map(|c| c.owner) {
            if !out.contains(&o) {
                out.push(o);
            }
        }
        out
    }

    /// The owner's contributions `c` is built under: the one it is paired
    /// with, or every one.
    fn under(&self, c: &Contribution) -> Vec<&'b Contribution> {
        let Some(o) = c.owner else { return Vec::new() };
        let all = self.contributions(o);
        match c.under {
            Some(k) => all.get(k).into_iter().collect(),
            None => all.iter().collect(),
        }
    }

    /// The instance is a pinned domain's anchor: it runs on a thread of
    /// its own. Its fields initialize there too; their later teardown
    /// remains a separate obligation.
    fn is_pinned(&self, i: usize) -> bool {
        let Template::Static(key) = &self.subjects[i].site.template else { return false };
        self.all(i, |c| c.own.is_some_and(|d| matches!(self.kind(d), DomainKind::Pinned { anchor, .. } if anchor == key)))
    }

    /// In a pinned domain without being its anchor: where its own run
    /// would execute is not the domain's thread.
    fn under_pinned(&self, i: usize, c: &Contribution) -> bool {
        !self.is_pinned(i) && c.own != c.it && c.own.is_some_and(|d| matches!(self.kind(d), DomainKind::Pinned { .. }))
    }

    fn is_pool(&self, d: Option<DomainId>) -> bool {
        d.is_some_and(|d| matches!(self.kind(d), DomainKind::Pool { .. }))
    }

    fn is_async_pool(&self, d: Option<DomainId>) -> bool {
        d.is_some_and(|d| matches!(self.kind(d), DomainKind::Pool { async_io: true, .. }))
    }

    /// A field the root's placement entry put on a pool other than the
    /// one building it: its lifecycle methods' domains wait on line 3.
    fn pool_placed(&self, c: &Contribution) -> bool {
        self.is_pool(c.own) && c.own != c.it
    }

    /// Its `run()` is posted to the pool worker that tears its owner down
    /// (line 19, the retention L5 shipped): the run is retained against
    /// that teardown, which cancels it if it is still queued before it
    /// reclaims the child, so its end is ordered before the reclaim's
    /// completion, not before the drain. Only a field or an accepted
    /// child is posted (inventory C12: a placed field, or one owned beyond
    /// its scope); a body literal's run() runs inline at its statement.
    fn posted_to_its_owners_teardown(&self, i: usize, c: &Contribution) -> bool {
        let under = self.under(c);
        matches!(self.subjects[i].how, How::Field | How::Accepted { .. })
            // Static fields inside a pool anchor's init run inline on
            // its worker; the init does not post them behind itself.
            && !matches!(self.subjects[i].site.template, Template::Static(_))
            && self.is_pool(c.own)
            && !under.is_empty()
            && under.iter().all(|o| o.it == c.own)
    }

    /// The domain the instance's birth runs on: its pinned thread, or
    /// the instantiating thread.
    fn birth_domain(&self, i: usize, c: &Contribution) -> Option<DomainId> {
        if self.is_pinned(i) { c.own } else { c.it }
    }

    fn site(&self, i: usize) -> Option<SourceSite> {
        Some(self.subjects[i].site.clone())
    }

    fn row(&self, i: usize, kind: super::ObligationKind, holder: Holder) -> Obligation {
        Obligation {
            site: self.site(i),
            kind,
            epoch: None,
            source: None,
            guard: PathGuard::Normal,
            holder,
            runs_on: None,
            edges: Edges::default(),
            terminals: vec![Terminal::Completed],
            multiplicity: Multiplicity::OncePerInstance,
            lifetime: Vec::new(),
            progress: local(),
            line: None,
            status: Status::Shipped,
        }
    }

    fn on(domain: Option<DomainId>, rule: Rule) -> Option<RunsOn> {
        domain.map(|domain| RunsOn { domains: BTreeSet::from([domain]), rule })
    }

    /// The spine that tears the instance down, and the domain role it
    /// runs on there.
    fn teardown(&self, i: usize) -> (Spine, DomainRole) {
        let s = &self.subjects[i];
        if self.is_pinned(i) {
            return (Spine::PinnedMain, DomainRole::Own);
        }
        match s.how {
            How::Field => (Spine::Cascade, DomainRole::Teardown),
            How::Accepted { .. } => (Spine::Reclaim, DomainRole::Teardown),
            How::Top { built: Built::Statement, .. } | How::Body { built: Built::Statement, .. } => {
                (Spine::EagerTeardown, DomainRole::Instantiating)
            }
            How::Top { .. } | How::Body { .. } => (Spine::DeferredEntry, DomainRole::Teardown),
            How::Adapter => (Spine::Process, DomainRole::Main),
        }
    }

    /// The instances whose handler a failure of `i` reaches (each owner
    /// it is built under that routes it), and whether a handler of
    /// theirs restarts it.
    fn route(&self, i: usize) -> Option<(Vec<usize>, bool)> {
        let mut owners = Vec::new();
        let mut restarts = false;
        for o in self.owners(i) {
            if let Some(r) = self.restarts_from(o, i) {
                owners.push(o);
                restarts |= r;
            }
        }
        (!owners.is_empty()).then_some((owners, restarts))
    }

    /// Whether `o`'s handler restarts `i`, where it has one for it.
    fn restarts_from(&self, o: usize, i: usize) -> Option<bool> {
        let row = self.inputs.handlers.route(&self.subjects[o].site.decl.lowered, &self.subjects[i].site.decl.lowered)?;
        Some(row.ops.iter().any(|op| matches!(op, RecoveryOp::Restart | RecoveryOp::RestartInPlace)))
    }

    /// Every row one instance owes, in the order its domain performs
    /// them.
    fn instance(&mut self, i: usize) -> Rows {
        let subjects = self.subjects;
        let s = &subjects[i];
        let mut r = Rows::default();
        let pinned = self.is_pinned(i);
        let on_pool = self.any(i, |c| self.is_pool(c.own));
        let on_async_pool = self.any(i, |c| self.is_async_pool(c.own));
        let in_pool_init = matches!(s.site.template, Template::Static(_)) && !s.placed
            && self.all(i, |c| self.is_pool(c.own) && c.own == c.it);
        let posted_to_teardown = self.any(i, |c| self.posted_to_its_owners_teardown(i, c));
        let accepted = matches!(s.how, How::Accepted { .. });
        let flow = matches!(s.how, How::Accepted { flow: true });
        let let_bound = matches!(s.how, How::Body { built: Built::Let | Built::Nested, .. });
        let brackets = self.facts[i].brackets;
        // A pinned thread runs `run()` whether the author wrote one or the
        // desugar gave the empty one.
        let run = self.facts[i].run || pinned;
        let subscribes = self.facts[i].subscribes;
        let instantiation = Holder { spine: Spine::Instantiation, domain: DomainRole::Instantiating };

        // Params settle (line 1): the bracket, when the declaration has one.
        if brackets {
            let mut o = self.row(i, K::ParamsSettle, instantiation);
            if pinned || (s.placed && on_pool) {
                o.holder.domain = DomainRole::Own;
            }
            o.line = Some("1");
            // Anchors settle on their initialization thread. The wider
            // construction-delivery policy for pool owners is pending.
            o.runs_on = self.claim(i, |c| {
                let on = initialization_domain(self.inputs.placement, s.placed, c);
                if self.pool_placed(c) {
                    Self::on(on, Rule::line("1", Status::Pending { condition: POOL_OWNER }))
                } else {
                    Self::on(on, shipped("1"))
                }
            });
            r.params_settle = Some(self.push(o));
        }
        // Accept (line 5): after the params, before the birth; no rejection.
        let accept = accepted.then(|| {
            let mut o = self.row(i, K::Accept, instantiation);
            o.line = Some("5");
            o.runs_on = self.claim(i, |c| Self::on(c.it, shipped("5")));
            o.lifetime.push(Retention {
                resource: Resource::OwnerArena,
                until: Event { obligation: ObligationId(0), point: Point::Completed },
                status: Status::Shipped,
            });
            let id = self.push(o);
            self.get(id).lifetime[0].until.obligation = id;
            id
        });
        // Registration before birth, and readiness after it (line 6).
        let subscribe = subscribes.then(|| {
            let mut o = self.row(i, K::Subscribe, instantiation);
            o.line = Some("6");
            o.runs_on = self.claim(i, |c| Self::on(c.it, shipped("6")));
            self.push(o)
        });
        // Birth: on the pinned thread, else on the instantiating thread,
        // a pool-placed locus's domain pending (line 3).
        let birth_holder = if pinned {
            Holder { spine: Spine::PinnedMain, domain: DomainRole::Own }
        } else {
            instantiation
        };
        let mut o = self.row(i, K::Birth, birth_holder);
        o.multiplicity = Multiplicity::OncePerIncarnation;
        o.terminals = vec![Terminal::Completed, Terminal::FailureDelivered];
        o.runs_on = self.claim(i, |c| {
            if self.pool_placed(c) {
                Self::on(c.it, Rule::line("3", Status::Pending { condition: NO_OPTION }))
            } else {
                Self::on(self.birth_domain(i, c), Rule::SHIPPED)
            }
        });
        if let Some(p) = r.params_settle {
            o.edges.entry.push(after(p, Point::Completed, Rule::SHIPPED));
        }
        if let Some(a) = accept {
            o.edges.entry.push(after(a, Point::Completed, shipped("5")));
        }
        if let Some(sub) = subscribe {
            o.edges.entry.push(after(sub, Point::Completed, shipped("6")));
        }
        let birth = self.push(o);
        r.birth = Some(birth);
        if subscribe.is_some() {
            let mut o = self.row(i, K::Readiness, Holder { spine: Spine::Instantiation, domain: DomainRole::Own });
            o.line = Some("6");
            o.status = Status::KnownOpen { inventory_row: "C8" };
            o.edges.entry.push(after(birth, Point::Completed, open("6", "C8")));
            o.lifetime.push(Retention {
                resource: Resource::Cell,
                until: Event { obligation: birth, point: Point::Completed },
                status: Status::KnownOpen { inventory_row: "C8" },
            });
            self.push(o);
        }
        // The birth-epoch closures and the birth_check (lines 8, 9).
        self.failures(i, &mut r, &[FailureSource::BirthClosure, FailureSource::BirthCheck]);
        // The run: admitted to its pool (line 19), then executed.
        if run {
            let posted = (on_pool && !in_pool_init) || pinned;
            let run_holder = if pinned {
                Holder { spine: Spine::PinnedMain, domain: DomainRole::Own }
            } else {
                Holder { spine: Spine::PoolRun, domain: DomainRole::Own }
            };
            if posted && !pinned {
                let mut o = self.row(i, K::RunAdmission, instantiation);
                o.line = Some("19");
                o.status = Status::KnownOpen { inventory_row: "R19" };
                o.terminals = vec![
                    Terminal::Completed,
                    Terminal::NotStarted(NotStarted::Shutdown(ShutdownCause::PoolShutdown)),
                ];
                o.multiplicity = Multiplicity::OncePerTrigger;
                o.edges.entry.push(after(birth, Point::Completed, Rule::SHIPPED));
                self.push(o);
            }
            let mut o = self.row(i, K::Run, run_holder);
            o.multiplicity = Multiplicity::OncePerIncarnation;
            // A nested field's inline run executes on its anchor's
            // initialization thread, the pool the table gives it (C50).
            o.runs_on = self.claim(i, |c| {
                if self.under_pinned(i, c) {
                    None
                } else {
                    Self::on(c.own, Rule::SHIPPED)
                }
            });
            o.edges.entry.push(after(birth, Point::Completed, Rule::SHIPPED));
            // Every end an occurrence can reach, under any contribution.
            o.terminals = vec![Terminal::Completed, Terminal::FailureDelivered];
            if on_pool && !in_pool_init {
                o.terminals.push(Terminal::NotStarted(NotStarted::Shutdown(ShutdownCause::PoolShutdown)));
                o.terminals.push(Terminal::NotStarted(NotStarted::Acknowledged));
            }
            if on_async_pool && !in_pool_init {
                o.terminals.push(Terminal::CanceledAfterStart);
            }
            o.lifetime.push(Retention {
                resource: Resource::Instance,
                until: Event { obligation: ObligationId(0), point: Point::Ended },
                status: Status::Shipped,
            });
            let id = self.push(o);
            self.get(id).lifetime[0].until.obligation = id;
            r.run = Some(id);
            // A started run a shutdown abandons, parked on an async pool
            // (line 19, R20a): its own named step, on the async pools of
            // the contributions that put it on one.
            if on_async_pool {
                let mut o = self.row(i, K::Cancellation, Holder { spine: Spine::PoolRun, domain: DomainRole::PoolWorker });
                o.line = Some("19");
                o.guard = PathGuard::DrainInFlight;
                o.runs_on = combine(
                    self.contributions(i)
                        .iter()
                        .filter(|c| self.is_async_pool(c.own))
                        .map(|c| Self::on(c.own, shipped("19"))),
                );
                o.edges.entry.push(after(id, Point::Entered, shipped("19")));
                self.push(o);
            }
        }
        // The closures after a run and after a handler (lines 2, 9).
        for epoch in self.facts[i].closure_epochs.clone() {
            let mut o = self.row(i, K::Closures, Holder { spine: Spine::PoolRun, domain: DomainRole::Own });
            o.epoch = Some(epoch);
            o.multiplicity = Multiplicity::OncePerIncarnation;
            match epoch {
                Epoch::Tick | Epoch::Duration if self.any(i, |c| self.pool_placed(c)) => {
                    o.line = Some("2");
                    o.status = Status::Pending { condition: NO_OPTION };
                }
                Epoch::Dissolve => o.line = Some("10"),
                _ => o.line = Some("9"),
            }
            self.push(o);
        }
        self.failures(i, &mut r, &[FailureSource::Run, FailureSource::Handler]);
        // The teardown: drain, dissolve, the pinned join, the reclaim.
        let (spine, role) = self.teardown(i);
        let holder = Holder { spine, domain: role };
        let mut o = self.row(i, K::Drain, holder);
        if pinned {
            o.runs_on = self.claim(i, |c| Self::on(c.own, shipped("12")));
        }
        // A run queued behind its owner's teardown is canceled by the
        // reclaim, after this drain: its end is ordered before the reclaim
        // (for every occurrence, once one contribution posts it there).
        if let Some(run) = r.run.filter(|_| !posted_to_teardown) {
            let rule = if flow || let_bound { shipped("11") } else { Rule::SHIPPED };
            o.edges.entry.push(after(run, Point::Ended, rule));
        }
        o.edges.entry.push(after(birth, Point::Completed, Rule::SHIPPED));
        if let_bound {
            o.line = Some("11");
        }
        if s.how == How::Field && self.owners(i).into_iter().any(|p| self.is_pinned(p)) && !pinned {
            o.line = Some("12");
            o.status = Status::KnownOpen { inventory_row: "C9" };
        }
        let drain = self.push(o);
        r.drain = Some(drain);
        self.failures(i, &mut r, &[FailureSource::Drain]);
        let mut o = self.row(i, K::Dissolve, holder);
        o.line = Some("10");
        // A pool-placed locus's dissolve runs on the teardown thread
        // today; its domain waits on line 3, as its birth's does.
        if pinned {
            o.runs_on = self.claim(i, |c| Self::on(c.own, Rule::SHIPPED));
        }
        o.edges.entry.push(after(drain, Point::Completed, Rule::SHIPPED));
        let dissolve = self.push(o);
        r.dissolve = Some(dissolve);
        self.failures(i, &mut r, &[FailureSource::Dissolve]);
        if pinned {
            let mut o = self.row(i, K::PinnedJoin, Holder { spine: Spine::DeferredEntry, domain: DomainRole::Teardown });
            o.edges.completion.push(after(dissolve, Point::Completed, Rule::SHIPPED));
            o.progress = Progress {
                rule: ProgressRule::Join { pumps: K::FailureDelivery },
                status: Status::KnownOpen { inventory_row: "C18" },
            };
            o.lifetime.push(Retention {
                resource: Resource::Mailbox,
                until: Event { obligation: ObligationId(0), point: Point::Completed },
                status: Status::Shipped,
            });
            let id = self.push(o);
            self.get(id).lifetime[0].until.obligation = id;
            r.pinned_join = Some(id);
            let mut o = self.row(i, K::JoinProgress, Holder { spine: Spine::DeferredEntry, domain: DomainRole::Teardown });
            o.line = Some("JP");
            o.status = Status::KnownOpen { inventory_row: "C18" };
            o.edges.entry.push(after(id, Point::Entered, open("JP", "C18")));
            o.progress = Progress {
                rule: ProgressRule::Join { pumps: K::FailureDelivery },
                status: Status::KnownOpen { inventory_row: "C18" },
            };
            self.push(o);
        }
        if subscribes {
            let mut o = self.row(i, K::TeardownDelivery, holder);
            o.line = Some("17");
            o.status = Status::Pending {
                condition: "the deferred rule is the baseline only if GH #253's final-publish guarantees hold across eager, deferred and declaration permutations",
            };
            o.edges.completion.push(after(dissolve, Point::Entered, Rule::line("17", o.status)));
            self.push(o);
        }
        let reclaim_holder =
            Holder { spine: if spine == Spine::PinnedMain { Spine::DeferredEntry } else { spine }, domain: DomainRole::Teardown };
        let mut o = self.row(i, K::Reclaim, reclaim_holder);
        o.line = Some("14");
        o.edges.entry.push(after(dissolve, Point::Completed, Rule::SHIPPED));
        if let Some(j) = r.pinned_join {
            o.edges.entry.push(after(j, Point::Completed, Rule::SHIPPED));
        }
        for &d in &r.deliveries {
            // The failed child is kept until its handler completes (line 8;
            // line 4 for a delivery in teardown).
            o.edges.entry.push(after(d, Point::Completed, shipped("8")));
        }
        o.lifetime.push(Retention {
            resource: Resource::Arena,
            until: Event { obligation: ObligationId(0), point: Point::Completed },
            status: Status::Shipped,
        });
        let id = self.push(o);
        self.get(id).lifetime[0].until.obligation = id;
        r.reclaim = Some(id);
        // A run still queued behind its owner's teardown on the worker is
        // canceled inside the reclaim and named, NotStarted(Acknowledged),
        // before the child is released (line 19, the retention L5
        // shipped).
        // Claimed on the pools of the contributions that post it there.
        if let Some(run) = r.run.filter(|_| posted_to_teardown) {
            let mut o = self.row(i, K::Cancellation, reclaim_holder);
            o.line = Some("19");
            o.guard = PathGuard::DrainInFlight;
            o.runs_on = combine(
                self.contributions(i)
                    .iter()
                    .filter(|c| self.posted_to_its_owners_teardown(i, c))
                    .map(|c| Self::on(c.own, shipped("19"))),
            );
            o.edges.entry.push(after(id, Point::Entered, shipped("19")));
            let cancel = self.push(o);
            self.get(id).edges.completion.push(after(run, Point::Ended, shipped("19")));
            // Only an occurrence posted there has a cancellation to wait for.
            if self.all(i, |c| self.posted_to_its_owners_teardown(i, c)) {
                self.get(id).edges.completion.push(after(cancel, Point::Completed, shipped("19")));
            }
        }
        r
    }

    /// The rows of each failure `i` can raise from `sources`: its
    /// delivery on its own path, the held alternative while the owner's
    /// params are open, and the recovery decision and restart when the
    /// owner's handler can restart it.
    fn failures(&mut self, i: usize, r: &mut Rows, sources: &[FailureSource]) {
        let Some((owners, restarts)) = self.route(i) else { return };
        let subjects = self.subjects;
        let s = &subjects[i];
        let pinned = self.is_pinned(i);
        let owner_brackets = owners.iter().any(|&o| self.facts[o].brackets);
        let field = s.how == How::Field;
        let flow = matches!(s.how, How::Accepted { flow: true });
        let in_fn_main_cascade = field && {
            // A field of a template's top whose cascade is lowered in
            // `fn main`, where no locus is `self` (inventory C31).
            owners.iter().any(|&o| matches!(self.subjects[o].how, How::Top { .. }))
        };
        // The occurrences a handler is reached from: the contributions
        // whose owner routes the failure.
        let routed: Vec<&Contribution> =
            self.contributions(i).iter().filter(|c| c.owner.is_some_and(|o| owners.contains(&o))).collect();
        let present = self.facts[i].sources();
        for source in sources.iter().copied().filter(|src| present.contains(src)) {
            let (guard, epoch) = match source {
                FailureSource::BirthClosure => (PathGuard::FailedInBirth, Some(Epoch::Birth)),
                FailureSource::BirthCheck => (PathGuard::FailedInBirth, Some(Epoch::Inline)),
                FailureSource::Run | FailureSource::Handler => (PathGuard::FailedInRun, Some(Epoch::Inline)),
                FailureSource::Drain => (PathGuard::FailedInTeardown, Some(Epoch::Inline)),
                FailureSource::Dissolve => (PathGuard::FailedInTeardown, Some(Epoch::Dissolve)),
            };
            let enclosing = match source {
                FailureSource::BirthClosure | FailureSource::BirthCheck => r.birth,
                FailureSource::Run => r.run,
                FailureSource::Handler => None,
                FailureSource::Drain => r.drain,
                FailureSource::Dissolve => r.dissolve,
            };
            // Where the existence rule is not yet kept.
            let (line, status) = match source {
                FailureSource::BirthCheck if pinned => ("8", Status::KnownOpen { inventory_row: "C38" }),
                FailureSource::Dissolve if flow => ("4", Status::KnownOpen { inventory_row: "C25" }),
                FailureSource::Dissolve if in_fn_main_cascade => ("4", Status::KnownOpen { inventory_row: "C31" }),
                FailureSource::BirthClosure | FailureSource::BirthCheck => ("8", Status::Shipped),
                FailureSource::Dissolve => ("10", Status::Shipped),
                _ => ("9", Status::Shipped),
            };
            // Decision L0-1: the handler runs on the owner's domain. In
            // place on the raising thread is the same thing only when the
            // two are one domain (inventory C36 otherwise). Each occurrence
            // against the owner occurrence it is built under.
            let mut claims = Vec::new();
            for c in &routed {
                let raised_on = self.raised_on(i, c, source);
                for oc in self.under(c) {
                    claims.push(match (raised_on, oc.own) {
                        (Some(a), Some(b)) if a == b => Self::on(oc.own, shipped("L0-1")),
                        (_, Some(_)) => Self::on(oc.own, open("L0-1", "C36")),
                        _ => None,
                    });
                }
            }
            let claim = combine(claims);
            let delivered_in_place = claim.as_ref().is_some_and(|c| c.rule.status == Status::Shipped);
            let mut o = self.row(i, K::FailureDelivery, Holder { spine: Spine::QueueDrain, domain: DomainRole::Owner });
            o.source = Some(source);
            o.epoch = epoch;
            o.guard = guard;
            o.line = Some(line);
            o.status = status;
            o.runs_on = claim;
            o.multiplicity = Multiplicity::OncePerTrigger;
            o.terminals = vec![Terminal::Completed, Terminal::ClosureViolation];
            o.progress = Progress {
                rule: ProgressRule::WaitsFor {
                    event: Event { obligation: ObligationId(0), point: Point::Completed },
                    owed_by: DomainRole::Owner,
                },
                status: if delivered_in_place {
                    Status::Shipped
                } else {
                    Status::KnownOpen { inventory_row: "C36" }
                },
            };
            if let Some(e) = enclosing {
                match source {
                    // Birth-epoch closures and the check follow birth().
                    FailureSource::BirthClosure | FailureSource::BirthCheck => {
                        o.edges.entry.push(after(e, Point::Completed, shipped("8")));
                    }
                    _ => {
                        o.edges.entry.push(after(e, Point::Entered, Rule::SHIPPED));
                        o.edges.within = Some(e);
                    }
                }
            }
            let id = self.push(o);
            if let ProgressRule::WaitsFor { event, .. } = &mut self.get(id).progress.rule {
                event.obligation = id;
            }
            self.get(id).lifetime = vec![
                Retention {
                    resource: Resource::Instance,
                    until: Event { obligation: id, point: Point::Ended },
                    status: Status::Shipped,
                },
                Retention {
                    resource: Resource::FailurePayload,
                    until: Event { obligation: id, point: Point::Ended },
                    status: Status::Shipped,
                },
            ];
            // Delivered at the failing epoch: a run's own failure, in place
            // on its own domain, completes inside the run (line 9).
            if let Some(e) = enclosing {
                if matches!(source, FailureSource::Run | FailureSource::Drain | FailureSource::Dissolve) && delivered_in_place {
                    self.get(e).edges.completion.push(after(id, Point::Completed, shipped("9")));
                }
            }
            r.deliveries.push(id);
            self.recovery(i, source, guard, id, restarts);
            // The held alternative (line 1): raised while the owner's
            // params are open, delivered at its settle, before its birth.
            let holdable = owner_brackets
                && field
                && !pinned
                && matches!(source, FailureSource::BirthClosure | FailureSource::BirthCheck | FailureSource::Run);
            if holdable {
                self.held(i, source, epoch, &routed, restarts, r);
            }
        }
    }

    /// Where the occurrences of contribution `c` raise a failure of
    /// `source`; `None` where it is not one known domain.
    fn raised_on(&self, i: usize, c: &Contribution, source: FailureSource) -> Option<DomainId> {
        match source {
            FailureSource::BirthClosure | FailureSource::BirthCheck => self.birth_domain(i, c),
            FailureSource::Run | FailureSource::Handler => c.own,
            FailureSource::Drain | FailureSource::Dissolve => {
                if self.is_pinned(i) {
                    c.own
                } else {
                    None
                }
            }
        }
    }

    /// The held alternative of one failure source.
    #[allow(clippy::too_many_arguments)]
    fn held(
        &mut self,
        i: usize,
        source: FailureSource,
        epoch: Option<Epoch>,
        routed: &[&Contribution],
        restarts: bool,
        r: &mut Rows,
    ) {
        // The hold and the delivery at settle are shipped for every owner;
        // which domain a pool-placed owner's delivery owes is pending.
        let mut o = self.row(i, K::FailureDelivery, Holder { spine: Spine::Settle, domain: DomainRole::Instantiating });
        o.source = Some(source);
        o.epoch = epoch;
        o.guard = PathGuard::FailedAtSettle;
        o.line = Some("1");
        o.multiplicity = Multiplicity::OncePerTrigger;
        o.terminals = vec![Terminal::Completed];
        // Raised and delivered on one thread only when the failing child
        // raises on the thread settling the owner it is built under.
        let mut claims = Vec::new();
        for c in routed {
            let raised_on = self.raised_on(i, c, source);
            for oc in self.under(c) {
                let owner = c.owner.expect("a routed contribution has an owner");
                let settling = initialization_domain(self.inputs.placement, self.subjects[owner].placed, oc);
                let rule = if self.pool_placed(oc) {
                    Rule::line("1", Status::Pending { condition: POOL_OWNER })
                } else {
                    shipped("1")
                };
                claims.push(if raised_on == settling { Self::on(settling, rule) } else { None });
            }
        }
        o.runs_on = combine(claims);
        let enclosing = match source {
            FailureSource::Run => r.run,
            _ => r.birth,
        };
        if let Some(e) = enclosing {
            o.edges.entry.push(match source {
                FailureSource::Run => after(e, Point::Entered, Rule::SHIPPED),
                _ => after(e, Point::Completed, shipped("8")),
            });
        }
        // Completed at each owner's settle: the tree's edges, once every
        // owner's rows exist.
        let delivery = self.push(o);
        self.get(delivery).lifetime = vec![
            Retention { resource: Resource::Instance, until: Event { obligation: delivery, point: Point::Completed }, status: Status::Shipped },
            Retention {
                resource: Resource::FailurePayload,
                until: Event { obligation: delivery, point: Point::Completed },
                status: Status::Shipped,
            },
        ];
        let mut o = self.row(i, K::ConstructionDelivery, Holder { spine: Spine::Settle, domain: DomainRole::Instantiating });
        o.source = Some(source);
        o.guard = PathGuard::FailedAtSettle;
        o.line = Some("1");
        o.multiplicity = Multiplicity::OncePerTrigger;
        o.edges.entry.push(after(delivery, Point::Entered, shipped("1")));
        o.edges.completion.push(after(delivery, Point::Completed, shipped("1")));
        self.push(o);
        r.deliveries.push(delivery);
        self.recovery(i, source, PathGuard::FailedAtSettle, delivery, restarts);
        // A birth failure skipped the run: the child resumes once its
        // handler returns, through the same placement and admission as a
        // first run (line 13; inline today for a pool-placed child, C43).
        if matches!(source, FailureSource::BirthClosure | FailureSource::BirthCheck) && self.facts[i].run {
            let resume_rule = if routed.iter().any(|c| self.pool_placed(c)) { open("13", "C43") } else { shipped("13") };
            let mut o = self.row(i, K::Resume, Holder { spine: Spine::Settle, domain: DomainRole::Instantiating });
            o.source = Some(source);
            o.guard = PathGuard::FailedAtSettle;
            o.line = Some("13");
            o.status = resume_rule.status;
            o.edges.entry.push(after(delivery, Point::Completed, shipped("1")));
            let resume = self.push(o);
            let mut o = self.row(i, K::Run, Holder { spine: Spine::PoolRun, domain: DomainRole::Own });
            o.source = Some(source);
            o.guard = PathGuard::FailedAtSettle;
            o.multiplicity = Multiplicity::OncePerIncarnation;
            o.runs_on =
                combine(routed.iter().map(|c| if self.under_pinned(i, c) { None } else { Self::on(c.own, resume_rule) }));
            o.terminals = vec![Terminal::Completed, Terminal::FailureDelivered];
            o.edges.entry.push(after(resume, Point::Entered, resume_rule));
            o.edges.entry.push(after(delivery, Point::Completed, Rule::line("13", resume_rule.status)));
            self.push(o);
        }
    }

    /// The handler's recovery decision and, when it restarts, the
    /// restart: performed, or refused once the owner is in teardown or
    /// the process drains (restart during drain, RD).
    fn recovery(&mut self, i: usize, source: FailureSource, guard: PathGuard, delivery: ObligationId, restarts: bool) {
        let mut o = self.row(i, K::RecoveryDecision, Holder { spine: Spine::QueueDrain, domain: DomainRole::Owner });
        o.source = Some(source);
        o.guard = guard;
        o.line = Some("RD");
        o.multiplicity = Multiplicity::OncePerTrigger;
        o.edges.entry.push(after(delivery, Point::Entered, Rule::SHIPPED));
        o.edges.completion.push(after(delivery, Point::Completed, Rule::SHIPPED));
        let decision = self.push(o);
        if !restarts {
            return;
        }
        let spine = if self.is_pinned(i) { Spine::PinnedMain } else { Spine::PoolRun };
        let mut o = self.row(i, K::Restart, Holder { spine, domain: DomainRole::Own });
        o.source = Some(source);
        o.guard = PathGuard::Restart;
        o.line = Some("RD");
        o.multiplicity = Multiplicity::OncePerTrigger;
        o.edges.entry.push(after(decision, Point::Completed, Rule::SHIPPED));
        o.edges.entry.push(after(delivery, Point::Completed, shipped("1")));
        self.push(o);
        // A locus that declares no run() owes none in its next
        // incarnation (line 13); the resume enters one (inventory C48).
        if !self.facts[i].run && !self.is_pinned(i) {
            let mut o = self.row(i, K::Run, Holder { spine, domain: DomainRole::Own });
            o.source = Some(source);
            o.guard = PathGuard::Restart;
            o.line = Some("13");
            o.status = Status::KnownOpen { inventory_row: "C48" };
            o.multiplicity = Multiplicity::AtMostOncePerInstance;
            o.terminals = vec![Terminal::NotStarted(NotStarted::NoRun)];
            o.edges.entry.push(after(decision, Point::Completed, Rule::SHIPPED));
            self.push(o);
        }
        // Refused under teardown: no incarnation begins.
        let mut o = self.row(i, K::Restart, Holder { spine, domain: DomainRole::Own });
        o.source = Some(source);
        o.guard = PathGuard::DrainInFlight;
        o.line = Some("RD");
        o.status = Status::KnownOpen { inventory_row: "C42" };
        o.multiplicity = Multiplicity::AtMostOncePerInstance;
        o.terminals = vec![
            Terminal::NotStarted(NotStarted::Shutdown(ShutdownCause::OwnerTeardown)),
            Terminal::NotStarted(NotStarted::Shutdown(ShutdownCause::ProcessDrain)),
        ];
        o.edges.entry.push(after(decision, Point::Completed, Rule::SHIPPED));
        self.push(o);
    }

    /// The edges between an instance and each owner it is built under:
    /// born before the owner's birth, drained before its drain,
    /// dissolved after its dissolve, reclaimed before its reclaim; and a
    /// held failure delivered before the owner's birth.
    fn tree_edges(&mut self, i: usize) {
        for o in self.owners(i) {
            self.owner_edges(i, o);
        }
    }

    fn owner_edges(&mut self, i: usize, o: usize) {
        let child = self.rows[i].clone();
        let parent = self.rows[o].clone();
        let pinned = self.is_pinned(i);
        let parent_pinned = self.is_pinned(o);
        let field = self.subjects[i].how == How::Field;
        let restartable = self.restarts_from(o, i).unwrap_or(false);
        if field && !pinned {
            // Born inline in the owner's params loop (inventory C5); a
            // restart births it again later, so the order holds for a child
            // nothing restarts.
            if let (Some(cb), Some(pb), false) = (child.birth, parent.birth, restartable) {
                self.get(pb).edges.entry.push(after(cb, Point::Completed, Rule::SHIPPED));
            }
            // The owner's dissolve, then the cascade's (line 10, C31).
            if let (Some(cd), Some(pd)) = (child.dissolve, parent.dissolve) {
                self.get(cd).edges.entry.push(after(pd, Point::Completed, shipped("10")));
            }
        }
        if field {
            // Owned fields drain before their owner, in their own domain
            // (line 12). A pinned locus's are never drained (C9), and a
            // contract-typed field is torn down through its recorded
            // reclaim, its whole spine after the owner's dissolve (C32).
            if let (Some(cd), Some(pd)) = (child.drain, parent.drain) {
                let rule = if parent_pinned {
                    open("12", "C9")
                } else if self.subjects[i].contract {
                    open("12", "C32")
                } else {
                    shipped("12")
                };
                self.get(pd).edges.entry.push(after(cd, Point::Completed, rule));
            }
        }
        if matches!(self.subjects[i].how, How::Accepted { .. }) {
            // A resident accepted child is reclaimed by its owner's
            // cascade after the owner's dissolve (inventory C28).
            if let (Some(cd), Some(pd), How::Accepted { flow: false }) = (child.drain, parent.dissolve, self.subjects[i].how) {
                self.get(cd).edges.entry.push(after(pd, Point::Completed, Rule::SHIPPED));
            }
        }
        // Children before their owner's arena (line 14).
        if let (Some(cr), Some(pr)) = (child.reclaim, parent.reclaim) {
            self.get(pr).edges.entry.push(after(cr, Point::Completed, shipped("14")));
        }
        // A held failure: delivered once the owner's last param is
        // stored, completed at its settle, before its birth (line 1).
        let held: Vec<ObligationId> = child
            .deliveries
            .iter()
            .copied()
            .filter(|d| self.plan.obligations[d.0 as usize].guard == PathGuard::FailedAtSettle)
            .collect();
        if let Some(settle) = parent.params_settle.filter(|_| self.restarts_from(o, i).is_some()) {
            for &d in &held {
                self.get(d).edges.completion.push(after(settle, Point::Completed, shipped("1")));
            }
        }
        if let Some(pb) = parent.birth {
            for d in held {
                let rule = Rule::line("1", self.plan.obligations[d.0 as usize].status);
                self.get(pb).edges.entry.push(after(d, Point::Completed, rule));
            }
        }
    }

    /// What the spines owe the process: the main locus's eager teardown
    /// (its pre-drain, the wait-abort, the pool join), `fn main`'s exit
    /// (the pool join, its pre-drain, the wait-abort), and the signal
    /// path's cooperative drain.
    fn process(&mut self) {
        let pools = self.plan.domains.iter().any(|d| matches!(d.kind, DomainKind::Pool { .. }));
        let main = Some(PlacementTable::MAIN);
        let process_row = |kind, spine, line: Option<&'static str>, status| Obligation {
            site: None,
            kind,
            epoch: None,
            source: None,
            guard: PathGuard::Normal,
            holder: Holder { spine, domain: DomainRole::Main },
            runs_on: Self::on(main, Rule::SHIPPED),
            edges: Edges::default(),
            terminals: vec![Terminal::Completed],
            multiplicity: Multiplicity::OncePerTrigger,
            lifetime: Vec::new(),
            progress: local(),
            line,
            status,
        };
        let mut first_join: Option<ObligationId> = None;
        // Each template top a statement in `fn main` builds: torn down by
        // the eager spine where its statement ends.
        let eager: Vec<usize> = (0..self.subjects.len())
            .filter(|&i| matches!(self.subjects[i].how, How::Top { built: Built::Statement, .. }))
            .collect();
        let statements_done: Vec<ObligationId> = eager.iter().filter_map(|&i| self.rows[i].reclaim).collect();
        for i in eager {
            let main_locus = matches!(self.subjects[i].how, How::Top { main_locus: true, .. });
            let fields: Vec<usize> =
                (0..self.subjects.len()).filter(|&c| self.subjects[c].how == How::Field && self.owners(c).contains(&i)).collect();
            // Every teardown spine pre-drains (line 18; not the eager one
            // yet, C13).
            let mut o = process_row(K::PreDrain, Spine::EagerTeardown, Some("18"), Status::KnownOpen { inventory_row: "C13" });
            if let Some(run) = self.rows[i].run {
                o.edges.entry.push(after(run, Point::Completed, open("18", "C13")));
            }
            let pre = self.push(o);
            for &c in &fields {
                if let Some(d) = self.rows[c].drain {
                    self.get(d).edges.entry.push(after(pre, Point::Completed, open("18", "C13")));
                }
            }
            if !main_locus {
                continue;
            }
            // The main locus aborts the waits only teardown ends before it
            // joins the workers they block (line 7; after today, R34).
            let mut o = process_row(K::WaitAbort, Spine::EagerTeardown, Some("7"), Status::Shipped);
            if let Some(run) = self.rows[i].run {
                o.edges.entry.push(after(run, Point::Completed, Rule::SHIPPED));
            }
            let abort = self.push(o);
            if pools {
                let mut o = process_row(K::PoolJoin, Spine::EagerTeardown, None, Status::Shipped);
                o.edges.entry.push(after(abort, Point::Completed, open("7", "R34")));
                if let Some(run) = self.rows[i].run {
                    o.edges.entry.push(after(run, Point::Completed, Rule::SHIPPED));
                }
                o.progress = Progress {
                    rule: ProgressRule::Join { pumps: K::FailureDelivery },
                    status: Status::KnownOpen { inventory_row: "R20" },
                };
                let join = self.push(o);
                first_join.get_or_insert(join);
                // Rule (b): the main locus joins the pools before its
                // fields' teardown.
                for &c in &fields {
                    if let Some(d) = self.rows[c].drain {
                        self.get(d).edges.entry.push(after(join, Point::Completed, Rule::SHIPPED));
                    }
                }
                let mut o = process_row(K::JoinProgress, Spine::EagerTeardown, Some("JP"), Status::KnownOpen { inventory_row: "R20" });
                o.edges.entry.push(after(join, Point::Entered, open("JP", "R20")));
                self.push(o);
            }
        }
        // `fn main`'s fall-through exit: the pool join, its pre-drain, the
        // wait-abort, before the frame's entries are torn down.
        let has_fn_main = self.inputs.bundle.programs.values().any(|p| {
            p.items.iter().any(|it| matches!(it, TopDecl::Fn(f) if f.name.name == "main"))
        });
        if has_fn_main {
            // After every statement of `fn main` has torn its literal down.
            let after_statements = |mut o: Obligation| {
                o.edges.entry.extend(statements_done.iter().map(|&r| after(r, Point::Completed, Rule::SHIPPED)));
                o
            };
            if pools {
                let mut o = process_row(K::PoolJoin, Spine::MainFallThrough, None, Status::Shipped);
                o.progress = Progress {
                    rule: ProgressRule::Join { pumps: K::FailureDelivery },
                    status: Status::KnownOpen { inventory_row: "R20" },
                };
                let join = self.push(after_statements(o));
                first_join.get_or_insert(join);
            }
            let pre = self.push(after_statements(process_row(K::PreDrain, Spine::MainFallThrough, Some("18"), Status::Shipped)));
            self.push(after_statements(process_row(K::WaitAbort, Spine::MainFallThrough, Some("7"), Status::Shipped)));
            // A let-bound literal of `fn main`: run at its statement, drained
            // and dissolved at the scope's exit, after the pre-drain (line 11).
            for i in 0..self.subjects.len() {
                let deferred = matches!(
                    self.subjects[i].how,
                    How::Body { built: Built::Let | Built::Nested, in_fn_main: true } | How::Top { built: Built::Let | Built::Nested, .. }
                );
                if !deferred {
                    continue;
                }
                if let Some(run) = self.rows[i].run {
                    self.get(pre).edges.entry.push(after(run, Point::Completed, shipped("11")));
                }
                if let Some(d) = self.rows[i].drain {
                    self.get(d).edges.entry.push(after(pre, Point::Completed, shipped("11")));
                }
            }
        }
        // A failure of a child the joins wait for completes before the join
        // does (join progress): today because the handler runs in place on
        // the child's thread (C36), once delivery follows L0-1 because the
        // joining owner keeps completing its children's decisions. Stated
        // for a template every occurrence of which is on a pool: the edge
        // holds of each occurrence.
        for i in 0..self.subjects.len() {
            let join = if self.is_pinned(i) {
                self.rows[i].pinned_join
            } else if self.all(i, |c| self.is_pool(c.own)) {
                first_join
            } else {
                None
            };
            let Some(join) = join else { continue };
            // A failure of the running child: one raised in its own
            // teardown comes after the join.
            let deliveries: Vec<ObligationId> = self.rows[i]
                .deliveries
                .iter()
                .copied()
                .filter(|d| {
                    let o = &self.plan.obligations[d.0 as usize];
                    o.guard != PathGuard::FailedAtSettle
                        && matches!(o.source, Some(FailureSource::Run | FailureSource::Handler))
                })
                .collect();
            for d in deliveries {
                self.get(join).edges.completion.push(after(d, Point::Completed, shipped("JP")));
            }
        }
        // Every run on a pool ends before that pool's join completes, and
        // a canceled one names its cancellation first (line 19: a parked
        // run abandoned, R20a, or a queued one its teardown cancels). A
        // run is held to the join where every occurrence is on a pool; a
        // cancellation exists only on one.
        if let Some(join) = first_join {
            for i in 0..self.subjects.len() {
                if !self.any(i, |c| self.is_pool(c.own)) {
                    continue;
                }
                if let Some(run) = self.rows[i].run.filter(|_| self.all(i, |c| self.is_pool(c.own))) {
                    self.get(join).edges.completion.push(after(run, Point::Ended, shipped("19")));
                }
                let cancels: Vec<ObligationId> = self
                    .plan
                    .iter()
                    .filter(|(_, o)| o.kind == K::Cancellation && o.site == self.site(i))
                    .map(|(id, _)| id)
                    .collect();
                for c in cancels {
                    self.get(join).edges.completion.push(after(c, Point::Completed, shipped("19")));
                }
            }
        }
        // A signal raises the cooperative flag; nothing on the signal path
        // calls a lifecycle method (line 15).
        let mut o = process_row(K::ProcessDrain, Spine::Process, Some("15"), Status::Shipped);
        o.guard = PathGuard::DrainInFlight;
        self.push(o);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on(d: u32, rule: Rule) -> Option<RunsOn> {
        Some(RunsOn { domains: BTreeSet::from([DomainId(d)]), rule })
    }

    /// A known limit: contributions that state different rules for one
    /// row make no claim, not a set under either rule. No program reaches
    /// it through the producer today, so it is pinned here: a template
    /// with several contributions is dynamic, each of its occurrences is
    /// built and run on its owner occurrence's domain, and every rule a
    /// contribution states is the same function of domains that agree.
    #[test]
    fn contributions_stating_different_rules_claim_no_domain() {
        let delivered = shipped("L0-1");
        let crossing = open("L0-1", "C36");
        assert_eq!(
            combine([on(0, delivered), on(1, delivered)]),
            Some(RunsOn { domains: BTreeSet::from([DomainId(0), DomainId(1)]), rule: delivered }),
            "one rule: the set"
        );
        assert_eq!(combine([on(0, delivered), on(0, delivered)]), on(0, delivered), "one rule, one domain");
        assert_eq!(combine([on(0, delivered), on(1, crossing)]), None, "two rules: no claim");
        assert_eq!(combine([on(0, delivered), on(0, crossing)]), None, "two rules on one domain: no claim");
        assert_eq!(combine([on(0, delivered), None]), None, "one occurrence's domain unknown: no claim");
    }
}

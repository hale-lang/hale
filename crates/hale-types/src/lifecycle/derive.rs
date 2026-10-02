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
//!   domains.
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
/// and the domains it is built on and runs on.
struct Subject<'a> {
    site: SourceSite,
    decl: &'a LocusDecl,
    universe: SiteUniverse,
    /// The instance whose `on_failure` it fails to and whose arena holds
    /// it: a field's owner, an accepted child's acceptor, a body
    /// literal's enclosing locus. `None` for a template's top and a
    /// literal in a free fn.
    owner: Option<usize>,
    how: How,
    /// The instantiating thread: the domain running the code that holds
    /// the literal. `None` when the scope's domains are unknown or many.
    it: Option<DomainId>,
    /// The queue owner: where its `run()` runs and its cells land.
    own: Option<DomainId>,
    bound: Bound,
    /// Built in an `on_failure` body: it exists only on a path where the
    /// handler runs.
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
        let contract = match (owner, key.path.last()) {
            (Some(o), Some(step)) => contract_field(out[o].decl, &step.field, out[o].universe, index),
            _ => false,
        };
        by_key.insert(key, out.len());
        out.push(Subject {
            site: SourceSite { decl: realizes.clone(), template: Template::Static(key.clone()) },
            decl,
            universe: realizes.site.universe,
            owner,
            how,
            it: Some(PlacementTable::MAIN),
            own: Some(row.domain),
            bound: bound_of(&key.origin),
            in_handler: false,
            contract,
            placed: matches!(row.decided_by, Decision::Entry { .. } | Decision::Binding { .. }),
        });
    }
    // A body literal's owner is its enclosing locus's instance, which may
    // be a dynamic site listed after it: resolved once every instance is.
    let mut enclosed: Vec<(usize, DeclRef)> = Vec::new();
    for d in &t.dynamic {
        let (Some(realizes), Some(decl)) = (d.realizes.clone(), d.realizes.as_ref().and_then(|r| index.decl(r.site))) else {
            continue;
        };
        let at = literals.get(&d.literal).copied().unwrap_or(LiteralAt { built: Built::Nested, member: Member::Other });
        let domain = (d.domains.len() == 1).then(|| *d.domains.iter().next().expect("one"));
        let i = out.len();
        let how = match &d.enclosing {
            Enclosing::Locus(enclosing) => {
                enclosed.push((i, enclosing.clone()));
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
            Enclosing::Fn(_) => How::Body { built: at.built, in_fn_main: at.member == Member::FnMain },
        };
        out.push(Subject {
            site: SourceSite { decl: realizes.clone(), template: Template::Dynamic { literal: d.literal } },
            decl,
            universe: realizes.site.universe,
            owner: None,
            how,
            it: domain,
            own: domain,
            bound: d.bound.clone(),
            in_handler: at.member == Member::Handler,
            contract: false,
            placed: false,
        });
        dynamic_fields(&mut out, i, index, &inputs.bundle.snapshot);
    }
    for (i, enclosing) in enclosed {
        out[i].owner = (0..out.len()).find(|&j| j != i && out[j].site.decl == enclosing);
    }
    // A locus that builds itself (`B { }` in B's run, or two loci that
    // build each other) owns no instance of its own chain: the instance
    // closing a cycle of owners has none.
    for i in 0..out.len() {
        let mut seen = BTreeSet::new();
        let mut at = out[i].owner;
        while let Some(o) = at {
            if o == i {
                out[i].owner = None;
                break;
            }
            if !seen.insert(o) {
                break;
            }
            at = out[o].owner;
        }
    }
    out
}

/// The locus fields a dynamic literal's declaration builds from its
/// defaults, each an instance of its field literal, recursively.
fn dynamic_fields<'a>(out: &mut Vec<Subject<'a>>, parent: usize, index: &LocusIndex<'a>, ids: &Snapshot) {
    let universe = out[parent].universe;
    if universe != SiteUniverse::User {
        return;
    }
    let decl = out[parent].decl;
    for m in &decl.members {
        let LocusMember::Params(pb) = m else { continue };
        for p in &pb.params {
            let ParamInit::Value(Expr::Struct { path, id, .. }) = &p.init else { continue };
            let ty = TypeExpr::Named { path: path.clone(), generic_args: Vec::new(), span: path.span };
            let Some(site) = index.names(&ty, universe) else { continue };
            let Some(field_decl) = index.decl(site) else { continue };
            let Some(literal) = ids.site_id(*id).map(SiteRef::user) else { continue };
            let realizes = DeclRef { site, args: Vec::new(), lowered: field_decl.name.name.clone() };
            let contract = p.ty.as_ref().is_some_and(|ty| index.names(ty, universe).is_none());
            // Every instance of a literal shares its rows: a field literal
            // reached under several of its declaration's literals is one
            // template, as often as they are.
            let template = Template::Dynamic { literal };
            if let Some(j) = out.iter().position(|s| s.site.template == template) {
                out[j].bound = add_bounds(&out[j].bound, &out[parent].bound);
                continue;
            }
            let p = &out[parent];
            let s = Subject {
                contract,
                placed: false,
                site: SourceSite { decl: realizes, template: Template::Dynamic { literal } },
                decl: field_decl,
                universe,
                owner: Some(parent),
                how: How::Field,
                it: p.it,
                own: p.own,
                bound: p.bound.clone(),
                in_handler: p.in_handler,
            };
            let i = out.len();
            out.push(s);
            dynamic_fields(out, i, index, ids);
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

    /// The instance is a pinned domain's anchor: it runs on a thread of
    /// its own. A field of a pinned locus shares its anchor's domain for
    /// its cells, but is built, drained and dissolved off that thread.
    fn is_pinned(&self, i: usize) -> bool {
        let Template::Static(key) = &self.subjects[i].site.template else { return false };
        self.subjects[i]
            .own
            .is_some_and(|d| matches!(self.kind(d), DomainKind::Pinned { anchor, .. } if anchor == key))
    }

    /// In a pinned domain without being its anchor: where its own run
    /// would execute is not the domain's thread.
    fn under_pinned(&self, i: usize) -> bool {
        let s = &self.subjects[i];
        !self.is_pinned(i) && s.own != s.it && s.own.is_some_and(|d| matches!(self.kind(d), DomainKind::Pinned { .. }))
    }

    fn is_pool(&self, d: Option<DomainId>) -> bool {
        d.is_some_and(|d| matches!(self.kind(d), DomainKind::Pool { .. }))
    }

    fn is_async_pool(&self, d: Option<DomainId>) -> bool {
        d.is_some_and(|d| matches!(self.kind(d), DomainKind::Pool { async_io: true, .. }))
    }

    /// A field the root's placement entry put on a pool other than the
    /// one building it: its lifecycle methods' domains wait on line 3.
    fn pool_placed(&self, i: usize) -> bool {
        let s = &self.subjects[i];
        self.is_pool(s.own) && s.own != s.it
    }

    /// A field nested under a pool-placed field: the table gives it its
    /// owner's pool, and no pool is chosen for its `run()`, which runs
    /// inline on the instantiating thread (inventory C12, R17 and R18).
    fn inline_off_its_pool(&self, i: usize) -> bool {
        let s = &self.subjects[i];
        self.is_pool(s.own) && !s.placed && s.own != s.it
    }

    /// Its `run()` is posted to the pool worker that tears its owner down
    /// (line 19, the retention L5 shipped): the run is retained against
    /// that teardown, which cancels it if it is still queued before it
    /// reclaims the child, so its end is ordered before the reclaim's
    /// completion, not before the drain.
    fn posted_to_its_owners_teardown(&self, i: usize) -> bool {
        let s = &self.subjects[i];
        self.is_pool(s.own) && s.owner.is_some_and(|o| self.subjects[o].it == s.own)
    }

    /// The domain the instance's birth runs on: its pinned thread, or
    /// the instantiating thread.
    fn birth_domain(&self, i: usize) -> Option<DomainId> {
        if self.is_pinned(i) { self.subjects[i].own } else { self.subjects[i].it }
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
        domain.map(|domain| RunsOn { domain, rule })
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

    /// The instance whose handler a failure of `i` reaches, and whether
    /// that handler restarts it.
    fn route(&self, i: usize) -> Option<(usize, bool)> {
        let o = self.subjects[i].owner?;
        let row = self.inputs.handlers.route(&self.subjects[o].site.decl.lowered, &self.subjects[i].site.decl.lowered)?;
        let restarts = row.ops.iter().any(|op| matches!(op, RecoveryOp::Restart | RecoveryOp::RestartInPlace));
        Some((o, restarts))
    }

    /// Every row one instance owes, in the order its domain performs
    /// them.
    fn instance(&mut self, i: usize) -> Rows {
        let subjects = self.subjects;
        let s = &subjects[i];
        let mut r = Rows::default();
        let pinned = self.is_pinned(i);
        let pool_placed = self.pool_placed(i);
        let (it, own) = (s.it, s.own);
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
            o.line = Some("1");
            // An owner placed on a cooperative pool settles on the
            // instantiating thread today; which domain it owes is pending.
            o.runs_on = if pool_placed {
                Self::on(it, Rule::line("1", Status::Pending { condition: POOL_OWNER }))
            } else {
                Self::on(it, shipped("1"))
            };
            r.params_settle = Some(self.push(o));
        }
        // Accept (line 5): after the params, before the birth; no rejection.
        let accept = accepted.then(|| {
            let mut o = self.row(i, K::Accept, instantiation);
            o.line = Some("5");
            o.runs_on = Self::on(it, shipped("5"));
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
            o.runs_on = Self::on(it, shipped("6"));
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
        o.runs_on = if pool_placed {
            Self::on(it, Rule::line("3", Status::Pending { condition: NO_OPTION }))
        } else {
            Self::on(self.birth_domain(i), Rule::SHIPPED)
        };
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
            let posted = self.is_pool(own) || pinned;
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
            // A field nested under a pool-placed field owes its run() to
            // the pool the table gives it, and runs it inline on the
            // instantiating thread today (line 3, inventory C12).
            o.runs_on = if self.under_pinned(i) {
                None
            } else if self.inline_off_its_pool(i) {
                Self::on(own, open("3", "C12"))
            } else {
                Self::on(own, Rule::SHIPPED)
            };
            o.edges.entry.push(after(birth, Point::Completed, Rule::SHIPPED));
            o.terminals = vec![Terminal::Completed, Terminal::FailureDelivered];
            if self.is_pool(own) {
                o.terminals.push(Terminal::NotStarted(NotStarted::Shutdown(ShutdownCause::PoolShutdown)));
                o.terminals.push(Terminal::NotStarted(NotStarted::Acknowledged));
            }
            if self.is_async_pool(own) {
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
            // (line 19, R20a): its own named step.
            if self.is_async_pool(own) {
                let mut o = self.row(i, K::Cancellation, Holder { spine: Spine::PoolRun, domain: DomainRole::PoolWorker });
                o.line = Some("19");
                o.guard = PathGuard::DrainInFlight;
                o.runs_on = Self::on(own, shipped("19"));
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
                Epoch::Tick | Epoch::Duration if self.is_pool(own) && own != it => {
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
            o.runs_on = Self::on(own, shipped("12"));
        }
        // A run queued behind its owner's teardown is canceled by the
        // reclaim, after this drain: its end is ordered before the reclaim.
        if let Some(run) = r.run.filter(|_| !self.posted_to_its_owners_teardown(i)) {
            let rule = if flow || let_bound { shipped("11") } else { Rule::SHIPPED };
            o.edges.entry.push(after(run, Point::Ended, rule));
        }
        o.edges.entry.push(after(birth, Point::Completed, Rule::SHIPPED));
        if let_bound {
            o.line = Some("11");
        }
        if s.how == How::Field && s.owner.is_some_and(|p| self.is_pinned(p)) && !pinned {
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
            o.runs_on = Self::on(own, Rule::SHIPPED);
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
        if let Some(run) = r.run.filter(|_| self.posted_to_its_owners_teardown(i)) {
            let mut o = self.row(i, K::Cancellation, reclaim_holder);
            o.line = Some("19");
            o.guard = PathGuard::DrainInFlight;
            o.runs_on = Self::on(own, shipped("19"));
            o.edges.entry.push(after(id, Point::Entered, shipped("19")));
            let cancel = self.push(o);
            self.get(id).edges.completion.push(after(run, Point::Ended, shipped("19")));
            self.get(id).edges.completion.push(after(cancel, Point::Completed, shipped("19")));
        }
        r
    }

    /// The rows of each failure `i` can raise from `sources`: its
    /// delivery on its own path, the held alternative while the owner's
    /// params are open, and the recovery decision and restart when the
    /// owner's handler can restart it.
    fn failures(&mut self, i: usize, r: &mut Rows, sources: &[FailureSource]) {
        let Some((owner, restarts)) = self.route(i) else { return };
        let subjects = self.subjects;
        let s = &subjects[i];
        let pinned = self.is_pinned(i);
        let owner_own = self.subjects[owner].own;
        let owner_brackets = self.facts[owner].brackets;
        let field = s.how == How::Field;
        let own = s.own;
        let flow = matches!(s.how, How::Accepted { flow: true });
        let in_fn_main_cascade = field && {
            // A field of a template's top whose cascade is lowered in
            // `fn main`, where no locus is `self` (inventory C31).
            s.owner.is_some_and(|o| matches!(self.subjects[o].how, How::Top { .. }))
        };
        let present = self.facts[i].sources();
        for source in sources.iter().copied().filter(|src| present.contains(src)) {
            let (guard, raised_on, epoch) = match source {
                FailureSource::BirthClosure => (PathGuard::FailedInBirth, self.birth_domain(i), Some(Epoch::Birth)),
                FailureSource::BirthCheck => (PathGuard::FailedInBirth, self.birth_domain(i), Some(Epoch::Inline)),
                FailureSource::Run => (PathGuard::FailedInRun, own, Some(Epoch::Inline)),
                FailureSource::Handler => (PathGuard::FailedInRun, own, Some(Epoch::Inline)),
                FailureSource::Drain => (PathGuard::FailedInTeardown, if pinned { own } else { None }, Some(Epoch::Inline)),
                FailureSource::Dissolve => {
                    (PathGuard::FailedInTeardown, if pinned { own } else { None }, Some(Epoch::Dissolve))
                }
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
            // two are one domain (inventory C36 otherwise).
            let claim = match (raised_on, owner_own) {
                (Some(a), Some(b)) if a == b => Self::on(owner_own, shipped("L0-1")),
                (_, Some(_)) => Self::on(owner_own, open("L0-1", "C36")),
                _ => None,
            };
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
                status: if claim.is_some_and(|c| c.rule.status == Status::Shipped) {
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
                if matches!(source, FailureSource::Run | FailureSource::Drain | FailureSource::Dissolve)
                    && claim.is_some_and(|c| c.rule.status == Status::Shipped)
                {
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
                self.held(i, source, epoch, raised_on, owner, restarts, r);
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
        raised_on: Option<DomainId>,
        owner: usize,
        restarts: bool,
        r: &mut Rows,
    ) {
        let settling = self.subjects[owner].it;
        let owner_on_pool = self.is_pool(self.subjects[owner].own) && self.subjects[owner].own != settling;
        let rule = if owner_on_pool {
            Rule::line("1", Status::Pending { condition: POOL_OWNER })
        } else {
            shipped("1")
        };
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
        // raises on the settling thread.
        o.runs_on = if raised_on == settling { Self::on(settling, rule) } else { None };
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
        if let Some(settle) = self.rows.get(owner).and_then(|rows| rows.params_settle) {
            o.edges.completion.push(after(settle, Point::Completed, shipped("1")));
        }
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
            let resume_rule = if self.pool_placed(i) { open("13", "C43") } else { shipped("13") };
            let mut o = self.row(i, K::Resume, Holder { spine: Spine::Settle, domain: DomainRole::Instantiating });
            o.source = Some(source);
            o.guard = PathGuard::FailedAtSettle;
            o.line = Some("13");
            o.status = resume_rule.status;
            o.edges.entry.push(after(delivery, Point::Completed, shipped("1")));
            let resume = self.push(o);
            let own = self.subjects[i].own;
            let mut o = self.row(i, K::Run, Holder { spine: Spine::PoolRun, domain: DomainRole::Own });
            o.source = Some(source);
            o.guard = PathGuard::FailedAtSettle;
            o.multiplicity = Multiplicity::OncePerIncarnation;
            o.runs_on = if self.under_pinned(i) { None } else { Self::on(own, resume_rule) };
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

    /// The edges between an instance and its owner: born before the
    /// owner's birth, drained before its drain, dissolved after its
    /// dissolve, reclaimed before its reclaim; and a held failure
    /// delivered before the owner's birth.
    fn tree_edges(&mut self, i: usize) {
        let Some(o) = self.subjects[i].owner else { return };
        let child = self.rows[i].clone();
        let parent = self.rows[o].clone();
        let pinned = self.is_pinned(i);
        let parent_pinned = self.is_pinned(o);
        let field = self.subjects[i].how == How::Field;
        let restartable = self.route(i).is_some_and(|(_, restarts)| restarts);
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
        // stored, before the owner's birth (line 1).
        if let Some(pb) = parent.birth {
            let held: Vec<ObligationId> = child
                .deliveries
                .iter()
                .copied()
                .filter(|d| self.plan.obligations[d.0 as usize].guard == PathGuard::FailedAtSettle)
                .collect();
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
                (0..self.subjects.len()).filter(|&c| self.subjects[c].owner == Some(i) && self.subjects[c].how == How::Field).collect();
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
        // joining owner keeps completing its children's decisions.
        for i in 0..self.subjects.len() {
            let join = if self.is_pinned(i) {
                self.rows[i].pinned_join
            } else if self.is_pool(self.subjects[i].own) {
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
        // run abandoned, R20a, or a queued one its teardown cancels).
        if let Some(join) = first_join {
            for i in 0..self.subjects.len() {
                if !self.is_pool(self.subjects[i].own) {
                    continue;
                }
                if let Some(run) = self.rows[i].run {
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


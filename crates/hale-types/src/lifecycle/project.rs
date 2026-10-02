//! A plan rendered as what one run owes (F.40 phase 3, L1): the
//! obligations of the derived plan ([`super::derive`]) on one run's
//! path, as the trace oracle ([`super::trace::Expected`]) checks them.
//!
//! A plan is guarded; a run takes one path through it. [`RunPath`]
//! says which: the failures the run raises (where, whether the owner's
//! params were still open, whether the owner was in teardown, how many
//! restarts were performed), the runs a shutdown abandons parked, the
//! queued runs a teardown cancels before they start, how
//! many occurrences a body literal has (a literal in a body is
//! unbounded, and owed by none until the path counts it), whether the
//! run ends inside an obligation (the structural exit, or a hang), and
//! which declarations it holds (the subject under test, not what places
//! it). Those are facts of the run, not of the program: the producer
//! cannot know that a closure's assertion fails or that a failure lands
//! while the owner joins its pools.
//!
//! [`Focus`] says which decision lines' known-open rules the run is
//! held to. A rule shipped or adopted is always held; a pending one
//! never is (it is not guessed); a known-open one is held where its
//! line is in focus, so the fixture of that line pins today's
//! departure, and is left unchecked elsewhere, where it is that line's
//! fixture's to pin. An unchecked obligation is transparent: what
//! follows it is still owed. A process row (a spine's step, not an
//! instance's) is held where a line in focus binds its kind or orders
//! it against a held instance's row.
//!
//! What is rendered: the kinds the trace build records (spec/runtime.md
//! § The lifecycle trace; readiness, subscription, the run's admission,
//! the closures and the recovery decision have no events yet); one
//! line per declaration in the order its rows are owed, the process's
//! rows each on their own; a spine where the instance's own spine tears
//! it down and a line in focus is about that spine (the deferred entry,
//! 11; the run end's reclaim, 14); a domain where a held rule claims
//! one; every edge a held rule states between two rendered rows, except
//! an instance's plain program order, which its line states. The trace
//! names an instance by its declaration, so a row or an edge that only
//! some of a declaration's templates on the path owe is not held, nor an
//! edge between two declarations each with several occurrences (which
//! occurrence is built under which is not in the trace). A domain claim
//! is one domain, or the set a template's occurrences run on when they
//! are built under parents on different domains, each occurrence held
//! to one of them.

use std::collections::{BTreeMap, BTreeSet};

use super::trace::{Count, Expected, Owed};
use super::{
    DomainRole, Event, FailureSource, LifecyclePlan, Multiplicity, NotStarted, Obligation, ObligationId,
    ObligationKind as K, PathGuard, Point, Rule, Spine, Status, Terminal,
};
use crate::placement::{Bound, DomainKind, Template};

/// One run's path through a plan.
#[derive(Debug, Clone, Default)]
pub struct RunPath {
    pub failures: Vec<PathFailure>,
    /// How many instances of a declaration (its lowered name) the run
    /// builds, where the plan's bound does not say (a literal in a body
    /// is unbounded; one in a handler occurs only when the handler runs).
    pub occurrences: BTreeMap<String, u32>,
    /// Declarations whose started run a shutdown abandons parked
    /// (line 19, R20a).
    pub abandoned: BTreeSet<String>,
    /// Declarations whose run, queued behind its owner's teardown on the
    /// worker, that teardown cancels before it starts (line 19, the
    /// retention L5 shipped), with how many of their occurrences: every
    /// one, or some (a field template whose parents are on several
    /// domains, its run posted behind the teardown on one), whose runs
    /// then end either way.
    pub canceled: BTreeMap<String, u32>,
    /// The run ends inside this obligation: entered, never ended, and
    /// nothing that waits for its end is owed.
    pub ends_inside: Option<Inside>,
    /// The declarations the run is held to, by lowered name; `None` for
    /// every one. The others are on the path, unchecked: what places
    /// the subject under test, not what it tests.
    pub scope: Option<BTreeSet<String>>,
}

/// One failure of a run.
#[derive(Debug, Clone)]
pub struct PathFailure {
    /// The failing instances' declaration, by its lowered name.
    pub decl: String,
    pub source: FailureSource,
    /// Raised while the owner's params were open: held, delivered at
    /// settle (line 1).
    pub held: bool,
    /// Raised while the owner was in teardown: a restart is refused (RD).
    pub in_teardown: bool,
    /// Restarts the handler asked for and the runtime performed.
    pub restarts: u32,
}

/// An obligation a run ends inside.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inside {
    /// `None` for a process-level one.
    pub decl: Option<String>,
    pub kind: super::ObligationKind,
    /// The spine, for a process-level one.
    pub spine: Option<Spine>,
}

/// The decision lines whose known-open rules a run is held to.
#[derive(Debug, Clone, Copy)]
pub enum Focus<'f> {
    All,
    Lines(&'f [&'f str]),
}

impl Focus<'_> {
    /// Whether `line` is one the run is held to.
    fn names(self, line: &str) -> bool {
        match self {
            Focus::All => true,
            Focus::Lines(ls) => ls.contains(&line),
        }
    }

    fn holds(self, rule: Rule) -> bool {
        match rule.status {
            Status::Shipped | Status::Adopted => true,
            Status::Pending { .. } => false,
            Status::KnownOpen { .. } => match self {
                Focus::All => true,
                Focus::Lines(ls) => rule.line.is_some_and(|l| ls.contains(&l)),
            },
        }
    }
}

/// The kinds the trace build records.
pub const TRACED: &[super::ObligationKind] = &[
    K::ParamsSettle,
    K::ConstructionDelivery,
    K::Accept,
    K::Birth,
    K::Run,
    K::FailureDelivery,
    K::Restart,
    K::Drain,
    K::PreDrain,
    K::WaitAbort,
    K::PinnedJoin,
    K::PoolJoin,
    K::Cancellation,
    K::Dissolve,
    K::Reclaim,
];

/// How the trace notation counts a kind: per incarnation for a step
/// every incarnation repeats, else per instance.
pub fn trace_multiplicity(kind: super::ObligationKind) -> Multiplicity {
    match kind {
        K::Birth | K::Run | K::RunEnd | K::Closures => Multiplicity::OncePerIncarnation,
        _ => Multiplicity::OncePerInstance,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Not on this path.
    Off,
    /// Owed, and rendered.
    Owed,
    /// On the path, and not checked here: transparent.
    Unchecked,
    /// Waits for an event the run never reaches.
    Cut,
}

type Key = (Option<String>, super::ObligationKind, Option<Spine>);

/// What a run on `path` owes under `plan`, its known-open rules held
/// where `focus` says. An error names a failure the path raises that
/// the plan has no row for: the plan is missing it.
pub fn expected(plan: &LifecyclePlan, focus: Focus<'_>, path: &RunPath) -> Result<Expected, String> {
    let n = plan.obligations.len();
    let decl = |o: &Obligation| o.site.as_ref().map(|s| s.decl.lowered.clone());
    // 1. On the path.
    let mut on = vec![false; n];
    let mut selected: BTreeSet<(String, FailureSource)> = BTreeSet::new();
    for (id, o) in plan.iter() {
        on[id.0 as usize] = match &o.site {
            None => o.guard == PathGuard::Normal,
            Some(site) => {
                let l = &site.decl.lowered;
                let failure = |src: FailureSource| path.failures.iter().find(|f| &f.decl == l && f.source == src);
                let held_birth = path.failures.iter().any(|f| {
                    &f.decl == l
                        && f.held
                        && matches!(f.source, FailureSource::BirthClosure | FailureSource::BirthCheck)
                });
                match o.source {
                    None => match o.kind {
                        // The reclaim's cancellation of a queued run, or
                        // the pool worker's of a parked one.
                        K::Cancellation if o.holder.domain == DomainRole::Teardown => path.canceled.contains_key(l),
                        K::Cancellation => path.abandoned.contains(l),
                        K::Run => !held_birth,
                        _ => true,
                    },
                    Some(src) => match failure(src) {
                        None => false,
                        Some(f) => {
                            let follows_on = o
                                .edges
                                .entry
                                .iter()
                                .filter(|p| {
                                    matches!(plan.obligations[p.event.obligation.0 as usize].kind, K::FailureDelivery | K::RecoveryDecision)
                                })
                                .all(|p| on[p.event.obligation.0 as usize]);
                            let this_path = match (o.kind, o.guard) {
                                (K::FailureDelivery, _) => (o.guard == PathGuard::FailedAtSettle) == f.held,
                                (K::Restart, PathGuard::DrainInFlight) => f.in_teardown,
                                (_, PathGuard::Restart) => !f.in_teardown && f.restarts > 0,
                                _ => (o.guard == PathGuard::FailedAtSettle) == f.held,
                            };
                            let on_it = this_path && follows_on;
                            if on_it && o.kind == K::FailureDelivery {
                                selected.insert((l.clone(), src));
                            }
                            on_it
                        }
                    },
                }
            }
        };
    }
    for f in &path.failures {
        if !selected.contains(&(f.decl.clone(), f.source)) {
            return Err(format!(
                "the path fails {} at {} (held {}), and the plan has no delivery row for it",
                f.decl,
                f.source.name(),
                f.held
            ));
        }
    }
    // 2. Held or unchecked.
    let mut state: Vec<State> = plan
        .obligations
        .iter()
        .enumerate()
        .map(|(i, o)| {
            let in_scope = match (&path.scope, decl(o)) {
                (Some(scope), Some(l)) => scope.contains(&l),
                _ => true,
            };
            if !on[i] {
                State::Off
            } else if in_scope && TRACED.contains(&o.kind) && focus.holds(Rule { line: o.line, status: o.status }) {
                State::Owed
            } else {
                State::Unchecked
            }
        })
        .collect();
    // A declaration with no instance on this path owes nothing.
    let instances = |l: &str| -> Option<Count> {
        if let Some(k) = path.occurrences.get(l) {
            return Some(Count::Exactly(*k as usize));
        }
        // A template the plan bounds; a literal in a body (unbounded) or
        // in a handler occurs as often as the path says, and is owed by
        // none where it says nothing.
        let total: usize = plan
            .instances
            .iter()
            .filter(|i| i.site.decl.lowered == l && !i.in_handler)
            .map(|i| match i.bound {
                Bound::Once => 1,
                Bound::AtMost(k) => k as usize,
                Bound::Unbounded(_) => 0,
            })
            .sum();
        (total > 0).then_some(Count::Exactly(total))
    };
    for (i, o) in plan.obligations.iter().enumerate() {
        if let Some(l) = decl(o) {
            if matches!(instances(&l), None | Some(Count::Exactly(0))) {
                state[i] = State::Off;
            }
        }
    }
    // 3. What the run never reaches.
    let inside = |o: &Obligation| -> bool {
        path.ends_inside.as_ref().is_some_and(|x| {
            x.decl == decl(o) && x.kind == o.kind && (o.site.is_some() || x.spine == Some(o.holder.spine))
        })
    };
    loop {
        let mut changed = false;
        for i in 0..n {
            if state[i] != State::Owed {
                continue;
            }
            let o = &plan.obligations[i];
            let cut = o.edges.entry.iter().filter(|p| focus.holds(p.rule)).any(|p| {
                let j = p.event.obligation.0 as usize;
                match state[j] {
                    State::Cut => true,
                    State::Owed => inside(&plan.obligations[j]) && p.event.point != Point::Entered,
                    State::Off | State::Unchecked => false,
                }
            });
            if cut {
                state[i] = State::Cut;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    // The trace names an instance by its declaration, not its template,
    // so a row is owed per declaration: where a declaration has several
    // templates on the path (a field and an accepted child of one locus),
    // a row only some of them owe is not held, and an edge is held only
    // where every template of each of its declarations states it.
    let mut templates: BTreeMap<String, BTreeSet<&Template>> = BTreeMap::new();
    let mut owing: BTreeMap<(String, super::ObligationKind), BTreeSet<&Template>> = BTreeMap::new();
    for (i, o) in plan.obligations.iter().enumerate() {
        let Some(site) = &o.site else { continue };
        if matches!(state[i], State::Owed | State::Unchecked) {
            templates.entry(site.decl.lowered.clone()).or_default().insert(&site.template);
        }
        if state[i] == State::Owed {
            owing.entry((site.decl.lowered.clone(), o.kind)).or_default().insert(&site.template);
        }
    }
    let every_template = |l: &str, of: &BTreeSet<&Template>| templates.get(l).is_none_or(|all| all.is_subset(of));
    for (i, o) in plan.obligations.iter().enumerate() {
        let Some(site) = &o.site else { continue };
        let l = &site.decl.lowered;
        if state[i] == State::Owed && !every_template(l, &owing[&(l.clone(), o.kind)]) {
            state[i] = State::Unchecked;
        }
    }
    // A process row is a spine's step, not an instance's: it is held where
    // a line in focus is about it, or orders it against an instance.
    let about = |line: Option<&str>| line.is_some_and(|l| focus.names(l));
    let bound_here = |kind: super::ObligationKind| {
        super::DECISION_LINES.iter().any(|l| focus.names(l.line) && l.kinds.contains(&kind))
    };
    let mut linked: BTreeSet<usize> = BTreeSet::new();
    for (i, o) in plan.obligations.iter().enumerate() {
        if state[i] != State::Owed {
            continue;
        }
        for p in o.edges.entry.iter().chain(&o.edges.completion) {
            let j = p.event.obligation.0 as usize;
            if state[j] != State::Owed || !about(p.rule.line) || !focus.holds(p.rule) {
                continue;
            }
            match (o.site.is_some(), plan.obligations[j].site.is_some()) {
                (false, true) => {
                    linked.insert(i);
                }
                (true, false) => {
                    linked.insert(j);
                }
                _ => {}
            }
        }
    }
    for (i, o) in plan.obligations.iter().enumerate() {
        if o.site.is_none() && state[i] == State::Owed && !bound_here(o.kind) && !linked.contains(&i) {
            state[i] = State::Unchecked;
        }
    }
    // 4. Rendered.
    // An instance torn down by a spine of its own names it where a line
    // in focus is about that spine: the deferred entry a let-bound
    // literal drains in (11), the reclaim a flow's run end runs (14).
    let own_spine: BTreeMap<String, Spine> = plan
        .obligations
        .iter()
        .filter(|o| {
            o.kind == K::Drain
                && match o.holder.spine {
                    Spine::DeferredEntry => focus.names("11"),
                    Spine::Reclaim => focus.names("14"),
                    _ => false,
                }
        })
        .filter_map(|o| decl(o).map(|l| (l, o.holder.spine)))
        .collect();
    let key = |o: &Obligation| -> Key {
        let spine = match decl(o) {
            None => Some(o.holder.spine),
            Some(l) if matches!(o.kind, K::Drain | K::Dissolve | K::Reclaim) => own_spine.get(&l).copied(),
            Some(_) => None,
        };
        (decl(o), o.kind, spine)
    };
    let label = |d: crate::placement::DomainId| -> String {
        match &plan.domains[d.0 as usize].kind {
            DomainKind::Main => "main".to_string(),
            DomainKind::Pool { name, .. } => format!("pool:{name}"),
            DomainKind::Pinned { .. } => "pinned".to_string(),
        }
    };
    let mut exp = Expected::default();
    let mut index: BTreeMap<Key, usize> = BTreeMap::new();
    let mut of: Vec<Option<usize>> = vec![None; n];
    let mut claims: BTreeMap<usize, BTreeSet<Option<Vec<String>>>> = BTreeMap::new();
    let mut process_rows: BTreeMap<usize, usize> = BTreeMap::new();
    let mut lines: Vec<(Option<String>, Vec<usize>)> = Vec::new();
    for (i, o) in plan.obligations.iter().enumerate() {
        if state[i] != State::Owed {
            continue;
        }
        // A row whose every terminal is a not-started one is owed by no
        // subject: the plan says it is not performed (a restart refused
        // under teardown, a run a resumed locus does not declare).
        let never_started = !o.terminals.is_empty() && o.terminals.iter().all(|t| matches!(t, Terminal::NotStarted(_)));
        let k = key(o);
        let at = match index.get(&k) {
            Some(&at) => at,
            None => {
                let failure = decl(o).and_then(|l| path.failures.iter().find(|f| f.decl == l && Some(f.source) == o.source));
                let count = match &k.0 {
                    None => Count::Exactly(0),
                    Some(l) => {
                        let base = instances(l).unwrap_or(Count::Exactly(0));
                        let restarts = path.failures.iter().filter(|f| &f.decl == l).map(|f| f.restarts).sum::<u32>() as usize;
                        // A teardown's cancellation, once per run it cancels.
                        let canceled = path.canceled.get(l).filter(|_| o.holder.domain == DomainRole::Teardown);
                        match (o.kind, base) {
                            _ if never_started => Count::Exactly(0),
                            (K::Cancellation, _) if canceled.is_some() => Count::Exactly(canceled.map_or(0, |&k| k as usize)),
                            (K::Restart, Count::Exactly(b)) => Count::Exactly(b * failure.map_or(0, |f| f.restarts as usize)),
                            (K::Birth | K::Run, Count::Exactly(b)) => Count::Exactly(b * (1 + restarts)),
                            (_, c) => c,
                        }
                    }
                };
                let ends = match o.kind {
                    K::Run if decl(o).is_some_and(|l| path.abandoned.contains(&l)) => {
                        Point::Terminal(Terminal::CanceledAfterStart)
                    }
                    // Every occurrence's queued run canceled, or some: each
                    // ends completed or not started.
                    K::Run if decl(o).is_some_and(|l| path.canceled.contains_key(&l)) => {
                        let l = decl(o).expect("a declaration");
                        if instances(&l) == Some(Count::Exactly(path.canceled[&l] as usize)) {
                            Point::Terminal(Terminal::NotStarted(NotStarted::Acknowledged))
                        } else {
                            Point::Ended
                        }
                    }
                    _ => Point::Completed,
                };
                let at = exp.owed.len();
                exp.owed.push(Owed {
                    decl: k.0.clone(),
                    kind: o.kind,
                    spine: k.2,
                    multiplicity: trace_multiplicity(o.kind),
                    count,
                    ends,
                    domain: None,
                });
                index.insert(k.clone(), at);
                match lines.iter_mut().find(|(d, _)| d.is_some() && *d == k.0) {
                    Some((_, seq)) => seq.push(at),
                    None => lines.push((k.0.clone(), vec![at])),
                }
                at
            }
        };
        if k.0.is_none() {
            *process_rows.entry(at).or_insert(0) += 1;
        }
        of[i] = Some(at);
        let claim = o.runs_on.as_ref().filter(|r| focus.holds(r.rule)).map(|r| {
            let labels: BTreeSet<String> = r.domains.iter().map(|&d| label(d)).collect();
            labels.into_iter().collect::<Vec<String>>()
        });
        claims.entry(at).or_default().insert(claim);
    }
    for (at, rows) in process_rows {
        exp.owed[at].count = Count::Exactly(rows);
    }
    for (at, cs) in claims {
        if cs.len() == 1 {
            exp.owed[at].domain = cs.into_iter().next().flatten();
        }
    }
    exp.sequences = lines.into_iter().map(|(_, seq)| seq.into_iter().map(|a| ObligationId(a as u32)).collect()).collect();
    // Edges.
    type Stating<'p> = (BTreeSet<&'p Template>, BTreeSet<&'p Template>);
    let mut edges: BTreeMap<(Event, Event), Stating<'_>> = BTreeMap::new();
    for (i, o) in plan.obligations.iter().enumerate() {
        let Some(b) = of[i] else { continue };
        for (prereqs, point) in [(&o.edges.entry, Point::Entered), (&o.edges.completion, Point::Completed)] {
            for p in prereqs {
                let j = p.event.obligation.0 as usize;
                let Some(a) = of[j] else { continue };
                if !focus.holds(p.rule) || a == b {
                    continue;
                }
                let program_order = point == Point::Entered
                    && p.rule.line.is_none()
                    && p.event.point == Point::Completed
                    && plan.obligations[j].site == o.site
                    && o.site.is_some();
                if program_order {
                    continue;
                }
                let stating = edges
                    .entry((
                        Event { obligation: ObligationId(a as u32), point: p.event.point },
                        Event { obligation: ObligationId(b as u32), point },
                    ))
                    .or_default();
                stating.0.extend(plan.obligations[j].site.as_ref().map(|s| &s.template));
                stating.1.extend(o.site.as_ref().map(|s| &s.template));
            }
        }
    }
    let held_by_every = |at: usize, by: &BTreeSet<&Template>| exp.owed[at].decl.as_deref().is_none_or(|l| every_template(l, by));
    // An edge between two declarations' instances ties each occurrence to
    // the one it is built under, and the trace holds it for every
    // occurrence of the first before any of the second: the same thing
    // only where one side has a single occurrence. With several on both
    // (a field reached under several constructions of its owner), the
    // trace cannot tell which occurrence is whose, and the edge is not
    // held.
    let single = |at: usize| exp.owed[at].decl.as_deref().is_none_or(|l| instances(l) == Some(Count::Exactly(1)));
    let tied = |a: usize, b: usize| {
        let (da, db) = (&exp.owed[a].decl, &exp.owed[b].decl);
        da.is_none() || db.is_none() || da == db || single(a) || single(b)
    };
    exp.edges = edges
        .into_iter()
        .filter(|((a, b), (by_a, by_b))| {
            let (a, b) = (a.obligation.0 as usize, b.obligation.0 as usize);
            held_by_every(a, by_a) && held_by_every(b, by_b) && tied(a, b)
        })
        .map(|(edge, _)| edge)
        .collect();
    Ok(exp)
}

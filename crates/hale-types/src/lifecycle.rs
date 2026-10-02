//! The `lifecycle_order` family (F.40 phase 3, L1): the lifecycle as a
//! table of obligations.
//!
//! This module fixes what a row says, so the producer, the trace oracle
//! (L2), the matrix (L3) and emission (L4) are written against one
//! shape. The producer is [`derive::derive_lifecycle`], one plan per
//! snapshot (`Snapshot::demand_lifecycle`), over P1's placement table,
//! whose identities this schema shares, the handler rows, the flow rows
//! and the bus graph. [`project::expected`] renders a plan as what one
//! run owes, the form the trace oracle ([`trace::Expected`]) checks.
//! [`spine`] is the emitters' reader: one spine's obligations for one
//! instance template, in the order the plan places them.
//!
//! An obligation is something the compiler emits or the runtime
//! performs that some domain owes some instance: params settle, a held
//! failure's delivery, accept, readiness, birth, a run's admission and
//! execution, a closure epoch, a failure's delivery to the owner, a
//! recovery decision and its execution, drain, the pre-drain, the
//! wait-abort, a join and the progress it owes, a cancellation,
//! teardown delivery, dissolve, the reclaim. The inventory
//! (`notes/f40-lifecycle-inventory.md`, rows C1–C48, R1–R49 and R20a)
//! is the list of those actions as the code performs them;
//! [`ObligationKind`] names each one once, and [`ObligationKind::rows`]
//! points back at the rows it stands for.
//!
//! ## Two identities
//!
//! A row is keyed by its **source site** ([`SourceSite`]): the declaration
//! that is built ([`DeclRef`], a generic's specialization by its
//! substitution) and the construction template that builds it
//! ([`Template`]). The template is P1's static key (hale-lang/hale#1296,
//! `notes/f40-placement-correspondence.md` § 1): an origin, a path of
//! fields with the literal taken at each guarded step, a replica index.
//! A literal outside the static tower (a method body, a let-bound
//! literal, an accepted child) is its literal site, as P1's dynamic
//! sites are. Both are placement's own types: every site a row names is
//! a [`SiteRef`], carrying the universe that minted it (the snapshot's,
//! or the stdlib analysis copy's). A process-level row (a pool join, a
//! wait-abort, a pre-drain) has no source site: a spine owes it the
//! process.
//!
//! The runtime has the second level: which live object, and which
//! incarnation of it ([`RuntimeSubject`]). An instance is one execution
//! of a template's literal; each `restart` begins its next incarnation.
//! The runtime mints both numbers and the table never does: a plan row
//! speaks of a template and is owed once per instance or once per
//! incarnation ([`Multiplicity`]), and a trace event (L2) carries the
//! runtime subject beside its template so the oracle can join the two
//! ([`Occurrence`]). [`RuntimeInstance`] and [`Incarnation`] have no
//! constructor but [`RuntimeInstance::observed`] and
//! [`Incarnation::observed`], which read what the runtime emitted.
//!
//! ## Edges, terminals, lifetime and progress
//!
//! Order is stated on events, never on calls ([`Event`]: an obligation
//! entered, completed, or ended in one of its terminals). A row has
//! entry edges (what happened before it is entered) and completion
//! edges (what happened before it completes). An action that runs
//! inside another (R32 inside R31, R36 inside R29, R42 inside R32, C43
//! inside R5) has both: an entry edge on its enclosing action's
//! entry, and a completion edge from its own completion into the
//! enclosing action's. A call relationship is never read as a
//! completion edge (the inventory's warning above its table).
//!
//! Every obligation ends in exactly one of its named terminal
//! alternatives ([`Terminal`]): completed; not started, with a shutdown
//! reason or with an acknowledgement; canceled after start, with
//! quiescence witnessed by a separate obligation; or, where the
//! inventory names them, failure delivered, closure violation, or
//! dissolved. No path ends silently.
//!
//! Lifetime and progress are separate fields. [`Retention`] says what
//! must stay alive until which event (the child and the copied
//! violation until the handler completes). [`Progress`] says what
//! makes the obligation reach a terminal at all (the owner keeps
//! completing failure decisions while it joins). A row can keep its
//! lifetime and still owe progress it does not make, which is the
//! join-progress defect: each of the two carries its own [`Status`].
//!
//! ## Adopted and shipped
//!
//! Each row, each retention and each progress rule carries a
//! [`Status`]: [`Status::Shipped`] when the code does what the rule
//! says, [`Status::Adopted`] when the rule is decided and nothing
//! today contradicts it, [`Status::KnownOpen`] when the rule is
//! decided and today's behaviour differs at a named inventory row, and
//! [`Status::Pending`] when the rule itself waits on a named condition.
//! The fixtures under `crates/hale-codegen/tests/fixtures/lifecycle/`
//! pin today's outcome of each `KnownOpen` row a program can show
//! (`lifecycle_fixtures.rs`, its `KNOWN_OPEN` table). Two no program
//! can show, the eager spine's missing pre-drain (line 18), which a
//! body's exit flush covers, and the domain a handler runs on (join
//! progress); the trace build ([`trace`], L2) shows both, and the same
//! file's `TRACE_KNOWN_OPEN` table pins them.
//!
//! ## The decision lines
//!
//! The inventory's nineteen decision lines, the two adopted
//! requirements beside them (restart during drain, join progress) and
//! the kinds each one binds. [`DECISION_LINES`] is the same table as
//! data, and a test holds the two equal.
//!
//! ```text
//! line  kinds                                    status
//! 1     ConstructionDelivery ParamsSettle        Shipped; Pending (pool-placed owner)
//! 2     Closures Run                             Pending (no option chosen)
//! 3     Accept Birth Run Dissolve                Pending (no option chosen); Shipped (C12, L4)
//! 4     FailureDelivery Reclaim                  KnownOpen C25; KnownOpen C31
//! 5     Accept                                   Shipped
//! 6     Subscribe Readiness                      Shipped; Shipped (L4)
//! 7     WaitAbort PoolJoin                       KnownOpen R34
//! 8     FailureDelivery Birth                    Shipped
//! 9     FailureDelivery Closures                 Shipped
//! 10    Closures Dissolve                        Shipped
//! 11    Drain                                    Shipped
//! 12    Drain                                    KnownOpen C9; KnownOpen C32
//! 13    Resume RunAdmission Run                  KnownOpen C43; KnownOpen C48
//! 14    Reclaim                                  Shipped; Shipped (L2 verifies)
//! 15    ProcessDrain                             Shipped
//! 16    PoolJoin WaitAbort                       Pending (P3's capability matrix)
//! 17    PinnedJoin TeardownDelivery              Pending (teardown delivery contract)
//! 18    PreDrain                                 KnownOpen C13
//! 19    RunAdmission Run Cancellation            Shipped (retention, L5); KnownOpen R19 (refused or freed unrun); Shipped (R20a, named by L2)
//! RD    RecoveryDecision Restart                 Shipped (process drain); KnownOpen C42 (owner teardown)
//! JP    JoinProgress FailureDelivery             KnownOpen C18; KnownOpen R20
//! ```
//!
//! **Pending, and why.** Line 16 waits for P3: the obligations a target
//! without threads owes come from the capability matrix, and gating
//! the eager spine on wasm is an interim correction, not the rule.
//! Line 17 prefers the deferred spine's join order everywhere, on the
//! condition that the teardown delivery contract's final-publish
//! guarantees (GH #253) survive every eager, deferred and declaration
//! permutation; it is settled only once that is shown. Line 1's
//! pool-placed owner subcase waits for the construction-time domain:
//! decision L0-1 names the pool's worker as that owner's domain, and
//! `spec/semantics.md` names the thread settling the parent. Lines 2
//! and 3 have no option chosen: the wave-2 decisions treat lines 1–3
//! as one protocol (construction, readiness and failure delivery,
//! settled by events) without choosing (a) or (b) for the post-run
//! tick on a posted `run()` or for where lifecycle methods run on a
//! pool, so their rows record the shipped domains and wait.

pub mod derive;
pub mod project;
pub mod spine;
pub mod trace;

// ------------------------------------------------------------ identity

/// P1's identities, shared: a row is keyed by the placement table's
/// declaration and template, every site a [`SiteRef`] carrying the
/// universe that minted it.
pub use crate::placement::{DeclRef, InstanceKey, Origin, SiteRef, Step, Template};

/// A row's static identity: the declaration built and the template
/// that builds it. Two specializations of one generic share
/// [`DeclRef::site`] and differ in [`DeclRef::lowered`], so they are
/// two source sites with two sets of rows.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceSite {
    pub decl: DeclRef,
    pub template: Template,
}

/// The runtime level: one live object and one incarnation of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RuntimeSubject {
    pub instance: RuntimeInstance,
    pub incarnation: Incarnation,
}

/// A live instance, numbered by the runtime. The table never mints
/// one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RuntimeInstance(u64);

impl RuntimeInstance {
    /// The number a trace event carried. Reading the runtime's numbers
    /// is the only way to hold one.
    pub fn observed(raw: u64) -> RuntimeInstance {
        RuntimeInstance(raw)
    }

    pub fn raw(self) -> u64 {
        self.0
    }
}

/// An incarnation of one instance: 0 from its first `birth()`, the
/// next at each `restart` / `restart_in_place` the runtime performs. A
/// restart requested and not performed (restart during drain) begins
/// none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Incarnation(u32);

impl Incarnation {
    /// The number a trace event carried.
    pub fn observed(raw: u32) -> Incarnation {
        Incarnation(raw)
    }

    pub fn raw(self) -> u32 {
        self.0
    }
}

impl RuntimeSubject {
    /// The subject a trace event carried: the runtime's instance and
    /// incarnation numbers, read back. `trace::parse_line` is the one
    /// caller; nothing else mints a subject.
    pub fn observed(instance: u64, incarnation: u32) -> RuntimeSubject {
        RuntimeSubject { instance: RuntimeInstance::observed(instance), incarnation: Incarnation::observed(incarnation) }
    }
}

/// A runtime subject joined to the source site it is an instance of:
/// what the trace oracle builds from an event, and the only place the
/// two levels meet.
#[derive(Debug, Clone, PartialEq)]
pub struct Occurrence {
    pub site: SourceSite,
    pub subject: RuntimeSubject,
}

// ------------------------------------------------------------ the rows

/// One obligation of the plan.
#[derive(Debug, Clone, PartialEq)]
pub struct Obligation {
    /// The instance template that owes it; `None` for a process-level
    /// obligation (a pool join, a wait-abort, a pre-drain, the process
    /// drain), which a spine owes the process.
    pub site: Option<SourceSite>,
    pub kind: ObligationKind,
    /// For [`ObligationKind::Closures`] and a failure raised by one:
    /// the epoch.
    pub epoch: Option<Epoch>,
    /// For a failure's rows (its delivery, the construction-time
    /// delivery, the recovery decision, the restart, the resume): which
    /// failure of the instance they follow.
    pub source: Option<FailureSource>,
    /// The paths on which the obligation exists. A child that fails
    /// at params settle never reaches `run()`; a restart repeats the
    /// guarded subsequence; no row is unconditional over every path.
    pub guard: PathGuard,
    pub holder: Holder,
    /// The domain the holder's role resolves to in this deployment,
    /// with the rule that says so; `None` where the role does not
    /// resolve to one domain (the events span two, the domain is a
    /// hole, or the rule is pending).
    pub runs_on: Option<RunsOn>,
    pub edges: Edges,
    /// The named terminal alternatives, every one this obligation can
    /// end in. A path ends in exactly one.
    pub terminals: Vec<Terminal>,
    pub multiplicity: Multiplicity,
    pub lifetime: Vec<Retention>,
    pub progress: Progress,
    /// The decision line whose rule makes the obligation exist on its
    /// guard's path, and [`Obligation::status`] is that rule's status;
    /// `None` for an action no line is about (the lifecycle the spec
    /// has always stated).
    pub line: Option<&'static str>,
    pub status: Status,
}

/// What raised the failure a row follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FailureSource {
    /// A birth-epoch closure.
    BirthClosure,
    /// The `birth_check`.
    BirthCheck,
    /// A `violate` in `run()`, or a tick or duration closure after it.
    Run,
    /// A `violate` in a bus handler, or a closure evaluated after one.
    Handler,
    /// A `violate` in `drain()`.
    Drain,
    /// A dissolve-epoch closure.
    Dissolve,
}

impl FailureSource {
    pub const ALL: &'static [FailureSource] = &[
        FailureSource::BirthClosure,
        FailureSource::BirthCheck,
        FailureSource::Run,
        FailureSource::Handler,
        FailureSource::Drain,
        FailureSource::Dissolve,
    ];

    pub fn name(self) -> &'static str {
        match self {
            FailureSource::BirthClosure => "BirthClosure",
            FailureSource::BirthCheck => "BirthCheck",
            FailureSource::Run => "Run",
            FailureSource::Handler => "Handler",
            FailureSource::Drain => "Drain",
            FailureSource::Dissolve => "Dissolve",
        }
    }
}

/// A resolved domain claim: every event of the obligation runs on this
/// domain, by this rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunsOn {
    pub domain: crate::placement::DomainId,
    pub rule: Rule,
}

/// The rule an edge or a claim states, and its status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rule {
    /// The decision line, or `None` for an order no line is about.
    pub line: Option<&'static str>,
    pub status: Status,
}

impl Rule {
    /// An order the code keeps and no decision line is about.
    pub const SHIPPED: Rule = Rule { line: None, status: Status::Shipped };

    pub const fn line(line: &'static str, status: Status) -> Rule {
        Rule { line: Some(line), status }
    }
}

/// An obligation's position in its plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObligationId(pub u32);

/// One snapshot's plan: every obligation, indexed by [`ObligationId`],
/// the instance templates that owe them, and the placement table's
/// domains its claims name.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LifecyclePlan {
    pub obligations: Vec<Obligation>,
    /// Every instance template a row names, once, in the order the
    /// producer visited them.
    pub instances: Vec<Instance>,
    /// The placement table's domains, indexed by `DomainId`.
    pub domains: Vec<crate::placement::Domain>,
}

/// One instance template of a plan.
#[derive(Debug, Clone, PartialEq)]
pub struct Instance {
    pub site: SourceSite,
    /// How many occurrences of it can be live at once (P1's bound on
    /// its template or its site).
    pub bound: crate::placement::Bound,
    /// Built in an `on_failure` body: it occurs only on a path where the
    /// handler runs.
    pub in_handler: bool,
}

impl LifecyclePlan {
    pub fn get(&self, id: ObligationId) -> Option<&Obligation> {
        self.obligations.get(id.0 as usize)
    }

    /// Every obligation with its id.
    pub fn iter(&self) -> impl Iterator<Item = (ObligationId, &Obligation)> {
        self.obligations.iter().enumerate().map(|(i, o)| (ObligationId(i as u32), o))
    }
}

/// What an obligation is: one name per lifecycle action the inventory
/// lists, never one per emission site.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ObligationKind {
    /// The params bracket: opened before the first field is built,
    /// settled once the last is stored. Settlement is the event the
    /// held deliveries wait for.
    ParamsSettle,
    /// Construction-time delivery: a failure held while the owner's
    /// params are open is delivered at settle, in arrival order,
    /// before the owner's `birth()`; its child waits for the decision.
    ConstructionDelivery,
    /// `accept(c)`: after the child's params, before its birth; it
    /// cannot reject.
    Accept,
    /// Subscription registration, before `birth()`.
    Subscribe,
    /// Delivery to a new instance becomes eligible; what was published
    /// to it before then is retained, never dropped.
    Readiness,
    /// `birth()`, and its `birth_check`.
    Birth,
    /// A run's admission: attempted by the caller, then admitted or
    /// rejected by the domain that would run it. Attempted is the
    /// caller's knowledge, never a third outcome.
    RunAdmission,
    /// `run()`'s execution, from start to return.
    Run,
    /// The run end: await phase 0, the restart loop, the per-child
    /// reclaim decision (a kept failed child stays, GH #1069).
    RunEnd,
    /// A closure epoch's evaluation (see [`Epoch`]).
    Closures,
    /// A failure's delivery: raised by a closure, `birth_check` or
    /// `violate`, completed when the owner's handler returns on the
    /// owner's domain (decision L0-1).
    FailureDelivery,
    /// The handler's recovery decision (restart, quarantine, bubble,
    /// absorb), recorded apart from its execution.
    RecoveryDecision,
    /// A restart's execution: `birth()` again on the same instance,
    /// beginning the next incarnation.
    Restart,
    /// The resume after a held handler: restart, start `run()`, or the
    /// run end, through the same placement and admission as a first
    /// run.
    Resume,
    /// `drain()`, with the owned fields' drains before it.
    Drain,
    /// The bus drain a teardown spine runs before its first entry.
    PreDrain,
    /// Waking every `or wait` that only teardown ends.
    WaitAbort,
    /// Joining an owned pinned child: mailbox shutdown, the join, the
    /// mailbox's destroy.
    PinnedJoin,
    /// Joining every cooperative pool's worker.
    PoolJoin,
    /// The owner keeps completing outstanding failure decisions while
    /// it joins, until the children it waits for have quiesced.
    JoinProgress,
    /// A deliberate abandonment at shutdown: a parked coroutine's
    /// stack freed without resuming it.
    Cancellation,
    /// The teardown delivery contract: a result in flight to a
    /// subscriber is delivered before the subscriber is torn down, or
    /// discarded through deregister-on-dissolve, never dispatched to
    /// freed memory.
    TeardownDelivery,
    /// The dissolve-epoch closures, then the user's `dissolve()`.
    Dissolve,
    /// The reclaim: the instance's teardown spine and its arena's
    /// release, exactly once (the `__arena` latch).
    Reclaim,
    /// The whole-process drain a signal begins: a cooperative flag,
    /// never a lifecycle call from the signal path.
    ProcessDrain,
}

impl ObligationKind {
    /// Every kind, in declaration order.
    pub const ALL: &'static [ObligationKind] = &[
        ObligationKind::ParamsSettle,
        ObligationKind::ConstructionDelivery,
        ObligationKind::Accept,
        ObligationKind::Subscribe,
        ObligationKind::Readiness,
        ObligationKind::Birth,
        ObligationKind::RunAdmission,
        ObligationKind::Run,
        ObligationKind::RunEnd,
        ObligationKind::Closures,
        ObligationKind::FailureDelivery,
        ObligationKind::RecoveryDecision,
        ObligationKind::Restart,
        ObligationKind::Resume,
        ObligationKind::Drain,
        ObligationKind::PreDrain,
        ObligationKind::WaitAbort,
        ObligationKind::PinnedJoin,
        ObligationKind::PoolJoin,
        ObligationKind::JoinProgress,
        ObligationKind::Cancellation,
        ObligationKind::TeardownDelivery,
        ObligationKind::Dissolve,
        ObligationKind::Reclaim,
        ObligationKind::ProcessDrain,
    ];

    /// The kind a trace line names (the inverse of [`ObligationKind::name`]).
    pub fn from_name(name: &str) -> Option<ObligationKind> {
        ObligationKind::ALL.iter().copied().find(|k| k.name() == name)
    }

    pub fn name(self) -> &'static str {
        match self {
            ObligationKind::ParamsSettle => "ParamsSettle",
            ObligationKind::ConstructionDelivery => "ConstructionDelivery",
            ObligationKind::Accept => "Accept",
            ObligationKind::Subscribe => "Subscribe",
            ObligationKind::Readiness => "Readiness",
            ObligationKind::Birth => "Birth",
            ObligationKind::RunAdmission => "RunAdmission",
            ObligationKind::Run => "Run",
            ObligationKind::RunEnd => "RunEnd",
            ObligationKind::Closures => "Closures",
            ObligationKind::FailureDelivery => "FailureDelivery",
            ObligationKind::RecoveryDecision => "RecoveryDecision",
            ObligationKind::Restart => "Restart",
            ObligationKind::Resume => "Resume",
            ObligationKind::Drain => "Drain",
            ObligationKind::PreDrain => "PreDrain",
            ObligationKind::WaitAbort => "WaitAbort",
            ObligationKind::PinnedJoin => "PinnedJoin",
            ObligationKind::PoolJoin => "PoolJoin",
            ObligationKind::JoinProgress => "JoinProgress",
            ObligationKind::Cancellation => "Cancellation",
            ObligationKind::TeardownDelivery => "TeardownDelivery",
            ObligationKind::Dissolve => "Dissolve",
            ObligationKind::Reclaim => "Reclaim",
            ObligationKind::ProcessDrain => "ProcessDrain",
        }
    }

    /// The inventory rows the kind stands for.
    pub fn rows(self) -> &'static [&'static str] {
        match self {
            ObligationKind::ParamsSettle => &["C3", "C5", "C44", "R1", "R5"],
            ObligationKind::ConstructionDelivery => &["C11", "R2", "R3", "R4", "R5"],
            ObligationKind::Accept => &["C2", "C7", "R6"],
            ObligationKind::Subscribe => &["C8"],
            ObligationKind::Readiness => &["C8", "C10"],
            ObligationKind::Birth => &["C1", "C9", "C10", "C38", "R9", "R11", "R12", "R46"],
            ObligationKind::RunAdmission => &["C12", "R17", "R18", "R19"],
            ObligationKind::Run => &["C9", "C12", "C48", "R24", "R25"],
            ObligationKind::RunEnd => &["C26", "R7"],
            ObligationKind::Closures => &["C37", "C40"],
            ObligationKind::FailureDelivery => &["C6", "C34", "C35", "C36", "C38", "C39", "C46", "R36"],
            ObligationKind::RecoveryDecision => &["C45", "C47"],
            ObligationKind::Restart => &["C41", "C42", "R38", "R48"],
            ObligationKind::Resume => &["C43", "C48"],
            ObligationKind::Drain => &["C9", "C14", "C30", "C32", "R44"],
            ObligationKind::PreDrain => &["C16", "R29"],
            ObligationKind::WaitAbort => &["C17", "R34"],
            ObligationKind::PinnedJoin => &["C13", "C16", "C18", "R26", "R27"],
            ObligationKind::PoolJoin => &["C13", "C19", "C21", "C22", "C23", "R20"],
            ObligationKind::JoinProgress => &["C18", "R20"],
            ObligationKind::Cancellation => &["R19", "R20a", "R21"],
            ObligationKind::TeardownDelivery => &["C16", "R33", "R35"],
            ObligationKind::Dissolve => &["C31", "C32"],
            ObligationKind::Reclaim => &["C15", "C24", "C25", "C27", "C28", "C29", "C33", "R8", "R10", "R13", "R14", "R47"],
            ObligationKind::ProcessDrain => &["R43"],
        }
    }
}

/// A closure epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Epoch {
    Birth,
    Tick,
    Duration,
    Inline,
    Dissolve,
}

/// The path an obligation exists on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PathGuard {
    /// No failure on the way.
    Normal,
    /// The instance failed while its owner's params were open: the
    /// failure is held, and its `run()` waits for the decision.
    FailedAtSettle,
    /// The instance failed during `birth()`.
    FailedInBirth,
    /// The instance failed in `run()` or after a handler.
    FailedInRun,
    /// The instance failed in its own teardown: in `drain()`, or in a
    /// dissolve-epoch closure.
    FailedInTeardown,
    /// A restart was performed: the guarded subsequence repeats in
    /// the next incarnation.
    Restart,
    /// The owner is in teardown, or the process drains.
    DrainInFlight,
    /// Any path.
    Any,
}

/// Who owes the obligation: which spine runs it, on which domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Holder {
    pub spine: Spine,
    pub domain: DomainRole,
}

/// The emitted sequence an obligation is discharged in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Spine {
    /// The instantiation: params, accept, subscribe, birth, run start
    /// (C1–C12).
    Instantiation,
    /// The posted run's wrapper and its run end (C26).
    PoolRun,
    /// A pinned locus's thread function (C9).
    PinnedMain,
    /// Teardown spine 1: a statement-position literal (C13).
    EagerTeardown,
    /// The per-entry teardown of a deferred frame entry (C18).
    DeferredEntry,
    /// Teardown spine 2: the deferred main-locus entry (C19).
    DeferredMainEntry,
    /// Teardown spine 3: `fn main` falls through (C21).
    MainFallThrough,
    /// Teardown spine 4: `fn main`'s test-failure exit (C22).
    MainTestFailure,
    /// Teardown spine 5: `return` from `fn main` (C23).
    MainReturn,
    /// The reclaim spine (C25), from a run end, a handler's
    /// `terminate`, settle, a cascade or a reassignment.
    Reclaim,
    /// The owned-field cascades (C30, C31, C32) and the accepted
    /// children's (C28).
    Cascade,
    /// A held failure's settle and its resume (R5, C43).
    Settle,
    /// The owner's queue drain, where handlers run (R29, R36).
    QueueDrain,
    /// The process's start and exit tail (C20, C24).
    Process,
}

impl Spine {
    pub const ALL: &'static [Spine] = &[
        Spine::Instantiation,
        Spine::PoolRun,
        Spine::PinnedMain,
        Spine::EagerTeardown,
        Spine::DeferredEntry,
        Spine::DeferredMainEntry,
        Spine::MainFallThrough,
        Spine::MainTestFailure,
        Spine::MainReturn,
        Spine::Reclaim,
        Spine::Cascade,
        Spine::Settle,
        Spine::QueueDrain,
        Spine::Process,
    ];

    /// The name a trace line carries.
    pub fn name(self) -> &'static str {
        match self {
            Spine::Instantiation => "Instantiation",
            Spine::PoolRun => "PoolRun",
            Spine::PinnedMain => "PinnedMain",
            Spine::EagerTeardown => "EagerTeardown",
            Spine::DeferredEntry => "DeferredEntry",
            Spine::DeferredMainEntry => "DeferredMainEntry",
            Spine::MainFallThrough => "MainFallThrough",
            Spine::MainTestFailure => "MainTestFailure",
            Spine::MainReturn => "MainReturn",
            Spine::Reclaim => "Reclaim",
            Spine::Cascade => "Cascade",
            Spine::Settle => "Settle",
            Spine::QueueDrain => "QueueDrain",
            Spine::Process => "Process",
        }
    }

    pub fn from_name(name: &str) -> Option<Spine> {
        Spine::ALL.iter().copied().find(|s| s.name() == name)
    }
}

/// The domain an obligation runs on, relative to its instance. The
/// producer resolves a role to P1's domain id; a role is what the rule
/// says, a domain id is what a deployment makes of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DomainRole {
    /// The thread running the code that holds the literal.
    Instantiating,
    /// The instance's own queue owner: main, its pool's worker, or its
    /// pinned thread.
    Own,
    /// The instance's owner's queue owner: where the owner's handlers
    /// run (decision L0-1).
    Owner,
    /// The thread running the frame or cascade that tears the instance
    /// down.
    Teardown,
    /// The process's main thread.
    Main,
    /// A cooperative pool's worker, for an obligation every worker owes
    /// (the pool join's cancellation).
    PoolWorker,
    /// Compile time: the obligation is discharged by what is emitted.
    Compile,
}

/// The order an obligation is placed in, on events.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Edges {
    /// Events that happen before this obligation is entered. A
    /// prerequisite is a completion unless it names an entry.
    pub entry: Vec<Prerequisite>,
    /// Events that happen before this obligation completes or reaches
    /// any terminal.
    pub completion: Vec<Prerequisite>,
    /// For a nested action: the action it runs inside, entered and not
    /// completed. Its entry edge names that action's entry, and that
    /// action's completion edges name this one's terminal.
    pub within: Option<ObligationId>,
}

/// One edge into an obligation: the event that happens before it, and
/// the rule that orders the two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Prerequisite {
    pub event: Event,
    pub rule: Rule,
}

/// A point in one obligation's life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Event {
    pub obligation: ObligationId,
    pub point: Point,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Point {
    Entered,
    /// Ended in [`Terminal::Completed`].
    Completed,
    /// Ended in any of its terminals.
    Ended,
    /// Ended in this terminal.
    Terminal(Terminal),
}

/// A named terminal outcome (decision line 19's table, with the
/// inventory's).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Terminal {
    /// Started and ran to its end.
    Completed,
    /// Never started. A run rejected before admission (a shutdown
    /// reason), or admitted and then canceled before it started (an
    /// acknowledgement); a restart asked for during drain.
    NotStarted(NotStarted),
    /// Started, then abandoned by an asynchronous shutdown (a parked
    /// coroutine). The worker's quiescence is not this terminal: it is
    /// the pool join's completion, a separate obligation.
    CanceledAfterStart,
    /// Ended by raising a failure; its delivery is its own
    /// [`ObligationKind::FailureDelivery`].
    FailureDelivered,
    /// Ended in a violation with no route: the report and the
    /// structural exit.
    ClosureViolation,
    /// Not performed because its subject was torn down first: a cell to
    /// a dissolved subscriber, discarded by deregister-on-dissolve.
    Dissolved,
}

impl Terminal {
    /// The form a trace line writes: `CanceledAfterStart`,
    /// `NotStarted(Shutdown(PoolShutdown))`.
    pub fn name(self) -> String {
        match self {
            Terminal::Completed => "Completed".into(),
            Terminal::NotStarted(NotStarted::Acknowledged) => "NotStarted(Acknowledged)".into(),
            Terminal::NotStarted(NotStarted::NoRun) => "NotStarted(NoRun)".into(),
            Terminal::NotStarted(NotStarted::Shutdown(c)) => format!("NotStarted(Shutdown({}))", c.name()),
            Terminal::CanceledAfterStart => "CanceledAfterStart".into(),
            Terminal::FailureDelivered => "FailureDelivered".into(),
            Terminal::ClosureViolation => "ClosureViolation".into(),
            Terminal::Dissolved => "Dissolved".into(),
        }
    }

    pub fn from_name(name: &str) -> Option<Terminal> {
        const ALL: &[Terminal] = &[
            Terminal::Completed,
            Terminal::NotStarted(NotStarted::Acknowledged),
            Terminal::NotStarted(NotStarted::NoRun),
            Terminal::NotStarted(NotStarted::Shutdown(ShutdownCause::PoolShutdown)),
            Terminal::NotStarted(NotStarted::Shutdown(ShutdownCause::OwnerTeardown)),
            Terminal::NotStarted(NotStarted::Shutdown(ShutdownCause::ProcessDrain)),
            Terminal::CanceledAfterStart,
            Terminal::FailureDelivered,
            Terminal::ClosureViolation,
            Terminal::Dissolved,
        ];
        ALL.iter().copied().find(|t| t.name() == name)
    }
}

impl Point {
    /// `Entered`, `Completed`, `Ended`, or `Terminal(<terminal>)`.
    pub fn name(self) -> String {
        match self {
            Point::Entered => "Entered".into(),
            Point::Completed => "Completed".into(),
            Point::Ended => "Ended".into(),
            Point::Terminal(t) => format!("Terminal({})", t.name()),
        }
    }

    pub fn from_name(name: &str) -> Option<Point> {
        match name {
            "Entered" => Some(Point::Entered),
            "Completed" => Some(Point::Completed),
            "Ended" => Some(Point::Ended),
            _ => name.strip_prefix("Terminal(")?.strip_suffix(')').and_then(Terminal::from_name).map(Point::Terminal),
        }
    }

    /// Whether an event at `self` (as a trace writes it: `Entered`,
    /// `Completed` or a terminal) is the point `want` names. `Ended`
    /// is any end; `Completed` and `Terminal(Completed)` are one.
    pub fn satisfies(self, want: Point) -> bool {
        let end = |p: Point| match p {
            Point::Completed => Some(Terminal::Completed),
            Point::Terminal(t) => Some(t),
            _ => None,
        };
        match want {
            Point::Entered => self == Point::Entered,
            Point::Ended => end(self).is_some(),
            _ => end(self).is_some() && end(self) == end(want),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NotStarted {
    /// Rejected before admission: the domain is shutting down.
    Shutdown(ShutdownCause),
    /// Admitted, then canceled before start, with an acknowledgement
    /// the caller can read.
    Acknowledged,
    /// Nothing to start: the locus declares no `run()`, and a resumed
    /// incarnation owes none (line 13; inventory C48 enters one today).
    NoRun,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ShutdownCause {
    /// The pool's shutdown is set.
    PoolShutdown,
    /// The owner has entered teardown.
    OwnerTeardown,
    /// The process drains (a signal raised the draining flag).
    ProcessDrain,
}

impl ShutdownCause {
    pub fn name(self) -> &'static str {
        match self {
            ShutdownCause::PoolShutdown => "PoolShutdown",
            ShutdownCause::OwnerTeardown => "OwnerTeardown",
            ShutdownCause::ProcessDrain => "ProcessDrain",
        }
    }
}

/// How often an obligation is owed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Multiplicity {
    /// Exactly once per incarnation: birth, run, run end.
    OncePerIncarnation,
    /// Exactly once per instance, across its incarnations: subscribe,
    /// accept, drain, dissolve, the reclaim.
    OncePerInstance,
    /// Exactly once per trigger: a delivery per raised failure, an
    /// admission per attempted post.
    OncePerTrigger,
    /// At most once per instance, and only on its guard's path.
    AtMostOncePerInstance,
}

/// Something that must stay alive until an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Retention {
    pub resource: Resource,
    pub until: Event,
    pub status: Status,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Resource {
    /// The instance's struct.
    Instance,
    /// Its owner's struct, with the failure route bound at birth.
    Owner,
    /// The copied violation a held or travelling failure carries.
    FailurePayload,
    /// The instance's arena.
    Arena,
    /// Its owner's arena (an accepted, bubbled or field-owned child's
    /// struct lives in it).
    OwnerArena,
    /// A frame slot that holds the instance until the flush.
    Slot,
    /// A pinned instance's mailbox.
    Mailbox,
    /// A cell in flight: a run post, a bus cell, a failure cell.
    Cell,
}

/// What makes an obligation reach a terminal, apart from what keeps its
/// resources alive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub rule: ProgressRule,
    pub status: Status,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressRule {
    /// Runs to a terminal on its holder's domain without waiting on
    /// another.
    Local,
    /// Waits for an event on another domain, which that domain owes
    /// whatever else it is doing.
    WaitsFor { event: Event, owed_by: DomainRole },
    /// A join: unbounded, and the joining domain keeps completing the
    /// failure decisions the joined children wait for until they
    /// quiesce (join progress).
    Join { pumps: ObligationKind },
    /// Bounded by a named limit, loud when hit
    /// (`LOTUS_BUS_QUIESCE_MS`).
    Bounded { limit: &'static str },
}

/// Whether a rule is what the code does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Status {
    /// Decided; nothing today contradicts it, and nothing yet checks it.
    Adopted,
    /// The code does what the rule says.
    Shipped,
    /// Decided; today's behaviour differs at this inventory row, and a
    /// fixture pins today's outcome.
    KnownOpen { inventory_row: &'static str },
    /// The rule waits on this condition, and is not guessed.
    Pending { condition: &'static str },
}

// ------------------------------------------------------ the decisions

/// One decision line: what it binds, and its status there.
#[derive(Debug, Clone, Copy)]
pub struct DecisionLine {
    /// The inventory's number, or `RD` (restart during drain) and `JP`
    /// (join progress).
    pub line: &'static str,
    pub title: &'static str,
    pub kinds: &'static [ObligationKind],
    /// Every status the line carries, each with what it covers.
    pub statuses: &'static [(Status, &'static str)],
}

use ObligationKind as K;

const POOL_OWNER: &str = "the construction-time domain for an owner placed on a cooperative pool: decision L0-1 names the pool's worker, spec/semantics.md the settling thread";
const NO_OPTION: &str = "the wave-2 decisions frame lines 1-3 as one protocol and choose no option for this line";

/// The decision lines and the kinds each binds. The module docs carry
/// the same table, and `the_doc_table_is_the_data` holds the two equal.
pub const DECISION_LINES: &[DecisionLine] = &[
    DecisionLine {
        line: "1",
        title: "construction-time delivery",
        kinds: &[K::ConstructionDelivery, K::ParamsSettle],
        statuses: &[
            (Status::Shipped, "an owner on the instantiating thread's domain"),
            (Status::Pending { condition: POOL_OWNER }, "an owner placed on a cooperative pool"),
        ],
    },
    DecisionLine {
        line: "2",
        title: "post-run tick closures on a posted run()",
        kinds: &[K::Closures, K::Run],
        statuses: &[(Status::Pending { condition: NO_OPTION }, "the tick and duration closures after a posted run()")],
    },
    DecisionLine {
        line: "3",
        title: "where lifecycle methods run on a pool",
        kinds: &[K::Accept, K::Birth, K::Run, K::Dissolve],
        statuses: &[
            (Status::Pending { condition: NO_OPTION }, "birth, accept and dissolve of a pool-placed locus"),
            (
                Status::Shipped,
                "a field nested under a pool-placed field runs its run() on the pool the placement table gives it (L4)",
            ),
        ],
    },
    DecisionLine {
        line: "4",
        title: "the failure route bound at birth, in every spine",
        kinds: &[K::FailureDelivery, K::Reclaim],
        statuses: &[
            (Status::KnownOpen { inventory_row: "C25" }, "a dissolve-epoch violation under the reclaim spine"),
            (Status::KnownOpen { inventory_row: "C31" }, "a dissolve-epoch violation in a cascade lowered in fn main"),
        ],
    },
    DecisionLine {
        line: "5",
        title: "accept: after params, before birth, no rejection",
        kinds: &[K::Accept],
        statuses: &[(Status::Shipped, "accept's position; the admission interface is separate work")],
    },
    DecisionLine {
        line: "6",
        title: "registration before birth; readiness",
        kinds: &[K::Subscribe, K::Readiness],
        statuses: &[
            (Status::Shipped, "registration before birth()"),
            (
                Status::Shipped,
                "delivery eligible only once birth() completes; what is published before is parked, never dropped (L4)",
            ),
        ],
    },
    DecisionLine {
        line: "7",
        title: "abort unsatisfiable waits before the join",
        kinds: &[K::WaitAbort, K::PoolJoin],
        statuses: &[(Status::KnownOpen { inventory_row: "R34" }, "the wait-abort raised after the pool join")],
    },
    DecisionLine {
        line: "8",
        title: "a birth failure is a ClosureViolation; the child is kept",
        kinds: &[K::FailureDelivery, K::Birth],
        statuses: &[(Status::Shipped, "the failure's shape and the kept child")],
    },
    DecisionLine {
        line: "9",
        title: "delivery at the failing epoch, held while params are open",
        kinds: &[K::FailureDelivery, K::Closures],
        statuses: &[(Status::Shipped, "the epoch of delivery")],
    },
    DecisionLine {
        line: "10",
        title: "dissolve-epoch closures before the user's dissolve()",
        kinds: &[K::Closures, K::Dissolve],
        statuses: &[(Status::Shipped, "the order")],
    },
    DecisionLine {
        line: "11",
        title: "a let-bound literal drains at scope exit",
        kinds: &[K::Drain],
        statuses: &[(Status::Shipped, "drain deferred with dissolve")],
    },
    DecisionLine {
        line: "12",
        title: "owned fields drain before their parent, in the child's domain",
        kinds: &[K::Drain],
        statuses: &[
            (Status::KnownOpen { inventory_row: "C9" }, "a pinned locus's owned fields are never drained"),
            (
                Status::KnownOpen { inventory_row: "C32" },
                "an interface- or perspective-typed field is drained after its owner's dissolve",
            ),
        ],
    },
    DecisionLine {
        line: "13",
        title: "resume through placement and admission",
        kinds: &[K::Resume, K::RunAdmission, K::Run],
        statuses: &[
            (Status::KnownOpen { inventory_row: "C43" }, "a pool-placed child's resumed run() runs inline"),
            (Status::KnownOpen { inventory_row: "C48" }, "a resumed locus with no run() still enters Run"),
        ],
    },
    DecisionLine {
        line: "14",
        title: "order by construction, with latches",
        kinds: &[K::Reclaim],
        statuses: &[
            (Status::Shipped, "emission order and the latches"),
            (Status::Shipped, "verified by the trace build (L2): each instance reclaimed once, after its birth, its children before it"),
        ],
    },
    DecisionLine {
        line: "15",
        title: "a signal raises a cooperative flag",
        kinds: &[K::ProcessDrain],
        statuses: &[(Status::Shipped, "no lifecycle call from the signal path")],
    },
    DecisionLine {
        line: "16",
        title: "the target's lifecycle obligations come from the capability matrix",
        kinds: &[K::PoolJoin, K::WaitAbort],
        statuses: &[(
            Status::Pending { condition: "P3's capability matrix is the authority; gating the eager spine is an interim correction" },
            "pool and wait actions on a target without threads",
        )],
    },
    DecisionLine {
        line: "17",
        title: "the pinned join set and order",
        kinds: &[K::PinnedJoin, K::TeardownDelivery],
        statuses: &[(
            Status::Pending {
                condition: "the deferred rule is the baseline only if GH #253's final-publish guarantees hold across eager, deferred and declaration permutations",
            },
            "subscription-less pinned first, subscribers in their slots, in every spine",
        )],
    },
    DecisionLine {
        line: "18",
        title: "every spine pre-drains; not a quiescence witness",
        kinds: &[K::PreDrain],
        statuses: &[(Status::KnownOpen { inventory_row: "C13" }, "the eager spine has no pre-drain")],
    },
    DecisionLine {
        line: "19",
        title: "a run's admission and its named terminal outcome",
        kinds: &[K::RunAdmission, K::Run, K::Cancellation],
        statuses: &[
            (
                Status::Shipped,
                "a queued run is retained against its child's teardown, which cancels it first: NotStarted(Acknowledged) (L5)",
            ),
            (Status::KnownOpen { inventory_row: "R19" }, "a post refused at shutdown, or freed unrun at the pools' teardown, is silent"),
            (Status::Shipped, "an abandoned parked run ends CanceledAfterStart, named by the trace build (L2)"),
        ],
    },
    DecisionLine {
        line: "RD",
        title: "restart during drain is not performed",
        kinds: &[K::RecoveryDecision, K::Restart],
        statuses: &[
            (Status::Shipped, "while the process drains"),
            (Status::KnownOpen { inventory_row: "C42" }, "while the owner is in teardown"),
        ],
    },
    DecisionLine {
        line: "JP",
        title: "join progress",
        kinds: &[K::JoinProgress, K::FailureDelivery],
        statuses: &[
            (Status::KnownOpen { inventory_row: "C18" }, "a pinned join pumps no queue"),
            (Status::KnownOpen { inventory_row: "R20" }, "the pool joins pump no queue"),
        ],
    },
];

impl Status {
    /// The status as the decision table writes it.
    pub fn label(self) -> String {
        match self {
            Status::Adopted => "Adopted".to_string(),
            Status::Shipped => "Shipped".to_string(),
            Status::KnownOpen { inventory_row } => format!("KnownOpen {inventory_row}"),
            Status::Pending { .. } => "Pending".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    /// Lines 1-19, then the two adopted requirements, each once.
    #[test]
    fn every_decision_line_binds_a_kind() {
        let lines: Vec<&str> = DECISION_LINES.iter().map(|l| l.line).collect();
        let mut expected: Vec<String> = (1..=19).map(|n| n.to_string()).collect();
        expected.push("RD".into());
        expected.push("JP".into());
        assert_eq!(lines, expected);
        for l in DECISION_LINES {
            assert!(!l.kinds.is_empty(), "line {} binds no kind", l.line);
            assert!(!l.statuses.is_empty(), "line {} has no status", l.line);
        }
    }

    /// The Pending lines are exactly the ones the decisions leave open.
    #[test]
    fn pending_is_named_never_guessed() {
        let pending: BTreeSet<&str> = DECISION_LINES
            .iter()
            .filter(|l| l.statuses.iter().any(|(s, _)| matches!(s, Status::Pending { .. })))
            .map(|l| l.line)
            .collect();
        assert_eq!(pending, BTreeSet::from(["1", "2", "3", "16", "17"]));
    }

    #[test]
    fn every_kind_names_its_rows_and_a_name() {
        let names: BTreeSet<&str> = ObligationKind::ALL.iter().map(|k| k.name()).collect();
        assert_eq!(names.len(), ObligationKind::ALL.len());
        for k in ObligationKind::ALL {
            assert!(!k.rows().is_empty(), "{} names no inventory row", k.name());
        }
    }

    /// The table in the module docs is [`DECISION_LINES`], line for line.
    #[test]
    fn the_doc_table_is_the_data() {
        let src = include_str!("lifecycle.rs");
        let table: Vec<&str> = src
            .lines()
            .skip_while(|l| !l.starts_with("//! line  kinds"))
            .skip(1)
            .take_while(|l| !l.starts_with("//! ```"))
            .map(|l| l.trim_start_matches("//!").trim())
            .collect();
        assert_eq!(table.len(), DECISION_LINES.len());
        for (row, line) in table.iter().zip(DECISION_LINES) {
            let mut words = row.split_whitespace();
            assert_eq!(words.next(), Some(line.line));
            let kinds: Vec<&str> = words.by_ref().take(line.kinds.len()).collect();
            let want: Vec<&str> = line.kinds.iter().map(|k| k.name()).collect();
            assert_eq!(kinds, want, "line {}", line.line);
            let status: String = words.collect::<Vec<_>>().join(" ");
            for (s, _) in line.statuses {
                assert!(status.contains(&s.label()), "line {}: {status:?} lacks {}", line.line, s.label());
            }
        }
    }
}

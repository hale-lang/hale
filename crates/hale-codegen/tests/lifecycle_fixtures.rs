//! The lifecycle fixtures (F.40 phase 3, L1): one program per decision
//! line of the lifecycle inventory, and the negative cases the wave-2
//! decisions require, under `tests/fixtures/lifecycle/`.
//!
//! Each fixture's header names the obligation it exercises
//! (`hale_types::lifecycle::ObligationKind`) and its expected terminal
//! outcome. The harness checks it (`hale check` clean), holds it to
//! `hale fmt`, builds it, runs it under a deadline, and judges what it
//! printed into one outcome word: the fixture's own `outcome:` line, or
//! an order the harness reads from its `ev` lines.
//!
//! [`FIXTURES`] gives each fixture's adopted outcome. A fixture whose
//! adopted outcome differs from today's is listed in [`KNOWN_OPEN`] with
//! its inventory row and today's outcome, and is asserted to give
//! today's: when the fix lands the fixture gives the adopted outcome,
//! the assertion fails, and the entry has to go. A fixture on a line
//! the decisions leave pending has no adopted outcome; [`PENDING`]
//! pins today's, with the line's condition in its header.
//!
//! The fixtures stay out of the example corpus on purpose: a known-open
//! fixture may hang today (decision line 7), and the corpus oracle runs
//! every example to completion.
//!
//! **The trace oracle (L2).** Every fixture is built with the lifecycle
//! trace (`BuildOptions::lifecycle_trace`; spec/runtime.md § The
//! lifecycle trace), and beside the outcome word its trace is checked:
//! against the laws every trace owes (`lifecycle::trace::laws`), and,
//! for a line with an adopted rule, against [`PLANS`], the line's
//! obligations with their entry and completion edges, order-insensitive
//! across threads and ordered within a domain. A fixture whose trace
//! shows today's departure from the plan is in [`TRACE_KNOWN_OPEN`],
//! with the inventory row and the violation the trace shows; when the
//! fix lands the violation goes, the assertion fails, and so does the
//! entry. A pending line's fixture is held to the laws alone.
//!
//! [`CONTROLS`] are the negative controls: a step removed or reordered
//! (`LOTUS_LIFECYCLE_SKIP` in the trace build, or today's own order for
//! a rule not yet shipped), each asserted to make the oracle fail, and
//! with the violation that says why.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use hale_codegen::build_executable_with_options;
use hale_types::lifecycle::trace::{self, Count, Expected, Owed, Trace, Violation};
use hale_types::lifecycle::{Event, Multiplicity, ObligationId, ObligationKind, Point, Spine, Terminal};

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

/// Every fixture finishes in well under a second; a hang is a known-open
/// outcome of its own (`timeout`), not a stalled suite.
const DEADLINE: Duration = Duration::from_secs(10);

struct Fixture {
    file: &'static str,
    /// The inventory's decision line, or `RD` / `JP`.
    line: &'static str,
    /// The adopted terminal outcome; `None` on a pending line.
    adopted: Option<&'static str>,
    run: RunMode,
    judge: fn(&Ran) -> String,
}

#[derive(Clone, Copy, PartialEq)]
enum RunMode {
    Plain,
    /// Send SIGINT once stdout carries this line.
    SigintAfter(&'static str),
    /// Checked and built, never run: the order it needs takes a runtime
    /// handshake (L5).
    CompileOnly,
}

struct Ran {
    stdout: String,
    /// stderr without the trace's lines.
    stderr: String,
    trace: Trace,
    code: Option<i32>,
    timed_out: bool,
}

impl Ran {
    /// The process ended on its own and cleanly, so an obligation
    /// entered and never ended is a violation, not a cut-off.
    fn complete(&self) -> bool {
        !self.timed_out && self.code == Some(0)
    }
}

const FIXTURES: &[Fixture] = &[
    Fixture { file: "l01_held_failure_settle.hl", line: "1", adopted: Some("delivered-at-settle"), run: RunMode::Plain, judge: outcome_line },
    Fixture { file: "l01_pool_owner_settle.hl", line: "1", adopted: None, run: RunMode::Plain, judge: outcome_line },
    Fixture { file: "l01_neg_same_pool_held.hl", line: "1", adopted: Some("delivered-once-resumed"), run: RunMode::Plain, judge: outcome_line },
    Fixture { file: "l01_neg_it_waits_worker_queue.hl", line: "1", adopted: Some("delivered-at-settle-queue-ran"), run: RunMode::Plain, judge: outcome_line },
    Fixture { file: "l02_tick_after_posted_run.hl", line: "2", adopted: None, run: RunMode::Plain, judge: outcome_line },
    Fixture { file: "l03_pool_birth_domain.hl", line: "3", adopted: None, run: RunMode::Plain, judge: outcome_line },
    Fixture { file: "l04_dissolve_route_reclaim.hl", line: "4", adopted: Some("owner-handled-once"), run: RunMode::Plain, judge: outcome_or_exit },
    Fixture { file: "l04_dissolve_route_cascade.hl", line: "4", adopted: Some("owner-handled-once"), run: RunMode::Plain, judge: handled_once },
    Fixture { file: "l05_accept_position.hl", line: "5", adopted: Some("accept-after-params-before-birth"), run: RunMode::Plain, judge: outcome_line },
    Fixture { file: "l06_readiness_main.hl", line: "6", adopted: Some("delivered-after-birth"), run: RunMode::Plain, judge: outcome_line },
    Fixture { file: "l06_readiness_pool.hl", line: "6", adopted: Some("delivered-after-birth"), run: RunMode::Plain, judge: outcome_line },
    Fixture { file: "l07_pool_or_wait_teardown.hl", line: "7", adopted: Some("wait-aborted"), run: RunMode::Plain, judge: wait_abort },
    Fixture { file: "l08_birth_failure_kept.hl", line: "8", adopted: Some("closure-violation-child-kept"), run: RunMode::Plain, judge: outcome_line },
    Fixture { file: "l09_delivery_at_epoch.hl", line: "9", adopted: Some("delivered-at-epoch"), run: RunMode::Plain, judge: outcome_line },
    Fixture { file: "l10_dissolve_closures_first.hl", line: "10", adopted: Some("closures-before-dissolve"), run: RunMode::Plain, judge: closures_before_dissolve },
    Fixture { file: "l11_let_bound_drain.hl", line: "11", adopted: Some("drain-at-scope-exit"), run: RunMode::Plain, judge: drain_at_scope_exit },
    Fixture { file: "l12_pinned_fields_drain.hl", line: "12", adopted: Some("fields-drained-first"), run: RunMode::Plain, judge: fields_drained_first },
    Fixture { file: "l13_resume_pool_child.hl", line: "13", adopted: Some("resumed-posted"), run: RunMode::Plain, judge: outcome_line },
    Fixture { file: "l14_reclaim_exactly_once.hl", line: "14", adopted: Some("torn-down-once"), run: RunMode::Plain, judge: torn_down_once },
    Fixture { file: "l15_sigint_flag.hl", line: "15", adopted: Some("cooperative-drain"), run: RunMode::SigintAfter("ev ready"), judge: cooperative_drain },
    Fixture { file: "l16_eager_spine_pool_join.hl", line: "16", adopted: None, run: RunMode::Plain, judge: joined_before_teardown },
    Fixture { file: "l17_pinned_join_eager.hl", line: "17", adopted: None, run: RunMode::Plain, judge: final_publish },
    Fixture { file: "l17_pinned_join_deferred.hl", line: "17", adopted: None, run: RunMode::Plain, judge: final_publish },
    Fixture { file: "l18_eager_pre_drain.hl", line: "18", adopted: Some("delivered-before-teardown"), run: RunMode::Plain, judge: delivered_before_teardown },
    Fixture { file: "l19_parked_started_coroutine.hl", line: "19", adopted: Some("named:canceled-after-start"), run: RunMode::Plain, judge: parked_cancellation },
    Fixture { file: "l19_self_post_overflow.hl", line: "19", adopted: Some("all-completed"), run: RunMode::Plain, judge: all_completed },
    Fixture { file: "l19_resumed_run_at_shutdown.hl", line: "19", adopted: Some("completed-or-named"), run: RunMode::Plain, judge: completed_or_named },
    Fixture { file: "l19_full_ring.hl", line: "19", adopted: Some("admitted-or-named"), run: RunMode::CompileOnly, judge: outcome_line },
    Fixture { file: "l19_empty_ring_last_check.hl", line: "19", adopted: Some("admitted-or-named"), run: RunMode::CompileOnly, judge: outcome_line },
    Fixture { file: "rd_restart_during_teardown.hl", line: "RD", adopted: Some("restart-not-performed"), run: RunMode::Plain, judge: restart_during_teardown },
    Fixture { file: "jp_late_failure_pinned_join.hl", line: "JP", adopted: Some("delivered-once"), run: RunMode::Plain, judge: outcome_line },
    Fixture { file: "jp_late_failure_pool_join.hl", line: "JP", adopted: Some("delivered-once"), run: RunMode::Plain, judge: outcome_line },
];

/// Fixtures whose adopted outcome is not today's: (file, inventory row,
/// today's outcome). Each is asserted to give today's outcome, and to
/// not give the adopted one.
const KNOWN_OPEN: &[(&str, &str, &str)] = &[
    ("l04_dissolve_route_reclaim.hl", "C25", "structural-exit"),
    ("l04_dissolve_route_cascade.hl", "C31", "structural-exit"),
    ("l06_readiness_main.hl", "C8", "delivered-during-birth"),
    ("l06_readiness_pool.hl", "C8", "delivered-during-birth"),
    ("l07_pool_or_wait_teardown.hl", "R34", "hang-in-pool-join"),
    ("l12_pinned_fields_drain.hl", "C9", "inner-not-drained"),
    ("l13_resume_pool_child.hl", "C43", "resumed-inline"),
    ("l19_full_ring.hl", "R19", "not-run"),
    ("l19_empty_ring_last_check.hl", "R19", "not-run"),
    ("rd_restart_during_teardown.hl", "C42", "restarted-during-teardown"),
];

/// Fixtures on a pending line: (file, today's outcome).
const PENDING: &[(&str, &str)] = &[
    ("l01_pool_owner_settle.hl", "delivered-at-settle"),
    ("l02_tick_after_posted_run.hl", "tick-before-run-returned"),
    ("l03_pool_birth_domain.hl", "birth-inline-run-posted"),
    ("l16_eager_spine_pool_join.hl", "joined-before-teardown"),
    ("l17_pinned_join_eager.hl", "final-dropped"),
    ("l17_pinned_join_deferred.hl", "final-dropped"),
];

/// Each adopted line's plan, as the trace oracle reads it (a fixture on
/// a pending line, or compiled only, has none). One line per
/// declaration, its steps in the order they hold within one domain:
///
/// ```text
/// <Decl>[*N|*+]: <Kind>[@Spine][*N][=Terminal][!domain] ...
/// edge <Decl>.<Kind>[@Spine].<Point> -> <Decl>.<Kind>[@Spine].<Point>
/// ```
///
/// `Decl` is the lowered declaration name, `-` for a process-level
/// obligation. A step is owed once by each of `N` subjects (default 1;
/// `*+` at least one): instances, or incarnations for `Birth` and
/// `Run`. `=Terminal` names the end it reaches (default `Completed`);
/// `!domain` claims the thread it runs on. An edge's `Point` is
/// `Entered`, `Completed` or `Ended`; within one declaration it holds
/// per instance, across declarations for every instance of the first.
const PLANS: &[(&str, &str)] = &[
    (
        "l01_held_failure_settle.hl",
        "App: ParamsSettle Birth Run Drain Dissolve Reclaim
         Boom: Birth Run FailureDelivery ConstructionDelivery Drain Dissolve Reclaim
         edge App.ParamsSettle.Completed -> Boom.FailureDelivery.Completed
         edge Boom.FailureDelivery.Completed -> App.Birth.Entered
         edge Boom.Reclaim.Completed -> App.Reclaim.Entered",
    ),
    (
        "l01_neg_same_pool_held.hl",
        "App: Birth Run Drain Dissolve Reclaim
         Spawner: Birth Run!pool:side Drain Dissolve Reclaim
         Owner: ParamsSettle!pool:side Birth Run Drain Dissolve Reclaim
         Late: Birth*2 FailureDelivery!pool:side ConstructionDelivery Restart Run Drain Dissolve Reclaim
         edge Owner.ParamsSettle.Completed -> Late.FailureDelivery.Completed
         edge Late.FailureDelivery.Completed -> Owner.Birth.Entered
         edge Late.FailureDelivery.Completed -> Late.Restart.Entered",
    ),
    (
        "l01_neg_it_waits_worker_queue.hl",
        "App: ParamsSettle Birth Run Drain Dissolve Reclaim
         Failer: Birth Run!pool:side FailureDelivery ConstructionDelivery Drain Dissolve Reclaim
         Quick: Birth Run!pool:side Drain Dissolve Reclaim
         Slow: Birth Run!main Drain Dissolve Reclaim
         edge App.ParamsSettle.Completed -> Failer.FailureDelivery.Completed
         edge Failer.FailureDelivery.Completed -> App.Birth.Entered",
    ),
    (
        "l04_dissolve_route_reclaim.hl",
        "App: Birth Run Drain Dissolve Reclaim
         Flowing: Accept Birth Run Drain Dissolve FailureDelivery Reclaim
         edge Flowing.FailureDelivery.Completed -> Flowing.Reclaim.Entered",
    ),
    (
        "l04_dissolve_route_cascade.hl",
        "App: ParamsSettle Birth Drain Dissolve Reclaim
         Kid: Birth Drain Dissolve FailureDelivery Reclaim
         edge Kid.FailureDelivery.Completed -> Kid.Reclaim.Entered",
    ),
    (
        "l05_accept_position.hl",
        "App: Birth Run Drain Dissolve Reclaim
         Kid: Accept Birth Drain Dissolve Reclaim
         edge Kid.Accept.Completed -> Kid.Birth.Entered",
    ),
    // Line 6's readiness has no trace event yet; the plan holds the rest.
    ("l06_readiness_main.hl", "App: Birth Run Drain Dissolve Reclaim\n Sub: Birth Drain Dissolve Reclaim"),
    ("l06_readiness_pool.hl", "App: Birth Run Drain Dissolve Reclaim\n Sub: Birth Drain Dissolve Reclaim"),
    (
        "l07_pool_or_wait_teardown.hl",
        "-: WaitAbort@EagerTeardown PoolJoin@EagerTeardown
         edge -.WaitAbort@EagerTeardown.Completed -> -.PoolJoin@EagerTeardown.Entered",
    ),
    (
        "l08_birth_failure_kept.hl",
        "App: Birth Run Drain Dissolve Reclaim
         Kid: Birth FailureDelivery Drain Dissolve Reclaim
         edge Kid.Birth.Completed -> Kid.FailureDelivery.Entered
         edge Kid.FailureDelivery.Completed -> Kid.Reclaim.Entered",
    ),
    (
        "l09_delivery_at_epoch.hl",
        "App: Birth Run Drain Dissolve Reclaim
         Kid: Birth Run FailureDelivery Drain Dissolve Reclaim
         edge Kid.FailureDelivery.Completed -> Kid.Run.Completed",
    ),
    // The violation exits the process inside the cascade's Dissolve.
    (
        "l10_dissolve_closures_first.hl",
        "App: ParamsSettle Birth Drain Dissolve
         Kid: Birth Drain Dissolve
         edge App.Dissolve.Completed -> Kid.Dissolve.Entered",
    ),
    (
        "l11_let_bound_drain.hl",
        "Kid: Birth Run Drain@DeferredEntry Dissolve@DeferredEntry Reclaim
         -: PreDrain@MainFallThrough
         edge Kid.Run.Completed -> -.PreDrain@MainFallThrough.Entered
         edge -.PreDrain@MainFallThrough.Completed -> Kid.Drain@DeferredEntry.Entered",
    ),
    (
        "l12_pinned_fields_drain.hl",
        "App: Birth Run Drain Dissolve Reclaim
         Outer: Birth!pinned Run!pinned Drain!pinned Dissolve!pinned PinnedJoin!main Reclaim
         Inner: Birth Drain Dissolve Reclaim
         edge Inner.Drain.Completed -> Outer.Drain.Entered
         edge Outer.Dissolve.Completed -> Outer.PinnedJoin.Completed",
    ),
    (
        "l13_resume_pool_child.hl",
        "App: ParamsSettle Birth Run Drain Dissolve Reclaim
         Kid: Birth FailureDelivery ConstructionDelivery Run!pool:side Drain Dissolve Reclaim
         edge App.ParamsSettle.Completed -> Kid.FailureDelivery.Completed
         edge Kid.FailureDelivery.Completed -> Kid.Run.Entered",
    ),
    // Line 14's verification: each child torn down once, by its run
    // end's reclaim, and before its owner's arena goes.
    (
        "l14_reclaim_exactly_once.hl",
        "App: Birth Run Drain Dissolve Reclaim
         Kid*3: Accept Birth Run Drain@Reclaim Dissolve@Reclaim Reclaim@Reclaim
         edge Kid.Run.Completed -> Kid.Drain@Reclaim.Entered
         edge Kid.Reclaim@Reclaim.Completed -> App.Reclaim.Entered",
    ),
    (
        "l15_sigint_flag.hl",
        "App: Birth Drain Dissolve Reclaim
         Loop: Birth!pinned Run!pinned Drain!pinned Dissolve!pinned PinnedJoin!main Reclaim
         edge Loop.Run.Completed -> Loop.Drain.Entered
         edge Loop.Dissolve.Completed -> Loop.PinnedJoin.Completed",
    ),
    (
        "l18_eager_pre_drain.hl",
        "Hub: Birth Run Drain Dissolve Reclaim
         Sub: Birth Drain Dissolve Reclaim
         -: PreDrain@EagerTeardown
         edge Hub.Run.Completed -> -.PreDrain@EagerTeardown.Entered
         edge -.PreDrain@EagerTeardown.Completed -> Sub.Drain.Entered",
    ),
    // R20a: the started run parked in accept() is abandoned at the pool
    // join and named; the worker's quiescence is the join's completion.
    (
        "l19_parked_started_coroutine.hl",
        "App: Birth Run Drain Dissolve Reclaim
         __StdIoTcpListener: Birth Run=CanceledAfterStart!pool:io Cancellation!pool:io Drain Dissolve Reclaim
         -: PoolJoin@EagerTeardown
         edge __StdIoTcpListener.Cancellation.Completed -> -.PoolJoin@EagerTeardown.Completed
         edge __StdIoTcpListener.Run.Ended -> __StdIoTcpListener.Drain.Entered",
    ),
    (
        "l19_self_post_overflow.hl",
        "App: Birth Drain Dissolve Reclaim
         Spawner: Birth Run!pool:side Drain Dissolve Reclaim
         Kid*20: Birth Run!pool:side Drain Dissolve Reclaim
         edge Kid.Run.Completed -> Kid.Drain.Entered",
    ),
    (
        "l19_resumed_run_at_shutdown.hl",
        "App: ParamsSettle Birth Drain Dissolve Reclaim
         Kid: Birth FailureDelivery ConstructionDelivery Run Drain Dissolve Reclaim
         edge Kid.Run.Ended -> Kid.Dissolve.Entered",
    ),
    (
        "rd_restart_during_teardown.hl",
        "App: ParamsSettle Birth Run Drain Dissolve Reclaim
         Kid: Birth Run FailureDelivery Restart*0 Drain Dissolve Reclaim",
    ),
    (
        "jp_late_failure_pinned_join.hl",
        "App: ParamsSettle Birth Run Drain Dissolve Reclaim
         Late: Birth!pinned Run!pinned FailureDelivery!main Drain!pinned Dissolve!pinned PinnedJoin Reclaim
         edge Late.FailureDelivery.Completed -> Late.PinnedJoin.Completed",
    ),
    (
        "jp_late_failure_pool_join.hl",
        "App: ParamsSettle Birth Run Drain Dissolve Reclaim
         Late: Birth Run!pool:side FailureDelivery!main Drain Dissolve Reclaim
         -: PoolJoin@EagerTeardown
         edge Late.FailureDelivery.Completed -> -.PoolJoin@EagerTeardown.Completed",
    ),
];

/// Fixtures whose trace departs from their plan today: (file,
/// inventory row, the violation the trace shows, as it starts). Each is
/// asserted to show it; when the fix lands it does not, and the entry
/// has to go.
const TRACE_KNOWN_OPEN: &[(&str, &str, &str)] = &[
    ("l04_dissolve_route_reclaim.hl", "C25", "missing: Flowing.FailureDelivery"),
    ("l04_dissolve_route_cascade.hl", "C31", "missing: Kid.FailureDelivery"),
    // Line 7: the host joins the pools and only then aborts the waits,
    // and the join never returns.
    ("l07_pool_or_wait_teardown.hl", "R34", "edge: -.PoolJoin@EagerTeardown.Entered"),
    ("l12_pinned_fields_drain.hl", "C9", "missing: Inner.Drain"),
    ("l13_resume_pool_child.hl", "C43", "domain: Kid.Run"),
    // Line 18: the step the outcome cannot show.
    ("l18_eager_pre_drain.hl", "C13", "missing: -.PreDrain@EagerTeardown"),
    ("rd_restart_during_teardown.hl", "C42", "count: Kid.Restart"),
    // Join progress: the late failure completes only because its
    // handler runs in place on the child's thread (decision L0-1).
    ("jp_late_failure_pinned_join.hl", "C36", "domain: Late.FailureDelivery"),
    ("jp_late_failure_pool_join.hl", "C36", "domain: Late.FailureDelivery"),
];

// ------------------------------------------------------------ the plans

/// The kinds owed once per incarnation; the rest once per instance.
fn multiplicity(kind: ObligationKind) -> Multiplicity {
    match kind {
        ObligationKind::Birth | ObligationKind::Run | ObligationKind::RunEnd | ObligationKind::Closures => {
            Multiplicity::OncePerIncarnation
        }
        _ => Multiplicity::OncePerInstance,
    }
}

fn parse_count(s: &str) -> Count {
    match s {
        "+" => Count::AtLeast(1),
        n => Count::Exactly(n.parse().unwrap_or_else(|_| panic!("plan: bad count {n:?}"))),
    }
}

/// `Kind[@Spine]` into its parts.
fn kind_spine(s: &str) -> (ObligationKind, Option<Spine>) {
    let (k, sp) = match s.split_once('@') {
        Some((k, sp)) => (k, Some(Spine::from_name(sp).unwrap_or_else(|| panic!("plan: unknown spine {sp}")))),
        None => (s, None),
    };
    (ObligationKind::from_name(k).unwrap_or_else(|| panic!("plan: unknown kind {k}")), sp)
}

fn decl_of(s: &str) -> Option<String> {
    (s != "-").then(|| s.to_string())
}

/// A plan in [`PLANS`]' notation.
fn plan(text: &str) -> Expected {
    let mut exp = Expected::default();
    let mut edges: Vec<(&str, &str)> = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some(edge) = line.strip_prefix("edge ") {
            let (a, b) = edge.split_once(" -> ").unwrap_or_else(|| panic!("plan: bad edge {line:?}"));
            edges.push((a, b));
            continue;
        }
        let (head, steps) = line.split_once(':').unwrap_or_else(|| panic!("plan: bad line {line:?}"));
        let (decl, line_count) = match head.split_once('*') {
            Some((d, c)) => (d, parse_count(c)),
            None => (head, Count::Exactly(1)),
        };
        let mut seq = Vec::new();
        for step in steps.split_whitespace() {
            let (step, domain) = match step.split_once('!') {
                Some((s, d)) => (s, Some(d.to_string())),
                None => (step, None),
            };
            let (step, ends) = match step.split_once('=') {
                Some((s, t)) => {
                    (s, Point::Terminal(Terminal::from_name(t).unwrap_or_else(|| panic!("plan: unknown terminal {t}"))))
                }
                None => (step, Point::Completed),
            };
            let (step, count) = match step.split_once('*') {
                Some((s, c)) => (s, parse_count(c)),
                None => (step, line_count),
            };
            let (kind, spine) = kind_spine(step);
            seq.push(ObligationId(exp.owed.len() as u32));
            exp.owed.push(Owed {
                decl: decl_of(decl),
                kind,
                spine,
                multiplicity: multiplicity(kind),
                count,
                ends,
                domain,
            });
        }
        exp.sequences.push(seq);
    }
    for (a, b) in edges {
        let ev = |r: &str| -> Event {
            let parts: Vec<&str> = r.split('.').collect();
            let [decl, ks, point] = parts[..] else { panic!("plan: bad edge end {r:?}") };
            let (kind, spine) = kind_spine(ks);
            let decl = decl_of(decl);
            let i = exp
                .owed
                .iter()
                .position(|o| o.decl == decl && o.kind == kind && o.spine == spine)
                .unwrap_or_else(|| panic!("plan: edge names {r:?}, which no line owes"));
            Event {
                obligation: ObligationId(i as u32),
                point: Point::from_name(point).unwrap_or_else(|| panic!("plan: bad point {point}")),
            }
        };
        exp.edges.push((ev(a), ev(b)));
    }
    exp
}

/// What the trace oracle says about a run of `file`: the laws, then the
/// plan's violations.
fn trace_violations(file: &str, ran: &Ran) -> Vec<Violation> {
    let mut v = trace::laws(&ran.trace, ran.complete());
    if let Some((_, text)) = PLANS.iter().find(|(f, _)| *f == file) {
        v.extend(plan(text).check(&ran.trace, ran.complete()));
    }
    v
}

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/lifecycle")
}

fn fixture(file: &str) -> &'static Fixture {
    FIXTURES.iter().find(|f| f.file == file).unwrap_or_else(|| panic!("{file} is not in FIXTURES"))
}

fn source(file: &str) -> String {
    let path = dir().join(file);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// Check one fixture and build it with the lifecycle trace; `None` when
/// it is compiled only.
fn build_fixture(f: &Fixture) -> Option<PathBuf> {
    let program = hale_syntax::parse_source(&source(f.file)).unwrap_or_else(|e| panic!("{}: parse: {e:?}", f.file));
    let errs: Vec<String> = hale_types::check_program(&program)
        .iter()
        .filter(|d| d.is_error())
        .map(|d| d.message.clone())
        .collect();
    assert!(errs.is_empty(), "{}: `hale check` refuses it: {errs:?}", f.file);
    let bin = harness::unique_bin(&format!("hale_lifecycle_{}", f.file.trim_end_matches(".hl")));
    let opts = hale_codegen::BuildOptions { lifecycle_trace: true, ..build_opts::options() };
    build_executable_with_options(&program, &bin, &[], &opts).unwrap_or_else(|e| panic!("{}: build: {e:?}", f.file));
    if f.run == RunMode::CompileOnly {
        let _ = std::fs::remove_file(&bin);
        return None;
    }
    Some(bin)
}

/// Build and run one fixture, the runtime given `env`; what it did.
fn run_fixture(f: &Fixture, env: &[(&str, &str)]) -> Option<Ran> {
    let bin = build_fixture(f)?;
    let ran = run_bin(&bin, f.run, env);
    let _ = std::fs::remove_file(&bin);
    eprintln!("{}: --- stdout\n{}--- stderr\n{}--- trace", f.file, ran.stdout, ran.stderr);
    for e in &ran.trace.events {
        eprintln!(
            "{} {} {} {:?} {} {} {:?}",
            e.seq,
            e.kind.name(),
            e.point.name(),
            e.spine.map(Spine::name),
            e.domain,
            e.decl.as_deref().unwrap_or("-"),
            e.subject.map(|s| (s.instance.raw(), s.incarnation.raw()))
        );
    }
    Some(ran)
}

fn run_bin(bin: &Path, mode: RunMode, env: &[(&str, &str)]) -> Ran {
    let mut child = Command::new(bin)
        .envs(env.iter().copied())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn the fixture");
    let mut out = child.stdout.take().expect("stdout");
    let mut err = child.stderr.take().expect("stderr");
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let reader = std::thread::spawn(move || {
        let mut all = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            match out.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    all.extend_from_slice(&buf[..n]);
                    let _ = tx.send(String::from_utf8_lossy(&all).into_owned());
                }
            }
        }
        String::from_utf8_lossy(&all).into_owned()
    });
    let err_reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = err.read_to_string(&mut s);
        s
    });
    let start = Instant::now();
    let mut signalled = false;
    let mut timed_out = false;
    let status = loop {
        if let RunMode::SigintAfter(line) = mode {
            while let Ok(seen) = rx.try_recv() {
                if !signalled && seen.lines().any(|l| l == line) {
                    unsafe { libc::kill(child.id() as i32, libc::SIGINT) };
                    signalled = true;
                }
            }
        }
        if let Some(status) = child.try_wait().expect("wait") {
            break Some(status);
        }
        if start.elapsed() > DEADLINE {
            let _ = child.kill();
            let _ = child.wait();
            timed_out = true;
            break None;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let stderr = err_reader.join().unwrap_or_default();
    let trace = trace::parse(&stderr).unwrap_or_else(|e| panic!("{}: the trace does not parse: {e}", bin.display()));
    Ran {
        stdout: reader.join().unwrap_or_default(),
        stderr: trace.rest.clone(),
        trace,
        code: status.and_then(|s| s.code()),
        timed_out,
    }
}

// ------------------------------------------------------------- judges

fn exit_word(r: &Ran) -> String {
    if r.timed_out {
        return "timeout".to_string();
    }
    match r.code {
        Some(c) => format!("exit-{c}"),
        None => "signal".to_string(),
    }
}

/// The fixture's last `outcome:` line.
fn outcome_line(r: &Ran) -> String {
    if r.timed_out {
        return "timeout".to_string();
    }
    match r.stdout.lines().rev().find_map(|l| l.strip_prefix("outcome: ")) {
        Some(o) => o.to_string(),
        None => exit_word(r),
    }
}

/// The `outcome:` line, or, when the process ended early, how.
fn outcome_or_exit(r: &Ran) -> String {
    match r.stdout.lines().rev().find_map(|l| l.strip_prefix("outcome: ")) {
        Some(o) if !r.timed_out => o.to_string(),
        _ if r.code.is_some_and(|c| c != 0) && r.stderr.contains("ClosureViolation") => "structural-exit".to_string(),
        _ => exit_word(r),
    }
}

/// The owner's handler line exactly once, or the structural exit.
fn handled_once(r: &Ran) -> String {
    match count(r, "ev handler clean") {
        1 if r.code == Some(0) => "owner-handled-once".to_string(),
        0 if r.code.is_some_and(|c| c != 0) && r.stderr.contains("ClosureViolation") => "structural-exit".to_string(),
        n => format!("handler {n}, {}", exit_word(r)),
    }
}

fn pos(r: &Ran, line: &str) -> Option<usize> {
    r.stdout.lines().position(|l| l == line)
}

fn count(r: &Ran, line: &str) -> usize {
    r.stdout.lines().filter(|l| *l == line).count()
}

fn wait_abort(r: &Ran) -> String {
    if r.timed_out {
        return "hang-in-pool-join".to_string();
    }
    if r.stderr.contains("BusWaitAborted") {
        return "wait-aborted".to_string();
    }
    outcome_line(r)
}

/// The handler's line before the dissolve's; or, while the cascade's
/// route misses the handler, the violation's exit with no dissolve
/// line printed before it. Both say the closure ran first.
fn closures_before_dissolve(r: &Ran) -> String {
    match (pos(r, "ev handler clean"), pos(r, "ev kid-dissolve")) {
        (Some(h), Some(d)) if h < d => "closures-before-dissolve".to_string(),
        (Some(_), Some(_)) => "dissolve-before-closures".to_string(),
        (None, None) if r.stderr.contains("closure `clean` failed at dissolve") => "closures-before-dissolve".to_string(),
        (None, Some(_)) if r.stderr.contains("closure `clean` failed at dissolve") => "dissolve-before-closures".to_string(),
        _ => exit_word(r),
    }
}

fn drain_at_scope_exit(r: &Ran) -> String {
    match (pos(r, "ev after-let"), pos(r, "ev kid-drain"), pos(r, "ev kid-dissolve")) {
        (Some(a), Some(d), Some(x)) if a < d && d < x => "drain-at-scope-exit".to_string(),
        (Some(a), Some(d), _) if d < a => "drain-at-construction".to_string(),
        _ => exit_word(r),
    }
}

fn fields_drained_first(r: &Ran) -> String {
    match (pos(r, "ev inner-drain"), pos(r, "ev outer-drain")) {
        (Some(i), Some(o)) if i < o => "fields-drained-first".to_string(),
        (Some(_), Some(_)) => "outer-drained-first".to_string(),
        (None, Some(_)) => "inner-not-drained".to_string(),
        _ => exit_word(r),
    }
}

fn torn_down_once(r: &Ran) -> String {
    let counts: Vec<usize> = ["a", "b", "c"].iter().map(|t| count(r, &format!("ev kid-dissolve {t}"))).collect();
    if counts == [1, 1, 1] && r.code == Some(0) {
        "torn-down-once".to_string()
    } else {
        format!("dissolves {counts:?}, {}", exit_word(r))
    }
}

fn cooperative_drain(r: &Ran) -> String {
    let once = |l: &str| count(r, l) == 1;
    if r.code == Some(0) && once("ev loop-sees-drain") && once("ev loop-drain") && once("ev loop-dissolve") {
        "cooperative-drain".to_string()
    } else {
        exit_word(r)
    }
}

fn joined_before_teardown(r: &Ran) -> String {
    match (pos(r, "ev worker-run-end"), pos(r, "ev worker-dissolve")) {
        (Some(e), Some(d)) if e < d => "joined-before-teardown".to_string(),
        (_, Some(_)) => "torn-down-under-run".to_string(),
        _ => exit_word(r),
    }
}

fn final_publish(r: &Ran) -> String {
    if r.timed_out || r.code != Some(0) {
        return exit_word(r);
    }
    match count(r, "ev sink-heard-final") {
        0 => "final-dropped".to_string(),
        1 => "final-delivered".to_string(),
        n => format!("final-delivered-{n}-times"),
    }
}

fn delivered_before_teardown(r: &Ran) -> String {
    match (pos(r, "ev sub-heard"), pos(r, "ev sub-dissolve")) {
        (Some(h), Some(d)) if h < d => "delivered-before-teardown".to_string(),
        (Some(_), Some(_)) => "delivered-after-teardown".to_string(),
        (None, Some(_)) => "not-delivered".to_string(),
        _ => exit_word(r),
    }
}

/// The abandoned run's outcome is named where the trace build records
/// it: its `Run` ends in `Terminal(CanceledAfterStart)`.
fn parked_cancellation(r: &Ran) -> String {
    if r.timed_out {
        return "timeout".to_string();
    }
    let named = r
        .trace
        .events
        .iter()
        .any(|e| e.kind == ObligationKind::Run && e.point == Point::Terminal(Terminal::CanceledAfterStart));
    if named {
        return "named:canceled-after-start".to_string();
    }
    if r.code == Some(0) {
        return "exit-clean-unnamed".to_string();
    }
    exit_word(r)
}

fn all_completed(r: &Ran) -> String {
    let runs = (0..20).filter(|i| count(r, &format!("ev kid-run {i}")) == 1).count();
    let dissolves = (0..20).filter(|i| count(r, &format!("ev kid-dissolve {i}")) == 1).count();
    if runs == 20 && dissolves == 20 && r.code == Some(0) {
        "all-completed".to_string()
    } else {
        format!("runs {runs}/20, dissolves {dissolves}/20, {}", exit_word(r))
    }
}

fn completed_or_named(r: &Ran) -> String {
    let dissolved_once = count(r, "ev kid-dissolve") == 1;
    let ended = count(r, "ev kid-run-end") == 1 || r.stderr.contains("not-started");
    match (dissolved_once, ended, r.code) {
        (true, true, Some(0)) => "completed-or-named".to_string(),
        (true, false, Some(0)) => "silent".to_string(),
        _ => exit_word(r),
    }
}

fn restart_during_teardown(r: &Ran) -> String {
    if r.timed_out {
        return "timeout".to_string();
    }
    let handled = count(r, "ev handler");
    if handled == 1 && count(r, "ev kid-dissolve births=1") == 1 {
        "restart-not-performed".to_string()
    } else if handled == 1 && count(r, "ev kid-dissolve births=2") == 1 {
        "restarted-during-teardown".to_string()
    } else {
        format!("handler {handled}, {}", exit_word(r))
    }
}

// -------------------------------------------------------------- tests

/// Every `.hl` under `tests/fixtures/lifecycle/` is a row, every row a
/// file; each is `hale fmt` clean; every known-open entry names a row
/// with an adopted outcome, every pending entry one without.
#[test]
fn every_fixture_is_listed_and_formatted() {
    let mut on_disk: Vec<String> = std::fs::read_dir(dir())
        .expect("the fixture directory")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".hl"))
        .collect();
    on_disk.sort();
    let mut listed: Vec<String> = FIXTURES.iter().map(|f| f.file.to_string()).collect();
    listed.sort();
    assert_eq!(on_disk, listed, "the fixture directory and FIXTURES differ");
    for f in FIXTURES {
        let src = source(f.file);
        let formatted = hale_syntax::fmt::format_source(&src).unwrap_or_else(|e| panic!("{}: fmt: {e:?}", f.file));
        assert!(formatted == src, "{} is not `hale fmt` clean; formatted:\n{formatted}", f.file);
        assert!(src.lines().next().is_some_and(|l| l.contains(&format!("line {}", f.line)) || matches!(f.line, "RD" | "JP")), "{}: the header names another line than {}", f.file, f.line);
    }
    for (file, row, today) in KNOWN_OPEN {
        let f = fixture(file);
        let adopted = f.adopted.unwrap_or_else(|| panic!("{file} is KNOWN_OPEN at {row} but has no adopted outcome"));
        assert_ne!(adopted, *today, "{file}: today's outcome is the adopted one");
    }
    for (file, _) in PENDING {
        assert!(fixture(file).adopted.is_none(), "{file} is PENDING but has an adopted outcome");
    }
    for f in FIXTURES.iter().filter(|f| f.adopted.is_none()) {
        assert!(PENDING.iter().any(|(p, _)| *p == f.file), "{} has no adopted outcome and no PENDING entry", f.file);
    }
    // Every adopted line that runs has a plan; a pending or compiled-only
    // fixture has none; every plan parses and names steps that exist.
    for f in FIXTURES {
        let has_plan = PLANS.iter().any(|(p, _)| *p == f.file);
        let wants_plan = f.adopted.is_some() && f.run != RunMode::CompileOnly;
        assert_eq!(has_plan, wants_plan, "{}: a plan is owed exactly by an adopted line that runs", f.file);
    }
    for (file, text) in PLANS {
        fixture(file);
        assert!(!plan(text).owed.is_empty(), "{file}: an empty plan");
    }
    for (file, _, _) in TRACE_KNOWN_OPEN {
        assert!(PLANS.iter().any(|(p, _)| p == file), "{file} is TRACE_KNOWN_OPEN but has no plan");
    }
}

/// The trace oracle over one run: clean, or, for a known-open trace,
/// showing the violation its entry names.
fn assert_trace(file: &str, ran: &Ran) {
    let v = trace_violations(file, ran);
    let shown: Vec<String> = v.iter().map(Violation::to_string).collect();
    eprintln!("{file}: trace oracle: {shown:#?}");
    if let Some((_, row, shows)) = TRACE_KNOWN_OPEN.iter().find(|(k, _, _)| *k == file) {
        assert!(
            shown.iter().any(|s| s.starts_with(shows)),
            "{file}: the trace no longer shows `{shows}`: the fix for inventory row {row} has landed, so its TRACE_KNOWN_OPEN entry has to go; it shows {shown:#?}"
        );
    } else {
        assert!(shown.is_empty(), "{file}: the trace oracle fails: {shown:#?}");
    }
}

fn assert_fixture(file: &str) {
    let f = fixture(file);
    let ran = run_fixture(f, &[]);
    let got = match &ran {
        Some(ran) => (f.judge)(ran),
        None => "not-run".to_string(),
    };
    eprintln!("{file}: outcome {got}");
    if let Some(ran) = &ran {
        assert_trace(file, ran);
    }
    if let Some((_, row, today)) = KNOWN_OPEN.iter().find(|(k, _, _)| *k == file) {
        assert!(
            Some(got.as_str()) != f.adopted,
            "{file} now gives its adopted outcome `{got}`: the fix for inventory row {row} has landed, so its KNOWN_OPEN entry has to go"
        );
        assert_eq!(got, *today, "{file} (known open at {row}) changed its outcome");
    } else if let Some(adopted) = f.adopted {
        assert_eq!(got, adopted, "{file} (decision line {}) does not give its adopted outcome", f.line);
    } else {
        let today = PENDING.iter().find(|(p, _)| *p == file).map(|(_, t)| *t).unwrap_or("<no PENDING entry>");
        assert_eq!(got, today, "{file} (pending, decision line {}) changed its outcome", f.line);
    }
}

macro_rules! fixture_tests {
    ($($name:ident => $file:literal),* $(,)?) => {
        $(
            #[test]
            fn $name() { assert_fixture($file); }
        )*
    };
}

fixture_tests! {
    l01_held_failure_settle => "l01_held_failure_settle.hl",
    l01_pool_owner_settle => "l01_pool_owner_settle.hl",
    l01_neg_same_pool_held => "l01_neg_same_pool_held.hl",
    l01_neg_it_waits_worker_queue => "l01_neg_it_waits_worker_queue.hl",
    l02_tick_after_posted_run => "l02_tick_after_posted_run.hl",
    l03_pool_birth_domain => "l03_pool_birth_domain.hl",
    l04_dissolve_route_reclaim => "l04_dissolve_route_reclaim.hl",
    l04_dissolve_route_cascade => "l04_dissolve_route_cascade.hl",
    l05_accept_position => "l05_accept_position.hl",
    l06_readiness_main => "l06_readiness_main.hl",
    l06_readiness_pool => "l06_readiness_pool.hl",
    l07_pool_or_wait_teardown => "l07_pool_or_wait_teardown.hl",
    l08_birth_failure_kept => "l08_birth_failure_kept.hl",
    l09_delivery_at_epoch => "l09_delivery_at_epoch.hl",
    l10_dissolve_closures_first => "l10_dissolve_closures_first.hl",
    l11_let_bound_drain => "l11_let_bound_drain.hl",
    l12_pinned_fields_drain => "l12_pinned_fields_drain.hl",
    l13_resume_pool_child => "l13_resume_pool_child.hl",
    l14_reclaim_exactly_once => "l14_reclaim_exactly_once.hl",
    l15_sigint_flag => "l15_sigint_flag.hl",
    l16_eager_spine_pool_join => "l16_eager_spine_pool_join.hl",
    l17_pinned_join_eager => "l17_pinned_join_eager.hl",
    l17_pinned_join_deferred => "l17_pinned_join_deferred.hl",
    l18_eager_pre_drain => "l18_eager_pre_drain.hl",
    l19_parked_started_coroutine => "l19_parked_started_coroutine.hl",
    l19_self_post_overflow => "l19_self_post_overflow.hl",
    l19_resumed_run_at_shutdown => "l19_resumed_run_at_shutdown.hl",
    l19_full_ring => "l19_full_ring.hl",
    l19_empty_ring_last_check => "l19_empty_ring_last_check.hl",
    rd_restart_during_teardown => "rd_restart_during_teardown.hl",
    jp_late_failure_pinned_join => "jp_late_failure_pinned_join.hl",
    jp_late_failure_pool_join => "jp_late_failure_pool_join.hl",
}

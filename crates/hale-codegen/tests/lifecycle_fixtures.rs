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
//! for a line with an adopted rule, against the plan the producer
//! derives for the fixture's program (`hale_types::lifecycle::derive`,
//! [`derived_plan`]): the obligations with their entry and completion
//! edges, order-insensitive across threads and ordered within a domain,
//! rendered for the run by `lifecycle::project` along [`run_path`] (the
//! failures the run raises and where, how many body literals it builds,
//! where it ends early, and the lines whose known-open rules it is held
//! to; facts of the run, not of the program). A fixture whose trace
//! shows today's departure from the plan is in [`TRACE_KNOWN_OPEN`],
//! with the inventory row and every violation the defect causes; any
//! other violation fails the fixture, and when the fix lands the listed
//! ones go, the assertion fails, and so does the entry. A pending
//! line's fixture is held to the laws alone. The fixtures in
//! [`UNDERIVED`] have no derived plan yet and are held to their
//! hand-written one.
//!
//! [`CONTROLS`] are the negative controls: a step removed or reordered
//! (`LOTUS_LIFECYCLE_SKIP` in the trace build, or today's own order for
//! a rule not yet shipped), each asserted to make the oracle fail, and
//! with the violation that says why.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use std::collections::BTreeMap;

use hale_codegen::build_executable_with_options;
use hale_frontend::frontend::LoadMode;
use hale_frontend::snapshot::{Config, Snapshot};
use hale_frontend::source::Disk;
use hale_types::lifecycle::project::{self, Focus, Inside, PathFailure, RunPath};
use hale_types::lifecycle::trace::{self, Expected, Trace, Violation};
use hale_types::lifecycle::{FailureSource, NotStarted, ObligationKind, Point, ShutdownCause, Spine, Terminal};

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;
#[path = "support/lifecycle_plan.rs"]
mod lifecycle_plan;

use lifecycle_plan::{normalized, plan, render};

/// Every fixture finishes in well under a second; a hang is a known-open
/// outcome of its own (`timeout`), not a stalled suite.
const DEADLINE: Duration = Duration::from_secs(10);

/// A slot's key is claimed before its instance number is assigned. Stop
/// the creator in that window and make a second thread look up the same
/// subject: it must wait for publication, never return instance zero.
#[test]
fn trace_subject_is_initialized_before_another_thread_uses_it() {
    let runtime = include_str!("../runtime/lotus_arena.c");
    let subject = runtime.split_once("#define LOTUS_LC_SLOTS").unwrap().1;
    let subject = subject.split_once("static void lotus_lc_domain").unwrap().0;
    // Compile the production lookup itself. These test-only wrappers stop
    // the mint just before assignment and observe the reader's ready wait;
    // neither hook is part of a shipped runtime or relies on a timed sleep.
    let source = format!(r#"
#include <assert.h>
#include <pthread.h>
#include <stdatomic.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

static void mint_pause(void);
static void ready_read(void);
#define __atomic_add_fetch(...) \
    ({{ __auto_type value = __atomic_add_fetch(__VA_ARGS__); mint_pause(); value; }})
#undef atomic_load_explicit
#define atomic_load_explicit(...) \
    ({{ __auto_type value = __c11_atomic_load(__VA_ARGS__); ready_read(); value; }})

#define LOTUS_LC_SLOTS{subject}

#undef __atomic_add_fetch
#undef atomic_load_explicit
#define atomic_load_explicit __c11_atomic_load

static pthread_mutex_t gate = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t changed = PTHREAD_COND_INITIALIZER;
static int creator_paused, release_creator, reader_waiting, reader_done;
static _Thread_local int reader;
static int object;
static uint64_t creator_inst, reader_inst;

static void mint_pause(void) {{
    pthread_mutex_lock(&gate);
    creator_paused = 1;
    pthread_cond_broadcast(&changed);
    while (!release_creator) pthread_cond_wait(&changed, &gate);
    pthread_mutex_unlock(&gate);
}}

static void ready_read(void) {{
    if (!reader) return;
    pthread_mutex_lock(&gate);
    reader_waiting = 1;
    pthread_cond_broadcast(&changed);
    pthread_mutex_unlock(&gate);
}}

static void *create_subject(void *unused) {{
    (void)unused;
    creator_inst = lotus_lc_subject(&object, NULL)->inst;
    return NULL;
}}

static void *read_subject(void *unused) {{
    (void)unused;
    reader = 1;
    lotus_lc_slot_t *slot = lotus_lc_subject(&object, "Subject");
    reader_inst = slot->inst;
    pthread_mutex_lock(&gate);
    reader_done = 1;
    pthread_cond_broadcast(&changed);
    pthread_mutex_unlock(&gate);
    return NULL;
}}

int main(void) {{
    alarm(5);
    pthread_t creator, observer;
    assert(pthread_create(&creator, NULL, create_subject, NULL) == 0);
    pthread_mutex_lock(&gate);
    while (!creator_paused) pthread_cond_wait(&changed, &gate);
    pthread_mutex_unlock(&gate);
    assert(pthread_create(&observer, NULL, read_subject, NULL) == 0);
    pthread_mutex_lock(&gate);
    while (!reader_waiting && !reader_done) pthread_cond_wait(&changed, &gate);
    release_creator = 1;
    pthread_cond_broadcast(&changed);
    pthread_mutex_unlock(&gate);
    assert(pthread_join(creator, NULL) == 0);
    assert(pthread_join(observer, NULL) == 0);
    assert(reader_inst != 0 && reader_inst == creator_inst);
    lotus_lc_slot_t *slot = lotus_lc_subject(&object, NULL);
    assert(strcmp(slot->type, "Subject") == 0);
    assert(slot->inc == 0 && slot->running == 0);
    return 0;
}}
"#);
    let bin = harness::unique_bin("hale_trace_subject_publication");
    let c = bin.with_extension("c");
    std::fs::write(&c, source).unwrap();
    let built = Command::new("clang")
        .args(["-std=gnu11", "-pthread"])
        .arg(&c).arg("-o").arg(&bin)
        .output().expect("compile the trace subject publication probe");
    assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
    let ran = Command::new(&bin).output().expect("run the trace subject publication probe");
    let _ = std::fs::remove_file(&c);
    let _ = std::fs::remove_file(&bin);
    assert!(ran.status.success(), "trace subject publication: {}\n{}",
        ran.status, String::from_utf8_lossy(&ran.stderr));
}

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
    /// Run with these variables set: a runtime knob the fixture's shape
    /// needs (a small ring, to fill it).
    Env(&'static [(&'static str, &'static str)]),
}

/// A 64-cell pool ring (the floor `LOTUS_BUS_QUEUE_CAP` allows), so a
/// fixture fills it in a few hundred posts.
const SMALL_RING: &[(&str, &str)] = &[("LOTUS_BUS_QUEUE_CAP", "64")];

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
    Fixture { file: "l03_field_parents_two_domains.hl", line: "3", adopted: Some("born-on-each-parents-domain"), run: RunMode::Plain, judge: leaf_born },
    Fixture { file: "l03_field_parents_two_domains_nested.hl", line: "3", adopted: Some("born-on-each-parents-domain"), run: RunMode::Plain, judge: twig_born },
    Fixture { file: "l03_field_parents_one_domain.hl", line: "3", adopted: Some("born-on-main"), run: RunMode::Plain, judge: leaf_born },
    Fixture { file: "l03_body_literal_two_domains.hl", line: "3", adopted: Some("born-on-each-parents-domain"), run: RunMode::Plain, judge: leaf_born },
    Fixture { file: "l03_body_literal_one_domain.hl", line: "3", adopted: Some("born-on-main"), run: RunMode::Plain, judge: leaf_born },
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
    Fixture { file: "l19_queued_run_canceled.hl", line: "19", adopted: Some("named:not-started"), run: RunMode::Plain, judge: queued_run_canceled },
    Fixture { file: "l19_cross_pool_queued_run_canceled.hl", line: "19", adopted: Some("named:not-started"), run: RunMode::Plain, judge: cross_pool_run_canceled },
    Fixture { file: "l19_started_run_retained.hl", line: "19", adopted: Some("run-held"), run: RunMode::Plain, judge: run_held },
    Fixture { file: "l19_started_run_retained_async.hl", line: "19", adopted: Some("run-held"), run: RunMode::Plain, judge: run_held },
    Fixture { file: "l19_started_run_publishes_back.hl", line: "19", adopted: Some("answered-in-wait"), run: RunMode::Plain, judge: answered_in_wait },
    Fixture { file: "l19_started_run_publishes_back_async.hl", line: "19", adopted: Some("answered-in-wait"), run: RunMode::Plain, judge: answered_in_wait },
    Fixture { file: "l19_handler_replaces_started_run.hl", line: "19", adopted: Some("answered-after-handler"), run: RunMode::Plain, judge: answered_after_handler },
    Fixture { file: "l19_handler_replaces_started_run_async.hl", line: "19", adopted: Some("answered-after-handler"), run: RunMode::Plain, judge: answered_after_handler },
    Fixture { file: "l19_resumed_run_at_shutdown.hl", line: "19", adopted: Some("completed-or-named"), run: RunMode::Plain, judge: completed_or_named },
    Fixture { file: "l19_full_ring.hl", line: "19", adopted: Some("admitted-or-named"), run: RunMode::Env(SMALL_RING), judge: full_ring },
    Fixture { file: "l19_empty_ring_last_check.hl", line: "19", adopted: Some("admitted-or-named"), run: RunMode::Plain, judge: empty_ring },
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

/// The hand-written plans of the fixtures in [`UNDERIVED`], which the
/// producer does not derive yet, in the notation
/// `support/lifecycle_plan.rs` parses: one line per declaration, its
/// steps in the order they hold within one domain, then the edges.
const PLANS: &[(&str, &str)] = &[
    // R19 across pools: the run queued on `side` is canceled by the
    // reclaim on main, inside its bracket. The replacement's run is the
    // judge's, since one plan step cannot owe two ends.
    (
        "l19_cross_pool_queued_run_canceled.hl",
        "App: Birth Run Drain Dissolve Reclaim
         Holder: Birth Drain Dissolve Reclaim
         Kid*2: Birth Drain Dissolve Reclaim
         Kid: Cancellation!main
         edge Kid.Reclaim.Entered -> Kid.Cancellation.Entered",
    ),
    // R19's other half: a run admitted after the worker's last check is
    // canceled by its child's reclaim; once the canceled cells fill the
    // ring, a post is refused for the pool's shutdown, named with no
    // cancellation. The judge counts the two ends.
    (
        "l19_full_ring.hl",
        "App*200: Birth Drain Dissolve Reclaim
         Kid*200: Birth Drain Dissolve Reclaim
         Kid*+: Cancellation!main
         edge Kid.Reclaim.Entered -> Kid.Cancellation.Entered",
    ),
    (
        "l19_empty_ring_last_check.hl",
        "App*2: Birth Drain Dissolve Reclaim
         Kid*2: Birth Drain Dissolve Reclaim
         Kid: Cancellation!main
         edge Kid.Reclaim.Entered -> Kid.Cancellation.Entered",
    ),
];

/// Each adopted line's run, as the producer's plan reads it
/// (`hale_types::lifecycle::project`): the lines whose known-open rules
/// the fixture is held to, and the run's path through the plan (which
/// failures it raises and where, how many instances a body literal
/// builds, a run a shutdown abandons or a teardown cancels, where the
/// run ends early). Facts
/// of the run the producer cannot know from the program. A fixture on a
/// pending line has none and is held to the laws; an UNDERIVED one has
/// none and is held to its hand-written plan.
fn run_path(file: &str) -> Option<(&'static [&'static str], RunPath)> {
    let fails = |decl: &str, source: FailureSource, held: bool, in_teardown: bool, restarts: u32| PathFailure {
        decl: decl.to_string(),
        source,
        held,
        in_teardown,
        restarts,
    };
    let count = |pairs: &[(&str, u32)]| -> BTreeMap<String, u32> { pairs.iter().map(|(d, n)| (d.to_string(), *n)).collect() };
    let mut p = RunPath::default();
    let lines: &'static [&'static str] = match file {
        "l01_held_failure_settle.hl" => {
            p.failures.push(fails("Boom", FailureSource::Run, true, false, 0));
            &["1"]
        }
        "l01_neg_same_pool_held.hl" => {
            p.failures.push(fails("Late", FailureSource::BirthClosure, true, false, 1));
            p.occurrences = count(&[("Owner", 1), ("Late", 1)]);
            // Line 13: the resumed Late declares no run() (C48).
            &["1", "13"]
        }
        "l01_neg_it_waits_worker_queue.hl" => {
            p.failures.push(fails("Failer", FailureSource::Run, true, false, 0));
            &["1"]
        }
        "l04_dissolve_route_reclaim.hl" => {
            p.failures.push(fails("Flowing", FailureSource::Dissolve, false, false, 0));
            p.occurrences = count(&[("Flowing", 1)]);
            &["4"]
        }
        "l04_dissolve_route_cascade.hl" => {
            p.failures.push(fails("Kid", FailureSource::Dissolve, false, false, 0));
            &["4"]
        }
        // One field template, its occurrences built under the two Parents
        // on main and on pool `side`, where the Parent statement's
        // teardown cancels the one run queued behind it; or both on main,
        // the control.
        "l03_field_parents_two_domains.hl" => {
            p.occurrences = count(&[("Parent", 2), ("Leaf", 2)]);
            p.canceled = count(&[("Leaf", 1)]);
            &["3"]
        }
        "l03_field_parents_two_domains_nested.hl" => {
            p.occurrences = count(&[("Parent", 2), ("Leaf", 2), ("Twig", 2)]);
            p.canceled = count(&[("Twig", 1)]);
            &["3"]
        }
        "l03_field_parents_one_domain.hl" => {
            p.occurrences = count(&[("Parent", 2), ("Leaf", 2)]);
            &["3"]
        }
        // One body-literal template, built by a method of the two Mids on
        // main and on pool `side`, each run inline at its statement; or
        // both on main, the control.
        "l03_body_literal_two_domains.hl" => {
            p.occurrences = count(&[("Mid", 2), ("Leaf", 2)]);
            &["3"]
        }
        "l03_body_literal_one_domain.hl" => {
            p.occurrences = count(&[("Mid", 2), ("Leaf", 2)]);
            &["3"]
        }
        "l05_accept_position.hl" => {
            p.occurrences = count(&[("Kid", 1)]);
            &["5"]
        }
        "l06_readiness_main.hl" | "l06_readiness_pool.hl" => &["6"],
        // Line 7 today: the pool join never returns.
        "l07_pool_or_wait_teardown.hl" => {
            p.ends_inside = Some(Inside { decl: None, kind: ObligationKind::PoolJoin, spine: Some(Spine::EagerTeardown) });
            &["7"]
        }
        "l08_birth_failure_kept.hl" => {
            p.failures.push(fails("Kid", FailureSource::BirthClosure, false, false, 0));
            p.occurrences = count(&[("Kid", 1)]);
            &["8"]
        }
        "l09_delivery_at_epoch.hl" => {
            p.failures.push(fails("Kid", FailureSource::Run, false, false, 0));
            p.occurrences = count(&[("Kid", 1)]);
            &["9"]
        }
        // Line 10's fixture runs on line 4's open path (C31): the
        // violation takes the structural exit inside the cascade's
        // dissolve of Kid.
        "l10_dissolve_closures_first.hl" => {
            p.failures.push(fails("Kid", FailureSource::Dissolve, false, false, 0));
            p.ends_inside = Some(Inside { decl: Some("Kid".into()), kind: ObligationKind::Dissolve, spine: None });
            &["10"]
        }
        "l11_let_bound_drain.hl" => &["11"],
        "l12_pinned_fields_drain.hl" => &["12"],
        "l13_resume_pool_child.hl" => {
            p.failures.push(fails("Kid", FailureSource::BirthClosure, true, false, 0));
            &["13"]
        }
        "l14_reclaim_exactly_once.hl" => {
            p.occurrences = count(&[("Kid", 3)]);
            &["14"]
        }
        "l15_sigint_flag.hl" => &["15"],
        "l18_eager_pre_drain.hl" => &["18"],
        "l19_parked_started_coroutine.hl" => {
            p.abandoned.insert("__StdIoTcpListener".to_string());
            &["19"]
        }
        "l19_self_post_overflow.hl" => {
            p.occurrences = count(&[("Kid", 20)]);
            &["19"]
        }
        // R19's retention: each Kid's run, queued behind the teardown
        // that reclaims it, is canceled before it starts.
        "l19_queued_run_canceled.hl" => {
            p.occurrences = count(&[("Own", 1), ("Host", 1), ("Kid", 2)]);
            p.canceled.insert("Kid".to_string(), 2);
            &["19"]
        }
        "l19_resumed_run_at_shutdown.hl" => {
            p.failures.push(fails("Kid", FailureSource::BirthClosure, true, false, 0));
            &["19"]
        }
        // The placed child's started run and the inline replacement
        // both end before their own physical reclaim (line 19).
        "l19_started_run_retained.hl" | "l19_started_run_retained_async.hl"
        => {
            p.occurrences = count(&[("Kid", 2)]);
            &["19"]
        }
        "l19_started_run_publishes_back.hl" | "l19_started_run_publishes_back_async.hl"
        | "l19_handler_replaces_started_run.hl" | "l19_handler_replaces_started_run_async.hl" => {
            p.occurrences = count(&[("Kid", 2), ("Rows", 2)]);
            &["19"]
        }
        "rd_restart_during_teardown.hl" => {
            p.failures.push(fails("Kid", FailureSource::Run, false, true, 0));
            &["RD"]
        }
        "jp_late_failure_pinned_join.hl" | "jp_late_failure_pool_join.hl" => {
            p.failures.push(fails("Late", FailureSource::Run, false, true, 0));
            &["JP", "L0-1"]
        }
        // A pending line's fixture, or an UNDERIVED one: no run path.
        _ => return None,
    };
    Some((lines, p))
}

/// Fixtures with a plan the producer does not derive yet, held to their
/// hand-written one alone: (file, what the producer does not state).
/// L5's second part wrote them after the producer. Each needs a fact a
/// run path cannot carry or a row the producer does not emit: R19a's
/// cancellation at a reclaim off the run's pool, the instances of one
/// declaration whose runs end differently in one run (the trace names
/// an instance by its declaration), and a main locus built more than
/// once, each literal with its own teardown.
const UNDERIVED: &[(&str, &str)] = &[
    (
        "l19_cross_pool_queued_run_canceled.hl",
        "the old Kid's run is canceled by its reclaim on main, the replacement's runs: one declaration, two ends",
    ),
    (
        "l19_full_ring.hl",
        "200 App literals, each torn down where it stands; Kid's runs complete once, then are canceled on main or refused",
    ),
    ("l19_empty_ring_last_check.hl", "two App literals; the first Kid's run completes, the second's is canceled on main"),
];

/// The plan the producer derives for a fixture's program, on its run's
/// path: what the trace oracle holds the run to. `None` for a fixture
/// with no run path.
fn derived_plan(file: &str) -> Option<Expected> {
    let (lines, run) = run_path(file)?;
    let path = dir().join(file);
    let snap = Snapshot::load(&path, LoadMode::WholeSeed, &Disk, Config::check(false, false))
        .unwrap_or_else(|_| panic!("{file} does not load"));
    let plan = snap.demand_lifecycle().unwrap_or_else(|_| panic!("{file}: the lifecycle plan is blocked"));
    Some(project::expected(plan, Focus::Lines(lines), &run).unwrap_or_else(|e| panic!("{file}: {e}")))
}

/// Whether the trace oracle holds a fixture's run to a plan: a run path,
/// or a hand-written plan while it is [`UNDERIVED`].
fn has_plan(file: &str) -> bool {
    run_path(file).is_some() || PLANS.iter().any(|(p, _)| *p == file)
}

/// The plan the trace oracle holds a fixture's run to: its hand-written
/// one while it is [`UNDERIVED`], else the producer's ([`derived_plan`]).
fn fixture_plan(file: &str) -> Option<Expected> {
    match PLANS.iter().find(|(p, _)| *p == file) {
        Some((_, text)) => Some(plan(text)),
        None => derived_plan(file),
    }
}

/// Fixtures whose trace departs from their plan today: (file,
/// inventory row, the departures the trace shows). The departures are
/// the row's defect and its documented consequences, whole violations
/// with the instance number written `_` (the runtime mints it); the
/// trace is asserted to show exactly these, so a violation outside the
/// list fails the fixture as it would any other, and when the fix lands
/// they go, the assertion fails, and so does the entry.
const TRACE_KNOWN_OPEN: &[(&str, &str, &[&str])] = &[
    // The failure raised at Flowing's dissolve takes the structural
    // exit: the process ends inside that dissolve, so nothing owed after
    // it is reached.
    (
        "l04_dissolve_route_reclaim.hl",
        "C25",
        &[
            "missing: Flowing.FailureDelivery",
            "missing: Flowing.Reclaim",
            "missing: App.Drain",
            "missing: App.Dissolve",
            "missing: App.Reclaim",
        ],
    ),
    // The same exit, inside the cascade's dissolve of Kid.
    (
        "l04_dissolve_route_cascade.hl",
        "C31",
        &["missing: Kid.FailureDelivery", "missing: Kid.Reclaim", "missing: App.Reclaim"],
    ),
    // Line 7: the host joins the pools and only then aborts the waits,
    // and the join never returns, so the abort never comes.
    (
        "l07_pool_or_wait_teardown.hl",
        "R34",
        &[
            "edge: -.PoolJoin@EagerTeardown.Entered (process) with -.WaitAbort@EagerTeardown.Completed not reached",
            "missing: -.WaitAbort@EagerTeardown",
        ],
    ),
    // Late declares no run(), and its resumed incarnation enters one.
    ("l01_neg_same_pool_held.hl", "C48", &["count: Late.Run has 1 subjects, owes 0"]),
    // Inner is never drained, so Outer's drain starts without it.
    (
        "l12_pinned_fields_drain.hl",
        "C9",
        &["missing: Inner.Drain", "edge: Outer.Drain.Entered (inst _ inc 0) with Inner.Drain.Completed not reached"],
    ),
    ("l13_resume_pool_child.hl", "C43", &["domain: Kid.Run (inst _ inc 0) ran on main, claimed pool:side"]),
    // Line 18: the step the outcome cannot show, and Sub's drain that
    // should follow it.
    (
        "l18_eager_pre_drain.hl",
        "C13",
        &[
            "missing: -.PreDrain@EagerTeardown",
            "edge: Sub.Drain.Entered (inst _ inc 0) with -.PreDrain@EagerTeardown.Completed not reached",
        ],
    ),
    // The restart performed during teardown begins a second
    // incarnation, born and run.
    (
        "rd_restart_during_teardown.hl",
        "C42",
        &[
            "count: Kid.Restart has 1 subjects, owes 0",
            "count: Kid.Birth has 2 subjects, owes 1",
            "count: Kid.Run has 2 subjects, owes 1",
        ],
    ),
    // Join progress: the late failure completes only because its
    // handler runs in place on the child's thread (decision L0-1).
    (
        "jp_late_failure_pinned_join.hl",
        "C36",
        &["domain: Late.FailureDelivery (inst _ inc 0) ran on pinned:1, claimed main"],
    ),
    (
        "jp_late_failure_pool_join.hl",
        "C36",
        &["domain: Late.FailureDelivery (inst _ inc 0) ran on pool:side, claimed main"],
    ),
];

/// A negative control: a run in which a step is removed or reordered,
/// held to a plan, and asserted to fail the oracle with the violation
/// that says why.
struct Control {
    name: &'static str,
    /// The obligation kind the control shows the oracle needs.
    covers: ObligationKind,
    fixture: &'static str,
    /// `LOTUS_LIFECYCLE_SKIP` for the trace build's runtime; empty when
    /// the reordering is today's own (a rule not yet shipped).
    skip: &'static str,
    /// The plan, written by hand; `None` is the fixture's own, the one
    /// the producer derives ([`derived_plan`]).
    plan: Option<&'static str>,
    /// The violation the oracle reports, as it starts.
    fails_with: &'static str,
    /// Also run the fixture without the skip and hold it to the same
    /// plan, which it must pass: the oracle tells the two apart.
    baseline_passes: bool,
}

/// Line 7's rule over a pool that terminates (`l16`'s worker).
const LINE_7_PLAN: &str = "-: WaitAbort@EagerTeardown PoolJoin@EagerTeardown
     edge -.WaitAbort@EagerTeardown.Completed -> -.PoolJoin@EagerTeardown.Entered";

/// The held failure's restart, without line 13's resumed run (C48,
/// which the fixture's own plan pins).
const RESTART_PLAN: &str = "Late: Birth*2 FailureDelivery!pool:side ConstructionDelivery Restart Drain Dissolve Reclaim
     edge Late.FailureDelivery.Completed -> Late.Restart.Entered";

/// The worker's teardown waits for its run to end: the pool join is
/// what orders the two across threads.
const POOL_WORKER_PLAN: &str = "Worker: Birth Run!pool:side Drain Dissolve Reclaim
     -: PoolJoin@EagerTeardown
     edge Worker.Run.Ended -> Worker.Drain.Entered
     edge Worker.Run.Ended -> -.PoolJoin@EagerTeardown.Completed";

const CONTROLS: &[Control] = &[
    // The first control, the host's own order (inventory decision 7):
    // quiesce, join, then abort. The trace shows the join entered
    // before any wait-abort completed.
    Control {
        name: "host_joins_before_it_aborts_waits",
        covers: ObligationKind::WaitAbort,
        fixture: "l16_eager_spine_pool_join.hl",
        skip: "",
        plan: Some(LINE_7_PLAN),
        fails_with: "edge: -.PoolJoin@EagerTeardown.Entered",
        baseline_passes: false,
    },
    // The deferred-pool-join regression's shape: teardown reaches the
    // worker's fields with its run() still going.
    Control {
        name: "pool_join_removed",
        covers: ObligationKind::PoolJoin,
        fixture: "l16_eager_spine_pool_join.hl",
        skip: "PoolJoin",
        plan: Some(POOL_WORKER_PLAN),
        fails_with: "edge: Worker.Drain.Entered",
        baseline_passes: true,
    },
    // No hold: the handler runs in place while the owner's params are
    // still open, before the settle it must follow.
    Control {
        name: "hold_removed_delivers_before_settle",
        covers: ObligationKind::ConstructionDelivery,
        fixture: "l01_held_failure_settle.hl",
        skip: "ConstructionDelivery",
        plan: None,
        fails_with: "edge: Boom.FailureDelivery.Completed",
        baseline_passes: false,
    },
    // A required completion omitted: the owner's arena goes with no
    // child's reclaim seen to complete.
    Control {
        name: "reclaim_completion_omitted",
        covers: ObligationKind::Reclaim,
        fixture: "l14_reclaim_exactly_once.hl",
        skip: "Reclaim.Completed",
        plan: None,
        fails_with: "unended: Kid.Reclaim",
        baseline_passes: false,
    },
    // A child reclaimed with its handler not seen to complete.
    Control {
        name: "delivery_completion_omitted",
        covers: ObligationKind::FailureDelivery,
        fixture: "l08_birth_failure_kept.hl",
        skip: "FailureDelivery.Completed",
        plan: None,
        fails_with: "edge: Kid.Reclaim.Entered",
        baseline_passes: false,
    },
    Control {
        name: "settle_completion_omitted",
        covers: ObligationKind::ParamsSettle,
        fixture: "l01_held_failure_settle.hl",
        skip: "ParamsSettle.Completed",
        plan: None,
        fails_with: "edge: Boom.FailureDelivery.Completed",
        baseline_passes: false,
    },
    // R20a: without the named terminal the listener's teardown follows
    // a run that never ended.
    Control {
        name: "canceled_run_unnamed",
        covers: ObligationKind::Run,
        fixture: "l19_parked_started_coroutine.hl",
        skip: "Run.Terminal(CanceledAfterStart)",
        plan: None,
        fails_with: "edge: __StdIoTcpListener.Reclaim.Completed",
        baseline_passes: false,
    },
    Control {
        name: "cancellation_completion_omitted",
        covers: ObligationKind::Cancellation,
        fixture: "l19_parked_started_coroutine.hl",
        skip: "Cancellation.Completed",
        plan: None,
        fails_with: "edge: -.PoolJoin@EagerTeardown.Completed",
        baseline_passes: false,
    },
    // R19: a queued run canceled by its child's reclaim and never named
    // is a run the trace cannot account for.
    Control {
        name: "queued_run_cancel_unnamed",
        covers: ObligationKind::Run,
        fixture: "l19_queued_run_canceled.hl",
        skip: "Run.Terminal(NotStarted(Acknowledged))",
        plan: None,
        fails_with: "missing: Kid.Run",
        baseline_passes: true,
    },
    // R19a: without the reclaim's wait for the run hold, the old child's
    // reclaim completes while its started run is still running.
    Control {
        name: "run_hold_wait_removed",
        covers: ObligationKind::Run,
        fixture: "l19_started_run_retained.hl",
        skip: "RunHold",
        plan: None,
        fails_with: "edge: Kid.Reclaim.Completed (inst 1 inc 0) with Kid.Run.Ended not reached",
        baseline_passes: true,
    },
    Control {
        name: "restart_completion_omitted",
        covers: ObligationKind::Restart,
        fixture: "l01_neg_same_pool_held.hl",
        skip: "Restart.Completed",
        plan: Some(RESTART_PLAN),
        fails_with: "unended: Late.Restart",
        baseline_passes: false,
    },
    Control {
        name: "wait_abort_removed",
        covers: ObligationKind::WaitAbort,
        fixture: "l16_eager_spine_pool_join.hl",
        skip: "WaitAbort",
        plan: Some("-: WaitAbort@EagerTeardown WaitAbort@MainFallThrough"),
        fails_with: "missing: -.WaitAbort@EagerTeardown",
        baseline_passes: true,
    },
    Control {
        name: "birth_removed",
        covers: ObligationKind::Birth,
        fixture: "l05_accept_position.hl",
        skip: "Birth",
        plan: None,
        fails_with: "missing: App.Birth",
        baseline_passes: false,
    },
    Control {
        name: "run_removed",
        covers: ObligationKind::Run,
        fixture: "l09_delivery_at_epoch.hl",
        skip: "Run",
        plan: None,
        fails_with: "missing: App.Run",
        baseline_passes: false,
    },
    Control {
        name: "accept_removed",
        covers: ObligationKind::Accept,
        fixture: "l05_accept_position.hl",
        skip: "Accept",
        plan: None,
        fails_with: "missing: Kid.Accept",
        baseline_passes: false,
    },
    Control {
        name: "drain_removed",
        covers: ObligationKind::Drain,
        fixture: "l14_reclaim_exactly_once.hl",
        skip: "Drain",
        plan: None,
        fails_with: "missing: App.Drain",
        baseline_passes: false,
    },
    Control {
        name: "dissolve_removed",
        covers: ObligationKind::Dissolve,
        fixture: "l14_reclaim_exactly_once.hl",
        skip: "Dissolve",
        plan: None,
        fails_with: "missing: App.Dissolve",
        baseline_passes: false,
    },
    Control {
        name: "pre_drain_removed",
        covers: ObligationKind::PreDrain,
        fixture: "l11_let_bound_drain.hl",
        skip: "PreDrain",
        plan: None,
        fails_with: "missing: -.PreDrain@MainFallThrough",
        baseline_passes: false,
    },
    Control {
        name: "pinned_join_removed",
        covers: ObligationKind::PinnedJoin,
        fixture: "l15_sigint_flag.hl",
        skip: "PinnedJoin",
        plan: None,
        fails_with: "missing: Loop.PinnedJoin",
        baseline_passes: false,
    },
    // The oracle reads order within a domain: the same run, held to a
    // plan that claims the accept after the birth.
    Control {
        name: "order_reversed_in_the_plan",
        covers: ObligationKind::Accept,
        fixture: "l05_accept_position.hl",
        skip: "",
        plan: Some("Kid: Birth Accept"),
        fails_with: "order: Kid.Accept before Kid.Birth",
        baseline_passes: false,
    },
];

fn control(name: &str) -> &'static Control {
    CONTROLS.iter().find(|c| c.name == name).unwrap_or_else(|| panic!("{name} is not in CONTROLS"))
}

/// The control's run fails the oracle with the violation it names;
/// with `baseline_passes`, the run without the skip passes the same
/// plan.
fn assert_control(name: &str) {
    let c = control(name);
    let f = fixture(c.fixture);
    let held_to = match c.plan {
        Some(text) => plan(text),
        None => fixture_plan(c.fixture).expect("a plan"),
    };
    let check = |ran: &Ran| -> Vec<String> {
        let mut v = trace::laws(&ran.trace, ran.complete());
        v.extend(held_to.check(&ran.trace, ran.complete()));
        v.iter().map(Violation::to_string).collect()
    };
    let env: Vec<(&str, &str)> = if c.skip.is_empty() { vec![] } else { vec![("LOTUS_LIFECYCLE_SKIP", c.skip)] };
    let ran = run_fixture(f, &env);
    let shown = check(&ran);
    eprintln!("control {name}: {shown:#?}");
    assert!(
        shown.iter().any(|s| s.starts_with(c.fails_with)),
        "control {name}: the oracle does not report `{}`; it reports {shown:#?}",
        c.fails_with
    );
    if c.baseline_passes {
        let ran = run_fixture(f, &[]);
        let shown = check(&ran);
        assert!(shown.is_empty(), "control {name}: without the skip the plan should hold; the oracle reports {shown:#?}");
    }
}

/// What the trace oracle says about a run of `file`: the laws, then the
/// violations of the plan it is held to ([`fixture_plan`]).
fn trace_violations(file: &str, ran: &Ran) -> Vec<Violation> {
    let mut v = trace::laws(&ran.trace, ran.complete());
    if let Some(held_to) = fixture_plan(file) {
        v.extend(held_to.check(&ran.trace, ran.complete()));
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

/// Check one fixture and build it with the lifecycle trace.
fn build_fixture(f: &Fixture) -> PathBuf {
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
    bin
}

/// Build and run one fixture, the runtime given `env`; what it did.
fn run_fixture(f: &Fixture, env: &[(&str, &str)]) -> Ran {
    let bin = build_fixture(f);
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
    ran
}

fn run_bin(bin: &Path, mode: RunMode, env: &[(&str, &str)]) -> Ran {
    let knobs: &[(&str, &str)] = if let RunMode::Env(knobs) = mode { knobs } else { &[] };
    let mut child = Command::new(bin)
        .envs(knobs.iter().copied())
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

fn queued_run_canceled(r: &Ran) -> String {
    let ran = (0..2).filter(|i| count(r, &format!("ev kid-run {i}")) > 0).count();
    let dissolved = (0..2).filter(|i| count(r, &format!("ev kid-dissolve {i}")) == 1).count();
    let named = r
        .trace
        .events
        .iter()
        .filter(|e| e.kind == ObligationKind::Run && e.point == Point::Terminal(Terminal::NotStarted(NotStarted::Acknowledged)))
        .count();
    match (ran, dissolved, named, r.code) {
        (0, 2, 2, Some(0)) => "named:not-started".to_string(),
        (0, 2, 0, Some(0)) => "not-started-unnamed".to_string(),
        _ if r.code != Some(0) => exit_word(r),
        _ => format!("runs {ran}/0, dissolves {dissolved}/2, named {named}/2"),
    }
}

/// The queued child's run never started, the replacement's ran, and
/// each child dissolved once.
fn cross_pool_printed(r: &Ran) -> bool {
    count(r, "ev kid-run 0") == 0
        && count(r, "ev kid-run 1") == 1
        && count(r, "ev kid-dissolve 0") == 1
        && count(r, "ev kid-dissolve 1") == 1
}

/// Both children's runs read their own names, and each child is torn
/// down once.
fn started_printed(r: &Ran) -> bool {
    count(r, "ev kid-run 0 kid-0-name") == 1
        && count(r, "ev kid-run 1 kid-1-name") == 1
        && count(r, "ev kid-dissolve 0") == 1
        && count(r, "ev kid-dissolve 1") == 1
}

/// The started run returned, reading its own child, and the trace's
/// plan holds its end before its child's reclaim completes.
fn run_held(r: &Ran) -> String {
    match (started_printed(r), r.code) {
        (true, Some(0)) => "run-held".to_string(),
        _ if r.code != Some(0) => exit_word(r),
        _ => "printed otherwise".to_string(),
    }
}

/// As [`started_printed`], and App's handler heard the run's note on
/// main before the reassignment completed: inside the reclaim's wait.
fn answered_in_wait_printed(r: &Ran) -> bool {
    let heard = r.stdout.find("ev heard 0");
    let replaced = r.stdout.find("ev replaced");
    started_printed(r)
        && count(r, "ev rows 0 1") == 1 && count(r, "ev rows 1 1") == 1
        && matches!((heard, replaced), (Some(h), Some(p)) if h < p)
}

fn answered_in_wait(r: &Ran) -> String {
    match (answered_in_wait_printed(r), r.code) {
        (true, Some(0)) => "answered-in-wait".to_string(),
        _ if r.code != Some(0) => exit_word(r),
        _ => "printed otherwise".to_string(),
    }
}

fn answered_after_handler_printed(r: &Ran) -> bool {
    started_printed(r)
        && count(r, "ev rows 0 1") == 1
        && count(r, "ev rows 1 1") == 1
        && count(r, "ev scratch-dissolve") == 1
        && matches!((pos(r, "ev scratch-dissolve"), pos(r, "ev replaced")), (Some(d), Some(p)) if d < p)
        && matches!((pos(r, "ev replaced"), pos(r, "ev heard 0")), (Some(p), Some(h)) if p < h)
        && matches!((pos(r, "ev kid-dissolve 0"), pos(r, "ev kid-run 1 kid-1-name")), (Some(d), Some(n)) if d < n)
}

fn answered_after_handler(r: &Ran) -> String {
    match (answered_after_handler_printed(r), r.code) {
        (true, Some(0)) => "answered-after-handler".to_string(),
        _ if r.code != Some(0) => exit_word(r),
        _ => "printed otherwise".to_string(),
    }
}

/// The run queued on `side` is canceled by the reclaim on main and
/// named there.
fn cross_pool_run_canceled(r: &Ran) -> String {
    let named: Vec<&str> = r
        .trace
        .events
        .iter()
        .filter(|e| e.kind == ObligationKind::Run && e.point == Point::Terminal(Terminal::NotStarted(NotStarted::Acknowledged)))
        .map(|e| e.domain.as_str())
        .collect();
    match (cross_pool_printed(r), named.as_slice(), r.code) {
        (true, ["main"], Some(0)) => "named:not-started".to_string(),
        (true, [], Some(0)) => "not-started-unnamed".to_string(),
        _ if r.code != Some(0) => exit_word(r),
        (printed, named, _) => format!("printed as owed: {printed}, named on {named:?}"),
    }
}

/// How many runs the trace names as ending in `t`.
fn runs_ending(r: &Ran, t: Terminal) -> usize {
    r.trace.events.iter().filter(|e| e.kind == ObligationKind::Run && e.point == Point::Terminal(t)).count()
}

const ACKNOWLEDGED: Terminal = Terminal::NotStarted(NotStarted::Acknowledged);
const REFUSED: Terminal = Terminal::NotStarted(NotStarted::Shutdown(ShutdownCause::PoolShutdown));
const FREED_AT_TEARDOWN: Terminal = Terminal::NotStarted(NotStarted::Shutdown(ShutdownCause::PoolTeardown));

/// The first of 200 runs completes; each later one is canceled or
/// refused, named either way, and both happen (the ring filled).
fn full_ring(r: &Ran) -> String {
    if r.code != Some(0) {
        return exit_word(r);
    }
    let (ran, dissolved) = (count(r, "ev kid-run"), count(r, "ev kid-dissolve"));
    let (canceled, refused) = (runs_ending(r, ACKNOWLEDGED), runs_ending(r, REFUSED));
    match (ran, dissolved, canceled, refused) {
        (1, 200, c, f) if c > 0 && f > 0 && c + f == 199 => "admitted-or-named".to_string(),
        _ => format!("runs {ran}, dissolves {dissolved}/200, canceled {canceled}, refused {refused}"),
    }
}

/// The first run completes; the second, admitted after the worker's last
/// check, is canceled by its child's reclaim and named.
fn empty_ring(r: &Ran) -> String {
    if r.code != Some(0) {
        return exit_word(r);
    }
    let (ran, dissolved) = (count(r, "ev kid-run"), count(r, "ev kid-dissolve"));
    match (ran, dissolved, runs_ending(r, ACKNOWLEDGED), runs_ending(r, FREED_AT_TEARDOWN)) {
        (1, 2, 1, 0) => "admitted-or-named".to_string(),
        (1, 2, 0, 0) => "freed-unrun-silently".to_string(),
        (ran, dissolved, c, t) => format!("runs {ran}, dissolves {dissolved}/2, canceled {c}, freed at teardown {t}"),
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

/// Where each occurrence of `decl` was born: on its own parent's domain,
/// one on main and one on pool `side`, or both on main.
fn born_where(r: &Ran, decl: &str) -> String {
    if r.timed_out || r.code != Some(0) {
        return exit_word(r);
    }
    let mut on: Vec<&str> = r
        .trace
        .events
        .iter()
        .filter(|e| e.kind == ObligationKind::Birth && e.point == Point::Entered && e.decl.as_deref() == Some(decl))
        .map(|e| e.domain.as_str())
        .collect();
    on.sort();
    match on[..] {
        ["main", side] if side.starts_with("pool:side") => "born-on-each-parents-domain".to_string(),
        ["main", "main"] => "born-on-main".to_string(),
        _ => format!("{decl} born on {on:?}"),
    }
}

fn leaf_born(r: &Ran) -> String {
    born_where(r, "Leaf")
}

fn twig_born(r: &Ran) -> String {
    born_where(r, "Twig")
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
    // Every adopted line has a plan, the producer's through a run path
    // or, while it is UNDERIVED, a hand-written one, never both; a
    // pending fixture has none; every plan owes something.
    for f in FIXTURES {
        assert_eq!(has_plan(f.file), f.adopted.is_some(), "{}: a plan is owed exactly by an adopted line", f.file);
        if let Some(held_to) = fixture_plan(f.file) {
            assert!(!held_to.owed.is_empty(), "{}: an empty plan", f.file);
        }
    }
    let hand_written: Vec<&str> = PLANS.iter().map(|(p, _)| *p).collect();
    let underived: Vec<&str> = UNDERIVED.iter().map(|(p, _)| *p).collect();
    assert_eq!(hand_written, underived, "the hand-written plans are exactly the UNDERIVED fixtures'");
    for file in underived {
        assert!(run_path(file).is_none(), "{file} is UNDERIVED but has a run path: its hand-written plan goes");
    }
    for (file, _, departures) in TRACE_KNOWN_OPEN {
        assert!(has_plan(file), "{file} is TRACE_KNOWN_OPEN but has no plan");
        assert!(!departures.is_empty(), "{file} is TRACE_KNOWN_OPEN with no departure");
        for d in *departures {
            assert_eq!(normalized(d), *d, "{file}: a departure names its instance as `_`");
        }
    }
}

/// Every obligation kind a plan holds a run to has a negative control,
/// and a control held to its fixture's own plan starts from a run that
/// passes it.
#[test]
fn every_planned_kind_has_a_negative_control() {
    let mut planned: Vec<ObligationKind> = FIXTURES
        .iter()
        .filter_map(|f| fixture_plan(f.file))
        .flat_map(|p| p.owed.into_iter().map(|o| o.kind))
        .collect();
    planned.sort();
    planned.dedup();
    for kind in planned {
        assert!(CONTROLS.iter().any(|c| c.covers == kind), "no negative control covers {}", kind.name());
    }
    for c in CONTROLS {
        fixture(c.fixture);
        match c.plan {
            Some(text) => assert!(!plan(text).owed.is_empty(), "control {}: an empty plan", c.name),
            None => {
                assert!(has_plan(c.fixture), "control {}: {} has no plan", c.name, c.fixture);
                assert!(
                    !TRACE_KNOWN_OPEN.iter().any(|(p, _, _)| *p == c.fixture),
                    "control {}: {} fails its own plan already",
                    c.name,
                    c.fixture
                );
            }
        }
    }
}

/// The projected expectations of the field-template fixtures, in the
/// notation: a template reached under parents on main and on pool
/// `side` claims the set for its birth and run, two levels down as one,
/// where the template that kept its first parent's context only claimed
/// `Birth*2!pool:side Run*2!pool:side`, false of the occurrence on main;
/// the side occurrence's run is canceled behind its parent's teardown
/// and the main one's completes (`=Ended`). The edges between the
/// Parents and their fields are not held: two occurrences of each, and
/// the trace does not say which is whose. The control, both parents on
/// main, claims main alone. Every run is held until reclaim completes.
const FIELD_PLANS: &[(&str, &str)] = &[
    (
        "l03_field_parents_two_domains.hl",
        "App: Birth!main Run!main Drain Dissolve Reclaim\n\
         Worker: Birth Run!pool:side Drain Dissolve Reclaim\n\
         Parent: Birth*2 Drain*2 Dissolve*2 Reclaim*2\n\
         Leaf: Birth*2!{main,pool:side} Run*2=Ended!{main,pool:side} Drain*2 Dissolve*2 Reclaim*2 Cancellation!pool:side\n\
         edge App.Run.Ended -> App.Drain.Entered\n\
         edge App.Run.Ended -> App.Reclaim.Completed\n\
         edge App.Dissolve.Completed -> Worker.Dissolve.Entered\n\
         edge Worker.Birth.Completed -> App.Birth.Entered\n\
         edge Worker.Run.Ended -> Worker.Reclaim.Completed\n\
         edge Worker.Drain.Completed -> App.Drain.Entered\n\
         edge Worker.Dissolve.Completed -> App.Reclaim.Entered\n\
         edge Worker.Reclaim.Completed -> App.Reclaim.Completed\n\
         edge Leaf.Run.Ended -> Leaf.Reclaim.Completed\n\
         edge Leaf.Reclaim.Entered -> Leaf.Cancellation.Entered",
    ),
    (
        "l03_field_parents_two_domains_nested.hl",
        "App: Birth!main Run!main Drain Dissolve Reclaim\n\
         Worker: Birth Run!pool:side Drain Dissolve Reclaim\n\
         Parent: Birth*2 Drain*2 Dissolve*2 Reclaim*2\n\
         Leaf: Birth*2!{main,pool:side} Drain*2 Dissolve*2 Reclaim*2\n\
         Twig: Birth*2!{main,pool:side} Run*2=Ended!{main,pool:side} Drain*2 Dissolve*2 Reclaim*2 Cancellation!pool:side\n\
         edge App.Run.Ended -> App.Drain.Entered\n\
         edge App.Run.Ended -> App.Reclaim.Completed\n\
         edge App.Dissolve.Completed -> Worker.Dissolve.Entered\n\
         edge Worker.Birth.Completed -> App.Birth.Entered\n\
         edge Worker.Run.Ended -> Worker.Reclaim.Completed\n\
         edge Worker.Drain.Completed -> App.Drain.Entered\n\
         edge Worker.Dissolve.Completed -> App.Reclaim.Entered\n\
         edge Worker.Reclaim.Completed -> App.Reclaim.Completed\n\
         edge Twig.Run.Ended -> Twig.Reclaim.Completed\n\
         edge Twig.Reclaim.Entered -> Twig.Cancellation.Entered",
    ),
    (
        "l03_field_parents_one_domain.hl",
        "App: Birth!main Run!main Drain Dissolve Reclaim\n\
         Worker: Birth!main Run!main Drain Dissolve Reclaim\n\
         Parent: Birth*2!main Drain*2 Dissolve*2 Reclaim*2\n\
         Leaf: Birth*2!main Run*2!main Drain*2 Dissolve*2 Reclaim*2\n\
         edge App.Run.Ended -> App.Drain.Entered\n\
         edge App.Run.Ended -> App.Reclaim.Completed\n\
         edge App.Dissolve.Completed -> Worker.Dissolve.Entered\n\
         edge Worker.Birth.Completed -> App.Birth.Entered\n\
         edge Worker.Run.Ended -> Worker.Drain.Entered\n\
         edge Worker.Run.Ended -> Worker.Reclaim.Completed\n\
         edge Worker.Drain.Completed -> App.Drain.Entered\n\
         edge Worker.Dissolve.Completed -> App.Reclaim.Entered\n\
         edge Worker.Reclaim.Completed -> App.Reclaim.Completed\n\
         edge Leaf.Run.Ended -> Leaf.Drain.Entered\n\
         edge Leaf.Run.Ended -> Leaf.Reclaim.Completed",
    ),
];

/// The projected expectations of the body-literal fixtures: a literal a
/// method of `Mid` builds, with Mids on main and on pool `side`, claims
/// the set for its birth and run, where the template that kept its
/// first enclosing Mid as its only owner claimed neither (the scope's
/// two domains named no one). Each occurrence's run() runs inline at its
/// statement and ends before its drain, on side as on main: a body
/// literal's run is never posted behind its owner's teardown. The
/// control, both Mids on main, retains that inline ordering. Both also
/// carry the started-run retention edge through physical reclaim.
const BODY_PLANS: &[(&str, &str)] = &[
    (
        "l03_body_literal_two_domains.hl",
        "App: Birth!main Run!main Drain Dissolve Reclaim\n\
         Worker: Birth Run!pool:side Drain Dissolve Reclaim\n\
         Leaf: Birth*2!{main,pool:side} Run*2!{main,pool:side} Drain*2 Dissolve*2 Reclaim*2\n\
         Mid: Birth*2 Drain*2 Dissolve*2 Reclaim*2\n\
         edge App.Run.Ended -> App.Drain.Entered\n\
         edge App.Run.Ended -> App.Reclaim.Completed\n\
         edge App.Dissolve.Completed -> Worker.Dissolve.Entered\n\
         edge Worker.Birth.Completed -> App.Birth.Entered\n\
         edge Worker.Run.Ended -> Worker.Reclaim.Completed\n\
         edge Worker.Drain.Completed -> App.Drain.Entered\n\
         edge Worker.Dissolve.Completed -> App.Reclaim.Entered\n\
         edge Worker.Reclaim.Completed -> App.Reclaim.Completed\n\
         edge Leaf.Run.Ended -> Leaf.Drain.Entered\n\
         edge Leaf.Run.Ended -> Leaf.Reclaim.Completed",
    ),
    (
        "l03_body_literal_one_domain.hl",
        "App: Birth!main Run!main Drain Dissolve Reclaim\n\
         Worker: Birth!main Run!main Drain Dissolve Reclaim\n\
         Leaf: Birth*2!main Run*2!main Drain*2 Dissolve*2 Reclaim*2\n\
         Mid: Birth*2!main Drain*2 Dissolve*2 Reclaim*2\n\
         edge App.Run.Ended -> App.Drain.Entered\n\
         edge App.Run.Ended -> App.Reclaim.Completed\n\
         edge App.Dissolve.Completed -> Worker.Dissolve.Entered\n\
         edge Worker.Birth.Completed -> App.Birth.Entered\n\
         edge Worker.Run.Ended -> Worker.Drain.Entered\n\
         edge Worker.Run.Ended -> Worker.Reclaim.Completed\n\
         edge Worker.Drain.Completed -> App.Drain.Entered\n\
         edge Worker.Dissolve.Completed -> App.Reclaim.Entered\n\
         edge Worker.Reclaim.Completed -> App.Reclaim.Completed\n\
         edge Leaf.Run.Ended -> Leaf.Drain.Entered\n\
         edge Leaf.Run.Ended -> Leaf.Reclaim.Completed",
    ),
];

#[test]
fn a_body_literal_under_enclosing_templates_claims_their_domains() {
    let mut differ = Vec::new();
    for (file, want) in BODY_PLANS {
        let got = render(&derived_plan(file).expect("a plan"));
        if got != *want {
            differ.push(format!("{file}:\n{got}"));
        }
    }
    assert!(differ.is_empty(), "the projected plans differ:\n{}", differ.join("\n\n"));
}

#[test]
fn a_field_reached_under_two_parents_claims_their_domains() {
    let mut differ = Vec::new();
    for (file, want) in FIELD_PLANS {
        let got = render(&derived_plan(file).expect("a plan"));
        if got != *want {
            differ.push(format!("{file}:\n{got}"));
        }
    }
    assert!(differ.is_empty(), "the projected plans differ:\n{}", differ.join("\n\n"));
}

/// The trace oracle over one run: clean, or, for a known-open trace,
/// showing exactly the departures its entry names.
fn assert_trace(file: &str, ran: &Ran) {
    let v = trace_violations(file, ran);
    let shown: Vec<String> = v.iter().map(|v| normalized(&v.to_string())).collect();
    eprintln!("{file}: trace oracle: {shown:#?}");
    let Some((_, row, departures)) = TRACE_KNOWN_OPEN.iter().find(|(k, _, _)| *k == file) else {
        assert!(shown.is_empty(), "{file}: the trace oracle fails: {shown:#?}");
        return;
    };
    let unmatched: Vec<&String> = shown.iter().filter(|s| !departures.contains(&s.as_str())).collect();
    assert!(
        unmatched.is_empty(),
        "{file}: the trace oracle fails beyond inventory row {row}'s departures: {unmatched:#?}"
    );
    for d in *departures {
        assert!(
            shown.iter().any(|s| s == d),
            "{file}: the trace no longer shows `{d}`: the fix for inventory row {row} has landed, so its TRACE_KNOWN_OPEN entry has to change or go; it shows {shown:#?}"
        );
    }
}

fn assert_fixture(file: &str) {
    let f = fixture(file);
    let ran = run_fixture(f, &[]);
    let got = (f.judge)(&ran);
    eprintln!("{file}: outcome {got}");
    assert_trace(file, &ran);
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
    l03_field_parents_two_domains => "l03_field_parents_two_domains.hl",
    l03_field_parents_two_domains_nested => "l03_field_parents_two_domains_nested.hl",
    l03_field_parents_one_domain => "l03_field_parents_one_domain.hl",
    l03_body_literal_two_domains => "l03_body_literal_two_domains.hl",
    l03_body_literal_one_domain => "l03_body_literal_one_domain.hl",
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
    l19_queued_run_canceled => "l19_queued_run_canceled.hl",
    l19_cross_pool_queued_run_canceled => "l19_cross_pool_queued_run_canceled.hl",
    l19_started_run_retained => "l19_started_run_retained.hl",
    l19_started_run_retained_async => "l19_started_run_retained_async.hl",
    l19_started_run_publishes_back => "l19_started_run_publishes_back.hl",
    l19_started_run_publishes_back_async => "l19_started_run_publishes_back_async.hl",
    l19_handler_replaces_started_run => "l19_handler_replaces_started_run.hl",
    l19_handler_replaces_started_run_async => "l19_handler_replaces_started_run_async.hl",
    l19_resumed_run_at_shutdown => "l19_resumed_run_at_shutdown.hl",
    l19_full_ring => "l19_full_ring.hl",
    l19_empty_ring_last_check => "l19_empty_ring_last_check.hl",
    rd_restart_during_teardown => "rd_restart_during_teardown.hl",
    jp_late_failure_pinned_join => "jp_late_failure_pinned_join.hl",
    jp_late_failure_pool_join => "jp_late_failure_pool_join.hl",
}

/// Decision line 19's last unrun path: a cell the pools' teardown frees
/// with its run never ended. Every reclaim cancels its child's runs
/// first, so a well-formed program reaches the teardown with none; the
/// trace build's `LOTUS_LIFECYCLE_SKIP=Cancellation` removes the
/// reclaim's cancel on the empty-ring fixture, whose admitted cell no
/// worker dequeues, and the teardown then names the run not started for
/// its own reason, never silent. The plan's cancellation inside the
/// reclaim is what goes missing (and the trace's subject, retired at the
/// reclaim, is named again by an address nothing built: the step on a
/// reclaimed struct the cancel exists to prevent).
#[test]
fn pool_teardown_names_a_cell_no_reclaim_canceled() {
    let file = "l19_empty_ring_last_check.hl";
    let ran = run_fixture(fixture(file), &[("LOTUS_LIFECYCLE_SKIP", "Cancellation")]);
    assert_eq!(ran.code, Some(0), "{file} without the cancel: {}\n{}", exit_word(&ran), ran.stderr);
    assert_eq!((count(&ran, "ev kid-run"), count(&ran, "ev kid-dissolve")), (1, 2), "{file} without the cancel:\n{}", ran.stdout);
    let at_teardown: Vec<&str> = ran
        .trace
        .events
        .iter()
        .filter(|e| e.kind == ObligationKind::Run && e.point == Point::Terminal(FREED_AT_TEARDOWN))
        .map(|e| e.domain.as_str())
        .collect();
    assert_eq!(at_teardown, ["main"], "{file} without the cancel: the run freed at the pools' teardown is not named once, on main");
    assert_eq!(runs_ending(&ran, ACKNOWLEDGED), 0, "{file}: the skip did not remove the reclaim's cancel");
    let shown: Vec<String> = trace_violations(file, &ran).iter().map(Violation::to_string).collect();
    assert!(
        shown.iter().any(|s| s == "missing: Kid.Cancellation"),
        "{file} without the cancel: the plan should miss the cancellation inside the reclaim; the oracle reports {shown:#?}"
    );
}

const SANITIZER_MARKERS: &[&str] = &[
    "ERROR: AddressSanitizer",
    "ERROR: LeakSanitizer",
    "heap-use-after-free",
    "double-free",
    "heap-buffer-overflow",
    "SEGV on unknown address",
];

/// Decision line 19 across pools, under AddressSanitizer with chunk
/// pooling off (GH #816): the worker on `side` dequeues the cell whose
/// child main reclaimed and drops it without a step on the struct.
#[test]
fn l19_cross_pool_queued_run_canceled_under_asan() {
    let file = "l19_cross_pool_queued_run_canceled.hl";
    let program = hale_syntax::parse_source(&source(file)).unwrap_or_else(|e| panic!("{file}: parse: {e:?}"));
    let bin = harness::unique_bin("hale_lifecycle_asan_l19_cross_pool");
    harness::build_asan(&program, &bin);
    let ran = run_bin(&bin, RunMode::Plain, &[("ASAN_OPTIONS", "detect_leaks=1"), ("LOTUS_NO_CHUNK_POOL", "1")]);
    let _ = std::fs::remove_file(&bin);
    let report = [ran.stdout.as_str(), ran.stderr.as_str()].concat();
    let hits: Vec<&str> = SANITIZER_MARKERS.iter().copied().filter(|m| report.contains(m)).collect();
    assert!(hits.is_empty(), "{file} under ASan: {hits:?}\n{report}");
    assert!(cross_pool_printed(&ran) && ran.code == Some(0), "{file} under ASan, {}:\n{report}", exit_word(&ran));
}

/// Decision line 19, a started run, under AddressSanitizer with chunk
/// pooling off (GH #816): main reclaims a child whose run is running on
/// `side`, and the run then reads the heap String in the child's arena.
/// Before the run hold the reclaim released that arena under the run (a
/// heap-use-after-free); with it the reclaim waits for the run to return.
fn assert_clean_under_asan(file: &str, tag: &str, printed: fn(&Ran) -> bool) {
    let program = hale_syntax::parse_source(&source(file)).unwrap_or_else(|e| panic!("{file}: parse: {e:?}"));
    let bin = harness::unique_bin(&format!("hale_lifecycle_asan_{tag}"));
    harness::build_asan(&program, &bin);
    let ran = run_bin(&bin, RunMode::Plain, &[("ASAN_OPTIONS", "detect_leaks=1"), ("LOTUS_NO_CHUNK_POOL", "1")]);
    let _ = std::fs::remove_file(&bin);
    let report = [ran.stdout.as_str(), ran.stderr.as_str()].concat();
    let hits: Vec<&str> = SANITIZER_MARKERS.iter().copied().filter(|m| report.contains(m)).collect();
    assert!(hits.is_empty(), "{file} under ASan: {hits:?}\n{report}");
    assert!(printed(&ran) && ran.code == Some(0), "{file} under ASan, {}:\n{report}", exit_word(&ran));
}

#[test]
fn l19_started_run_retained_under_asan() {
    assert_clean_under_asan("l19_started_run_retained.hl", "l19_started", started_printed);
}

#[test]
fn l19_started_run_retained_async_under_asan() {
    assert_clean_under_asan("l19_started_run_retained_async.hl", "l19_started_async", started_printed);
}

#[test]
fn l19_started_run_publishes_back_under_asan() {
    assert_clean_under_asan("l19_started_run_publishes_back.hl", "l19_publishes_back", answered_in_wait_printed);
}

#[test]
fn l19_started_run_publishes_back_async_under_asan() {
    assert_clean_under_asan("l19_started_run_publishes_back_async.hl", "l19_publishes_back_async", answered_in_wait_printed);
}

/// A handler replacement and its run-body control, on classic and async
/// pools, in both dispatch modes. The trace holds memory retention to
/// the producer's plan; stdout separately holds handler completion order.
#[test]
fn handler_reclaim_and_run_control_under_asan_both_dispatch_modes() {
    for file in [
        "l19_handler_replaces_started_run.hl",
        "l19_handler_replaces_started_run_async.hl",
        "l19_started_run_publishes_back.hl",
        "l19_started_run_publishes_back_async.hl",
    ] {
        let f = fixture(file);
        let program = hale_syntax::parse_source(&source(file)).expect("parse the fixture");
        for no_bus_devirt in [false, true] {
            let bin = harness::unique_bin("hale_handler_reclaim_asan");
            let options = hale_codegen::BuildOptions {
                asan: true, lifecycle_trace: true, no_bus_devirt, ..build_opts::options()
            };
            build_executable_with_options(&program, &bin, &[], &options).expect("ASan build");
            let image = std::fs::read(&bin).expect("read ASan binary");
            assert!(image.windows(b"__asan_init".len()).any(|w| w == b"__asan_init"), "ASan instrumentation is required");
            let ran = run_bin(&bin, RunMode::Plain, &[("ASAN_OPTIONS", "detect_leaks=1"), ("LOTUS_NO_CHUNK_POOL", "1")]);
            let _ = std::fs::remove_file(&bin);
            let report = format!("{}\n{}", ran.stdout, ran.stderr);
            for marker in SANITIZER_MARKERS {
                assert!(!report.contains(marker), "{file}, no_bus_devirt={no_bus_devirt}: {report}");
            }
            assert_eq!(Some((f.judge)(&ran).as_str()), f.adopted, "{file}, no_bus_devirt={no_bus_devirt}: {report}");
            assert_trace(file, &ran);
        }
    }
}

/// Removing the handler boundary restores the original deadlock: the
/// child has dissolved, but replacement cannot finish and the queued
/// reply cannot run. The ASan cases above are its completing controls.
#[test]
fn synchronous_handler_storage_wait_restores_the_deadlock() {
    let program = hale_syntax::parse_source(&source("l19_handler_replaces_started_run.hl")).expect("parse");
    let bin = harness::unique_bin("hale_handler_storage_negative");
    let options = hale_codegen::BuildOptions { lifecycle_trace: true, ..build_opts::options() };
    build_executable_with_options(&program, &bin, &[], &options).expect("build");
    let ran = run_bin(&bin, RunMode::Plain, &[("LOTUS_LIFECYCLE_SKIP", "HandlerStorage")]);
    let _ = std::fs::remove_file(&bin);
    assert!(ran.timed_out, "synchronous handler wait unexpectedly completed: {} {}", ran.stdout, ran.stderr);
    assert_eq!(count(&ran, "ev kid-dissolve 0"), 1);
    assert_eq!(count(&ran, "ev replaced"), 0);
    assert_eq!(count(&ran, "ev heard 0"), 0);
}

#[test]
fn owner_release_cannot_finish_before_its_child_release() {
    let file = "l14_reclaim_exactly_once.hl";
    let mut ran = run_fixture(fixture(file), &[]);
    assert_trace(file, &ran);
    let parent = ran.trace.events.iter().find(|e| e.decl.as_deref() == Some("App")
        && e.kind == ObligationKind::Reclaim && e.point == Point::Completed).expect("owner completion").seq;
    let child = ran.trace.events.iter_mut().find(|e| e.decl.as_deref() == Some("Kid")
        && e.kind == ObligationKind::Reclaim && e.point == Point::Completed).expect("child completion");
    child.seq = parent + 1;
    ran.trace.events.sort_by_key(|e| e.seq);
    let violations = trace_violations(file, &ran);
    assert!(violations.iter().any(|v| v.to_string().starts_with("edge: App.Reclaim.Completed")), "{violations:#?}");
}

macro_rules! control_tests {
    ($($name:ident),* $(,)?) => {
        mod controls {
            $(
                #[test]
                fn $name() { super::assert_control(stringify!($name)); }
            )*
        }
    };
}

control_tests! {
    host_joins_before_it_aborts_waits,
    pool_join_removed,
    hold_removed_delivers_before_settle,
    reclaim_completion_omitted,
    delivery_completion_omitted,
    settle_completion_omitted,
    canceled_run_unnamed,
    cancellation_completion_omitted,
    queued_run_cancel_unnamed,
    run_hold_wait_removed,
    restart_completion_omitted,
    wait_abort_removed,
    birth_removed,
    run_removed,
    accept_removed,
    drain_removed,
    dissolve_removed,
    pre_drain_removed,
    pinned_join_removed,
    order_reversed_in_the_plan,
}

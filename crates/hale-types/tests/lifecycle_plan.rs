//! The lifecycle plan (F.40 phase 3, L1): the `lifecycle_order`
//! family's producer, `hale_types::lifecycle::derive`, through the
//! frontend's snapshot.
//!
//! The plan's own laws hold over every corpus program the snapshot
//! scopes: every edge names a row of the plan, the edges are acyclic on
//! events, every instance owes its birth and its teardown once, every
//! row names a decision line the table has, and every failure delivered
//! to an owner whose domain is known names the domain it owes. The rows
//! the decision lines are about are pinned on small programs.

use std::collections::{BTreeMap, BTreeSet};

use hale_frontend::snapshot::{Config, Snapshot};
use hale_types::lifecycle::{
    FailureSource, LifecyclePlan, NotStarted, Obligation, ObligationKind as K, PathGuard, Point, Spine, Status,
    Template, Terminal, DECISION_LINES,
};
use hale_types::lifecycle::spine::BIRTH_KINDS;
use hale_types::placement::{DomainKind, SiteUniverse};

fn snapshot(src: &str) -> Snapshot {
    let program = hale_syntax::parse_source(src).unwrap_or_else(|e| panic!("parse: {e:?}"));
    Snapshot::from_program(program, Vec::new(), Config::check(false, false)).unwrap_or_else(|_| panic!("no snapshot"))
}

fn plan(s: &Snapshot) -> &LifecyclePlan {
    s.demand_lifecycle().unwrap_or_else(|_| panic!("the lifecycle plan is blocked"))
}

fn rows<'p>(p: &'p LifecyclePlan, decl: &str, kind: K) -> Vec<&'p Obligation> {
    p.obligations
        .iter()
        .filter(|o| o.kind == kind && o.site.as_ref().is_some_and(|s| s.decl.lowered == decl))
        .collect()
}

fn one<'p>(p: &'p LifecyclePlan, decl: &str, kind: K) -> &'p Obligation {
    let r = rows(p, decl, kind);
    assert_eq!(r.len(), 1, "{decl} owes one {}, not {}", kind.name(), r.len());
    r[0]
}

/// What a plan breaks of its own laws.
fn laws(p: &LifecyclePlan) -> Vec<String> {
    let mut out = Vec::new();
    let lines: BTreeSet<&str> = DECISION_LINES.iter().map(|l| l.line).chain(["L0-1"]).collect();
    // Events as nodes: (obligation, entered?) with entered -> ended, and
    // each prerequisite's point before its dependant's.
    let node = |id: u32, entered: bool| (id as usize) * 2 + usize::from(!entered);
    let n = p.obligations.len() * 2;
    let mut next: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (id, o) in p.iter() {
        next[node(id.0, true)].push(node(id.0, false));
        for (prereqs, at_entry) in [(&o.edges.entry, true), (&o.edges.completion, false)] {
            for pr in prereqs {
                if p.get(pr.event.obligation).is_none() {
                    out.push(format!("{} names {:?}, which is not in the plan", o.kind.name(), pr.event.obligation));
                    continue;
                }
                let from = node(pr.event.obligation.0, pr.event.point == Point::Entered);
                next[from].push(node(id.0, at_entry));
            }
        }
        if let Some(l) = o.line {
            if !lines.contains(l) {
                out.push(format!("{} names line {l}", o.kind.name()));
            }
        }
        for pr in o.edges.entry.iter().chain(&o.edges.completion) {
            if let Some(l) = pr.rule.line {
                if !lines.contains(l) {
                    out.push(format!("an edge into {} names line {l}", o.kind.name()));
                }
            }
        }
        if o.terminals.is_empty() {
            out.push(format!("{} has no terminal", o.kind.name()));
        }
        // A failure delivered to an owner whose domain is known names it.
        if o.kind == K::FailureDelivery && o.guard != PathGuard::FailedAtSettle && o.runs_on.is_none() {
            out.push(format!(
                "{}'s delivery of a {} failure names no domain",
                o.site.as_ref().map(|s| s.decl.lowered.as_str()).unwrap_or("-"),
                o.source.map(FailureSource::name).unwrap_or("-")
            ));
        }
    }
    // Acyclic: a depth-first walk finds no back edge.
    let mut state = vec![0u8; n];
    fn visit(v: usize, next: &[Vec<usize>], state: &mut [u8]) -> bool {
        state[v] = 1;
        for &w in &next[v] {
            if state[w] == 1 || (state[w] == 0 && !visit(w, next, state)) {
                return false;
            }
        }
        state[v] = 2;
        true
    }
    for v in 0..n {
        if state[v] == 0 && !visit(v, &next, &mut state) {
            out.push("the edges have a cycle".to_string());
            break;
        }
    }
    // Every instance owes its birth and its teardown once.
    for inst in &p.instances {
        for kind in [K::Birth, K::Drain, K::Dissolve, K::Reclaim] {
            let n = p.obligations.iter().filter(|o| o.kind == kind && o.site.as_ref() == Some(&inst.site)).count();
            if n != 1 {
                out.push(format!("{} owes {n} {}", inst.site.decl.lowered, kind.name()));
            }
        }
    }
    out
}

/// The plan is a family of the snapshot: demanded, it runs once, and a
/// check demands it not at all (no consumer reads it yet).
#[test]
fn the_plan_is_demanded_once_and_no_check_builds_it() {
    let s = snapshot("locus Kid { run() { } }\nmain locus App { params { k: Kid = Kid { }; } }\nfn main() { App { }; }\n");
    let errors: Vec<String> = s.demand_check().expect("checked").diags.iter().filter(|d| d.is_error()).map(|d| d.message.clone()).collect();
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(s.builds()["lifecycle_order"], 0, "the check reads no lifecycle plan");
    let first: *const LifecyclePlan = plan(&s);
    let again: *const LifecyclePlan = plan(&s);
    assert_eq!(first, again);
    assert_eq!(s.builds()["lifecycle_order"], 1);
    assert_eq!(s.builds()["placement"], 1, "the plan demands the placement table once");
}

/// The plan's laws over every corpus program the snapshot scopes.
#[test]
fn the_plans_laws_hold_over_the_corpus() {
    let mut programs = 0;
    let mut instances = 0;
    let mut rows = 0;
    let mut by_kind: BTreeMap<&str, usize> = BTreeMap::new();
    let mut broken: Vec<String> = Vec::new();
    for p in hale_corpus::parseable(|s| hale_syntax::parse_source(s).is_ok()) {
        let Ok(program) = hale_syntax::parse_source(&p.source) else { continue };
        let Ok(s) = Snapshot::from_program(program, Vec::new(), Config::check(false, false)) else { continue };
        let Ok(plan) = s.demand_lifecycle() else { continue };
        programs += 1;
        instances += plan.instances.len();
        rows += plan.obligations.len();
        for o in &plan.obligations {
            *by_kind.entry(o.kind.name()).or_insert(0) += 1;
        }
        for l in laws(plan) {
            broken.push(format!("{}: {l}", p.origin));
        }
    }
    eprintln!("{programs} programs, {instances} instance templates, {rows} rows: {by_kind:?}");
    assert!(programs > 100, "the corpus shrank to {programs} programs");
    assert!(broken.is_empty(), "{} law(s) broken:\n{}", broken.len(), broken.join("\n"));
}

/// The emitters' reader (L4) over every corpus program: each template's
/// birth spine follows its entry edges, and a declaration's templates
/// agree on the order of the birth kinds they share, so an emitter
/// lowering one literal of the declaration reads one order.
#[test]
fn every_declaration_reads_one_birth_order_over_the_corpus() {
    let mut decls = 0;
    let mut broken: Vec<String> = Vec::new();
    for p in hale_corpus::parseable(|s| hale_syntax::parse_source(s).is_ok()) {
        let Ok(program) = hale_syntax::parse_source(&p.source) else { continue };
        let Ok(s) = Snapshot::from_program(program, Vec::new(), Config::check(false, false)) else { continue };
        let Ok(plan) = s.demand_lifecycle() else { continue };
        let names: BTreeSet<&str> = plan.instances.iter().map(|i| i.site.decl.lowered.as_str()).collect();
        for name in names {
            decls += 1;
            let mut kinds: Vec<K> = Vec::new();
            for site in plan.templates(name) {
                let steps = plan.birth_spine(site);
                for (i, step) in steps.iter().enumerate() {
                    let o = plan.get(step.obligation).expect("a row");
                    // No row of the spine waits for one placed after it.
                    for later in &steps[i + 1..] {
                        if o.edges.entry.iter().any(|pr| pr.event.obligation == later.obligation) {
                            broken.push(format!("{}: {name}: {} placed before {}", p.origin, o.kind.name(), later.kind.name()));
                        }
                    }
                    if !kinds.contains(&step.kind) {
                        kinds.push(step.kind);
                    }
                }
                // The producer's order, which the reader falls back on.
                let at: Vec<usize> = steps
                    .iter()
                    .map(|s| BIRTH_KINDS.iter().position(|k| *k == s.kind).expect("a birth kind"))
                    .collect();
                if at.windows(2).any(|w| w[0] >= w[1]) {
                    broken.push(format!("{}: {name}: the birth spine departs from BIRTH_KINDS", p.origin));
                }
            }
            if let Err(e) = plan.birth_order(name, &kinds) {
                broken.push(format!("{}: {e}", p.origin));
            }
        }
    }
    assert!(decls > 100, "the corpus shrank to {decls} declarations");
    assert!(broken.is_empty(), "{} broken:\n{}", broken.len(), broken.join("\n"));
}

/// The reader's order on the shapes the birth spine has: registration,
/// then the birth, then readiness (line 6); a pinned locus's thread
/// births, runs, drains and dissolves it; an accepted child is accepted
/// before its birth (line 5); a declaration the plan has no template of
/// reads the order every template states.
#[test]
fn the_reader_orders_each_spine_by_the_plans_edges() {
    let kinds = |p: &LifecyclePlan, decl: &str, spine: Spine| -> Vec<&'static str> {
        let site = p.templates(decl).next().unwrap_or_else(|| panic!("{decl} has a template"));
        p.spine(site, spine).iter().map(|s| s.kind.name()).collect()
    };
    let s = snapshot(include_str!("../../hale-codegen/tests/fixtures/lifecycle/l06_readiness_main.hl"));
    let p = plan(&s);
    assert_eq!(kinds(p, "Sub", Spine::Instantiation), ["Subscribe", "Birth", "Readiness"]);
    let s = snapshot(include_str!("../../hale-codegen/tests/fixtures/lifecycle/l12_pinned_fields_drain.hl"));
    let p = plan(&s);
    assert_eq!(kinds(p, "Outer", Spine::PinnedMain), ["Birth", "Run", "Drain", "Dissolve"]);
    let s = snapshot(include_str!("../../hale-codegen/tests/fixtures/lifecycle/l05_accept_position.hl"));
    let p = plan(&s);
    assert_eq!(kinds(p, "Kid", Spine::Instantiation), ["Accept", "Birth"]);
    assert_eq!(p.birth_order("Kid", &[K::Birth, K::Accept]).expect("ordered"), [K::Accept, K::Birth]);
    assert_eq!(p.birth_order("NoSuchLocus", &[K::Run, K::Birth]).expect("ordered by every template"), [K::Birth, K::Run]);
    // No template of this plan subscribes: the producer's order.
    assert_eq!(p.birth_order("NoSuchLocus", &[K::Readiness, K::Birth]).expect("ordered"), [K::Birth, K::Readiness]);
    // A run queued on the worker that tears its owner down: the path no
    // failure takes owes no cancellation; the one a shutdown takes owes it
    // inside the child's reclaim, after the reclaim's entry.
    let s = snapshot(include_str!("../../hale-codegen/tests/fixtures/lifecycle/l19_queued_run_canceled.hl"));
    let p = plan(&s);
    let site = p
        .templates("Kid")
        .find(|site| p.shutdown_spine(site, Spine::Cascade).iter().any(|s| s.kind == K::Cancellation))
        .expect("a Kid template whose reclaim cancels its queued run");
    let normal: Vec<&str> = p.spine(site, Spine::Cascade).iter().map(|s| s.kind.name()).collect();
    let shutdown: Vec<&str> = p.shutdown_spine(site, Spine::Cascade).iter().map(|s| s.kind.name()).collect();
    assert_eq!(normal, ["Drain", "Dissolve", "Reclaim"]);
    assert_eq!(shutdown, ["Drain", "Dissolve", "Reclaim", "Cancellation"]);
}

/// Line 1: a child failing while its owner's params are open has its
/// delivery held to the owner's settle, and its owner's birth waits for
/// it; the plan states the held alternative beside the delivery in
/// place, each on its own path.
#[test]
fn a_held_failure_is_delivered_at_settle_before_the_owners_birth() {
    let s = snapshot(include_str!("../../hale-codegen/tests/fixtures/lifecycle/l01_held_failure_settle.hl"));
    let p = plan(&s);
    let settle = one(p, "App", K::ParamsSettle);
    assert_eq!(settle.line, Some("1"));
    let deliveries = rows(p, "Boom", K::FailureDelivery);
    let guards: BTreeSet<PathGuard> = deliveries.iter().map(|o| o.guard).collect();
    assert_eq!(guards, BTreeSet::from([PathGuard::FailedInRun, PathGuard::FailedAtSettle]));
    let held = deliveries.iter().find(|o| o.guard == PathGuard::FailedAtSettle).expect("held");
    assert_eq!(held.source, Some(FailureSource::Run));
    let settle_id = p.iter().find(|(_, o)| std::ptr::eq(*o, settle)).map(|(id, _)| id).expect("id");
    assert!(held.edges.completion.iter().any(|pr| pr.event.obligation == settle_id && pr.event.point == Point::Completed));
    assert_eq!(rows(p, "Boom", K::ConstructionDelivery).len(), 1);
    let held_id = p.iter().find(|(_, o)| std::ptr::eq(*o, *held)).map(|(id, _)| id).expect("id");
    let app_birth = one(p, "App", K::Birth);
    assert!(app_birth.edges.entry.iter().any(|pr| pr.event.obligation == held_id), "the owner's birth waits for the held delivery");
    // Keyed by the placement table's identities, each site with its universe.
    let site = &one(p, "Boom", K::Birth).site.as_ref().expect("a site");
    assert_eq!(site.decl.site.universe, SiteUniverse::User);
    assert!(matches!(&site.template, Template::Static(k) if k.path.len() == 1 && k.path[0].field == "c"));
}

/// Lines 12 and 17: a pinned locus runs on its own thread, owes its
/// join, and its own fields are owed a drain the code never runs (C9).
#[test]
fn a_pinned_anchor_owes_its_thread_and_its_fields_their_drain() {
    let s = snapshot(include_str!("../../hale-codegen/tests/fixtures/lifecycle/l12_pinned_fields_drain.hl"));
    let p = plan(&s);
    assert_eq!(one(p, "Outer", K::Birth).holder.spine, Spine::PinnedMain);
    assert_eq!(one(p, "Outer", K::Run).holder.spine, Spine::PinnedMain, "a pinned thread runs run(), written or not");
    assert_eq!(rows(p, "Outer", K::PinnedJoin).len(), 1);
    assert_eq!(one(p, "Inner", K::Birth).holder.spine, Spine::Instantiation, "a field of a pinned locus is born off its thread");
    let drain = one(p, "Inner", K::Drain);
    assert_eq!((drain.line, drain.status), (Some("12"), Status::KnownOpen { inventory_row: "C9" }));
    let outer_drain = one(p, "Outer", K::Drain);
    assert!(outer_drain.edges.entry.iter().any(|pr| pr.rule.status == Status::KnownOpen { inventory_row: "C9" }));
}

/// Line 12 for a field typed by an interface: it drains before its
/// owner, a rule its recorded reclaim does not keep today (C32).
#[test]
fn a_contract_typed_field_owes_its_drain_before_its_owners() {
    let s = snapshot(
        "interface Probe { fn v() -> Int; }\nlocus Kid { fn v() -> Int { return 1; } }\nmain locus App { params { k: Probe = Kid { }; } }\nfn main() { App { }; }\n",
    );
    let p = plan(&s);
    let kid_drain = p.obligations.iter().position(|o| o.kind == K::Drain && o.site.as_ref().is_some_and(|s| s.decl.lowered == "Kid"));
    let order = one(p, "App", K::Drain)
        .edges
        .entry
        .iter()
        .find(|pr| Some(pr.event.obligation.0 as usize) == kid_drain)
        .expect("the owner's drain waits for its field's");
    assert_eq!((order.rule.line, order.rule.status), (Some("12"), Status::KnownOpen { inventory_row: "C32" }));
}

/// Line 3: a field nested under a pool-placed field owes its run() to
/// that pool, and runs it inline today (C12).
#[test]
fn a_field_under_a_pool_placed_field_owes_its_run_to_the_pool() {
    let s = snapshot(
        "locus Kid { run() { } }\nlocus Mid { params { k: Kid = Kid { }; } }\nmain locus App {\n    params { m: Mid = Mid { }; }\n    placement { m: cooperative(pool = side); }\n}\nfn main() { App { }; }\n",
    );
    let p = plan(&s);
    let on = one(p, "Kid", K::Run).runs_on.expect("the table gives it a pool");
    assert!(matches!(&p.domains[on.domain.0 as usize].kind, DomainKind::Pool { name, .. } if name == "side"));
    assert_eq!((on.rule.line, on.rule.status), (Some("3"), Status::KnownOpen { inventory_row: "C12" }));
}

/// Line 13: a locus that declares no run() owes none when it resumes;
/// the row says so, known open (C48), and owes no event.
#[test]
fn a_resumed_locus_with_no_run_owes_none() {
    let s = snapshot(include_str!("../../hale-codegen/tests/fixtures/lifecycle/l01_neg_same_pool_held.hl"));
    let p = plan(&s);
    // One per failure path that restarts it.
    let runs = rows(p, "Late", K::Run);
    assert!(!runs.is_empty());
    for run in runs {
        assert_eq!((run.guard, run.line, run.status), (PathGuard::Restart, Some("13"), Status::KnownOpen { inventory_row: "C48" }));
        assert_eq!(run.terminals, vec![Terminal::NotStarted(NotStarted::NoRun)]);
    }
}

/// Line 14 and GH #736: an accepted flow is torn down by the reclaim its
/// run's end runs, before its owner's arena goes.
#[test]
fn a_flow_child_is_reclaimed_at_its_runs_end_before_its_owner() {
    let s = snapshot(include_str!("../../hale-codegen/tests/fixtures/lifecycle/l14_reclaim_exactly_once.hl"));
    let p = plan(&s);
    assert_eq!(p.instances.iter().filter(|i| i.site.decl.lowered == "Kid").count(), 3, "three literals, three templates");
    for kind in [K::Accept, K::Birth, K::Run, K::Drain, K::Dissolve, K::Reclaim] {
        assert_eq!(rows(p, "Kid", kind).len(), 3, "{}", kind.name());
    }
    assert!(rows(p, "Kid", K::Drain).iter().all(|o| o.holder.spine == Spine::Reclaim));
    let app_reclaim = one(p, "App", K::Reclaim);
    assert_eq!(app_reclaim.edges.entry.iter().filter(|pr| pr.rule.line == Some("14")).count(), 3);
}

/// RD: a handler that restarts has its restart performed on one path and
/// refused, not started, under teardown on another (C42 today).
#[test]
fn a_restart_is_refused_under_teardown() {
    let s = snapshot(include_str!("../../hale-codegen/tests/fixtures/lifecycle/rd_restart_during_teardown.hl"));
    let p = plan(&s);
    let restarts = rows(p, "Kid", K::Restart);
    let refused = restarts.iter().find(|o| o.guard == PathGuard::DrainInFlight).expect("a refused restart");
    assert_eq!(refused.status, Status::KnownOpen { inventory_row: "C42" });
    assert!(refused.terminals.iter().all(|t| matches!(t, hale_types::lifecycle::Terminal::NotStarted(_))));
    assert!(restarts.iter().any(|o| o.guard == PathGuard::Restart && o.status == Status::Shipped));
    // One decision per path its failure can take: in place, or held at
    // its owner's settle.
    let decisions: BTreeSet<PathGuard> = rows(p, "Kid", K::RecoveryDecision).iter().map(|o| o.guard).collect();
    assert_eq!(decisions, BTreeSet::from([PathGuard::FailedInRun, PathGuard::FailedAtSettle]));
}

/// Lines 7 and 18: the main locus's eager teardown owes the process its
/// wait-abort before the pool join (R34 today) and a pre-drain it does
/// not emit (C13).
#[test]
fn the_eager_spine_owes_the_wait_abort_before_the_join() {
    let s = snapshot(include_str!("../../hale-codegen/tests/fixtures/lifecycle/l07_pool_or_wait_teardown.hl"));
    let p = plan(&s);
    let process = |kind: K, spine: Spine| -> &Obligation {
        p.obligations.iter().find(|o| o.site.is_none() && o.kind == kind && o.holder.spine == spine).expect("a process row")
    };
    let join = process(K::PoolJoin, Spine::EagerTeardown);
    assert!(join.edges.entry.iter().any(|pr| pr.rule.status == Status::KnownOpen { inventory_row: "R34" }));
    assert_eq!(process(K::PreDrain, Spine::EagerTeardown).status, Status::KnownOpen { inventory_row: "C13" });
    // The pool's run ends before the join completes (line 19).
    let pusher_run = one(p, "Pusher", K::Run);
    let on = pusher_run.runs_on.expect("a placed run names its domain");
    assert!(matches!(&p.domains[on.domain.0 as usize].kind, DomainKind::Pool { name, .. } if name == "side"));
    assert!(join.edges.completion.iter().any(|pr| pr.rule.line == Some("19") && pr.event.point == Point::Ended));
}

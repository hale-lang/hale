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
    DomainRole, FailureSource, LifecyclePlan, NotStarted, Obligation, ObligationId, ObligationKind as K, PathGuard, Point, Rule,
    Spine, Status, Template, Terminal, DECISION_LINES,
};
use hale_types::lifecycle::spine::{BIRTH_KINDS, CASCADE_STEPS, RECLAIM_STEPS};
use hale_types::placement::{Bound, DomainKind, SiteUniverse};

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
/// lowering one literal of the declaration reads one order; and every
/// declaration reads one order of its reclaim's steps.
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
            // The reclaim's steps: one order per declaration, the
            // producer's (the emitter refuses any other it cannot emit).
            match plan.reclaim_order(name) {
                Ok(order) if order != RECLAIM_STEPS => {
                    broken.push(format!("{}: {name}: the reclaim order {order:?} departs from RECLAIM_STEPS", p.origin))
                }
                Ok(_) => {}
                Err(e) => broken.push(format!("{}: {e}", p.origin)),
            }
            // And the cascade's, the one the dissolve cascade emits.
            match plan.cascade_order(name) {
                Ok(order) if order != CASCADE_STEPS => {
                    broken.push(format!("{}: {name}: the cascade order {order:?} departs from CASCADE_STEPS", p.origin))
                }
                Ok(_) => {}
                Err(e) => broken.push(format!("{}: {e}", p.origin)),
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

/// Lines 14 and 19, inside one reclaim: the rows order the latch before
/// the releases (the arena retained until the reclaim completes), the
/// cancellation after the latch (its entry edge) and before the releases
/// (the run holds the instance until it ends, and the reclaim completes
/// only after both), and an owned child's reclaim before its owner's
/// storage release (the line-14 completion edge). The order the emitters read is those pairs,
/// whatever template states them.
#[test]
fn the_reclaim_order_is_read_from_the_rows() {
    use hale_types::lifecycle::spine::ReclaimStep as R;
    let s = snapshot(include_str!("../../hale-codegen/tests/fixtures/lifecycle/l19_queued_run_canceled.hl"));
    let p = plan(&s);
    let canceled = p
        .templates("Kid")
        .find(|site| p.reclaim_pairs(site).contains(&(R::Latch, R::CancelQueuedRuns)))
        .expect("a Kid template whose reclaim cancels its queued run");
    let pairs = p.reclaim_pairs(canceled);
    for pair in [
        (R::Latch, R::CancelQueuedRuns),
        (R::CancelQueuedRuns, R::WaitForRuns),
        (R::WaitForRuns, R::ReleaseArena),
        (R::WaitForRuns, R::ReleaseStruct),
        (R::CancelQueuedRuns, R::ReleaseArena),
        (R::CancelQueuedRuns, R::ReleaseStruct),
        (R::Latch, R::ReleaseArena),
        (R::Latch, R::ReleaseStruct),
    ] {
        assert!(pairs.contains(&pair), "{pair:?} not stated by Kid's rows: {pairs:?}");
    }
    let s = snapshot(include_str!("../../hale-codegen/tests/fixtures/lifecycle/l12_pinned_fields_drain.hl"));
    let p = plan(&s);
    let outer = p.templates("Outer").next().expect("Outer");
    assert!(p.reclaim_pairs(outer).contains(&(R::Children, R::ReleaseArena)), "a field's reclaim before its owner's release");
    // Inner has no queued run: its cancellation's place comes from the
    // other templates, or the producer's order.
    let inner = p.templates("Inner").next().expect("Inner");
    assert!(!p.reclaim_pairs(inner).contains(&(R::Latch, R::CancelQueuedRuns)));
    assert_eq!(
        p.reclaim_order("Inner").expect("ordered"),
        [R::Children, R::Latch, R::CancelQueuedRuns, R::WaitForRuns, R::ReleaseArena, R::ReleaseStruct]
    );
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
/// join, and its own fields drain on that thread before it does (C9,
/// shipped by L4's cascade).
#[test]
fn a_pinned_anchor_owes_its_thread_and_its_fields_their_drain() {
    let s = snapshot(include_str!("../../hale-codegen/tests/fixtures/lifecycle/l12_pinned_fields_drain.hl"));
    let p = plan(&s);
    assert_eq!(one(p, "Outer", K::Birth).holder.spine, Spine::PinnedMain);
    assert_eq!(one(p, "Outer", K::Run).holder.spine, Spine::PinnedMain, "a pinned thread runs run(), written or not");
    assert_eq!(rows(p, "Outer", K::PinnedJoin).len(), 1);
    assert_eq!(one(p, "Inner", K::Birth).holder.spine, Spine::Instantiation, "the field's instantiation runs inside the pinned init");
    assert_eq!(claimed(p, one(p, "Inner", K::Birth)), labels(&["pinned"]));
    let drain = one(p, "Inner", K::Drain);
    assert_eq!((drain.line, drain.status), (Some("12"), Status::Shipped));
    let on = drain.runs_on.as_ref().expect("its owner's thread").one().expect("one pinned domain");
    assert!(matches!(p.domains[on.0 as usize].kind, DomainKind::Pinned { .. }), "drained on the pinned thread");
    let outer_drain = one(p, "Outer", K::Drain);
    assert!(outer_drain.edges.entry.iter().any(|pr| pr.rule == hale_types::lifecycle::Rule::line("12", Status::Shipped)));
}

/// Line 12 over the instance tree: an owner's fields drain in their
/// declaration order, and each is torn down before the next is dissolved.
#[test]
fn an_owners_fields_are_torn_down_in_declaration_order() {
    let s = snapshot(
        "locus Kid { run() { } }\nmain locus App { params { z: Kid = Kid { }; a: Kid = Kid { }; m: Kid = Kid { }; } }\nfn main() { App { }; }\n",
    );
    let p = plan(&s);
    assert_eq!(p.cascade_fields("App"), ["z", "a", "m"]);
    assert_eq!(p.cascade_order("App").expect("ordered"), CASCADE_STEPS);
}

/// Line 12 for a field typed by an interface: it drains before its
/// owner, like every owned field (C32, shipped by L4's cascade).
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
    assert_eq!((order.rule.line, order.rule.status), (Some("12"), Status::Shipped));
}

/// Line 3: a field nested under a pool-placed field owes its run() to
/// that pool, and initializes there inside the anchor's init (C50).
#[test]
fn a_field_under_a_pool_placed_field_owes_its_run_to_the_pool() {
    let s = snapshot(
        "locus Kid { run() { let n = 1; } }\nlocus Mid { params { k: Kid = Kid { }; } }\nmain locus App {\n    params { m: Mid = Mid { }; }\n    placement { m: cooperative(pool = side); }\n}\nfn main() { App { }; }\n",
    );
    let p = plan(&s);
    let on = one(p, "Kid", K::Run).runs_on.clone().expect("the table gives it a pool");
    assert!(matches!(&p.domains[on.one().expect("one domain").0 as usize].kind, DomainKind::Pool { name, .. } if name == "side"));
    assert_eq!(on.rule, Rule::SHIPPED);
    assert_eq!(claimed(p, one(p, "Kid", K::Birth)), labels(&["pool:side"]));
    assert!(rows(p, "Kid", K::RunAdmission).is_empty(), "the nested run does not enter the pool queue");
    assert!(rows(p, "Kid", K::Cancellation).is_empty(), "the nested run is inline, not queued behind the init");
}

#[test]
fn an_anchors_params_and_held_delivery_use_its_initialization_domain() {
    let source = include_str!("../../hale-codegen/tests/fixtures/lifecycle/l01_pool_owner_settle.hl");
    for (placement, domain, birth_domain) in [
        ("cooperative(pool = side)", "pool:side", "main"),
        ("pinned", "pinned", "pinned"),
    ] {
        let s = snapshot(&source.replace("cooperative(pool = side)", placement));
        let p = plan(&s);
        assert_eq!(claimed(p, one(p, "Owner", K::ParamsSettle)), labels(&[domain]));
        assert_eq!(one(p, "Owner", K::ParamsSettle).holder.domain, DomainRole::Own);
        assert_eq!(claimed(p, one(p, "Owner", K::Birth)), labels(&[birth_domain]));
        for kind in [K::Birth, K::Run] {
            assert_eq!(claimed(p, one(p, "Boom", kind)), labels(&[domain]));
        }
        let held = rows(p, "Boom", K::FailureDelivery).into_iter()
            .find(|o| o.guard == PathGuard::FailedAtSettle).expect("the held alternative");
        assert_eq!(claimed(p, held), labels(&[domain]));
    }
}

/// The labels of the domains a row claims.
fn claimed(p: &LifecyclePlan, o: &Obligation) -> BTreeSet<String> {
    let on = o.runs_on.as_ref().unwrap_or_else(|| panic!("{} claims no domain", o.kind.name()));
    on.domains
        .iter()
        .map(|d| match &p.domains[d.0 as usize].kind {
            DomainKind::Main => "main".to_string(),
            DomainKind::Pool { name, .. } => format!("pool:{name}"),
            DomainKind::Pinned { .. } => "pinned".to_string(),
        })
        .collect()
}

fn labels(ls: &[&str]) -> BTreeSet<String> {
    ls.iter().map(|l| l.to_string()).collect()
}

fn id_of(p: &LifecyclePlan, o: &Obligation) -> ObligationId {
    p.iter().find(|(_, x)| std::ptr::eq(*x, o)).map(|(id, _)| id).expect("in the plan")
}

/// `Parent { }` built by App's run() on main and by Worker's on pool
/// `side`: Parent's field default `Leaf { }` is one template with both.
fn two_parents(leaf: &str, extra: &str, worker_placed: bool) -> String {
    let placement = if worker_placed { "    placement { worker: cooperative(pool = side); }\n" } else { "" };
    format!(
        "{leaf}\n{extra}locus Parent {{ params {{ leaf: Leaf = Leaf {{ }}; }} }}\nlocus Worker {{ run() {{ Parent {{ }}; }} }}\nmain locus App {{\n    params {{ worker: Worker = Worker {{ }}; }}\n{placement}    run() {{ Parent {{ }}; }}\n}}\nfn main() {{ App {{ }}; }}\n"
    )
}

/// Line 3: a field literal reached through two constructions of its
/// owner's declaration, one on main and one on pool `side`, is one
/// template that keeps both: its birth and run claim the set (each
/// occurrence runs on its own parent's domain, never on the first
/// parent's alone), its bound is the two summed, and it is a child of
/// both parents.
#[test]
fn a_field_reached_under_parents_on_two_domains_claims_both() {
    let s = snapshot(&two_parents("locus Leaf { run() { let n = 1; } }", "", true));
    let p = plan(&s);
    assert!(laws(p).is_empty(), "{:?}", laws(p));
    let leaf: Vec<_> = p.instances.iter().filter(|i| i.site.decl.lowered == "Leaf").collect();
    assert_eq!(leaf.len(), 1, "one template");
    // Two literals in run() bodies, each unbounded.
    assert!(matches!(leaf[0].bound, Bound::Unbounded(_)));
    for kind in [K::Birth, K::Run] {
        let o = one(p, "Leaf", kind);
        assert_eq!(claimed(p, o), labels(&["main", "pool:side"]), "{}", kind.name());
        assert_eq!(o.runs_on.as_ref().map(|r| r.rule), Some(Rule::SHIPPED), "{}", kind.name());
        assert_eq!(o.runs_on.as_ref().and_then(|r| r.one()), None);
    }
    // A child of each parent: born before its birth, reclaimed before its
    // arena.
    let (leaf_birth, leaf_reclaim) = (id_of(p, one(p, "Leaf", K::Birth)), id_of(p, one(p, "Leaf", K::Reclaim)));
    let parent_births = rows(p, "Parent", K::Birth);
    assert_eq!(parent_births.len(), 2, "two Parent literals, two templates");
    for b in parent_births {
        assert!(b.edges.entry.iter().any(|pr| pr.event.obligation == leaf_birth));
    }
    for r in rows(p, "Parent", K::Reclaim) {
        assert!(r.edges.completion.iter().any(|pr| pr.event.obligation == leaf_reclaim));
    }
}

/// The same two levels down: Twig, Leaf's field, takes Leaf's
/// contributions with its own placement, so it claims the same set; and
/// its bound is the sum of its grandparents' (it kept the first one's
/// alone before: `Once` for two Parents built once each).
#[test]
fn a_field_two_levels_under_parents_on_two_domains_claims_both() {
    let s = snapshot(&two_parents("locus Leaf { params { twig: Twig = Twig { }; } }", "locus Twig { run() { let n = 1; } }\n", true));
    let p = plan(&s);
    assert!(laws(p).is_empty(), "{:?}", laws(p));
    assert_eq!(p.instances.iter().filter(|i| i.site.decl.lowered == "Twig").count(), 1);
    for kind in [K::Birth, K::Run] {
        assert_eq!(claimed(p, one(p, "Twig", kind)), labels(&["main", "pool:side"]), "{}", kind.name());
    }
    assert_eq!(claimed(p, one(p, "Leaf", K::Birth)), labels(&["main", "pool:side"]));
    let s = snapshot(
        "locus Twig { }\nlocus Leaf { params { twig: Twig = Twig { }; } }\nlocus Parent { params { leaf: Leaf = Leaf { }; } }\nfn one() { Parent { }; }\nfn two() { Parent { }; }\nfn main() { one(); two(); }\n",
    );
    let p = plan(&s);
    for decl in ["Leaf", "Twig"] {
        let bound: Vec<&Bound> = p.instances.iter().filter(|i| i.site.decl.lowered == decl).map(|i| &i.bound).collect();
        assert_eq!(bound, [&Bound::AtMost(2)], "{decl}");
    }
}

/// The control: both parents on main. Every contribution agrees, and
/// the claim is the one domain.
#[test]
fn a_field_reached_under_parents_on_one_domain_claims_it() {
    let s = snapshot(&two_parents("locus Leaf { run() { let n = 1; } }", "", false));
    let p = plan(&s);
    for kind in [K::Birth, K::Run] {
        let o = one(p, "Leaf", kind);
        assert_eq!(claimed(p, o), labels(&["main"]), "{}", kind.name());
        assert_eq!(o.runs_on.as_ref().and_then(|r| r.one()), Some(hale_types::placement::PlacementTable::MAIN));
    }
}

/// The failure route of such a field reaches each parent's handler on
/// that parent's domain: the delivery in place and the one held to the
/// settle claim the set, and the held one waits for both parents'
/// settles.
#[test]
fn a_field_under_parents_on_two_domains_fails_to_each() {
    let s = snapshot(&two_parents(
        "locus Leaf {\n    params { name: String = \"\"; }\n    closure fuse { captures: name; epoch inline; }\n    run() { violate fuse; }\n}",
        "",
        true,
    )
    .replace(
        "locus Parent { params { leaf: Leaf = Leaf { }; } }",
        "locus Parent {\n    params { leaf: Leaf = Leaf { }; }\n    on_failure(c: Leaf, err: ClosureViolation) { }\n}",
    ));
    let p = plan(&s);
    assert!(laws(p).is_empty(), "{:?}", laws(p));
    let deliveries = rows(p, "Leaf", K::FailureDelivery);
    let in_place = deliveries.iter().find(|o| o.guard == PathGuard::FailedInRun).expect("delivered in place");
    assert_eq!(claimed(p, in_place), labels(&["main", "pool:side"]));
    assert_eq!(in_place.runs_on.as_ref().map(|r| (r.rule.line, r.rule.status)), Some((Some("L0-1"), Status::Shipped)));
    let held = deliveries.iter().find(|o| o.guard == PathGuard::FailedAtSettle).expect("held to the settle");
    assert_eq!(claimed(p, held), labels(&["main", "pool:side"]));
    let settles: BTreeSet<ObligationId> = rows(p, "Parent", K::ParamsSettle).into_iter().map(|o| id_of(p, o)).collect();
    assert_eq!(settles.len(), 2);
    let waits: BTreeSet<ObligationId> = held.edges.completion.iter().map(|pr| pr.event.obligation).collect();
    assert!(settles.is_subset(&waits), "the held delivery waits for each parent's settle");
}

/// `Mid { }` built and its method `make()` called by App's run() on main
/// and by Worker's on pool `side` (or on main): `make()`'s `Leaf { }` is
/// one template whose owners are both Mid templates.
fn two_enclosing(leaf: &str, mid_extra: &str, worker_placed: bool) -> String {
    let placement = if worker_placed { "    placement { worker: cooperative(pool = side); }\n" } else { "" };
    format!(
        "{leaf}\nlocus Mid {{\n    fn make() {{ Leaf {{ }}; }}\n{mid_extra}}}\nlocus Worker {{ run() {{ let m = Mid {{ }}; m.make(); }} }}\nmain locus App {{\n    params {{ worker: Worker = Worker {{ }}; }}\n{placement}    run() {{ let m = Mid {{ }}; m.make(); }}\n}}\nfn main() {{ App {{ }}; }}\n"
    )
}

/// Line 3: a body literal whose enclosing locus has a template on main
/// and one on pool `side` keeps both as owners. Each occurrence is built
/// on the domain its own Mid runs the method on, so its birth and run
/// claim the set; the template that kept the first Mid alone claimed
/// neither. It is a child of both, and its bound is the table's.
#[test]
fn a_body_literal_under_enclosing_templates_on_two_domains_claims_both() {
    let s = snapshot(&two_enclosing("locus Leaf { run() { let n = 1; } }", "", true));
    let p = plan(&s);
    assert!(laws(p).is_empty(), "{:?}", laws(p));
    let leaf: Vec<_> = p.instances.iter().filter(|i| i.site.decl.lowered == "Leaf").collect();
    assert_eq!(leaf.len(), 1, "one template");
    assert!(matches!(leaf[0].bound, Bound::Unbounded(_)), "a locus body runs any number of times");
    for kind in [K::Birth, K::Run] {
        let o = one(p, "Leaf", kind);
        assert_eq!(claimed(p, o), labels(&["main", "pool:side"]), "{}", kind.name());
        assert_eq!(o.runs_on.as_ref().map(|r| r.rule), Some(Rule::SHIPPED), "{}", kind.name());
    }
    // A body literal's run is not posted (C12): on side as on main it runs
    // inline at its statement, before its drain, and nothing cancels it.
    assert!(rows(p, "Leaf", K::Cancellation).is_empty());
    let leaf_run = id_of(p, one(p, "Leaf", K::Run));
    assert!(one(p, "Leaf", K::Drain).edges.entry.iter().any(|pr| pr.event.obligation == leaf_run && pr.event.point == Point::Ended));
    // A child of each Mid: reclaimed before its arena.
    let leaf_reclaim = id_of(p, one(p, "Leaf", K::Reclaim));
    let mid_reclaims = rows(p, "Mid", K::Reclaim);
    assert_eq!(mid_reclaims.len(), 2, "two Mid literals, two templates");
    for r in mid_reclaims {
        assert!(r.edges.completion.iter().any(|pr| pr.event.obligation == leaf_reclaim));
    }
}

/// The failure route of such a literal reaches each Mid's handler on that
/// Mid's domain, in place: the delivery claims the set, shipped. The
/// template that kept the first Mid alone (Worker's, the first written)
/// claimed that Mid's domain, `pool:side`, known open (C36), false of
/// the occurrence raising and handled on main.
#[test]
fn a_body_literal_under_enclosing_templates_on_two_domains_fails_to_each() {
    let s = snapshot(&two_enclosing(
        "locus Leaf {\n    params { name: String = \"\"; }\n    closure fuse { captures: name; epoch inline; }\n    run() { violate fuse; }\n}",
        "    on_failure(c: Leaf, err: ClosureViolation) { }\n",
        true,
    ));
    let p = plan(&s);
    assert!(laws(p).is_empty(), "{:?}", laws(p));
    let deliveries = rows(p, "Leaf", K::FailureDelivery);
    assert_eq!(deliveries.len(), 1, "a body literal's failure is never held");
    assert_eq!(claimed(p, deliveries[0]), labels(&["main", "pool:side"]));
    assert_eq!(deliveries[0].runs_on.as_ref().map(|r| (r.rule.line, r.rule.status)), Some((Some("L0-1"), Status::Shipped)));
}

/// An accepted literal the same: a Host on main and one on `side` each
/// accept the Kid their method builds, so the Kid template's accept,
/// birth and run claim the set.
#[test]
fn an_accepted_literal_under_acceptors_on_two_domains_claims_both() {
    let s = snapshot(
        "locus Kid { run() { } }\nlocus Host {\n    accept(c: Kid) { }\n    release (c: Kid) { }\n    fn make() { Kid { }; }\n}\nlocus Worker { run() { let h = Host { }; h.make(); } }\nmain locus App {\n    params { worker: Worker = Worker { }; }\n    placement { worker: cooperative(pool = side); }\n    run() { let h = Host { }; h.make(); }\n}\nfn main() { App { }; }\n",
    );
    let p = plan(&s);
    assert!(laws(p).is_empty(), "{:?}", laws(p));
    for kind in [K::Accept, K::Birth, K::Run] {
        assert_eq!(claimed(p, one(p, "Kid", kind)), labels(&["main", "pool:side"]), "{}", kind.name());
    }
}

/// The control: both Mids on main. Every contribution agrees, and the
/// claim is the one domain.
#[test]
fn a_body_literal_under_enclosing_templates_on_one_domain_claims_it() {
    let s = snapshot(&two_enclosing("locus Leaf { run() { let n = 1; } }", "", false));
    let p = plan(&s);
    assert!(laws(p).is_empty(), "{:?}", laws(p));
    for kind in [K::Birth, K::Run] {
        let o = one(p, "Leaf", kind);
        assert_eq!(o.runs_on.as_ref().and_then(|r| r.one()), Some(hale_types::placement::PlacementTable::MAIN), "{}", kind.name());
    }
}

/// A known limit: a template exists only where a handler runs when every
/// contribution is built in one. A field whose parents are built one in
/// an `on_failure` body and one outside it is stated as always built.
#[test]
fn a_template_is_built_in_a_handler_only_where_every_contribution_is() {
    let src = "locus Boom {\n    params { name: String = \"\"; }\n    closure fuse { captures: name; epoch inline; }\n    run() { violate fuse; }\n}\nlocus Leaf { run() { } }\nlocus Parent { params { leaf: Leaf = Leaf { }; } }\nlocus Worker { run() { Parent { }; } }\nmain locus App {\n    params { boom: Boom = Boom { }; worker: Worker = Worker { }; }\n    on_failure(c: Boom, err: ClosureViolation) { Parent { }; }\n}\nfn main() { App { }; }\n";
    let in_handler = |src: &str| -> (Vec<bool>, Vec<bool>) {
        let s = snapshot(src);
        let p = plan(&s);
        assert!(laws(p).is_empty(), "{:?}", laws(p));
        let of = |decl: &str| p.instances.iter().filter(|i| i.site.decl.lowered == decl).map(|i| i.in_handler).collect::<Vec<_>>();
        (of("Parent"), of("Leaf"))
    };
    let (parents, leaf) = in_handler(src);
    assert_eq!(parents, [false, true], "Worker's Parent, then the handler's");
    assert_eq!(leaf, [false], "one Leaf template, built outside a handler under one of its parents");
    let (parents, leaf) = in_handler(&src.replace("locus Worker { run() { Parent { }; } }", "locus Worker { run() { } }"));
    assert_eq!((parents, leaf), (vec![true], vec![true]), "the control: every parent built in the handler");
}

/// A known limit: the pool join waits for a run's end only where every
/// occurrence of its template is on a pool. A literal built on main and
/// on `side` has its run left out of the join; built on `side` alone, it
/// is joined. Neither owes a cancellation: a body literal under a single
/// owner on a pool was owed one, as if its run were posted behind that
/// owner's teardown, where it runs inline at its statement (C12).
#[test]
fn the_pool_join_holds_a_run_only_where_every_occurrence_is_on_a_pool() {
    let joined = |src: &str| -> (bool, bool) {
        let s = snapshot(src);
        let p = plan(&s);
        assert!(laws(p).is_empty(), "{:?}", laws(p));
        let run = id_of(p, one(p, "Leaf", K::Run));
        let waits: BTreeSet<ObligationId> = p
            .obligations
            .iter()
            .filter(|o| o.kind == K::PoolJoin)
            .flat_map(|o| o.edges.completion.iter().map(|pr| pr.event.obligation))
            .collect();
        (waits.contains(&run), rows(p, "Leaf", K::Cancellation).is_empty())
    };
    let both = two_enclosing("locus Leaf { run() { let n = 1; } }", "", true);
    assert_eq!(joined(&both), (false, true), "main and side: the run not joined");
    let side_only = both.replace("    run() { let m = Mid { }; m.make(); }\n}", "}");
    assert_eq!(joined(&side_only), (true, true), "side alone: the run joined");
}

/// A restart's steps, as an emitter reads them (L4): the decision, the
/// restart's entry, the next incarnation's birth, then its run where the
/// locus declares one (line 13, C48); nothing torn down in between.
#[test]
fn a_restart_reads_decision_entry_birth_then_run() {
    use hale_types::lifecycle::spine::RecoveryStep as R;
    let s = snapshot(include_str!("../../hale-codegen/tests/fixtures/lifecycle/rd_restart_during_teardown.hl"));
    let p = plan(&s);
    assert_eq!(p.recovery_order("Kid").unwrap(), [R::Decision, R::Restart, R::Birth, R::Run]);
    let site = p.templates("Kid").next().expect("a Kid template");
    let pairs = p.recovery_pairs(site);
    for pair in [(R::Decision, R::Restart), (R::Restart, R::Birth), (R::Birth, R::Run)] {
        assert!(pairs.contains(&pair), "{pair:?} is the rows' own: {pairs:?}");
    }
    let s = snapshot(include_str!("../../hale-codegen/tests/fixtures/lifecycle/l01_neg_same_pool_held.hl"));
    assert_eq!(plan(&s).recovery_order("Late").unwrap(), [R::Decision, R::Restart, R::Birth]);
}

/// Line 13: a locus that declares no run() owes none when it resumes;
/// the row says so, shipped (C48, L4), and owes no event.
#[test]
fn a_resumed_locus_with_no_run_owes_none() {
    let s = snapshot(include_str!("../../hale-codegen/tests/fixtures/lifecycle/l01_neg_same_pool_held.hl"));
    let p = plan(&s);
    // One per failure path that restarts it.
    let runs = rows(p, "Late", K::Run);
    assert!(!runs.is_empty());
    for run in runs {
        assert_eq!((run.guard, run.line, run.status), (PathGuard::Restart, Some("13"), Status::Shipped));
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
    assert_eq!(app_reclaim.edges.completion.iter().filter(|pr| pr.rule.line == Some("14")).count(), 3);
    for edge in app_reclaim.edges.completion.iter().filter(|pr| pr.rule.line == Some("14")) {
        assert_eq!(p.obligations[edge.event.obligation.0 as usize].kind, K::Reclaim);
        assert_eq!(edge.event.point, Point::Completed);
    }
}

/// Line 19 holds a started run against cross-pool field replacement as
/// well as queued cancellation. The replacement's inline run obeys the
/// same reclaim edge, so the trace projection can hold both instances
/// to the producer's rule without a hand-written plan.
#[test]
fn a_started_run_is_retained_until_reclaim_completes() {
    for src in [
        include_str!("../../hale-codegen/tests/fixtures/lifecycle/l19_started_run_retained.hl"),
        include_str!("../../hale-codegen/tests/fixtures/lifecycle/l19_started_run_retained_async.hl"),
        include_str!("../../hale-codegen/tests/fixtures/lifecycle/l19_started_run_publishes_back.hl"),
        include_str!("../../hale-codegen/tests/fixtures/lifecycle/l19_started_run_publishes_back_async.hl"),
        include_str!("../../hale-codegen/tests/fixtures/lifecycle/l19_handler_replaces_started_run.hl"),
        include_str!("../../hale-codegen/tests/fixtures/lifecycle/l19_handler_replaces_started_run_async.hl"),
    ] {
        let s = snapshot(src);
        let p = plan(&s);
        assert!(laws(p).is_empty(), "{:?}", laws(p));
        assert_eq!(rows(p, "Kid", K::Run).len(), 2);
        let subscriber = !rows(p, "App", K::Subscribe).is_empty();
        assert_eq!(one(p, "App", K::Drain).holder.spine,
                   if subscriber { Spine::DeferredMainEntry } else { Spine::EagerTeardown });
        assert_eq!(p.obligations.iter().any(|o| o.kind == K::PoolJoin && o.holder.spine == Spine::EagerTeardown), !subscriber,
                   "a statement-position subscriber joins at frame exit");
        for reclaim in rows(p, "Kid", K::Reclaim) {
            let run = p.obligations.iter().find(|o| o.kind == K::Run && o.site == reclaim.site).expect("this instance's run");
            let run_id = id_of(p, run);
            assert!(reclaim.edges.completion.iter().any(|pr| {
                pr.event.obligation == run_id && pr.event.point == Point::Ended && pr.rule == Rule::line("19", Status::Shipped)
            }), "each instance owes its run's end before reclaim completes");
            let drain = p.obligations.iter().find(|o| o.kind == K::Drain && o.site == reclaim.site).expect("this instance's drain");
            let waits_before_drain = drain.edges.entry.iter().any(|pr| pr.event.obligation == run_id);
            assert_eq!(waits_before_drain, !matches!(reclaim.site.as_ref().unwrap().template, Template::Static(_)),
                       "only the inline replacement owes run completion before drain");
        }
    }
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
/// wait-abort before the pool join and a pre-drain it does
/// not emit (C13).
#[test]
fn the_eager_spine_owes_the_wait_abort_before_the_join() {
    let s = snapshot(include_str!("../../hale-codegen/tests/fixtures/lifecycle/l07_pool_or_wait_teardown.hl"));
    let p = plan(&s);
    let process = |kind: K, spine: Spine| -> &Obligation {
        p.obligations.iter().find(|o| o.site.is_none() && o.kind == kind && o.holder.spine == spine).expect("a process row")
    };
    let join = process(K::PoolJoin, Spine::EagerTeardown);
    assert!(join.edges.entry.iter().any(|pr| {
        pr.rule == Rule { line: Some("7"), status: Status::Shipped }
            && pr.event.point == Point::Completed
            && p.obligations[pr.event.obligation.0 as usize].kind == K::WaitAbort
    }));
    assert_eq!(process(K::PreDrain, Spine::EagerTeardown).status, Status::KnownOpen { inventory_row: "C13" });
    // The pool's run ends before the join completes (line 19).
    let pusher_run = one(p, "Pusher", K::Run);
    let on = pusher_run.runs_on.clone().expect("a placed run names its domain");
    assert!(matches!(&p.domains[on.one().expect("one domain").0 as usize].kind, DomainKind::Pool { name, .. } if name == "side"));
    assert!(join.edges.completion.iter().any(|pr| pr.rule.line == Some("19") && pr.event.point == Point::Ended));
}

/// The fall-through spine follows the same abort-before-join rule as
/// eager teardown, before releasing the main frame's entries.
#[test]
fn the_main_fall_through_spine_owes_the_wait_abort_before_the_join() {
    let s = snapshot(include_str!("../../hale-codegen/tests/fixtures/lifecycle/l07_or_wait_main_fall_through.hl"));
    let p = plan(&s);
    let join = p.obligations.iter().find(|o| {
        o.site.is_none() && o.kind == K::PoolJoin && o.holder.spine == Spine::MainFallThrough
    }).expect("the main fall-through pool join");
    assert!(join.edges.entry.iter().any(|pr| {
        let predecessor = &p.obligations[pr.event.obligation.0 as usize];
        pr.rule == Rule { line: Some("7"), status: Status::Shipped }
            && pr.event.point == Point::Completed
            && predecessor.kind == K::WaitAbort
            && predecessor.holder.spine == Spine::MainFallThrough
    }));
    let pre = p.obligations.iter().find(|o| {
        o.site.is_none() && o.kind == K::PreDrain && o.holder.spine == Spine::MainFallThrough
    }).expect("the main frame pre-drain");
    assert!(pre.edges.entry.iter().any(|pr| {
        let predecessor = &p.obligations[pr.event.obligation.0 as usize];
        pr.rule.status == Status::Shipped
            && pr.event.point == Point::Completed
            && predecessor.kind == K::PoolJoin
            && predecessor.holder.spine == Spine::MainFallThrough
    }));
    for decl in ["App", "Pusher", "Tally"] {
        let drain = one(p, decl, K::Drain);
        assert!(drain.edges.entry.iter().any(|pr| {
            let predecessor = &p.obligations[pr.event.obligation.0 as usize];
            pr.rule.status == Status::Shipped
                && pr.event.point == Point::Completed
                && predecessor.kind == K::PreDrain
                && predecessor.holder.spine == Spine::MainFallThrough
        }), "{decl}'s drain waits for the frame pre-drain");
    }
}

/// Lines 7 and 16, from one source: every teardown spine states its
/// process rows and the plan's reader orders them. The quiesce, the
/// wait-abort, the join on the main locus's own teardown, eager or
/// deferred; `fn main`'s three exits likewise, then the frame's
/// pre-drain, or, without pools, the pre-drain ahead of the wait-abort.
/// The eager spine's pre-drain is the one it does not emit (C13).
#[test]
fn every_teardown_spine_states_its_process_rows() {
    let pool_app = "locus Worker { run() { } }\n\
                    main locus App { params { w: Worker = Worker { }; } placement { w: cooperative(pool = side); } run() { } }\n";
    let bare_app = "main locus App { run() { } }\n";
    let order = |app: &str, main: &str, spine: Spine| -> Vec<K> {
        let s = snapshot(&format!("{app}{main}"));
        let p = plan(&s);
        assert!(laws(p).is_empty(), "{:?}", laws(p));
        p.process_order(spine).expect("one order").iter().map(|st| st.kind).collect()
    };
    let eager = "fn main() { App { }; }\n";
    let deferred = "fn start() { let app = App { }; }\nfn main() { start(); }\n";
    let let_bound = "fn main() { let app = App { }; }\n";
    let head = [K::IngressQuiesce, K::WaitAbort, K::PoolJoin, K::JoinProgress];
    assert_eq!(order(pool_app, eager, Spine::EagerTeardown), [&[K::PreDrain][..], &head].concat());
    assert_eq!(order(bare_app, eager, Spine::EagerTeardown), vec![K::PreDrain, K::IngressQuiesce, K::WaitAbort]);
    assert_eq!(order(pool_app, deferred, Spine::DeferredMainEntry), head.to_vec());
    assert_eq!(order(pool_app, let_bound, Spine::DeferredMainEntry), head.to_vec());
    for spine in [Spine::MainFallThrough, Spine::MainReturn, Spine::MainTestFailure] {
        assert_eq!(order(pool_app, let_bound, spine), [&head[..], &[K::PreDrain]].concat(), "{}", spine.name());
        assert_eq!(order(bare_app, let_bound, spine), vec![K::IngressQuiesce, K::PreDrain, K::WaitAbort], "{}", spine.name());
    }
    // A main locus built by another fn is torn down at that fn's exit,
    // before `fn main`'s: its head comes first.
    let s = snapshot(&format!("{pool_app}{deferred}"));
    let p = plan(&s);
    let row = |kind: K, spine: Spine| p.iter().find(|(_, o)| o.site.is_none() && o.kind == kind && o.holder.spine == spine).expect("a row").0;
    let (entry_join, exit_quiesce) = (row(K::PoolJoin, Spine::DeferredMainEntry), row(K::IngressQuiesce, Spine::MainFallThrough));
    assert!(p.get(exit_quiesce).expect("a row").edges.entry.iter().any(|pr| pr.event.obligation == entry_join));
    let eager_pre = p.iter().find(|(_, o)| o.kind == K::PreDrain && o.holder.spine == Spine::EagerTeardown);
    assert!(eager_pre.is_none(), "no eager statement here");
    let s = snapshot(&format!("{pool_app}{eager}"));
    let p = plan(&s);
    let pre = p.iter().find(|(_, o)| o.kind == K::PreDrain && o.holder.spine == Spine::EagerTeardown).expect("a pre-drain").1;
    assert_eq!(pre.status, Status::KnownOpen { inventory_row: "C13" });
}

/// A subscribing main instance uses its deferred main-entry spine;
/// static posted fields also owe cancellation if reclaimed while queued.
#[test]
fn deferred_main_and_cross_pool_cancellation_name_their_spines() {
    let s = snapshot(include_str!("../../hale-codegen/tests/fixtures/lifecycle/l19_handler_replaces_started_run.hl"));
    let p = plan(&s);
    assert_eq!(one(p, "App", K::Reclaim).holder.spine, Spine::DeferredMainEntry);
    let canceled = rows(p, "Kid", K::Cancellation);
    assert!(canceled.iter().any(|o| o.holder.spine == Spine::Cascade && o.guard == PathGuard::DrainInFlight));
    // Its reclaim happens on main, so the child's worker must not be
    // asserted as the cancellation's execution domain.
    assert!(canceled.iter().filter(|o| o.holder.spine == Spine::Cascade).all(|o| o.runs_on.is_none()));
}

/// L4's ruling on the empty run: a `Run` is owed exactly where lowering
/// calls `run()` (`hale_types::lifecycle::run_is_called`, which both
/// read). An author-written empty `run() { }` is not called and owes
/// none; a run with a body is; a flow's is called even with no run of its
/// own, since its run wrapper reclaims it; a pinned locus's thread takes
/// the step whatever the body.
#[test]
fn a_run_is_owed_exactly_where_lowering_calls_it() {
    let s = snapshot(
        "locus Quiet { run() { } }\n\
         locus Busy { run() { println(\"busy\"); } }\n\
         locus Kid { params { n: Int = 0; } }\n\
         locus Spin { run() { } }\n\
         locus Holder {\n\
             accept(k: Kid) { }\n\
             release(k: Kid) { }\n\
             fn spawn() { Kid { }; }\n\
             run() { self.spawn(); }\n\
         }\n\
         main locus App {\n\
             params { holder: Holder = Holder { }; spin: Spin = Spin { }; }\n\
             placement { spin: pinned; }\n\
             run() { Quiet { }; Busy { }; }\n\
         }\n\
         fn main() { App { }; }\n",
    );
    let p = plan(&s);
    let normal = |decl: &str| -> Vec<&Obligation> {
        rows(p, decl, K::Run).into_iter().filter(|o| o.guard == PathGuard::Normal).collect()
    };
    assert!(normal("Quiet").is_empty(), "an empty run() is not called and owes no Run");
    assert_eq!(normal("Busy").len(), 1, "a run() with a body is called");
    assert_eq!(normal("Kid").len(), 1, "a flow's run wrapper calls its run(), the desugar's empty one included");
    let spin = normal("Spin");
    assert_eq!(spin.len(), 1, "a pinned thread takes its Run step");
    assert_eq!(spin[0].holder.spine, Spine::PinnedMain);
    assert!(hale_types::lifecycle::run_is_called(false, false));
    assert!(!hale_types::lifecycle::run_is_called(true, false));
    assert!(hale_types::lifecycle::run_is_called(true, true));
}

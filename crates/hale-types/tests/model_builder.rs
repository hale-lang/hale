//! GH #476 Change 2 — the ApplicationModel builder.
//!
//! Two proof obligations:
//!
//!   1. **Lawfulness**: every derived model passes
//!      `ApplicationModel::validate()` — all 31 schema laws — over
//!      fixtures exercising each table family (calls at site grain,
//!      stdlib contraction, keyed delivery incl. fallback, bounds,
//!      literal/wildcard endpoints, groups with authored selectors,
//!      supervision incl. external children, the declaration
//!      universe, holes vs dead dispatches).
//!   2. **Agreement**: the model and `dump_topology` extract the
//!      SAME facts from one bundle — the Change-2 exit criterion
//!      ("model rows match existing semantic fragments") and the
//!      seed of Change 3's projection differential. Where the two
//!      encode differently (sites vs merged endpoints, holes+dead
//!      vs stringly unknowns), the test projects the model down and
//!      compares.

use std::collections::{BTreeMap, BTreeSet};

use hale_model::{
    DispatchKind, EntityRef, HoleKind, KeyOnUnmatched, KeyPredicate,
    SelectorForm, SupervisedRef,
};
use hale_types::model_builder::derive_application_model;
use hale_types::Bundle;

fn bundle_of(src: &str) -> (hale_syntax::ast::Program, ()) {
    (hale_syntax::parse_source(src).expect("parse"), ())
}

fn derive(src: &str) -> hale_model::ApplicationModel {
    let (program, ()) = bundle_of(src);
    let mut programs = BTreeMap::new();
    programs.insert("app.hl".to_string(), &program);
    let bundle = Bundle::new(programs);
    let m = derive_application_model(&bundle);
    m.validate().expect("derived model is lawful");
    m
}

const RICH: &str = r#"
type Reading { sensor: Int = 0; v: Int = 0; }
type Cmd { op: Int = 0; }
type Event { n: Int = 0; }
topic Readings {
    payload: Reading;
    subject: "sense.reading";
    keyed_by sensor;
    on_unmatched: fallback;
}
topic Cmds { payload: Cmd; subject: "ctl.cmd"; bounded(64); on_full: fail; }

interface Notifier {
    fn notify(v: Int) -> Int;
}

fn double(v: Int) -> Int { return v * 2; }
fn call_it(f: fn (Int) -> Int, v: Int) -> Int { return f(v); }

locus Worker {
    params { seen: Int = 0; }
    bus {
        subscribe Readings as on_r where key == replica;
        publish Cmds;
    }
    fn on_r(r: Reading) {
        self.seen = self.seen + 1;
        if r.v > 100 { Cmds <- Cmd { op: call_it(double, r.v) } or discard; }
    }
}

locus Catcher {
    params { seen: Int = 0; }
    bus { subscribe Readings as on_any bounded(8, drop_old) where key == _; }
    fn on_any(r: Reading) { self.seen = self.seen + 1; }
}

locus Store {
    params { total: Int = 0; }
    bus {
        subscribe Cmds as on_c;
        subscribe "audit.**" as on_audit of type Event;
    }
    fn on_c(c: Cmd) { self.total = self.total + double(c.op); }
    fn on_audit(e: Event) { self.total = self.total + e.n; }
    on_failure(w: Worker, err: ClosureViolation) {
        restart (w) for 3;
    }
}

group stores = { Store };
group workers = { Worker, double };

main locus App {
    params { w: Worker = Worker { }; c: Catcher = Catcher { }; s: Store = Store { }; }
    bus { publish Readings; publish "audit.trace" of type Event; }
    run() {
        Readings <- Reading { sensor: 1, v: 10 };
        "audit.trace" <- Event { n: 1 };
    }
}
fn main() { App { }; }
"#;

#[test]
fn the_rich_fixture_derives_a_lawful_model_with_every_family() {
    let m = derive(RICH);
    let e = &m.entities;

    // Sorts.
    let loci: Vec<&str> =
        e.loci.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(loci, ["App", "Catcher", "Store", "Worker"]);
    assert!(e.functions.iter().any(|f| f.name == "call_it"));
    assert_eq!(e.topics.len(), 2);

    // Keyed topic facts survive: key field, fallback policy.
    let readings =
        e.topics.iter().find(|t| t.name == "Readings").unwrap();
    let k = readings.key.as_ref().expect("keyed");
    assert_eq!(k.field, "sensor");
    assert_eq!(k.on_unmatched, KeyOnUnmatched::Fallback);
    // Topic bound (the #255 publisher-facing knob).
    let cmds = e.topics.iter().find(|t| t.name == "Cmds").unwrap();
    assert_eq!(cmds.bound.map(|b| b.capacity), Some(64));

    // Wire subjects are the subject sort; the wildcard and literal
    // endpoints are subjects WITHOUT topic rows.
    let pats: BTreeSet<&str> =
        e.subjects.iter().map(|s| s.pattern.as_str()).collect();
    assert!(pats.contains("sense.reading"));
    assert!(pats.contains("audit.**"));
    assert!(pats.contains("audit.trace"));

    // Subscriptions: EqReplica, Fallback, plain, and wildcard rows.
    let r = &m.relations;
    let preds: Vec<&KeyPredicate> =
        r.subscribes.iter().map(|s| &s.key_predicate).collect();
    assert!(preds.iter().any(|p| **p == KeyPredicate::EqReplica));
    assert!(preds.iter().any(|p| **p == KeyPredicate::Fallback));
    // The bounded subscription carries its capacity + shed policy.
    assert!(r.subscribes.iter().any(|s| matches!(
        (s.capacity, s.shed),
        (
            hale_model::Capacity::Bounded(8),
            hale_model::ShedPolicy::DropOld
        )
    )));
    // The wildcard subscription is declared-topic-less.
    assert!(r
        .subscribes
        .iter()
        .any(|s| s.declared_topic.is_none()));

    // Publishes: the literal-subject publish has no declared topic;
    // the keyed publish carries an AnyOfType domain (typed, not
    // Unknown — no spurious hole).
    assert!(r.publishes.iter().any(|p| p.declared_topic.is_none()));
    assert!(r
        .publishes
        .iter()
        .any(|p| matches!(&p.key_domain, Some(hale_model::KeyDomain::AnyOfType(_)))));

    // Calls: the indirect call is a HOLE (not a row); double's
    // direct call from Store::on_c is a site row.
    assert!(m
        .holes
        .iter()
        .any(|h| h.kind == HoleKind::IndirectCall));
    assert!(!m.capabilities.exact_calls, "indirect call ⇒ inexact");
    let fname = |id: hale_model::FunctionId| {
        e.functions[id.index()].name.clone()
    };
    assert!(r.calls.iter().any(|c| fname(c.from) == "Store::on_c"
        && fname(c.to) == "double"
        && c.dispatch == DispatchKind::Direct));

    // Supervision: per-handler row with error type + retry bound.
    let sup = &r.supervises[0];
    assert!(matches!(sup.child, SupervisedRef::Locus(_)));
    assert_eq!(sup.error_type, "ClosureViolation");
    assert_eq!(sup.policy.retry_bound, Some(3));
    assert_eq!(sup.policy.ops, ["restart"]);

    // Groups: authored selectors in order; the both-form member
    // list resolves loci AND free fns.
    let sel: Vec<String> = r
        .group_selectors
        .iter()
        .map(|s| match &s.selector {
            SelectorForm::Named { display, .. } => display.clone(),
            SelectorForm::SeedGlob { display, .. } => display.clone(),
        })
        .collect();
    assert_eq!(sel, ["Store", "Worker", "double"]);
    assert!(r.group_members.iter().any(|gm| matches!(
        gm.member,
        EntityRef::Function(_)
    )));

    // Declaration universe: types + the interface present.
    assert!(e.types.iter().any(|t| t.name == "Reading"));
    assert!(e.interfaces.iter().any(|i| i.name == "Notifier"));
}

#[test]
fn model_and_artifact_extract_the_same_facts() {
    let (program, ()) = bundle_of(RICH);
    let mut programs = BTreeMap::new();
    programs.insert("app.hl".to_string(), &program);
    let bundle = Bundle::new(programs);
    let m = derive_application_model(&bundle);
    let art: serde_json::Value = serde_json::from_str(
        &hale_types::topology::dump_topology(&bundle),
    )
    .expect("artifact parses");

    let e = &m.entities;
    let fname =
        |id: hale_model::FunctionId| e.functions[id.index()].name.clone();

    // fn sort equality.
    let art_fns: BTreeSet<String> = art["sorts"]["fns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let model_fns: BTreeSet<String> =
        e.functions.iter().map(|f| f.name.clone()).collect();
    assert_eq!(art_fns, model_fns, "fn sorts agree");

    // loci sort equality.
    let art_loci: BTreeSet<String> = art["sorts"]["loci"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let model_loci: BTreeSet<String> =
        e.loci.iter().map(|l| l.name.clone()).collect();
    assert_eq!(art_loci, model_loci, "locus sorts agree");

    // calls: endpoint projection of the model's site rows equals the
    // artifact's merged relation (Direct + Interface dispatch; the
    // ViaStdlib rows correspond to calls_via_stdlib).
    let art_calls: BTreeSet<(String, String)> = art["relations"]["calls"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["from"].as_str().unwrap().to_string(),
                r["to"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    let model_calls: BTreeSet<(String, String)> = m
        .relations
        .calls
        .iter()
        .filter(|c| c.dispatch != DispatchKind::ViaStdlib)
        .map(|c| (fname(c.from), fname(c.to)))
        .collect();
    assert_eq!(art_calls, model_calls, "call endpoints agree");
    let art_via: BTreeSet<(String, String)> = art["relations"]
        ["calls_via_stdlib"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["from"].as_str().unwrap().to_string(),
                r["to"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    let model_via: BTreeSet<(String, String)> = m
        .relations
        .calls
        .iter()
        .filter(|c| c.dispatch == DispatchKind::ViaStdlib)
        .map(|c| (fname(c.from), fname(c.to)))
        .collect();
    assert_eq!(art_via, model_via, "through-stdlib contraction agrees");

    // publishes: (fn, display subject) sets agree. The artifact
    // spells declared topics by NAME; the model rows carry the wire
    // subject + declared topic — project back to the display.
    let display_of = |p: &hale_model::Publish| -> String {
        match p.declared_topic {
            Some(t) => e.topics[t.index()].name.clone(),
            None => e.subjects[p.subject.index()].pattern.clone(),
        }
    };
    let art_pubs: BTreeSet<(String, String)> = art["relations"]
        ["publishes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["fn"].as_str().unwrap().to_string(),
                r["subject"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    let model_pubs: BTreeSet<(String, String)> = m
        .relations
        .publishes
        .iter()
        .map(|p| (fname(p.function), display_of(p)))
        .collect();
    assert_eq!(art_pubs, model_pubs, "publish endpoints agree");

    // subscribes: (subject display, handler) agree.
    let art_subs: BTreeSet<(String, String, String)> = art["relations"]
        ["subscribes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["subject"].as_str().unwrap().to_string(),
                r["locus"].as_str().unwrap().to_string(),
                r["handler"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    let model_subs: BTreeSet<(String, String, String)> = m
        .relations
        .subscribes
        .iter()
        .map(|s| {
            let display = match s.declared_topic {
                Some(t) => e.topics[t.index()].name.clone(),
                None => e.subjects[s.subject.index()].pattern.clone(),
            };
            let handler_full = fname(s.handler);
            let (locus, handler) =
                handler_full.rsplit_once("::").expect("locus handler");
            (display, locus.to_string(), handler.to_string())
        })
        .collect();
    assert_eq!(art_subs, model_subs, "subscription endpoints agree");

    // unknowns: the artifact's stringly reasons correspond to the
    // model's typed holes (the dead-dispatch species would land in
    // dead_interface_calls — none in this fixture).
    let art_unknown_fns: BTreeSet<String> = art["unknowns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["fn"].as_str().unwrap().to_string())
        .collect();
    let model_hole_fns: BTreeSet<String> = m
        .holes
        .iter()
        .filter_map(|h| match h.at {
            EntityRef::Function(id) => Some(fname(id)),
            _ => None,
        })
        .collect();
    assert_eq!(art_unknown_fns, model_hole_fns, "residue anchors agree");

    // groups: the artifact's as-authored member lists equal the
    // model's authored selector displays, per group, in order.
    let art_groups = art["groups"].as_object().unwrap();
    for g in &e.groups {
        let art_members: Vec<String> = art_groups[&g.name]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        let gid = hale_model::GroupId(
            e.groups.iter().position(|x| x.name == g.name).unwrap()
                as u32,
        );
        let mut sel: Vec<(u32, String)> = m
            .relations
            .group_selectors
            .iter()
            .filter(|s| s.group == gid)
            .map(|s| {
                (
                    s.ordinal,
                    match &s.selector {
                        SelectorForm::Named { display, .. } => {
                            display.clone()
                        }
                        SelectorForm::SeedGlob { display, .. } => {
                            display.clone()
                        }
                    },
                )
            })
            .collect();
        sel.sort();
        let model_members: Vec<String> =
            sel.into_iter().map(|(_, d)| d).collect();
        assert_eq!(
            art_members, model_members,
            "group `{}` authored selectors agree",
            g.name
        );
    }

    // supervision agrees (locus + child spelling).
    let art_sup: BTreeSet<(String, String)> = art["supervision"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["locus"].as_str().unwrap().to_string(),
                r["child"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    let model_sup: BTreeSet<(String, String)> = m
        .relations
        .supervises
        .iter()
        .map(|s| {
            (
                e.loci[s.parent.index()].name.clone(),
                match &s.child {
                    SupervisedRef::Locus(id) => {
                        e.loci[id.index()].name.clone()
                    }
                    SupervisedRef::External(n) => n.clone(),
                },
            )
        })
        .collect();
    assert_eq!(art_sup, model_sup, "supervision rows agree");

    // effects: per-fn derived classes agree.
    let art_effects = art["effects"].as_object().unwrap();
    for f in &e.functions {
        let art_classes: Vec<String> = art_effects
            .get(&f.name)
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .map(|x| x.as_str().unwrap().to_string())
                    .collect()
            })
            .unwrap_or_default();
        assert_eq!(
            art_classes, f.effects,
            "effects agree for {}",
            f.name
        );
    }
}

#[test]
fn dead_dispatch_and_indirect_are_separated() {
    // A call through an uninhabited interface + a genuine indirect
    // call: one dead row, one hole — never conflated.
    let src = r#"
interface Notifier {
    fn notify(v: Int) -> Int;
}
fn call_it(f: fn (Int) -> Int, v: Int) -> Int { return f(v); }
fn poke(n: Notifier, v: Int) -> Int { return n.notify(v); }
fn id(v: Int) -> Int { return v; }
main locus App {
    run() {
        let a = call_it(id, 1);
        println(a);
    }
}
fn main() { App { }; }
"#;
    let m = derive(src);
    assert!(
        m.relations
            .dead_interface_calls
            .iter()
            .any(|d| d.interface == "Notifier" && d.method == "notify"),
        "uninhabited dispatch is a dead row: {:?}",
        m.relations.dead_interface_calls
    );
    assert!(m
        .holes
        .iter()
        .any(|h| h.kind == HoleKind::IndirectCall));
    // The dead row does NOT make calls inexact; the indirect hole
    // does.
    assert!(!m.capabilities.exact_calls);
}

#[test]
fn a_clean_program_claims_exactness() {
    let src = r#"
type T { n: Int = 0; }
topic Evt { payload: T; subject: "evt"; }
locus Sub {
    params { seen: Int = 0; }
    bus { subscribe Evt as on_e; }
    fn on_e(t: T) { self.seen = self.seen + 1; }
}
main locus App {
    params { s: Sub = Sub { }; }
    bus { publish Evt; }
    run() { Evt <- T { n: 1 }; }
}
fn main() { App { }; }
"#;
    let m = derive(src);
    assert!(m.holes.is_empty());
    assert!(m.capabilities.exact_calls);
    assert!(m.capabilities.exact_bus_endpoints);
    assert!(m.capabilities.exact_key_filters);
    // Never claimed at Change 2 (empty tables, deferred adapters):
    assert!(!m.capabilities.exact_ownership);
    assert!(!m.capabilities.exact_placement);
    assert!(!m.capabilities.exact_routes);
}

#[test]
fn derivation_is_deterministic() {
    let (program, ()) = bundle_of(RICH);
    let mut programs = BTreeMap::new();
    programs.insert("app.hl".to_string(), &program);
    let bundle = Bundle::new(programs);
    let a = hale_types::model_builder::render_internal(
        &derive_application_model(&bundle),
    );
    let b = hale_types::model_builder::render_internal(
        &derive_application_model(&bundle),
    );
    assert_eq!(a, b, "two derivations render identically");
}

/// The corpus property, two-sided:
///   - derivation NEVER panics on any parseable program;
///   - a derived model may fail the schema laws ONLY where the
///     checker also refuses the program (negative-test fixtures
///     with deliberately ill-formed keyed/fallback usage derive
///     models whose law violations MIRROR the checker's own
///     refusals — `IllegalFallback` where the checker rejects a
///     stray `_`, `KeyContract` where a filter hits an unkeyed
///     topic). A program that CHECKS clean and derives an unlawful
///     model is a builder bug, full stop.
#[test]
fn every_corpus_program_derives_a_lawful_model() {
    let mut bad: Vec<String> = Vec::new();
    for p in hale_corpus::parseable(|s| hale_syntax::parse_source(s).is_ok())
    {
        let Ok(program) = hale_syntax::parse_source(&p.source) else {
            continue;
        };
        let caught =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut programs = BTreeMap::new();
                programs.insert("app.hl".to_string(), &program);
                let bundle = Bundle::new(programs);
                let m = derive_application_model(&bundle);
                m.validate().map_err(|e| format!("{:?}", e))
            }));
        match caught {
            Err(_) => bad.push(format!("{}: PANIC", p.origin)),
            Ok(Err(e)) => {
                let checks_clean = hale_types::check_program(&program)
                    .iter()
                    .all(|d| !d.is_error());
                if checks_clean {
                    bad.push(format!(
                        "{}: checks clean but unlawful: {}",
                        p.origin, e
                    ));
                }
            }
            Ok(Ok(())) => {}
        }
    }
    assert!(
        bad.is_empty(),
        "{} corpus programs violate the model property:\n{}",
        bad.len(),
        bad.join("\n")
    );
}

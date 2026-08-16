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
        Cmds <- Cmd { op: 0 } or raise;
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
    // The model's universe is DECLARATIONS (round 6): a superset of
    // the artifact's summary-derived sort — an empty fn exists here
    // and not there.
    assert!(
        art_fns.is_subset(&model_fns),
        "artifact fn sort ⊆ model universe; missing: {:?}",
        art_fns.difference(&model_fns).collect::<Vec<_>>()
    );
    // …and the legacy projection recovers the EXACT legacy sort from
    // the model alone (round 7): no summary/AST side channel for
    // Change 3.
    let legacy_fns: BTreeSet<String> = m
        .legacy
        .topology_v1_fns
        .iter()
        .map(|id| e.functions[id.index()].display.clone())
        .collect();
    assert_eq!(
        art_fns, legacy_fns,
        "legacy.topology_v1_fns projects the artifact's fn sort exactly"
    );

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
    // The model's residue is a SUPERSET: it holes bodies the legacy
    // summary never walked (on_failure, module scope). Every
    // artifact anchor must be a model hole, and every extra must be
    // exactly that richer species.
    let model_hole_fns: BTreeSet<String> = m
        .holes
        .iter()
        .filter(|h| h.kind != HoleKind::UnanalyzedBody)
        .filter_map(|h| match h.at {
            EntityRef::Function(id) => Some(fname(id)),
            _ => None,
        })
        .collect();
    assert_eq!(
        art_unknown_fns, model_hole_fns,
        "legacy-visible residue anchors agree"
    );
    assert!(
        m.holes
            .iter()
            .any(|h| h.kind == HoleKind::UnanalyzedBody),
        "the on_failure body holes out (model-richer residue)"
    );

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

// -----------------------------------------------------------------
// Review round 6 — identity separation, dispositions, sites, the
// declaration-derived function universe, and key types.
// -----------------------------------------------------------------

/// Builds a post-merge bundle the way the import pass does: the
/// imported seed's decls arrive mangled, and import_renames maps
/// author paths to mangled names.
fn xseed_bundle(
    main_src: &str,
    lib_src: &str,
    renames: &[(&[&str], &str)],
) -> hale_model::ApplicationModel {
    let main_p = hale_syntax::parse_source(main_src).expect("parse main");
    let lib_p = hale_syntax::parse_source(lib_src).expect("parse lib");
    let mut programs = BTreeMap::new();
    programs.insert("app/main.hl".to_string(), &main_p);
    programs.insert("lib/events.hl".to_string(), &lib_p);
    let mut bundle = Bundle::new(programs);
    bundle.import_renames = renames
        .iter()
        .map(|(segs, m)| {
            (
                segs.iter().map(|s| s.to_string()).collect(),
                m.to_string(),
            )
        })
        .collect();
    let m = derive_application_model(&bundle);
    m.validate().expect("cross-seed model is lawful");
    m
}

/// P1 (round 6): raw identity vs author spelling for imported
/// topics — the wire subject is the DECLARED subject (or the raw
/// mangled default), never the author spelling; the topic NAME is
/// the author spelling; named selectors store author spelling.
#[test]
fn imported_topic_identities_stay_separated() {
    let lib = r#"
type __lib_e_events_Order { id: Int = 0; }
topic __lib_e_events_Orders {
    payload: __lib_e_events_Order;
    subject: "orders.wire";
}
topic __lib_e_events_Audit { payload: __lib_e_events_Order; }
locus __lib_e_events_Worker {
    params { n: Int = 0; }
    fn poke(v: Int) -> Int { return v + self.n; }
}
"#;
    let main_src = r#"
group workers = { __lib_e_events_Worker };
locus Sub {
    params { seen: Int = 0; }
    bus { subscribe __lib_e_events_Orders as on_o; }
    fn on_o(o: __lib_e_events_Order) { self.seen = self.seen + 1; }
}
main locus App {
    params { s: Sub = Sub { }; }
    run() { println(self.s.seen); }
}
fn main() { App { }; }
"#;
    let m = xseed_bundle(
        main_src,
        lib,
        &[
            (&["e", "Order"], "__lib_e_events_Order"),
            (&["e", "Orders"], "__lib_e_events_Orders"),
            (&["e", "Audit"], "__lib_e_events_Audit"),
            (&["e", "Worker"], "__lib_e_events_Worker"),
        ],
    );
    let e = &m.entities;
    // Topic names are author-spelled…
    let orders = e
        .topics
        .iter()
        .find(|t| t.display == "e::Orders")
        .unwrap();
    assert_eq!(
        orders.name, "__lib_e_events_Orders",
        "canonical topic identity is the RAW post-merge symbol"
    );
    // …but the wire subject is the DECLARED subject, raw.
    assert_eq!(
        e.subjects[orders.subject.index()].pattern, "orders.wire",
        "explicit subject survives import demangling"
    );
    // A subject-less imported topic defaults to the RAW mangled
    // name (the byte-exact runtime join key), NOT `e::Audit`.
    let audit = e
        .topics
        .iter()
        .find(|t| t.display == "e::Audit")
        .unwrap();
    assert_eq!(
        e.subjects[audit.subject.index()].pattern,
        "__lib_e_events_Audit",
        "subject-less imported topic keeps its raw wire identity"
    );
    // Named selector display is author-spelled even though the
    // import pass collapsed the member to a mangled segment.
    let sel = &m.relations.group_selectors[0];
    match &sel.selector {
        SelectorForm::Named { display, .. } => {
            assert_eq!(display, "e::Worker", "selector display demangled")
        }
        other => panic!("expected Named selector, got {:?}", other),
    }
}

/// P1 (round 6): payload identity is SHAPE-only. Equal shapes on
/// two subjects share one contract; a field change on a literal
/// endpoint's type changes its contract; a rename that keeps the
/// structure does not.
#[test]
fn payload_identity_is_structural() {
    let two_subjects = r#"
type Evt { n: Int = 0; }
topic A { payload: Evt; subject: "wire.a"; }
topic B { payload: Evt; subject: "wire.b"; }
locus S {
    params { n: Int = 0; }
    bus { subscribe A as on_a; subscribe B as on_b; }
    fn on_a(e: Evt) { self.n = self.n + e.n; }
    fn on_b(e: Evt) { self.n = self.n + e.n; }
}
main locus App {
    params { s: S = S { }; }
    bus { publish A; publish B; }
    run() { A <- Evt { n: 1 }; B <- Evt { n: 2 }; }
}
fn main() { App { }; }
"#;
    let m = derive(two_subjects);
    let a = m.entities.topics.iter().find(|t| t.name == "A").unwrap();
    let b = m.entities.topics.iter().find(|t| t.name == "B").unwrap();
    assert_eq!(
        a.payload, b.payload,
        "one shape on two subjects = ONE payload contract"
    );

    let lit = |fields: &str| {
        format!(
            r#"
type Event {{ {} }}
locus S {{
    params {{ n: Int = 0; }}
    bus {{ subscribe "orders.created" as on_e of type Event; }}
    fn on_e(e: Event) {{ self.n = self.n + 1; }}
}}
main locus App {{
    params {{ s: S = S {{ }}; }}
    run() {{ println(self.n_of()); }}
    fn n_of() -> Int {{ return 0; }}
}}
fn main() {{ App {{ }}; }}
"#,
            fields
        )
    };
    let shape_of = |src: &str| -> (String, u64) {
        let m = derive(src);
        let sub = &m.relations.subscribes[0];
        let p = &m.entities.payloads[sub.payload.index()];
        (p.shape.clone(), p.hash)
    };
    let v1 = shape_of(&lit("id: Int = 0;"));
    let v2 = shape_of(&lit("id: Int = 0; amount: Decimal = 0.0d;"));
    assert_ne!(
        v1, v2,
        "adding a field to a literal endpoint's type MUST change \
         its contract"
    );
    // Rename without structural change: same shape.
    let renamed = lit("id: Int = 0;").replace("Event", "Evt2");
    assert_eq!(
        v1,
        shape_of(&renamed),
        "renaming a type without changing structure keeps the contract"
    );
}

/// P1 (round 6): dispositions survive. Two sends to one topic in
/// one function — `or discard` and `or raise` — are two site rows
/// with distinct dispositions.
#[test]
fn publish_dispositions_are_preserved_per_site() {
    let m = derive(RICH);
    let e = &m.entities;
    let cmds_rows: Vec<_> = m
        .relations
        .publishes
        .iter()
        .filter(|p| {
            p.declared_topic
                .map(|t| e.topics[t.index()].name == "Cmds")
                .unwrap_or(false)
        })
        .collect();
    assert_eq!(cmds_rows.len(), 2, "two authored sites on Cmds");
    let dispositions: BTreeSet<_> = cmds_rows
        .iter()
        .map(|p| format!("{:?}", p.disposition))
        .collect();
    assert_eq!(
        dispositions,
        ["Discard".to_string(), "Raise".to_string()]
            .into_iter()
            .collect(),
        "each site keeps ITS disposition"
    );
    assert_ne!(cmds_rows[0].site, cmds_rows[1].site);
}

/// P1 (round 6): AnyOfType names the key's TYPE, not the field.
#[test]
fn key_domain_names_the_key_type() {
    let m = derive(RICH);
    let keyed: Vec<_> = m
        .relations
        .publishes
        .iter()
        .filter_map(|p| match &p.key_domain {
            Some(hale_model::KeyDomain::AnyOfType(t)) => Some(t.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(keyed, ["Int"], "keyed_by sensor (an Int field) → Int");
}

/// P1 (round 6): one interface dispatch = ONE authored site shared
/// by every conformer row, and adding a conformer must not renumber
/// later calls.
#[test]
fn interface_alternatives_share_one_site() {
    let base = |extra_conformer: &str| {
        format!(
            r#"
interface Notifier {{
    fn notify(v: Int) -> Int;
}}
locus Bell {{
    params {{ n: Int = 0; }}
    fn notify(v: Int) -> Int {{ return v + 1; }}
}}
locus Horn {{
    params {{ n: Int = 0; }}
    fn notify(v: Int) -> Int {{ return v + 2; }}
}}
{}
fn after(v: Int) -> Int {{ return v; }}
fn go(x: Notifier, v: Int) -> Int {{
    let a = x.notify(v);
    let b = after(a);
    return b;
}}
main locus App {{
    run() {{
        let e = Bell {{ }};
        println(go(e, 1));
    }}
}}
fn main() {{ App {{ }}; }}
"#,
            extra_conformer
        )
    };
    let site_facts = |src: &str| {
        let m = derive(src);
        let e = m.entities.clone();
        let fname = |id: hale_model::FunctionId| {
            e.functions[id.index()].name.clone()
        };
        let iface_sites: BTreeSet<u32> = m
            .relations
            .calls
            .iter()
            .filter(|c| {
                matches!(c.dispatch, DispatchKind::Interface { .. })
            })
            .map(|c| c.site)
            .collect();
        let iface_rows = m
            .relations
            .calls
            .iter()
            .filter(|c| {
                matches!(c.dispatch, DispatchKind::Interface { .. })
            })
            .count();
        let after_site = m
            .relations
            .calls
            .iter()
            .find(|c| fname(c.to) == "after")
            .map(|c| c.site)
            .expect("the later direct call exists");
        (iface_rows, iface_sites, after_site)
    };
    let (rows2, sites2, after2) = site_facts(&base(""));
    assert_eq!(rows2, 2, "two conformers = two rows");
    assert_eq!(sites2.len(), 1, "…sharing ONE authored site");
    let (rows3, sites3, after3) = site_facts(&base(
        "locus Siren { params { n: Int = 0; } \
         fn notify(v: Int) -> Int { return v + 3; } }",
    ));
    assert_eq!(rows3, 3, "three conformers = three rows");
    assert_eq!(sites3.len(), 1, "…still one authored site");
    assert_eq!(
        after2, after3,
        "adding a conformer must not renumber the later call"
    );
}

/// P1 (round 6): the function universe comes from DECLARATIONS. An
/// empty free fn exists (and stays groupable); a mode is a Mode,
/// not a Hook.
#[test]
fn declaration_universe_includes_empty_fns_and_modes() {
    let src = r#"
fn unused_helper() { }
locus Cell {
    params { v: Int = 0; }
    mode bulk() -> Int { return self.v; }
    fn poke(v: Int) { self.v = v; }
}
group helpers = { unused_helper };
main locus App {
    params { c: Cell = Cell { }; }
    run() { self.c.poke(2); println(self.c.bulk()); }
}
fn main() { App { }; }
"#;
    let m = derive(src);
    let e = &m.entities;
    let helper = e
        .functions
        .iter()
        .find(|f| f.name == "unused_helper")
        .expect("an EMPTY free fn is still an entity");
    assert_eq!(helper.kind, hale_model::FunctionKind::Free);
    // …and its group membership resolved.
    assert!(m.relations.group_members.iter().any(|gm| matches!(
        gm.member,
        EntityRef::Function(id)
            if e.functions[id.index()].name == "unused_helper"
    )));
    let bulk = e
        .functions
        .iter()
        .find(|f| f.name == "Cell::bulk")
        .expect("mode present");
    assert_eq!(
        bulk.kind,
        hale_model::FunctionKind::Mode,
        "a mode is a Mode, not a Hook"
    );
}

/// P1 (round 6): "inside a loop" and "unbounded" are SEPARATE
/// facts, at both grains the model records.
///
/// Direct grain: the summarizer proves `while i < 3` bounded, so a
/// call inside it is (in_loop, !unbounded) — the exact cell the old
/// single-boolean lattice destroyed — while `while true` is
/// (in_loop, unbounded).
///
/// Contraction grain: the Router-dispatch path re-emerges at the
/// user handler THROUGH the router's own entry loop, so the
/// contracted edge is honestly (true, true) even when the user call
/// site has no loop — the flags reflect the whole path, joined with
/// the two-component lattice (revisit-on-strengthen, so results
/// cannot depend on traversal order). No current stdlib surface
/// re-emerges at user code through a PROVEN-bounded interior path;
/// when one exists, pin the (true, false) contraction cell here.
#[test]
fn loop_and_unbounded_stay_separate_facts() {
    let src = r#"
fn leaf(v: Int) -> Int { return v + 1; }
fn bounded_caller() -> Int {
    let mut acc = 0;
    let mut i = 0;
    while i < 3 {
        acc = acc + leaf(i);
        i = i + 1;
    }
    return acc;
}
fn unbounded_caller() -> Int {
    let mut acc = 0;
    while true {
        acc = acc + leaf(acc);
        if acc > 10 { return acc; }
    }
    return acc;
}
fn main() { println(bounded_caller() + unbounded_caller()); }
"#;
    let m = derive(src);
    let e = &m.entities;
    let fname =
        |id: hale_model::FunctionId| e.functions[id.index()].name.clone();
    let flags = |from: &str| -> (bool, bool) {
        let c = m
            .relations
            .calls
            .iter()
            .find(|c| fname(c.from) == from && fname(c.to) == "leaf")
            .expect("edge exists");
        (c.in_loop, c.unbounded)
    };
    assert_eq!(
        flags("bounded_caller"),
        (true, false),
        "a PROVEN-bounded loop is looped but NOT unbounded — the \
         cell a single-boolean lattice destroys"
    );
    assert_eq!(flags("unbounded_caller"), (true, true));
}

#[test]
fn stdlib_contraction_reflects_the_whole_path() {
    let src = r#"
type Req { n: Int = 0; }
locus Fwd {
    params { n: Int = 0; }
    fn handle(ctx: std::http::Context) -> std::http::Response {
        return std::http::Response { status: 200, body: "ok" };
    }
}
locus Oms {
    params { n: Int = 0; }
    fn kick(i: Int) {
        let r = std::http::Router { };
        r.add("GET", "/fwd", Fwd { });
        let resp = r.dispatch(std::http::Request {
            method: "GET", path: "/fwd", body: ""
        });
        self.n = resp.status;
    }
}
main locus App {
    params { o: Oms = Oms { }; }
    run() { self.o.kick(1); }
}
fn main() { App { }; }
"#;
    let m = derive(src);
    let e = &m.entities;
    let fname =
        |id: hale_model::FunctionId| e.functions[id.index()].name.clone();
    let via = m
        .relations
        .calls
        .iter()
        .find(|c| {
            c.dispatch == DispatchKind::ViaStdlib
                && fname(c.to) == "Fwd::handle"
        })
        .expect("the through-stdlib edge to the handler exists");
    // The router's entry walk is a loop the path crosses — the
    // contracted edge carries the path's truth even though the user
    // call site has none.
    assert!(via.in_loop, "path crosses the router's entry loop");
}

// -----------------------------------------------------------------
// Review round 7 — declared ends, unanalyzed bodies, the legacy
// bridge, canonical identity.
// -----------------------------------------------------------------

/// P1 (round 7): a locus that DECLARES a publisher end but never
/// sends still publishes in the endpoint sense — the grain
/// `require publishes(...)` quantifies over.
#[test]
fn declared_publisher_ends_survive_without_sends() {
    let src = r#"
type T { n: Int = 0; }
topic Orders { payload: T; subject: "orders.wire"; }
locus Gateway {
    params { n: Int = 0; }
    bus { publish Orders; }
    fn idle(v: Int) -> Int { return v; }
}
locus Sub {
    params { seen: Int = 0; }
    bus { subscribe Orders as on_o; }
    fn on_o(t: T) { self.seen = self.seen + 1; }
}
main locus App {
    params { g: Gateway = Gateway { }; s: Sub = Sub { }; }
    run() { println(self.g.idle(1)); }
}
fn main() { App { }; }
"#;
    let m = derive(src);
    let e = &m.entities;
    // No send site exists…
    assert!(m.relations.publishes.is_empty(), "no send expressions");
    // …but the declared end does, with the topic's contract.
    let end = m
        .relations
        .declares_publish
        .iter()
        .find(|d| e.loci[d.locus.index()].name == "Gateway")
        .expect("Gateway's declared publisher end exists");
    assert!(end.declared_topic.is_some());
    assert_eq!(
        e.subjects[end.subject.index()].pattern, "orders.wire",
        "declared end carries the wire subject"
    );
}

/// P1 (round 7): module-scoped and on_failure bodies the behavior
/// analysis never walked HOLE OUT — the entities exist, their
/// behavior is typed-unknown, and the capabilities go false.
#[test]
fn unanalyzed_bodies_hole_out() {
    let src = r#"
type T { n: Int = 0; }
topic Evt { payload: T; subject: "evt"; }
module hidden {
    fn sneaky(f: fn (Int) -> Int, v: Int) -> Int { return f(v); }
}
locus Child {
    params { n: Int = 0; }
    fn poke(v: Int) { self.n = v; }
}
locus Parent {
    params { c: Child = Child { }; cb: fn (Int) -> Int = fallback_fn; }
    bus { publish Evt; }
    on_failure(c: Child, err: ClosureViolation) {
        Evt <- T { n: 1 };
        let v = self.cb(1);
        if v > 0 { restart (c); }
    }
}
fn fallback_fn(v: Int) -> Int { return v; }
main locus App {
    params { p: Parent = Parent { }; }
    run() { println(1); }
}
fn main() { App { }; }
"#;
    let m = derive(src);
    let e = &m.entities;
    // The module fn and the failure handler EXIST as entities…
    assert!(e.functions.iter().any(|f| f.name == "sneaky"));
    let handler = e
        .functions
        .iter()
        .find(|f| f.name.contains("on_failure"))
        .expect("failure handler is an executable entity");
    assert_eq!(handler.kind, hale_model::FunctionKind::Hook);
    // …and both hole out as UnanalyzedBody…
    let unanalyzed: Vec<String> = m
        .holes
        .iter()
        .filter(|h| h.kind == HoleKind::UnanalyzedBody)
        .filter_map(|h| match h.at {
            EntityRef::Function(id) => {
                Some(e.functions[id.index()].name.clone())
            }
            _ => None,
        })
        .collect();
    assert!(
        unanalyzed.iter().any(|n| n == "sneaky"),
        "module body holes out: {:?}",
        unanalyzed
    );
    assert!(
        unanalyzed.iter().any(|n| n.contains("on_failure")),
        "failure body holes out: {:?}",
        unanalyzed
    );
    // …so the capabilities cannot lie.
    assert!(!m.capabilities.exact_calls);
    assert!(!m.capabilities.exact_bus_endpoints);
    assert!(!m.capabilities.exact_effects);
}

/// P1 (round 7): canonical identity is importer-independent — the
/// same declaration imported under two aliases is ONE identity with
/// two spellings only at the display layer.
#[test]
fn canonical_identity_is_alias_independent() {
    let lib = r#"
locus __lib_x_kv_Store {
    params { n: Int = 0; }
    fn get(k: Int) -> Int { return self.n + k; }
}
"#;
    let main_src = r#"
main locus App {
    params { s: __lib_x_kv_Store = __lib_x_kv_Store { }; }
    run() { println(self.s.get(1)); }
}
fn main() { App { }; }
"#;
    let via_p = xseed_bundle(
        main_src,
        lib,
        &[(&["p", "Store"], "__lib_x_kv_Store")],
    );
    let via_db = xseed_bundle(
        main_src,
        lib,
        &[(&["db", "Store"], "__lib_x_kv_Store")],
    );
    let canon = |m: &hale_model::ApplicationModel| -> Vec<String> {
        m.entities.loci.iter().map(|l| l.name.clone()).collect()
    };
    assert_eq!(
        canon(&via_p),
        canon(&via_db),
        "alias choice must not change canonical identity"
    );
    let disp = |m: &hale_model::ApplicationModel| -> Vec<String> {
        m.entities.loci.iter().map(|l| l.display.clone()).collect()
    };
    assert_ne!(
        disp(&via_p),
        disp(&via_db),
        "…only the display spelling differs"
    );
}

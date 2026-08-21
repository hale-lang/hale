//! GH #476 Change 8 — the derived dispatch plan.
//!
//! `DispatchPlan::derive(&ApplicationModel)` is where the lowering
//! decision lives: gate facts × arrangement → one flavor per
//! subject, plus #464's stage-0 survey field (`same_domain`). The
//! obligations pinned here:
//!
//!   1. **Wire grain.** Plan subjects are wire subjects, not topic
//!      decl names — the identity codegen, the runtime, and the
//!      artifact's routes all use. A plan keyed the other way could
//!      not be compared to codegen's at all.
//!   2. **The ladder.** direct ⇒ static_direct, eligible-only ⇒
//!      static_bucket, otherwise dynamic with the gate's reason.
//!   3. **Conservatism.** A transport-bound subject is dynamic (the
//!      adapter is a counterparty no static bucket knows about),
//!      and a subject whose loci are NOT in the arrangement forfeits
//!      `same_domain` rather than guessing.
//!   4. **Identity.** The digest moves when the plan moves.

use std::collections::BTreeMap;

use hale_model::dispatch_plan::{DispatchFlavor, DispatchPlan};
use hale_types::model_builder::derive_application_model;
use hale_types::Bundle;

fn plan_of(src: &str) -> DispatchPlan {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut programs = BTreeMap::new();
    programs.insert("app.hl".to_string(), &program);
    let bundle = Bundle::new(programs);
    let m = derive_application_model(&bundle);
    m.validate().expect("derived model is lawful");
    DispatchPlan::derive(&m)
}

fn model_of(src: &str) -> hale_model::ApplicationModel {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut programs = BTreeMap::new();
    programs.insert("app.hl".to_string(), &program);
    let bundle = Bundle::new(programs);
    let m = derive_application_model(&bundle);
    m.validate().expect("derived model is lawful");
    m
}

fn flavor(p: &DispatchPlan, subject: &str) -> DispatchFlavor {
    p.subjects
        .iter()
        .find(|s| s.subject == subject)
        .unwrap_or_else(|| {
            panic!(
                "no plan row for `{}` — rows: {:?}",
                subject,
                p.subjects.iter().map(|s| &s.subject).collect::<Vec<_>>()
            )
        })
        .flavor
}

/// A topic-addressed dispatch plans at WIRE grain, and a
/// single-subscriber closed-world program reaches the direct tier.
#[test]
fn topic_subjects_plan_at_wire_grain() {
    let p = plan_of(
        r#"
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
"#,
    );
    assert_eq!(
        p.subjects.iter().map(|s| s.subject.as_str()).collect::<Vec<_>>(),
        vec!["evt"],
        "the plan is keyed by the WIRE subject, never the topic \
         decl name — codegen desugars topics away before it builds \
         the graph its own lowering uses"
    );
    assert_eq!(flavor(&p, "evt"), DispatchFlavor::StaticDirect);
    let row = &p.subjects[0];
    assert_eq!(row.publisher_domains, vec!["main".to_string()]);
    assert_eq!(row.subscriber_domains, vec!["main".to_string()]);
    assert!(row.same_domain, "both ends are arranged on main");
    assert_eq!(row.subscribers, vec![("Sub".into(), "on_e".into())]);
}

/// A subject whose subscriber lives on its own pool is NOT
/// same-domain — the stage-0 survey question #464 asks.
#[test]
fn a_pooled_subscriber_is_not_same_domain() {
    let p = plan_of(
        r#"
type T { n: Int = 0; }
topic Evt { payload: T; subject: "evt"; }
locus Worker {
    params { seen: Int = 0; }
    bus { subscribe Evt as on_e; }
    fn on_e(t: T) { self.seen = self.seen + 1; }
}
main locus App {
    params { w: Worker = Worker { }; }
    placement { w: cooperative(pool = io); }
    bus { publish Evt; }
    run() { Evt <- T { n: 1 }; }
}
fn main() { App { }; }
"#,
    );
    let row = p.subjects.iter().find(|s| s.subject == "evt").unwrap();
    assert_eq!(row.publisher_domains, vec!["main".to_string()]);
    assert_eq!(
        row.subscriber_domains,
        vec!["pool:io".to_string()],
        "a cooperative(pool = X) locus runs on X's worker, not main"
    );
    assert!(
        !row.same_domain,
        "publisher and subscriber sit in different thread domains"
    );
}

/// A transport-bound subject stays dynamic, and says why. This is
/// also the regression pin for the gate bug Change 8's plan
/// differential found: `bindings { Beat: unix(..) }` names the topic
/// DECL, but codegen's graph is keyed by the wire subject, so a
/// decl-name-only gate let a bound subject be devirtualized into a
/// static bucket the adapter is not part of.
#[test]
fn a_transport_bound_subject_stays_dynamic() {
    let p = plan_of(
        r#"
type Beat { n: Int = 0; }
topic Heartbeat { payload: Beat; subject: "demo.beat"; }
locus Watch {
    params { seen: Int = 0; }
    bus { subscribe Heartbeat as on_beat; }
    fn on_beat(b: Beat) { self.seen = self.seen + 1; }
}
main locus App {
    params { w: Watch = Watch { }; }
    bus { publish Heartbeat; }
    bindings { Heartbeat: unix("/tmp/hale-c8-plan.sock", role: listen); }
    run() { Heartbeat <- Beat { n: 1 }; }
}
fn main() { App { }; }
"#,
    );
    let row =
        p.subjects.iter().find(|s| s.subject == "demo.beat").unwrap();
    assert_eq!(row.flavor, DispatchFlavor::Dynamic);
    assert_eq!(row.ineligible_reason.as_deref(), Some("TransportBound"));
    let (same, total) = p.same_domain_queued();
    assert_eq!(total, 1);
    assert_eq!(
        same, 1,
        "queued (not direct) and both ends on main — exactly the \
         traffic #464's stage 0 counts"
    );
}

/// Loci born outside the arrangement have no known domain, so the
/// survey field is withheld rather than guessed — and the model
/// says so in its capability account.
#[test]
fn unarranged_loci_forfeit_the_survey_field() {
    let src = r#"
type T { n: Int = 0; }
locus Sub {
    params { seen: Int = 0; }
    bus { subscribe "evt" as on_e of type T; }
    fn on_e(t: T) { self.seen = self.seen + 1; }
}
locus Pub {
    bus { publish "evt" of type T; }
    birth() { "evt" <- T { n: 1 }; }
}
fn main() { Sub { }; Pub { }; }
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut programs = BTreeMap::new();
    programs.insert("app.hl".to_string(), &program);
    let bundle = Bundle::new(programs);
    let m = derive_application_model(&bundle);
    m.validate().expect("lawful");
    assert!(
        !m.capabilities.exact_placement && !m.capabilities.exact_ownership,
        "every instance here is born in `fn main` — the arrangement \
         names none of them, and the capability account must admit it"
    );
    let p = DispatchPlan::derive(&m);
    let row = p.subjects.iter().find(|s| s.subject == "evt").unwrap();
    assert!(
        row.publisher_domains.is_empty()
            && row.subscriber_domains.is_empty()
    );
    assert!(
        !row.same_domain,
        "an unknown domain is not evidence of a shared one"
    );
    assert_eq!(p.same_domain_queued(), (0, 1));
}

/// The digest is an identity, not a summary: moving any decision
/// moves it. Same plan, same digest.
#[test]
fn the_digest_tracks_the_plan() {
    const ONE_SUB: &str = r#"
type T { n: Int = 0; }
topic Evt { payload: T; subject: "evt"; }
locus A {
    params { seen: Int = 0; }
    bus { subscribe Evt as on_e; }
    fn on_e(t: T) { self.seen = self.seen + 1; }
}
main locus App {
    params { a: A = A { }; }
    bus { publish Evt; }
    run() { Evt <- T { n: 1 }; }
}
fn main() { App { }; }
"#;
    // Same program, second derivation: identical.
    assert_eq!(plan_of(ONE_SUB).digest(), plan_of(ONE_SUB).digest());
    // A second subscriber changes what the direct lowering bakes.
    let two_subs = ONE_SUB.replace(
        "params { a: A = A { }; }",
        "params { a: A = A { }; b: B = B { }; }",
    ) + r#"
locus B {
    params { seen: Int = 0; }
    bus { subscribe Evt as on_e; }
    fn on_e(t: T) { self.seen = self.seen + 1; }
}
"#;
    let two = plan_of(&two_subs);
    assert_ne!(
        plan_of(ONE_SUB).digest(),
        two.digest(),
        "a different subscriber set is a different lowering"
    );
    // The empty plan (the LOTUS_NO_BUS_DEVIRT control arm) is its
    // own identity — the CLI folds exactly this into the exec
    // digest when the flag forces all-dynamic lowering.
    assert_ne!(DispatchPlan::default().digest(), two.digest());
}

/// Review round 1, blocker 1: each replica carries its OWN index.
/// The runtime pins replica `i` to one core and a keyed subscriber
/// on a replicated field registers under `key == i`, so a model
/// that stored the fan-out COUNT in every row ([3, 3, 3] for
/// `replicas = 3`) names a population the process does not have.
#[test]
fn replicas_carry_contiguous_zero_based_indices() {
    let src = r#"
locus Worker {
    params { id: Int = 0; }
    fn tick() { self.id = self.id + 1; }
}
main locus App {
    params { workers: Worker = Worker { }; }
    placement { workers: pinned(cores = 0..3, replicas = 3); }
    run() { self.workers.tick(); }
}
fn main() { App { }; }
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut programs = BTreeMap::new();
    programs.insert("app.hl".to_string(), &program);
    let bundle = Bundle::new(programs);
    let m = derive_application_model(&bundle);
    m.validate().expect("lawful");

    let replicas: Vec<(String, Option<u32>)> = m
        .entities
        .locus_instances
        .iter()
        .filter(|i| i.path.contains('['))
        .map(|i| (i.path.clone(), i.replica))
        .collect();
    assert_eq!(
        replicas,
        vec![
            ("App.workers[0]".to_string(), Some(0)),
            ("App.workers[1]".to_string(), Some(1)),
            ("App.workers[2]".to_string(), Some(2)),
        ],
        "each replica carries its index, not the fan-out count"
    );
    // The root and any non-replicated instance claim no index.
    assert!(m
        .entities
        .locus_instances
        .iter()
        .filter(|i| !i.path.contains('['))
        .all(|i| i.replica.is_none()));
}

/// …and the law is enforced at the model, not merely produced by
/// the builder: the count-in-every-row shape is refused.
#[test]
fn a_non_contiguous_replica_set_is_not_a_model() {
    let src = r#"
locus Worker { params { id: Int = 0; } fn tick() { self.id = self.id + 1; } }
main locus App {
    params { workers: Worker = Worker { }; }
    placement { workers: pinned(cores = 0..3, replicas = 3); }
    run() { self.workers.tick(); }
}
fn main() { App { }; }
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut programs = BTreeMap::new();
    programs.insert("app.hl".to_string(), &program);
    let bundle = Bundle::new(programs);
    let mut m = derive_application_model(&bundle);
    // The pre-review shape: every replica stamped with the COUNT.
    for inst in m.entities.locus_instances.iter_mut() {
        if inst.replica.is_some() {
            inst.replica = Some(3);
        }
    }
    assert!(
        matches!(
            m.validate(),
            Err(hale_model::ModelError::ReplicaIndicesNotContiguous { .. })
        ),
        "validate accepted a replica set that is not 0..K"
    );
}

/// Review round 1, blocker 3: a locus can be BOTH arranged and
/// dynamically born. The arranged instance answers "main" for the
/// whole population, but the dynamic sibling is born inside a
/// pooled locus and runs on that pool's worker — so a plan that
/// trusted the arranged answer would report a same-domain
/// optimization opportunity about a process that does not have
/// one. A placement hole at the locus deletes the answer.
#[test]
fn a_partly_dynamic_population_is_not_same_domain() {
    let src = r#"
type T { n: Int = 0; }
topic Evt { payload: T; subject: "evt"; }
locus Sub {
    params { seen: Int = 0; }
    bus { subscribe Evt as on_e; }
    fn on_e(t: T) { self.seen = self.seen + 1; }
}
locus Spawner {
    params { n: Int = 0; }
    fn spawn() { Sub { }; }
}
main locus App {
    params { s: Sub = Sub { }; sp: Spawner = Spawner { }; }
    placement { sp: cooperative(pool = io); }
    bus { publish Evt; }
    run() { self.sp.spawn(); Evt <- T { n: 1 }; }
}
fn main() { App { }; }
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut programs = BTreeMap::new();
    programs.insert("app.hl".to_string(), &program);
    let bundle = Bundle::new(programs);
    let m = derive_application_model(&bundle);
    m.validate().expect("lawful");

    // Premise: the arrangement DOES name a `Sub` on main…
    assert!(
        m.entities
            .locus_instances
            .iter()
            .any(|i| i.path == "App.s"),
        "fixture premise: an arranged Sub exists"
    );
    // …and the model admits a second, unplaced one exists.
    assert!(
        !m.capabilities.exact_placement,
        "fixture premise: the dynamic birth is holed out"
    );

    let p = DispatchPlan::derive(&m);
    let row = p.subjects.iter().find(|s| s.subject == "evt").unwrap();
    assert!(
        row.subscriber_domains.is_empty(),
        "an incomplete population has no domain answer, not a \
         partial one: {:?}",
        row.subscriber_domains
    );
    assert!(
        !row.same_domain,
        "the plan reported same_domain over a population it \
         cannot fully place"
    );
    assert_eq!(p.same_domain_queued().0, 0);
}

/// Review round 1, blocker 2: an explicit `role: connect` binding
/// is a CONNECT binding in the model. The role was hardcoded to
/// Listen, so a publish-only binding — the shape whose role
/// codegen infers as Connect — was modeled as its opposite.
#[test]
fn a_connect_binding_is_modeled_as_connect() {
    let m = model_of(
        r#"
type Beat { n: Int = 0; }
topic Heartbeat { payload: Beat; subject: "demo.beat"; }
main locus App {
    bus { publish Heartbeat; }
    bindings { Heartbeat: unix("/tmp/hale-c8-connect.sock", role: connect); }
    run() { Heartbeat <- Beat { n: 1 }; }
}
fn main() { App { }; }
"#,
    );
    assert_eq!(m.entities.bindings.len(), 1);
    let b = &m.entities.bindings[0];
    assert_eq!(b.role, hale_model::BindingRole::Connect);
    assert_eq!(
        b.loss,
        hale_model::keys::BindingLossBehavior::WaitCapable,
        "the connect side is the publish side: a send failure marks \
         the entry lost and `or wait` parks through the reconnect \
         window"
    );
}

/// …and the INFERRED role follows the same rule the desugar uses:
/// subscribe-only is a listener, whose link loss is structural.
#[test]
fn an_inferred_listen_binding_is_modeled_as_listen() {
    let m = model_of(
        r#"
type Beat { n: Int = 0; }
topic Heartbeat { payload: Beat; subject: "demo.beat"; }
main locus App {
    params { seen: Int = 0; }
    bus { subscribe Heartbeat as on_beat; }
    bindings { Heartbeat: unix("/tmp/hale-c8-listen.sock"); }
    fn on_beat(b: Beat) { self.seen = self.seen + 1; }
    run() { }
}
fn main() { App { }; }
"#,
    );
    assert_eq!(m.entities.bindings.len(), 1);
    assert_eq!(m.entities.bindings[0].role, hale_model::BindingRole::Listen);
    assert_eq!(
        m.entities.bindings[0].loss,
        hale_model::keys::BindingLossBehavior::Fail
    );
}

/// Two bindings authored in reverse canonical order still produce a
/// MODEL: entity ids are assigned after the canonical sort, and the
/// `binds` relation points at the post-sort ids. Authoring order is
/// not a model property, and `validate` requires the sort.
#[test]
fn bindings_authored_out_of_order_still_validate() {
    let m = model_of(
        r#"
type A { n: Int = 0; }
type B { n: Int = 0; }
topic Zed { payload: A; subject: "z.sub"; }
topic Alpha { payload: B; subject: "a.sub"; }
main locus App {
    params { seen: Int = 0; }
    bus {
        subscribe Zed as on_z;
        subscribe Alpha as on_a;
    }
    bindings {
        Zed: unix("/tmp/hale-c8-z.sock");
        Alpha: unix("/tmp/hale-c8-a.sock");
    }
    fn on_z(a: A) { self.seen = self.seen + 1; }
    fn on_a(b: B) { self.seen = self.seen + 1; }
    run() { }
}
fn main() { App { }; }
"#,
    );
    // validate() ran inside model_of — the canonical-order law is
    // what used to fail here.
    let subjects: Vec<&str> = m
        .entities
        .bindings
        .iter()
        .map(|b| m.entities.subjects[b.subject.index()].pattern.as_str())
        .collect();
    assert_eq!(
        subjects,
        vec!["a.sub", "z.sub"],
        "the binding table is canonically sorted, not source-ordered"
    );
    // …and `binds` joins each topic to the binding for ITS subject.
    for row in &m.relations.binds {
        let topic_subject = m.entities.subjects
            [m.entities.topics[row.topic.index()].subject.index()]
        .pattern
        .clone();
        let binding_subject = m.entities.subjects[m.entities.bindings
            [row.binding.index()]
        .subject
        .index()]
        .pattern
        .clone();
        assert_eq!(
            topic_subject, binding_subject,
            "a binds row points at the wrong post-sort BindingId"
        );
    }
    assert_eq!(m.relations.binds.len(), 2);
}

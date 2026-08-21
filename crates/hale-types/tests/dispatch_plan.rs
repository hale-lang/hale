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

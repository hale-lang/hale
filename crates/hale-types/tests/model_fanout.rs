//! GH #476 Change 5h round 2 — `@budget(fanout = N)` counts
//! subscriber DELIVERIES.
//!
//! The first implementation resolved a subject and counted covering
//! rows in `relations.subscribes`. A `Subscribe` row is
//! DECLARATION-grained: one subscription declared by one handler.
//! Three arranged replicas of one `Sink` are three runtime
//! registrations and three deliveries, and the count said one — so
//! `@budget(fanout = 1)` certified a publish that dispatched three
//! cells. Keyed subscriptions failed the other way: address
//! coverage charged two mutually-exclusive filters that one exact
//! publish key can never both reach.

use std::collections::BTreeMap;

use hale_types::symbol::SourceFile;
use hale_types::Bundle;

fn diags(src: &str) -> Vec<String> {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut programs = BTreeMap::new();
    programs.insert("app.hl".to_string(), &program);
    let mut b = Bundle::new(programs);
    b.sources = vec![SourceFile {
        id: 0,
        path: "app.hl".to_string(),
        digest: "0".to_string(),
        base: 0,
        len: src.len() as u32,
    }];
    hale_types::check_bundle_opts(&b, false)
        .into_iter()
        .map(|d| d.message)
        .collect()
}

#[test]
fn replicas_of_one_subscription_are_separate_deliveries() {
    // ONE `subscribe` declaration, THREE arranged instances. The
    // declaration count says 1; the delivery count says 3.
    let src = r#"
type T { n: Int = 0; }
topic Ticks { payload: T; subject: "ticks"; }
locus Sink {
    bus { subscribe Ticks as on_tick; }
    params { n: Int = 0; }
    fn on_tick(t: T) { self.n = t.n; }
}
main locus App {
    params {
        a: Sink = Sink { };
        b: Sink = Sink { };
        c: Sink = Sink { };
    }
    bus { publish Ticks; }
    @budget(fanout = 1)
    fn beat() { Ticks <- T { n: 1 }; }
    run() { self.beat(); }
}
fn main() { App { }; }
"#;
    let ds = diags(src);
    assert!(
        ds.iter().any(|m| m.contains("budget exceeded")
            && m.contains("fanout")),
        "three instances of one declaration are three deliveries: \
         {:?}",
        ds
    );
}

#[test]
fn one_instance_of_one_subscription_fits_a_budget_of_one() {
    // The premise of the control above: the count is not simply
    // inflated.
    let src = r#"
type T { n: Int = 0; }
topic Ticks { payload: T; subject: "ticks"; }
locus Sink {
    bus { subscribe Ticks as on_tick; }
    params { n: Int = 0; }
    fn on_tick(t: T) { self.n = t.n; }
}
main locus App {
    params { a: Sink = Sink { }; }
    bus { publish Ticks; }
    @budget(fanout = 1)
    fn beat() { Ticks <- T { n: 1 }; }
    run() { self.beat(); }
}
fn main() { App { }; }
"#;
    let ds = diags(src);
    assert!(
        !ds.iter().any(|m| m.contains("budget exceeded")),
        "one registration, one delivery: {:?}",
        ds
    );
}

#[test]
fn a_computed_subject_makes_fanout_unbounded() {
    // A publish whose subject the model cannot name may address any
    // endpoint — including a high-fanout one.
    let src = r#"
type T { n: Int = 0; }
topic Ticks { payload: T; subject: "ticks"; }
locus Sink {
    bus { subscribe Ticks as on_tick; }
    params { n: Int = 0; }
    fn on_tick(t: T) { self.n = t.n; }
}
fn pick(n: Int) -> String { if n > 0 { return "ticks"; } return "tocks"; }
main locus App {
    params { a: Sink = Sink { }; }
    bus { publish Ticks; publish "t.**" of type T; }
    @budget(fanout = 1)
    fn beat(n: Int) {
        Ticks <- T { n: n };
        pick(n) <- T { n: n };
    }
    run() { self.beat(1); }
}
fn main() { App { }; }
"#;
    let ds = diags(src);
    assert!(
        ds.iter().any(|m| m.contains("budget exceeded")
            && m.contains("fanout")),
        "an unnameable subject is not a fanout of one: {:?}",
        ds
    );
}

/// Review pin (round 3): fan-out is TRANSITIVE amplification.
///
/// `App::fire` publishes A; `Relay::on_a` receives it and publishes
/// B; three `Sink` instances receive B. That is four deliveries
/// caused by one invocation. The engine's ordinary call graph never
/// enters a handler through the bus, so the closure has to carry the
/// whole delivery chain — asking only about the A site got 1 and let
/// `@budget(fanout = 1)` hold falsely.
#[test]
fn fanout_follows_the_bus_to_the_second_hop() {
    let src = r#"
type T { n: Int = 0; }
topic A { payload: T; subject: "a"; }
topic B { payload: T; subject: "b"; }
locus Relay {
    bus { subscribe A as on_a; publish B; }
    params { n: Int = 0; }
    fn on_a(t: T) { B <- T { n: t.n }; }
}
locus Sink {
    bus { subscribe B as on_b; }
    params { n: Int = 0; }
    fn on_b(t: T) { self.n = t.n; }
}
main locus App {
    params {
        r: Relay = Relay { };
        s1: Sink = Sink { }; s2: Sink = Sink { }; s3: Sink = Sink { };
    }
    bus { publish A; }
    @budget(fanout = 3)
    fn fire() { A <- T { n: 1 }; }
    run() { self.fire(); }
}
fn main() { App { }; }
"#;
    let ds = diags(src);
    assert!(
        ds.iter().any(|m| m.contains("budget exceeded")
            && m.contains("fanout")
            && m.contains(" 4 ")),
        "one delivery to Relay plus three to Sink is four: {:?}",
        ds
    );
}

/// …and the same program at the true bound certifies.
#[test]
fn the_transitive_bound_is_reachable() {
    let src = r#"
type T { n: Int = 0; }
topic A { payload: T; subject: "a"; }
topic B { payload: T; subject: "b"; }
locus Relay {
    bus { subscribe A as on_a; publish B; }
    params { n: Int = 0; }
    fn on_a(t: T) { B <- T { n: t.n }; }
}
locus Sink {
    bus { subscribe B as on_b; }
    params { n: Int = 0; }
    fn on_b(t: T) { self.n = t.n; }
}
main locus App {
    params {
        r: Relay = Relay { };
        s1: Sink = Sink { }; s2: Sink = Sink { }; s3: Sink = Sink { };
    }
    bus { publish A; }
    @budget(fanout = 4)
    fn fire() { A <- T { n: 1 }; }
    run() { self.fire(); }
}
fn main() { App { }; }
"#;
    let ds = diags(src);
    assert!(
        !ds.iter().any(|m| m.contains("budget exceeded")),
        "{:?}",
        ds
    );
}

/// Review pin (round 3): population completeness is scoped to the
/// loci ON this delivery closure.
///
/// It used to be one global question — "is there an OWNS or
/// CARDINALITY hole anywhere?" — so a single unrelated dynamically
/// born locus made every fan-out in the program unbounded.
#[test]
fn an_unrelated_dynamic_birth_does_not_poison_a_local_fanout() {
    let src = r#"
type T { n: Int = 0; }
topic Ticks { payload: T; subject: "ticks"; }
locus Ephemeral {
    params { n: Int = 0; }
    fn work() -> Int { return self.n; }
}
locus Sink {
    bus { subscribe Ticks as on_tick; }
    params { n: Int = 0; }
    fn on_tick(t: T) { self.n = t.n; }
}
main locus App {
    params { s: Sink = Sink { }; }
    bus { publish Ticks; }
    @budget(fanout = 1)
    fn beat() { Ticks <- T { n: 1 }; }
    run() {
        let e = Ephemeral { };
        self.beat();
    }
}
fn main() { App { }; }
"#;
    let ds = diags(src);
    assert!(
        !ds.iter().any(|m| m.contains("budget exceeded")
            && m.contains("fanout")),
        "`Ephemeral` subscribes to nothing on this closure: {:?}",
        ds
    );
}

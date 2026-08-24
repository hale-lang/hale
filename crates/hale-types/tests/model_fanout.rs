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

/// Review pin (round 4): fan-out counts EXECUTIONS, not a set.
///
/// Three `Relay` instances each receive A and each publish B to one
/// `Sink`: three deliveries to Relay plus three executions of
/// `on_a`, one delivery each — six. Deduplicating the delivery graph
/// as a set followed `on_a` once and counted four, so
/// `@budget(fanout = 4)` held for an invocation causing six.
#[test]
fn handler_multiplicity_multiplies_downstream_deliveries() {
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
        r1: Relay = Relay { }; r2: Relay = Relay { };
        r3: Relay = Relay { }; s: Sink = Sink { };
    }
    bus { publish A; }
    @budget(fanout = 5)
    fn fire() { A <- T { n: 1 }; }
    run() { self.fire(); }
}
fn main() { App { }; }
"#;
    let ds = diags(src);
    assert!(
        ds.iter().any(|m| m.contains("budget exceeded")
            && m.contains("fanout")
            && m.contains(" 6 ")),
        "three relays each publishing once is six deliveries: {:?}",
        ds
    );
}

/// A handler that calls a publishing HELPER amplifies too — onward
/// publishes are not limited to rows whose function is the handler.
#[test]
fn a_publishing_helper_counts_toward_the_handlers_fanout() {
    let src = r#"
type T { n: Int = 0; }
topic A { payload: T; subject: "a"; }
topic B { payload: T; subject: "b"; }
locus Relay {
    bus { subscribe A as on_a; publish B; }
    params { n: Int = 0; }
    // The publish sits in a HELPER method, not in the handler.
    fn shout(n: Int) { B <- T { n: n }; }
    fn on_a(t: T) { self.shout(t.n); }
}
locus Sink {
    bus { subscribe B as on_b; }
    params { n: Int = 0; }
    fn on_b(t: T) { self.n = t.n; }
}
main locus App {
    params { r: Relay = Relay { }; s: Sink = Sink { }; }
    bus { publish A; }
    @budget(fanout = 1)
    fn fire() { A <- T { n: 1 }; }
    run() { self.fire(); }
}
fn main() { App { }; }
"#;
    let ds = diags(src);
    assert!(
        ds.iter().any(|m| m.contains("budget exceeded")
            && m.contains("fanout")),
        "the helper's publish is caused by the same invocation: {:?}",
        ds
    );
}

/// A loop-nested onward publish repeats per invocation.
#[test]
fn a_looped_onward_publish_is_unbounded() {
    let src = r#"
type T { n: Int = 0; }
topic A { payload: T; subject: "a"; }
topic B { payload: T; subject: "b"; }
locus Relay {
    bus { subscribe A as on_a; publish B; }
    params { n: Int = 0; }
    fn on_a(t: T) {
        let mut i = 0;
        while i < 4 { B <- T { n: i }; i = i + 1; }
    }
}
locus Sink {
    bus { subscribe B as on_b; }
    params { n: Int = 0; }
    fn on_b(t: T) { self.n = t.n; }
}
main locus App {
    params { r: Relay = Relay { }; s: Sink = Sink { }; }
    bus { publish A; }
    @budget(fanout = 99)
    fn fire() { A <- T { n: 1 }; }
    run() { self.fire(); }
}
fn main() { App { }; }
"#;
    let ds = diags(src);
    assert!(
        ds.iter().any(|m| m.contains("budget exceeded")
            && m.contains("unbounded")),
        "a loop-nested contributor has no per-call bound: {:?}",
        ds
    );
}

/// A productive bus CYCLE is unbounded, not settled.
#[test]
fn a_republish_cycle_is_unbounded() {
    let src = r#"
type T { n: Int = 0; }
topic A { payload: T; subject: "a"; }
topic B { payload: T; subject: "b"; }
locus Ping {
    bus { subscribe A as on_a; publish B; }
    params { n: Int = 0; }
    fn on_a(t: T) { B <- T { n: t.n }; }
}
locus Pong {
    bus { subscribe B as on_b; publish A; }
    params { n: Int = 0; }
    fn on_b(t: T) { A <- T { n: t.n }; }
}
main locus App {
    params { p: Ping = Ping { }; q: Pong = Pong { }; }
    bus { publish A; }
    @budget(fanout = 99)
    fn fire() { A <- T { n: 1 }; }
    run() { self.fire(); }
}
fn main() { App { }; }
"#;
    let ds = diags(src);
    assert!(
        ds.iter().any(|m| m.contains("budget exceeded")
            && m.contains("unbounded")),
        "a cycle amplifies without bound: {:?}",
        ds
    );
}

/// Review pin (round 5): the handler's CALL TREE carries execution
/// counts, not reachability.
///
/// Model call rows are site-grained precisely because two calls are
/// two executions. Collapsing the tree to a set counted a helper
/// called twice as one publish, so one invocation causing three
/// deliveries measured two.
#[test]
fn a_helper_called_twice_publishes_twice() {
    let src = r#"
type T { n: Int = 0; }
topic A { payload: T; subject: "a"; }
topic B { payload: T; subject: "b"; }
locus Relay {
    bus { subscribe A as on_a; publish B; }
    params { n: Int = 0; }
    fn shout(n: Int) { B <- T { n: n }; }
    fn on_a(t: T) { self.shout(t.n); self.shout(t.n); }
}
locus Sink {
    bus { subscribe B as on_b; }
    params { n: Int = 0; }
    fn on_b(t: T) { self.n = t.n; }
}
main locus App {
    params { r: Relay = Relay { }; s: Sink = Sink { }; }
    bus { publish A; }
    @budget(fanout = 2)
    fn fire() { A <- T { n: 1 }; }
    run() { self.fire(); }
}
fn main() { App { }; }
"#;
    let ds = diags(src);
    assert!(
        ds.iter().any(|m| m.contains("budget exceeded")
            && m.contains("fanout")
            && m.contains(" 3 ")),
        "one delivery to Relay plus two helper publishes is three: \
         {:?}",
        ds
    );
}

/// A recursive publishing helper has no per-call bound.
#[test]
fn a_recursive_publishing_helper_is_unbounded() {
    let src = r#"
type T { n: Int = 0; }
topic A { payload: T; subject: "a"; }
topic B { payload: T; subject: "b"; }
locus Relay {
    bus { subscribe A as on_a; publish B; }
    params { n: Int = 0; }
    fn shout(n: Int) {
        B <- T { n: n };
        if n > 0 { self.shout(n - 1); }
    }
    fn on_a(t: T) { self.shout(t.n); }
}
locus Sink {
    bus { subscribe B as on_b; }
    params { n: Int = 0; }
    fn on_b(t: T) { self.n = t.n; }
}
main locus App {
    params { r: Relay = Relay { }; s: Sink = Sink { }; }
    bus { publish A; }
    @budget(fanout = 99)
    fn fire() { A <- T { n: 1 }; }
    run() { self.fire(); }
}
fn main() { App { }; }
"#;
    let ds = diags(src);
    assert!(
        ds.iter().any(|m| m.contains("budget exceeded")
            && m.contains("unbounded")),
        "recursion amplifies without a static bound: {:?}",
        ds
    );
}

/// Review pin (round 5): ONE message carries ONE key, so replica
/// filters do not all receive it.
///
/// `where key == replica` is satisfied by the instance whose index
/// EQUALS the message key. Summing every replica the key might
/// select counted the union of possible recipients rather than the
/// recipients of one message, so a three-replica sink read as
/// fan-out 3 for a message that reaches exactly one cell.
#[test]
fn replica_keyed_delivery_reaches_one_cell_per_message() {
    let src = r#"
type T { shard: Int = 0; }
topic Ticks { payload: T; subject: "ticks"; keyed_by shard; }
locus Sink {
    bus { subscribe Ticks as on_tick where key == replica; }
    params { n: Int = 0; }
    fn on_tick(t: T) { self.n = t.shard; }
}
main locus App {
    params { sinks: Sink = Sink { }; }
    placement { sinks: pinned(replicas = 3); }
    bus { publish Ticks; }
    @budget(fanout = 1)
    fn fire() { Ticks <- T { shard: 1 }; }
    run() { self.fire(); }
}
fn main() { App { }; }
"#;
    let ds = diags(src);
    assert!(
        !ds.iter().any(|m| m.contains("budget exceeded")
            && m.contains("fanout")),
        "a keyed message reaches the replica its key selects, not \
         all three: {:?}",
        ds
    );
}

/// A declared but never instantiated subscriber receives nothing —
/// an exact zero, not an unknown.
#[test]
fn an_uninstantiated_subscriber_contributes_no_deliveries() {
    let src = r#"
type T { n: Int = 0; }
topic Ticks { payload: T; subject: "ticks"; }
locus Unused {
    bus { subscribe Ticks as on_tick; }
    params { n: Int = 0; }
    fn on_tick(t: T) { self.n = t.n; }
}
main locus App {
    params { n: Int = 0; }
    bus { publish Ticks; }
    @budget(fanout = 0)
    fn fire() { Ticks <- T { n: 1 }; }
    run() { self.fire(); }
}
fn main() { App { }; }
"#;
    let ds = diags(src);
    assert!(
        !ds.iter().any(|m| m.contains("budget exceeded")
            && m.contains("fanout")),
        "no instance, no delivery: {:?}",
        ds
    );
}

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

/// Review pin (round 6): the per-key SCENARIO survives into the
/// downstream walk.
///
/// The immediate count chose one key while the downstream traversal
/// followed every handler reachable under ANY key. One message
/// carries one key, so `where key == replica` selects one relay —
/// and only that relay's onward publish happens.
#[test]
fn a_keyed_hop_carries_its_key_choice_downstream() {
    let src = r#"
type T { shard: Int = 0; }
topic A { payload: T; subject: "a"; keyed_by shard; }
topic B { payload: T; subject: "b"; }
locus Relay {
    bus { subscribe A as on_a where key == replica; publish B; }
    params { n: Int = 0; }
    fn on_a(t: T) { B <- T { shard: t.shard }; }
}
locus Sink {
    bus { subscribe B as on_b; }
    params { n: Int = 0; }
    fn on_b(t: T) { self.n = t.shard; }
}
main locus App {
    params { relays: Relay = Relay { }; sink: Sink = Sink { }; }
    placement { relays: pinned(replicas = 3); }
    bus { publish A; }
    @budget(fanout = 2)
    fn fire() { A <- T { shard: 1 }; }
    run() { self.fire(); }
}
fn main() { App { }; }
"#;
    let ds = diags(src);
    assert!(
        !ds.iter().any(|m| m.contains("budget exceeded")
            && m.contains("fanout")),
        "one relay receives and one relay republishes: {:?}",
        ds
    );
}

/// …and the same program does NOT fit in one.
#[test]
fn the_keyed_hop_still_costs_its_second_delivery() {
    let src = r#"
type T { shard: Int = 0; }
topic A { payload: T; subject: "a"; keyed_by shard; }
topic B { payload: T; subject: "b"; }
locus Relay {
    bus { subscribe A as on_a where key == replica; publish B; }
    params { n: Int = 0; }
    fn on_a(t: T) { B <- T { shard: t.shard }; }
}
locus Sink {
    bus { subscribe B as on_b; }
    params { n: Int = 0; }
    fn on_b(t: T) { self.n = t.shard; }
}
main locus App {
    params { relays: Relay = Relay { }; sink: Sink = Sink { }; }
    placement { relays: pinned(replicas = 3); }
    bus { publish A; }
    @budget(fanout = 1)
    fn fire() { A <- T { shard: 1 }; }
    run() { self.fire(); }
}
fn main() { App { }; }
"#;
    let ds = diags(src);
    assert!(
        ds.iter().any(|m| m.contains("budget exceeded")
            && m.contains("fanout")),
        "the relay's own republish is a second delivery: {:?}",
        ds
    );
}

/// Review pin (round 6): interface alternatives are a CHOICE, and
/// the choice is made over whole contributions.
///
/// A pointwise max over publish sites keeps one entry per site
/// across alternatives — a union, not a choice — so two conformers
/// publishing different subjects both counted. One dispatch runs
/// one conformer.
#[test]
fn interface_alternatives_take_the_max_of_whole_contributions() {
    let src = r#"
type T { n: Int = 0; }
topic A { payload: T; subject: "a"; }
topic X { payload: T; subject: "x"; }
topic Y { payload: T; subject: "y"; }
interface Runner { fn go(v: Int) -> Int; }
locus Left {
    bus { publish X; }
    params { n: Int = 0; }
    fn go(v: Int) -> Int { X <- T { n: v }; return v; }
}
locus Right {
    bus { publish Y; }
    params { n: Int = 0; }
    fn go(v: Int) -> Int { Y <- T { n: v }; return v; }
}
locus SinkX {
    bus { subscribe X as on_x; }
    params { n: Int = 0; }
    fn on_x(t: T) { self.n = t.n; }
}
locus SinkY {
    bus { subscribe Y as on_y; }
    params { n: Int = 0; }
    fn on_y(t: T) { self.n = t.n; }
}
locus Handler {
    bus { subscribe A as on_a; }
    params { r: Runner = Left { }; }
    fn on_a(t: T) { self.r.go(t.n); }
}
main locus App {
    params {
        h: Handler = Handler { };
        sx: SinkX = SinkX { }; sy: SinkY = SinkY { };
    }
    bus { publish A; }
    @budget(fanout = 2)
    fn fire() { A <- T { n: 1 }; }
    run() { self.fire(); }
}
fn main() { App { }; }
"#;
    let ds = diags(src);
    assert!(
        !ds.iter().any(|m| m.contains("budget exceeded")
            && m.contains("fanout")),
        "one dispatch runs one conformer: {:?}",
        ds
    );
}

// ---------------------------------------------------------------
// Through-stdlib entries. The absorption account is the AUTHORED
// site grain — one `StdlibAbsorption` row per entry site, with
// `entry_group` marking alternatives of one dispatch — while
// `relations.calls`' `ViaStdlib` rows are the CONTRACTED endpoint
// pair and collapse several entries into one. Counting the
// contraction lost a second entry's execution; counting both would
// double. These pin the account the traversal actually reads.
//
// Built by deriving a real model and shaping its absorption rows,
// the established pattern for absorption controls — no stdlib
// function takes a user callback, so the shape is not writable in
// source today.
// ---------------------------------------------------------------

fn model_and_ids(
    src: &str,
) -> (hale_model::ApplicationModel, hale_types::Bundle<'static>) {
    let program: &'static hale_syntax::ast::Program = Box::leak(
        Box::new(hale_syntax::parse_source(src).expect("parse")),
    );
    let src: &'static str = Box::leak(src.to_string().into_boxed_str());
    let mut programs = BTreeMap::new();
    programs.insert("app.hl".to_string(), program);
    let mut b = hale_types::Bundle::new(programs);
    b.sources = vec![SourceFile {
        id: 0,
        path: "app.hl".to_string(),
        digest: "0".to_string(),
        base: 0,
        len: src.len() as u32,
    }];
    let m = hale_types::model_builder::derive_application_model(&b);
    (m, b)
}

fn fn_id(
    m: &hale_model::ApplicationModel,
    name: &str,
) -> hale_model::FunctionId {
    hale_model::FunctionId(
        m.entities
            .functions
            .iter()
            .position(|f| f.name == name)
            .unwrap_or_else(|| panic!("no fn `{}`", name))
            as u32,
    )
}

const HOP: &str = r#"
type T { n: Int = 0; }
topic A { payload: T; subject: "a"; }
topic B { payload: T; subject: "b"; }
locus Emitter {
    bus { publish B; }
    params { n: Int = 0; }
    fn emit(v: Int) { B <- T { n: v }; }
}
locus Handler {
    bus { subscribe A as on_a; }
    params { e: Emitter = Emitter { }; }
    fn on_a(t: T) { self.e.emit(t.n); }
}
locus Sink {
    bus { subscribe B as on_b; }
    params { n: Int = 0; }
    fn on_b(t: T) { self.n = t.n; }
}
main locus App {
    params {
        h: Handler = Handler { }; s: Sink = Sink { };
        e: Emitter = Emitter { };
    }
    bus { publish A; }
    fn fire() { A <- T { n: 1 }; }
    run() { self.fire(); }
}
fn main() { App { }; }
"#;

/// Two bounded stdlib ENTRY SITES re-emerging at one publishing
/// callback are two executions of it.
#[test]
fn two_stdlib_entries_to_one_callback_count_twice() {
    let (mut m, _b) = model_and_ids(HOP);
    let handler = fn_id(&m, "Handler::on_a");
    let emitter = fn_id(&m, "Emitter::emit");
    // Replace the direct call with two absorbed entry sites that
    // both re-emerge at `Emitter::emit`.
    m.relations.calls.retain(|c| c.from != handler);
    for site in 0..2u32 {
        m.analyses.stdlib_absorption.push(
            hale_model::StdlibAbsorption {
                from: handler,
                site,
                entry_dispatch: None,
                entry_in_loop: false,
                entry_group: None,
                entry_provenance: hale_model::ProvenanceId(0),
                nodes: vec![hale_model::AbsorbedNode {
                    display: "std::x::apply".to_string(),
                    carries: Vec::new(),
                    direct_effects: Vec::new(),
                    events: vec![hale_model::AbsorbedEvent::Call {
                        target: hale_model::AbsorbedTarget::User(
                            emitter,
                        ),
                        dispatch: None,
                        in_loop: false,
                        group: None,
                    }],
                }],
            },
        );
    }
    let f = hale_types::evidence::model_fanout(&m);
    let n = f(
        &hale_types::alloc_summary::FnKey::method("App", "fire"),
        0,
        "a",
    );
    assert_eq!(
        n,
        Some(3),
        "one delivery to Handler plus TWO callback publishes, each \
         reaching one Sink"
    );
}

/// …and two entries sharing an `entry_group` are alternatives of
/// ONE dispatch: their whole contributions take the max.
#[test]
fn stdlib_conformer_alternatives_take_the_max() {
    let (mut m, _b) = model_and_ids(HOP);
    let handler = fn_id(&m, "Handler::on_a");
    let emitter = fn_id(&m, "Emitter::emit");
    m.relations.calls.retain(|c| c.from != handler);
    for _ in 0..2u32 {
        m.analyses.stdlib_absorption.push(
            hale_model::StdlibAbsorption {
                from: handler,
                // One AUTHORED site, two conformers: alternatives
                // of one dispatch share the site ordinal, which is
                // the dispatch-site class the validator defines
                // across both call accounts.
                site: 0,
                entry_dispatch: None,
                entry_in_loop: false,
                entry_group: Some(7),
                entry_provenance: hale_model::ProvenanceId(0),
                nodes: vec![hale_model::AbsorbedNode {
                    display: "std::x::apply".to_string(),
                    carries: Vec::new(),
                    direct_effects: Vec::new(),
                    events: vec![hale_model::AbsorbedEvent::Call {
                        target: hale_model::AbsorbedTarget::User(
                            emitter,
                        ),
                        dispatch: None,
                        in_loop: false,
                        group: None,
                    }],
                }],
            },
        );
    }
    let f = hale_types::evidence::model_fanout(&m);
    let n = f(
        &hale_types::alloc_summary::FnKey::method("App", "fire"),
        0,
        "a",
    );
    assert_eq!(
        n,
        Some(2),
        "one dispatch runs one conformer: one delivery to Handler \
         plus one callback publish"
    );
}

/// Review pin (round 7): the absorption INTERIOR is a graph, and
/// its events carry their own dispatch group.
///
/// `std::http::Router.dispatch` fanning to two conformers is a
/// CHOICE. Flattening the interior into a bag summed both
/// re-emergences and reported three deliveries where two happen.
#[test]
fn interior_dispatch_alternatives_take_the_max() {
    let (mut m, _b) = model_and_ids(HOP);
    let handler = fn_id(&m, "Handler::on_a");
    let emitter = fn_id(&m, "Emitter::emit");
    m.relations.calls.retain(|c| c.from != handler);
    // One entry whose INTERIOR dispatches to two conformers, both
    // re-emerging at the publishing callback.
    m.analyses.stdlib_absorption.push(
        hale_model::StdlibAbsorption {
            from: handler,
            site: 0,
            entry_dispatch: None,
            entry_in_loop: false,
            entry_group: None,
            entry_provenance: hale_model::ProvenanceId(0),
            nodes: vec![hale_model::AbsorbedNode {
                display: "std::http::Router::dispatch".to_string(),
                carries: Vec::new(),
                direct_effects: Vec::new(),
                events: vec![
                    hale_model::AbsorbedEvent::Call {
                        target: hale_model::AbsorbedTarget::User(
                            emitter,
                        ),
                        dispatch: Some((
                            "Handler".to_string(),
                            "handle".to_string(),
                        )),
                        in_loop: false,
                        group: Some(1),
                    },
                    hale_model::AbsorbedEvent::Call {
                        target: hale_model::AbsorbedTarget::User(
                            emitter,
                        ),
                        dispatch: Some((
                            "Handler".to_string(),
                            "handle".to_string(),
                        )),
                        in_loop: false,
                        group: Some(1),
                    },
                ],
            }],
        },
    );
    let f = hale_types::evidence::model_fanout(&m);
    let n = f(
        &hale_types::alloc_summary::FnKey::method("App", "fire"),
        0,
        "a",
    );
    assert_eq!(
        n,
        Some(2),
        "one interior dispatch runs one conformer"
    );
}

/// An INTERIOR edge is followed, not ignored: a nested interior
/// node's re-emergence still counts.
#[test]
fn an_interior_edge_is_followed() {
    let (mut m, _b) = model_and_ids(HOP);
    let handler = fn_id(&m, "Handler::on_a");
    let emitter = fn_id(&m, "Emitter::emit");
    m.relations.calls.retain(|c| c.from != handler);
    m.analyses.stdlib_absorption.push(
        hale_model::StdlibAbsorption {
            from: handler,
            site: 0,
            entry_dispatch: None,
            entry_in_loop: false,
            entry_group: None,
            entry_provenance: hale_model::ProvenanceId(0),
            nodes: vec![
                // node 0 calls node 1…
                hale_model::AbsorbedNode {
                    display: "std::x::outer".to_string(),
                    carries: Vec::new(),
                    direct_effects: Vec::new(),
                    events: vec![hale_model::AbsorbedEvent::Call {
                        target: hale_model::AbsorbedTarget::Interior(1),
                        dispatch: None,
                        in_loop: false,
                        group: None,
                    }],
                },
                // …which re-emerges at the publishing callback.
                hale_model::AbsorbedNode {
                    display: "std::x::inner".to_string(),
                    carries: Vec::new(),
                    direct_effects: Vec::new(),
                    events: vec![hale_model::AbsorbedEvent::Call {
                        target: hale_model::AbsorbedTarget::User(
                            emitter,
                        ),
                        dispatch: None,
                        in_loop: false,
                        group: None,
                    }],
                },
            ],
        },
    );
    let f = hale_types::evidence::model_fanout(&m);
    let n = f(
        &hale_types::alloc_summary::FnKey::method("App", "fire"),
        0,
        "a",
    );
    assert_eq!(
        n,
        Some(2),
        "the nested interior's re-emergence publishes"
    );
}

/// Review pin (round 7): one authored site with a USER conformer
/// and a STDLIB conformer is one choice, not two executions.
#[test]
fn a_site_mixing_both_call_accounts_is_one_dispatch() {
    let (mut m, _b) = model_and_ids(HOP);
    let handler = fn_id(&m, "Handler::on_a");
    let emitter = fn_id(&m, "Emitter::emit");
    // The direct call already sits at some authored site; put the
    // absorption entry at the SAME one.
    let site = m
        .relations
        .calls
        .iter()
        .find(|c| c.from == handler)
        .map(|c| c.site)
        .expect("the handler calls its emitter");
    m.analyses.stdlib_absorption.push(
        hale_model::StdlibAbsorption {
            from: handler,
            site,
            entry_dispatch: None,
            entry_in_loop: false,
            entry_group: None,
            entry_provenance: hale_model::ProvenanceId(0),
            nodes: vec![hale_model::AbsorbedNode {
                display: "std::x::apply".to_string(),
                carries: Vec::new(),
                direct_effects: Vec::new(),
                events: vec![hale_model::AbsorbedEvent::Call {
                    target: hale_model::AbsorbedTarget::User(emitter),
                    dispatch: None,
                    in_loop: false,
                    group: None,
                }],
            }],
        },
    );
    let f = hale_types::evidence::model_fanout(&m);
    let n = f(
        &hale_types::alloc_summary::FnKey::method("App", "fire"),
        0,
        "a",
    );
    assert_eq!(
        n,
        Some(2),
        "one authored site is one choice across both accounts"
    );
}

/// Review pin (round 8): `loop × 0 = 0`.
///
/// The supplier refused a looped publish before knowing what it
/// delivered, so an exact zero read as unbounded. The surrounding
/// quantitative semiring already states the law; the model side has
/// to reach it.
#[test]
fn a_looped_publish_to_nobody_is_still_nobody() {
    let src = r#"
type T { n: Int = 0; }
topic A { payload: T; subject: "a"; }
locus Unused {
    bus { subscribe A as on_a; }
    params { n: Int = 0; }
    fn on_a(t: T) { self.n = t.n; }
}
main locus App {
    params { n: Int = 0; }
    bus { publish A; }
    @budget(fanout = 0)
    fn fire() {
        let mut i = 0;
        while i < 4 { A <- T { n: i }; i = i + 1; }
    }
    run() { self.fire(); }
}
fn main() { App { }; }
"#;
    let ds = diags(src);
    assert!(
        !ds.iter().any(|m| m.contains("budget exceeded")
            && m.contains("fanout")),
        "no registration, so repeating it changes nothing: {:?}",
        ds
    );
}

/// …and a looped PURE helper behind one real delivery is still
/// bounded by that delivery.
#[test]
fn a_looped_non_publishing_helper_keeps_the_bound() {
    let src = r#"
type T { n: Int = 0; }
topic A { payload: T; subject: "a"; }
locus Sink {
    bus { subscribe A as on_a; }
    params { n: Int = 0; }
    fn tick(v: Int) -> Int { return v + 1; }
    fn on_a(t: T) {
        let mut i = 0;
        while i < 4 { self.n = self.tick(self.n); i = i + 1; }
    }
}
main locus App {
    params { s: Sink = Sink { }; }
    bus { publish A; }
    @budget(fanout = 1)
    fn fire() { A <- T { n: 1 }; }
    run() { self.fire(); }
}
fn main() { App { }; }
"#;
    let ds = diags(src);
    assert!(
        !ds.iter().any(|m| m.contains("budget exceeded")
            && m.contains("fanout")),
        "a looped helper that publishes nothing amplifies nothing: \
         {:?}",
        ds
    );
}

/// An UNKNOWN filter on a locus with no instances belongs to a
/// registration that never happens.
#[test]
fn an_unknown_filter_without_a_registration_keeps_the_bound() {
    let src = r#"
type T { n: Int = 0; }
topic A { payload: T; subject: "a"; keyed_by n; }
locus Unused {
    bus { subscribe A as on_a where key == self.my_key; }
    params { my_key: Int = 1; n: Int = 0; }
    fn on_a(t: T) { self.n = t.n; }
}
main locus App {
    params { n: Int = 0; }
    bus { publish A; }
    @budget(fanout = 0)
    fn fire() { A <- T { n: 1 }; }
    run() { self.fire(); }
}
fn main() { App { }; }
"#;
    let ds = diags(src);
    assert!(
        !ds.iter().any(|m| m.contains("budget exceeded")
            && m.contains("fanout")),
        "an unknown expression on a locus with no instance routes \
         nothing: {:?}",
        ds
    );
}

/// Review pin (round 8): a FINITE type-wide domain is enumerable,
/// and enumerating it removes the impossible unmatched scenario.
///
/// A Boolean message is `false` or `true`; one of the two specific
/// registrations always matches, so the `_` fallback can never
/// fire. Inventing an unmatched scenario charged all three fallback
/// replicas.
#[test]
fn a_boolean_key_domain_leaves_the_fallback_unreachable() {
    let src = r#"
type Ev { key: Bool = false; }
topic K {
    payload: Ev;
    subject: "k";
    keyed_by key;
    on_unmatched: fallback;
}
locus FalseSink {
    bus { subscribe K as on_k where key == false; }
    params { n: Int = 0; }
    fn on_k(e: Ev) { self.n = 1; }
}
locus TrueSink {
    bus { subscribe K as on_k where key == true; }
    params { n: Int = 0; }
    fn on_k(e: Ev) { self.n = 1; }
}
locus FallbackSink {
    bus { subscribe K as on_k where key == _; }
    params { n: Int = 0; }
    fn on_k(e: Ev) { self.n = 1; }
}
main locus App {
    params {
        f: FalseSink = FalseSink { };
        t: TrueSink = TrueSink { };
        fb: FallbackSink = FallbackSink { };
    }
    placement { fb: pinned(replicas = 3); }
    bus { publish K; }
    @budget(fanout = 1)
    fn fire() { K <- Ev { key: true }; }
    run() { self.fire(); }
}
fn main() { App { }; }
"#;
    let ds = diags(src);
    assert!(
        !ds.iter().any(|m| m.contains("budget exceeded")
            && m.contains("fanout")),
        "`false` and `true` exhaust the domain, so `_` never \
         receives: {:?}",
        ds
    );
}

/// Review pin (round 8): interval coverage is a DISTINCT SET of
/// active keys, not a count of declarations.
///
/// Two subscriptions both naming key 0 cover ONE value of a
/// two-value interval, so key 1 is still possible and the fallback
/// still fires there. Counting declarations concluded the interval
/// was covered and never costed that scenario — a false pass, not a
/// conservative refusal.
#[test]
fn duplicate_specific_keys_do_not_cover_an_interval() {
    let src = r#"
type Ev { k: Int = 0; }
topic K {
    payload: Ev;
    subject: "k";
    keyed_by k;
    on_unmatched: fallback;
}
locus A0 {
    bus { subscribe K as on_k where key == 0; }
    params { n: Int = 0; }
    fn on_k(e: Ev) { self.n = 1; }
}
locus B0 {
    bus { subscribe K as on_k where key == 0; }
    params { n: Int = 0; }
    fn on_k(e: Ev) { self.n = 1; }
}
locus Fb {
    bus { subscribe K as on_k where key == _; }
    params { n: Int = 0; }
    fn on_k(e: Ev) { self.n = 1; }
}
main locus App {
    params {
        a: A0 = A0 { }; b: B0 = B0 { }; fb: Fb = Fb { };
    }
    placement { fb: pinned(replicas = 5); }
    bus { publish K; }
    fn fire() { K <- Ev { k: 0 }; }
    run() { self.fire(); }
}
fn main() { App { }; }
"#;
    let (mut m, _b) = model_and_ids(src);
    // Shape the lawful fact the source builder does not yet infer:
    // this site's keys lie in [0, 1].
    let fire = fn_id(&m, "App::fire");
    for p in m.relations.publishes.iter_mut() {
        if p.function == fire {
            p.key_domain =
                Some(hale_model::keys::KeyDomain::IntRange {
                    min: 0,
                    max: 1,
                });
        }
    }
    let f = hale_types::evidence::model_fanout(&m);
    let n = f(
        &hale_types::alloc_summary::FnKey::method("App", "fire"),
        0,
        "k",
    );
    assert_eq!(
        n,
        Some(5),
        "key 0 reaches two specific subscribers; key 1 reaches five \
         fallback replicas, and that is the maximum"
    );
}

/// Review pin (round 9): unknown population is NOT no registration.
///
/// `population_of` has three outcomes — exactly none, exactly n,
/// and not knowable. Collapsing the last two dropped the key from
/// the active partition entirely, so a `key == 1` subscriber with
/// one ARRANGED instance plus a runtime-birth hole produced no
/// candidate at all and certified fan-out zero over a delivery that
/// definitely happens. Unresolved knowledge is never absence.
#[test]
fn an_incomplete_subscriber_population_withdraws_the_bound() {
    let src = r#"
type Ev { k: Int = 0; }
topic K { payload: Ev; subject: "k"; keyed_by k; }
locus Sink {
    bus { subscribe K as on_k where key == 1; }
    params { n: Int = 0; }
    fn on_k(e: Ev) { self.n = 1; }
}
main locus App {
    params { s: Sink = Sink { }; }
    bus { publish K; }
    fn fire() { K <- Ev { k: 1 }; }
    run() { self.fire(); }
}
fn main() { App { }; }
"#;
    let (mut m, _b) = model_and_ids(src);
    let sink = hale_model::LocusDeclId(
        m.entities
            .loci
            .iter()
            .position(|l| l.name == "Sink")
            .expect("Sink") as u32,
    );
    // The arranged instance stays a known fact; the hole says more
    // can be born outside the arrangement.
    m.holes.push(hale_model::Hole {
        at: hale_model::EntityRef::LocusDecl(sink),
        kind: hale_model::HoleKind::RuntimeInheritedPlacement,
        hides: hale_model::RelationSet::OWNS
            .union(hale_model::RelationSet::PLACED),
        reason: "born outside the arrangement".to_string(),
        authored_site: None,
        provenance: hale_model::ProvenanceId(0),
    });
    m.capabilities.exact_ownership = false;
    m.capabilities.exact_placement = false;
    for p in m.relations.publishes.iter_mut() {
        p.key_domain =
            Some(hale_model::keys::KeyDomain::AnyOfType(
                "Int".to_string(),
            ));
    }
    let f = hale_types::evidence::model_fanout(&m);
    let n = f(
        &hale_types::alloc_summary::FnKey::method("App", "fire"),
        0,
        "k",
    );
    assert_eq!(
        n, None,
        "one arranged Sink definitely receives key 1, and more may \
         exist — that is not a bound of zero"
    );
}

/// The same, for `where key == replica`: a concrete-row count is a
/// LOWER bound when the population is incomplete, because a
/// dynamically born ordinary instance registers under key 0 and is
/// not listed.
#[test]
fn an_incomplete_replica_population_is_not_a_finite_count() {
    let src = r#"
type Ev { k: Int = 0; }
topic K { payload: Ev; subject: "k"; keyed_by k; }
locus Sink {
    bus { subscribe K as on_k where key == replica; }
    params { n: Int = 0; }
    fn on_k(e: Ev) { self.n = 1; }
}
main locus App {
    params { s: Sink = Sink { }; }
    bus { publish K; }
    fn fire() { K <- Ev { k: 0 }; }
    run() { self.fire(); }
}
fn main() { App { }; }
"#;
    let (mut m, _b) = model_and_ids(src);
    let sink = hale_model::LocusDeclId(
        m.entities
            .loci
            .iter()
            .position(|l| l.name == "Sink")
            .expect("Sink") as u32,
    );
    m.holes.push(hale_model::Hole {
        at: hale_model::EntityRef::LocusDecl(sink),
        kind: hale_model::HoleKind::RuntimeInheritedPlacement,
        hides: hale_model::RelationSet::OWNS
            .union(hale_model::RelationSet::PLACED),
        reason: "born outside the arrangement".to_string(),
        authored_site: None,
        provenance: hale_model::ProvenanceId(0),
    });
    m.capabilities.exact_ownership = false;
    m.capabilities.exact_placement = false;
    let f = hale_types::evidence::model_fanout(&m);
    let n = f(
        &hale_types::alloc_summary::FnKey::method("App", "fire"),
        0,
        "k",
    );
    assert_eq!(n, None, "the listed rows are not the whole population");
}

/// A subject whose SUBSCRIBER COUNT is unknown has no fan-out
/// bound, even though `causes:` is right not to care: another
/// instance of one locus runs the same handler, so it reaches the
/// same classes but not the same number of cells.
#[test]
fn a_dynamic_endpoint_withdraws_the_fanout_bound() {
    let src = r#"
type T { n: Int = 0; }
topic A { payload: T; subject: "a"; }
locus Sink {
    bus { subscribe A as on_a; }
    params { n: Int = 0; }
    fn on_a(t: T) { self.n = t.n; }
}
main locus App {
    params { s: Sink = Sink { }; }
    bus { publish A; }
    fn fire() { A <- T { n: 1 }; }
    run() { self.fire(); }
}
fn main() { App { }; }
"#;
    let (mut m, _b) = model_and_ids(src);
    let subject = hale_model::SubjectId(
        m.entities
            .subjects
            .iter()
            .position(|s| s.pattern == "a")
            .expect("subject") as u32,
    );
    m.holes.push(hale_model::Hole {
        at: hale_model::EntityRef::Subject(subject),
        kind: hale_model::HoleKind::DynamicEndpoint,
        hides: hale_model::RelationSet::CARDINALITY,
        reason: "subscriber count not statically known".to_string(),
        authored_site: None,
        provenance: hale_model::ProvenanceId(0),
    });
    m.capabilities.exact_cardinality = false;
    let f = hale_types::evidence::model_fanout(&m);
    let n = f(
        &hale_types::alloc_summary::FnKey::method("App", "fire"),
        0,
        "a",
    );
    assert_eq!(n, None, "an unknown subscriber count is no bound");
}

/// …and the anti-control: the SAME residue on an unrelated locus or
/// subject changes nothing.
#[test]
fn unrelated_population_residue_stays_irrelevant() {
    let src = r#"
type T { n: Int = 0; }
topic A { payload: T; subject: "a"; }
topic Z { payload: T; subject: "z"; }
locus Sink {
    bus { subscribe A as on_a; }
    params { n: Int = 0; }
    fn on_a(t: T) { self.n = t.n; }
}
locus Elsewhere {
    bus { subscribe Z as on_z; }
    params { n: Int = 0; }
    fn on_z(t: T) { self.n = t.n; }
}
main locus App {
    params { s: Sink = Sink { }; e: Elsewhere = Elsewhere { }; }
    bus { publish A; }
    fn fire() { A <- T { n: 1 }; }
    run() { self.fire(); }
}
fn main() { App { }; }
"#;
    let (mut m, _b) = model_and_ids(src);
    let other = hale_model::LocusDeclId(
        m.entities
            .loci
            .iter()
            .position(|l| l.name == "Elsewhere")
            .expect("Elsewhere") as u32,
    );
    let z = hale_model::SubjectId(
        m.entities
            .subjects
            .iter()
            .position(|s| s.pattern == "z")
            .expect("z") as u32,
    );
    m.holes.push(hale_model::Hole {
        at: hale_model::EntityRef::LocusDecl(other),
        kind: hale_model::HoleKind::RuntimeInheritedPlacement,
        hides: hale_model::RelationSet::OWNS
            .union(hale_model::RelationSet::PLACED),
        reason: "born outside the arrangement".to_string(),
        authored_site: None,
        provenance: hale_model::ProvenanceId(0),
    });
    m.holes.push(hale_model::Hole {
        at: hale_model::EntityRef::Subject(z),
        kind: hale_model::HoleKind::DynamicEndpoint,
        hides: hale_model::RelationSet::CARDINALITY,
        reason: "subscriber count not statically known".to_string(),
        authored_site: None,
        provenance: hale_model::ProvenanceId(0),
    });
    m.capabilities.exact_ownership = false;
    m.capabilities.exact_placement = false;
    m.capabilities.exact_cardinality = false;
    let f = hale_types::evidence::model_fanout(&m);
    let n = f(
        &hale_types::alloc_summary::FnKey::method("App", "fire"),
        0,
        "a",
    );
    assert_eq!(
        n,
        Some(1),
        "residue on another locus and another subject says nothing \
         about this closure"
    );
}

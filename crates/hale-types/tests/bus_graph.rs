//! GH #18 #4 — bus-graph property checks. PR A: ORPHAN topics.
//!
//! A bus subject wired to only one end (published with no subscriber,
//! or subscribed with no publisher) is dead wiring. These warnings
//! fire only on a closed-world program (a `main` locus present), and
//! are suppressed by transport bindings, wildcard coverage, cross-seed
//! references, and self-publish/subscribe.

use hale_syntax::parse_source;
use hale_types::check_program;

fn check(src: &str) -> Vec<String> {
    let prog = parse_source(src).expect("parse failed");
    check_program(&prog).into_iter().map(|d| d.message).collect()
}

const NO_SUB: &str = "has no subscriber";
const NO_PUB: &str = "never published";
const DEAD: &str = "neither published nor subscribed";

// --- positives -----------------------------------------------------

#[test]
fn topic_published_but_not_subscribed_warns() {
    let src = r#"
type Tick { n: Int; }
topic Beat { payload: Tick; subject: "beat"; }

locus Producer {
    bus { publish Beat; }
    birth() { Beat <- Tick { n: 1 }; }
}

main locus App {
    params { p: Producer = Producer { }; }
}

fn main() { App { }; }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().any(|m| m.contains("`Beat`") && m.contains(NO_SUB)),
        "a published-but-unsubscribed topic must warn; got: {:?}",
        msgs
    );
}

#[test]
fn topic_subscribed_but_not_published_warns() {
    let src = r#"
type Tick { n: Int; }
topic Beat { payload: Tick; subject: "beat"; }

locus Consumer {
    bus { subscribe Beat as on_beat; }
    fn on_beat(t: Tick) { }
}

main locus App {
    params { c: Consumer = Consumer { }; }
}

fn main() { App { }; }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().any(|m| m.contains("`Beat`") && m.contains(NO_PUB)),
        "a subscribed-but-unpublished topic must warn; got: {:?}",
        msgs
    );
}

#[test]
fn literal_subject_published_but_not_subscribed_warns() {
    let src = r#"
type Tick { n: Int; }

locus Producer {
    bus { publish "demo.tick" of type Tick; }
    birth() { "demo.tick" <- Tick { n: 1 }; }
}

main locus App {
    params { p: Producer = Producer { }; }
}

fn main() { App { }; }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter()
            .any(|m| m.contains("demo.tick") && m.contains(NO_SUB)),
        "a published-but-unsubscribed literal subject must warn; got: {:?}",
        msgs
    );
}

#[test]
fn topic_declared_but_unused_warns() {
    let src = r#"
type Tick { n: Int; }
topic Beat { payload: Tick; subject: "beat"; }

locus Worker { run() { } }

main locus App {
    params { w: Worker = Worker { }; }
}

fn main() { App { }; }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().any(|m| m.contains("`Beat`") && m.contains(DEAD)),
        "a declared-but-unused topic must warn; got: {:?}",
        msgs
    );
}

// --- guards (no false positives) -----------------------------------

#[test]
fn both_ends_present_is_clean() {
    let src = r#"
type Tick { n: Int; }
topic Beat { payload: Tick; subject: "beat"; }

locus Producer {
    bus { publish Beat; }
    birth() { Beat <- Tick { n: 1 }; }
}

locus Consumer {
    bus { subscribe Beat as on_beat; }
    fn on_beat(t: Tick) { }
}

main locus App {
    params {
        p: Producer = Producer { };
        c: Consumer = Consumer { };
    }
}

fn main() { App { }; }
"#;
    let msgs = check(src);
    assert!(
        !msgs.iter().any(|m| m.contains("Beat") && (m.contains(NO_SUB) || m.contains(NO_PUB) || m.contains(DEAD))),
        "a fully-wired topic must not warn; got: {:?}",
        msgs
    );
}

#[test]
fn bound_topic_is_not_orphan() {
    // Published + bound to a transport adapter, never locally
    // subscribed — the binding implies an external consumer.
    let src = r#"
type Tick { n: Int; }
topic Beat { payload: Tick; subject: "beat"; }

locus MyAdapter {
    params { label: String = "noname"; }
    fn send(subject: String, bytes: Bytes) { }
}

locus Producer {
    bus { publish Beat; }
    birth() { Beat <- Tick { n: 1 }; }
}

main locus App {
    bindings { Beat: MyAdapter { label: "T" }; }
}

fn main() { App { }; Producer { }; }
"#;
    let msgs = check(src);
    assert!(
        !msgs.iter().any(|m| m.contains("Beat") && m.contains(NO_SUB)),
        "a bound topic has an external consumer and must not be an \
         orphan; got: {:?}",
        msgs
    );
}

#[test]
fn wildcard_subscriber_covers_concrete_publish() {
    // publish "log.app", subscribe "log.**" — the wildcard covers
    // the concrete subject, so log.app is not an orphan.
    let src = r#"
type Line { msg: String; }

locus Emitter {
    bus { publish "log.app" of type Line; }
    birth() { "log.app" <- Line { msg: "hi" }; }
}

locus Sink {
    bus { subscribe "log.**" as on_log of type Line; }
    fn on_log(l: Line) { }
}

main locus App {
    params {
        e: Emitter = Emitter { };
        s: Sink    = Sink { };
    }
}

fn main() { App { }; }
"#;
    let msgs = check(src);
    assert!(
        !msgs.iter().any(|m| m.contains("log.app") && m.contains(NO_SUB)),
        "a wildcard subscriber must cover the concrete publish; got: {:?}",
        msgs
    );
}

#[test]
fn self_publish_subscribe_is_not_orphan() {
    let src = r#"
type Tick { n: Int; }
topic Beat { payload: Tick; subject: "beat"; }

locus Loop {
    bus {
        publish Beat;
        subscribe Beat as on_beat;
    }
    fn on_beat(t: Tick) { }
    birth() { Beat <- Tick { n: 1 }; }
}

main locus App {
    params { l: Loop = Loop { }; }
}

fn main() { App { }; }
"#;
    let msgs = check(src);
    assert!(
        !msgs.iter().any(|m| m.contains("Beat") && (m.contains(NO_SUB) || m.contains(NO_PUB) || m.contains(DEAD))),
        "a self-publish/subscribe topic has both ends; got: {:?}",
        msgs
    );
}

// --- cycles (PR B) -------------------------------------------------

const REENTRANT: &str = "re-entrant synchronous bus cycle";
const CROSS_CYCLE: &str = "across loci";

#[test]
fn intra_locus_self_republish_is_reentrant_error() {
    // on_t unconditionally republishes T — devirtualized synchronous
    // self-dispatch recurses without bound. Hard error.
    let src = r#"
type Tick { n: Int; }
topic T { payload: Tick; subject: "t"; }

locus Loop {
    bus { publish T; subscribe T as on_t; }
    fn on_t(x: Tick) { T <- Tick { n: 1 }; }
    birth() { T <- Tick { n: 0 }; }
}

main locus App {
    params { l: Loop = Loop { }; }
}

fn main() { App { }; }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().any(|m| m.contains(REENTRANT) && m.contains("`Loop`")),
        "an unconditional intra-locus self-republish must be a re-entrant \
         error; got: {:?}",
        msgs
    );
}

#[test]
fn intra_locus_two_topic_cycle_is_reentrant_error() {
    // on_a publishes B, on_b publishes A — both within one locus.
    let src = r#"
type Tick { n: Int; }
topic A { payload: Tick; subject: "a"; }
topic B { payload: Tick; subject: "b"; }

locus Loop {
    bus {
        publish A; publish B;
        subscribe A as on_a;
        subscribe B as on_b;
    }
    fn on_a(x: Tick) { B <- Tick { n: 1 }; }
    fn on_b(x: Tick) { A <- Tick { n: 1 }; }
    birth() { A <- Tick { n: 0 }; }
}

main locus App {
    params { l: Loop = Loop { }; }
}

fn main() { App { }; }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().any(|m| m.contains(REENTRANT)),
        "an intra-locus A→B→A cycle must be a re-entrant error; got: {:?}",
        msgs
    );
}

#[test]
fn conditional_self_republish_is_not_an_error() {
    // The send is guarded by an `if` — a terminating state machine,
    // not unbounded recursion. Must NOT error.
    let src = r#"
type Tick { n: Int; }
topic T { payload: Tick; subject: "t"; }

locus Stepper {
    bus { publish T; subscribe T as on_t; }
    fn on_t(x: Tick) {
        if x.n < 10 { T <- Tick { n: x.n + 1 }; }
    }
    birth() { T <- Tick { n: 0 }; }
}

main locus App {
    params { s: Stepper = Stepper { }; }
}

fn main() { App { }; }
"#;
    let msgs = check(src);
    assert!(
        !msgs.iter().any(|m| m.contains(REENTRANT)),
        "a guarded (conditional) self-republish terminates and must not \
         error; got: {:?}",
        msgs
    );
}

#[test]
fn cross_locus_cycle_warns() {
    // P: on_a publishes B; Q: on_b publishes A. A cell loops P↔Q
    // through the cooperative queue — a spin warning, not an error.
    let src = r#"
type Tick { n: Int; }
topic A { payload: Tick; subject: "a"; }
topic B { payload: Tick; subject: "b"; }

locus P {
    bus { subscribe A as on_a; publish B; }
    fn on_a(x: Tick) { B <- Tick { n: 1 }; }
}

locus Q {
    bus { subscribe B as on_b; publish A; }
    fn on_b(x: Tick) { A <- Tick { n: 1 }; }
    birth() { A <- Tick { n: 0 }; }
}

main locus App {
    params {
        p: P = P { };
        q: Q = Q { };
    }
}

fn main() { App { }; }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().any(|m| m.contains(CROSS_CYCLE)),
        "a cross-locus pub/sub cycle must warn; got: {:?}",
        msgs
    );
    assert!(
        !msgs.iter().any(|m| m.contains(REENTRANT)),
        "a cross-locus cycle is a warning, not the synchronous error; \
         got: {:?}",
        msgs
    );
}

#[test]
fn acyclic_pub_sub_chain_has_no_cycle_diagnostic() {
    // A → B → C, no back-edge. No cycle.
    let src = r#"
type Tick { n: Int; }

locus First {
    bus { subscribe "a" as on_a of type Tick; publish "b" of type Tick; }
    fn on_a(x: Tick) { "b" <- Tick { n: 1 }; }
}

locus Second {
    bus { subscribe "b" as on_b of type Tick; publish "c" of type Tick; }
    fn on_b(x: Tick) { "c" <- Tick { n: 1 }; }
}

locus Source {
    bus { publish "a" of type Tick; }
    birth() { "a" <- Tick { n: 0 }; }
}

locus Sink {
    bus { subscribe "c" as on_c of type Tick; }
    fn on_c(x: Tick) { }
}

main locus App {
    params {
        f: First = First { };
        s: Second = Second { };
        src: Source = Source { };
        snk: Sink = Sink { };
    }
}

fn main() { App { }; }
"#;
    let msgs = check(src);
    assert!(
        !msgs.iter().any(|m| m.contains(CROSS_CYCLE) || m.contains(REENTRANT)),
        "an acyclic chain must produce no cycle diagnostic; got: {:?}",
        msgs
    );
}

#[test]
fn library_without_main_is_not_checked() {
    // No `main` locus: the publishers/subscribers may live in
    // downstream consumers, so orphan detection is suppressed.
    let src = r#"
type Tick { n: Int; }
topic Beat { payload: Tick; subject: "beat"; }

locus Producer {
    bus { publish Beat; }
    birth() { Beat <- Tick { n: 1 }; }
}
"#;
    let msgs = check(src);
    assert!(
        !msgs.iter().any(|m| m.contains("Beat") && m.contains(NO_SUB)),
        "a library (no main) must not get orphan warnings; got: {:?}",
        msgs
    );
}

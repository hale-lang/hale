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

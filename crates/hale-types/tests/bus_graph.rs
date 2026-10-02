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

// --- backpressure (PR C) -------------------------------------------

const BACKPRESSURE: &str = "no backpressure";

fn flood_src(run_body: &str) -> String {
    format!(
        r#"
type Tick {{ n: Int; }}
topic Beat {{ payload: Tick; subject: "beat"; }}

locus Flooder {{
    bus {{ publish Beat; subscribe Beat as on_beat; }}
    fn on_beat(t: Tick) {{ }}
    run() {{ {run_body} }}
}}

main locus App {{
    params {{ f: Flooder = Flooder {{ }}; }}
}}

fn main() {{ App {{ }}; }}
"#
    )
}

#[test]
fn unbounded_publish_loop_with_no_flow_control_warns() {
    let msgs = check(&flood_src("while true { Beat <- Tick { n: 1 }; }"));
    assert!(
        msgs.iter().any(|m| m.contains(BACKPRESSURE) && m.contains("`Flooder`")),
        "an unthrottled publish loop must warn; got: {:?}",
        msgs
    );
}

#[test]
fn throttled_publish_loop_is_ok() {
    let msgs = check(&flood_src(
        "while true { Beat <- Tick { n: 1 }; std::time::sleep(1s); }",
    ));
    assert!(
        !msgs.iter().any(|m| m.contains(BACKPRESSURE)),
        "a sleep-paced publish loop has backpressure; must not warn; got: {:?}",
        msgs
    );
}

#[test]
fn yielding_publish_loop_is_ok() {
    let msgs = check(&flood_src("while true { Beat <- Tick { n: 1 }; yield; }"));
    assert!(
        !msgs.iter().any(|m| m.contains(BACKPRESSURE)),
        "a yielding publish loop lets the subscriber drain; must not warn; \
         got: {:?}",
        msgs
    );
}

#[test]
fn input_driven_publish_loop_is_ok() {
    // A blocking recv paces the loop — publish rate follows input.
    // GH #829: the buffer is a real `BytesBuilder` (hoisted out of
    // the loop, as the recv_into idiom wants). Passing `0` there was
    // a program `hale check` accepted and `hale build` refused.
    let msgs = check(&flood_src(
        "let b = std::bytes::BytesBuilder { initial_cap: 64 }; \
         while true { let n = std::io::tcp::recv_into(0, b, 64); Beat <- Tick { n: 1 }; }",
    ));
    assert!(
        !msgs.iter().any(|m| m.contains(BACKPRESSURE)),
        "an input-paced publish loop must not warn; got: {:?}",
        msgs
    );
}

#[test]
fn stdin_driven_publish_loop_is_ok() {
    // GH #830: "input-pacing" is the registry's `block` rows, so a
    // line read off stdin paces the loop exactly as a blocking
    // `recv` does. Under the old hand list of 11 paths this drew the
    // flood warning — whose own suggested fix ("drive it from an
    // input") was what the program already did.
    let msgs = check(&flood_src(
        "while true { let line = std::io::stdin::read_line(); \
         Beat <- Tick { n: 1 }; }",
    ));
    assert!(
        !msgs.iter().any(|m| m.contains(BACKPRESSURE)),
        "a stdin-paced publish loop must not warn; got: {:?}",
        msgs
    );
}

#[test]
fn bounded_for_loop_publish_is_ok() {
    let msgs = check(&flood_src("for i in 0..10 { Beat <- Tick { n: i }; }"));
    assert!(
        !msgs.iter().any(|m| m.contains(BACKPRESSURE)),
        "a bounded loop posts a bounded number of cells; must not warn; \
         got: {:?}",
        msgs
    );
}

#[test]
fn breakable_publish_loop_is_ok() {
    let msgs = check(&flood_src(
        "let mut i = 0; while true { Beat <- Tick { n: i }; i = i + 1; if i > 5 { break; } }",
    ));
    assert!(
        !msgs.iter().any(|m| m.contains(BACKPRESSURE)),
        "a loop that can `break` is bounded; must not warn; got: {:?}",
        msgs
    );
}

#[test]
fn unbounded_loop_without_publish_is_ok() {
    let msgs = check(&flood_src("while true { let x = 1 + 1; }"));
    assert!(
        !msgs.iter().any(|m| m.contains(BACKPRESSURE)),
        "an unbounded loop that doesn't publish isn't a bus flood; got: {:?}",
        msgs
    );
}

// --- subject type-mismatch (PR D) ----------------------------------

const CONFLICT: &str = "conflicting payload types";

#[test]
fn literal_subject_with_conflicting_payload_types_errors() {
    let src = r#"
type Tick { n: Int; }
type Pulse { hz: Int; }

locus Pub {
    bus { publish "wire.sig" of type Tick; }
    birth() { "wire.sig" <- Tick { n: 1 }; }
}

locus Sub {
    bus { subscribe "wire.sig" as on_sig of type Pulse; }
    fn on_sig(p: Pulse) { }
}

main locus App {
    params { p: Pub = Pub { }; s: Sub = Sub { }; }
}

fn main() { App { }; }
"#;
    let msgs = check(src);
    assert!(
        msgs.iter().any(|m| m.contains(CONFLICT)
            && m.contains("wire.sig")
            && m.contains("Tick")
            && m.contains("Pulse")),
        "mismatched payload types on the same literal subject must error; \
         got: {:?}",
        msgs
    );
}

#[test]
fn literal_subject_with_matching_payload_types_is_clean() {
    let src = r#"
type Tick { n: Int; }

locus Pub {
    bus { publish "wire.sig" of type Tick; }
    birth() { "wire.sig" <- Tick { n: 1 }; }
}

locus Sub {
    bus { subscribe "wire.sig" as on_sig of type Tick; }
    fn on_sig(t: Tick) { }
}

main locus App {
    params { p: Pub = Pub { }; s: Sub = Sub { }; }
}

fn main() { App { }; }
"#;
    let msgs = check(src);
    assert!(
        !msgs.iter().any(|m| m.contains(CONFLICT)),
        "agreeing payload types must not error; got: {:?}",
        msgs
    );
}

#[test]
fn different_subjects_with_different_types_is_clean() {
    // Two different subjects, each with its own type — no conflict.
    let src = r#"
type Tick { n: Int; }
type Pulse { hz: Int; }

locus L {
    bus {
        publish "a" of type Tick;
        publish "b" of type Pulse;
        subscribe "a" as on_a of type Tick;
        subscribe "b" as on_b of type Pulse;
    }
    fn on_a(t: Tick) { }
    fn on_b(p: Pulse) { }
}

main locus App {
    params { l: L = L { }; }
}

fn main() { App { }; }
"#;
    let msgs = check(src);
    assert!(
        !msgs.iter().any(|m| m.contains(CONFLICT)),
        "distinct subjects with distinct types must not error; got: {:?}",
        msgs
    );
}

#[test]
fn declared_topic_is_not_subject_to_mismatch() {
    // A declared topic unifies its payload at the declaration, so two
    // sites referencing it can't disagree — and carry no `of type`.
    let src = r#"
type Tick { n: Int; }
topic Beat { payload: Tick; subject: "beat"; }

locus Pub {
    bus { publish Beat; }
    birth() { Beat <- Tick { n: 1 }; }
}

locus Sub {
    bus { subscribe Beat as on_beat; }
    fn on_beat(t: Tick) { }
}

main locus App {
    params { p: Pub = Pub { }; s: Sub = Sub { }; }
}

fn main() { App { }; }
"#;
    let msgs = check(src);
    assert!(
        !msgs.iter().any(|m| m.contains(CONFLICT)),
        "declared topics are unified by their declaration; got: {:?}",
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

// --- the intra-locus relation (F.40 phase 1.5, boundary 7) ----------

/// The intra-locus rewrite turns `PingT <- ..` into `self.w.on_ping(..)`,
/// erasing the send from the program lowering walks. The resolved
/// program keeps it as a relation, and the graph over the rewritten
/// program records it on the subject's wire key.
#[test]
fn a_rewritten_send_stays_on_the_resolved_graph() {
    let src = r#"
type Ping { n: Int = 0; }
topic PingT { payload: Ping; subject: "p.ping"; }
locus Worker {
    params { seen: Int = 0; }
    bus { subscribe PingT as on_ping; }
    fn on_ping(p: Ping) { self.seen = p.n; }
}
main locus App {
    params { w: Worker = Worker { }; }
    bus { publish PingT; }
    run() { PingT <- Ping { n: 1 }; }
}
fn main() { App { }; }
"#;
    let prog = parse_source(src).expect("parse failed");
    let resolved = hale_types::resolved::resolve_program(&prog, &[], &[], None, None, &hale_types::form_rows::FormRows::default(), &hale_types::placement::PlacementTable::default())
        .expect("resolves");
    assert_eq!(resolved.intra_locus.len(), 1, "{:?}", resolved.intra_locus);
    let rw = &resolved.intra_locus[0];
    assert_eq!((rw.locus.as_str(), rw.subject.as_str(), rw.handler.as_str()), ("App", "PingT", "on_ping"));
    let info = resolved.bus.subjects.get("p.ping").expect("the wire subject is in the graph");
    assert_eq!(info.direct_sends, vec![("App".to_string(), "on_ping".to_string())]);

    // Outside review of #1276, finding 4: this is the adapter path (the
    // program was never minted before `resolve_program`), and the
    // relation's sends are minted all the same, because the mint runs
    // before the rewrite: no recorded send is `NONE`, and each is the
    // id of the direct call that replaced it, a site of the snapshot.
    assert!(
        resolved.intra_locus.iter().all(|rw| !rw.send.is_none()),
        "{:?}",
        resolved.intra_locus
    );
    let mut call_ids = Vec::new();
    for item in &resolved.merged.items {
        let hale_syntax::ast::TopDecl::Locus(l) = item else { continue };
        if l.name.name != "App" {
            continue;
        }
        for m in &l.members {
            let hale_syntax::ast::LocusMember::Lifecycle(d) = m else { continue };
            for s in &d.body.stmts {
                if let hale_syntax::ast::Stmt::Expr(hale_syntax::ast::Expr::Call { id, .. }) = s {
                    call_ids.push(id.0);
                }
            }
        }
    }
    assert_eq!(call_ids, vec![rw.send.0], "the direct call carries the recorded send's id");
    assert!(resolved.snapshot.site_id(rw.send).is_some(), "the send's id is a site of the snapshot");
}

/// F.40 phase 2.1b: the topic rewrite turns every topic reference into
/// its wire literal after the check, erasing the declaration name from
/// the program lowering walks. The resolved program keeps each as a
/// `TopicRewrite` row (the site, the topic as written, the wire), and
/// the graph over the rewritten program records them on the wire
/// subject. Publisher and subscriber are unrelated loci, so the
/// intra-locus rewrite leaves the send to the topic rewrite.
#[test]
fn a_rewritten_topic_reference_stays_on_the_resolved_graph() {
    let src = r#"
type Tick { n: Int = 0; }
topic Beat { payload: Tick; subject: "b.beat"; }
locus Worker {
    params { seen: Int = 0; }
    bus { subscribe Beat as on_beat; }
    fn on_beat(t: Tick) { self.seen = t.n; }
}
locus Clock {
    bus { publish Beat; }
    run() { Beat <- Tick { n: 1 }; }
}
main locus App {
    params { w: Worker = Worker { }; c: Clock = Clock { }; }
}
fn main() { App { }; }
"#;
    // As an entry point: the sequence, the mint, then the resolve.
    let mut prog = parse_source(src).expect("parse failed");
    hale_types::desugar_sequence::desugar_before_check(
        &mut [&mut prog],
        &hale_types::desugar_sequence::Sequence { import_renames: &[], api: None, api_roles: None },
    )
    .expect("no --api to refuse");
    hale_types::snapshot::mint([("app.hl", &mut prog)], &[]);
    let resolved = hale_types::resolved::resolve_program(&prog, &[], &[], None, None, &hale_types::form_rows::FormRows::default(), &hale_types::placement::PlacementTable::default())
        .expect("resolves");
    assert!(resolved.intra_locus.is_empty(), "{:?}", resolved.intra_locus);

    // The subscribe, the publish and the send, each by its minted site.
    let rows = &resolved.topic_rewrites;
    assert_eq!(rows.len(), 3, "{rows:?}");
    for rw in rows {
        assert_eq!((rw.written.as_str(), rw.wire.as_str()), ("Beat", "b.beat"), "{rw:?}");
        assert!(!rw.site.is_none(), "the site keeps its identity: {rw:?}");
    }
    // `NodeId`'s equality ignores the id (it compares shapes), so the
    // sites are compared by their index.
    let sites: std::collections::BTreeSet<u32> = rows.iter().map(|rw| rw.site.0).collect();
    assert_eq!(sites.len(), 3, "three distinct sites: {rows:?}");

    let info = resolved.bus.subjects.get("b.beat").expect("the wire subject is in the graph");
    let mut on_graph: Vec<(u32, String)> =
        info.written_topics.iter().map(|(s, t)| (s.0, t.clone())).collect();
    on_graph.sort();
    let mut expected: Vec<(u32, String)> = rows.iter().map(|rw| (rw.site.0, rw.written.clone())).collect();
    expected.sort();
    assert_eq!(on_graph, expected);
}

// --- placement labels read the placement table (F.40 phase 3, P1 3 of 6) ---
//
// The rows are the placement correspondence's (`notes/f40-placement-
// correspondence.md` § 2.2). Each program loads as `hale check` loads it,
// so the graph reads the snapshot's table.

use hale_types::bus_graph::Placement;

/// The snapshot `hale check` would build for `src`, checked clean.
fn snapshot_of(src: &str) -> hale_frontend::snapshot::Snapshot {
    use hale_frontend::snapshot::{Config, Snapshot};
    let program = parse_source(src).expect("parse failed");
    let s = Snapshot::from_program(program, Vec::new(), Config::check(false, false))
        .unwrap_or_else(|_| panic!("the program does not load"));
    let checked = s.demand_check().unwrap_or_else(|_| panic!("the check is blocked"));
    let errors: Vec<&str> = checked.diags.iter().filter(|d| d.is_error()).map(|d| d.message.as_str()).collect();
    assert!(errors.is_empty(), "the program must check clean: {errors:?}");
    s
}

/// The label of `locus`'s subscription on `subject`, and the subject's
/// direct-call gate.
fn label(s: &hale_frontend::snapshot::Snapshot, subject: &str, locus: &str) -> (Placement, bool) {
    let g = s.demand_bus_graph().unwrap_or_else(|_| panic!("the bus graph is blocked"));
    let info = g.subjects.get(subject).unwrap_or_else(|| panic!("no subject `{subject}`: {:?}", g.subjects.keys()));
    let site = info.subscribers.iter().find(|x| x.locus == locus).unwrap_or_else(|| panic!("no subscriber `{locus}`"));
    (site.placement.clone(), info.direct_call_eligible)
}

/// The label the table gives `decl`, by the name lowering keys on.
fn type_label(s: &hale_frontend::snapshot::Snapshot, decl: &str) -> Placement {
    let t = s.demand_placement().unwrap_or_else(|_| panic!("placement is blocked"));
    hale_types::bus_graph::type_placements(t).get(decl).cloned().unwrap_or(Placement::SameThread)
}

/// A quiet subscriber on a flat payload, published from the root on
/// main: every leg of the direct-call gate but placement holds.
fn nested_under(placement: &str) -> String {
    format!(
        r#"
type P {{ n: Int; }}
topic T {{ payload: P; }}

locus Kid {{
    params {{ got: Int = 0; }}
    bus {{ subscribe T as on_t; }}
    fn on_t(p: P) {{ self.got = self.got + 1; }}
}}

locus Owner {{
    params {{ k: Kid = Kid {{ }}; }}
}}

main locus App {{
    params {{ o: Owner = Owner {{ }}; }}
    placement {{ o: {placement}; }}
    bus {{ publish T; }}
    run() {{ T <- P {{ n: 1 }}; }}
}}

fn main() {{ App {{ }}; }}
"#
    )
}

/// B-1: a locus nested under a root field placed off main runs on that
/// field's thread, so it is not same-thread, and its subject is no
/// direct call. The legacy label read the root's entries by the written
/// type of the field alone and called `Kid` `SameThread`.
#[test]
fn a_nested_subscriber_under_a_placed_owner_is_not_same_thread() {
    assert_eq!(label(&snapshot_of(&nested_under("pinned")), "T", "Kid"), (Placement::Pinned, false));
    assert_eq!(
        label(&snapshot_of(&nested_under("cooperative(pool = io)")), "T", "Kid"),
        (Placement::CrossPool("io".into()), false)
    );
    // The control: under a root field on main, the gate holds.
    assert_eq!(
        label(&snapshot_of(&nested_under("cooperative(pool = main)")), "T", "Kid"),
        (Placement::SameThread, true)
    );
}

/// B-4: an imported `__lib_` root is never deployed, so its entries
/// label nothing (the legacy label read every `placement { }` block,
/// first wins, and called `W` pinned).
#[test]
fn an_imported_roots_placement_labels_nothing() {
    let src = r#"
type P { n: Int; }
topic T { payload: P; }

locus W {
    params { got: Int = 0; }
    bus { subscribe T as on_t; publish T; }
    fn on_t(p: P) { self.got = self.got + 1; }
}

main locus __lib_App {
    params { w: W = W { }; }
    placement { w: pinned; }
}

fn main() { W { }; }
"#;
    assert_eq!(label(&snapshot_of(src), "T", "W"), (Placement::SameThread, true));
}

/// B-3 and B-5: a root field typed by a qualified stdlib path is placed
/// by the declaration it builds (`__StdIoTcpListener`), not by its last
/// segment; so a user `locus Listener` beside it is labelled by its own
/// instances. The legacy label keyed both by `Listener` and gave the
/// user's the stdlib field's pool.
#[test]
fn a_qualified_field_labels_its_declaration_by_identity() {
    let src = r#"
type P { n: Int; }
topic T { payload: P; }

fn ignore_conn(s: std::io::tcp::Stream) { }

locus Listener {
    params { got: Int = 0; }
    bus { subscribe T as on_t; }
    fn on_t(p: P) { self.got = self.got + 1; }
}

main locus App {
    params {
        l: std::io::tcp::Listener = std::io::tcp::Listener {
            host: "127.0.0.1", port: 0, max_accepts: -1, on_connection: ignore_conn,
        };
        u: Listener = Listener { };
    }
    placement { l: cooperative(pool = io) where async_io; }
    bus { publish T; }
    run() { T <- P { n: 1 }; }
}

fn main() { App { }; }
"#;
    let s = snapshot_of(src);
    assert_eq!(type_label(&s, "__StdIoTcpListener"), Placement::CrossPool("io".into()), "B-3");
    assert_eq!(label(&s, "T", "Listener"), (Placement::SameThread, true), "B-5");
}

/// B-6: one type, two instances in two domains: the label is the set's,
/// and the gate holds only when every instance runs on main.
#[test]
fn a_type_with_an_instance_off_main_is_not_same_thread() {
    let src = r#"
type P { n: Int; }
topic T { payload: P; }

locus W {
    params { got: Int = 0; }
    bus { subscribe T as on_t; }
    fn on_t(p: P) { self.got = self.got + 1; }
}

main locus App {
    params { a: W = W { }; b: W = W { }; }
    placement { a: pinned; }
    bus { publish T; }
    run() { T <- P { n: 1 }; }
}

fn main() { App { }; }
"#;
    assert_eq!(label(&snapshot_of(src), "T", "W"), (Placement::Pinned, false));
}

/// B-7: an adapter in the root's `bindings { }` runs on a thread of its
/// own (GH #1032; measured by `nested_offthread_delivery.rs`'s adapter
/// case), so it is pinned, and a subject it publishes is no direct call.
#[test]
fn an_adapter_is_not_same_thread() {
    let src = r#"
type P { n: Int; }
type Wire { n: Int; }
topic Beat { payload: Wire; subject: "beat"; }
topic T { payload: P; }

locus Probe {
    params { got: Int = 0; }
    bus { subscribe T as on_t; publish T; }
    fn send(subject: String, bytes: Bytes) { }
    fn on_t(p: P) { self.got = self.got + 1; }
}

locus Sink {
    params { got: Int = 0; }
    bus { subscribe T as on_t; }
    fn on_t(p: P) { self.got = self.got + 1; }
}

main locus App {
    params { s: Sink = Sink { }; }
    bindings { Beat: Probe { }; }
    bus { publish Beat; }
}

fn main() { App { }; }
"#;
    let s = snapshot_of(src);
    assert_eq!(label(&s, "T", "Probe"), (Placement::Pinned, false));
    assert_eq!(label(&s, "T", "Sink").0, Placement::SameThread, "the subscriber on main keeps its label");
}

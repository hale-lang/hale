//! GH #1327 § 2 — `require no_silent_loss(topic T | all G)`: no
//! modeled boundary on a named route is configured to discard
//! silently.
//!
//! Discipline (soundness law 4): the claim ships with the canaries
//! that MUST fail — one program per kind of silent discard, naming the
//! boundary and its setting — beside the programs that must hold, the
//! observable refusal among them, and the custom refusal handler that
//! is neither: it is `uncertified`, never `holds`.

#[path = "support/entries.rs"]
mod entries;
use hale_syntax::parse_source;

fn diags(src: &str) -> Vec<String> {
    let program = parse_source(src).expect("parse");
    entries::check_program(&program)
        .into_iter()
        .map(|d| d.message)
        .collect()
}

fn claim_diags(src: &str) -> Vec<String> {
    diags(src)
        .into_iter()
        .filter(|m| m.contains("claim `loss`"))
        .collect()
}

/// A keyed topic whose unmatched key FAILS the send (`on_unmatched:
/// fail`: the refusal is the sender's to see), one subscriber, one
/// publisher. `SUBOPTS` is the subscription's bound, `SEND` the send's
/// `or` clause, `CLAIM` the claim under test.
const ROUTE: &str = r#"
type Pay { id: Int = 0; }
topic Payments { payload: Pay; subject: "pay"; keyed_by id; on_unmatched: fail; }

locus Ledger {
    params { seen: Int = 0; }
    bus { subscribe Payments as on_pay SUBOPTS where key == 1; }
    fn on_pay(p: Pay) { self.seen = self.seen + 1; }
}

group route = { Ledger };

main locus App {
    params { l: Ledger = Ledger { }; }
    bus { publish Payments; }
    claims {
        loss: CLAIM;
    }
    run() { Payments <- Pay { id: 1 } SEND; }
}
fn main() { App { }; }
"#;

fn route(subopts: &str, send: &str, claim: &str) -> String {
    ROUTE
        .replace("SUBOPTS", subopts)
        .replace("SEND", send)
        .replace("CLAIM", claim)
}

/// A bounded topic whose full queue refuses the send (`on_full:
/// fail`). `SEND` is the disposition; `BINDINGS` an optional binding.
const BOUNDED: &str = r#"
type Pay { id: Int = 0; }
topic Payments { payload: Pay; subject: "pay"; bounded(8); on_full: fail; }

locus Ledger {
    params { seen: Int = 0; }
    bus { subscribe Payments as on_pay; }
    fn on_pay(p: Pay) { self.seen = self.seen + 1; }
}

main locus App {
    params { l: Ledger = Ledger { }; }
    BINDINGS
    bus { publish Payments; }
    claims {
        loss: require no_silent_loss(topic Payments);
    }
    run() { Payments <- Pay { id: 1 } SEND; }
}
fn main() { App { }; }
"#;

fn bounded(send: &str, bindings: &str) -> String {
    BOUNDED.replace("SEND", send).replace("BINDINGS", bindings)
}

/// The same route with no bound, carried over a unix link.
fn linked(send: &str, role: &str) -> String {
    let b = format!(
        "bindings {{ Payments: unix(\"/tmp/hale-noloss.sock\", role: {role}); }}"
    );
    bounded(send, &b).replace("bounded(8); on_full: fail;", "")
}

const TOPIC: &str = "require no_silent_loss(topic Payments)";
const GROUP: &str = "require no_silent_loss(all route)";

fn assert_clean(src: &str) {
    assert_eq!(claim_diags(src), Vec::<String>::new(), "{:?}", diags(src));
    assert!(diags(src).is_empty(), "{:?}", diags(src));
}

// ---- holds ----------------------------------------------------------

#[test]
fn a_route_with_every_boundary_non_lossy_holds() {
    // The unbounded queue sheds nothing; a key nobody serves fails
    // the send, and `or raise` makes that the caller's to see.
    assert_clean(&route("", "or raise", TOPIC));
    assert_clean(&route("", "or raise", GROUP));
}

#[test]
fn a_waiting_send_is_an_observable_refusal_and_holds() {
    assert_clean(&bounded("or wait", ""));
}

#[test]
fn a_fallback_unmatched_policy_holds() {
    let src = r#"
type Pay { id: Int = 0; }
topic Payments { payload: Pay; subject: "pay"; keyed_by id; on_unmatched: fallback; }
locus Ledger {
    params { seen: Int = 0; }
    bus {
        subscribe Payments as on_pay where key == 1;
        subscribe Payments as on_rest where key == _;
    }
    fn on_pay(p: Pay) { self.seen = self.seen + 1; }
    fn on_rest(p: Pay) { self.seen = self.seen + 1; }
}
main locus App {
    params { l: Ledger = Ledger { }; }
    bus { publish Payments; }
    claims { loss: require no_silent_loss(topic Payments); }
    run() { Payments <- Pay { id: 7 }; }
}
fn main() { App { }; }
"#;
    assert_clean(src);
}

// ---- violated: one canary per kind of silent discard ----------------

#[test]
fn a_swallowing_unmatched_policy_is_named() {
    let src = r#"
type Pay { id: Int = 0; }
topic Payments { payload: Pay; subject: "pay"; keyed_by id; }
locus Ledger {
    params { seen: Int = 0; }
    bus { subscribe Payments as on_pay where key == 1; }
    fn on_pay(p: Pay) { self.seen = self.seen + 1; }
}
main locus App {
    params { l: Ledger = Ledger { }; }
    bus { publish Payments; }
    claims { loss: require no_silent_loss(topic Payments); }
    run() { Payments <- Pay { id: 7 }; }
}
fn main() { App { }; }
"#;
    let ds = claim_diags(src);
    assert_eq!(ds.len(), 1, "{:?}", ds);
    assert!(ds[0].contains("violated"), "{}", ds[0]);
    assert!(ds[0].contains("topic `Payments` keyed by `id`"), "{}", ds[0]);
    assert!(ds[0].contains("on_unmatched: swallow"), "{}", ds[0]);
}

#[test]
fn a_shedding_subscriber_queue_is_named() {
    for policy in ["drop_old", "drop_new"] {
        let src = route(&format!("bounded(4, {policy})"), "or raise", TOPIC);
        let ds = claim_diags(&src);
        assert_eq!(ds.len(), 1, "{:?} {:?}", ds, diags(&src));
        assert!(ds[0].contains("violated"), "{}", ds[0]);
        assert!(ds[0].contains("subscriber `Ledger::on_pay`"), "{}", ds[0]);
        assert!(
            ds[0].contains(&format!("bounded(4), on_full: {policy}")),
            "{}",
            ds[0]
        );
    }
}

#[test]
fn an_explicit_discard_is_named() {
    let src = route("", "or discard", TOPIC);
    let ds = claim_diags(&src);
    assert_eq!(ds.len(), 1, "{:?}", ds);
    assert!(ds[0].contains("violated"), "{}", ds[0]);
    assert!(ds[0].contains("send in `App::run`"), "{}", ds[0]);
    assert!(ds[0].contains("`or discard`"), "{}", ds[0]);
}

#[test]
fn a_group_claim_finds_the_discard_on_a_route_it_subscribes() {
    // The group is `route` = { Ledger }, which only SUBSCRIBES: its
    // route is `Payments`, and the discarding send sits in `App`.
    let src = route("", "or discard", GROUP);
    let ds = claim_diags(&src);
    assert_eq!(ds.len(), 1, "{:?}", ds);
    assert!(ds[0].contains("violated"), "{}", ds[0]);
    assert!(ds[0].contains("a route of `route`"), "{}", ds[0]);
    assert!(ds[0].contains("`or discard`"), "{}", ds[0]);
}

#[test]
fn every_silent_setting_is_listed_not_just_the_first() {
    let src = route("bounded(4, drop_old)", "or discard", TOPIC);
    let ds = claim_diags(&src);
    assert_eq!(ds.len(), 1, "{:?}", ds);
    assert!(ds[0].contains("drop_old"), "{}", ds[0]);
    assert!(ds[0].contains("or discard"), "{}", ds[0]);
}

#[test]
fn a_violation_beats_an_unproven_handler() {
    let src = route("bounded(4, drop_old)", "or println(\"lost\")", TOPIC);
    let ds = claim_diags(&src);
    assert_eq!(ds.len(), 1, "{:?} {:?}", ds, diags(&src));
    assert!(ds[0].contains("violated"), "{}", ds[0]);
}

// ---- unproven: a custom refusal handler, a supervised binding -------

#[test]
fn a_custom_refusal_handler_is_uncertified_not_holding() {
    let src = route("", "or println(\"lost\")", TOPIC);
    let ds = claim_diags(&src);
    assert_eq!(ds.len(), 1, "{:?} {:?}", ds, diags(&src));
    assert!(ds[0].contains("uncertified"), "{}", ds[0]);
    assert!(
        ds[0].contains("send in `App::run` on `Payments` routes its refusal"),
        "{}",
        ds[0]
    );
    assert!(ds[0].contains("custom handler"), "{}", ds[0]);
}

#[test]
fn a_connect_binding_whose_loss_is_supervision_policy_is_uncertified() {
    let src = linked("", "connect");
    let ds = claim_diags(&src);
    assert_eq!(ds.len(), 1, "{:?} {:?}", ds, diags(&src));
    assert!(ds[0].contains("uncertified"), "{}", ds[0]);
    assert!(ds[0].contains("unix connect binding"), "{}", ds[0]);
}

#[test]
fn a_connect_binding_whose_every_send_waits_holds() {
    assert_clean(&linked("or wait", "connect"));
}

#[test]
fn a_listen_binding_fails_loudly_and_holds() {
    assert_clean(&linked("", "listen"));
}

// ---- wildcard endpoints cover the wires under them ------------------

/// A group whose only member subscribes a WILDCARD, `"pay.**"`, and a
/// keyed topic on a concrete wire under it. `SUB` is the subscribed
/// subject, `SEND` the send's `or` clause, `CLAIM` the claim.
const WILD: &str = r#"
type Pay { id: Int = 0; }
topic Payments { payload: Pay; subject: "pay.event"; keyed_by id; on_unmatched: fail; }

locus Ledger {
    params { seen: Int = 0; }
    bus { subscribe "SUB" as on_pay of type Pay; }
    fn on_pay(p: Pay) { self.seen = self.seen + 1; }
}

group route = { Ledger };

main locus App {
    params { l: Ledger = Ledger { }; }
    bus { publish Payments; }
    claims {
        loss: CLAIM;
    }
    run() { Payments <- Pay { id: 1 } SEND; }
}
fn main() { App { }; }
"#;

fn wild(sub: &str, send: &str, claim: &str) -> String {
    WILD.replace("SUB", sub)
        .replace("SEND", send)
        .replace("CLAIM", claim)
}

#[test]
fn a_wildcard_subscription_judges_the_wires_it_covers() {
    // The reviewer's control: the discarding send is on `pay.event`,
    // which the group reaches only through its `pay.**` subscription.
    // Read as one literal subject, the pattern matched no send and the
    // claim held.
    let src = wild("pay.**", "or discard", GROUP);
    let ds = claim_diags(&src);
    assert_eq!(ds.len(), 1, "{:?} {:?}", ds, diags(&src));
    assert!(ds[0].contains("violated"), "{}", ds[0]);
    assert!(ds[0].contains("send in `App::run` on `Payments`"), "{}", ds[0]);
    assert!(ds[0].contains("`or discard`"), "{}", ds[0]);
    // …and it agrees with the two spellings that always saw it.
    for (sub, claim) in [("pay.event", GROUP), ("pay.**", TOPIC)] {
        let src = wild(sub, "or discard", claim);
        let ds = claim_diags(&src);
        assert_eq!(ds.len(), 1, "{sub} {claim}: {:?}", ds);
        assert!(ds[0].contains("violated"), "{}", ds[0]);
        assert!(ds[0].contains("`or discard`"), "{}", ds[0]);
    }
}

#[test]
fn a_topic_address_spelled_with_a_wildcard_is_still_a_route() {
    // The reviewer's control: the topic's own subject is
    // `pay.inner.**`. The bus delivers a send to it on `pay.**`, so it
    // is a route of the group — not a pattern's remainder to skip.
    let src = WILD
        .replace("\"pay.event\"", "\"pay.inner.**\"")
        .replace("SUB", "pay.**")
        .replace("SEND", "or discard");
    for claim in [GROUP, TOPIC] {
        let src = src.replace("CLAIM", claim);
        let ds = claim_diags(&src);
        assert_eq!(ds.len(), 1, "{claim}: {:?} {:?}", ds, diags(&src));
        assert!(ds[0].contains("violated"), "{}", ds[0]);
        assert!(
            ds[0].contains("send in `App::run` on `Payments`"),
            "{}",
            ds[0]
        );
        assert!(ds[0].contains("`or discard`"), "{}", ds[0]);
    }
}

#[test]
fn a_wildcard_subscription_finds_a_swallowing_topic_under_it() {
    let src = wild("pay.**", "", GROUP).replace("on_unmatched: fail;", "");
    let ds = claim_diags(&src);
    assert_eq!(ds.len(), 1, "{:?} {:?}", ds, diags(&src));
    assert!(ds[0].contains("violated"), "{}", ds[0]);
    assert!(ds[0].contains("on_unmatched: swallow"), "{}", ds[0]);
}

#[test]
fn a_shedding_wildcard_subscriber_is_named_once() {
    let src = wild("pay.**", "or raise", GROUP)
        .replace("of type Pay;", "of type Pay bounded(4, drop_old);");
    let ds = claim_diags(&src);
    assert_eq!(ds.len(), 1, "{:?} {:?}", ds, diags(&src));
    assert!(ds[0].contains("violated"), "{}", ds[0]);
    assert_eq!(
        ds[0].matches("subscriber `Ledger::on_pay` on `pay.**`").count(),
        1,
        "{}",
        ds[0]
    );
}

#[test]
fn a_wildcard_route_with_every_boundary_non_lossy_holds() {
    assert_clean(&wild("pay.**", "or raise", GROUP));
}

// ---- computed publishers are route evidence the model lacks ---------

/// An ordinary publisher and subscriber of `Payments` on `pay.event`,
/// beside a called `Sender::go` that publishes to a COMPUTED subject
/// under its declared `PATTERN`. `SEND` is the ordinary send's `or`
/// clause.
const COMPUTED: &str = r#"
type Pay { id: Int = 0; }
topic Payments { payload: Pay; subject: "pay.event"; keyed_by id; on_unmatched: fail; }

locus Ledger {
    params { seen: Int = 0; }
    bus { subscribe Payments as on_pay; }
    fn on_pay(p: Pay) { self.seen = self.seen + 1; }
}

locus Sender {
    params { subject: String = "pay.event"; }
    bus { publish "PATTERN" of type Pay; }
    fn go() { self.subject <- Pay { id: 1 }; }
}

group route = { Ledger };

main locus App {
    params { l: Ledger = Ledger { }; s: Sender = Sender { }; }
    bus { publish Payments; }
    claims {
        loss: CLAIM;
    }
    run() {
        Payments <- Pay { id: 1 } SEND;
        self.s.go();
    }
}
fn main() { App { }; }
"#;

fn computed(pattern: &str, send: &str, claim: &str) -> String {
    COMPUTED
        .replace("PATTERN", pattern)
        .replace("SEND", send)
        .replace("CLAIM", claim)
}

#[test]
fn a_computed_publisher_under_a_covering_pattern_is_uncertified() {
    // The reviewer's control: the model records `Sender::go` as a
    // computed publish (a hole at the FUNCTION), which the subject-
    // and topic-grain hole index never meets — the claim held.
    for claim in [TOPIC, GROUP] {
        let src = computed("pay.**", "or raise", claim);
        let ds = claim_diags(&src);
        assert_eq!(ds.len(), 1, "{claim}: {:?} {:?}", ds, diags(&src));
        assert!(ds[0].contains("uncertified"), "{}", ds[0]);
        assert!(
            ds[0].contains(
                "`Sender::go` publishes to a subject computed at runtime \
                 under its declared `pay.**`"
            ),
            "{}",
            ds[0]
        );
    }
}

#[test]
fn a_silent_setting_beats_a_computed_publisher() {
    let src = computed("pay.**", "or discard", TOPIC);
    let ds = claim_diags(&src);
    assert_eq!(ds.len(), 1, "{:?} {:?}", ds, diags(&src));
    assert!(ds[0].contains("violated"), "{}", ds[0]);
    assert!(ds[0].contains("`or discard`"), "{}", ds[0]);
}

#[test]
fn a_computed_publisher_whose_pattern_misses_the_route_does_not_count() {
    // The declaration bounds the send (the publish site enforces it):
    // `audit.**` cannot reach `pay.event`.
    let src = computed("audit.**", "or raise", TOPIC)
        .replace("\"pay.event\"; }\n    bus { publish", "\"audit.x\"; }\n    bus { publish");
    assert_clean(&src);
}

#[test]
fn a_computed_publisher_on_a_wildcard_remainder_is_uncertified() {
    // `Ledger` subscribes `pay.**`; the computed send's pattern
    // `pay.other.**` covers no modeled wire, only the remainder of the
    // subscription, where the send's policy is unknown.
    let src = computed("pay.other.**", "or raise", GROUP)
        .replace("subscribe Payments as on_pay;", "subscribe \"pay.**\" as on_pay of type Pay;")
        .replace("\"pay.event\"; }\n    bus { publish", "\"pay.other.x\"; }\n    bus { publish");
    let ds = claim_diags(&src);
    assert_eq!(ds.len(), 1, "{:?} {:?}", ds, diags(&src));
    assert!(ds[0].contains("uncertified"), "{}", ds[0]);
    assert!(ds[0].contains("`Sender::go`"), "{}", ds[0]);
    assert!(ds[0].contains("`pay.**`"), "{}", ds[0]);
}

// ---- claim hygiene --------------------------------------------------

#[test]
fn an_undeclared_topic_is_an_error_not_an_empty_route() {
    let src = route("", "or raise", "require no_silent_loss(topic Nope)");
    let ds = claim_diags(&src);
    assert_eq!(ds.len(), 1, "{:?}", ds);
    assert!(ds[0].contains("never declared"), "{}", ds[0]);
}

#[test]
fn a_group_with_no_route_is_invalid_not_vacuously_true() {
    let src = r#"
type Pay { id: Int = 0; }
topic Payments { payload: Pay; subject: "pay"; }
locus Idle { params { n: Int = 0; } }
group quiet = { Idle };
main locus App {
    params { i: Idle = Idle { }; }
    bus { publish Payments; }
    claims { loss: require no_silent_loss(all quiet); }
    run() { Payments <- Pay { id: 1 }; }
}
fn main() { App { }; }
"#;
    let ds = claim_diags(src);
    assert_eq!(ds.len(), 1, "{:?}", ds);
    assert!(ds[0].contains("names no route"), "{}", ds[0]);
}

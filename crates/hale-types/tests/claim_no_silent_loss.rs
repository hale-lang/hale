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

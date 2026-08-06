//! Fleet claim shapes: the ways a claim can be well-formed and still
//! assert nothing, and the ways a plan can disagree with the
//! artifacts it names.
//!
//! `fleet_compose.rs` covers each verb doing its job. This file
//! covers the adversarial complement — a claim that parses, names a
//! real verb, and quietly holds against every possible fleet is worse
//! than no claim at all, because it reads like law in review.
//!
//! Two defects were found writing it, and both had the same shape:
//! the check existed and the degenerate case slipped past it.
//!
//!  - a `count_*_instances` naming no `eq`/`min`/`max` held against
//!    anything (every comparison defaults to true when its bound is
//!    absent), even though a claim naming no VERB was already an
//!    error;
//!  - a witness of three or more nodes labelled every hop with the
//!    LAST route, because the route labels were shifted the wrong
//!    way. Two-node witnesses came out right by accident, and every
//!    existing witness test was two-node.

use std::path::{Path, PathBuf};
use std::process::Command;

fn root(tag: &str) -> PathBuf {
    let d = std::env::temp_dir()
        .join(format!("hale_fcs_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn write(root: &Path, rel: &str, src: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
    std::fs::write(&p, src).expect("write");
}

fn hale(args: &[&str]) -> (String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(args)
        .output()
        .expect("run hale");
    (
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
        out.status.code().unwrap_or(-1),
    )
}

const TOPICS: &str = r#"
type Intent { id: Int; }
type Order  { id: Int; }
topic OrderIntent  { payload: Intent; subject: "svc.order.intent"; }
topic OrderRequest { payload: Order;  subject: "svc.order.request"; }
"#;

const PROBER: &str = r#"
import "../lib" as t;
locus Probe {
    params { n: Int = 0; }
    bus { publish t::OrderIntent; }
    fn submit() { let i = t::Intent { id: 1 }; t::OrderIntent <- i; }
}
main locus Prober { params { p: Probe = Probe { }; } }
fn main() { Prober { }; }
"#;

const OMS: &str = r#"
import "../lib" as t;
locus Oms {
    params { n: Int = 0; }
    bus { subscribe t::OrderIntent as on_intent; publish t::OrderRequest; }
    fn on_intent(i: t::Intent) {
        self.n = i.id;
        let o = t::Order { id: i.id };
        t::OrderRequest <- o;
    }
}
main locus OmsApp { params { o: Oms = Oms { }; } }
fn main() { OmsApp { }; }
"#;

const GW: &str = r#"
import "../lib" as t;
locus Gateway {
    params { n: Int = 0; }
    bus { subscribe t::OrderRequest as on_order; }
    fn on_order(o: t::Order) { self.n = o.id; }
}
main locus GwApp { params { g: Gateway = Gateway { }; } }
fn main() { GwApp { }; }
"#;

/// The two real routes: prober -[intent]-> oms -[request]-> gw.
const ROUTES: &str = r#"[
    {"id": "intent", "transport": "unix",
     "publishers":  [{"instance": "prober-0", "topic": "t::OrderIntent"}],
     "subscribers": [{"instance": "oms-0",    "topic": "t::OrderIntent"}]},
    {"id": "request", "transport": "unix",
     "publishers":  [{"instance": "oms-0", "topic": "t::OrderRequest"}],
     "subscribers": [{"instance": "gw-0",  "topic": "t::OrderRequest"}]}]"#;

const INSTANCES: &str = r#"[
    {"id": "prober-0", "artifact": "artifacts/prober.json", "labels": ["strategy"]},
    {"id": "oms-0",    "artifact": "artifacts/oms.json",    "labels": ["oms"]},
    {"id": "gw-0",     "artifact": "artifacts/gw.json",     "labels": ["gateway"]}]"#;

const GROUPS: &str = r#"{
    "strategy": {"labels": ["strategy"]},
    "oms":      {"labels": ["oms"]},
    "gateway":  {"labels": ["gateway"]},
    "all":      {"labels": ["strategy", "oms", "gateway"]}}"#;

/// Three artifacts built once per test.
fn fixture(tag: &str) -> PathBuf {
    let r = root(tag);
    write(&r, "lib/topics.hl", TOPICS);
    write(&r, "prober/main.hl", PROBER);
    write(&r, "oms/main.hl", OMS);
    write(&r, "gw/main.hl", GW);
    write(&r, "hale.toml", "[deps]\n");
    for app in ["prober", "oms", "gw"] {
        let dst = r.join(format!("artifacts/{}.json", app));
        std::fs::create_dir_all(dst.parent().expect("parent")).expect("mkdir");
        let (out, code) = hale(&[
            "check",
            r.join(app).to_str().expect("utf8"),
            &format!("--dump-topology={}", dst.display()),
        ]);
        assert_eq!(code, 0, "component `{}` must check clean: {}", app, out);
    }
    r
}

/// Write a plan with the standard instances/groups and the given
/// routes + claims, then check it.
fn check_with(
    r: &Path,
    routes: &str,
    claims: &str,
) -> (String, i32) {
    let plan = format!(
        r#"{{"schema": "1.0", "name": "prod",
            "instances": {INSTANCES},
            "routes": {routes},
            "groups": {GROUPS},
            "claims": {claims}}}"#
    );
    write(r, "plan.json", &plan);
    hale(&["fleet", "check", r.join("plan.json").to_str().expect("utf8")])
}

// ---------------------------------------------------------------
// Degenerate claim shapes
// ---------------------------------------------------------------

/// A cardinality claim with no bound compared nothing and held
/// against every fleet. It now gets the same answer as a claim naming
/// no verb: an error, not a pass.
#[test]
fn a_count_claim_with_no_bound_is_refused() {
    let r = fixture("nobound");
    for verb in ["count_publisher_instances", "count_subscriber_instances"] {
        let (out, code) = check_with(
            &r,
            ROUTES,
            &format!(
                r#"[{{"name": "vacuous",
                      "{verb}": {{"subject": "svc.order.request"}}}}]"#
            ),
        );
        assert_eq!(
            code, 1,
            "`{verb}` with no eq/min/max asserts nothing and must not \
             pass: {out}"
        );
        assert!(
            out.contains("names no bound"),
            "the diagnostic should say what is missing: {out}"
        );
    }
    let _ = std::fs::remove_dir_all(&r);
}

/// The bounded forms still work — the guard rejects emptiness, not
/// cardinality claims in general.
#[test]
fn every_bound_form_still_evaluates() {
    let r = fixture("bounds");
    // exactly one instance publishes svc.order.request (oms-0).
    for (bound, want_code) in [
        (r#""eq": 1"#, 0),
        (r#""eq": 2"#, 1),
        (r#""min": 1"#, 0),
        (r#""min": 2"#, 1),
        (r#""max": 1"#, 0),
        (r#""max": 0"#, 1),
        (r#""min": 1, "max": 1"#, 0),
    ] {
        let (out, code) = check_with(
            &r,
            ROUTES,
            &format!(
                r#"[{{"name": "card",
                      "count_publisher_instances":
                        {{"subject": "svc.order.request", {bound}}}}}]"#
            ),
        );
        assert_eq!(code, want_code, "bound `{bound}`: {out}");
    }
    let _ = std::fs::remove_dir_all(&r);
}

/// A claim naming several verbs is refused, because only one of them
/// would ever be judged.
///
/// This test replaces one that asserted the opposite — that every
/// verb is evaluated "whatever the order" — and passed while the bug
/// was live. Two things were wrong with it. JSON field order cannot
/// affect anything: serde fills a struct, and precedence is fixed by
/// the `if/else` chain in the evaluator, so the two "orders" it tried
/// were the same case twice. And it paired a violating
/// `require_subscribes` with a holding `count`, where the violating
/// verb happens to come FIRST in that chain — so the claim failed for
/// a reason the test was not measuring, and a green result meant
/// nothing.
///
/// The case that mattered is the reverse: an earlier verb that holds
/// hiding a later one that cannot. Below, `require_subscribes` is
/// satisfied and `count ... eq: 999` is impossible against a
/// one-publisher fleet, and the whole claim used to pass.
#[test]
fn a_claim_naming_several_verbs_is_refused() {
    let r = fixture("multiverb");
    let (out, code) = check_with(
        &r,
        ROUTES,
        r#"[{"name": "both",
             "require_subscribes": {"group": "gateway",
                                    "subject": "svc.order.request"},
             "count_publisher_instances": {"subject": "svc.order.request",
                                           "eq": 999}}]"#,
    );
    assert_eq!(
        code, 1,
        "an impossible count must not be ignored because an earlier \
         verb in the same claim holds: {out}"
    );
    assert!(
        out.contains("names 2 verbs"),
        "the diagnostic should say the claim is ambiguous rather than \
         silently judging one verb: {out}"
    );

    // Split into two claims, each with its own name, and the
    // impossible one is now visible on its own terms.
    let (out, code) = check_with(
        &r,
        ROUTES,
        r#"[{"name": "fed", "require_subscribes":
               {"group": "gateway", "subject": "svc.order.request"}},
            {"name": "impossible", "count_publisher_instances":
               {"subject": "svc.order.request", "eq": 999}}]"#,
    );
    assert_eq!(code, 1, "the impossible count must fail: {out}");
    assert!(
        out.contains("impossible"),
        "the failing claim's own name should be reported: {out}"
    );
    let _ = std::fs::remove_dir_all(&r);
}

/// A prohibition whose source and target overlap is violated by the
/// overlap itself: an instance in both groups already sits in the
/// forbidden set without going anywhere.
///
/// The BFS cannot see this — it refuses to accept a destination that
/// is also a source — so `forbid_reaches(g, g)` reported `holds`
/// while every path in the fleet ran inside the prohibition. The
/// application tier calls this a zero-length violation; so does this.
#[test]
fn an_overlapping_prohibition_is_violated_not_vacuous() {
    let r = fixture("overlap");
    for (from, to) in [("all", "all"), ("all", "oms")] {
        let (out, code) = check_with(
            &r,
            ROUTES,
            &format!(
                r#"[{{"name": "ovl", "forbid_reaches":
                     {{"from": "{from}", "to": "{to}"}}}}]"#
            ),
        );
        assert_eq!(
            code, 1,
            "`forbid_reaches({from}, {to})` overlaps and must not \
             hold: {out}"
        );
        assert!(
            out.contains("zero-length"),
            "the witness should name the overlap as the violation: \
             {out}"
        );
    }
    let _ = std::fs::remove_dir_all(&r);
}

/// `avoiding` may not name an endpoint of its own claim. Masking one
/// deletes what the claim quantifies over, and an empty domain makes
/// any prohibition hold.
#[test]
fn avoiding_may_not_mask_an_endpoint() {
    let r = fixture("mask");
    for group in ["strategy", "gateway"] {
        let (out, code) = check_with(
            &r,
            ROUTES,
            &format!(
                r#"[{{"name": "av", "forbid_reaches":
                     {{"from": "strategy", "to": "gateway",
                       "avoiding": "{group}"}}}}]"#
            ),
        );
        assert_eq!(
            code, 1,
            "masking the `{group}` endpoint must be refused: {out}"
        );
        assert!(out.contains("already an endpoint"), "{out}");
    }

    // The legitimate interposition form — masking the gate in the
    // middle — still works, so this is not simply always-failing.
    let (out, code) = check_with(
        &r,
        ROUTES,
        r#"[{"name": "ok", "forbid_reaches":
             {"from": "strategy", "to": "gateway",
              "avoiding": "oms"}}]"#,
    );
    assert_eq!(
        code, 0,
        "every strategy->gateway path goes through oms: {out}"
    );
    let _ = std::fs::remove_dir_all(&r);
}

/// `min` above `max` can never be satisfied. It fails rather than
/// passing, which is the safe direction for a contradiction.
#[test]
fn a_contradictory_bound_fails_closed() {
    let r = fixture("contradiction");
    let (out, code) = check_with(
        &r,
        ROUTES,
        r#"[{"name": "impossible", "count_publisher_instances":
             {"subject": "svc.order.request", "min": 5, "max": 2}}]"#,
    );
    assert_eq!(code, 1, "an unsatisfiable bound must not hold: {out}");
    let _ = std::fs::remove_dir_all(&r);
}

/// `only_edges` granting nothing seals the boundary completely. An
/// empty grant list must mean "nothing may cross", not "no grants
/// configured, so allow everything".
#[test]
fn only_edges_with_no_grants_seals_the_boundary() {
    let r = fixture("sealed");
    let (out, code) = check_with(
        &r,
        ROUTES,
        r#"[{"name": "sealed",
             "only_edges": {"from": "strategy", "to": "oms",
                            "grant_subjects": []}}]"#,
    );
    assert_eq!(code, 1, "an empty grant list must forbid every edge: {out}");
    assert!(out.contains("svc.order.intent"), "{out}");
    let _ = std::fs::remove_dir_all(&r);
}

// ---------------------------------------------------------------
// Witnesses
// ---------------------------------------------------------------

/// A witness crossing three artifacts must name the route that
/// carries EACH hop.
///
/// The labels used to be shifted the wrong way, so both hops were
/// reported as `request` — pointing a reader at the wrong route
/// entry, which for a real deployment means the wrong config file.
/// Two-node witnesses were right by accident, so this needs the
/// three-node path to catch it.
#[test]
fn a_multi_hop_witness_names_the_route_of_each_hop() {
    let r = fixture("witness");
    let (out, code) = check_with(
        &r,
        ROUTES,
        r#"[{"name": "no_reach",
             "forbid_reaches": {"from": "strategy", "to": "gateway"}}]"#,
    );
    assert_eq!(code, 1, "strategy does reach gateway: {out}");

    let intent_at = out.find("route `intent`");
    let request_at = out.find("route `request`");
    assert!(
        intent_at.is_some(),
        "the prober -> oms hop rides route `intent`, and the witness \
         never mentions it:\n{out}"
    );
    assert!(
        request_at.is_some(),
        "the oms -> gw hop rides route `request`:\n{out}"
    );
    assert!(
        intent_at < request_at,
        "the hops must be labelled in path order:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&r);
}

// ---------------------------------------------------------------
// Plan vs. artifact disagreement
// ---------------------------------------------------------------

/// A route naming a topic the instance's artifact does not declare is
/// a plan error. Otherwise a typo silently drops the edge, and a
/// `forbid_reaches` law then "holds" because the path it was written
/// to catch was never modelled.
#[test]
fn a_route_naming_an_undeclared_topic_is_refused() {
    let r = fixture("typo");
    let routes = r#"[
        {"id": "intent", "transport": "unix",
         "publishers":  [{"instance": "prober-0", "topic": "t::OrderIntnet"}],
         "subscribers": [{"instance": "oms-0",    "topic": "t::OrderIntnet"}]}]"#;
    let (out, code) = check_with(
        &r,
        routes,
        r#"[{"name": "no_reach",
             "forbid_reaches": {"from": "strategy", "to": "gateway"}}]"#,
    );
    assert_eq!(code, 1, "a misspelled topic must not pass silently: {out}");
    assert!(
        out.contains("declares no topic"),
        "the error should name the undeclared topic, not report the \
         claim as holding: {out}"
    );
    let _ = std::fs::remove_dir_all(&r);
}

/// The plan may not vouch for behavior the code does not have.
///
/// `prober` imports the topic module, so `t::OrderRequest` IS in its
/// artifact's topic table and the old admission check — "does the
/// instance declare this topic?" — accepted it as a publisher. But
/// prober never publishes it, so the route had a phantom producer,
/// and a `require_subscribes` law about the consumer side could hold
/// with nothing feeding it.
///
/// Role validity is now an ADMISSION check: the plan is refused
/// before any claim is evaluated. That is the right stage — a plan
/// that misdescribes its components is not a law failure, it is an
/// invalid model, and the two should not be reported as if a claim
/// had been judged.
#[test]
fn a_route_endpoint_must_hold_the_role_the_plan_gives_it() {
    let r = fixture("planlies");
    let routes = r#"[
        {"id": "bogus", "transport": "unix",
         "publishers":  [{"instance": "prober-0", "topic": "t::OrderRequest"}],
         "subscribers": [{"instance": "gw-0",     "topic": "t::OrderRequest"}]}]"#;
    let (out, code) = check_with(
        &r,
        routes,
        r#"[{"name": "gateway_is_fed",
             "require_subscribes": {"group": "gateway",
                                    "subject": "svc.order.request"}}]"#,
    );
    assert_eq!(
        code, 1,
        "nothing in `prober` publishes svc.order.request, so this \
         route has no producer: {out}"
    );
    assert!(
        out.contains("named as a publisher")
            && out.contains("prober-0"),
        "the diagnostic should name the route, the instance and the \
         missing role: {out}"
    );

    // The subscriber side is checked the same way: gw-0 does not
    // subscribe `svc.order.intent`.
    let (out, code) = check_with(
        &r,
        r#"[
        {"id": "wrongsub", "transport": "unix",
         "publishers":  [{"instance": "prober-0", "topic": "t::OrderIntent"}],
         "subscribers": [{"instance": "gw-0",     "topic": "t::OrderIntent"}]}]"#,
        "[]",
    );
    assert_eq!(code, 1, "gw-0 does not subscribe that subject: {out}");
    assert!(
        out.contains("named as a subscriber"),
        "the missing role should be named: {out}"
    );

    // Control: a route whose both ends really hold their roles is
    // admitted, so the check is not simply always-failing.
    let (out, code) = check_with(
        &r,
        r#"[
        {"id": "request", "transport": "unix",
         "publishers":  [{"instance": "oms-0", "topic": "t::OrderRequest"}],
         "subscribers": [{"instance": "gw-0",  "topic": "t::OrderRequest"}]}]"#,
        r#"[{"name": "oms_publishes",
             "require_publishes": {"group": "oms",
                                   "subject": "svc.order.request"}}]"#,
    );
    assert_eq!(code, 0, "oms does publish it: {out}");
    let _ = std::fs::remove_dir_all(&r);
}

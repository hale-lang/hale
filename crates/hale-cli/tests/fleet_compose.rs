//! GH #408: composing a fleet's instances into one model, from a fleet
//! seed (`fleet locus`, phase 4).
//!
//! The composition unit is the INSTANCE's artifact, cut from its seed,
//! never merged source. Matching wire identities establish
//! compatibility; only a binding creates a fleet edge. A source-merged
//! "super-main" would invent local bus edges between binaries no route
//! connects and turn cross-process messaging into call reachability —
//! so these tests care as much about what is NOT connected as what is.

#[path = "support/fleet_seed.rs"]
mod fleet_seed;

use fleet_seed::{fleet, hale_in, root, stdout_in, write};
use std::path::{Path, PathBuf};

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

/// A prober that also sends orders straight to the gateway.
fn rogue() -> String {
    PROBER
        .replace(
            "bus { publish t::OrderIntent; }",
            "bus { publish t::OrderIntent; publish t::OrderRequest; }",
        )
        .replace(
            "fn submit() { let i = t::Intent { id: 1 }; t::OrderIntent <- i; }",
            "fn submit() { let i = t::Intent { id: 1 }; t::OrderIntent <- i; \
             let o = t::Order { id: 1 }; t::OrderRequest <- o; }",
        )
}

const IMPORTS: &[(&str, &str)] = &[
    ("pa", "../prober"),
    ("oa", "../oms"),
    ("ga", "../gw"),
    ("t", "../lib"),
];
const PARAMS: &[&str] = &["prober: pa::Prober;", "oms: oa::OmsApp;", "gw: ga::GwApp;"];
const BOTH: &[&str] = &[
    "t::OrderIntent: unix(\"/tmp/hale-fleet-intent.sock\");",
    "t::OrderRequest: unix(\"/tmp/hale-fleet-request.sock\");",
];
const CLAIMS: &[&str] = &[
    "orders_pass_oms: forbid reaches(prober, gw) avoiding oms;",
    "one_order_authority: count publishers(topic t::OrderRequest) == 1;",
    "gw_receives_orders: require subscribes(some gw, topic t::OrderRequest);",
];

/// Three seeds and a fleet seed `prod/` over their mains.
fn workspace(tag: &str) -> PathBuf {
    let r = root(tag);
    write(&r, "lib/topics.hl", TOPICS);
    write(&r, "prober/main.hl", PROBER);
    write(&r, "oms/main.hl", OMS);
    write(&r, "gw/main.hl", GW);
    write(&r, "hale.toml", "[deps]\n");
    write(&r, "prod/fleet.hl", &fleet(IMPORTS, PARAMS, &[], BOTH, &[]));
    r
}

/// A fleet seed `<name>/` beside the others.
fn seed(r: &Path, name: &str, placement: &[&str], bindings: &[&str], claims: &[&str]) {
    write(
        r,
        &format!("{name}/fleet.hl"),
        &fleet(IMPORTS, PARAMS, placement, bindings, claims),
    );
}

fn dump(r: &Path, seed: &str) -> serde_json::Value {
    let out = stdout_in(r, &["fleet", "dump", seed]);
    serde_json::from_str(&out).unwrap_or_else(|e| panic!("fleet artifact parses ({e}): {out}"))
}

fn routed(v: &serde_json::Value) -> Vec<(String, String, String)> {
    v["relations"]["routed"]
        .as_array()
        .expect("routed")
        .iter()
        .map(|e| {
            (
                e["from"].as_str().unwrap_or("").to_string(),
                e["to"].as_str().unwrap_or("").to_string(),
                e["route"].as_str().unwrap_or("").to_string(),
            )
        })
        .collect()
}

/// Three independently compiled seeds compose, instance-qualified, and
/// the fleet's two bindings derive a route each: A → route → B → route →
/// C.
#[test]
fn three_seeds_compose_into_a_routed_chain() {
    let r = workspace("chain");
    let v = dump(&r, "prod");
    let _ = std::fs::remove_dir_all(&r);
    let e = routed(&v);
    assert!(
        e.contains(&(
            "prober-0::Probe::submit".into(),
            "oms-0::Oms::on_intent".into(),
            "t::OrderIntent".into()
        )),
        "A → B: {e:?}"
    );
    assert!(
        e.contains(&(
            "oms-0::Oms::on_intent".into(),
            "gw-0::Gateway::on_order".into(),
            "t::OrderRequest".into()
        )),
        "B → C: {e:?}"
    );
    for vtx in v["vertices"].as_array().expect("vertices") {
        let s = vtx.as_str().unwrap_or("");
        assert!(
            s.starts_with("prober-0::") || s.starts_with("oms-0::") || s.starts_with("gw-0::"),
            "unqualified vertex `{s}`"
        );
    }
}

/// Matching wire identities are COMPATIBILITY, not connection: with no
/// binding, nothing is routed though every instance knows the topics.
#[test]
fn matching_topics_alone_do_not_create_an_edge() {
    let r = workspace("noroute");
    seed(&r, "bare", &[], &[], &[]);
    let v = dump(&r, "bare");
    let _ = std::fs::remove_dir_all(&r);
    assert!(
        routed(&v).is_empty(),
        "only a binding creates a fleet edge: {v}"
    );
}

/// The join key is the WIRE identity: a shared subject over a different
/// payload shape cannot form a route.
#[test]
fn a_same_subject_different_payload_route_is_rejected() {
    let r = workspace("wire");
    write(
        &r,
        "lib2/topics.hl",
        &TOPICS.replace(
            "type Order  { id: Int; }",
            "type Order  { id: Int; extra: Int; }",
        ),
    );
    write(&r, "gw2/main.hl", &GW.replace("\"../lib\"", "\"../lib2\""));
    let mut imports = IMPORTS.to_vec();
    imports.push(("g2", "../gw2"));
    let mut params = PARAMS.to_vec();
    params.push("gw2: g2::GwApp;");
    write(
        &r,
        "wire/fleet.hl",
        &fleet(&imports, &params, &[], BOTH, &[]),
    );
    let (out, code) = hale_in(&r, &["fleet", "check", "wire"]);
    let _ = std::fs::remove_dir_all(&r);
    assert_ne!(code, 0, "incompatible payloads must fail: {out}");
    assert!(
        out.contains("cannot be formed") && out.contains("WIRE identity"),
        "{out}"
    );
}

/// A downstream handoff, the cross-tier lock: a component carrying a
/// `@budget` contract is admissible to the fleet its own check passes.
#[test]
fn a_component_with_a_budget_contract_is_admissible() {
    let r = workspace("budget");
    write(
        &r,
        "gw/main.hl",
        r#"
import "../lib" as t;
locus Gateway {
    params { n: Int = 0; }
    bus { subscribe t::OrderRequest as on_order; }
    @budget(alloc_per_call = 0)
    fn score(o: t::Order) -> Int { return o.id + 1; }
    fn on_order(o: t::Order) { self.n = self.score(o); }
}
main locus GwApp { params { g: Gateway = Gateway { }; } }
fn main() { GwApp { }; }
"#,
    );
    let (out, code) = hale_in(&r, &["fleet", "check", "prod"]);
    let art =
        std::fs::read_to_string(r.join(".hale/fleet/prod/gw.topology.json")).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&r);
    assert!(
        art.contains("\"family\": \"budget\""),
        "test premise: the artifact carries a budget law row"
    );
    assert_eq!(
        code, 0,
        "the deployment tier admits what the application tier passed: {out}"
    );
}

/// Local law is a precondition of admission: a seed whose own claim
/// fails has no artifact to compose.
#[test]
fn a_component_whose_own_claims_fail_is_refused() {
    let r = workspace("verdict");
    write(
        &r,
        "prober/main.hl",
        &PROBER.replace(
            "main locus Prober { params { p: Probe = Probe { }; } }",
            "main locus Prober {\n    params { p: Probe = Probe { }; }\n    \
             claims { quiet: count publishers(topic t::OrderIntent) == 0; }\n}",
        ),
    );
    let (out, code) = hale_in(&r, &["fleet", "check", "prod"]);
    let _ = std::fs::remove_dir_all(&r);
    assert_ne!(code, 0, "{out}");
    assert!(
        out.contains("does not check, so it has no artifact"),
        "{out}"
    );
}

/// The fleet identity moves when the deployed arrangement moves, and
/// stays put when only source locations do.
#[test]
fn the_fleet_shape_hash_tracks_the_arrangement_not_provenance() {
    let r = workspace("hash");
    let hash_of = |seed: &str| {
        dump(&r, seed)["fleet_shape_hash"]
            .as_str()
            .unwrap_or("")
            .to_string()
    };
    let base = hash_of("prod");
    assert!(!base.is_empty());
    // provenance only: a leading comment shifts every span of a component
    write(
        &r,
        "prober/main.hl",
        &format!("// shift every span\n{PROBER}"),
    );
    assert_eq!(
        hash_of("prod"),
        base,
        "moving source must not change the deployed arrangement"
    );
    // a transport is the arrangement
    seed(
        &r,
        "ring",
        &[],
        &[
            BOTH[0],
            "t::OrderRequest: shm_ring(\"/orders\", slot_count: 8, on_overflow: block);",
        ],
        &[],
    );
    assert_ne!(
        hash_of("ring"),
        base,
        "a changed transport is a changed fleet"
    );
    // and so is instance cardinality
    seed(&r, "two", &["gw: replicas(2);"], BOTH, &[]);
    assert_ne!(
        hash_of("two"),
        base,
        "a second instance of one seed is a different deployment"
    );
    let _ = std::fs::remove_dir_all(&r);
}

fn results(v: &serde_json::Value) -> Vec<(String, String)> {
    v["claims"]
        .as_array()
        .expect("claims")
        .iter()
        .map(|c| {
            (
                c["name"].as_str().unwrap_or("").to_string(),
                c["result"].as_str().unwrap_or("").to_string(),
            )
        })
        .collect()
}

#[test]
fn fleet_claims_hold_on_a_compliant_fleet() {
    let r = workspace("claims_ok");
    seed(&r, "lawful", &[], BOTH, CLAIMS);
    let v = dump(&r, "lawful");
    let _ = std::fs::remove_dir_all(&r);
    let res = results(&v);
    assert_eq!(res.len(), 3, "{res:?}");
    for (name, verdict) in &res {
        assert_eq!(verdict, "holds", "claim `{name}`: {res:?}");
    }
}

/// A prober that also publishes the gateway's topic joins the derived
/// route, and that path avoids the OMS.
#[test]
fn a_bypass_route_violates_the_interposition_claim() {
    let r = workspace("bypass");
    write(&r, "prober/main.hl", &rogue());
    seed(&r, "lawful", &[], BOTH, CLAIMS);
    let (out, code) = hale_in(&r, &["fleet", "check", "lawful"]);
    let _ = std::fs::remove_dir_all(&r);
    assert_ne!(code, 0, "a bypass must fail the fleet: {out}");
    assert!(out.contains("orders_pass_oms"), "{out}");
    assert!(
        out.contains("prober-0::Probe::submit") && out.contains("gw-0::Gateway::on_order"),
        "witness names both ends: {out}"
    );
    assert!(
        out.contains("route `t::OrderRequest`"),
        "and the route, since the hop is not a call: {out}"
    );
    assert!(
        out.contains("prober/main.hl") && out.contains("gw/main.hl"),
        "and the file each vertex lives in: {out}"
    );
}

#[test]
fn a_second_deployed_publisher_violates_the_cardinality_claim() {
    let r = workspace("cardinality");
    seed(&r, "lawful", &["oms: replicas(2);"], BOTH, CLAIMS);
    let (out, code) = hale_in(&r, &["fleet", "check", "lawful"]);
    let _ = std::fs::remove_dir_all(&r);
    assert_ne!(code, 0, "{out}");
    assert!(
        out.contains("one_order_authority") && out.contains("counted 2 deployed publisher"),
        "{out}"
    );
}

/// The existence claim is about traffic: a subscriber nothing routes to
/// does not satisfy it.
#[test]
fn require_subscribes_needs_a_route_not_just_an_endpoint() {
    let r = workspace("unrouted");
    seed(&r, "unrouted", &[], &[BOTH[0]], &[CLAIMS[2]]);
    let (out, code) = hale_in(&r, &["fleet", "check", "unrouted"]);
    assert_ne!(
        code, 0,
        "the endpoint exists but nothing routes to it: {out}"
    );
    assert!(
        out.contains("gw_receives_orders") && out.contains("no route in this plan carries it"),
        "{out}"
    );
    seed(&r, "routed", &[], BOTH, &[CLAIMS[2]]);
    let (out, code) = hale_in(&r, &["fleet", "check", "routed"]);
    let _ = std::fs::remove_dir_all(&r);
    assert_eq!(code, 0, "the routed fleet passes: {out}");
}

/// An instance in both groups is a zero-length violation; `avoiding`
/// may not mask an endpoint; a sealed boundary refuses a routed edge it
/// does not grant; a path over two routes names each.
#[test]
fn the_reachability_forms_judge_the_fleet() {
    let r = workspace("forms");
    seed(
        &r,
        "same",
        &[],
        BOTH,
        &["self_reach: forbid reaches(gw, gw);"],
    );
    let (out, code) = hale_in(&r, &["fleet", "check", "same"]);
    assert!(
        code != 0 && out.contains("self_reach"),
        "the source already is the destination: {out}"
    );
    seed(
        &r,
        "masked",
        &[],
        BOTH,
        &["masked: forbid reaches(prober, gw) avoiding gw;"],
    );
    let (out, code) = hale_in(&r, &["fleet", "check", "masked"]);
    assert!(
        code != 0 && out.contains("masked"),
        "avoiding an endpoint deletes the domain: {out}"
    );
    seed(
        &r,
        "sealed",
        &[],
        BOTH,
        &["sealed: only edges prober -> oms { };"],
    );
    let (out, code) = hale_in(&r, &["fleet", "check", "sealed"]);
    assert!(
        code != 0 && out.contains("sealed"),
        "an ungranted routed edge crosses the boundary: {out}"
    );
    seed(
        &r,
        "granted",
        &[],
        BOTH,
        &["granted: only edges prober -> oms { publish t::OrderIntent; };"],
    );
    let (out, code) = hale_in(&r, &["fleet", "check", "granted"]);
    assert_eq!(code, 0, "the granted subject crosses: {out}");
    seed(
        &r,
        "hops",
        &[],
        BOTH,
        &["no_orders: forbid reaches(prober, gw);"],
    );
    let (out, code) = hale_in(&r, &["fleet", "check", "hops"]);
    let _ = std::fs::remove_dir_all(&r);
    assert_ne!(code, 0, "{out}");
    assert!(
        out.contains("route `t::OrderIntent`") && out.contains("route `t::OrderRequest`"),
        "{out}"
    );
}

/// Every bound of a count evaluates over deployed instances.
#[test]
fn every_count_bound_evaluates() {
    let r = workspace("bounds");
    seed(
        &r,
        "bounds",
        &["gw: replicas(3);"],
        BOTH,
        &[
            "exactly: count subscribers(topic t::OrderRequest) == 3;",
            "at_most: count subscribers(topic t::OrderRequest) <= 2;",
            "at_least: count subscribers(topic t::OrderRequest) >= 3;",
        ],
    );
    let (out, code) = hale_in(&r, &["fleet", "check", "bounds"]);
    let _ = std::fs::remove_dir_all(&r);
    assert_ne!(code, 0, "{out}");
    assert!(out.contains("fleet claim `at_most` violated"), "{out}");
    assert!(
        !out.contains("`exactly` violated") && !out.contains("`at_least` violated"),
        "{out}"
    );
}

/// Route identity is the topic grain: a component that sends the wire
/// LITERALLY, never the topic, publishes nothing a binding can route.
#[test]
fn literal_only_publisher_cannot_satisfy_a_topic_route() {
    let r = workspace("literalroute");
    write(
        &r,
        "prober/main.hl",
        r#"
import "../lib" as t;
locus Probe {
    params { n: Int = 0; }
    bus { publish "svc.order.intent" of type t::Intent; }
    fn submit() {
        let i = t::Intent { id: 1 };
        "svc.order.intent" <- i;
    }
}
main locus Prober { params { p: Probe = Probe { }; } }
fn main() { Prober { }; }
"#,
    );
    let (out, code) = hale_in(&r, &["fleet", "check", "prod"]);
    let _ = std::fs::remove_dir_all(&r);
    assert_ne!(code, 0, "{out}");
    assert!(
        out.contains("binding `t::OrderIntent` wires nothing"),
        "{out}"
    );
}

#[test]
fn fleet_check_with_no_seed_checks_every_declared_deployment() {
    let r = workspace("allfleets");
    seed(&r, "staging", &["gw: replicas(2);"], BOTH, &[]);
    write(
        &r,
        "hale.toml",
        "[deps]\n\n[fleets]\nproduction = \"prod\"\nstaging = \"staging\"\n",
    );
    let (out, code) = hale_in(&r, &["fleet", "check"]);
    let _ = std::fs::remove_dir_all(&r);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("2 fleet(s) checked"), "{out}");
    assert!(
        out.contains("fleet `production`") && out.contains("fleet `staging`"),
        "{out}"
    );
}

#[test]
fn a_missing_seed_fails_without_hiding_the_rest() {
    let r = workspace("missing");
    write(
        &r,
        "hale.toml",
        "[deps]\n\n[fleets]\nproduction = \"prod\"\nghost = \"nope\"\n",
    );
    let (out, code) = hale_in(&r, &["fleet", "check"]);
    let _ = std::fs::remove_dir_all(&r);
    assert_eq!(code, 2, "a missing seed is a usage error: {out}");
    assert!(out.contains("fleet seed not found"), "{out}");
    assert!(
        out.contains("fleet `production`"),
        "the healthy deployment was still checked: {out}"
    );
}

#[test]
fn a_workspace_with_no_declared_fleets_is_a_usage_error() {
    let r = workspace("nofleets");
    let (out, code) = hale_in(&r, &["fleet", "check"]);
    let _ = std::fs::remove_dir_all(&r);
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("declares no `[fleets]`"), "{out}");
}

/// A plan JSON is no input: not on the command line, not in `[fleets]`.
#[test]
fn a_plan_is_not_an_input() {
    let r = workspace("noplan");
    write(&r, "prod.plan.json", "{}");
    let (out, code) = hale_in(&r, &["fleet", "check", "prod.plan.json"]);
    assert_eq!(code, 2, "{out}");
    assert!(
        out.contains("not a fleet seed") && out.contains("hale fleet export"),
        "{out}"
    );
    write(
        &r,
        "hale.toml",
        "[deps]\n\n[fleets]\nproduction = \"prod.plan.json\"\n",
    );
    let (out, code) = hale_in(&r, &["fleet", "check"]);
    let _ = std::fs::remove_dir_all(&r);
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("names a plan"), "{out}");
}

/// `[fleets]` and `[environments]` are separate axes: a workspace
/// declares both, and each check runs its own.
#[test]
fn fleets_and_environments_are_independent_axes() {
    let r = workspace("axes");
    write(
        &r,
        "hale.toml",
        "[claims]\nno_base = true\n\n\
         [environments.production]\nsource_only = true\n\
         entrypoints = [\"prober\", \"oms\", \"gw\"]\n\n\
         [fleets]\nproduction = \"prod\"\n",
    );
    let (out, code) = hale_in(&r, &["fleet", "check"]);
    assert_eq!(code, 0, "fleet axis: {out}");
    assert!(out.contains("1 fleet(s) checked"), "{out}");
    let (out, code) = hale_in(&r, &["check", "--matrix"]);
    let _ = std::fs::remove_dir_all(&r);
    assert_eq!(code, 0, "entrypoint axis: {out}");
}

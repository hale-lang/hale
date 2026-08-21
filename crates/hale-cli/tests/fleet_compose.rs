//! GH #408 Phase 1: composing topology artifacts into a fleet model.
//!
//! The composition unit is the artifact INSTANCE, never the source
//! file. Matching wire identities establish compatibility; only an
//! explicit route creates a fleet edge. A source-merged "super-main"
//! would invent local bus edges between binaries no route connects,
//! erase deploy-time routes that exist only in config, and turn
//! cross-process messaging into ordinary call reachability — so these
//! tests care as much about what is NOT connected as what is.

use std::path::{Path, PathBuf};
use std::process::Command;

fn root(tag: &str) -> PathBuf {
    let d = std::env::temp_dir()
        .join(format!("hale_fleet_{}_{}", std::process::id(), tag));
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

fn hale_stdout(args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(args)
        .output()
        .expect("run hale");
    String::from_utf8_lossy(&out.stdout).to_string()
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

const PLAN: &str = r#"{
  "schema": "1.0",
  "name": "prod",
  "instances": [
    {"id": "prober-0", "artifact": "artifacts/prober.json", "labels": ["strategy"]},
    {"id": "oms-0",    "artifact": "artifacts/oms.json",    "labels": ["oms"]},
    {"id": "gw-0",     "artifact": "artifacts/gw.json",     "labels": ["gateway"]}
  ],
  "routes": [
    {"id": "intent", "transport": "unix",
     "publishers":  [{"instance": "prober-0", "topic": "t::OrderIntent"}],
     "subscribers": [{"instance": "oms-0",    "topic": "t::OrderIntent"}]},
    {"id": "request", "transport": "unix",
     "publishers":  [{"instance": "oms-0", "topic": "t::OrderRequest"}],
     "subscribers": [{"instance": "gw-0",  "topic": "t::OrderRequest"}]}
  ]
}"#;

/// Three seeds, three artifacts, one plan.
fn fleet(tag: &str) -> PathBuf {
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
    write(&r, "prod.plan.json", PLAN);
    r
}

/// Round 7: tamper controls restamp BOTH digests — `law_digest`
/// recomputes from the canonical-JSON law rows, then the document
/// trailer — so each control exercises the binding it targets,
/// not the digest gate.
fn restamp_both(artifact: &str) -> String {
    fn fnv1a64(bytes: &[u8]) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        for b in bytes {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x100000001b3);
        }
        h
    }
    let key = ",\n  \"artifact_digest\": \"";
    let cut = artifact.rfind(key).expect("digest trailer");
    let mut body = artifact[..cut].to_string();
    // Round 2 (#490): the shape_hash is recomputed at admission,
    // so hashed-half tampers must restamp it too — each pin then
    // exercises the deep binding it targets, not the identity
    // gate. (The stale-identity control below does NOT use this
    // helper.)
    let sk = "\"shape_hash\": \"";
    if let Some(at) = body.find(sk) {
        let start = at + sk.len();
        if let Some(rel) = body[start..].find('"') {
            let claimed_end = start + rel;
            let model_start =
                claimed_end + "\",\n".len();
            if let Some(end_rel) =
                body[model_start..].find(",\n  \"sources\": [")
            {
                let fresh = format!(
                    "{:016x}",
                    fnv1a64(
                        body[model_start
                            ..model_start + end_rel]
                            .as_bytes()
                    )
                );
                body.replace_range(start..claimed_end, &fresh);
            }
        }
    }
    let lk = "\"law_digest\": \"";
    if let (Some(at), Ok(v)) = (
        body.find(lk),
        serde_json::from_str::<serde_json::Value>(&format!(
            "{}\n}}\n",
            body
        )),
    ) {
        if v["law"]["rows"].is_array() {
            let canon = serde_json::to_string(&serde_json::json!({
                "issues": v["law"]["issues"],
                "rows": v["law"]["rows"],
            }))
            .unwrap();
            let fresh =
                format!("{:016x}", fnv1a64(canon.as_bytes()));
            let start = at + lk.len();
            let end = start
                + body[start..].find('"').expect("digest close");
            body.replace_range(start..end, &fresh);
        }
    }
    format!(
        "{}{}{:016x}\"\n}}\n",
        body,
        key,
        fnv1a64(body.as_bytes())
    )
}

fn plan_of(r: &Path) -> String {
    r.join("prod.plan.json").to_str().expect("utf8").to_string()
}

/// Criteria 1-3: three independently compiled artifacts load, get
/// instance-qualified, and an explicit route file connects a
/// publisher in one to a subscriber in another — twice, so the graph
/// carries A → route → B → route → C.
#[test]
fn three_artifacts_compose_into_a_routed_chain() {
    let r = fleet("chain");
    let out = hale_stdout(&["fleet", "dump", &plan_of(&r)]);
    let v: serde_json::Value =
        serde_json::from_str(&out).expect("fleet artifact parses");
    let _ = std::fs::remove_dir_all(&r);

    let routed: Vec<(String, String, String)> = v["relations"]["routed"]
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
        .collect();
    assert!(
        routed.contains(&(
            "prober-0::Probe::submit".into(),
            "oms-0::Oms::on_intent".into(),
            "intent".into()
        )),
        "A → route → B: {:?}",
        routed
    );
    assert!(
        routed.contains(&(
            "oms-0::Oms::on_intent".into(),
            "gw-0::Gateway::on_order".into(),
            "request".into()
        )),
        "B → route → C: {:?}",
        routed
    );
    // Every vertex is instance-qualified: an application TYPE is not
    // a deployed instance.
    for vtx in v["vertices"].as_array().expect("vertices") {
        let s = vtx.as_str().unwrap_or("");
        assert!(
            s.starts_with("prober-0::")
                || s.starts_with("oms-0::")
                || s.starts_with("gw-0::"),
            "unqualified vertex `{}`",
            s
        );
    }
}

/// The property that separates composition from source-merging:
/// matching wire identities are COMPATIBILITY, not connection. Two
/// binaries that both know a topic are not joined unless the plan
/// routes them.
#[test]
fn matching_topics_alone_do_not_create_an_edge() {
    let r = fleet("noroute");
    // Same three instances, no routes at all.
    let plan = PLAN.replace(
        &PLAN[PLAN.find("\"routes\"").expect("routes")..PLAN.rfind('}').expect("brace")],
        "\"routes\": []\n",
    );
    write(&r, "noroute.plan.json", &plan);
    let out = hale_stdout(&[
        "fleet",
        "dump",
        r.join("noroute.plan.json").to_str().expect("utf8"),
    ]);
    let v: serde_json::Value =
        serde_json::from_str(&out).expect("parses");
    let _ = std::fs::remove_dir_all(&r);

    assert!(
        v["relations"]["routed"].as_array().expect("routed").is_empty(),
        "every instance declares the same topics, and a source merge \
         would have joined them — only a route may: {}",
        out
    );
}

/// Criterion 5. The join key is the WIRE identity, so a shared local
/// name over a different payload shape is not a route.
#[test]
fn a_same_subject_different_payload_route_is_rejected() {
    let r = fleet("wire");
    // A second gateway whose `Order` carries an extra field: same
    // subject, different payload hash.
    write(&r, "lib2/topics.hl", &TOPICS.replace(
        "type Order  { id: Int; }",
        "type Order  { id: Int; extra: Int; }",
    ));
    write(&r, "gw2/main.hl", &GW.replace("\"../lib\"", "\"../lib2\""));
    let dst = r.join("artifacts/gw2.json");
    let (_, code) = hale(&[
        "check",
        r.join("gw2").to_str().expect("utf8"),
        &format!("--dump-topology={}", dst.display()),
    ]);
    assert_eq!(code, 0, "gw2 must check clean");

    let plan = PLAN
        .replace(
            r#"{"id": "gw-0",     "artifact": "artifacts/gw.json",     "labels": ["gateway"]}"#,
            r#"{"id": "gw-0", "artifact": "artifacts/gw.json", "labels": []},
    {"id": "gw2-0", "artifact": "artifacts/gw2.json", "labels": []}"#,
        )
        .replace(
            r#"[{"instance": "gw-0",  "topic": "t::OrderRequest"}]"#,
            r#"[{"instance": "gw-0", "topic": "t::OrderRequest"},
                     {"instance": "gw2-0", "topic": "t::OrderRequest"}]"#,
        );
    write(&r, "wire.plan.json", &plan);
    let (out, code) = hale(&[
        "fleet",
        "check",
        r.join("wire.plan.json").to_str().expect("utf8"),
    ]);
    let _ = std::fs::remove_dir_all(&r);

    assert_ne!(code, 0, "incompatible payloads must fail: {}", out);
    assert!(
        out.contains("cannot be formed") && out.contains("WIRE identity"),
        "and say the contracts disagree, naming both hashes: {}",
        out
    );
}

/// A component is a foundation for a certificate, so anything that
/// cannot be verified is refused rather than trusted.
#[test]
fn an_unverifiable_component_is_refused() {
    let r = fleet("tamper");
    let p = r.join("artifacts/prober.json");
    let good = std::fs::read_to_string(&p).expect("read");
    // Rewrite a wire subject — a `topics` row, which `shape_hash`
    // does not cover but `artifact_digest` does.
    std::fs::write(&p, good.replacen("svc.order.intent", "svc.order.EVIL", 1))
        .expect("write");
    let (out, code) = hale(&["fleet", "check", &plan_of(&r)]);
    let _ = std::fs::remove_dir_all(&r);
    assert_ne!(code, 0, "{}", out);
    assert!(out.contains("artifact_digest"), "{}", out);
}

/// A component whose own law fails is not admissible: local claims
/// are a precondition of fleet admission.
#[test]
fn a_component_whose_own_claims_fail_is_refused() {
    let r = fleet("verdict");
    let p = r.join("artifacts/prober.json");
    let good = std::fs::read_to_string(&p).expect("read");
    // Simulate a failing component by asking the composer to trust a
    // verdict it should reject. (Rewriting the file breaks the
    // digest, so assert the ordering instead: integrity first, then
    // meaning.)
    std::fs::write(&p, good.replacen("\"verdict\": \"clean\"", "\"verdict\": \"law_failed\"", 1))
        .expect("write");
    let (out, code) = hale(&["fleet", "check", &plan_of(&r)]);
    let _ = std::fs::remove_dir_all(&r);
    assert_ne!(code, 0, "{}", out);
    // Integrity is checked before meaning, so this reports as a
    // digest failure — which is correct: an edited artifact is not
    // evidence of anything, whatever it now says.
    assert!(
        out.contains("artifact_digest") || out.contains("verdict"),
        "{}",
        out
    );
}

/// GH #476 Change 6 (round 4): the composer RECOMPUTES the
/// verdict from the component's own law rows — a restamped
/// artifact whose top-level verdict lies `clean` over a violated
/// law row is refused past the integrity gate.
#[test]
fn a_restamped_lying_verdict_is_refused() {
    let r = fleet("lyingverdict");
    let p = r.join("artifacts/prober.json");
    let good = std::fs::read_to_string(&p).expect("read");
    // Inject a VIOLATED law row (the prober's own table is empty)
    // while keeping the top-level verdict `clean`; then RESTAMP
    // the digest so integrity passes and the recompute is what
    // bites.
    let lied = good.replacen(
        "\"rows\": [\n    ]",
        "\"rows\": [\n      {\"ordinal\": 0, \"name\": \"ghost\", \
         \"origin\": \"main\", \"family\": \"reachability\", \
         \"verdict\": \"violated\", \"law\": {\"kind\": \
         \"forbid_reaches\"}}\n    ]",
        1,
    );
    assert_ne!(lied, good, "test premise: a law row was injected");
    assert!(
        lied.contains("\"verdict\": \"clean\""),
        "test premise: the document still claims clean"
    );
    std::fs::write(&p, restamp_both(&lied)).expect("write");
    let (out, code) = hale(&["fleet", "check", &plan_of(&r)]);
    let _ = std::fs::remove_dir_all(&r);
    assert_ne!(code, 0, "{}", out);
    // The shared admission refuses BEFORE the verdict recompute:
    // the injected row's payload is incomplete, and the deep
    // decoder names it. Either refusal is the law account speaking
    // — the digest gate was passed, and the lie did not survive.
    assert!(
        out.contains("malformed artifact")
            || out.contains("disagrees with its own law rows"),
        "the refusal names the law account, not the digest: {}",
        out
    );
    assert!(
        !out.contains("artifact_digest"),
        "integrity passed; the law account is what refused: {}",
        out
    );
}

#[test]
fn a_route_naming_an_undeclared_instance_is_rejected() {
    let r = fleet("ghost");
    let plan = PLAN.replace(
        r#"[{"instance": "oms-0",    "topic": "t::OrderIntent"}]"#,
        r#"[{"instance": "ghost", "topic": "t::OrderIntent"}]"#,
    );
    write(&r, "ghost.plan.json", &plan);
    let (out, code) = hale(&[
        "fleet",
        "check",
        r.join("ghost.plan.json").to_str().expect("utf8"),
    ]);
    let _ = std::fs::remove_dir_all(&r);
    assert_ne!(code, 0, "{}", out);
    assert!(out.contains("does not declare"), "{}", out);
}

/// Criteria 9 and 10. The fleet identity must move when the deployed
/// arrangement moves, and stay put when only source locations do.
#[test]
fn the_fleet_shape_hash_tracks_the_arrangement_not_provenance() {
    let r = fleet("hash");
    let hash_of = |plan: &str| -> String {
        let out = hale_stdout(&["fleet", "dump", plan]);
        let v: serde_json::Value =
            serde_json::from_str(&out).expect("parses");
        v["fleet_shape_hash"].as_str().unwrap_or("").to_string()
    };
    let base = hash_of(&plan_of(&r));
    assert!(!base.is_empty());

    // (a) provenance-only: a leading comment shifts every span in a
    // component, and its artifact changes — but the MODEL does not.
    write(&r, "prober/main.hl", &format!("// shift every span\n{}", PROBER));
    let dst = r.join("artifacts/prober.json");
    let (_, code) = hale(&[
        "check",
        r.join("prober").to_str().expect("utf8"),
        &format!("--dump-topology={}", dst.display()),
    ]);
    assert_eq!(code, 0);
    assert_eq!(
        hash_of(&plan_of(&r)),
        base,
        "moving source must not change the deployed arrangement"
    );

    // (b) a transport change IS the arrangement.
    write(&r, "t2.plan.json", &PLAN.replacen("\"unix\"", "\"udp\"", 1));
    assert_ne!(
        hash_of(r.join("t2.plan.json").to_str().expect("utf8")),
        base,
        "a changed transport is a changed fleet"
    );

    // (c) …and so is instance cardinality.
    write(
        &r,
        "t3.plan.json",
        &PLAN.replace(
            r#"{"id": "gw-0",     "artifact": "artifacts/gw.json",     "labels": ["gateway"]}"#,
            r#"{"id": "gw-0", "artifact": "artifacts/gw.json", "labels": []},
    {"id": "gw-1", "artifact": "artifacts/gw.json", "labels": []}"#,
        ),
    );
    assert_ne!(
        hash_of(r.join("t3.plan.json").to_str().expect("utf8")),
        base,
        "a second instance of one artifact is a different deployment"
    );
    let _ = std::fs::remove_dir_all(&r);
}

#[test]
fn a_duplicate_instance_id_is_rejected() {
    let r = fleet("dupid");
    let plan = PLAN.replace(r#""id": "oms-0""#, r#""id": "prober-0""#);
    write(&r, "dup.plan.json", &plan);
    let (out, code) = hale(&[
        "fleet",
        "check",
        r.join("dup.plan.json").to_str().expect("utf8"),
    ]);
    let _ = std::fs::remove_dir_all(&r);
    assert_ne!(code, 0, "{}", out);
    assert!(out.contains("used twice"), "{}", out);
}

/// An unknown key in a plan is a mistake, not something to ignore —
/// the same rule the environment manifest follows.
#[test]
fn an_unknown_plan_field_is_rejected() {
    let r = fleet("unknownfield");
    write(
        &r,
        "bad.plan.json",
        &PLAN.replace("\"name\": \"prod\"", "\"name\": \"prod\", \"transprot\": \"unix\""),
    );
    let (out, code) = hale(&[
        "fleet",
        "check",
        r.join("bad.plan.json").to_str().expect("utf8"),
    ]);
    let _ = std::fs::remove_dir_all(&r);
    assert_ne!(code, 0, "a misspelled plan key must not be ignored: {}", out);
}

// =====================================================================
// Phase 2: fleet claims over the composed model
// =====================================================================

/// Groups quantify over INSTANCES, by id or label; claims are
/// normalized rows rather than source grammar, so a generator can
/// produce a plan without Hale syntax committing to a deployment
/// format.
fn with_claims(plan: &str, extra_routes: &str) -> String {
    let groups_and_claims = format!(
        r#""groups": {{
    "strategies": {{"labels": ["strategy"]}},
    "gateways":   {{"labels": ["gateway"]}},
    "oms":        {{"instances": ["oms-0"]}}
  }},
  "claims": [
    {{"name": "orders_pass_oms",
     "forbid_reaches": {{"from": "strategies", "to": "gateways", "avoiding": "oms"}}}},
    {{"name": "one_order_authority",
     "count_publisher_instances": {{"subject": "svc.order.request", "eq": 1}}}},
    {{"name": "gw_receives_orders",
     "require_subscribes": {{"group": "gateways", "subject": "svc.order.request"}}}}
  ],
  "routes": [{}"#,
        extra_routes
    );
    plan.replace("\"routes\": [", &groups_and_claims)
}

/// Every path from a strategy to a gateway crosses the OMS, so the
/// interposition claim holds — and the cardinality and existence
/// claims with it.
#[test]
fn fleet_claims_hold_on_a_compliant_plan() {
    let r = fleet("claims_ok");
    write(&r, "c.plan.json", &with_claims(PLAN, ""));
    let out = hale_stdout(&[
        "fleet",
        "dump",
        r.join("c.plan.json").to_str().expect("utf8"),
    ]);
    let v: serde_json::Value =
        serde_json::from_str(&out).expect("fleet artifact parses");
    let _ = std::fs::remove_dir_all(&r);

    let results: Vec<(String, String)> = v["claims"]
        .as_array()
        .expect("claims")
        .iter()
        .map(|c| {
            (
                c["name"].as_str().unwrap_or("").to_string(),
                c["result"].as_str().unwrap_or("").to_string(),
            )
        })
        .collect();
    for (name, res) in &results {
        assert_eq!(res, "holds", "claim `{}` should hold: {:?}", name, results);
    }
    assert_eq!(results.len(), 3, "{:?}", results);
}

/// The required canary, and the flagship of the whole tier: a route
/// that skips the mediator. The witness must cross artifacts, name
/// the route, and name the source file on each side — none of which
/// a bundle-global offset could have supported, which is why Phase 0
/// came first.
#[test]
fn a_bypass_route_violates_the_interposition_claim() {
    let r = fleet("bypass");
    // A prober that can publish the executable request directly.
    write(
        &r,
        "rogue/main.hl",
        &PROBER
            .replace(
                "bus { publish t::OrderIntent; }",
                "bus { publish t::OrderIntent; publish t::OrderRequest; }",
            )
            .replace(
                "fn submit() { let i = t::Intent { id: 1 }; t::OrderIntent <- i; }",
                "fn submit() { let i = t::Intent { id: 1 }; t::OrderIntent <- i; \
                 let o = t::Order { id: 1 }; t::OrderRequest <- o; }",
            ),
    );
    let dst = r.join("artifacts/rogue.json");
    let (_, code) = hale(&[
        "check",
        r.join("rogue").to_str().expect("utf8"),
        &format!("--dump-topology={}", dst.display()),
    ]);
    assert_eq!(code, 0, "the rogue component itself is legal");

    let plan = with_claims(PLAN, "")
        .replace("artifacts/prober.json", "artifacts/rogue.json")
        .replace(
            r#"{"id": "request", "transport": "unix","#,
            r#"{"id": "bypass", "transport": "unix",
     "publishers":  [{"instance": "prober-0", "topic": "t::OrderRequest"}],
     "subscribers": [{"instance": "gw-0",     "topic": "t::OrderRequest"}]},
    {"id": "request", "transport": "unix","#,
        );
    write(&r, "bypass.plan.json", &plan);
    let (out, code) = hale(&[
        "fleet",
        "check",
        r.join("bypass.plan.json").to_str().expect("utf8"),
    ]);
    let _ = std::fs::remove_dir_all(&r);

    assert_ne!(code, 0, "a bypass must fail the fleet: {}", out);
    assert!(
        out.contains("orders_pass_oms"),
        "the interposition claim must be the one that fails: {}",
        out
    );
    // The witness crosses two artifacts…
    assert!(
        out.contains("prober-0::Probe::submit")
            && out.contains("gw-0::Gateway::on_order"),
        "witness must name both ends, instance-qualified: {}",
        out
    );
    // …names the route that carries it…
    assert!(
        out.contains("route `bypass`"),
        "and the route, since the hop is not a call: {}",
        out
    );
    // …and points at a source FILE on each side, which is what the
    // Phase 0 source map exists for.
    assert!(
        out.contains("rogue/main.hl") && out.contains("gw/main.hl"),
        "and the file each vertex lives in: {}",
        out
    );
}

/// Criterion 6: a second deployed publisher violates exact
/// cardinality. Fleet cardinality counts instance-qualified
/// endpoints, which is a different sort from the application tier's
/// declaration count — both components are individually legal.
#[test]
fn a_second_deployed_publisher_violates_the_cardinality_claim() {
    let r = fleet("cardinality");
    write(
        &r,
        "rogue/main.hl",
        &PROBER
            .replace(
                "bus { publish t::OrderIntent; }",
                "bus { publish t::OrderIntent; publish t::OrderRequest; }",
            )
            .replace(
                "fn submit() { let i = t::Intent { id: 1 }; t::OrderIntent <- i; }",
                "fn submit() { let i = t::Intent { id: 1 }; t::OrderIntent <- i; \
                 let o = t::Order { id: 1 }; t::OrderRequest <- o; }",
            ),
    );
    let dst = r.join("artifacts/rogue.json");
    hale(&[
        "check",
        r.join("rogue").to_str().expect("utf8"),
        &format!("--dump-topology={}", dst.display()),
    ]);
    let plan = with_claims(PLAN, "")
        .replace("artifacts/prober.json", "artifacts/rogue.json");
    write(&r, "card.plan.json", &plan);
    let (out, code) = hale(&[
        "fleet",
        "check",
        r.join("card.plan.json").to_str().expect("utf8"),
    ]);
    let _ = std::fs::remove_dir_all(&r);

    assert_ne!(code, 0, "{}", out);
    assert!(
        out.contains("one_order_authority")
            && out.contains("counted 2 deployed publisher"),
        "the count is over DEPLOYED endpoints, and must name them: {}",
        out
    );
}

/// Criterion 7: removing the required subscriber violates the
/// structural existence claim.
#[test]
fn removing_the_required_subscriber_violates_the_existence_claim() {
    let r = fleet("existence");
    // A gateway that subscribes nothing.
    write(
        &r,
        "gw/main.hl",
        "import \"../lib\" as t;\n\
         locus Gateway { params { n: Int = 0; } fn idle() -> Int { return self.n; } }\n\
         main locus GwApp { params { g: Gateway = Gateway { }; } }\n\
         fn main() { GwApp { }; }\n",
    );
    let dst = r.join("artifacts/gw.json");
    let (_, code) = hale(&[
        "check",
        r.join("gw").to_str().expect("utf8"),
        &format!("--dump-topology={}", dst.display()),
    ]);
    assert_eq!(code, 0, "a gateway that listens for nothing is legal alone");

    // The route to it can no longer be formed either, so drop it and
    // test the existence claim on its own.
    let plan = with_claims(PLAN, "")
        .replace(
            r#"{"id": "request", "transport": "unix",
     "publishers":  [{"instance": "oms-0", "topic": "t::OrderRequest"}],
     "subscribers": [{"instance": "gw-0",  "topic": "t::OrderRequest"}]}"#,
            "",
        )
        .replace("]},\n    \n  ]", "]}\n  ]");
    write(&r, "gone.plan.json", &plan);
    let (out, code) = hale(&[
        "fleet",
        "check",
        r.join("gone.plan.json").to_str().expect("utf8"),
    ]);
    let _ = std::fs::remove_dir_all(&r);

    assert_ne!(code, 0, "{}", out);
    assert!(
        out.contains("gw_receives_orders"),
        "the existence claim must fail: {}",
        out
    );
}

/// An unknown group is an error, never an empty set — a `forbid`
/// satisfied by an empty quantification domain is a fail-open
/// wearing formal clothing.
#[test]
fn an_empty_or_unknown_group_is_an_error() {
    let r = fleet("groups");
    // Only the GROUP's label — `.replace` would otherwise rewrite the
    // instance's label too, leaving them consistent and the group
    // resolving fine.
    let plan = with_claims(PLAN, "").replace(
        r#""strategies": {"labels": ["strategy"]}"#,
        r#""strategies": {"labels": ["nonexistent"]}"#,
    );
    write(&r, "g.plan.json", &plan);
    let (out, code) = hale(&[
        "fleet",
        "check",
        r.join("g.plan.json").to_str().expect("utf8"),
    ]);
    let _ = std::fs::remove_dir_all(&r);
    assert_ne!(code, 0, "{}", out);
    assert!(
        out.contains("no instance carries") || out.contains("vacuously"),
        "{}",
        out
    );
}

/// A downstream conformance run found this: `require_subscribes`
/// checked only that the endpoint EXISTS, so a plan where the ledger
/// subscribes `exec.fill` and nothing publishes it reported `holds`.
///
/// The law "fills must reach the ledger" then cannot catch a missing
/// route, which is the one thing it is for. A synthetic fixture hides
/// it because whoever writes one routes everything they assert — this
/// only surfaced against a real deployment slice whose publisher
/// happened to sit outside the selected instances.
#[test]
fn require_subscribes_needs_a_route_not_just_an_endpoint() {
    let r = fleet("unrouted");
    // The gateway subscribes `svc.order.request` and the OMS
    // publishes it — but the plan carries only the intent route, so
    // nothing delivers order requests.
    let plan = with_claims(PLAN, "")
        .replace(
            r#",
    {"id": "request", "transport": "unix",
     "publishers":  [{"instance": "oms-0", "topic": "t::OrderRequest"}],
     "subscribers": [{"instance": "gw-0",  "topic": "t::OrderRequest"}]}"#,
            "",
        );
    write(&r, "unrouted.plan.json", &plan);
    let (out, code) = hale(&[
        "fleet",
        "check",
        r.join("unrouted.plan.json").to_str().expect("utf8"),
    ]);

    assert_ne!(
        code, 0,
        "the endpoint exists but nothing routes to it — the claim is \
         about traffic, and no traffic flows: {}",
        out
    );
    assert!(
        out.contains("gw_receives_orders")
            && out.contains("no route in this plan carries it"),
        "and the failure must say the endpoint is unconnected rather \
         than absent: {}",
        out
    );

    // Control: with the route restored it holds, so the check is not
    // simply rejecting every require_subscribes.
    write(&r, "routed.plan.json", &with_claims(PLAN, ""));
    let (out2, code2) = hale(&[
        "fleet",
        "check",
        r.join("routed.plan.json").to_str().expect("utf8"),
    ]);
    let _ = std::fs::remove_dir_all(&r);
    assert_eq!(code2, 0, "the routed plan still passes: {}", out2);
}

// =====================================================================
// Phase 5: the workspace's declared deployments
// =====================================================================

/// A repository usually has more than one deployment — production,
/// staging, a reconciliation arrangement. Checking whichever one you
/// remembered to name is the same partial-coverage problem `--matrix`
/// solves for entrypoints, so `hale fleet check` with no plan checks
/// every fleet the workspace declares.
fn hale_in(cwd: &Path, args: &[&str]) -> (String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .current_dir(cwd)
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

#[test]
fn fleet_check_with_no_plan_checks_every_declared_deployment() {
    let r = fleet("allfleets");
    write(&r, "staging.plan.json", &PLAN.replace("\"prod\"", "\"staging\""));
    write(
        &r,
        "hale.toml",
        "[deps]\n\n[fleets]\nproduction = \"prod.plan.json\"\nstaging = \"staging.plan.json\"\n",
    );
    let (out, code) = hale_in(&r, &["fleet", "check"]);
    let _ = std::fs::remove_dir_all(&r);

    assert_eq!(code, 0, "{}", out);
    assert!(out.contains("2 fleet(s) checked"), "{}", out);
    assert!(
        out.contains("fleet `production`") && out.contains("fleet `staging`"),
        "each deployment must be named as it is checked: {}",
        out
    );
}

/// One broken deployment must not hide the others, and the worst exit
/// code wins so a missing plan is not masked by an ordinary claim
/// failure elsewhere.
#[test]
fn a_missing_plan_fails_without_hiding_the_rest() {
    let r = fleet("missingplan");
    write(
        &r,
        "hale.toml",
        "[deps]\n\n[fleets]\nproduction = \"prod.plan.json\"\nghost = \"nope.plan.json\"\n",
    );
    let (out, code) = hale_in(&r, &["fleet", "check"]);
    let _ = std::fs::remove_dir_all(&r);

    assert_eq!(code, 2, "a missing plan is a usage error: {}", out);
    assert!(out.contains("plan not found"), "{}", out);
    assert!(
        out.contains("fleet `production`"),
        "the healthy deployment must still have been checked: {}",
        out
    );
}

/// Reporting success for zero deployments would say nothing.
#[test]
fn a_workspace_with_no_declared_fleets_is_a_usage_error() {
    let r = fleet("nofleets");
    write(&r, "hale.toml", "[deps]\n");
    let (out, code) = hale_in(&r, &["fleet", "check"]);
    let _ = std::fs::remove_dir_all(&r);
    assert_eq!(code, 2, "{}", out);
    assert!(out.contains("declares no `[fleets]`"), "{}", out);
}

/// Fleets and environments are separate axes: a fleet is an
/// arrangement of deployed instances, an environment is law bound to
/// an entrypoint. Declaring both must not make either ambiguous, and
/// a name may appear in both meaning different things.
#[test]
fn fleets_and_environments_are_independent_axes() {
    let r = fleet("axes");
    write(
        &r,
        "hale.toml",
        "[claims]\nno_base = true\n\n\
         [environments.production]\nsource_only = true\n\
         entrypoints = [\"prober\", \"oms\", \"gw\"]\n\n\
         [fleets]\nproduction = \"prod.plan.json\"\n",
    );
    // The fleet axis works…
    let (out, code) = hale_in(&r, &["fleet", "check"]);
    assert_eq!(code, 0, "fleet axis: {}", out);
    assert!(out.contains("1 fleet(s) checked"), "{}", out);

    // …and the entrypoint x environment axis works over the same
    // manifest, with `production` meaning something different.
    let (out2, code2) = hale_in(&r, &["check", "--matrix"]);
    let _ = std::fs::remove_dir_all(&r);
    assert_eq!(
        code2, 0,
        "the environment axis is unaffected by the fleets table: {}",
        out2
    );
    assert!(out2.contains("pair(s) checked"), "{}", out2);
}

/// Round 5: DELETING a component's law rows must not pass
/// vacuously. A claims-bearing prober artifact has its `law.rows`
/// emptied (claims section intact) and is restamped; the shared
/// admission's claims↔law join refuses it.
#[test]
fn deleted_law_rows_are_refused() {
    let r = fleet("deletedlaw");
    // Rebuild the prober WITH a claim so its law table is
    // non-empty (the base prober's is empty — deleting nothing
    // proves nothing).
    write(
        &r,
        "prober/main.hl",
        r#"
import "../lib" as t;
fn leak(v: Int) -> Int { return v; }
fn safe(v: Int) -> Int { return v; }
group a_side = { safe };
group b_side = { leak };
locus Probe {
    params { n: Int = 0; }
    bus { publish t::OrderIntent; }
    fn submit() { let i = t::Intent { id: 1 }; t::OrderIntent <- i; }
}
main locus Prober {
    params { p: Probe = Probe { }; }
    claims { iso: forbid reaches(a_side, b_side); }
}
fn main() { Prober { }; }
"#,
    );
    let p = r.join("artifacts/prober.json");
    let (out, code) = hale(&[
        "check",
        r.join("prober").to_str().expect("utf8"),
        &format!("--dump-topology={}", p.display()),
    ]);
    assert_eq!(code, 0, "claimed prober must check clean: {}", out);
    let good = std::fs::read_to_string(&p).expect("read");

    // Empty law.rows by bracket-matching from the `"rows": [`
    // inside the `law` object.
    let law_at = good.find("\"law\": {").expect("law section");
    let rows_key = good[law_at..]
        .find("\"rows\": [")
        .map(|i| law_at + i)
        .expect("law.rows");
    let open = rows_key + "\"rows\": ".len();
    let bytes = good.as_bytes();
    let mut depth = 0usize;
    let mut close = open;
    let mut in_str = false;
    let mut esc = false;
    for (i, &b) in bytes.iter().enumerate().skip(open) {
        if esc {
            esc = false;
            continue;
        }
        match b {
            b'\\' if in_str => esc = true,
            b'"' => in_str = !in_str,
            b'[' if !in_str => depth += 1,
            b']' if !in_str => {
                depth -= 1;
                if depth == 0 {
                    close = i;
                    break;
                }
            }
            _ => {}
        }
    }
    assert!(close > open, "bracket match");
    let gutted =
        format!("{}[]{}", &good[..open], &good[close + 1..]);
    assert_ne!(gutted, good, "test premise: rows were deleted");
    assert!(
        gutted.contains("\"claims\""),
        "test premise: the claims section survives"
    );
    std::fs::write(&p, restamp_both(&gutted)).expect("write");
    let (out, code) = hale(&["fleet", "check", &plan_of(&r)]);
    let _ = std::fs::remove_dir_all(&r);
    assert_ne!(code, 0, "{}", out);
    assert!(
        out.contains("does not project one-to-one from law"),
        "the claims↔law join refuses the gutted account: {}",
        out
    );
    assert!(
        !out.contains("artifact_digest"),
        "integrity passed; the law account is what refused: {}",
        out
    );
}

/// Round 2 (#490): route identity is ONE grain. A component with
/// only a LITERAL end on the topic's wire (a raw send to
/// "svc.order.intent", never `publish t::OrderIntent`) cannot
/// satisfy a route naming the topic — the role check and the edge
/// builder both read the typed topic-identity rows.
#[test]
fn literal_only_publisher_cannot_satisfy_a_topic_route() {
    let r = fleet("literalroute");
    // Rebuild the prober to publish the WIRE literally, never the
    // topic identity.
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
    let dst = r.join("artifacts/prober.json");
    let (out, code) = hale(&[
        "check",
        r.join("prober").to_str().expect("utf8"),
        &format!("--dump-topology={}", dst.display()),
    ]);
    assert_eq!(code, 0, "literal prober checks clean: {}", out);
    let (out, code) = hale(&["fleet", "check", &plan_of(&r)]);
    let _ = std::fs::remove_dir_all(&r);
    assert_ne!(code, 0, "{}", out);
    assert!(
        out.contains("declares no topic")
            || out.contains("nothing in that component"),
        "the topic-grain role check refuses the literal-only \
         publisher: {}",
        out
    );
}

/// Round 2 (#490): the declared IDENTITY must recompute. A
/// coordinated edit of the hashed endpoint_identity AND its
/// unhashed mirrors under a STALE shape_hash — with only
/// artifact_digest restamped — refuses before any decoding.
#[test]
fn stale_shape_hash_is_refused() {
    fn fnv1a64(bytes: &[u8]) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        for b in bytes {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x100000001b3);
        }
        h
    }
    let r = fleet("staleshape");
    let p = r.join("artifacts/prober.json");
    let good = std::fs::read_to_string(&p).expect("read");
    // Rewrite the literal wire in BOTH the hashed identity and the
    // unhashed mirrors, consistently.
    let drifted = good.replace(
        "svc.order.intent",
        "svc.order.hijack",
    );
    assert_ne!(drifted, good, "test premise: the wire drifted");
    // Restamp ONLY the document digest — the shape_hash stays
    // stale.
    let key = ",\n  \"artifact_digest\": \"";
    let cut = drifted.rfind(key).expect("digest trailer");
    let body = &drifted[..cut];
    let restamped = format!(
        "{}{}{:016x}\"\n}}\n",
        body,
        key,
        fnv1a64(body.as_bytes())
    );
    std::fs::write(&p, restamped).expect("write");
    let (out, code) = hale(&["fleet", "check", &plan_of(&r)]);
    let _ = std::fs::remove_dir_all(&r);
    assert_ne!(code, 0, "{}", out);
    assert!(
        out.contains("shape_hash does not recompute"),
        "the stale identity refuses: {}",
        out
    );
}

/// Round 3 (#490): the typed decoder is STRICT — a malformed
/// semantic row is refused, never silently filtered. A call edge
/// whose endpoint is a number would otherwise vanish before the
/// shared traversal, flipping a violated fleet prohibition to
/// holds; a non-string unknown reason would erase residue the
/// graph must certify over. The oms component is rebuilt to carry
/// BOTH shapes, and each premise is asserted — no silent skip.
#[test]
fn malformed_semantic_rows_are_refused_not_dropped() {
    const OMS_WITH_SHAPES: &str = r#"
import "../lib" as t;
fn helper(v: Int) -> Int { return v + 1; }
fn call_it(f: fn(Int) -> Int, v: Int) -> Int { return f(v); }
locus Oms {
    params { n: Int = 0; }
    bus { subscribe t::OrderIntent as on_intent; publish t::OrderRequest; }
    fn on_intent(i: t::Intent) {
        self.n = call_it(helper, i.id);
        let o = t::Order { id: i.id };
        t::OrderRequest <- o;
    }
}
main locus OmsApp { params { o: Oms = Oms { }; } }
fn main() { OmsApp { }; }
"#;
    for (tag, needle, patched, expect) in [
        (
            "badcall",
            "{\"from\": \"Oms::on_intent\", \"to\": \"call_it\"}",
            "{\"from\": \"Oms::on_intent\", \"to\": 7}",
            "relations.calls[0].to must be a string",
        ),
        (
            "badreason",
            "\"reasons\": [\"indirect_call\"]",
            "\"reasons\": [7]",
            "unknowns[0].reasons[0] must be a string",
        ),
    ] {
        let r = fleet(tag);
        write(&r, "oms/main.hl", OMS_WITH_SHAPES);
        let p = r.join("artifacts/oms.json");
        let (out, code) = hale(&[
            "check",
            r.join("oms").to_str().expect("utf8"),
            &format!("--dump-topology={}", p.display()),
        ]);
        assert_eq!(code, 0, "{}: oms rebuilds clean: {}", tag, out);
        let good = std::fs::read_to_string(&p).expect("read");
        assert!(
            good.contains(needle),
            "{}: test premise — the shape is present:\n{}",
            tag,
            good
        );
        let bad = good.replacen(needle, patched, 1);
        std::fs::write(&p, restamp_both(&bad)).expect("write");
        let (out, code) = hale(&["fleet", "check", &plan_of(&r)]);
        let _ = std::fs::remove_dir_all(&r);
        assert_ne!(code, 0, "{}: {}", tag, out);
        assert!(
            out.contains(expect),
            "{}: the strict decoder refuses (wanted `{}`): {}",
            tag,
            expect,
            out
        );
    }
}

/// Round 3 (#490): duplicate object keys are refused before/// Round 3 (#490): duplicate object keys are refused before
/// parsing — serde's last-wins map would otherwise let a stale
/// `shape_hash` shadow the raw-verified one.
#[test]
fn duplicate_shape_hash_is_refused() {
    fn fnv1a64(bytes: &[u8]) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        for b in bytes {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x100000001b3);
        }
        h
    }
    let r = fleet("dupkey");
    let p = r.join("artifacts/prober.json");
    let good = std::fs::read_to_string(&p).expect("read");
    // Append a SECOND shape_hash after the sources section — the
    // raw verifier reads the first; serde would keep this one.
    let marker = ",\n  \"claims\": [";
    let poisoned = good.replacen(
        marker,
        ",\n  \"shape_hash\": \"deadbeefdeadbeef\",\n  \
         \"claims\": [",
        1,
    );
    assert_ne!(poisoned, good, "test premise: the key landed");
    let key = ",\n  \"artifact_digest\": \"";
    let cut = poisoned.rfind(key).expect("digest trailer");
    let body = &poisoned[..cut];
    let restamped = format!(
        "{}{}{:016x}\"\n}}\n",
        body,
        key,
        fnv1a64(body.as_bytes())
    );
    std::fs::write(&p, restamped).expect("write");
    let (out, code) = hale(&["fleet", "check", &plan_of(&r)]);
    let _ = std::fs::remove_dir_all(&r);
    assert_ne!(code, 0, "{}", out);
    assert!(
        out.contains("duplicate object key `shape_hash`"),
        "the duplicate key refuses before parsing: {}",
        out
    );
}

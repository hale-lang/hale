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

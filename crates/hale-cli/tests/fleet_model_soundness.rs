//! The two rules a composed model must not break: keep every edge
//! the components represent, and never certify an absence over a
//! graph with holes in it.
//!
//! GH #408. The fleet tier used to read `relations.calls` and stop
//! there, so a user→stdlib→user path — which the application checker
//! walks, and which the artifact exports as `calls_via_stdlib` —
//! vanished the moment those components were composed. It also
//! copied every component's `unknowns` into its output and evaluated
//! as though they were not there, so an indirect call could remove
//! the only modeled path to a target and the prohibition reported
//! `holds`: an absence certified by not looking.
//!
//! Both are now answered by one shared reachability engine
//! (`hale_types::model_graph`) rather than a second graph walk that
//! has to remember every rule the first one learned.

use std::path::{Path, PathBuf};
use std::process::Command;

fn root(tag: &str) -> PathBuf {
    let d = std::env::temp_dir()
        .join(format!("hale_fms_{}_{}", std::process::id(), tag));
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

const A: &str = r#"
import "../lib" as t;
locus Probe {
    params { n: Int = 0; }
    bus { publish t::OrderIntent; }
    fn submit() { let i = t::Intent { id: 1 }; t::OrderIntent <- i; }
}
main locus Prober { params { p: Probe = Probe { }; } }
fn main() { Prober { }; }
"#;

/// The middle component's handler reaches its publisher ONLY through
/// `std::http::Router`. Its artifact has an empty `calls` relation
/// and one `calls_via_stdlib` edge, so a composer reading `calls`
/// alone sees an instance that connects nothing.
const B_VIA_STDLIB: &str = r#"
import "../lib" as t;
locus Fwd {
    params { n: Int = 0; }
    bus { publish t::OrderRequest; }
    fn handle(ctx: std::http::Context) -> std::http::Response {
        let o = t::Order { id: 1 };
        t::OrderRequest <- o;
        return std::http::Response { status: 200, body: "ok" };
    }
}
locus Oms {
    params { n: Int = 0; }
    bus { subscribe t::OrderIntent as on_intent; }
    fn on_intent(i: t::Intent) {
        self.n = i.id;
        let r = std::http::Router { };
        r.add("GET", "/fwd", Fwd { });
        let resp = r.dispatch(std::http::Request {
            method: "GET", path: "/fwd",
            version: "HTTP/1.1", headers: "", body: ""
        });
        self.n = resp.status;
    }
}
main locus OmsApp { params { o: Oms = Oms { }; } }
fn main() { OmsApp { }; }
"#;

/// The middle component's handler calls through an index-result
/// receiver, which stays untypeable — its outgoing edges are
/// incomplete, so nothing downstream of it can be proved absent.
const B_UNKNOWN: &str = r#"
import "../lib" as t;
locus Sink { params { n: Int = 0; } fn work(v: Int) -> Int { return v * 2; } }
locus Oms {
    params { n: Int = 0; }
    bus { subscribe t::OrderIntent as on_intent; }
    fn on_intent(i: t::Intent) {
        let xs = [Sink { }];
        self.n = xs[0].work(i.id);
    }
}
main locus OmsApp { params { o: Oms = Oms { }; } }
fn main() { OmsApp { }; }
"#;

const C: &str = r#"
import "../lib" as t;
locus Gateway {
    params { n: Int = 0; }
    bus { subscribe t::OrderRequest as on_order; }
    fn on_order(o: t::Order) { self.n = o.id; }
}
main locus GwApp { params { g: Gateway = Gateway { }; } }
fn main() { GwApp { }; }
"#;

const INSTANCES: &str = r#"[
    {"id": "a-0", "artifact": "artifacts/a.json", "labels": ["strategy"]},
    {"id": "b-0", "artifact": "artifacts/b.json", "labels": ["oms"]},
    {"id": "c-0", "artifact": "artifacts/c.json", "labels": ["gateway"]}]"#;

const GROUPS: &str = r#"{
    "strategy": {"labels": ["strategy"]},
    "gateway":  {"labels": ["gateway"]}}"#;

const NO_REACH: &str = r#"[{"name": "no_reach",
    "forbid_reaches": {"from": "strategy", "to": "gateway"}}]"#;

/// Build the three components with the given middle one.
fn fixture(tag: &str, middle: &str) -> PathBuf {
    let r = root(tag);
    write(&r, "lib/topics.hl", TOPICS);
    write(&r, "a/main.hl", A);
    write(&r, "b/main.hl", middle);
    write(&r, "c/main.hl", C);
    write(&r, "hale.toml", "[deps]\n");
    for app in ["a", "b", "c"] {
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

fn check(r: &Path, routes: &str) -> (String, i32) {
    let plan = format!(
        r#"{{"schema": "1.0", "name": "prod",
            "instances": {INSTANCES}, "routes": {routes},
            "groups": {GROUPS}, "claims": {NO_REACH}}}"#
    );
    write(r, "plan.json", &plan);
    hale(&["fleet", "check", r.join("plan.json").to_str().expect("utf8")])
}

const BOTH_ROUTES: &str = r#"[
    {"id": "intent", "transport": "unix",
     "publishers":  [{"instance": "a-0", "topic": "t::OrderIntent"}],
     "subscribers": [{"instance": "b-0", "topic": "t::OrderIntent"}]},
    {"id": "request", "transport": "unix",
     "publishers":  [{"instance": "b-0", "topic": "t::OrderRequest"}],
     "subscribers": [{"instance": "c-0", "topic": "t::OrderRequest"}]}]"#;

const INTENT_ONLY: &str = r#"[
    {"id": "intent", "transport": "unix",
     "publishers":  [{"instance": "a-0", "topic": "t::OrderIntent"}],
     "subscribers": [{"instance": "b-0", "topic": "t::OrderIntent"}]}]"#;

/// F-1: the hop through stdlib is part of the path.
#[test]
fn a_path_through_stdlib_survives_composition() {
    let r = fixture("viastdlib", B_VIA_STDLIB);

    // The premise: this component's user→user edge exists ONLY in
    // `calls_via_stdlib`. If that ever stops being true the test
    // still passes but stops testing anything, so assert it.
    let art = std::fs::read_to_string(r.join("artifacts/b.json"))
        .expect("artifact");
    let v: serde_json::Value =
        serde_json::from_str(&art).expect("parses");
    assert!(
        v["relations"]["calls"].as_array().expect("calls").is_empty(),
        "the middle component must have NO direct call edges, or this \
         test does not exercise the contracted relation"
    );
    assert!(
        !v["relations"]["calls_via_stdlib"]
            .as_array()
            .expect("via")
            .is_empty(),
        "the middle component must have a through-stdlib edge"
    );

    let (out, code) = check(&r, BOTH_ROUTES);
    assert_eq!(
        code, 1,
        "strategy reaches gateway through the middle component's \
         stdlib-interior path: {out}"
    );
    assert!(
        out.contains("Oms::on_intent") && out.contains("Fwd::handle"),
        "the witness must include the hop the stdlib contracted: {out}"
    );
    let _ = std::fs::remove_dir_all(&r);
}

/// F-3: a hole the source can walk to blocks certification.
#[test]
fn a_reachable_unknown_makes_an_absence_uncertified() {
    let r = fixture("unknown", B_UNKNOWN);

    let art = std::fs::read_to_string(r.join("artifacts/b.json"))
        .expect("artifact");
    assert!(
        art.contains("untyped_receiver_call"),
        "the middle component must carry an unknown, or this test \
         does not exercise uncertainty:\n{art}"
    );

    // Nothing routes b -> c, so the modeled graph has no path. The
    // old answer was `holds`.
    let (out, code) = check(&r, INTENT_ONLY);
    assert_eq!(
        code, 1,
        "an unknown reachable from the source could conceal the path, \
         so this absence must not be certified: {out}"
    );
    assert!(
        out.contains("uncertified"),
        "the verdict must be `uncertified` — nothing was disproved, \
         so it is not `violated` either: {out}"
    );
    assert!(
        out.contains("Oms::on_intent"),
        "the refusal must name the vertex whose edges are missing, or \
         it is not actionable: {out}"
    );
    let _ = std::fs::remove_dir_all(&r);
}

/// The rule that keeps `uncertified` usable: an unknown the claim's
/// source cannot reach is not evidence about that claim.
///
/// Same artifacts and same claim as the test above — only the route
/// differs. With nothing routed into the component that carries the
/// unknown, the absence is real and certified.
#[test]
fn an_unreachable_unknown_does_not_poison_the_fleet() {
    let r = fixture("unrelated", B_UNKNOWN);
    let (out, code) = check(&r, "[]");
    assert_eq!(
        code, 0,
        "with nothing routed from strategy, the unknown in the middle \
         component is unreachable and must not block certification: \
         {out}"
    );
    let _ = std::fs::remove_dir_all(&r);
}

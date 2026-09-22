//! GH #527 B6 — the typed control channel.
//!
//! What these hold the design to:
//!   1. A `bindings { }` entry may name an IMPORTED topic
//!      (`alias::Topic`): the organism binds the DNA core's own
//!      declarations, it does not re-declare them.
//!   2. The edge is a declared route in the fleet model: `hale fleet
//!      check` admits it over the organism's artifact and the membrane
//!      client's (`dna/membrane`, the program `hale dna task create`
//!      and `hale dna review` exec), and refuses a route naming a
//!      topic the client never publishes.
//!
//! That a verdict and an intent published by the membrane client cross
//! the socket and the owning locus decides is proven end to end by the
//! DNA fixtures (`dna/tests/membrane_loss_test.hl`,
//! `dna/tests/relay_repeated_request_test.hl`). Iris publishes nothing
//! into the organism (#998).

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

fn hale() -> Command {
    Command::new(env!("CARGO_BIN_EXE_hale"))
}

fn workdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hale_membrane_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The reference organism, relocated: `dna/core` copied beside it and
/// its two literal socket paths pointed into `root`, so parallel
/// tests never share a socket.
fn organism_fixture(root: &Path) -> PathBuf {
    let core_src = repo().join("dna/core");
    let core_dst = root.join("dna/core");
    std::fs::create_dir_all(&core_dst).unwrap();
    for e in std::fs::read_dir(&core_src).unwrap() {
        let e = e.unwrap();
        if e.path().extension().and_then(|x| x.to_str()) == Some("hl") {
            std::fs::copy(e.path(), core_dst.join(e.file_name())).unwrap();
        }
    }
    let src = std::fs::read_to_string(repo().join("dna/organism/main.hl")).unwrap();
    let relocated = src
        .replace("/tmp/hale-dna.review.verdict.sock", &format!("{}/hale-dna.review.verdict.sock", root.display()))
        .replace("/tmp/hale-dna.intent.offered.sock", &format!("{}/hale-dna.intent.offered.sock", root.display()));
    assert_ne!(src, relocated, "the reference organism must bind the documented socket paths");
    let seed = root.join("dna/organism");
    std::fs::create_dir_all(&seed).unwrap();
    std::fs::write(seed.join("main.hl"), relocated).unwrap();
    seed
}

#[test]
fn a_binding_may_name_an_imported_topic() {
    let dir = workdir("qualified");
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    std::fs::create_dir_all(dir.join("app")).unwrap();
    std::fs::write(
        dir.join("lib/main.hl"),
        "type T { n: Int = 0; }\ntopic Evt { payload: T; subject: \"x.evt\"; }\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("app/main.hl"),
        format!(
            "import \"../lib\" as lib;\nlocus Sub {{\n    params {{ seen: Int = 0; }}\n    bus {{ subscribe lib::Evt as on_e; }}\n    fn on_e(t: lib::T) {{ self.seen = self.seen + 1; }}\n}}\nmain locus App {{\n    params {{ s: Sub = Sub {{ }}; }}\n    bindings {{ lib::Evt: unix(\"{}/evt.sock\", role: listen); }}\n    run() {{ std::time::sleep(100ms); }}\n}}\nfn main() {{ App {{ }}; }}\n",
            dir.display()
        ),
    )
    .unwrap();
    let out = hale().arg("check").arg(dir.join("app")).output().unwrap();
    assert!(out.status.success(), "check: {}", String::from_utf8_lossy(&out.stderr));
    let out = hale().arg("build").arg(dir.join("app")).output().unwrap();
    assert!(out.status.success(), "build: {}", String::from_utf8_lossy(&out.stderr));
    let out = Command::new(dir.join("app/app")).output().unwrap();
    assert!(out.status.success(), "the listener realizes and exits clean: {}", String::from_utf8_lossy(&out.stderr));
    // and a binding to a topic the alias does not declare is refused
    std::fs::write(
        dir.join("app/main.hl"),
        format!(
            "import \"../lib\" as lib;\nmain locus App {{\n    bindings {{ lib::Nope: unix(\"{}/x.sock\", role: listen); }}\n    run() {{ }}\n}}\nfn main() {{ App {{ }}; }}\n",
            dir.display()
        ),
    )
    .unwrap();
    let out = hale().arg("check").arg(dir.join("app")).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown topic"), "{}", String::from_utf8_lossy(&out.stderr));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_membrane_edge_is_a_declared_route_in_the_fleet_model() {
    let dir = workdir("fleet");
    let seed = organism_fixture(&dir);
    let org = dir.join("organism.topology");
    let out = hale().arg("check").arg(&seed).arg(format!("--dump-topology={}", org.display())).output().unwrap();
    assert!(org.is_file(), "organism artifact: {}", String::from_utf8_lossy(&out.stderr));
    let client = dir.join("membrane.topology");
    let out = hale()
        .arg("check")
        .arg(repo().join("dna/membrane"))
        .arg(format!("--dump-topology={}", client.display()))
        .output()
        .unwrap();
    assert!(client.is_file(), "membrane client artifact: {}", String::from_utf8_lossy(&out.stderr));
    let plan = |routes: &str, claims: &str| -> (bool, String) {
        let p = dir.join("plan.json");
        std::fs::write(
            &p,
            format!(
                r#"{{"schema": "1.0", "name": "organism-with-membrane-client",
  "instances": [
    {{"id": "organism-0", "artifact": "organism.topology", "labels": ["organism"]}},
    {{"id": "membrane-0", "artifact": "membrane.topology", "labels": ["client"]}}],
  "routes": {routes},
  "groups": {{"organism": {{"labels": ["organism"]}}, "client": {{"labels": ["client"]}}}},
  "claims": {claims}}}"#
            ),
        )
        .unwrap();
        let out = hale().args(["fleet", "check"]).arg(&p).output().unwrap();
        (
            out.status.success(),
            format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)),
        )
    };
    let routes = r#"[
    {"id": "verdicts", "transport": "unix",
     "publishers":  [{"instance": "membrane-0", "topic": "dna::ReviewVerdict"}],
     "subscribers": [{"instance": "organism-0", "topic": "dna::ReviewVerdict"}]},
    {"id": "intent", "transport": "unix",
     "publishers":  [{"instance": "membrane-0", "topic": "dna::IntentOffered"}],
     "subscribers": [{"instance": "organism-0", "topic": "dna::IntentOffered"}]}]"#;
    let claims = r#"[
    {"name": "verdicts_reach_the_organism", "require_subscribes": {"group": "organism", "subject": "dna.review.verdict"}},
    {"name": "the_client_publishes_verdicts", "require_publishes": {"group": "client", "subject": "dna.review.verdict"}},
    {"name": "intent_reaches_the_organism", "require_subscribes": {"group": "organism", "subject": "dna.intent.offered"}}]"#;
    let (ok, out) = plan(routes, claims);
    assert!(ok, "the membrane routes and their laws hold:\n{out}");
    // A route with a phantom producer — the client declares the topic
    // (it imports the core) but nothing in its artifact publishes it —
    // is refused. The client's artifact carries the core's loci too, so
    // the phantom is a topic none of them publishes: `RecordResumed` is
    // the host's to publish, and the host is not imported here.
    let phantom = r#"[
    {"id": "resumes", "transport": "unix",
     "publishers":  [{"instance": "membrane-0", "topic": "dna::RecordResumed"}],
     "subscribers": [{"instance": "organism-0", "topic": "dna::RecordResumed"}]}]"#;
    let (ok, out) = plan(phantom, "[]");
    assert!(!ok, "a phantom producer must be refused:\n{out}");
    assert!(out.contains("RecordResumed") || out.contains("dna.record.resumed"), "the refusal names the phantom topic:\n{out}");
    let _ = std::fs::remove_dir_all(&dir);
}

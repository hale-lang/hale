//! GH #527 B6 / GH #986 — the typed control channel, now the nerves.
//!
//! What these hold the design to:
//!   1. A `bindings { }` entry may name an IMPORTED topic
//!      (`alias::Topic`): the organism binds the DNA core's own
//!      declarations, it does not re-declare them.
//!   2. The edge is a declared route in the fleet model: `hale fleet
//!      check` admits it over the organism's artifact and the host's
//!      (`dna/host`, the `main locus Host` whose `NodeRelay` publishes
//!      the fact topics — `hale dna task create` and `hale dna review`
//!      exec it), and refuses a route naming a topic the host never
//!      publishes.
//!
//! That a verdict and an intent published onto the nerves cross the
//! stream and the owning locus decides is proven end to end by the DNA
//! fixtures (`dna/tests/nerves_fake_test.hl`,
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
    let dir = std::env::temp_dir().join(format!("hale_nerves_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The reference organism, beside a copy of `dna/core` (and what it
/// imports beneath it, pond's driver — GH #985): it binds its two typed
/// topics to pond's NATS adapter directly (GH #986), so — unlike the
/// membrane's Unix sockets — there is no per-test path to relocate.
fn organism_fixture(root: &Path) -> PathBuf {
    let core_src = repo().join("dna/core");
    let core_dst = root.join("dna/core");
    std::fs::create_dir_all(&core_dst).unwrap();
    fn copy_hl(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for e in std::fs::read_dir(from).unwrap() {
            let e = e.unwrap();
            if e.path().is_dir() {
                copy_hl(&e.path(), &to.join(e.file_name()));
            } else if e.path().extension().and_then(|x| x.to_str()) == Some("hl") {
                std::fs::copy(e.path(), to.join(e.file_name())).unwrap();
            }
        }
    }
    copy_hl(&core_src, &core_dst);
    let seed = root.join("dna/organism");
    std::fs::create_dir_all(&seed).unwrap();
    std::fs::copy(repo().join("dna/organism/main.hl"), seed.join("main.hl")).unwrap();
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
fn the_nerves_edge_is_a_declared_route_in_the_fleet_model() {
    let dir = workdir("fleet");
    let seed = organism_fixture(&dir);
    let org = dir.join("organism.topology");
    let out = hale().arg("check").arg(&seed).arg(format!("--dump-topology={}", org.display())).output().unwrap();
    assert!(org.is_file(), "organism artifact: {}", String::from_utf8_lossy(&out.stderr));
    // the host, not a membrane client: `NodeRelay` (dna/host/host.hl)
    // publishes every fact topic once its row is in the record
    let host_artifact = dir.join("host.topology");
    let out = hale()
        .arg("check")
        .arg(repo().join("dna/host"))
        .arg(format!("--dump-topology={}", host_artifact.display()))
        .output()
        .unwrap();
    assert!(host_artifact.is_file(), "host artifact: {}", String::from_utf8_lossy(&out.stderr));
    let plan = |routes: &str, claims: &str| -> (bool, String) {
        let p = dir.join("plan.json");
        std::fs::write(
            &p,
            format!(
                r#"{{"schema": "1.0", "name": "organism-with-host",
  "instances": [
    {{"id": "organism-0", "artifact": "organism.topology", "labels": ["organism"]}},
    {{"id": "host-0", "artifact": "host.topology", "labels": ["host"]}}],
  "routes": {routes},
  "groups": {{"organism": {{"labels": ["organism"]}}, "host": {{"labels": ["host"]}}}},
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
    {"id": "verdicts", "transport": "nats",
     "publishers":  [{"instance": "host-0", "topic": "dna::ReviewVerdict"}],
     "subscribers": [{"instance": "organism-0", "topic": "dna::ReviewVerdict"}]},
    {"id": "intent", "transport": "nats",
     "publishers":  [{"instance": "host-0", "topic": "dna::IntentOffered"}],
     "subscribers": [{"instance": "organism-0", "topic": "dna::IntentOffered"}]}]"#;
    let claims = r#"[
    {"name": "verdicts_reach_the_organism", "require_subscribes": {"group": "organism", "subject": "dna.review.verdict"}},
    {"name": "the_host_publishes_verdicts", "require_publishes": {"group": "host", "subject": "dna.review.verdict"}},
    {"name": "intent_reaches_the_organism", "require_subscribes": {"group": "organism", "subject": "dna.intent.offered"}}]"#;
    let (ok, out) = plan(routes, claims);
    assert!(ok, "the nerves' routes and their laws hold:\n{out}");
    // A route with a phantom producer — the host does not publish
    // RecordResumed — is refused.
    let phantom = r#"[
    {"id": "resumes", "transport": "nats",
     "publishers":  [{"instance": "host-0", "topic": "dna::RecordResumed"}],
     "subscribers": [{"instance": "organism-0", "topic": "dna::RecordResumed"}]}]"#;
    let (ok, out) = plan(phantom, "[]");
    assert!(!ok, "a phantom producer must be refused:\n{out}");
    assert!(out.contains("RecordResumed") || out.contains("dna.record.resumed"), "the refusal names the phantom topic:\n{out}");
    let _ = std::fs::remove_dir_all(&dir);
}

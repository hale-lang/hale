//! GH #527 B6 — the typed control channel.
//!
//! What these hold the design to:
//!   1. A `bindings { }` entry may name an IMPORTED topic
//!      (`alias::Topic`): the organism binds the DNA core's own
//!      declarations, it does not re-declare them.
//!   2. A verdict and an intent published by the observer (`hale iris
//!      --membrane`) cross a unix socket into the organism, where the
//!      OWNING locus decides: a verdict with the wrong authority is
//!      refused by the Review, one with the right authority settles
//!      it; an intent births a Task through the membrane gate.
//!      Nothing crosses through the observation segment.
//!   3. The edge is a declared route in the fleet model: `hale fleet
//!      check` admits it over the two artifacts, and refuses a route
//!      naming a topic the observer never publishes.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
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

fn http(port: u16, req: &str) -> String {
    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    s.write_all(req.as_bytes()).unwrap();
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    out
}

fn post(port: u16, path: &str, body: &str) -> String {
    http(
        port,
        &format!(
            "POST {path} HTTP/1.0\r\nHost: x\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        ),
    )
}

fn wait_for(deadline: Instant, mut cond: impl FnMut() -> bool) -> bool {
    while Instant::now() < deadline {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    cond()
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
fn verdict_and_intent_cross_the_membrane_and_the_organism_decides() {
    let dir = workdir("e2e");
    let seed = organism_fixture(&dir);
    let out = hale().arg("build").arg(&seed).output().unwrap();
    assert!(out.status.success(), "organism build: {}", String::from_utf8_lossy(&out.stderr));
    let mut organism = Command::new(seed.join("organism"))
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn organism");
    let lines: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    {
        let lines = lines.clone();
        let stdout = organism.stdout.take().unwrap();
        std::thread::spawn(move || {
            for l in BufReader::new(stdout).lines().map_while(Result::ok) {
                lines.lock().unwrap().push(l);
            }
        });
    }
    let saw = |needle: &str| -> bool { lines.lock().unwrap().iter().any(|l| l.contains(needle)) };
    let vsock = dir.join("hale-dna.review.verdict.sock");
    let isock = dir.join("hale-dna.intent.offered.sock");
    assert!(
        wait_for(Instant::now() + Duration::from_secs(15), || vsock.exists() && isock.exists()),
        "the organism realizes both listen bindings: {:?}",
        lines.lock().unwrap()
    );

    let cache = std::env::temp_dir().join("hale-tests-iris-cache");
    let port = free_port();
    let mut iris = hale()
        .args(["iris", &port.to_string(), "--membrane"])
        .arg(&dir)
        .env("XDG_CACHE_HOME", &cache)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn hale iris --membrane");
    let ok = wait_for(Instant::now() + Duration::from_secs(40), || {
        TcpStream::connect(("127.0.0.1", port)).is_ok()
            && http(port, "GET /snapshot HTTP/1.0\r\nHost: x\r\n\r\n").contains("\"membrane\"")
    });
    let finish = |organism: &mut std::process::Child, iris: &mut std::process::Child| {
        let _ = iris.kill();
        let _ = iris.wait();
        let _ = organism.kill();
        let _ = organism.wait();
    };
    if !ok {
        finish(&mut organism, &mut iris);
        panic!("iris did not come up with a membrane");
    }

    // 1. intent -> the membrane gate -> a Task is born.
    let r = post(port, "/ctl/intent", r#"{"intent_id":"i1","outcome":"write the changelog","from":"riley"}"#);
    assert!(r.contains("200") && r.contains("dna.intent.offered"), "{r}");
    let born = wait_for(Instant::now() + Duration::from_secs(15), || saw("intent accepted, task t1 born"));

    // 2. a verdict with the wrong authority is REFUSED by the Review.
    let r = post(port, "/ctl/review", r#"{"review_id":"r1","subject_digest":"sha256:purpose-v1","verdict":"approve","reviewer":"riley","authority":"agent"}"#);
    assert!(r.contains("200"), "{r}");
    let refused = wait_for(Instant::now() + Duration::from_secs(15), || saw("refused a verdict: authority agent does not satisfy maintainer"));

    // 3. the right authority settles it.
    let r = post(port, "/ctl/review", r#"{"review_id":"r1","subject_digest":"sha256:purpose-v1","verdict":"approve","reviewer":"riley","authority":"maintainer","comment":"ok"}"#);
    assert!(r.contains("200"), "{r}");
    let settled = wait_for(Instant::now() + Duration::from_secs(15), || saw("review r1 settled approve by riley"));

    // 4. a malformed request never becomes a publish.
    let r = post(port, "/ctl/review", r#"{"verdict":"approve"}"#);
    // the counters land on the fusion loop's next snapshot tick
    let mut snap = String::new();
    let dl = Instant::now() + Duration::from_secs(10);
    while Instant::now() < dl {
        snap = http(port, "GET /snapshot HTTP/1.0\r\nHost: x\r\n\r\n");
        if snap.contains("\"verdicts\":2") && snap.contains("\"intents\":1") {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    finish(&mut organism, &mut iris);
    let transcript = lines.lock().unwrap().join("\n");
    assert!(born, "task born via the membrane:\n{transcript}");
    assert!(refused, "authority checked by the owning locus:\n{transcript}");
    assert!(settled, "the Review settled on the admitted verdict:\n{transcript}");
    assert!(r.contains("400"), "{r}");
    assert!(snap.contains("\"verdicts\":2") && snap.contains("\"intents\":1"), "{snap}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_membrane_edge_is_a_declared_route_in_the_fleet_model() {
    let dir = workdir("fleet");
    let seed = organism_fixture(&dir);
    let org = dir.join("organism.topology");
    let out = hale().arg("check").arg(&seed).arg(format!("--dump-topology={}", org.display())).output().unwrap();
    assert!(org.is_file(), "organism artifact: {}", String::from_utf8_lossy(&out.stderr));
    let obs = dir.join("iris.topology");
    let out = hale()
        .arg("check")
        .arg(repo().join("iris/consumer/fuse-hl"))
        .arg(format!("--dump-topology={}", obs.display()))
        .output()
        .unwrap();
    assert!(obs.is_file(), "observer artifact: {}", String::from_utf8_lossy(&out.stderr));
    let plan = |routes: &str, claims: &str| -> (bool, String) {
        let p = dir.join("plan.json");
        std::fs::write(
            &p,
            format!(
                r#"{{"schema": "1.0", "name": "organism-with-iris",
  "instances": [
    {{"id": "organism-0", "artifact": "organism.topology", "labels": ["organism"]}},
    {{"id": "iris-0",     "artifact": "iris.topology",     "labels": ["observer"]}}],
  "routes": {routes},
  "groups": {{"organism": {{"labels": ["organism"]}}, "observer": {{"labels": ["observer"]}}}},
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
     "publishers":  [{"instance": "iris-0",     "topic": "dna::ReviewVerdict"}],
     "subscribers": [{"instance": "organism-0", "topic": "dna::ReviewVerdict"}]},
    {"id": "intent", "transport": "unix",
     "publishers":  [{"instance": "iris-0",     "topic": "dna::IntentOffered"}],
     "subscribers": [{"instance": "organism-0", "topic": "dna::IntentOffered"}]}]"#;
    let claims = r#"[
    {"name": "verdicts_reach_the_organism", "require_subscribes": {"group": "organism", "subject": "dna.review.verdict"}},
    {"name": "the_observer_publishes_verdicts", "require_publishes": {"group": "observer", "subject": "dna.review.verdict"}},
    {"name": "intent_reaches_the_organism", "require_subscribes": {"group": "organism", "subject": "dna.intent.offered"}}]"#;
    let (ok, out) = plan(routes, claims);
    assert!(ok, "the membrane routes and their laws hold:\n{out}");
    // A route with a phantom producer — the observer declares the
    // topic (it imports the core) but nobody publishes it — is refused.
    // (`ReviewRequested` is published by the assembly since GH #566 F2;
    // `ConcernRaised` is still declared and unpublished.)
    let phantom = r#"[
    {"id": "concerns", "transport": "unix",
     "publishers":  [{"instance": "iris-0",     "topic": "dna::ConcernRaised"}],
     "subscribers": [{"instance": "organism-0", "topic": "dna::ConcernRaised"}]}]"#;
    let (ok, out) = plan(phantom, "[]");
    assert!(!ok, "a phantom producer must be refused:\n{out}");
    let _ = std::fs::remove_dir_all(&dir);
}

//! GH #583 M4 — the recorded fixture: a workspace of three services
//! under a plan with claims across them (`dna/acceptance/trio`), whose
//! organization is driven by scripted asks end to end — a change
//! deployed to two nodes, the organization grown by a supervisor under
//! pressure, a cross-service change denied by the fleet's law — every
//! model call answered from the checked-in tape
//! (`dna/acceptance/trio.fixture/tape`, its catalog beside it), keyless.
//! This is the acceptance for every later change to the organization.
//! GH #583 K4 — the learning scenario: the worker observes a recurring
//! condition and raises a concern; the node puts it in the record; the
//! organization proposes it; the Board ratifies the exact digest; the
//! next change to the trio is made with it in hand, and the editor's
//! evidence names the package.
//!
//! Re-record (a key in the environment, the tape rewritten):
//!   HALE_DNA_TAPE=record cargo test --release -p hale-cli --test dna_recorded_fixture

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

type Row = (u64, String, String, String);

fn git(args: &[&str], cwd: &Path) -> String {
    let out = Command::new("git").args(["-c", "user.name=riley", "-c", "user.email=r@l"]).args(args).current_dir(cwd).output().expect("git");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn journal(app: &Path) -> Vec<Row> {
    let out = Command::new("git").args(["-C", &app.to_string_lossy(), "show", "refs/dna/journal:journal.jsonl"]).output().unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .map(|v| {
            let s = |k: &str| v[k].as_str().unwrap_or("").to_string();
            (v["seq"].as_u64().unwrap_or(0), s("kind"), s("entity"), s("body"))
        })
        .collect()
}

fn wait_row(app: &Path, secs: u64, pred: impl Fn(&Row) -> bool) -> bool {
    let dl = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < dl {
        if journal(app).iter().any(&pred) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    false
}

fn dump(app: &Path) -> String {
    journal(app).iter().map(|(q, k, e, b)| format!("{q} {k} {e} {}", b.chars().take(160).collect::<String>())).collect::<Vec<_>>().join("\n")
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dst = to.join(e.file_name());
        if e.path().is_dir() {
            copy_dir(&e.path(), &dst);
        } else {
            std::fs::copy(e.path(), &dst).unwrap();
        }
    }
}

struct Fixture {
    d: PathBuf,
    app: PathBuf,
    bare: PathBuf,
    edges: Vec<PathBuf>,
    procs: Vec<std::process::Child>,
    tape: PathBuf,
    mode: String,
    kport: u16, // the knowledge service (K4), in memory over the record
}

impl Fixture {
    fn cmd(&self, args: &[&str], cwd: &Path) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_hale"));
        c.args(args)
            .current_dir(cwd)
            .env("HALE_BIN", env!("CARGO_BIN_EXE_hale"))
            .env("XDG_CACHE_HOME", std::env::temp_dir().join("hale-tests-iris-cache"))
            .env("HALE_DNA_TAPE", &self.mode)
            .env("HALE_DNA_TAPE_DIR", &self.tape)
            .env("HALE_DNA_KNOWLEDGE_DSN", "memory")
            .env("HALE_DNA_KNOWLEDGE_PORT", self.kport.to_string())
            .env("HALE_DNA_KNOWLEDGE_URL", format!("http://127.0.0.1:{}", self.kport));
        c
    }
    fn hale(&self, args: &[&str], cwd: &Path) -> (bool, String) {
        let out = self.cmd(args, cwd).output().expect("hale");
        (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
    }
    fn spawn(&mut self, args: &[&str], cwd: &Path) {
        let log = std::fs::File::create(self.d.join(format!("{}.stderr", args.iter().take(2).map(|a| a.replace('/', "_")).collect::<Vec<_>>().join("-")))).unwrap();
        let c = self.cmd(args, cwd).stdout(Stdio::null()).stderr(log).spawn().expect("spawn");
        self.procs.push(c);
    }
    fn logs(&self) -> String {
        let mut out = String::new();
        if let Ok(rd) = std::fs::read_dir(&self.d) {
            for f in rd.flatten() {
                if f.path().extension().map(|x| x == "stderr").unwrap_or(false) {
                    out.push_str(&format!("--- {}\n{}\n", f.file_name().to_string_lossy(), std::fs::read_to_string(f.path()).unwrap_or_default()));
                }
            }
        }
        if let Ok(log) = std::fs::read_to_string(self.app.join(".hale/dna/org.log")) {
            out.push_str(&format!("--- org.log\n{}\n", log.chars().rev().take(6000).collect::<String>().chars().rev().collect::<String>()));
        }
        out
    }
    fn stop(&mut self) {
        for p in self.procs.iter_mut() {
            let _ = p.kill();
            let _ = p.wait();
        }
        std::thread::sleep(Duration::from_millis(300));
        for f in ["org.pid", "app.pid", "knowledge.pid"] {
            if let Ok(pid) = std::fs::read_to_string(self.app.join(".hale/dna").join(f)) {
                let _ = Command::new("kill").args(["-9", pid.trim()]).status();
            }
        }
        for (i, e) in self.edges.iter().enumerate() {
            let nd = e.join(".hale/node").join(format!("edge-{}", i + 1));
            if let Ok(rd) = std::fs::read_dir(&nd) {
                for f in rd.flatten() {
                    if f.path().extension().map(|x| x == "pid").unwrap_or(false) {
                        if let Ok(pid) = std::fs::read_to_string(f.path()) {
                            let _ = Command::new("kill").args(["-9", pid.trim()]).status();
                        }
                    }
                }
            }
        }
    }
    fn fail(&mut self, why: &str) -> ! {
        let dump = dump(&self.app);
        let logs = self.logs();
        self.stop();
        panic!("{why}\n--- record\n{dump}\n{logs}");
    }
}

/// The trio with its DNA and the fixture's catalog, an origin, two node
/// clones, the organization and both nodes up, the base deployed.
fn bring_up() -> Fixture {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let mode = std::env::var("HALE_DNA_TAPE").unwrap_or_else(|_| "replay".to_string());
    let tape = repo.join("dna/acceptance/trio.fixture/tape");
    if mode == "record" {
        let _ = std::fs::remove_dir_all(&tape);
    }
    std::fs::create_dir_all(&tape).unwrap();
    let d = std::env::temp_dir().join(format!("hale_dna_trio_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    // the workspace name is part of what the organization is generated
    // from (its project's name), so it is fixed: the tape depends on it
    let app = d.join("trio");
    copy_dir(&repo.join("dna/acceptance/trio"), &app);
    let kport = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let mut f = Fixture { d: d.clone(), app: app.clone(), bare: d.join("origin.git"), edges: vec![d.join("edge-1"), d.join("edge-2")], procs: vec![], tape, mode, kport };
    git(&["init", "-q", "-b", "main"], &app);
    git(&["add", "-A"], &app);
    git(&["commit", "-q", "-m", "the trio and its fleet"], &app);
    let (ok, out) = f.hale(&["dna", "init", "."], &app);
    assert!(ok, "init: {out}");
    std::fs::write(app.join("dna/org/models.hl"), std::fs::read_to_string(repo.join("dna/acceptance/trio.fixture/catalog.hl")).unwrap()).unwrap();
    let (ok, out) = f.hale(&["check", "--matrix", "."], &app);
    assert!(ok, "the trio and its organization check: {out}");
    git(&["add", "-A"], &app);
    git(&["commit", "-q", "-m", "the organization"], &app);
    git(&["init", "-q", "--bare", "-b", "main", &f.bare.to_string_lossy()], &d);
    git(&["remote", "add", "origin", &f.bare.to_string_lossy()], &app);
    git(&["push", "-q", "origin", "main", "refs/dna/*:refs/dna/*"], &app);
    for e in &f.edges {
        git(&["clone", "-q", &f.bare.to_string_lossy(), &e.to_string_lossy()], &d);
    }
    // the knowledge service over the record (K4), then the organization
    // with its URL, then the nodes
    f.spawn(&["dna", "knowledge", ".", "--port", &kport.to_string()], &app);
    f.spawn(&["dna", "run", ".", "--no-iris", "--observe", "4"], &app);
    let edges = f.edges.clone();
    for (i, e) in edges.iter().enumerate() {
        f.spawn(&["node", &format!("edge-{}", i + 1), "--repo", &e.to_string_lossy(), "--tick", "300"], &d);
    }
    let dl = Instant::now() + Duration::from_secs(90);
    while !app.join(".hale/dna/hale-dna.intent.offered.sock").exists() && Instant::now() < dl {
        std::thread::sleep(Duration::from_millis(200));
    }
    if !app.join(".hale/dna/hale-dna.intent.offered.sock").exists() {
        f.fail("the membrane did not come up");
    }
    let (ok, out) = f.hale(&["dna", "deploy", "HEAD"], &app);
    if !ok || !out.contains("touching gateway-0 gateway-1 api-0 worker-0") {
        f.fail(&format!("deploy: {out}"));
    }
    let base = git(&["rev-parse", "HEAD"], &app);
    for id in ["gateway-0", "gateway-1", "api-0", "worker-0"] {
        if !wait_row(&app, 180, |(_, k, e, b)| k == "instance.up" && e == id && b.contains(&base)) {
            f.fail(&format!("{id} did not come up at the base"));
        }
    }
    f
}

#[test]
fn three_services_two_nodes_and_a_grown_organization_replay_from_the_tape() {
    let mut f = bring_up();
    let app = f.app.clone();

    // ---- 0. the learning scenario (K4): the worker on edge-2 observes
    //         its mail backlog and raises a concern, three times; the
    //         node puts each into the record; the host relays; the
    //         organization proposes it as knowledge for org/trio; the
    //         Board ratifies the exact digest; the service tails it
    if !wait_row(&app, 240, |(_, k, e, _)| k == "concern.proposed" && e == "org/trio/worker") {
        f.fail("the worker's concern did not become a proposal");
    }
    let rows = journal(&app);
    let requested = rows.iter().filter(|(_, k, e, _)| k == "concern.requested" && e == "org/trio/worker").count();
    let raised = rows.iter().filter(|(_, k, e, _)| k == "concern.raised" && e == "org/trio/worker").count();
    assert!(requested >= 3 && raised >= 3, "three concerns travelled from the node into the record and onto the membrane: requested {requested}, raised {raised}");
    assert!(rows.iter().any(|(_, k, e, b)| k == "concern.requested" && e == "org/trio/worker" && b.contains("\"node\": \"edge-2\"")), "the node that heard it is named");
    let kprop = rows.iter().find(|(_, k, _, b)| k == "knowledge.proposed" && b.contains("\"author\": \"org/trio/worker\"")).expect("the proposal");
    let kdigest = kprop.2.clone();
    assert!(kprop.3.contains("\"class\": \"concern\"") && kprop.3.contains("\"target\": \"org/trio\""), "a concern by the tower rule, bound to the application: {}", kprop.3);
    let (ok, reviews) = f.hale(&["dna", "review"], &app);
    let kreview = format!("k:{}", &kdigest[7..19]);
    assert!(ok && reviews.contains(&kreview) && reviews.contains("mail backlog behind fulfilment") && reviews.contains("needs board"), "the Board's queue has the concern:\n{reviews}");
    let (ok, out) = f.hale(&["dna", "review", &kreview, "approve", "--as", "riley", "--authority", "board", "--comment", "true, and worth knowing"], &app);
    if !ok {
        f.fail(&format!("ratify: {out}"));
    }
    if !wait_row(&app, 60, |(_, k, e, _)| k == "knowledge.ratified" && *e == kdigest) {
        f.fail("the concern was not ratified");
    }

    // ---- 1. a change to the gateway, deployed to both nodes — made with
    //         the ratified concern in hand: the organization consults
    //         the service for org/trio, folds it into the editor's
    //         objective, and the editor's evidence names the package
    let (ok, out) = f.hale(&["dna", "ask", "document", "the", "Gateway", "locus", "in", "main.hl", "with", "a", "doc", "comment", "saying", "what", "it", "takes", "and", "where", "it", "hands", "it"], &app);
    if !ok {
        f.fail(&format!("ask: {out}"));
    }
    if !wait_row(&app, 240, |(_, k, e, _)| k == "review.requested" && e == "review:m1") {
        f.fail("m1 did not reach its Review");
    }
    let (ok, out) = f.hale(&["dna", "review", "m1", "approve", "--as", "riley", "--comment", "documented"], &app);
    if !ok {
        f.fail(&format!("approve m1: {out}"));
    }
    if !wait_row(&app, 240, |(_, k, e, _)| k == "mutation.retained" && e == "m1") {
        f.fail("m1 was not retained");
    }
    let rows = journal(&app);
    // knowledge changed later work: the consult, the package on the evidence
    let consulted = rows.iter().find(|(_, k, e, _)| k == "knowledge.consulted" && e == "m1").unwrap_or_else(|| panic!("m1 consulted the service:\n{}", dump(&app)));
    assert!(consulted.3.contains("\"target\": \"org/trio\"") && consulted.3.contains("\"included_n\": 1") && consulted.3.contains(&format!("\"included\": \"{kdigest}\"")), "the package for the trio carries the ratified concern: {}", consulted.3);
    let ev = rows.iter().find(|(_, k, e, b)| k == "model.called" && e == "m1/a0" && b.contains("\"knowledge_bindings\": \"package:")).unwrap_or_else(|| panic!("the editor's evidence names the package:\n{}", dump(&app)));
    assert!(ev.3.contains(&kdigest), "and the concern's digest: {}", ev.3);
    let cand1 = rows.iter().find(|(_, k, e, _)| k == "mutation.candidate" && e == "m1").map(|r| r.3.clone()).unwrap_or_default();
    let deploy = rows.iter().find(|(_, k, e, b)| k == "fleet.deploy" && e == "m1" && b.contains("\"reason\": \"apply\"")).cloned();
    let Some(deploy) = deploy else { f.fail("no fleet.deploy for m1") };
    let db: serde_json::Value = serde_json::from_str(&deploy.3).unwrap();
    assert_eq!(db["touched"], serde_json::json!(["gateway-0", "gateway-1"]), "a change to the gateway touches its two instances, one per node: {}", deploy.3);
    // a node expresses the whole revision it checks out, so every
    // instance comes up at the candidate; the two the change touched
    // are the ones the window is judged over, one per node
    let ups: Vec<&Row> = rows.iter().filter(|(q, k, e, b)| *q > deploy.0 && k == "instance.up" && e.starts_with("gateway-") && b.contains(&cand1)).collect();
    assert_eq!(ups.len(), 2, "both gateway instances came up at the candidate:\n{}", dump(&app));
    let node_of = |b: &str| serde_json::from_str::<serde_json::Value>(b).unwrap()["node"].as_str().unwrap().to_string();
    assert!(ups.iter().any(|r| node_of(&r.3) == "edge-1") && ups.iter().any(|r| node_of(&r.3) == "edge-2"), "one per node");
    let observed = rows.iter().find(|(_, k, e, _)| k == "expression.observed" && e == "m1").expect("observed");
    assert!(observed.3.contains("healthy") && observed.3.contains("2 instance(s) up"), "the window was judged over the two touched instances: {}", observed.3);
    let live = git(&["show", "HEAD:main.hl"], &app);
    assert!(live != std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../dna/acceptance/trio/main.hl")).unwrap(), "the gateway changed");

    // ---- 2. pressure from the worker grows the organization: a supervisor
    let org_pid_before = std::fs::read_to_string(app.join(".hale/dna/org.pid")).unwrap_or_default();
    for _ in 0..3 {
        let (ok, out) = f.hale(&["dna", "pressure", "raise", "worker", "mail", "backlog", "behind", "fulfilment"], &app);
        if !ok {
            f.fail(&format!("pressure: {out}"));
        }
        std::thread::sleep(Duration::from_millis(400));
    }
    if !wait_row(&app, 300, |(_, k, e, _)| k == "review.requested" && e == "review:m2") {
        f.fail("the organization's growth did not reach its Review");
    }
    let (ok, board) = f.hale(&["dna", "board"], &app);
    assert!(ok && board.contains("m2"), "the Board's queue lists the growth: {board}");
    let (ok, out) = f.hale(&["dna", "review", "m2", "approve", "--as", "riley", "--comment", "grow it"], &app);
    if !ok {
        f.fail(&format!("approve m2: {out}"));
    }
    if !wait_row(&app, 300, |(_, k, e, _)| k == "mutation.retained" && e == "m2") {
        f.fail("m2 was not retained");
    }
    let rows = journal(&app);
    let m2 = rows.iter().find(|(_, k, e, _)| k == "review.requested" && e == "review:m2").unwrap();
    assert!(m2.3.contains("\"change_class\": \"organization\"") && m2.3.contains("\"seed\": \"dna/org\""), "{}", m2.3);
    let org_main = git(&["show", "HEAD:dna/org/main.hl"], &app);
    assert!(org_main.matches("dna::Leader {").count() >= 2, "a new position in the organization's main:\n{org_main}");
    let org_pid_after = std::fs::read_to_string(app.join(".hale/dna/org.pid")).unwrap_or_default();
    assert!(!org_pid_before.is_empty() && org_pid_before != org_pid_after, "the organization was restarted with its new position");
    assert!(!rows.iter().any(|(_, k, e, _)| k == "mutation.failed" && e == "m1"), "a retained Mutation whose worktree is gone is not 'in flight' to the restarted organization:\n{}", dump(&app));

    // ---- 3. a change to one service that breaks the fleet's law is
    //         denied, though the service itself still checks: the gateway
    //         renames the subject the api is routed on
    let (ok, out) = f.hale(&["dna", "ask", "in", "main.hl", "change", "the", "Orders", "topic's", "subject", "from", "\"trio.orders\"", "to", "\"trio.orders.v2\"", "and", "nothing", "else"], &app);
    if !ok {
        f.fail(&format!("ask: {out}"));
    }
    if !wait_row(&app, 240, |(_, k, e, b)| k == "mutation.deny" && e == "m3" && b.contains("breaks the fleet")) {
        f.fail("m3 was not denied by the fleet's law");
    }
    let rows = journal(&app);
    let deny = rows.iter().find(|(_, k, e, _)| k == "mutation.deny" && e == "m3").unwrap();
    assert!(deny.3.contains("check=0") && deny.3.contains("fleet=1"), "the service checks, the fleet does not: {}", deny.3);
    assert!(!rows.iter().any(|(_, k, e, _)| k == "review.requested" && e == "review:m3"), "a denied candidate is never a Review");

    // ---- the tape answered everything. A model call's evidence is
    // journaled by the assembly's bus handler, so it lands AFTER the
    // mutation row the test waited on; wait for the last attempt's
    // rows and re-read, rather than judging an older snapshot.
    let calls_landed = wait_row(&app, 60, |(_, k, e, _)| k == "model.called" && e == "m3/a0");
    let (ok, status) = f.hale(&["dna", "status"], &app);
    f.stop();
    assert!(ok, "{status}");
    assert!(calls_landed, "the last attempt's evidence reached the record:\n{}", dump(&app));
    let rows = journal(&app);
    let calls: Vec<&Row> = rows.iter().filter(|(_, k, _, _)| k == "model.called").collect();
    assert!(calls.len() >= 6, "model calls happened:\n{}", dump(&app));
    if f.mode == "replay" {
        for c in &calls {
            assert!(c.3.contains("\"adapter\": \"recorded\""), "every call was replayed: {}", c.3);
            assert!(!c.3.contains("tape miss"), "no miss: {}", c.3);
        }
    }
    let _ = std::fs::remove_dir_all(&f.d);
}

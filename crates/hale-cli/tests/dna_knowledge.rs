//! GH #583 K1 — the knowledge graph as a service. `init` writes
//! `dna/compose.yaml`; `hale dna dev` runs the knowledge service beside
//! the organization against HALE_DNA_KNOWLEDGE_DSN (here `memory`, so
//! CI needs no database; the Postgres store is exercised by
//! `dna/tests/knowledge_store_test.hl` under a DSN); the service tails
//! the record: a proposal ratified by the Board becomes a context
//! package for the position it binds to, with a digest and the record
//! revision, over HTTP. `hale dna knowledge` runs the same service in
//! the foreground and refuses without a DSN. K2: an ask makes the
//! organization consult the service and fold the practice into the
//! editor's objective (`knowledge.consulted`); three concerns from one
//! path become a proposal bound to its parent, the Board's to ratify.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn hale(args: &[&str], cwd: &Path, env: &[(&str, &str)]) -> (bool, String) {
    let mut c = Command::new(env!("CARGO_BIN_EXE_hale"));
    c.args(args).current_dir(cwd).env("HALE_BIN", env!("CARGO_BIN_EXE_hale")).env("XDG_CACHE_HOME", std::env::temp_dir().join("hale-tests-iris-cache"));
    for (k, v) in env {
        c.env(k, v);
    }
    let out = c.output().expect("hale");
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn http(port: u16, req: &str) -> String {
    let Ok(mut s) = TcpStream::connect(("127.0.0.1", port)) else { return String::new() };
    let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = s.write_all(req.as_bytes());
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    out
}

fn body(resp: &str) -> String {
    resp.split("\r\n\r\n").nth(1).unwrap_or("").to_string()
}

fn journal(app: &Path) -> Vec<(String, String, String)> {
    let out = Command::new("git").args(["-C", &app.to_string_lossy(), "show", "refs/dna/journal:journal.jsonl"]).output().unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .map(|v| {
            let s = |k: &str| v[k].as_str().unwrap_or("").to_string();
            (s("kind"), s("entity"), s("body"))
        })
        .collect()
}

fn wait_row(app: &Path, secs: u64, kind: &str, entity: &str) -> bool {
    let dl = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < dl {
        if journal(app).iter().any(|(k, e, _)| k == kind && e == entity) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    false
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

const PROPOSER: &str = r#"// a proposal into the record, the way a supervisor position would make one
import "vendor/dna" as dna;

fn main() {
    let core = dna::Dna {
        journal: dna::GitJournal { repo: "." },
        verification: dna::HaleVerification { receipts: dna::GitReceipts { repo: "." }, repo: "." },
        membrane: dna::Board { who: "board" }
    };
    // the Board's practice for the application: `org` is the root, the
    // application is `org/knowing`, so it is a goal that reaches every
    // path under the application
    let d = core.propose_knowledge(dna::Idea { id: "p1", kind: "practice", text: "retry a mail send once before raising pressure", author: "org" }, "org/knowing");
    println(d);
}
"#;

#[test]
fn init_writes_compose_and_dev_runs_the_knowledge_service_that_tails_the_record() {
    let d = std::env::temp_dir().join(format!("hale_dna_knowledge_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let (ok, out) = hale(&["dna", "new", "knowing"], &d, &[]);
    assert!(ok, "{out}");
    let app: PathBuf = d.join("knowing");
    // init wrote the compose file, a named volume per repository
    let compose = std::fs::read_to_string(app.join("dna/compose.yaml")).expect("dna/compose.yaml");
    assert!(compose.contains("image: pgvector/pgvector:pg16") && compose.contains("name: hale-dna-knowing-knowledge") && compose.contains("127.0.0.1:54"), "{compose}");
    assert!(out.contains("knowledge dna/compose.yaml: `hale dna dev` brings its Postgres up"), "{out}");
    // the service in the foreground refuses without a DSN
    let (ok, out) = hale(&["dna", "knowledge", "."], &app, &[("HALE_DNA_KNOWLEDGE_DSN", "")]);
    assert!(!ok && out.contains("HALE_DNA_KNOWLEDGE_DSN is not set"), "{out}");

    // the record: a proposal, made by a position, then the Board's verdict
    Command::new("git").args(["-c", "user.name=t", "-c", "user.email=t@l", "add", "-A"]).current_dir(&app).output().unwrap();
    Command::new("git").args(["-c", "user.name=t", "-c", "user.email=t@l", "commit", "-q", "-m", "genome"]).current_dir(&app).output().unwrap();
    std::fs::create_dir_all(app.join("propose")).unwrap();
    std::fs::write(app.join("propose/main.hl"), PROPOSER).unwrap();
    let (ok, out) = hale(&["run", "propose"], &app, &[]);
    assert!(ok, "proposer: {out}");
    let digest = out.lines().rev().find(|l| l.starts_with("sha256:")).expect("the digest").trim().to_string();
    let (ok, reviews) = hale(&["dna", "review"], &app, &[]);
    assert!(ok && reviews.contains(&format!("k:{}", &digest[7..19])) && reviews.contains("needs board"), "the knowledge Review is pending for the Board:\n{reviews}");

    // dev: the organization, the application and the knowledge service (in memory here)
    let kport = free_port();
    let mut host = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["dna", "dev", ".", "--no-iris"])
        .current_dir(&app)
        .env("HALE_BIN", env!("CARGO_BIN_EXE_hale"))
        .env("XDG_CACHE_HOME", std::env::temp_dir().join("hale-tests-iris-cache"))
        .env("HALE_DNA_KNOWLEDGE_DSN", "memory")
        .env("HALE_DNA_KNOWLEDGE_PORT", kport.to_string())
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(d.join("dev.stderr")).unwrap())
        .spawn()
        .expect("hale dna dev");
    let stop = |host: &mut std::process::Child| {
        for f in ["org.pid", "app.pid", "knowledge.pid"] {
            if let Ok(pid) = std::fs::read_to_string(app.join(".hale/dna").join(f)) {
                let _ = Command::new("kill").args(["-9", pid.trim()]).status();
            }
        }
        let _ = host.kill();
        let _ = host.wait();
    };
    let dl = Instant::now() + Duration::from_secs(120);
    let mut summary = String::new();
    // the service comes up before the organization (its environment
    // names the service), so wait for the membrane as well
    let membrane = app.join(".hale/dna/hale-dna.review.verdict.sock");
    while Instant::now() < dl {
        let s = body(&http(kport, "GET / HTTP/1.0\r\nHost: x\r\n\r\n"));
        if s.contains("\"store\": \"memory\"") && s.contains("\"ideas\": 1") && membrane.exists() {
            summary = s;
            break;
        }
        if let Ok(Some(st)) = host.try_wait() {
            let log = std::fs::read_to_string(d.join("dev.stderr")).unwrap_or_default();
            panic!("hale dna dev exited early: {st}\n{log}");
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    if summary.is_empty() {
        let log = std::fs::read_to_string(d.join("dev.stderr")).unwrap_or_default();
        stop(&mut host);
        panic!("the knowledge service did not tail the proposal:\n{log}");
    }
    assert!(std::fs::read_to_string(app.join(".hale/dna/knowledge.dsn")).unwrap().trim() == "memory", "the host recorded the DSN it used");
    // proposed, not ratified: no package yet
    let pkg = body(&http(kport, "GET /context?target=org%2Fknowing%2Fmailer&budget=8 HTTP/1.0\r\nHost: x\r\n\r\n"));
    assert!(pkg.contains("\"included_n\": 0"), "nothing ratified yet: {pkg}");
    // the Board ratifies the exact digest
    let (ok, out) = hale(&["dna", "review", &format!("k:{}", &digest[7..19]), "approve", "--as", "riley", "--authority", "board", "--comment", "a good practice"], &app, &[]);
    assert!(ok, "verdict: {out}");
    let dl = Instant::now() + Duration::from_secs(60);
    let mut pkg = String::new();
    while Instant::now() < dl {
        pkg = body(&http(kport, "GET /context?target=org%2Fknowing%2Fmailer&budget=8 HTTP/1.0\r\nHost: x\r\n\r\n"));
        if pkg.contains("\"included_n\": 1") {
            break;
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    let idea = body(&http(kport, &format!("GET /idea/{digest} HTTP/1.0\r\nHost: x\r\n\r\n")));
    let sibling = body(&http(kport, "GET /context?target=org%2Fother&budget=8 HTTP/1.0\r\nHost: x\r\n\r\n"));

    // ---- K2: knowledge changes later work. An ask on the application:
    // the organization consults the service for org/knowing and folds the
    // ratified practice into the editor's objective (the editor here has
    // no key, so the attempt fails after — the consult is what is asserted)
    let (ok, out) = hale(&["dna", "ask", "document", "the", "Echo", "locus", "in", "main.hl"], &app, &[]);
    assert!(ok, "ask: {out}");
    let consulted = wait_row(&app, 60, "knowledge.consulted", "m1");
    // ---- K2: a concern raised three times from a path under the
    // application becomes a proposal by that path, bound to its parent
    for _ in 0..3 {
        let (ok, out) = hale(&["dna", "concern", "raise", "org/knowing/echo", "pings", "arrive", "twice", "under", "load", "--severity", "2"], &app, &[]);
        assert!(ok && out.contains("concern raised by org/knowing/echo"), "{out}");
        std::thread::sleep(Duration::from_millis(300));
    }
    let proposed = wait_row(&app, 60, "concern.proposed", "org/knowing/echo");
    let rows = journal(&app);
    let (ok, reviews) = hale(&["dna", "review"], &app, &[]);
    let log = std::fs::read_to_string(d.join("dev.stderr")).unwrap_or_default();
    stop(&mut host);
    assert!(consulted, "the organization consulted the service for the ask:\n{}", rows.iter().map(|(k, e, b)| format!("{k} {e} {}", b.chars().take(120).collect::<String>())).collect::<Vec<_>>().join("\n"));
    let c = rows.iter().find(|(k, e, _)| k == "knowledge.consulted" && e == "m1").unwrap();
    assert!(c.2.contains("\"target\": \"org/knowing\"") && c.2.contains("\"included_n\": 1") && c.2.contains(&format!("\"included\": \"{digest}\"")), "the package for the application, with the ratified practice: {}", c.2);
    assert!(proposed, "three concerns became a proposal");
    let kp = rows.iter().filter(|(k, _, b)| k == "knowledge.proposed" && b.contains("\"class\": \"concern\"")).count();
    assert_eq!(kp, 1, "one concern proposed, by the source, bound to org/knowing");
    assert!(ok && reviews.contains("pings arrive twice under load") && reviews.contains("needs board"), "the concern's Review is the Board's:\n{reviews}");
    // the package: the ratified practice reaches the target's children,
    // with a digest and the revision, and the idea's text and author
    assert!(pkg.contains("\"included_n\": 1") && pkg.contains(&format!("\"included\": \"{digest}\"")) && pkg.contains("\"digest\": \"sha256:") && pkg.contains("\"revision\": "), "the package after ratification:\n{pkg}\n{log}");
    assert!(pkg.contains("\"text\": \"retry a mail send once before raising pressure\"") && pkg.contains("\"author\": \"org\""), "{pkg}");
    assert!(idea.contains("\"accepted\": true") && idea.contains("\"provenance\": \"ratified\""), "{idea}");
    assert!(sibling.contains("\"included_n\": 0"), "a sibling position sees nothing: {sibling}");
    assert!(log.contains("knowledge service (pid") && log.contains("over an in-memory store"), "{log}");
    let _ = std::fs::remove_dir_all(&d);
}

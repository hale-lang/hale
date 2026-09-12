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
    http_within(port, req, 15)
}

/// A request whose answer may take a while: a store that is down is
/// discovered by a connect timeout, and the service answers only after.
fn http_within(port: u16, req: &str, secs: u64) -> String {
    let Ok(mut s) = TcpStream::connect(("127.0.0.1", port)) else { return String::new() };
    let _ = s.set_read_timeout(Some(Duration::from_secs(secs)));
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

/// Proposes a practice and ratifies it in the same program: the Review
/// is a live child of the Dna, so the verdict travels the ordinary way
/// and the record ends with `knowledge.ratified`. The text is an
/// ordinary paragraph — a newline and a tab — which the driver's rows
/// carry unescaped (finding 9).
const RATIFIER: &str = r#"import "vendor/dna" as dna;

main locus App {
    params {
        core: dna::Dna = dna::Dna {
            journal: dna::GitJournal { repo: "." },
            verification: dna::HaleVerification { receipts: dna::GitReceipts { repo: "." }, repo: "." },
            membrane: dna::Board { who: "board" }
        };
    }
    bus { publish dna::ReviewVerdict; }
    run() {
        let d = self.core.propose_knowledge(dna::Idea { id: "p1", kind: "practice", text: "First line
Second line	with a tab", author: "org" }, "org/served");
        dna::ReviewVerdict <- dna::Verdict { review_id: dna::knowledge_review_id(d), subject_digest: d, verdict: "approve", reviewer: "riley", authority: "board" };
        std::time::sleep(500ms);
        println(d);
    }
}

fn main() { App { }; }
"#;

/// A project with a record, for the service to tail.
fn seeded_app(tag: &str) -> (PathBuf, PathBuf, String) {
    let d = std::env::temp_dir().join(format!("hale_dna_ksvc_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let (ok, out) = hale(&["dna", "new", "served"], &d, &[]);
    assert!(ok, "{out}");
    let app = d.join("served");
    Command::new("git").args(["-c", "user.name=t", "-c", "user.email=t@l", "add", "-A"]).current_dir(&app).output().unwrap();
    Command::new("git").args(["-c", "user.name=t", "-c", "user.email=t@l", "commit", "-q", "-m", "genome"]).current_dir(&app).output().unwrap();
    std::fs::create_dir_all(app.join("propose")).unwrap();
    std::fs::write(app.join("propose/main.hl"), RATIFIER).unwrap();
    let (ok, out) = hale(&["run", "propose"], &app, &[]);
    assert!(ok, "proposer: {out}");
    let digest = out.lines().rev().find(|l| l.starts_with("sha256:")).expect("the digest").trim().to_string();
    (d, app, digest)
}

/// Run the service in the foreground against a DSN, answer one request,
/// and stop it.
fn serve<T>(app: &Path, dsn: &str, port: u16, body_of: impl FnOnce(u16) -> T) -> (T, String) {
    let log = std::env::temp_dir().join(format!("hale_ksvc_{}_{}.log", std::process::id(), port));
    let mut c = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["dna", "knowledge", ".", "--port", &port.to_string()])
        .current_dir(app)
        .env("HALE_BIN", env!("CARGO_BIN_EXE_hale"))
        .env("XDG_CACHE_HOME", std::env::temp_dir().join("hale-tests-iris-cache"))
        .env("HALE_DNA_KNOWLEDGE_DSN", dsn)
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(&log).unwrap())
        .spawn()
        .expect("hale dna knowledge");
    // the first run builds the service into the toolchain cache, and a
    // sibling test may hold that build's lock
    // wait for the port to ACCEPT, not for a reply: the reply may be
    // held up by the store the service is about to discover is down
    let dl = Instant::now() + Duration::from_secs(300);
    while Instant::now() < dl && TcpStream::connect(("127.0.0.1", port)).is_err() {
        if let Ok(Some(st)) = c.try_wait() {
            let log = std::fs::read_to_string(&log).unwrap_or_default();
            panic!("hale dna knowledge exited early ({st}):\n{log}");
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    let out = body_of(port);
    let _ = c.kill();
    let _ = c.wait();
    (out, std::fs::read_to_string(&log).unwrap_or_default())
}

/// GH #583 K1, shakeout finding 4 — the service opens its store, and says
/// so when it cannot. `Pq.open()` is what connects and migrates; nothing
/// called it, so the Postgres path answered 200 with an empty graph and a
/// normal-looking digest while every query failed. `Mem.open()` is a
/// no-op, which is why every test passed.
#[test]
fn the_service_opens_its_store_and_refuses_to_answer_when_it_cannot() {
    let (d, app, _digest) = seeded_app("down");
    // a database that is not there: a port nothing listens on
    let closed = TcpListener::bind("127.0.0.1:0").unwrap();
    let dead_port = closed.local_addr().unwrap().port();
    drop(closed);
    let dsn = format!("postgres://dna:dna@127.0.0.1:{dead_port}/dna?sslmode=disable");
    let ((summary, ctx), log) = serve(&app, &dsn, free_port(), |p| {
        (
            body(&http_within(p, "GET / HTTP/1.0\r\nHost: x\r\n\r\n", 240)),
            http_within(p, "GET /context?target=org%2Fserved&budget=8 HTTP/1.0\r\nHost: x\r\n\r\n", 240),
        )
    });
    let _ = std::fs::remove_dir_all(&d);
    // the surface is up even though the store is not: that is where an
    // operator reads what is wrong
    assert!(summary.contains("\"store\": \"postgres\"") && summary.contains("\"open\": false"), "the summary reports the store is not open: {summary}\n{log}");
    assert!(summary.contains("the store is not open: open:"), "and why: {summary}");
    // a package request is refused, not answered with an empty package
    assert!(ctx.starts_with("HTTP/1.0 503") || ctx.starts_with("HTTP/1.1 503"), "a package is refused while the store is down: {ctx}");
    assert!(body(&ctx).contains("the store is not open"), "{ctx}");
    assert!(log.contains("postgres at 127.0.0.1"), "the service named the store it was pointed at: {log}");
}

/// The same service against a real Postgres, when one is configured
/// (CI's service container; a developer's compose). This is the path the
/// fixtures replaced with the memory store.
#[test]
fn the_service_serves_a_real_postgres() {
    let Ok(dsn) = std::env::var("HALE_DNA_KNOWLEDGE_DSN") else {
        eprintln!("the_service_serves_a_real_postgres: no HALE_DNA_KNOWLEDGE_DSN; skipped");
        return;
    };
    if !dsn.starts_with("postgres") {
        eprintln!("the_service_serves_a_real_postgres: DSN is not postgres; skipped");
        return;
    }
    let (d, app, digest) = seeded_app("pg");
    // A database of this run's own where the server will grant one. The
    // tables here are global to a database, and the native knowledge
    // fixture reads the same DSN: run unpartitioned, the two wipe each
    // other's rows (the review's second round, the isolation note). The
    // shared DSN is the fallback, wiped as before.
    let own = format!("hale_dna_svc_{}", std::process::id());
    let made = Command::new("psql")
        .args([&dsn, "-v", "ON_ERROR_STOP=1", "-c", &format!("DROP DATABASE IF EXISTS {own}"), "-c", &format!("CREATE DATABASE {own}")])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    let dsn = if made { with_database(&dsn, &own) } else { dsn.clone() };
    if !made {
        let wipe = Command::new("psql").args([&dsn, "-v", "ON_ERROR_STOP=1", "-c", "DROP TABLE IF EXISTS knowledge_bindings, knowledge_edges, knowledge_ideas, knowledge_meta, knowledge_structure, knowledge_signals"]).output();
        assert!(wipe.map(|o| o.status.success()).unwrap_or(false), "psql is needed to reset the database for this test");
    }
    let kill = dsn.clone();
    let ((summary, ctx, idea, after_kill, ctx2, broken, broken_summary, healed), log) = serve(&app, &dsn, free_port(), |p| {
        // the service applies the record on every request
        let s = body(&http(p, "GET / HTTP/1.0\r\nHost: x\r\n\r\n"));
        let c = body(&http(p, "GET /context?target=org%2Fserved%2Fmain&budget=8 HTTP/1.0\r\nHost: x\r\n\r\n"));
        let i = body(&http(p, &format!("GET /idea/{digest} HTTP/1.0\r\nHost: x\r\n\r\n")));
        // The connection dies while the database stays healthy — a
        // restart, a failover, an idle timeout. `opened` was sticky, so
        // the service went on reporting `open` and answering 200 with an
        // empty package and a normal-looking digest until someone
        // restarted it by hand (the review's second round, finding 4).
        let _ = Command::new("psql")
            .args([&kill, "-v", "ON_ERROR_STOP=1", "-c", "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE pid <> pg_backend_pid() AND datname = current_database()"])
            .output();
        let k = body(&http(p, "GET / HTTP/1.0\r\nHost: x\r\n\r\n"));
        let c2 = body(&http(p, "GET /context?target=org%2Fserved%2Fmain&budget=8 HTTP/1.0\r\nHost: x\r\n\r\n"));
        // A live connection that cannot answer the question. `SELECT 1`
        // says the session is healthy; it says nothing about whether
        // the knowledge queries work. Failing reads used to reach the
        // caller as an empty package with a well-formed digest — "there
        // is no knowledge here" (the third-round review's finding 3).
        // The schema is taken away here as controlled fault injection.
        let hide = |sql: &str| {
            let o = Command::new("psql").args([&kill, "-v", "ON_ERROR_STOP=1", "-c", sql]).output().expect("psql");
            assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        };
        hide("ALTER TABLE knowledge_ideas RENAME TO knowledge_ideas_hidden");
        let broken = http(p, "GET /context?target=org%2Fserved%2Fmain&budget=8 HTTP/1.0\r\nHost: x\r\n\r\n");
        let broken_summary = body(&http(p, "GET / HTTP/1.0\r\nHost: x\r\n\r\n"));
        hide("ALTER TABLE knowledge_ideas_hidden RENAME TO knowledge_ideas");
        let healed = body(&http(p, "GET /context?target=org%2Fserved%2Fmain&budget=8 HTTP/1.0\r\nHost: x\r\n\r\n"));
        (s, c, i, k, c2, broken, broken_summary, healed)
    });
    if made {
        let _ = Command::new("psql").args([&std::env::var("HALE_DNA_KNOWLEDGE_DSN").unwrap(), "-c", &format!("DROP DATABASE IF EXISTS {own}")]).output();
    }
    let _ = std::fs::remove_dir_all(&d);
    assert!(summary.contains("\"store\": \"postgres\"") && summary.contains("\"open\": true"), "the store is open: {summary}\n{log}");
    assert!(summary.contains("\"error\": \"\""), "and nothing failed: {summary}");
    // the record reached the database: the schema, the watermark, the projections
    let n = |k: &str| -> i64 {
        summary.split(&format!("\"{k}\": ")).nth(1).and_then(|t| t.split(|c: char| !c.is_ascii_digit()).next()).and_then(|t| t.parse().ok()).unwrap_or(-1)
    };
    assert!(n("watermark") > 0 && n("watermark") == n("record"), "the whole record was applied: {summary}");
    assert!(n("ratified") == 1 && n("bindings") == 1 && n("structure") > 0, "ideas, bindings and structure are in Postgres: {summary}");
    // the paragraph survives the driver's tab-separated rows (finding 9)
    assert!(idea.contains("\"text\": \"First line\\nSecond line\\twith a tab\"") && idea.contains("\"author\": \"org\"") && idea.contains("\"accepted\": true"), "the idea round-trips intact: {idea}");
    assert!(ctx.contains("\"included_n\": 1") && ctx.contains(&digest) && ctx.contains("Second line"), "and reaches the package: {ctx}");
    // the connection died and the service dialled again by itself: the
    // same knowledge, not an empty package with a well-formed digest
    assert!(after_kill.contains("\"open\": true") && after_kill.contains("\"error\": \"\""), "the service re-established its connection: {after_kill}\n{log}");
    assert!(after_kill.contains(&format!("\"watermark\": {}", n("watermark"))), "and is where it was: {after_kill}");
    assert!(ctx2.contains("\"included_n\": 1") && ctx2.contains(&digest), "the package survives a lost connection: {ctx2}\n{log}");
    // a query the database cannot run is a refusal, not an empty answer
    assert!(broken.starts_with("HTTP/1.0 503") || broken.starts_with("HTTP/1.1 503"), "a store that cannot answer is refused, not answered: {broken}\n{log}");
    assert!(body(&broken).contains("knowledge_ideas") || body(&broken).contains("the store cannot answer"), "and says what failed: {}", body(&broken));
    assert!(!broken_summary.contains("\"error\": \"\""), "the summary carries the failure: {broken_summary}");
    // and it is the same service that answers again once it can
    assert!(healed.contains("\"included_n\": 1") && healed.contains(&digest), "the practice comes back without a restart: {healed}\n{log}");
}

/// The DSN with a different database, keeping user, host, port and
/// parameters: `postgres://u:p@h:5432/<db>?sslmode=disable`.
fn with_database(dsn: &str, db: &str) -> String {
    let (head, query) = match dsn.find('?') {
        Some(i) => (&dsn[..i], &dsn[i..]),
        None => (dsn, ""),
    };
    let authority_end = head.find("//").map(|i| i + 2).unwrap_or(0);
    match head[authority_end..].find('/') {
        Some(i) => format!("{}/{db}{query}", &head[..authority_end + i]),
        None => format!("{head}/{db}{query}"),
    }
}

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
    // ---- K3: the projections and ranking, from the service
    std::thread::sleep(Duration::from_millis(500));
    let structure = body(&http(kport, "GET /structure HTTP/1.0\r\nHost: x\r\n\r\n"));
    let signals = body(&http(kport, "GET /signals HTTP/1.0\r\nHost: x\r\n\r\n"));
    let ranked = body(&http(kport, "GET /context?target=org%2Fknowing%2Fmailer&budget=1&query=retry%20the%20mail%20send HTTP/1.0\r\nHost: x\r\n\r\n"));
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
    // the code's structure as init observed it, projected by kind and name
    assert!(structure.contains("\"loci_names\": \"") && structure.contains("Echo") && structure.contains("\"topic_names\": \"") && structure.contains("Pings"), "the application's loci and topics: {structure}");
    // the concerns, counted per source
    assert!(signals.contains("\"kind\": \"concern\", \"source\": \"org/knowing/echo\", \"what\": \"pings arrive twice under load\", \"count\": 3"), "three concerns counted: {signals}");
    // ranked inside the bound, by what the work is about
    assert!(ranked.contains("\"ranked\": true") && ranked.contains("\"included_n\": 1") && ranked.contains(&digest), "ranked: {ranked}");
    // the package: the ratified practice reaches the target's children,
    // with a digest and the revision, and the idea's text and author
    assert!(pkg.contains("\"included_n\": 1") && pkg.contains(&format!("\"included\": \"{digest}\"")) && pkg.contains("\"digest\": \"sha256:") && pkg.contains("\"revision\": "), "the package after ratification:\n{pkg}\n{log}");
    assert!(pkg.contains("\"text\": \"retry a mail send once before raising pressure\"") && pkg.contains("\"author\": \"org\""), "{pkg}");
    assert!(idea.contains("\"accepted\": true") && idea.contains("\"provenance\": \"ratified\""), "{idea}");
    assert!(sibling.contains("\"included_n\": 0"), "a sibling position sees nothing: {sibling}");
    assert!(log.contains("knowledge service (pid") && log.contains("over an in-memory store"), "{log}");
    let _ = std::fs::remove_dir_all(&d);
}

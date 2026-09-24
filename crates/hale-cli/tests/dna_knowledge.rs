//! GH #583 K1 — the knowledge graph, in memory. `init` writes
//! `dna/compose.yaml`; `hale dna dev` applies memory's schema with the
//! owner's DSN and runs the organization's host as the spine, which
//! projects the record into memory on its tick (GH #985): a proposal
//! ratified by the Board becomes a context package for the position it
//! binds to, with a digest and the record revision, read by a head under
//! its own role. K2: an ask makes the organization consult memory and
//! fold the practice into the editor's objective (`knowledge.consulted`);
//! three concerns from one path become a proposal bound to its parent,
//! the Board's to ratify. K3: structure, signals and ranking, from memory.

#[path = "support/reap.rs"]
mod reap;
#[path = "support/trace.rs"]
mod trace;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

fn hale(args: &[&str], cwd: &Path, env: &[(&str, &str)]) -> (bool, String) {
    let _s = trace::Span::new("hale", args.join(" "));
    let mut c = Command::new(env!("CARGO_BIN_EXE_hale"));
    c.args(args).current_dir(cwd).env("HALE_BIN", env!("CARGO_BIN_EXE_hale"))
        .env("HALE_DNA_DISCOVER", "off").env("XDG_CACHE_HOME", std::env::temp_dir().join("hale-tests-iris-cache"));
    for (k, v) in env {
        c.env(k, v);
    }
    let out = c.output().expect("hale");
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn journal(app: &Path) -> Vec<(String, String, String)> {
    let _s = trace::Span::new("git", "show refs/dna/journal:journal.jsonl");
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
    trace::wait_until(format!("{kind} {entity}"), Duration::from_secs(secs), Duration::from_millis(300), || journal(app).iter().any(|(k, e, _)| k == kind && e == entity))
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

/// A head's read of memory, printed one tab-separated line per answer:
/// the package for `KT` (budget `KB`, ranked by `KQ` when set), the idea
/// `KI`, the ideas counted, the structure projected and the signals.
const READER: &str = r#"import "vendor/dna" as dna;

fn env(name: String) -> String { return if std::env::var_exists(name) { std::env::var(name) } else { "" }; }

fn main() {
    let budget = std::str::parse_int(env("KB")) or 8;
    let k = dna::MemoryKnowledge { repo: ".", dsn_env: "HALE_DNA_MEMORY_DSN_HEAD", budget: budget };
    let p = k.package_for(env("KT"), env("KQ"));
    println("pkg\t" + p.error + "\t" + to_string(p.included_n) + "\t" + p.included + "\t" + p.digest + "\t" + to_string(p.revision));
    let j = dna::GitJournal { repo: "." };
    let store = dna::Pq { dsn_env: "HALE_DNA_MEMORY_DSN_HEAD" };
    store.scope(j.genesis());
    if !store.open() { println("store\t" + store.last_error()); return; }
    let s = store.idea(env("KI"));
    println("idea\t" + (if s.present { "present" } else { "absent" }) + "\t" + (if s.idea.accepted { "accepted" } else { "not accepted" }) + "\t" + s.idea.provenance + "\t" + s.idea.author);
    println("ideas\t" + to_string(store.count("ideas")));
    println("structure\t" + store.structure_names("locus") + "|" + store.structure_names("topic"));
    println("signals\t" + store.signals());
    println("text\t" + p.text);
}
"#;

/// The reader's answer for one field (`pkg`, `idea`, `ideas`, …).
fn read_memory(app: &Path, head: &str, target: &str, budget: &str, query: &str, idea: &str) -> String {
    std::fs::create_dir_all(app.join("reader")).unwrap();
    std::fs::write(app.join("reader/main.hl"), READER).unwrap();
    let (_, out) = hale(&["run", "reader"], app, &[("HALE_DNA_MEMORY_DSN_HEAD", head), ("KT", target), ("KB", budget), ("KQ", query), ("KI", idea)]);
    out
}

fn field(out: &str, name: &str) -> String {
    out.lines().find_map(|l| l.strip_prefix(&format!("{name}\t"))).unwrap_or("").to_string()
}

#[test]
fn init_writes_compose_and_dev_projects_the_record_into_memory() {
    let _t = trace::test("dna_knowledge::dev");
    let Some(_owner) = std::env::var("HALE_DNA_MEMORY_DSN_OWNER").ok().filter(|d| !d.is_empty()) else {
        eprintln!("dna_knowledge: no HALE_DNA_MEMORY_DSN_OWNER; the graph is memory's, so nothing was exercised");
        return;
    };
    let d = std::env::temp_dir().join(format!("hale_dna_knowledge_{}", std::process::id()));
    let _reap = reap::ReapOnDrop(d.clone());
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let (ok, out) = hale(&["dna", "new", "knowing"], &d, &[]);
    assert!(ok, "{out}");
    let app: PathBuf = d.join("knowing");
    // init wrote the compose file, a named volume per repository
    let compose = std::fs::read_to_string(app.join("dna/compose.yaml")).expect("dna/compose.yaml");
    assert!(compose.contains("image: pgvector/pgvector:pg16") && compose.contains("name: hale-dna-knowing-knowledge") && compose.contains("127.0.0.1:54"), "{compose}");

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

    // dev: the organization and the application, the host as the spine
    let (ok, migrated) = hale(&["dna", "memory", "migrate"], &app, &[]);
    assert!(ok, "{migrated}");
    let head = migrated.lines().find_map(|l| l.strip_prefix("HALE_DNA_MEMORY_DSN_HEAD=")).expect("the head's DSN").to_string();
    let mut host = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["dna", "dev", ".", "--no-iris"])
        .current_dir(&app)
        .env("HALE_BIN", env!("CARGO_BIN_EXE_hale"))
        .env("HALE_DNA_DISCOVER", "off")
        .env("XDG_CACHE_HOME", std::env::temp_dir().join("hale-tests-iris-cache"))
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(d.join("dev.stderr")).unwrap())
        .spawn()
        .expect("hale dna dev");
    let stop = |host: &mut std::process::Child| {
        for f in ["org.pid", "app.pid"] {
            if let Ok(pid) = std::fs::read_to_string(app.join(".hale/dna").join(f)) {
                let _ = Command::new("kill").args(["-9", pid.trim()]).status();
            }
        }
        let _ = host.kill();
        let _ = host.wait();
    };
    // the eight seeded design practices and the six operating ones are
    // ideas too (GH #596 C, #994), plus this proposal; wait for the
    // membrane as well
    let membrane = app.join(".hale/dna/hale-dna.review.verdict.sock");
    let tailed = trace::wait_until("dna dev: the spine projected the record and the membrane bound", Duration::from_secs(180), Duration::from_millis(500), || {
        if let Ok(Some(st)) = host.try_wait() {
            panic!("hale dna dev exited early: {st}\n{}", std::fs::read_to_string(d.join("dev.stderr")).unwrap_or_default());
        }
        field(&read_memory(&app, &head, "org", "8", "", ""), "ideas") == "15" && membrane.exists()
    });
    if !tailed {
        let log = std::fs::read_to_string(d.join("dev.stderr")).unwrap_or_default();
        stop(&mut host);
        panic!("the spine did not project the proposal:\n{log}");
    }
    // proposed, not ratified: no package yet
    let pkg = field(&read_memory(&app, &head, "org/knowing/mailer", "8", "", ""), "pkg");
    assert!(pkg.split('\t').nth(1) == Some("0"), "nothing ratified yet: {pkg}");
    // the Board ratifies the exact digest
    let (ok, out) = hale(&["dna", "review", &format!("k:{}", &digest[7..19]), "approve", "--as", "riley", "--authority", "board", "--comment", "a good practice"], &app, &[]);
    assert!(ok, "verdict: {out}");
    let mut read = String::new();
    trace::wait_until("the ratified practice reached the package", Duration::from_secs(60), Duration::from_millis(300), || {
        read = read_memory(&app, &head, "org/knowing/mailer", "8", "", &digest);
        field(&read, "pkg").split('\t').nth(1) == Some("1")
    });
    let sibling = field(&read_memory(&app, &head, "org/other", "8", "", ""), "pkg");

    // ---- K2: knowledge changes later work. An ask on the application:
    // the organization consults memory for org/knowing and folds the
    // ratified practice into the editor's objective (the editor here has
    // no key, so the attempt fails after — the consult is what is asserted)
    let (ok, out) = hale(&["dna", "task", "create", "document", "the", "Echo", "locus", "in", "main.hl"], &app, &[]);
    assert!(ok, "ask: {out}");
    let consulted = wait_row(&app, 60, "knowledge.consulted", "m1");
    // ---- K2: a concern raised three times from a path under the
    // application becomes a proposal by that path, bound to its parent
    for _ in 0..3 {
        let (ok, out) = hale(&["dna", "concern", "raise", "org/knowing/echo", "pings", "arrive", "twice", "under", "load", "--severity", "2"], &app, &[]);
        assert!(ok && out.contains("concern raised by org/knowing/echo"), "{out}");
        trace::sleep("between concerns", Duration::from_millis(300));
    }
    let proposed = wait_row(&app, 60, "concern.proposed", "org/knowing/echo");
    // ---- K3: the projections and ranking, from memory; wait for the third
    // concern to be projected rather than for a fixed half second
    let mut k3 = String::new();
    trace::wait_until("the spine counted three concerns", Duration::from_secs(60), Duration::from_millis(300), || {
        k3 = read_memory(&app, &head, "org/knowing/mailer", "1", "retry the mail send", "");
        field(&k3, "signals").contains("\"count\":3")
    });
    let rows = journal(&app);
    let (ok, reviews) = hale(&["dna", "review"], &app, &[]);
    let log = std::fs::read_to_string(d.join("dev.stderr")).unwrap_or_default();
    stop(&mut host);
    assert!(consulted, "the organization consulted memory for the ask:\n{}", rows.iter().map(|(k, e, b)| format!("{k} {e} {}", b.chars().take(120).collect::<String>())).collect::<Vec<_>>().join("\n"));
    let c = rows.iter().find(|(k, e, _)| k == "knowledge.consulted" && e == "m1").unwrap();
    assert!(c.2.contains("\"target\": \"org/knowing\"") && c.2.contains("\"included_n\": 1") && c.2.contains(&format!("\"included\": \"{digest}\"")), "the package for the application, with the ratified practice: {}", c.2);
    assert!(proposed, "three concerns became a proposal");
    let kp = rows.iter().filter(|(k, _, b)| k == "knowledge.proposed" && b.contains("\"class\": \"concern\"")).count();
    assert_eq!(kp, 1, "one concern proposed, by the source, bound to org/knowing");
    assert!(ok && reviews.contains("pings arrive twice under load") && reviews.contains("needs board"), "the concern's Review is the Board's:\n{reviews}");
    // the code's structure as init observed it, projected by kind and name
    let structure = field(&k3, "structure");
    assert!(structure.contains("Echo") && structure.contains("Pings"), "the application's loci and topics: {structure}");
    // the concerns, counted per source
    assert!(field(&k3, "signals").contains("{\"kind\":\"concern\",\"source\":\"org/knowing/echo\",\"what\":\"pings arrive twice under load\",\"count\":3,"), "three concerns counted: {k3}");
    // ranked inside the bound, by what the work is about
    let ranked = field(&k3, "pkg");
    assert!(ranked.split('\t').nth(1) == Some("1") && ranked.contains(&digest), "ranked: {ranked}");
    // the package: the ratified practice reaches the target's children,
    // with a digest and the revision, and the idea's text and author
    let pkg = field(&read, "pkg");
    let f: Vec<&str> = pkg.split('\t').collect();
    assert!(f.len() == 5 && f[0].is_empty() && f[1] == "1" && f[2] == digest && f[3].starts_with("sha256:") && f[4].parse::<i64>().unwrap_or(0) > 0, "the package after ratification:\n{pkg}\n{log}");
    assert!(read.contains("retry a mail send once before raising pressure (org)"), "{read}");
    assert!(field(&read, "idea").starts_with("present\taccepted\tratified"), "{read}");
    assert!(sibling.split('\t').nth(1) == Some("0"), "a sibling position sees nothing: {sibling}");
    assert!(log.contains("memory: the graph, the ledger and protected evidence under the record's spine role"), "{log}");
    let _ = std::fs::remove_dir_all(&d);
}

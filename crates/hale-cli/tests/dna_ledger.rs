//! GH #646, #1590 — the Ledger. A record is born in memory with its
//! Ledger (routing 2, the cutover at its identity): an operational kind is
//! written straight into the Ledger under the writer's role, never into
//! the record, and a head that names no memory writes nothing. Readers of
//! history see one history across both memories. There is nothing to
//! adopt and nothing to abandon.

#[path = "support/vault.rs"]
mod vault;
#[path = "support/record.rs"]
mod record;
#[path = "support/reap.rs"]
mod reap;
#[path = "support/trace.rs"]
mod trace;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

fn hale(args: &[&str], cwd: &Path, env: &[(&str, &str)]) -> (bool, String) {
    let _s = trace::Span::new("hale", args.join(" "));
    let mut c = vault::hale();
    c.args(args).current_dir(cwd).env("HALE_BIN", env!("CARGO_BIN_EXE_hale")).env("HALE_DNA_DISCOVER", "off").env("XDG_CACHE_HOME", std::env::temp_dir().join("hale-tests-iris-cache"));
    for (k, v) in env {
        c.env(k, v);
    }
    let out = c.output().expect("hale");
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn record(app: &Path) -> Vec<serde_json::Value> {
    let _s = trace::Span::new("hale", "dna record-rows");
    record::text(app).lines().filter(|l| !l.trim().is_empty()).filter_map(|l| serde_json::from_str(l).ok()).collect()
}

/// Memory's owner for these organisms (CI's service container).
fn owner_dsn() -> Option<String> {
    std::env::var("HALE_DNA_MEMORY_DSN_OWNER").ok().filter(|d| !d.is_empty())
}

/// A role's DSN for `app`'s record (`HALE_DNA_MEMORY_DSN_SPINE=` or
/// `…_HEAD=`), from the owner's migration (GH #985).
fn memory_dsn(app: &Path, line: &str) -> String {
    let (ok, out) = hale(&["dna", "memory", "migrate"], app, &[]);
    assert!(ok, "memory migrates: {out}");
    out.lines().find_map(|l| l.strip_prefix(line)).unwrap_or_else(|| panic!("no {line} in {out}")).to_string()
}

/// The nerves' owner for these organisms (CI's NATS, GH #986).
fn nats_owner() -> Option<String> {
    std::env::var("HALE_DNA_NATS_URL_OWNER").ok().filter(|d| !d.is_empty())
}

/// The nerves a body runs on: the spine's URL and the organization's
/// token, from the owner's `nerves migrate` (GH #986) — a request a head
/// writes reaches the organization only over them.
fn nerves(app: &Path) -> (String, String) {
    let (ok, out) = hale(&["dna", "nerves", "migrate"], app, &[]);
    assert!(ok, "the nerves migrate: {out}");
    let line = |k: &str| out.lines().find_map(|l| l.strip_prefix(k)).unwrap_or_else(|| panic!("no {k} in {out}")).to_string();
    (line("HALE_DNA_NATS_URL_SPINE="), line("HALE_DNA_NATS_ORG="))
}

/// The spine: a body run on the spine's role and the nerves; its stderr in
/// `<name>.stderr`.
fn spine(app: &Path, d: &Path, spine_dsn: &str, name: &str) -> std::process::Child {
    let (url, org) = nerves(app);
    vault::hale()
        .args(["dna", "run", ".", "--no-iris"])
        .current_dir(app)
        .env("HALE_BIN", env!("CARGO_BIN_EXE_hale"))
        .env("HALE_DNA_DISCOVER", "off")
        .env("XDG_CACHE_HOME", std::env::temp_dir().join("hale-tests-iris-cache"))
        .env("HALE_DNA_MEMORY_DSN_SPINE", spine_dsn)
        .env("HALE_DNA_NATS_URL_SPINE", url)
        .env("HALE_DNA_NATS_ORG", org)
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(d.join(format!("{name}.stderr"))).unwrap())
        .spawn()
        .expect("hale dna run")
}

/// The host, then everything it started.
fn stop_host(app: &Path, host: &mut std::process::Child) {
    for f in ["org.pid", "app.pid"] {
        if let Ok(pid) = std::fs::read_to_string(app.join(".hale/dna").join(f)) {
            let _ = Command::new("kill").args(["-9", pid.trim()]).status();
        }
    }
    let _ = host.kill();
    let _ = host.wait();
}

/// The ledger's rows as a head reads them, once `pred` holds or two
/// minutes pass.
fn ledger_until(app: &Path, env: &[(&str, &str)], pred: impl Fn(&str) -> bool) -> String {
    let dl = std::time::Instant::now() + Duration::from_secs(120);
    loop {
        let (_, rows) = hale(&["dna", "ledger", "rows"], app, env);
        if pred(&rows) || std::time::Instant::now() > dl {
            return rows;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// This record's schema, roles and stream, gone again.
fn unmigrate(app: &Path, owner: &str) {
    record::unmigrate(app, owner);
    // and its stream on the nerves (GH #986), with the owner's URL
    let _ = vault::hale().args(["dna", "nerves", "drop", "."]).current_dir(app).env("HALE_DNA_DISCOVER", "off").output();
}

#[test]
fn an_organism_adopts_the_ledger_and_its_operations_leave_the_record() {
    let _t = trace::test("dna_ledger::adopts");
    let (Some(owner), Some(_)) = (owner_dsn(), nats_owner()) else {
        eprintln!("dna_ledger: no HALE_DNA_MEMORY_DSN_OWNER or HALE_DNA_NATS_URL_OWNER; the ledger is memory's, so nothing was exercised");
        return;
    };
    let d = std::env::temp_dir().join(format!("hale_dna_ledger_{}", std::process::id()));
    let _reap = reap::ReapOnDrop(d.clone());
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let (ok, out) = hale(&["dna", "new", "ledgered"], &d, &[]);
    assert!(ok, "{out}");
    let app: PathBuf = d.join("ledgered");
    Command::new("git").args(["-c", "user.name=t", "-c", "user.email=t@l", "add", "-A"]).current_dir(&app).output().unwrap();
    Command::new("git").args(["-c", "user.name=t", "-c", "user.email=t@l", "commit", "-q", "-m", "genome"]).current_dir(&app).output().unwrap();
    // the organism opens memory itself (GH #985): its template names no
    // ledger and no knowledge client
    let main = std::fs::read_to_string(app.join("dna/org/main.hl")).unwrap();
    assert!(main.contains("dna::RoutedJournal") && !main.contains("ServiceLedger") && !main.contains("KnowledgeClient"), "the organism reads the record and memory, wired by the core:\n{main}");

    let spine_dsn = memory_dsn(&app, "HALE_DNA_MEMORY_DSN_SPINE=");
    let head_dsn = memory_dsn(&app, "HALE_DNA_MEMORY_DSN_HEAD=");
    let head: &[(&str, &str)] = &[("HALE_DNA_MEMORY_DSN_HEAD", head_dsn.as_str())];
    // born with its Ledger: routing 2, its cutover stated in memory
    let (ok, st) = hale(&["dna", "ledger"], &app, head);
    assert!(ok && st.contains("routing:    2") && st.contains("row(s)") && !st.contains("states no cutover"), "{st}");
    assert!(record(&app).iter().any(|r| r["kind"] == "ledger.adopted"), "the record names its Ledger among its first rows");
    let (ok, again) = hale(&["dna", "ledger", "adopt"], &app, head);
    assert!(ok && again.contains("there is nothing to adopt"), "{again}");
    let (ok, ab) = hale(&["dna", "ledger", "abandon", "--why", "no", "--as", "riley"], &app, head);
    assert!(!ok && ab.contains("nothing to abandon"), "{ab}");

    // an operational row goes straight into the ledger, under the head's
    // role (GH #1026)
    let mut host = spine(&app, &d, &spine_dsn, "run");
    std::fs::write(app.join("second.txt"), "the second invoice").unwrap();
    let (ok, out) = hale(&["dna", "receipt", "file", "second.txt", "--as", "sam"], &app, head);
    assert!(ok, "{out}");
    let rows = ledger_until(&app, head, |r| r.contains("second.txt"));
    assert!(rows.contains("second.txt") && rows.contains("\"author\": \"sam\""), "the ledger took it, in its writer's name:\n{rows}");
    assert!(!record(&app).iter().any(|r| r["kind"] == "receipt.filed"), "the record holds none of it");
    // a head with no memory named writes nothing (the clone keeps the head's
    // DSN its migration answered, GH #1590: set aside here, it is a clone
    // with none)
    std::fs::write(app.join("third.txt"), "the third invoice").unwrap();
    let kept = app.join(".hale/dna/memory.env");
    let aside = app.join(".hale/dna/memory.env.aside");
    let had = std::fs::rename(&kept, &aside).is_ok();
    let (ok, out) = hale(&["dna", "receipt", "file", "third.txt", "--as", "sam"], &app, &[]);
    if had {
        std::fs::rename(&aside, &kept).unwrap();
    }
    assert!(!ok, "a head with no memory writes nothing: {out}");
    let rows = ledger_until(&app, head, |r| r.contains("second.txt"));
    assert!(!rows.contains("third.txt"), "and nothing landed for it:\n{rows}");
    // an evolutionary row goes to the record
    let (ok, out) = hale(&["dna", "practice", "propose", "billing/late", "--text", "Chase an invoice at seven days.", "--as", "riley"], &app, head);
    assert!(ok, "{out}");
    assert!(record(&app).iter().any(|r| r["kind"] == "practice.requested" || r["kind"] == "practice.proposed"), "the practice is in the record");
    // history reads one history across both memories
    let (ok, hist) = hale(&["dna", "history"], &app, head);
    assert!(ok && hist.contains("second.txt") && hist.contains("practice."), "one history:\n{hist}");

    // the body lease is a row of memory: a head reads it, a second body is refused
    let (ok, live) = hale(&["dna", "body"], &app, head);
    assert!(ok && live.contains("live on "), "a head reads the lease from memory: {live}");
    let (ok, second) = hale(&["dna", "run", ".", "--no-iris"], &app, &[("HALE_DNA_MEMORY_DSN_SPINE", spine_dsn.as_str())]);
    assert!(!ok && second.contains("lease"), "a second body is refused: {second}");
    // a body without its memory admits nothing (the head's DSN this clone
    // keeps, from which `run` derives the spine's, set aside: a clone with
    // none, GH #1590)
    let had = std::fs::rename(&kept, &aside).is_ok();
    let (ok, norun) = hale(&["dna", "run", ".", "--no-iris"], &app, &[]);
    if had {
        std::fs::rename(&aside, &kept).unwrap();
    }
    assert!(!ok && norun.contains("a body without its memory admits nothing"), "{norun}");
    stop_host(&app, &mut host);
    // the body is gone; its lease stands until forced, in memory
    let (ok, forced) = hale(&["dna", "body", "claim", "--force", "--as", "riley"], &app, head);
    assert!(ok && (forced.contains("body.claimed") || forced.contains("released")), "{forced}");
    let (ok, gone) = hale(&["dna", "body"], &app, head);
    assert!(ok && gone.contains("released"), "the forced claim released the lease in memory: {gone}");
    unmigrate(&app, &owner);
    let _ = std::fs::remove_dir_all(&d);
}



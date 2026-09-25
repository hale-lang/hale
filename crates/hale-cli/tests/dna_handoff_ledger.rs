//! GH #646, Riley's condition for stage 5: the git-based handoff keeps
//! working over the ledger. The firm has adopted the ledger (its
//! `handoff.*` and `task.*` rows live in the store behind the service);
//! the accountant is on the record alone. A task crosses from the firm
//! into the accountant's mailbox (a git ref, as before), the accountant
//! accepts it in its record, and the firm's `handoff sync` reads the
//! acceptance back and settles the task — in the ledger.

#[path = "support/reap.rs"]
mod reap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn hale(args: &[&str], cwd: &Path, env: &[(&str, &str)]) -> (bool, String) {
    let mut c = Command::new(env!("CARGO_BIN_EXE_hale"));
    c.args(args).current_dir(cwd).env("HALE_BIN", env!("CARGO_BIN_EXE_hale")).env("HALE_DNA_DISCOVER", "off").env("XDG_CACHE_HOME", std::env::temp_dir().join("hale-tests-iris-cache"));
    for (k, v) in env {
        c.env(k, v);
    }
    let out = c.output().expect("hale");
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn git(args: &[&str], cwd: &Path) -> String {
    let out = Command::new("git").args(args).current_dir(cwd).output().expect("git");
    assert!(out.status.success(), "git {args:?} in {}: {}", cwd.display(), String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn record(app: &Path) -> Vec<serde_json::Value> {
    let out = Command::new("git").args(["-C", &app.to_string_lossy(), "show", "refs/dna/journal:journal.jsonl"]).output().unwrap();
    String::from_utf8_lossy(&out.stdout).lines().filter(|l| !l.trim().is_empty()).filter_map(|l| serde_json::from_str(l).ok()).collect()
}

/// One row on the record by plumbing, the shape `GitRecord` writes.
fn plumb(app: &Path, kind: &str, entity: &str, body: &str, author: &str) {
    let head = git(&["rev-parse", "refs/dna/journal"], app);
    let text = git(&["show", "refs/dna/journal:journal.jsonl"], app);
    let seq = text.lines().filter(|l| !l.trim().is_empty()).count();
    let line = serde_json::json!({"seq": seq, "kind": kind, "entity": entity, "body": body, "author": author}).to_string();
    let tmp = app.join(".hale/dna/plumb.jsonl");
    std::fs::create_dir_all(tmp.parent().unwrap()).unwrap();
    std::fs::write(&tmp, format!("{text}\n{line}\n")).unwrap();
    let blob = git(&["hash-object", "-w", &tmp.to_string_lossy()], app);
    let idx = app.join(".hale/dna/plumb.index");
    let with_index = |args: &[&str]| {
        let out = Command::new("git").args(args).current_dir(app).env("GIT_INDEX_FILE", &idx).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    with_index(&["read-tree", &head]);
    with_index(&["update-index", "--add", "--cacheinfo", &format!("100644,{blob},journal.jsonl")]);
    let tree = with_index(&["write-tree"]);
    let commit = git(&["-c", "user.name=t", "-c", "user.email=t@l", "commit-tree", &tree, "-p", &head, "-m", &format!("{kind} {entity}")], app);
    git(&["update-ref", "refs/dna/journal", &commit, &head], app);
}

fn project(scratch: &Path, name: &str, user: &str) -> (PathBuf, PathBuf) {
    let (ok, out) = hale(&["dna", "new", name], scratch, &[]);
    assert!(ok, "{out}");
    let app = scratch.join(name);
    let bare = scratch.join(format!("{name}.git"));
    git(&["init", "-q", "--bare", "-b", "main", &bare.to_string_lossy()], scratch);
    git(&["config", "user.name", user], &app);
    git(&["config", "user.email", &format!("{user}@local")], &app);
    git(&["add", "-A"], &app);
    git(&["commit", "-q", "-m", "the app"], &app);
    git(&["remote", "add", "origin", &bare.to_string_lossy()], &app);
    git(&["push", "-q", "origin", "main"], &app);
    let (ok, out) = hale(&["dna", "sync"], &app, &[]);
    assert!(ok, "{out}");
    (app, bare)
}

/// Memory's owner for these organisms (CI's service container).
fn owner_dsn() -> Option<String> {
    std::env::var("HALE_DNA_MEMORY_DSN_OWNER").ok().filter(|d| !d.is_empty())
}

/// The head's DSN for `app`'s record, from the owner's migration.
fn head_dsn(app: &Path) -> String {
    let (ok, out) = hale(&["dna", "memory", "migrate"], app, &[]);
    assert!(ok, "memory migrates: {out}");
    out.lines().find_map(|l| l.strip_prefix("HALE_DNA_MEMORY_DSN_HEAD=")).expect("the head's DSN").to_string()
}

/// The spine: `hale dna dev` with the owner's DSN (it migrates and runs
/// the host under the spine's role), which adopts and admits.
fn spine(app: &Path, d: &Path) -> std::process::Child {
    Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["dna", "dev", ".", "--no-iris"])
        .current_dir(app)
        .env("HALE_BIN", env!("CARGO_BIN_EXE_hale"))
        .env("HALE_DNA_DISCOVER", "off")
        .env("XDG_CACHE_HOME", std::env::temp_dir().join("hale-tests-iris-cache"))
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(d.join("spine.stderr")).unwrap())
        .spawn()
        .expect("hale dna dev")
}

/// The ledger's rows as a head reads them, once `pred` holds or two
/// minutes pass.
fn ledger_until(app: &Path, env: &[(&str, &str)], pred: impl Fn(&str) -> bool) -> String {
    let dl = Instant::now() + Duration::from_secs(120);
    loop {
        let (_, rows) = hale(&["dna", "ledger", "rows"], app, env);
        if pred(&rows) || Instant::now() > dl {
            return rows;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// `hale dna status` once it contains `needle`, or after two minutes.
fn status_until(app: &Path, env: &[(&str, &str)], needle: &str) -> String {
    let dl = Instant::now() + Duration::from_secs(120);
    loop {
        let (_, st) = hale(&["dna", "status"], app, env);
        if st.contains(needle) || Instant::now() > dl {
            return st;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

fn word_after(text: &str, marker: &str) -> String {
    text.split_once(marker).map(|(_, r)| r.split_whitespace().next().unwrap_or("").to_string()).unwrap_or_default()
}

#[test]
fn a_handoff_crosses_from_an_adopted_firm_and_its_acceptance_settles_the_task_in_the_ledger() {
    let Some(_owner) = owner_dsn() else {
        eprintln!("dna_handoff_ledger: no HALE_DNA_MEMORY_DSN_OWNER; the ledger is memory's, so nothing was exercised");
        return;
    };
    let d = std::env::temp_dir().join(format!("hale_dna_handoff_ledger_{}", std::process::id()));
    let _reap = reap::ReapOnDrop(d.clone());
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let (firm, firm_bare) = project(&d, "firm", "riley");
    let (acct, acct_bare) = project(&d, "acct", "noor");
    // the firm's job, handed to a person, before adoption
    plumb(&firm, "task.born", "t7", "i-t7: prepare the year-end ledger", "dna");
    plumb(&firm, "task.handed", "t7", "{\"work\": \"t7/w\", \"assignee\": \"mara\", \"by\": \"leader\", \"narrative\": \"handed: not a software change\"}", "dna");
    let (ok, out) = hale(&["dna", "sync"], &firm, &[]);
    assert!(ok, "{out}");

    // the firm adopts the ledger: a head asks, the spine carries it out
    let head = head_dsn(&firm);
    let env: &[(&str, &str)] = &[("HALE_DNA_MEMORY_DSN_HEAD", head.as_str())];
    let mut service = spine(&firm, &d);
    let (ok, adopted) = hale(&["dna", "ledger", "adopt", "--as", "riley"], &firm, env);
    assert!(ok && adopted.contains("ledger adoption asked for"), "{adopted}");
    let dl = Instant::now() + Duration::from_secs(300);
    while Instant::now() < dl && !record(&firm).iter().any(|r| r["kind"] == "ledger.adopted") {
        let _ = hale(&["dna", "sync"], &firm, &[]);
        std::thread::sleep(Duration::from_millis(500));
    }
    assert!(record(&firm).iter().any(|r| r["kind"] == "ledger.adopted"), "the spine adopted the ledger: {}", std::fs::read_to_string(d.join("spine.stderr")).unwrap_or_default());
    let st = status_until(&firm, env, "t7 [handed]");
    assert!(st.contains("t7 [handed]"), "the handed task is read from the ledger:\n{st}");

    // a connection to the accountant, in force
    let (ok, proposed) = hale(&["dna", "connect", &acct_bare.to_string_lossy(), "--name", "acct", "--as", "accountant", "--purpose", "year-end books", "--classes", "internal", "--by", "riley"], &firm, env);
    assert!(ok && proposed.contains("connection acct proposed"), "{proposed}");
    let rid = word_after(&proposed, "hale dna review ");
    let (ok, out) = hale(&["dna", "review", &rid, "approve", "--as", "ana", "--authority", "board", "--no-wait"], &firm, env);
    assert!(ok, "{out}");
    let (ok, listed) = hale(&["dna", "connect"], &firm, env);
    assert!(ok && listed.contains("in force (approved by ana)"), "{listed}");

    // the task crosses: the envelope into the accountant's mailbox (git), the row into the ledger
    let (ok, handed) = hale(&["dna", "handoff", "acct", "task", "t7", "--as", "riley", "--note", "close by March"], &firm, env);
    assert!(ok && handed.contains("task t7 [internal] written into record"), "{handed}");
    let hid = word_after(&handed, "handoff ").trim_end_matches(':').to_string();
    let firm_genesis = git(&["rev-list", "--max-parents=0", "refs/dna/journal"], &firm);
    let mailbox = git(&["show", &format!("refs/dna/exchange/{firm_genesis}:journal.jsonl")], &acct_bare);
    assert!(mailbox.contains("handoff.received") && mailbox.contains(&hid), "the envelope is in the accountant's mailbox at the remote:\n{mailbox}");
    let published = format!("\"kind\": \"handoff.published\", \"entity\": \"handoff:{hid}\"");
    let rows = ledger_until(&firm, env, |r| r.contains(&published));
    assert!(rows.contains(&published), "the firm's row is the ledger's, admitted by the spine:\n{rows}");
    assert!(!record(&firm).iter().any(|r| r["kind"] == "handoff.published"), "and not the record's (only its request is)");
    let st = status_until(&firm, env, "t7 [transfer_requested]");
    assert!(st.contains("t7 [transfer_requested]"), "the task waits for acceptance:\n{st}");

    // the accountant, on the record alone, connects back and accepts
    let (ok, out) = hale(&["dna", "sync"], &acct, &[]);
    assert!(ok, "{out}");
    let (ok, back) = hale(&["dna", "connect", &firm_bare.to_string_lossy(), "--name", "firm", "--as", "client", "--purpose", "year-end books", "--classes", "internal", "--by", "noor"], &acct, &[]);
    assert!(ok, "{back}");
    let brid = word_after(&back, "hale dna review ");
    let (ok, out) = hale(&["dna", "review", &brid, "approve", "--as", "lee", "--authority", "board", "--no-wait"], &acct, &[]);
    assert!(ok, "{out}");
    let (ok, accepted) = hale(&["dna", "handoff", "accept", &hid, "--as", "noor", "--note", "taken on"], &acct, &[]);
    assert!(ok && accepted.contains("accepted by noor under connection `firm`"), "{accepted}");

    // the firm reads the acceptance back: the task settles, in the ledger
    let (ok, read_back) = hale(&["dna", "handoff", "sync"], &firm, env);
    assert!(ok && read_back.contains("1 acceptance(s) admitted"), "{read_back}");
    let st = status_until(&firm, env, "t7 [transfer_accepted]");
    assert!(st.contains("t7 [transfer_accepted]"), "settled:\n{st}");
    let rows = ledger_until(&firm, env, |r| r.contains("\"kind\": \"handoff.accepted_by_peer\""));
    assert!(rows.contains("\"kind\": \"task.transfer_accepted\", \"entity\": \"t7\"") && rows.contains("\"kind\": \"handoff.accepted_by_peer\""), "the settlement rows are the ledger's:\n{rows}");
    let (ok, again) = hale(&["dna", "handoff", "sync"], &firm, env);
    assert!(ok && again.contains("0 acceptance(s) admitted"), "admitted once: {again}");
    let _ = service.kill();
    let _ = service.wait();
    let _ = std::fs::remove_dir_all(&d);
}

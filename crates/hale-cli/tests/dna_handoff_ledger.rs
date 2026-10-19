//! GH #646, Riley's condition for stage 5: the handoff keeps working over
//! the ledger. The firm's `handoff.*` and `task.*` rows are its Ledger's,
//! as every record's are from its birth (GH #1590). A task crosses from the firm into the
//! accountant's inbox on its nerves (GH #1590: a signed envelope, kept in
//! the accountant's memory when it reads its inbox), the accountant
//! accepts it in its record, and the firm's `handoff sync` reads the
//! acceptance back and settles the task — in the ledger.

#[path = "support/vault.rs"]
mod vault;
#[path = "support/habitat.rs"]
mod habitat;
#[path = "support/scratch.rs"]
mod scratch;
#[path = "support/record.rs"]
mod record;
#[path = "support/reap.rs"]
mod reap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn hale(args: &[&str], cwd: &Path, env: &[(&str, &str)]) -> (bool, String) {
    let mut c = vault::hale();
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
    record::text(app).lines().filter(|l| !l.trim().is_empty()).filter_map(|l| serde_json::from_str(l).ok()).collect()
}

/// One row of the firm's own work in its Ledger, written as the spine
/// writes the organism's rows: through memory's append function, under the
/// spine's role (its password the vault's), in `author`'s name.
fn plumb(app: &Path, kind: &str, entity: &str, body: &str, author: &str) {
    let spine = record::spine(vault::hale(), app);
    assert!(!spine.is_empty(), "the spine's DSN for {}", app.display());
    let (dsn, query) = spine.split_once('?').unwrap_or((spine.as_str(), ""));
    let vault_name = query.split('&').find_map(|kv| kv.strip_prefix("vault=")).expect("the spine's vault entry");
    let password = std::fs::read_to_string(vault::dir().join(vault_name)).expect("the spine's password in the vault");
    let (scheme, rest) = dsn.split_once("://").unwrap();
    let (user, hostdb) = rest.split_once('@').unwrap();
    let url = format!("{scheme}://{user}:{}@{hostdb}?sslmode=disable", password.trim());
    let q = |v: &str| v.replace('\'', "''");
    let sql = format!("SELECT {}.ledger_append(-1, '{}', '{}', '{}', '{}', '', 0, '')", record::schema(app), q(kind), q(entity), q(body), q(author));
    let out = Command::new("psql").args([url.as_str(), "-q", "-t", "-A", "-v", "ON_ERROR_STOP=1", "-c", &sql]).output().expect("psql");
    assert!(out.status.success() && !String::from_utf8_lossy(&out.stdout).contains("\"error\": \"refused"), "the spine appends {kind} {entity}: {}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
}

fn project(scratch: &Path, name: &str, user: &str) -> (PathBuf, PathBuf) {
    habitat::habitat();
    let (ok, out) = hale(&["habitat", "graft", name, "--no-pack"], scratch, &[]);
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
    (app, bare)
}

/// Memory's owner for these organisms (CI's service container).
fn owner_dsn() -> Option<String> {
    std::env::var("HALE_DNA_MEMORY_DSN_OWNER").ok().filter(|d| !d.is_empty())
}

/// The nerves' server (CI's), as a part is handed it: no credentials.
fn nerves_url() -> Option<String> {
    let owner = std::env::var("HALE_DNA_NATS_URL_OWNER").ok().filter(|d| !d.is_empty())?;
    let (scheme, rest) = owner.split_once("://")?;
    Some(format!("{scheme}://{}", rest.rsplit_once('@').map(|(_, h)| h).unwrap_or(rest)))
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
    vault::hale()
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
    let (Some(owner), Some(nerves)) = (owner_dsn(), nerves_url()) else {
        eprintln!("dna_handoff_ledger: no HALE_DNA_MEMORY_DSN_OWNER or HALE_DNA_NATS_URL_OWNER; the ledger is memory's and the exchange the nerves', so nothing was exercised");
        return;
    };
    let d = scratch::Scratch::new(format!("hale_dna_handoff_ledger_{}", std::process::id()));
    let _reap = reap::ReapOnDrop(d.to_path_buf());
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let (firm, _firm_bare) = project(&d, "firm", "riley");
    let (acct, _acct_bare) = project(&d, "acct", "noor");
    // the firm's job, handed to a person, in its Ledger
    plumb(&firm, "task.born", "t7", "i-t7: prepare the year-end ledger", "dna");
    plumb(&firm, "task.handed", "t7", "{\"work\": \"t7/w\", \"assignee\": \"mara\", \"by\": \"leader\", \"narrative\": \"handed: not a software change\"}", "dna");

    let head = head_dsn(&firm);
    let env: &[(&str, &str)] = &[("HALE_DNA_MEMORY_DSN_HEAD", head.as_str()), ("HALE_DNA_NATS_URL_SPINE", nerves.as_str())];
    // each record's exchange and card on the nerves (GH #1590)
    for app in [&firm, &acct] {
        let (ok, out) = hale(&["dna", "nerves", "migrate"], app, &[]);
        assert!(ok, "nerves migrate: {out}");
    }
    let firm_genesis = record::identity(&firm);
    let acct_genesis = record::identity(&acct);
    let mut service = spine(&firm, &d);
    let st = status_until(&firm, env, "t7 [handed]");
    assert!(st.contains("t7 [handed]"), "the handed task is read from the ledger:\n{st}");

    // a connection to the accountant, in force
    let (ok, proposed) = hale(&["dna", "connect", &nerves, "--record", &acct_genesis, "--name", "acct", "--as", "accountant", "--purpose", "year-end books", "--classes", "internal", "--by", "riley"], &firm, env);
    assert!(ok && proposed.contains("connection acct proposed"), "{proposed}");
    let rid = word_after(&proposed, "hale dna review ");
    let (ok, out) = hale(&["dna", "review", &rid, "approve", "--as", "ana", "--authority", "board", "--no-wait"], &firm, env);
    assert!(ok, "{out}");
    let (ok, listed) = hale(&["dna", "connect"], &firm, env);
    assert!(ok && listed.contains("in force (approved by ana)"), "{listed}");

    // the task crosses: the envelope into the accountant's inbox (the nerves), the row into the ledger
    let (ok, handed) = hale(&["dna", "handoff", "acct", "task", "t7", "--as", "riley", "--note", "close by March"], &firm, env);
    assert!(ok && handed.contains("task t7 [internal] delivered to record"), "{handed}");
    let hid = word_after(&handed, "handoff ").trim_end_matches(':').to_string();
    let published = format!("\"kind\": \"handoff.published\", \"entity\": \"handoff:{hid}\"");
    let rows = ledger_until(&firm, env, |r| r.contains(&published));
    assert!(rows.contains(&published), "the firm's row is the ledger's, admitted by the spine:\n{rows}");
    assert!(!record(&firm).iter().any(|r| r["kind"] == "handoff.published"), "and not the record's (only its request is)");
    let st = status_until(&firm, env, "t7 [transfer_requested]");
    assert!(st.contains("t7 [transfer_requested]"), "the task waits for acceptance:\n{st}");

    // the accountant, its rows its record's alone, reads its inbox, connects back and accepts
    let acct_head = head_dsn(&acct);
    let acct_env: &[(&str, &str)] = &[("HALE_DNA_MEMORY_DSN_HEAD", acct_head.as_str()), ("HALE_DNA_NATS_URL_SPINE", nerves.as_str())];
    let (ok, inbox) = hale(&["dna", "handoff"], &acct, acct_env);
    assert!(ok && inbox.contains(&format!("{hid} ← task t7")), "the envelope is in the accountant's inbox:\n{inbox}");
    let (ok, back) = hale(&["dna", "connect", &nerves, "--record", &firm_genesis, "--name", "firm", "--as", "client", "--purpose", "year-end books", "--classes", "internal", "--by", "noor"], &acct, acct_env);
    assert!(ok, "{back}");
    let brid = word_after(&back, "hale dna review ");
    let (ok, out) = hale(&["dna", "review", &brid, "approve", "--as", "lee", "--authority", "board", "--no-wait"], &acct, acct_env);
    assert!(ok, "{out}");
    let (ok, accepted) = hale(&["dna", "handoff", "accept", &hid, "--as", "noor", "--note", "taken on"], &acct, acct_env);
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
    record::unmigrate(&firm, &owner);
    record::unmigrate(&acct, &owner);
    let _ = std::fs::remove_dir_all(&d);
}

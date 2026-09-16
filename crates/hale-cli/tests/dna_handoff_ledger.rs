//! GH #646, Riley's condition for stage 5: the git-based handoff keeps
//! working over the ledger. The firm has adopted the ledger (its
//! `handoff.*` and `task.*` rows live in the store behind the service);
//! the accountant is on the record alone. A task crosses from the firm
//! into the accountant's mailbox (a git ref, as before), the accountant
//! accepts it in its record, and the firm's `handoff sync` reads the
//! acceptance back and settles the task — in the ledger.

#[path = "support/reap.rs"]
mod reap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
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

fn http(port: u16, req: &str) -> String {
    let Ok(mut s) = TcpStream::connect(("127.0.0.1", port)) else { return String::new() };
    let _ = s.set_read_timeout(Some(Duration::from_secs(15)));
    let _ = s.write_all(req.as_bytes());
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    out
}

fn body(resp: &str) -> String {
    resp.split("\r\n\r\n").nth(1).unwrap_or("").to_string()
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

fn word_after(text: &str, marker: &str) -> String {
    text.split_once(marker).map(|(_, r)| r.split_whitespace().next().unwrap_or("").to_string()).unwrap_or_default()
}

#[test]
fn a_handoff_crosses_from_an_adopted_firm_and_its_acceptance_settles_the_task_in_the_ledger() {
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

    // the firm adopts the ledger
    let kport = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let mut service = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["dna", "knowledge", ".", "--port", &kport.to_string()])
        .current_dir(&firm)
        .env("HALE_BIN", env!("CARGO_BIN_EXE_hale"))
        .env("HALE_DNA_DISCOVER", "off")
        .env("XDG_CACHE_HOME", std::env::temp_dir().join("hale-tests-iris-cache"))
        .env("HALE_DNA_KNOWLEDGE_DSN", "memory")
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(d.join("service.stderr")).unwrap())
        .spawn()
        .expect("hale dna knowledge");
    let dl = Instant::now() + Duration::from_secs(120);
    while Instant::now() < dl && !body(&http(kport, "GET /ledger/head HTTP/1.0\r\nHost: x\r\n\r\n")).contains("\"revision\"") {
        std::thread::sleep(Duration::from_millis(200));
    }
    let url = format!("http://127.0.0.1:{kport}");
    let env: &[(&str, &str)] = &[("HALE_DNA_KNOWLEDGE_URL", &url)];
    let (ok, adopted) = hale(&["dna", "ledger", "adopt", "--as", "riley"], &firm, env);
    assert!(ok, "{adopted}");
    let (ok, st) = hale(&["dna", "status"], &firm, env);
    assert!(ok && st.contains("t7 [handed]"), "the handed task is read from the ledger:\n{st}");

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
    let rows = body(&http(kport, "GET /ledger/rows?from=0 HTTP/1.0\r\nHost: x\r\n\r\n"));
    assert!(rows.contains(&format!("\"kind\": \"handoff.published\", \"entity\": \"handoff:{hid}\"")), "the firm's row is the ledger's:\n{rows}");
    assert!(!record(&firm).iter().any(|r| r["kind"] == "handoff.published"), "and not the record's");
    let (ok, st) = hale(&["dna", "status"], &firm, env);
    assert!(ok && st.contains("t7 [transfer_requested]"), "the task waits for acceptance:\n{st}");

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
    let (ok, st) = hale(&["dna", "status"], &firm, env);
    assert!(ok && st.contains("t7 [transfer_accepted]"), "settled:\n{st}");
    let rows = body(&http(kport, "GET /ledger/rows?from=0 HTTP/1.0\r\nHost: x\r\n\r\n"));
    assert!(rows.contains("\"kind\": \"task.transfer_accepted\", \"entity\": \"t7\"") && rows.contains("\"kind\": \"handoff.accepted_by_peer\""), "the settlement rows are the ledger's:\n{rows}");
    let (ok, again) = hale(&["dna", "handoff", "sync"], &firm, env);
    assert!(ok && again.contains("0 acceptance(s) admitted"), "admitted once: {again}");
    let _ = service.kill();
    let _ = service.wait();
    let _ = std::fs::remove_dir_all(&d);
}

fn service(app: &Path, d: &Path, name: &str, port: u16) -> std::process::Child {
    Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["dna", "knowledge", ".", "--port", &port.to_string()])
        .current_dir(app)
        .env("HALE_BIN", env!("CARGO_BIN_EXE_hale"))
        .env("HALE_DNA_DISCOVER", "off")
        .env("XDG_CACHE_HOME", std::env::temp_dir().join("hale-tests-iris-cache"))
        .env("HALE_DNA_KNOWLEDGE_DSN", "memory")
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(d.join(format!("{name}.stderr"))).unwrap())
        .spawn()
        .expect("hale dna knowledge")
}

fn wait_service(port: u16) -> bool {
    let dl = Instant::now() + Duration::from_secs(120);
    while Instant::now() < dl {
        if body(&http(port, "GET /ledger/head HTTP/1.0\r\nHost: x\r\n\r\n")).contains("\"revision\"") {
            return true;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    false
}

/// Stage 5 (#662): two organisms, two services, no git remote between
/// them. A task crosses service to service, a delivery interrupted by
/// the receiver's service going away is completed by the sender's next
/// sync without a second envelope, the receiver accepts, and the
/// origin's task settles only then.
#[test]
fn a_handoff_crosses_service_to_service_and_a_lost_delivery_is_made_again_once() {
    let d = std::env::temp_dir().join(format!("hale_dna_exchange_{}", std::process::id()));
    let _reap = reap::ReapOnDrop(d.clone());
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let (firm, _) = project(&d, "firm", "riley");
    let (acct, _) = project(&d, "acct", "noor");
    plumb(&firm, "task.born", "t7", "i-t7: prepare the year-end ledger", "dna");
    plumb(&firm, "task.handed", "t7", "{\"work\": \"t7/w\", \"assignee\": \"mara\", \"by\": \"leader\", \"narrative\": \"handed: not a software change\"}", "dna");
    let fport = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let aport = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let mut fsvc = service(&firm, &d, "firm-service", fport);
    let mut asvc = service(&acct, &d, "acct-service", aport);
    assert!(wait_service(fport) && wait_service(aport), "both services up");
    let furl = format!("http://127.0.0.1:{fport}");
    let aurl = format!("http://127.0.0.1:{aport}");
    let fenv: &[(&str, &str)] = &[("HALE_DNA_KNOWLEDGE_URL", &furl)];
    let aenv: &[(&str, &str)] = &[("HALE_DNA_KNOWLEDGE_URL", &aurl)];
    for (app, env) in [(&firm, fenv), (&acct, aenv)] {
        let (ok, out) = hale(&["dna", "ledger", "adopt", "--as", "operator"], app, env);
        assert!(ok, "{out}");
    }
    // the connection names the peer's service, not a git remote
    let (ok, proposed) = hale(&["dna", "connect", &aurl, "--name", "acct", "--as", "accountant", "--purpose", "year-end books", "--classes", "internal", "--by", "riley"], &firm, fenv);
    assert!(ok && proposed.contains("(service exchange)"), "{proposed}");
    let rid = word_after(&proposed, "hale dna review ");
    let (ok, out) = hale(&["dna", "review", &rid, "approve", "--as", "ana", "--authority", "board", "--no-wait"], &firm, fenv);
    assert!(ok, "{out}");
    let (ok, back) = hale(&["dna", "connect", &furl, "--name", "firm", "--as", "client", "--purpose", "year-end books", "--classes", "internal", "--by", "noor"], &acct, aenv);
    assert!(ok, "{back}");
    let brid = word_after(&back, "hale dna review ");
    let (ok, out) = hale(&["dna", "review", &brid, "approve", "--as", "lee", "--authority", "board", "--no-wait"], &acct, aenv);
    assert!(ok, "{out}");

    // the receiver's service is away: the handoff cannot be delivered, and says so
    let _ = asvc.kill();
    let _ = asvc.wait();
    let (ok, away) = hale(&["dna", "handoff", "acct", "task", "t7", "--as", "riley", "--note", "close by March"], &firm, fenv);
    assert!(!ok && away.contains("cannot be reached") && away.contains("nothing was recorded here"), "{away}");
    let rows = body(&http(fport, "GET /ledger/rows?from=0 HTTP/1.0\r\nHost: x\r\n\r\n"));
    assert!(!rows.contains("handoff.published"), "nothing published while undeliverable:\n{rows}");
    // back: the handoff crosses, the envelope into the accountant's service mailbox
    let mut asvc = service(&acct, &d, "acct-service-2", aport);
    assert!(wait_service(aport), "the accountant's service is back");
    let (ok, handed) = hale(&["dna", "handoff", "acct", "task", "t7", "--as", "riley", "--note", "close by March"], &firm, fenv);
    assert!(ok && handed.contains("task t7 [internal] written into record"), "{handed}");
    let hid = word_after(&handed, "handoff ").trim_end_matches(':').to_string();
    let firm_genesis = git(&["rev-list", "--max-parents=0", "refs/dna/journal"], &firm);
    let mail = body(&http(aport, "GET /exchange HTTP/1.0\r\nHost: x\r\n\r\n"));
    assert!(mail.contains(&format!("\"sender\": \"{firm_genesis}\"")) && mail.contains("handoff.received") && mail.contains(&hid), "the envelope is in the accountant's service:\n{mail}");
    let (ok, st) = hale(&["dna", "status"], &firm, fenv);
    assert!(ok && st.contains("t7 [transfer_requested]"), "{st}");

    // the accountant's mailbox is lost (its service restarts empty, as an
    // in-memory one does): the firm's next sync delivers again, once
    let _ = asvc.kill();
    let _ = asvc.wait();
    let mut asvc = service(&acct, &d, "acct-service-3", aport);
    assert!(wait_service(aport), "the accountant's service is back, empty");
    let gone = body(&http(aport, "GET /exchange HTTP/1.0\r\nHost: x\r\n\r\n"));
    assert!(!gone.contains(&hid), "the envelope is gone with the store");
    let (ok, synced) = hale(&["dna", "handoff", "sync"], &firm, fenv);
    assert!(ok && synced.contains("delivered again (the other record did not hold it)"), "{synced}");
    let (ok, synced2) = hale(&["dna", "handoff", "sync"], &firm, fenv);
    assert!(ok && !synced2.contains("delivered again"), "held now, not delivered twice: {synced2}");
    let mail = body(&http(aport, "GET /exchange HTTP/1.0\r\nHost: x\r\n\r\n"));
    assert_eq!(mail.matches(&format!("\"entity\": \"handoff:{hid}\"")).count(), 1, "one envelope:\n{mail}");
    let rows = body(&http(fport, "GET /ledger/rows?from=0 HTTP/1.0\r\nHost: x\r\n\r\n"));
    assert_eq!(rows.matches("\"kind\": \"handoff.published\"").count(), 1, "one published row here");

    // the accountant sees it in its service mailbox, accepts; the acceptance
    // goes back service to service; the firm settles the task only then
    let (ok, listed) = hale(&["dna", "handoff"], &acct, aenv);
    assert!(ok && listed.contains(&format!("{hid} ← task t7")) && listed.contains("admitted under `firm`"), "{listed}");
    let (ok, st) = hale(&["dna", "status"], &firm, fenv);
    assert!(ok && st.contains("t7 [transfer_requested]"), "not settled by delivery: {st}");
    let (ok, accepted) = hale(&["dna", "handoff", "accept", &hid, "--as", "noor", "--note", "taken on"], &acct, aenv);
    assert!(ok && accepted.contains("accepted by noor under connection `firm`"), "{accepted}");
    let (ok, read_back) = hale(&["dna", "handoff", "sync"], &firm, fenv);
    assert!(ok && read_back.contains("1 acceptance(s) admitted"), "{read_back}");
    let (ok, st) = hale(&["dna", "status"], &firm, fenv);
    assert!(ok && st.contains("t7 [transfer_accepted]"), "settled on acceptance:\n{st}");
    let _ = fsvc.kill();
    let _ = fsvc.wait();
    let _ = asvc.kill();
    let _ = asvc.wait();
    let _ = std::fs::remove_dir_all(&d);
}

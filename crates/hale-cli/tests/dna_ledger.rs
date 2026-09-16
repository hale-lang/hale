//! GH #646 stage 1 (#650) — the Ledger. An organism starts on routing 0
//! (every row in the record) and moves the day's work to the store by an
//! explicit, one-way adoption: `ledger.adopting` in the record, the copy
//! and the cutover by the service, `ledger.adopted` naming the
//! checkpoint. From then on an operational kind is written through the
//! service, never into the record, and a head that knows no service is
//! refused with the checkpoint named — never silently accepted into git.
//! Readers of history see one history across both memories.

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

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

#[test]
fn an_organism_adopts_the_ledger_and_its_operations_leave_the_record() {
    let d = std::env::temp_dir().join(format!("hale_dna_ledger_{}", std::process::id()));
    let _reap = reap::ReapOnDrop(d.clone());
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let (ok, out) = hale(&["dna", "new", "ledgered"], &d, &[]);
    assert!(ok, "{out}");
    let app: PathBuf = d.join("ledgered");
    Command::new("git").args(["-c", "user.name=t", "-c", "user.email=t@l", "add", "-A"]).current_dir(&app).output().unwrap();
    Command::new("git").args(["-c", "user.name=t", "-c", "user.email=t@l", "commit", "-q", "-m", "genome"]).current_dir(&app).output().unwrap();
    // a remote, so a clone's verbs (a practice proposed, a receipt filed)
    // have a record to reach with no organism beside them
    let bare = d.join("origin.git");
    Command::new("git").args(["init", "-q", "--bare", "-b", "main", &bare.to_string_lossy()]).current_dir(&d).output().unwrap();
    Command::new("git").args(["remote", "add", "origin", &bare.to_string_lossy()]).current_dir(&app).output().unwrap();
    Command::new("git").args(["push", "-q", "origin", "main", "refs/dna/*:refs/dna/*"]).current_dir(&app).output().unwrap();
    // the organism's template reads both memories
    let main = std::fs::read_to_string(app.join("dna/org/main.hl")).unwrap();
    assert!(main.contains("dna::RoutedJournal") && main.contains("dna::ServiceLedger"), "the organism is wired for two memories:\n{main}");

    // routing 0: the record alone, and adoption is closed until stage 3
    let (ok, st) = hale(&["dna", "ledger"], &app, &[]);
    assert!(ok && st.contains("routing:    0") && st.contains("none known here"), "{st}");
    let (ok, status) = hale(&["dna", "status"], &app, &[]);
    assert!(ok && status.contains("memory:     the record alone (routing 0)"), "{status}");
    let (ok, closed) = hale(&["dna", "ledger", "adopt"], &app, &[]);
    assert!(!ok && closed.contains("no ledger service is known here"), "adoption needs a service to adopt into: {closed}");
    // an operational row before adoption lands in the record (a receipt
    // filed from a clone needs no organism beside it)
    std::fs::write(app.join("first.txt"), "the first invoice").unwrap();
    let (ok, out) = hale(&["dna", "receipt", "file", "first.txt", "--as", "sam"], &app, &[]);
    assert!(ok, "{out}");
    let before = record(&app);
    assert!(before.iter().any(|r| r["kind"] == "receipt.filed"), "the receipt row is in the record");
    let rows_before = before.len();

    // the service, in memory here, with no organism beside it
    let kport = free_port();
    let mut service = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["dna", "knowledge", ".", "--port", &kport.to_string()])
        .current_dir(&app)
        .env("HALE_BIN", env!("CARGO_BIN_EXE_hale"))
        .env("HALE_DNA_DISCOVER", "off")
        .env("XDG_CACHE_HOME", std::env::temp_dir().join("hale-tests-iris-cache"))
        .env("HALE_DNA_KNOWLEDGE_DSN", "memory")
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(d.join("service.stderr")).unwrap())
        .spawn()
        .expect("hale dna knowledge");
    let dl = Instant::now() + Duration::from_secs(120);
    let mut up = false;
    while Instant::now() < dl && !up {
        up = body(&http(kport, "GET /ledger/head HTTP/1.0\r\nHost: x\r\n\r\n")).contains("\"revision\": 0");
        std::thread::sleep(Duration::from_millis(200));
    }
    let stop = |service: &mut std::process::Child| {
        let _ = service.kill();
        let _ = service.wait();
    };
    if !up {
        stop(&mut service);
        panic!("the service did not come up:\n{}", std::fs::read_to_string(d.join("service.stderr")).unwrap_or_default());
    }
    let url = format!("http://127.0.0.1:{kport}");
    let service_env: &[(&str, &str)] = &[("HALE_DNA_KNOWLEDGE_URL", &url)];

    // a row of the record is not the ledger's to take
    let refused = body(&http(kport, "POST /ledger/append HTTP/1.0\r\nHost: x\r\nContent-Type: application/json\r\nContent-Length: 63\r\n\r\n{\"expected\": 0, \"kind\": \"review.verdict\", \"entity\": \"x\", \"body\": \"\"}"));
    assert!(refused.contains("is a row of the record, not the ledger"), "{refused}");

    // adoption, opened for the fixture
    let (ok, adopted) = hale(&["dna", "ledger", "adopt", "--as", "riley"], &app, &[("HALE_DNA_KNOWLEDGE_URL", &url), ("HALE_DNA_ADOPT_UNGATED", "1")]);
    assert!(ok && adopted.contains("ledger adopted at") && adopted.contains("operational row(s) copied"), "{adopted}");
    let after = record(&app);
    let adopting = after.iter().position(|r| r["kind"] == "ledger.adopting").expect("ledger.adopting");
    let adopted_at = after.iter().position(|r| r["kind"] == "ledger.adopted").expect("ledger.adopted");
    assert!(adopting < adopted_at, "the intent is in the record before the checkpoint");
    let cp = after[adopted_at]["body"].as_str().unwrap();
    assert!(cp.contains("\"routing\": 1") && cp.contains("\"checkpoint\": \"") && cp.contains("\"rows\": "), "{cp}");
    assert_eq!(after.len(), rows_before + 2, "the record gained the two adoption rows and lost nothing");
    let head = body(&http(kport, "GET /ledger/head HTTP/1.0\r\nHost: x\r\n\r\n"));
    assert!(head.contains("\"adopted\": true") && head.contains("\"routing\": 1"), "{head}");
    let copied = body(&http(kport, "GET /ledger/rows?from=0 HTTP/1.0\r\nHost: x\r\n\r\n"));
    assert!(copied.contains("\"kind\": \"receipt.filed\"") && copied.contains("\"author\": \"sam\""), "the operational rows were copied with their authors:\n{copied}");
    assert!(!copied.contains("\"kind\": \"review.requested\""), "the record's own rows were not:\n{copied}");
    let (ok, st) = hale(&["dna", "ledger"], &app, service_env);
    assert!(ok && st.contains("routing:    1") && st.contains("cutover at"), "{st}");
    let (ok, again) = hale(&["dna", "ledger", "adopt"], &app, &[("HALE_DNA_KNOWLEDGE_URL", &url), ("HALE_DNA_ADOPT_UNGATED", "1")]);
    assert!(!ok && again.contains("already on routing 1"), "{again}");

    // after cutover: an operational row goes through the service …
    std::fs::write(app.join("second.txt"), "the second invoice").unwrap();
    let (ok, out) = hale(&["dna", "receipt", "file", "second.txt", "--as", "sam"], &app, service_env);
    assert!(ok, "{out}");
    let now = record(&app);
    assert_eq!(now.len(), after.len(), "the record did not take the operational row");
    let rows = body(&http(kport, "GET /ledger/rows?from=0 HTTP/1.0\r\nHost: x\r\n\r\n"));
    assert!(rows.contains("second.txt"), "the ledger did:\n{rows}");
    // … and a head with no service is refused, the checkpoint named, nothing written
    std::fs::write(app.join("third.txt"), "the third invoice").unwrap();
    let (ok, refused) = hale(&["dna", "receipt", "file", "third.txt", "--as", "sam"], &app, &[]);
    assert!(!ok && refused.contains("is a row of the ledger") && refused.contains("no service is known here") && refused.contains("nothing was written"), "{refused}");
    assert_eq!(record(&app).len(), after.len(), "the record still did not take it");
    // an evolutionary row still goes to the record
    let (ok, out) = hale(&["dna", "practice", "propose", "billing/late", "--text", "Chase an invoice at seven days.", "--as", "riley"], &app, service_env);
    assert!(ok, "{out}");
    assert!(record(&app).iter().any(|r| r["kind"] == "practice.requested" || r["kind"] == "practice.proposed"), "the practice is in the record");
    // history reads one history across both memories
    let (ok, hist) = hale(&["dna", "history"], &app, service_env);
    assert!(ok && hist.contains("first.txt") && hist.contains("second.txt") && hist.contains("ledger.adopted"), "one history:\n{hist}");
    // a body without its store admits nothing
    let (ok, norun) = hale(&["dna", "run", ".", "--no-iris"], &app, &[("HALE_DNA_KNOWLEDGE_URL", "")]);
    assert!(!ok && norun.contains("no ledger service is known here"), "{norun}");

    // stage 2: the body lease is a row of the store. No body has run: the
    // store knows no lease; a body takes it there, the fence renews it
    // there, and a head reads it there
    let (ok, nobody) = hale(&["dna", "body"], &app, service_env);
    assert!(ok && nobody.contains("none (no body has run this record"), "{nobody}");
    let mut host = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["dna", "run", ".", "--no-iris"])
        .current_dir(&app)
        .env("HALE_BIN", env!("CARGO_BIN_EXE_hale"))
        .env("HALE_DNA_DISCOVER", "off")
        .env("XDG_CACHE_HOME", std::env::temp_dir().join("hale-tests-iris-cache"))
        .env("HALE_DNA_KNOWLEDGE_URL", &url)
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(d.join("run.stderr")).unwrap())
        .spawn()
        .expect("hale dna run");
    let dl = Instant::now() + Duration::from_secs(120);
    let mut lease = String::new();
    while Instant::now() < dl {
        lease = body(&http(kport, "GET /ledger/lease?key=body HTTP/1.0\r\nHost: x\r\n\r\n"));
        if lease.contains("\"present\": true") {
            break;
        }
        if let Ok(Some(st)) = host.try_wait() {
            panic!("hale dna run exited early: {st}\n{}", std::fs::read_to_string(d.join("run.stderr")).unwrap_or_default());
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    assert!(lease.contains("\"present\": true") && lease.contains("\"token\": 1"), "the body took its lease in the store:\n{lease}\n{}", std::fs::read_to_string(d.join("run.stderr")).unwrap_or_default());
    let (ok, live) = hale(&["dna", "body"], &app, service_env);
    assert!(ok && live.contains("live on "), "a head reads the lease from the store: {live}");
    let git_lease = Command::new("git").args(["rev-parse", "-q", "--verify", "refs/dna/lease/body"]).current_dir(&app).output().unwrap();
    assert!(!git_lease.status.success(), "no git lease ref was written on routing 1");
    // the second body is refused by the store's lease
    let (ok, second) = hale(&["dna", "run", ".", "--no-iris"], &app, service_env);
    assert!(!ok && second.contains("lease"), "a second body is refused: {second}");
    let stop_host = |host: &mut std::process::Child| {
        for f in ["org.pid", "app.pid"] {
            if let Ok(pid) = std::fs::read_to_string(app.join(".hale/dna").join(f)) {
                let _ = Command::new("kill").args(["-9", pid.trim()]).status();
            }
        }
        let _ = host.kill();
        let _ = host.wait();
    };
    stop_host(&mut host);
    std::thread::sleep(Duration::from_millis(500));
    // the body is gone; its lease stands until forced, in the store
    let (ok, forced) = hale(&["dna", "body", "claim", "--force", "--as", "riley"], &app, service_env);
    assert!(ok && (forced.contains("body.claimed") || forced.contains("released")), "{forced}");
    let after_force = body(&http(kport, "GET /ledger/lease?key=body HTTP/1.0\r\nHost: x\r\n\r\n"));
    assert!(after_force.contains("\"present\": false"), "the forced claim released the lease in the store:\n{after_force}");

    // abandon: back to the record alone; the record's rows were never removed
    let (ok, ab) = hale(&["dna", "ledger", "abandon", "--why", "the fixture is done", "--as", "riley"], &app, service_env);
    assert!(ok && ab.contains("ledger abandoned by riley"), "{ab}");
    let (ok, st) = hale(&["dna", "ledger"], &app, service_env);
    assert!(ok && st.contains("routing:    0"), "{st}");
    let last = record(&app);
    assert!(last.iter().any(|r| r["kind"] == "ledger.abandoned") && last.iter().any(|r| r["kind"] == "receipt.filed"), "abandoned, and the pre-adoption rows are still there");
    stop(&mut service);
    let _ = std::fs::remove_dir_all(&d);
}

/// One row on the record by plumbing, the shape `GitRecord` writes: a
/// pre-split organism's history is built this way so the continuity of
/// every projection across adoption is exercised on the real git
/// journal and the real migration, not described.
fn plumb(app: &Path, kind: &str, entity: &str, body: &str, author: &str) {
    let git = |args: &[&str]| -> String {
        let out = Command::new("git").args(args).current_dir(app).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    let head = git(&["rev-parse", "refs/dna/journal"]);
    let text = git(&["show", "refs/dna/journal:journal.jsonl"]);
    let seq = text.lines().filter(|l| !l.trim().is_empty()).count();
    let line = serde_json::json!({"seq": seq, "kind": kind, "entity": entity, "body": body, "author": author}).to_string();
    let tmp = app.join(".hale/dna/plumb.jsonl");
    std::fs::create_dir_all(tmp.parent().unwrap()).unwrap();
    std::fs::write(&tmp, format!("{text}\n{line}\n")).unwrap();
    let blob = git(&["hash-object", "-w", &tmp.to_string_lossy()]);
    let idx = app.join(".hale/dna/plumb.index");
    let with_index = |args: &[&str]| {
        let out = Command::new("git").args(args).current_dir(app).env("GIT_INDEX_FILE", &idx).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    with_index(&["read-tree", &head]);
    with_index(&["update-index", "--add", "--cacheinfo", &format!("100644,{blob},journal.jsonl")]);
    let tree = with_index(&["write-tree"]);
    let commit = git(&["-c", "user.name=t", "-c", "user.email=t@l", "commit-tree", &tree, "-p", &head, "-m", &format!("{kind} {entity}")]);
    git(&["update-ref", "refs/dna/journal", &commit, &head]);
}

/// The continuity seed (#646, Riley's gate): a pre-split organism carrying
/// unfinished work, a reservation, a practice bound to a handed task, a
/// retired participant, redacted evidence and a pending handoff is
/// adopted, and every projection reads the same before and after.
#[test]
fn a_pre_split_organism_carries_its_history_and_unfinished_work_through_adoption() {
    let d = std::env::temp_dir().join(format!("hale_dna_continuity_{}", std::process::id()));
    let _reap = reap::ReapOnDrop(d.clone());
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let (ok, out) = hale(&["dna", "new", "carried"], &d, &[]);
    assert!(ok, "{out}");
    let app: PathBuf = d.join("carried");
    Command::new("git").args(["-c", "user.name=t", "-c", "user.email=t@l", "add", "-A"]).current_dir(&app).output().unwrap();
    Command::new("git").args(["-c", "user.name=t", "-c", "user.email=t@l", "commit", "-q", "-m", "genome"]).current_dir(&app).output().unwrap();
    // the pre-split history, as the merged stack wrote it
    plumb(&app, "intent.requested", "i-1", "{\"outcome\": \"chase the late invoice\", \"from\": \"riley\", \"to\": \"\"}", "riley");
    plumb(&app, "intent.offered", "i-1", "{\"outcome\": \"chase the late invoice\"}", "dna");
    plumb(&app, "task.born", "t-1", "{\"intent\": \"i-1\", \"objective\": \"chase the late invoice\"}", "dna");
    plumb(&app, "task.planned", "t-1", "{\"plan\": \"a person's job\"}", "dna");
    plumb(&app, "practice.proposed", "p-1", "{\"name\": \"acceptance/collections\", \"digest\": \"sha256:aaaa\", \"review_id\": \"k:aaaa\", \"by\": \"riley\"}", "riley");
    plumb(&app, "task.handed", "t-1", "{\"work\": \"chase the late invoice\", \"assignee\": \"sam\", \"by\": \"dna\", \"acceptance\": \"sha256:aaaa\", \"evidence_required\": true}", "dna");
    plumb(&app, "grant.reserved", "op-1", "{\"op\": \"op-1\", \"amount\": \"120.00\", \"by\": \"sam\"}", "dna");
    plumb(&app, "receipt.filed", "sha256:bbbb", "{\"by\": \"sam\", \"name\": \"invoice.pdf\", \"class\": \"internal\"}", "sam");
    plumb(&app, "receipt.redacted", "sha256:bbbb", "{\"by\": \"riley\", \"why\": \"wrong customer\", \"policy\": \"gdpr\", \"class\": \"internal\", \"store\": \"git\"}", "riley");
    plumb(&app, "person.retired", "alex", "{\"by\": \"riley\", \"to\": \"sam\", \"transferred\": 0}", "riley");
    plumb(&app, "handoff.published", "handoff:h-1", "{\"kind\": \"task\", \"subject\": \"t-1\", \"connection\": \"partner\", \"by\": \"riley\"}", "riley");
    plumb(&app, "schedule.declared", "weekly-close", "{\"cron\": \"0 9 * * 1\"}", "dna");
    let seeded = record(&app);
    let before_kinds: Vec<String> = seeded.iter().map(|r| format!("{} {}", r["kind"].as_str().unwrap_or(""), r["entity"].as_str().unwrap_or(""))).collect();

    // the projections before
    let (ok, history_before) = hale(&["dna", "history"], &app, &[]);
    assert!(ok, "{history_before}");
    let (ok, task_before) = hale(&["dna", "history", "t-1"], &app, &[]);
    assert!(ok && task_before.contains("task.handed") && task_before.contains("task.born"), "{task_before}");
    let (ok, status_before) = hale(&["dna", "status", "--json"], &app, &[]);
    assert!(ok, "{status_before}");

    let kport = free_port();
    let mut service = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["dna", "knowledge", ".", "--port", &kport.to_string()])
        .current_dir(&app)
        .env("HALE_BIN", env!("CARGO_BIN_EXE_hale"))
        .env("HALE_DNA_DISCOVER", "off")
        .env("XDG_CACHE_HOME", std::env::temp_dir().join("hale-tests-iris-cache"))
        .env("HALE_DNA_KNOWLEDGE_DSN", "memory")
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(d.join("service.stderr")).unwrap())
        .spawn()
        .expect("hale dna knowledge");
    let dl = Instant::now() + Duration::from_secs(120);
    let mut up = false;
    while Instant::now() < dl && !up {
        up = body(&http(kport, "GET /ledger/head HTTP/1.0\r\nHost: x\r\n\r\n")).contains("\"revision\"");
        std::thread::sleep(Duration::from_millis(200));
    }
    assert!(up, "the service did not come up:\n{}", std::fs::read_to_string(d.join("service.stderr")).unwrap_or_default());
    let url = format!("http://127.0.0.1:{kport}");
    let env: &[(&str, &str)] = &[("HALE_DNA_KNOWLEDGE_URL", &url), ("HALE_DNA_ADOPT_UNGATED", "1")];

    let (ok, adopted) = hale(&["dna", "ledger", "adopt", "--as", "riley"], &app, env);
    assert!(ok, "{adopted}");
    // the copy: exactly the operational rows, in order, with their authors
    let copied = body(&http(kport, "GET /ledger/rows?from=0 HTTP/1.0\r\nHost: x\r\n\r\n"));
    let copied_kinds: Vec<String> = copied.lines().filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok()).map(|v| format!("{} {}", v["kind"].as_str().unwrap_or(""), v["entity"].as_str().unwrap_or(""))).collect();
    let expected: Vec<String> = ["intent.requested i-1", "intent.offered i-1", "task.born t-1", "task.planned t-1", "task.handed t-1", "grant.reserved op-1", "receipt.filed sha256:bbbb", "receipt.redacted sha256:bbbb", "handoff.published handoff:h-1", "schedule.declared weekly-close"].iter().map(|s| s.to_string()).collect();
    assert_eq!(copied_kinds, expected, "the operational rows, in order:\n{copied}");
    assert!(copied.contains("\"author\": \"sam\""), "with their authors:\n{copied}");
    // the record kept every row it had, plus the two adoption rows
    let now = record(&app);
    let now_kinds: Vec<String> = now.iter().map(|r| format!("{} {}", r["kind"].as_str().unwrap_or(""), r["entity"].as_str().unwrap_or(""))).collect();
    assert_eq!(&now_kinds[..before_kinds.len()], &before_kinds[..], "the record's rows are untouched");
    assert_eq!(now_kinds.len(), before_kinds.len() + 2);

    // the projections after: the same, across two memories
    let (ok, history_after) = hale(&["dna", "history"], &app, &[("HALE_DNA_KNOWLEDGE_URL", &url)]);
    assert!(ok, "{history_after}");
    // a row's position differs once two memories are read as one (the
    // ledger's rows follow the record's); what is compared is every row
    // without its position
    let strip = |s: &str| -> Vec<String> {
        s.lines()
            .filter(|l| l.contains(" ") && !l.contains("ledger.adopt") && !l.contains("event(s)"))
            .map(|l| l.trim().trim_start_matches(|c: char| c.is_ascii_digit()).trim().to_string())
            .collect()
    };
    let mut hb = strip(&history_before);
    let mut ha = strip(&history_after);
    hb.sort();
    ha.sort();
    assert_eq!(hb, ha, "history reads the same rows before and after adoption\nbefore:\n{history_before}\nafter:\n{history_after}");
    let (ok, task_after) = hale(&["dna", "history", "t-1"], &app, &[("HALE_DNA_KNOWLEDGE_URL", &url)]);
    assert!(ok && task_after.contains("task.handed") && task_after.contains("task.born") && task_after.contains("sam"), "the handed task keeps its id, its assignee and its bound practice:\n{task_after}");
    let (ok, status_after) = hale(&["dna", "status", "--json"], &app, &[("HALE_DNA_KNOWLEDGE_URL", &url)]);
    assert!(ok, "{status_after}");
    let sb: serde_json::Value = serde_json::from_str(status_before.lines().find(|l| l.starts_with('{')).unwrap_or("{}")).unwrap_or_default();
    let sa: serde_json::Value = serde_json::from_str(status_after.lines().find(|l| l.starts_with('{')).unwrap_or("{}")).unwrap_or_default();
    // a row's position (`since`) is its merged position, which moves once
    // two memories are read as one; everything else about a task, a
    // review or a mutation reads the same
    let unplaced = |v: &serde_json::Value| -> serde_json::Value {
        let mut v = v.clone();
        if let Some(arr) = v.as_array_mut() {
            for item in arr.iter_mut() {
                if let Some(o) = item.as_object_mut() {
                    o.remove("since");
                    o.remove("seq");
                }
            }
        }
        v
    };
    for key in ["tasks", "reviews", "pending", "handed", "mutations"] {
        if sb.get(key).is_some() {
            assert_eq!(unplaced(&sb[key]), unplaced(&sa[key]), "status `{key}` reads the same before and after:\nbefore {status_before}\nafter {status_after}");
        }
    }
    let _ = service.kill();
    let _ = service.wait();
    let _ = std::fs::remove_dir_all(&d);
}

/// Stage 3 (#652): a head that cannot reach the service keeps its
/// requests, and the service admits each on submission against the
/// record as it is then — a retired person's request is refused, a
/// request submitted twice lands once, a completion in the wrong name
/// is refused — and nothing on the head is ever authoritative.
#[test]
fn a_head_queues_while_the_service_is_unreachable_and_the_service_revalidates_on_submission() {
    let d = std::env::temp_dir().join(format!("hale_dna_queue_{}", std::process::id()));
    let _reap = reap::ReapOnDrop(d.clone());
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let (ok, out) = hale(&["dna", "new", "queued"], &d, &[]);
    assert!(ok, "{out}");
    let app: PathBuf = d.join("queued");
    Command::new("git").args(["-c", "user.name=t", "-c", "user.email=t@l", "add", "-A"]).current_dir(&app).output().unwrap();
    Command::new("git").args(["-c", "user.name=t", "-c", "user.email=t@l", "commit", "-q", "-m", "genome"]).current_dir(&app).output().unwrap();
    // a task handed to sam, before adoption, so the completion rule has something to check
    plumb(&app, "task.born", "t-1", "{\"objective\": \"chase the invoice\"}", "dna");
    plumb(&app, "task.handed", "t-1", "{\"work\": \"chase the invoice\", \"assignee\": \"sam\", \"by\": \"dna\"}", "dna");
    let kport = free_port();
    let url = format!("http://127.0.0.1:{kport}");
    let start = |d: &Path, app: &Path| -> std::process::Child {
        Command::new(env!("CARGO_BIN_EXE_hale"))
            .args(["dna", "knowledge", ".", "--port", &kport.to_string()])
            .current_dir(app)
            .env("HALE_BIN", env!("CARGO_BIN_EXE_hale"))
            .env("HALE_DNA_DISCOVER", "off")
            .env("XDG_CACHE_HOME", std::env::temp_dir().join("hale-tests-iris-cache"))
            .env("HALE_DNA_KNOWLEDGE_DSN", "memory")
            .stdout(Stdio::null())
            .stderr(std::fs::File::create(d.join("service.stderr")).unwrap())
            .spawn()
            .expect("hale dna knowledge")
    };
    let wait_up = || {
        let dl = Instant::now() + Duration::from_secs(120);
        while Instant::now() < dl {
            if body(&http(kport, "GET /ledger/head HTTP/1.0\r\nHost: x\r\n\r\n")).contains("\"revision\"") {
                return true;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        false
    };
    let mut service = start(&d, &app);
    assert!(wait_up(), "the service did not come up");
    let env: &[(&str, &str)] = &[("HALE_DNA_KNOWLEDGE_URL", &url)];
    // adoption is open now: no gate to lift
    let (ok, adopted) = hale(&["dna", "ledger", "adopt", "--as", "riley"], &app, env);
    assert!(ok, "{adopted}");
    // a completion in the wrong name is refused by the service, not written
    std::fs::write(app.join("proof.txt"), "the invoice was paid").unwrap();
    let (ok, wrong) = hale(&["dna", "task", "done", "t-1", "--as", "riley", "--note", "paid"], &app, env);
    assert!(!ok && wrong.contains("handed to sam, not to riley"), "{wrong}");
    // the service goes away; requests are queued, shown as queued, nothing written
    let _ = service.kill();
    let _ = service.wait();
    let (ok, q1) = hale(&["dna", "receipt", "file", "proof.txt", "--as", "sam"], &app, env);
    assert!(ok && q1.contains("queued locally"), "{q1}");
    std::fs::write(app.join("more.txt"), "another bill").unwrap();
    let (ok, q2) = hale(&["dna", "receipt", "file", "more.txt", "--as", "riley"], &app, env);
    assert!(ok && q2.contains("queued locally"), "{q2}");
    let (ok, listed) = hale(&["dna", "queue"], &app, env);
    assert!(ok && listed.contains("2 request(s) waiting") && listed.contains("as sam") && listed.contains("as riley"), "{listed}");
    let (ok, stuck) = hale(&["dna", "queue", "submit"], &app, env);
    assert!(ok && stuck.contains("unreachable") && stuck.contains("stay queued"), "{stuck}");
    // meanwhile sam retires; the record has it before the service is back
    plumb(&app, "person.retired", "sam", "{\"by\": \"riley\", \"to\": \"riley\", \"transferred\": 0}", "riley");
    let mut service = start(&d, &app);
    assert!(wait_up(), "the service did not come back");
    let (ok, sent) = hale(&["dna", "queue", "submit"], &app, env);
    assert!(ok && sent.contains("REFUSED") && sent.contains("sam retired from this organism") && sent.contains("admitted (row"), "{sent}");
    let rows = body(&http(kport, "GET /ledger/rows?from=0 HTTP/1.0\r\nHost: x\r\n\r\n"));
    assert!(rows.contains("more.txt") && !rows.contains("proof.txt"), "riley's row landed, sam's did not:\n{rows}");
    assert!(rows.contains("\"author\": \"riley\""), "in riley's name:\n{rows}");
    let refused_kept = std::fs::read_dir(app.join(".hale/dna/queue")).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".refused")).count();
    assert_eq!(refused_kept, 1, "the refused request is kept for the person");
    let (ok, empty) = hale(&["dna", "queue"], &app, env);
    assert!(ok && empty.contains("nothing queued"), "{empty}");
    // the same request offered again (the queue file restored) lands once
    let kept: Vec<_> = std::fs::read_dir(app.join(".hale/dna/queue")).unwrap().flatten().collect();
    let refused = kept.iter().find(|e| e.file_name().to_string_lossy().ends_with(".refused")).unwrap().path();
    let text = std::fs::read_to_string(&refused).unwrap();
    let first = text.lines().next().unwrap().to_string();
    let again = app.join(".hale/dna/queue/9999-again.json");
    std::fs::write(&again, first.replace("\"as\": \"sam\"", "\"as\": \"riley\"")).unwrap();
    let (ok, dup) = hale(&["dna", "queue", "submit"], &app, env);
    assert!(ok && dup.contains("REFUSED") && dup.contains("refused when it was first submitted"), "a request id the service refused stays refused, whoever resubmits it: {dup}");
    let before = body(&http(kport, "GET /ledger/head HTTP/1.0\r\nHost: x\r\n\r\n"));
    std::fs::write(app.join("third.txt"), "a third bill").unwrap();
    let (ok, out) = hale(&["dna", "receipt", "file", "third.txt", "--as", "riley"], &app, env);
    assert!(ok, "{out}");
    let after = body(&http(kport, "GET /ledger/head HTTP/1.0\r\nHost: x\r\n\r\n"));
    assert_ne!(before, after, "a live request lands directly");
    let _ = service.kill();
    let _ = service.wait();
    let _ = std::fs::remove_dir_all(&d);
}

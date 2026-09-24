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
#[path = "support/trace.rs"]
mod trace;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

fn hale(args: &[&str], cwd: &Path, env: &[(&str, &str)]) -> (bool, String) {
    let _s = trace::Span::new("hale", args.join(" "));
    let mut c = Command::new(env!("CARGO_BIN_EXE_hale"));
    c.args(args).current_dir(cwd).env("HALE_BIN", env!("CARGO_BIN_EXE_hale")).env("HALE_DNA_DISCOVER", "off").env("XDG_CACHE_HOME", std::env::temp_dir().join("hale-tests-iris-cache"));
    for (k, v) in env {
        c.env(k, v);
    }
    let out = c.output().expect("hale");
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn record(app: &Path) -> Vec<serde_json::Value> {
    let _s = trace::Span::new("git", "show refs/dna/journal:journal.jsonl");
    let out = Command::new("git").args(["-C", &app.to_string_lossy(), "show", "refs/dna/journal:journal.jsonl"]).output().unwrap();
    String::from_utf8_lossy(&out.stdout).lines().filter(|l| !l.trim().is_empty()).filter_map(|l| serde_json::from_str(l).ok()).collect()
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

/// The spine: a body run on the spine's role; its stderr in `<name>.stderr`.
fn spine(app: &Path, d: &Path, spine_dsn: &str, name: &str) -> std::process::Child {
    Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["dna", "run", ".", "--no-iris"])
        .current_dir(app)
        .env("HALE_BIN", env!("CARGO_BIN_EXE_hale"))
        .env("HALE_DNA_DISCOVER", "off")
        .env("XDG_CACHE_HOME", std::env::temp_dir().join("hale-tests-iris-cache"))
        .env("HALE_DNA_MEMORY_DSN_SPINE", spine_dsn)
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

/// This record's schema and roles, gone again.
fn unmigrate(app: &Path, owner: &str) {
    let out = Command::new("git").args(["rev-list", "--max-parents=0", "refs/dna/journal"]).current_dir(app).output().unwrap();
    let sch = format!("dna_{}", String::from_utf8_lossy(&out.stdout).trim().to_lowercase());
    let sql = format!("DROP SCHEMA IF EXISTS {sch} CASCADE; DROP ROLE IF EXISTS {sch}_spine; DROP ROLE IF EXISTS {sch}_head");
    let _ = Command::new("psql").args([owner, "-q", "-c", &sql]).output();
}

#[test]
fn an_organism_adopts_the_ledger_and_its_operations_leave_the_record() {
    let _t = trace::test("dna_ledger::adopts");
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
    // the organism opens memory itself (GH #985): its template names no
    // ledger and no knowledge client
    let main = std::fs::read_to_string(app.join("dna/org/main.hl")).unwrap();
    assert!(main.contains("dna::RoutedJournal") && !main.contains("ServiceLedger") && !main.contains("KnowledgeClient"), "the organism reads the record and memory, wired by the core:\n{main}");

    // routing 0: the record alone
    let (ok, st) = hale(&["dna", "ledger"], &app, &[]);
    assert!(ok && st.contains("routing:    0") && st.contains("none named here"), "{st}");
    let (ok, status) = hale(&["dna", "status"], &app, &[]);
    assert!(ok && status.contains("memory:     the record alone (routing 0)"), "{status}");
    // an operational row before adoption lands in the record (a receipt
    // filed from a clone needs no organism beside it)
    std::fs::write(app.join("first.txt"), "the first invoice").unwrap();
    let (ok, out) = hale(&["dna", "receipt", "file", "first.txt", "--as", "sam"], &app, &[]);
    assert!(ok, "{out}");
    let before = record(&app);
    assert!(before.iter().any(|r| r["kind"] == "receipt.filed"), "the receipt row is in the record");

    let Some(owner) = owner_dsn() else {
        eprintln!("dna_ledger: no HALE_DNA_MEMORY_DSN_OWNER; the ledger is memory's, so adoption was not exercised");
        return;
    };
    let _ = owner;
    let spine_dsn = memory_dsn(&app, "HALE_DNA_MEMORY_DSN_SPINE=");
    let head_dsn = memory_dsn(&app, "HALE_DNA_MEMORY_DSN_HEAD=");
    let head: &[(&str, &str)] = &[("HALE_DNA_MEMORY_DSN_HEAD", head_dsn.as_str())];

    // adoption: a head asks, the spine (the body, under the spine's role)
    // carries it out on its tick
    let mut host = spine(&app, &d, &spine_dsn, "run");
    let (ok, adopted) = hale(&["dna", "ledger", "adopt", "--as", "riley"], &app, head);
    assert!(ok && adopted.contains("ledger adoption requested"), "{adopted}");
    let landed = trace::wait_until("the spine adopted the ledger", Duration::from_secs(180), Duration::from_millis(300), || record(&app).iter().any(|r| r["kind"] == "ledger.adopted"));
    assert!(landed, "the spine adopts on its tick:\n{}", std::fs::read_to_string(d.join("run.stderr")).unwrap_or_default());
    let after = record(&app);
    let adopting = after.iter().position(|r| r["kind"] == "ledger.adopting").expect("ledger.adopting");
    let adopted_at = after.iter().position(|r| r["kind"] == "ledger.adopted").expect("ledger.adopted");
    assert!(adopting < adopted_at, "the intent is in the record before the checkpoint");
    let cp = after[adopted_at]["body"].as_str().unwrap();
    assert!(cp.contains("\"routing\": 1") && cp.contains("\"checkpoint\": \"") && cp.contains("\"rows\": "), "{cp}");
    assert!(before.iter().zip(after.iter()).all(|(a, b)| a == b), "the record lost nothing it had");
    let copied = ledger_until(&app, head, |r| r.contains("\"kind\": \"receipt.filed\""));
    assert!(copied.contains("\"kind\": \"receipt.filed\"") && copied.contains("\"author\": \"sam\""), "the operational rows were copied with their authors:\n{copied}");
    assert!(!copied.contains("\"kind\": \"review.requested\""), "the record's own rows were not:\n{copied}");
    let (ok, st) = hale(&["dna", "ledger"], &app, head);
    assert!(ok && st.contains("routing:    1") && st.contains("cutover at"), "{st}");
    let (ok, again) = hale(&["dna", "ledger", "adopt"], &app, head);
    assert!(!ok && again.contains("already on routing 1"), "{again}");

    // after cutover: an operational row is a request in the record, admitted
    // into the ledger by the spine
    std::fs::write(app.join("second.txt"), "the second invoice").unwrap();
    let (ok, out) = hale(&["dna", "receipt", "file", "second.txt", "--as", "sam"], &app, head);
    assert!(ok, "{out}");
    let rows = ledger_until(&app, head, |r| r.contains("second.txt"));
    assert!(rows.contains("second.txt"), "the ledger took it:\n{rows}");
    assert!(!record(&app).iter().any(|r| r["kind"] == "receipt.filed" && r["body"].as_str().unwrap_or("").contains("second.txt")), "the record holds its request, not the row");
    // a head with no memory named still requests: the record is the pager
    std::fs::write(app.join("third.txt"), "the third invoice").unwrap();
    let (ok, out) = hale(&["dna", "receipt", "file", "third.txt", "--as", "sam"], &app, &[]);
    assert!(ok && out.contains("requested"), "{out}");
    let rows = ledger_until(&app, head, |r| r.contains("third.txt"));
    assert!(rows.contains("third.txt"), "and the spine admits it:\n{rows}");
    // an evolutionary row still goes to the record
    let (ok, out) = hale(&["dna", "practice", "propose", "billing/late", "--text", "Chase an invoice at seven days.", "--as", "riley"], &app, head);
    assert!(ok, "{out}");
    assert!(record(&app).iter().any(|r| r["kind"] == "practice.requested" || r["kind"] == "practice.proposed"), "the practice is in the record");
    // history reads one history across both memories
    let (ok, hist) = hale(&["dna", "history"], &app, head);
    assert!(ok && hist.contains("first.txt") && hist.contains("second.txt") && hist.contains("ledger.adopted"), "one history:\n{hist}");

    // stage 2: the body lease is a row of memory (the adoption carried the
    // running body's lease there): a head reads it, a second body is refused
    let (ok, live) = hale(&["dna", "body"], &app, head);
    assert!(ok && live.contains("live on "), "a head reads the lease from memory: {live}");
    let (ok, second) = hale(&["dna", "run", ".", "--no-iris"], &app, &[("HALE_DNA_MEMORY_DSN_SPINE", spine_dsn.as_str())]);
    assert!(!ok && second.contains("lease"), "a second body is refused: {second}");
    // a body without its memory admits nothing
    let (ok, norun) = hale(&["dna", "run", ".", "--no-iris"], &app, &[]);
    assert!(!ok && norun.contains("a body without its memory admits nothing"), "{norun}");
    stop_host(&app, &mut host);
    // the body is gone; its lease stands until forced, in memory
    let (ok, forced) = hale(&["dna", "body", "claim", "--force", "--as", "riley"], &app, head);
    assert!(ok && (forced.contains("body.claimed") || forced.contains("released")), "{forced}");
    let (ok, gone) = hale(&["dna", "body"], &app, head);
    assert!(ok && gone.contains("released"), "the forced claim released the lease in memory: {gone}");

    // abandon: asked of the spine; back to the record alone, the record's
    // rows never removed
    let mut host = spine(&app, &d, &spine_dsn, "run2");
    let (ok, ab) = hale(&["dna", "ledger", "abandon", "--why", "the fixture is done", "--as", "riley"], &app, head);
    assert!(ok && ab.contains("requested"), "{ab}");
    let done = trace::wait_until("the spine abandoned the ledger", Duration::from_secs(180), Duration::from_millis(300), || record(&app).iter().any(|r| r["kind"] == "ledger.abandoned"));
    stop_host(&app, &mut host);
    assert!(done, "the spine abandons on its tick:\n{}", std::fs::read_to_string(d.join("run2.stderr")).unwrap_or_default());
    let (ok, st) = hale(&["dna", "ledger"], &app, head);
    assert!(ok && st.contains("routing:    0"), "{st}");
    assert!(record(&app).iter().any(|r| r["kind"] == "receipt.filed"), "the pre-adoption rows are still there");
    unmigrate(&app, &owner_dsn().unwrap());
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
    // a body kept in git before adoption, to be redacted after it (stage 4)
    std::fs::write(app.join("old-invoice.txt"), "an invoice from before adoption").unwrap();
    let (ok, filed) = hale(&["dna", "receipt", "file", "old-invoice.txt", "--as", "sam"], &app, &[]);
    assert!(ok, "{filed}");
    let old_digest = record(&app).iter().rev().find(|r| r["kind"] == "receipt.filed").map(|r| r["entity"].as_str().unwrap().to_string()).unwrap();
    let old_hex = old_digest.trim_start_matches("sha256:").to_string();
    let kept = Command::new("git").args(["rev-parse", "-q", "--verify", &format!("refs/dna/receipts/{old_hex}")]).current_dir(&app).output().unwrap();
    assert!(kept.status.success(), "the body is a blob of the record");
    let seeded = record(&app);
    let before_kinds: Vec<String> = seeded.iter().map(|r| format!("{} {}", r["kind"].as_str().unwrap_or(""), r["entity"].as_str().unwrap_or(""))).collect();

    // the projections before
    let (ok, history_before) = hale(&["dna", "history"], &app, &[]);
    assert!(ok, "{history_before}");
    let (ok, task_before) = hale(&["dna", "history", "t-1"], &app, &[]);
    assert!(ok && task_before.contains("task.handed") && task_before.contains("task.born"), "{task_before}");
    let (ok, status_before) = hale(&["dna", "status", "--json"], &app, &[]);
    assert!(ok, "{status_before}");

    let Some(owner) = owner_dsn() else {
        eprintln!("dna_ledger: no HALE_DNA_MEMORY_DSN_OWNER; the ledger is memory's, so adoption was not exercised");
        return;
    };
    let spine_dsn = memory_dsn(&app, "HALE_DNA_MEMORY_DSN_SPINE=");
    let head_dsn = memory_dsn(&app, "HALE_DNA_MEMORY_DSN_HEAD=");
    let head: &[(&str, &str)] = &[("HALE_DNA_MEMORY_DSN_HEAD", head_dsn.as_str())];
    let mut host = spine(&app, &d, &spine_dsn, "run");
    let (ok, adopted) = hale(&["dna", "ledger", "adopt", "--as", "riley"], &app, head);
    assert!(ok, "{adopted}");
    let landed = trace::wait_until("the spine adopted the ledger", Duration::from_secs(180), Duration::from_millis(300), || record(&app).iter().any(|r| r["kind"] == "ledger.adopted"));
    assert!(landed, "the spine adopts on its tick:\n{}", std::fs::read_to_string(d.join("run.stderr")).unwrap_or_default());
    // the copy: exactly the operational rows, in order, with their authors
    // (beside them, the running body's own rows: it is the spine that adopts)
    let copied = ledger_until(&app, head, |r| r.contains("schedule.declared"));
    let copied_kinds: Vec<String> = copied.lines().filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok()).map(|v| format!("{} {}", v["kind"].as_str().unwrap_or(""), v["entity"].as_str().unwrap_or(""))).filter(|k| !k.starts_with("body.")).collect();
    let expected: Vec<String> = ["intent.requested i-1", "intent.offered i-1", "task.born t-1", "task.planned t-1", "task.handed t-1", "grant.reserved op-1", "receipt.filed sha256:bbbb", "receipt.redacted sha256:bbbb", "handoff.published handoff:h-1", "schedule.declared weekly-close", &format!("receipt.filed {old_digest}")].iter().map(|s| s.to_string()).collect();
    assert_eq!(copied_kinds, expected, "the operational rows, in order:\n{copied}");
    assert!(copied.contains("\"author\": \"sam\""), "with their authors:\n{copied}");
    // the record kept every row it had; what it gained is the running
    // body's own (its claim, its credential check, its recovery pass over the
    // unfinished intent) and the adoption's
    let now = record(&app);
    let now_kinds: Vec<String> = now.iter().map(|r| format!("{} {}", r["kind"].as_str().unwrap_or(""), r["entity"].as_str().unwrap_or(""))).collect();
    assert_eq!(&now_kinds[..before_kinds.len()], &before_kinds[..], "the record's rows are untouched");
    assert!(now_kinds[before_kinds.len()..].iter().all(|k| k.starts_with("body.") || k.starts_with("spine.") || k.starts_with("ledger.adopt") || k.starts_with("intent.unrecovered")), "and it gained only the body's and the adoption's rows: {:?}", &now_kinds[before_kinds.len()..]);

    // the projections after: the same, across two memories
    let (ok, history_after) = hale(&["dna", "history"], &app, head);
    assert!(ok, "{history_after}");
    // a row's position differs once two memories are read as one (the
    // ledger's rows follow the record's); what is compared is every row
    // without its position
    let strip = |s: &str| -> Vec<String> {
        s.lines()
            .filter(|l| l.contains(" ") && !l.contains("ledger.adopt") && !l.contains("event(s)") && !l.contains("body.") && !l.contains("spine.") && !l.contains("intent.unrecovered"))
            .map(|l| l.trim().trim_start_matches(|c: char| c.is_ascii_digit()).trim().to_string())
            .collect()
    };
    let mut hb = strip(&history_before);
    let mut ha = strip(&history_after);
    hb.sort();
    ha.sort();
    assert_eq!(hb, ha, "history reads the same rows before and after adoption\nbefore:\n{history_before}\nafter:\n{history_after}");
    let (ok, task_after) = hale(&["dna", "history", "t-1"], &app, head);
    assert!(ok && task_after.contains("task.handed") && task_after.contains("task.born") && task_after.contains("sam"), "the handed task keeps its id, its assignee and its bound practice:\n{task_after}");
    let (ok, status_after) = hale(&["dna", "status", "--json"], &app, head);
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
    // stage 4: redacted evidence keeps its treatment across the two
    // memories — a redaction after adoption is a ledger row (asked of the
    // spine), and the body filed before adoption is still a blob of the
    // record: it goes on the next sync after the spine admits it
    let (ok, redacted) = hale(&["dna", "receipt", "redact", &old_digest, "--why", "wrong customer", "--policy", "gdpr", "--as", "riley"], &app, head);
    assert!(ok, "{redacted}");
    let redaction = format!("\"kind\": \"receipt.redacted\", \"entity\": \"{old_digest}\"");
    let rows = ledger_until(&app, head, |r| r.contains(&redaction));
    assert!(rows.contains(&redaction), "the redaction is a row of the ledger:\n{rows}");
    let gone = trace::wait_until("the redacted body left the record", Duration::from_secs(120), Duration::from_millis(300), || {
        let _ = hale(&["dna", "sync"], &app, head);
        !Command::new("git").args(["rev-parse", "-q", "--verify", &format!("refs/dna/receipts/{old_hex}")]).current_dir(&app).output().unwrap().status.success()
    });
    assert!(gone, "and the body filed before adoption is gone from the record");
    let (ok, hist) = hale(&["dna", "history", &old_digest], &app, head);
    assert!(ok && hist.contains("receipt.filed") && hist.contains("receipt.redacted"), "the evidence's history spans both memories:\n{hist}");
    stop_host(&app, &mut host);
    unmigrate(&app, &owner);
    let _ = std::fs::remove_dir_all(&d);
}

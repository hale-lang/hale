//! GH #566 F1, #1590 — the record across clones. One bare repository for
//! the genome, the organism running in clone A, a person in clone B with
//! no nerves of its own: the record is memory's, so B reaches it once its
//! memory is migrated, and B's `hale dna task create` and its verdict are
//! rows A's host reads at once, relays onto the nerves, and answers; B
//! reads the answers. Then, with nothing running, both clones append and
//! each reads the other's row at once: there is nothing to sync.

#[path = "support/vault.rs"]
mod vault;
#[path = "support/habitat.rs"]
mod habitat;
#[path = "support/scratch.rs"]
mod scratch;
#[path = "support/reap.rs"]
mod reap;
#[path = "support/record.rs"]
mod record;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn hale_in(args: &[&str], cwd: &Path) -> (bool, String) {
    let out = vault::hale()
        .args(args)
        .current_dir(cwd)
        .env("HALE_BIN", env!("CARGO_BIN_EXE_hale"))
        .env("HALE_DNA_DISCOVER", "off")
        .env("XDG_CACHE_HOME", std::env::temp_dir().join("hale-tests-iris-cache"))
        .output()
        .expect("hale");
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn git(args: &[&str], cwd: &Path) -> String {
    let out = Command::new("git").args(args).current_dir(cwd).output().expect("git");
    assert!(out.status.success(), "git {args:?} in {}: {}", cwd.display(), String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn record(cwd: &Path) -> Vec<serde_json::Value> {
    // the record's rows and its Ledger's (GH #1590)
    (record::text(cwd) + "\n" + &record::ledger_text(cwd)).lines().filter(|l| !l.trim().is_empty()).map(|l| serde_json::from_str(l).unwrap()).collect()
}

#[test]
fn a_person_in_another_clone_asks_and_decides_through_the_record() {
    let (Some(_nats_owner), Some(owner)) = (std::env::var("HALE_DNA_NATS_URL_OWNER").ok().filter(|d| !d.is_empty()), record::owner_dsn()) else {
        eprintln!("dna_record_sync: no HALE_DNA_NATS_URL_OWNER or HALE_DNA_MEMORY_DSN_OWNER; an ask cannot reach the organism, so nothing was exercised");
        return;
    };
    let d = scratch::Scratch::new(format!("hale_dna_sync_{}", std::process::id()));
    let _reap = reap::ReapOnDrop(d.to_path_buf());
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let bare = d.join("origin.git");
    git(&["init", "-q", "--bare", "-b", "main", &bare.to_string_lossy()], &d);
    // clone A: the organism's home
    habitat::habitat();
    let (ok, out) = hale_in(&["habitat", "graft", "orgsync", "--no-library", "--no-pack"], &d);
    assert!(ok, "{out}");
    let a: PathBuf = d.join("orgsync");
    git(&["config", "user.name", "organism-host"], &a);
    git(&["config", "user.email", "host@dna"], &a);
    git(&["add", "-A"], &a);
    git(&["commit", "-q", "-m", "the app"], &a);
    git(&["remote", "add", "origin", &bare.to_string_lossy()], &a);
    git(&["push", "-q", "origin", "main"], &a);
    let n_born = record(&a).len();
    // clone B: a person, elsewhere, with the genome and, once migrated, the
    // record's memory
    let b = d.join("riley");
    git(&["clone", "-q", &bare.to_string_lossy(), &b.to_string_lossy()], &d);
    git(&["config", "user.name", "riley"], &b);
    git(&["config", "user.email", "riley@l"], &b);
    let (ok, out) = hale_in(&["dna", "memory", "migrate"], &b);
    assert!(ok, "B's memory: {out}");
    let (ok, st) = hale_in(&["dna", "status"], &b);
    assert!(ok && st.contains(&format!("{n_born} event(s), chain verified")) && st.contains("not running"), "{st}");
    // the spine's DSN the organism in A runs on
    let (ok, migrated_a) = hale_in(&["dna", "memory", "migrate"], &a);
    assert!(ok, "A's memory: {migrated_a}");
    let spine = migrated_a.lines().find_map(|l| l.strip_prefix("HALE_DNA_MEMORY_DSN_SPINE=")).expect("the spine's DSN").to_string();

    // the nerves (GH #986): a verdict or a task only reaches the
    // organism over them
    let (ok, migrated) = hale_in(&["dna", "nerves", "migrate"], &a);
    assert!(ok, "{migrated}");
    let nats_spine = migrated.lines().find_map(|l| l.strip_prefix("HALE_DNA_NATS_URL_SPINE=")).expect("the spine's URL").to_string();
    let nats_org = migrated.lines().find_map(|l| l.strip_prefix("HALE_DNA_NATS_ORG=")).expect("the organization's token").to_string();
    // the organism runs in A
    let log = d.join("run.stderr");
    let mut host = vault::hale()
        .args(["dna", "run", ".", "--no-iris"])
        .current_dir(&a)
        .env("HALE_DNA_MEMORY_DSN_SPINE", record::spine(vault::hale(), &a))
        .env("XDG_CACHE_HOME", std::env::temp_dir().join("hale-tests-iris-cache"))
        .env("HALE_BIN", env!("CARGO_BIN_EXE_hale"))
        .env("HALE_DNA_DISCOVER", "off")
        .env("HALE_DNA_NATS_URL_SPINE", &nats_spine)
        .env("HALE_DNA_NATS_ORG", &nats_org)
        .env("HALE_DNA_MEMORY_DSN_SPINE", &spine)
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(&log).unwrap())
        .spawn()
        .expect("hale dna run");
    let up = |_a: &Path| std::fs::read_to_string(&log).unwrap_or_default().contains("the organization reads its facts from the nerves");
    let dl = Instant::now() + Duration::from_secs(180);
    while Instant::now() < dl && !up(&a) {
        std::thread::sleep(Duration::from_millis(200));
    }
    let finish = |host: &mut std::process::Child| {
        let _ = host.kill();
        let _ = host.wait();
        for f in ["org.pid", "app.pid"] {
            if let Ok(pid) = std::fs::read_to_string(a.join(".hale/dna").join(f)) {
                let _ = Command::new("kill").args(["-9", pid.trim()]).status();
            }
        }
        // GH #986: "no organism here" used to be answered by the
        // old socket transport being gone; it is the body lease now
        // (`organism_here`), which outlives the killed process until
        // its TTL — release it so A goes back to being a plain clone
        // (below, both A and B append offline with nobody local)
        let _ = hale_in(&["dna", "body", "release", "--force"], &a);
    };
    if !up(&a) {
        finish(&mut host);
        panic!("the organism never read its facts from the nerves:\n{}", std::fs::read_to_string(&log).unwrap_or_default());
    }
    std::thread::sleep(Duration::from_millis(500));

    // B asks, with no organism and no sockets of its own
    let (ok1, asked) = hale_in(&["dna", "task", "create", "write", "the", "changelog"], &b);
    // B decides the purpose review the same way
    let (ok2, decided) = hale_in(&["dna", "review", "purpose", "approve", "--as", "riley", "--comment", "ratified from afar"], &b);
    let (ok4, st_b) = hale_in(&["dna", "status"], &b);
    finish(&mut host);
    assert!(ok1 && asked.contains("task t1 born"), "ask from the other clone: {asked}");
    assert!(ok2 && decided.contains(" settled: approve by riley"), "verdict from the other clone: {decided}");
    assert!(ok4 && st_b.contains("t1 [") && st_b.contains("settled approve by riley") && st_b.contains("chain verified"), "status in B:\n{st_b}");
    let rows_a = record(&a);
    let rows_b = record(&b);
    assert_eq!(rows_a, rows_b, "one record, read the same from both clones");
    let kinds: Vec<String> = rows_b.iter().map(|r| r["kind"].as_str().unwrap_or("").to_string()).collect();
    for k in ["intent.requested", "intent.offered", "task.born", "review.verdict", "review.settled"] {
        assert!(kinds.iter().any(|x| x == k), "{k} in the record: {kinds:?}");
    }
    let req = rows_b.iter().find(|r| r["kind"] == "intent.requested").unwrap();
    // the asker's identity, as the ask names its asker (`from`)
    let asker = serde_json::from_str::<serde_json::Value>(req["body"].as_str().unwrap_or("{}")).ok().and_then(|v| v["from"].as_str().map(str::to_string)).unwrap_or_default();
    assert!(!asker.is_empty() && req["author"] == asker.as_str(), "the ask carries the asker's identity: {req}");
    let verdict = rows_b.iter().find(|r| r["kind"] == "review.verdict").unwrap();
    assert_eq!(verdict["author"], "riley");
    let settled = rows_b.iter().find(|r| r["kind"] == "review.settled").unwrap();
    assert_eq!(settled["author"], "dna", "the organism's answer is its own");

    // both clones append with nothing running; each reads the other's row
    // at once, in one order
    let n0 = rows_a.len();
    let (ok, out) = hale_in(&["dna", "task", "create", "--no-wait", "tidy", "the", "readme"], &a);
    assert!(ok && out.contains("requested in the ledger"), "{out}");
    let (ok, out) = hale_in(&["dna", "task", "create", "--no-wait", "rename", "the", "topic"], &b);
    assert!(ok && out.contains("requested in the ledger"), "{out}");
    let (ok, out) = hale_in(&["dna", "sync"], &a);
    assert!(ok && out.contains("the record is memory's: nothing to sync"), "{out}");
    let rows = record(&a);
    assert_eq!(rows, record(&b), "both clones read one record");
    assert_eq!(rows.len(), n0 + 2);
    let asks: Vec<(u64, String)> = rows.iter().filter(|r| r["kind"] == "intent.requested").map(|r| (r["seq"].as_u64().unwrap_or(0), r["body"].as_str().unwrap_or("").to_string())).collect();
    let tidy = asks.iter().find(|(_, b)| b.contains("tidy the readme")).expect("A's ask");
    let rename = asks.iter().find(|(_, b)| b.contains("rename the topic")).expect("B's ask");
    assert!(tidy.0 < rename.0, "in the order they were appended: {asks:?}");
    let (ok, st) = hale_in(&["dna", "status"], &b);
    assert!(ok && st.contains("chain verified"), "{st}");
    record::unmigrate(&a, &owner);
    let _ = std::fs::remove_dir_all(&d);
}

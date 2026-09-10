//! GH #566 F1 — the record across clones. One bare repository, the
//! organism running in clone A, a person in clone B with no sockets:
//! B's `hale dna ask` and its verdict are rows in the record, pushed;
//! A's host pulls them, relays them onto the membrane, and pushes the
//! organism's answers back; B reads them. Then, with nothing running,
//! both clones append offline and `sync` reconciles them into one
//! linear record with every event and identical heads.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn hale_in(args: &[&str], cwd: &Path) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(args)
        .current_dir(cwd)
        .env("HALE_BIN", env!("CARGO_BIN_EXE_hale"))
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
    let out = Command::new("git").args(["show", "refs/dna/journal:journal.jsonl"]).current_dir(cwd).output().unwrap();
    String::from_utf8_lossy(&out.stdout).lines().filter(|l| !l.trim().is_empty()).map(|l| serde_json::from_str(l).unwrap()).collect()
}

#[test]
fn a_person_in_another_clone_asks_and_decides_through_the_record() {
    let d = std::env::temp_dir().join(format!("hale_dna_sync_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let bare = d.join("origin.git");
    git(&["init", "-q", "--bare", "-b", "main", &bare.to_string_lossy()], &d);
    // clone A: the organism's home
    let (ok, out) = hale_in(&["dna", "new", "orgsync"], &d);
    assert!(ok, "{out}");
    let a: PathBuf = d.join("orgsync");
    git(&["config", "user.name", "organism-host"], &a);
    git(&["config", "user.email", "host@dna"], &a);
    git(&["add", "-A"], &a);
    git(&["commit", "-q", "-m", "the app"], &a);
    git(&["remote", "add", "origin", &bare.to_string_lossy()], &a);
    git(&["push", "-q", "origin", "main", "refs/dna/*:refs/dna/*"], &a);
    // clone B: a person, elsewhere, with the record fetched
    let b = d.join("riley");
    git(&["clone", "-q", &bare.to_string_lossy(), &b.to_string_lossy()], &d);
    git(&["config", "user.name", "riley"], &b);
    git(&["config", "user.email", "riley@l"], &b);
    let (ok, out) = hale_in(&["dna", "sync"], &b);
    assert!(ok && out.contains("pulled the record"), "{out}");
    let (ok, st) = hale_in(&["dna", "status"], &b);
    assert!(ok && st.contains("7 event(s), chain verified") && st.contains("not running"), "{st}");

    // the organism runs in A
    let mut host = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["dna", "run", ".", "--no-iris"])
        .current_dir(&a)
        .env("XDG_CACHE_HOME", std::env::temp_dir().join("hale-tests-iris-cache"))
        .env("HALE_BIN", env!("CARGO_BIN_EXE_hale"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("hale dna run");
    let up = |a: &Path| a.join(".hale/dna/hale-dna.review.verdict.sock").exists() && a.join(".hale/dna/hale-dna.intent.offered.sock").exists();
    let dl = Instant::now() + Duration::from_secs(90);
    while Instant::now() < dl && !up(&a) {
        std::thread::sleep(Duration::from_millis(200));
    }
    let finish = |host: &mut std::process::Child| {
        let _ = host.kill();
        let _ = host.wait();
        let _ = Command::new("pkill").args(["-x", "orgsync"]).status();
        for s in ["hale-dna.review.verdict.sock", "hale-dna.intent.offered.sock", "hale-dna.expression.observed.sock"] {
            let _ = std::fs::remove_file(a.join(".hale/dna").join(s));
        }
    };
    if !up(&a) {
        finish(&mut host);
        panic!("the membrane did not come up");
    }
    std::thread::sleep(Duration::from_millis(500));

    // B asks, with no organism and no sockets of its own
    let (ok1, asked) = hale_in(&["dna", "ask", "write", "the", "changelog"], &b);
    // B decides the purpose review the same way
    let (ok2, decided) = hale_in(&["dna", "review", "purpose", "approve", "--as", "riley", "--comment", "ratified from afar"], &b);
    std::thread::sleep(Duration::from_secs(3));
    let (ok3, synced) = hale_in(&["dna", "sync"], &b);
    let (ok4, st_b) = hale_in(&["dna", "status"], &b);
    finish(&mut host);
    let (ok5, _) = hale_in(&["dna", "sync"], &a);
    assert!(ok1 && asked.contains("task t1 born"), "ask from the other clone: {asked}");
    assert!(ok2 && decided.contains("review purpose settled: approve by riley"), "verdict from the other clone: {decided}");
    assert!(ok3, "{synced}");
    assert!(ok4 && st_b.contains("t1 [") && st_b.contains("settled approve by riley") && st_b.contains("chain verified"), "status in B:\n{st_b}");
    assert!(ok5);
    let rows_a = record(&a);
    let rows_b = record(&b);
    assert_eq!(git(&["rev-parse", "refs/dna/journal"], &a), git(&["rev-parse", "refs/dna/journal"], &b), "same head in both clones");
    assert_eq!(rows_a.len(), rows_b.len());
    let kinds: Vec<String> = rows_b.iter().map(|r| r["kind"].as_str().unwrap_or("").to_string()).collect();
    for k in ["intent.requested", "intent.offered", "task.born", "review.verdict", "review.settled"] {
        assert!(kinds.iter().any(|x| x == k), "{k} in the record: {kinds:?}");
    }
    let req = rows_b.iter().find(|r| r["kind"] == "intent.requested").unwrap();
    assert_eq!(req["author"], "riley", "the ask carries the asker's identity: {req}");
    let verdict = rows_b.iter().find(|r| r["kind"] == "review.verdict").unwrap();
    assert_eq!(verdict["author"], "riley");
    let settled = rows_b.iter().find(|r| r["kind"] == "review.settled").unwrap();
    assert_eq!(settled["author"], "dna", "the organism's answer is its own");

    // both clones append offline; sync makes one linear record
    let n0 = rows_a.len();
    let (ok, out) = hale_in(&["dna", "ask", "--no-wait", "tidy", "the", "readme"], &a);
    assert!(ok && out.contains("requested in the record"), "{out}");
    let (ok, out) = hale_in(&["dna", "ask", "--no-wait", "rename", "the", "topic"], &b);
    assert!(ok && out.contains("requested in the record"), "{out}");
    // A's ask pushed first; B's ask found the remote ahead and re-appended
    // its own event on top before pushing (ask syncs as it appends), so
    // A now pulls exactly one event and both heads agree
    let (ok, out) = hale_in(&["dna", "sync"], &a);
    assert!(ok && out.contains("pulled 1 event(s)"), "A pulls B's re-appended event: {out}");
    let (ok, out_b) = hale_in(&["dna", "sync"], &b);
    assert!(ok && out_b.contains("up to date"), "{out_b}");
    assert_eq!(git(&["rev-parse", "refs/dna/journal"], &a), git(&["rev-parse", "refs/dna/journal"], &b), "converged");
    let rows = record(&a);
    assert_eq!(rows.len(), n0 + 2);
    let asks: Vec<(u64, String)> = rows.iter().filter(|r| r["kind"] == "intent.requested").map(|r| (r["seq"].as_u64().unwrap_or(0), r["body"].as_str().unwrap_or("").to_string())).collect();
    let tidy = asks.iter().find(|(_, b)| b.contains("tidy the readme")).expect("A's ask");
    let rename = asks.iter().find(|(_, b)| b.contains("rename the topic")).expect("B's ask");
    assert!(tidy.0 < rename.0, "B's event was re-appended after A's: {asks:?}");
    assert_eq!(rename.0 as usize, rows.len() - 1, "at the tail, with its new seq");
    let (ok, st) = hale_in(&["dna", "status"], &b);
    assert!(ok && st.contains("chain verified"), "{st}");
    let _ = std::fs::remove_dir_all(&d);
}

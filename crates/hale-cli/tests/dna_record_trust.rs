//! GH #604 rule 6 — the admission boundary for record-originated rows.
//! Under `dna.trust = signed` the host relays an ask from another clone
//! only when its commit carries a signature git verifies; a row from a
//! writer git does not know is refused in the record and never reaches
//! the organism. The default, `local`, is the other test's world.

#[path = "support/reap.rs"]
mod reap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn hale_in(args: &[&str], cwd: &Path) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
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
    let out = Command::new("git").args(["show", "refs/dna/journal:journal.jsonl"]).current_dir(cwd).output().unwrap();
    String::from_utf8_lossy(&out.stdout).lines().filter(|l| !l.trim().is_empty()).map(|l| serde_json::from_str(l).unwrap()).collect()
}

fn keygen(dir: &Path, name: &str) -> (PathBuf, String) {
    let key = dir.join(name);
    let st = Command::new("ssh-keygen").args(["-q", "-t", "ed25519", "-N", "", "-C", name, "-f", &key.to_string_lossy()]).status().expect("ssh-keygen");
    assert!(st.success());
    let pubkey = std::fs::read_to_string(key.with_extension("pub")).unwrap().trim().to_string();
    (key, pubkey)
}

#[test]
fn a_signed_row_is_relayed_and_an_unverified_one_is_refused() {
    let Some(_nats_owner) = std::env::var("HALE_DNA_NATS_URL_OWNER").ok().filter(|d| !d.is_empty()) else {
        eprintln!("dna_record_trust: no HALE_DNA_NATS_URL_OWNER; a relay cannot reach the organism, so nothing was exercised");
        return;
    };
    let d = std::env::temp_dir().join(format!("hale_dna_trust_{}", std::process::id()));
    let _reap = reap::ReapOnDrop(d.clone());
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let bare = d.join("origin.git");
    git(&["init", "-q", "--bare", "-b", "main", &bare.to_string_lossy()], &d);
    let (ok, out) = hale_in(&["dna", "new", "trusted"], &d);
    assert!(ok, "{out}");
    let a: PathBuf = d.join("trusted");
    git(&["config", "user.name", "organism-host"], &a);
    git(&["config", "user.email", "host@dna"], &a);
    git(&["add", "-A"], &a);
    git(&["commit", "-q", "-m", "the app"], &a);
    git(&["remote", "add", "origin", &bare.to_string_lossy()], &a);
    git(&["push", "-q", "origin", "main", "refs/dna/*:refs/dna/*"], &a);
    // two people, two keys; only riley's is an allowed signer for the organism's host
    let (riley_key, riley_pub) = keygen(&d, "riley_key");
    let (mallory_key, _) = keygen(&d, "mallory_key");
    // the host is a writer too (its refusals are rows), so it has a key of its own
    let (host_key, host_pub) = keygen(&d, "host_key");
    let allowed = d.join("allowed_signers");
    std::fs::write(&allowed, format!("riley@l namespaces=\"git\" {riley_pub}\nhost@dna namespaces=\"git\" {host_pub}\n")).unwrap();
    git(&["config", "dna.trust", "signed"], &a);
    git(&["config", "gpg.format", "ssh"], &a);
    git(&["config", "user.signingkey", &host_key.to_string_lossy()], &a);
    git(&["config", "gpg.ssh.allowedSignersFile", &allowed.to_string_lossy()], &a);
    let clone = |name: &str, email: &str, key: &Path| -> PathBuf {
        let c = d.join(name);
        git(&["clone", "-q", &bare.to_string_lossy(), &c.to_string_lossy()], &d);
        git(&["config", "user.name", name], &c);
        git(&["config", "user.email", email], &c);
        git(&["config", "dna.trust", "signed"], &c);
        git(&["config", "gpg.format", "ssh"], &c);
        git(&["config", "user.signingkey", &key.to_string_lossy()], &c);
        let (ok, out) = hale_in(&["dna", "sync"], &c);
        assert!(ok, "{out}");
        c
    };
    let b = clone("riley", "riley@l", &riley_key);
    let m = clone("mallory", "mallory@l", &mallory_key);

    let (ok, migrated) = hale_in(&["dna", "nerves", "migrate"], &a);
    assert!(ok, "{migrated}");
    let nats_spine = migrated.lines().find_map(|l| l.strip_prefix("HALE_DNA_NATS_URL_SPINE=")).expect("the spine's URL").to_string();
    let nats_org = migrated.lines().find_map(|l| l.strip_prefix("HALE_DNA_NATS_ORG=")).expect("the organization's token").to_string();
    let log = d.join("run.stderr");
    let mut host = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["dna", "run", ".", "--no-iris"])
        .current_dir(&a)
        .env("XDG_CACHE_HOME", std::env::temp_dir().join("hale-tests-iris-cache"))
        .env("HALE_BIN", env!("CARGO_BIN_EXE_hale"))
        .env("HALE_DNA_DISCOVER", "off")
        .env("HALE_DNA_NATS_URL_SPINE", &nats_spine)
        .env("HALE_DNA_NATS_ORG", &nats_org)
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(&log).unwrap())
        .spawn()
        .expect("hale dna run");
    let up = |_a: &Path| std::fs::read_to_string(&log).unwrap_or_default().contains("the organization reads its facts from the nerves");
    let dl = Instant::now() + Duration::from_secs(90);
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
    };
    if !up(&a) {
        finish(&mut host);
        panic!("the organism never read its facts from the nerves:\n{}", std::fs::read_to_string(&log).unwrap_or_default());
    }
    std::thread::sleep(Duration::from_millis(500));

    // riley's ask is a signed commit the host verifies: relayed, a Task born
    let (ok, out) = hale_in(&["dna", "task", "create", "--no-wait", "write", "the", "changelog"], &b);
    assert!(ok, "{out}");
    // mallory's is signed with a key nobody allowed: refused, never relayed
    let (ok, out) = hale_in(&["dna", "task", "create", "--no-wait", "wire", "the", "money", "elsewhere"], &m);
    assert!(ok, "the row itself lands ({out}); admission is the host's");
    let dl = Instant::now() + Duration::from_secs(60);
    let mut seen = (false, false);
    while Instant::now() < dl && !(seen.0 && seen.1) {
        let _ = hale_in(&["dna", "sync"], &b);
        let rows = record(&b);
        seen.0 = rows.iter().any(|r| r["kind"] == "intent.offered" && r["body"].as_str().unwrap_or("").contains("write the changelog"));
        seen.1 = rows.iter().any(|r| r["kind"] == "intent.refused" && r["body"].as_str().unwrap_or("").contains("unverified writer"));
        std::thread::sleep(Duration::from_millis(500));
    }
    finish(&mut host);
    let rows = record(&b);
    let dump: Vec<String> = rows.iter().filter(|r| r["kind"].as_str().unwrap_or("").starts_with("intent.")).map(|r| format!("{} {} {} by {}", r["kind"], r["entity"], r["body"].as_str().unwrap_or("").chars().take(90).collect::<String>(), r["author"])).collect();
    assert!(seen.0, "riley's signed ask was relayed and a Task born:\n{}", dump.join("\n"));
    assert!(seen.1, "mallory's unverified ask was refused in the record:\n{}", dump.join("\n"));
    assert!(!rows.iter().any(|r| r["kind"] == "intent.offered" && r["body"].as_str().unwrap_or("").contains("wire the money")), "and never reached the organism:\n{}", dump.join("\n"));
    let refused = rows.iter().find(|r| r["kind"] == "intent.refused" && r["body"].as_str().unwrap_or("").contains("unverified writer")).unwrap();
    assert_eq!(refused["author"], "host", "refused in the host's name");
    let _ = std::fs::remove_dir_all(&d);
}

/// One row on the clone's record by plumbing alone — the shape `GitRecord`
/// writes, signed with `key` (or unsigned when none) — so a test can hold
/// a local-only row without any verb's sync pushing it first.
fn plumb_row(clone: &Path, key: Option<&Path>, kind: &str, entity: &str, body: &str, author: &str) -> String {
    let head = git(&["rev-parse", "refs/dna/journal"], clone);
    let text = git(&["show", "refs/dna/journal:journal.jsonl"], clone);
    let seq = text.lines().filter(|l| !l.trim().is_empty()).count();
    let line = serde_json::json!({"seq": seq, "kind": kind, "entity": entity, "body": body, "author": author}).to_string();
    let tmp = clone.join(".hale/dna/plumb.jsonl");
    std::fs::create_dir_all(tmp.parent().unwrap()).unwrap();
    std::fs::write(&tmp, format!("{text}\n{line}\n")).unwrap();
    let blob = git(&["hash-object", "-w", &tmp.to_string_lossy()], clone);
    let idx = clone.join(".hale/dna/plumb.index");
    let with_index = |args: &[&str]| {
        let out = Command::new("git").args(args).current_dir(clone).env("GIT_INDEX_FILE", &idx).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    with_index(&["read-tree", &head]);
    with_index(&["update-index", "--add", "--cacheinfo", &format!("100644,{blob},journal.jsonl")]);
    let tree = with_index(&["write-tree"]);
    let mut args: Vec<String> = vec![];
    if let Some(k) = key {
        args.extend(["-c".into(), format!("user.signingkey={}", k.to_string_lossy())]);
    }
    args.extend(["commit-tree".into(), tree, "-p".into(), head.clone()]);
    if key.is_some() {
        args.push("-S".into());
    }
    args.extend(["-m".into(), format!("{kind} {entity}")]);
    let argv: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let commit = git(&argv, clone);
    git(&["update-ref", "refs/dna/journal", &commit, &head], clone);
    commit
}

/// #639, narrowed option 3: a reconcile under `signed` trust rebuilds
/// only the rows this clone signed. A local-only row signed with another
/// key refuses the whole sync before any ref moves, naming the row and
/// the key; the clone's rows and the remote are untouched, and the way
/// that keeps the work is said. Once the clone signs with that key again
/// the reconcile lands, re-signed and naming the original commit. A row
/// never signed is rebuilt unsigned: a reconcile upgrades no provenance.
#[test]
fn a_reconcile_refuses_to_resign_a_row_signed_elsewhere_and_keeps_every_local_row() {
    let d = std::env::temp_dir().join(format!("hale_dna_resign_{}", std::process::id()));
    let _reap = reap::ReapOnDrop(d.clone());
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let bare = d.join("origin.git");
    git(&["init", "-q", "--bare", "-b", "main", &bare.to_string_lossy()], &d);
    let (ok, out) = hale_in(&["dna", "new", "resigned"], &d);
    assert!(ok, "{out}");
    let a: PathBuf = d.join("resigned");
    git(&["config", "user.name", "organism-host"], &a);
    git(&["config", "user.email", "host@dna"], &a);
    git(&["add", "-A"], &a);
    git(&["commit", "-q", "-m", "the app"], &a);
    git(&["remote", "add", "origin", &bare.to_string_lossy()], &a);
    git(&["push", "-q", "origin", "main", "refs/dna/*:refs/dna/*"], &a);
    let (riley_key, riley_pub) = keygen(&d, "riley_key");
    let (host_key, host_pub) = keygen(&d, "host_key");
    let allowed = d.join("allowed_signers");
    std::fs::write(&allowed, format!("riley@l namespaces=\"git\" {riley_pub}\nhost@dna namespaces=\"git\" {host_pub}\n")).unwrap();
    let sign_as = |c: &Path, key: &Path| {
        git(&["config", "dna.trust", "signed"], c);
        git(&["config", "gpg.format", "ssh"], c);
        git(&["config", "user.signingkey", &key.to_string_lossy()], c);
        git(&["config", "gpg.ssh.allowedSignersFile", &allowed.to_string_lossy()], c);
    };
    sign_as(&a, &host_key);
    // riley's clone, signing as riley
    let b = d.join("riley");
    git(&["clone", "-q", &bare.to_string_lossy(), &b.to_string_lossy()], &d);
    git(&["config", "user.name", "riley"], &b);
    git(&["config", "user.email", "riley@l"], &b);
    sign_as(&b, &riley_key);
    let (ok, out) = hale_in(&["dna", "sync"], &b);
    assert!(ok && out.contains("pulled the record"), "{out}");
    let count = |c: &Path| git(&["rev-list", "--count", "refs/dna/journal"], c);
    let head = |c: &Path| git(&["rev-parse", "refs/dna/journal"], c);
    let remote_head = || git(&["rev-parse", "refs/dna/journal"], &bare);

    // a row signed by riley, local to riley's clone (every verb syncs
    // after it appends, so the row is written by plumbing, as a verb would)
    let rileys_row = plumb_row(&b, Some(&riley_key), "intent.requested", "i-changelog", "{\"outcome\": \"write the changelog\"}", "riley");
    assert_eq!(head(&b), rileys_row);
    assert!(git(&["cat-file", "-p", &rileys_row], &b).contains("gpgsig"), "riley's row is a signed commit");
    // the organism's clone appends and pushes: the remote is ahead of riley's base
    let (ok, out) = hale_in(&["dna", "task", "create", "--no-wait", "tidy", "the", "readme"], &a);
    assert!(ok, "{out}");
    let remote_before = remote_head();
    assert_ne!(remote_before, rileys_row);

    // the clone now signs as the host (a rotated key, a shared clone): its
    // reconcile would re-sign riley's row as the host's — refused whole
    sign_as(&b, &host_key);
    let before = (head(&b), count(&b));
    let (ok, out) = hale_in(&["dna", "sync"], &b);
    assert!(!ok, "a reconcile that would re-sign another key's row is refused:\n{out}");
    assert!(out.contains(&rileys_row) && out.contains("was signed with key SHA256:") && out.contains("not this clone's"), "names the row and its key:\n{out}");
    assert!(out.contains("the record was not changed") && out.contains("every local row is kept"), "says what it kept:\n{out}");
    assert!(out.contains("sync first") && !out.to_lowercase().contains("discard"), "says how to keep the work, never to discard it:\n{out}");
    assert_eq!((head(&b), count(&b)), before, "riley's clone is exactly as it was");
    assert_eq!(remote_head(), remote_before, "and the remote too");

    // signing as riley again, the reconcile lands: riley's row re-signed by
    // riley, naming the commit it was rebuilt from
    sign_as(&b, &riley_key);
    let (ok, out) = hale_in(&["dna", "sync"], &b);
    assert!(ok && out.contains("re-appended 1 local event(s)"), "{out}");
    let rebuilt = head(&b);
    assert_ne!(rebuilt, rileys_row);
    let message = git(&["log", "-1", "--format=%B", "refs/dna/journal"], &b);
    assert!(message.contains(&format!("Rebuilt-From: {rileys_row}")), "the rebuilt row names its original:\n{message}");
    let verified = Command::new("git").args(["verify-commit", &rebuilt]).current_dir(&b).output().unwrap();
    assert!(verified.status.success(), "the rebuilt row verifies as riley's: {}", String::from_utf8_lossy(&verified.stderr));
    assert_eq!(remote_head(), rebuilt, "and was pushed");
    let rows = record(&b);
    assert!(rows.iter().any(|r| r["kind"] == "intent.requested" && r["body"].as_str().unwrap_or("").contains("write the changelog") && r["author"] == "riley"), "riley's row, riley's author");

    // a row never signed is rebuilt unsigned
    let unsigned_row = plumb_row(&b, None, "intent.requested", "i-floor", "{\"outcome\": \"sweep the floor\"}", "riley");
    assert!(!git(&["cat-file", "-p", &unsigned_row], &b).contains("gpgsig"), "unsigned");
    let (ok, out) = hale_in(&["dna", "task", "create", "--no-wait", "water", "the", "plants"], &a);
    assert!(ok, "{out}");
    let (ok, out) = hale_in(&["dna", "sync"], &b);
    assert!(ok && out.contains("re-appended 1 local event(s)"), "an unsigned row reconciles: {out}");
    let rebuilt_unsigned = head(&b);
    assert!(!git(&["cat-file", "-p", &rebuilt_unsigned], &b).contains("gpgsig"), "and stays unsigned: a reconcile upgrades no provenance");
    assert!(git(&["log", "-1", "--format=%B", "refs/dna/journal"], &b).contains(&format!("Rebuilt-From: {unsigned_row}")));
    let _ = std::fs::remove_dir_all(&d);
}

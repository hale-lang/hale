//! GH #604 rule 6, #1590 — the admission boundary for record-originated
//! rows. Under `dna.trust = signed` the host relays a person's row only
//! when it carries a signature by the key the record enrolled for them
//! (`hale dna signers`); a row signed with a key the record never
//! enrolled is refused in the record and never reaches the organism. The
//! default, `local`, is the other tests' world.

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
    (record::text(cwd) + "\n" + &record::ledger_text(cwd)).lines().filter(|l| !l.trim().is_empty()).filter_map(|l| serde_json::from_str(l).ok()).collect()
}

fn keygen(dir: &Path, name: &str) -> (PathBuf, PathBuf) {
    let key = dir.join(name);
    let st = Command::new("ssh-keygen").args(["-q", "-t", "ed25519", "-N", "", "-C", name, "-f", &key.to_string_lossy()]).status().expect("ssh-keygen");
    assert!(st.success());
    (key.clone(), key.with_extension("pub"))
}

#[test]
fn a_signed_row_is_relayed_and_an_unverified_one_is_refused() {
    let (Some(_nats_owner), Some(owner)) = (std::env::var("HALE_DNA_NATS_URL_OWNER").ok().filter(|d| !d.is_empty()), record::owner_dsn()) else {
        eprintln!("dna_record_trust: no HALE_DNA_NATS_URL_OWNER or HALE_DNA_MEMORY_DSN_OWNER; a relay cannot reach the organism, so nothing was exercised");
        return;
    };
    let d = scratch::Scratch::new(format!("hale_dna_trust_{}", std::process::id()));
    let _reap = reap::ReapOnDrop(d.to_path_buf());
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let bare = d.join("origin.git");
    git(&["init", "-q", "--bare", "-b", "main", &bare.to_string_lossy()], &d);
    habitat::habitat();
    let (ok, out) = hale_in(&["habitat", "graft", "trusted", "--no-pack"], &d);
    assert!(ok, "{out}");
    let a: PathBuf = d.join("trusted");
    git(&["config", "user.name", "organism-host"], &a);
    git(&["config", "user.email", "host@dna"], &a);
    git(&["add", "-A"], &a);
    git(&["commit", "-q", "-m", "the app"], &a);
    git(&["remote", "add", "origin", &bare.to_string_lossy()], &a);
    git(&["push", "-q", "origin", "main"], &a);
    // two people, two keys; only riley's is enrolled (the record's first
    // enrolment, the bootstrap)
    let (riley_key, riley_pub) = keygen(&d, "riley_key");
    let (mallory_key, _) = keygen(&d, "mallory_key");
    let (ok, out) = hale_in(&["dna", "signers", "enroll", "riley", "--key", &riley_pub.to_string_lossy()], &a);
    assert!(ok && out.contains("enrolled riley"), "{out}");
    git(&["config", "dna.trust", "signed"], &a);
    let clone = |name: &str, key: &Path| -> PathBuf {
        let c = d.join(name);
        git(&["clone", "-q", &bare.to_string_lossy(), &c.to_string_lossy()], &d);
        git(&["config", "user.name", name], &c);
        git(&["config", "user.email", &format!("{name}@l")], &c);
        git(&["config", "dna.trust", "signed"], &c);
        git(&["config", "gpg.format", "ssh"], &c);
        git(&["config", "user.signingkey", &key.to_string_lossy()], &c);
        // the record is memory's: the clone reaches it once migrated
        let (ok, out) = hale_in(&["dna", "memory", "migrate"], &c);
        assert!(ok, "{out}");
        c
    };
    let b = clone("riley", &riley_key);
    let m = clone("mallory", &mallory_key);

    let (ok, migrated) = hale_in(&["dna", "nerves", "migrate"], &a);
    assert!(ok, "{migrated}");
    let nats_spine = migrated.lines().find_map(|l| l.strip_prefix("HALE_DNA_NATS_URL_SPINE=")).expect("the spine's URL").to_string();
    let nats_org = migrated.lines().find_map(|l| l.strip_prefix("HALE_DNA_NATS_ORG=")).expect("the organization's token").to_string();
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
    };
    if !up(&a) {
        finish(&mut host);
        panic!("the organism never read its facts from the nerves:\n{}", std::fs::read_to_string(&log).unwrap_or_default());
    }
    std::thread::sleep(Duration::from_millis(500));

    // mallory's verdict is signed with a key the record never enrolled:
    // refused in the record, never relayed
    let (ok, out) = hale_in(&["dna", "review", "purpose", "reject", "--as", "mallory", "--no-wait"], &m);
    assert!(ok, "the row itself lands ({out}); admission is the host's");
    // riley's is signed with the key the record enrolled for her: relayed,
    // and the Review settles
    let (ok, out) = hale_in(&["dna", "review", "purpose", "approve", "--as", "riley", "--no-wait"], &b);
    assert!(ok, "{out}");
    let dl = Instant::now() + Duration::from_secs(60);
    let mut seen = (false, false);
    while Instant::now() < dl && !(seen.0 && seen.1) {
        let rows = record(&b);
        seen.0 = rows.iter().any(|r| r["kind"] == "review.settled" && r["body"].as_str().unwrap_or("").contains("riley"));
        seen.1 = rows.iter().any(|r| r["kind"] == "review.refused" && r["body"].as_str().unwrap_or("").contains("unverified writer"));
        std::thread::sleep(Duration::from_millis(500));
    }
    finish(&mut host);
    let rows = record(&b);
    let dump: Vec<String> = rows.iter().filter(|r| r["kind"].as_str().unwrap_or("").starts_with("review.")).map(|r| format!("{} {} {} by {}", r["kind"], r["entity"], r["body"].as_str().unwrap_or("").chars().take(90).collect::<String>(), r["author"])).collect();
    assert!(seen.0, "riley's signed verdict was relayed and the Review settled:\n{}", dump.join("\n"));
    assert!(seen.1, "mallory's unverified verdict was refused in the record:\n{}", dump.join("\n"));
    assert!(!rows.iter().any(|r| r["kind"] == "review.settled" && r["body"].as_str().unwrap_or("").contains("reject")), "and never reached the organism:\n{}", dump.join("\n"));
    let refused = rows.iter().find(|r| r["kind"] == "review.refused" && r["body"].as_str().unwrap_or("").contains("unverified writer")).unwrap();
    assert_eq!(refused["author"], "host", "refused in the host's name");
    record::unmigrate(&a, &owner);
    let _ = std::fs::remove_dir_all(&d);
}

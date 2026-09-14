//! GH #604 rule 6 — the admission boundary for record-originated rows.
//! Under `dna.trust = signed` the host relays an ask from another clone
//! only when its commit carries a signature git verifies; a row from a
//! writer git does not know is refused in the record and never reaches
//! the organism. The default, `local`, is the other test's world.

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
    let d = std::env::temp_dir().join(format!("hale_dna_trust_{}", std::process::id()));
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

    let mut host = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["dna", "run", ".", "--no-iris"])
        .current_dir(&a)
        .env("XDG_CACHE_HOME", std::env::temp_dir().join("hale-tests-iris-cache"))
        .env("HALE_BIN", env!("CARGO_BIN_EXE_hale"))
        .env("HALE_DNA_DISCOVER", "off")
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
        for f in ["org.pid", "app.pid"] {
            if let Ok(pid) = std::fs::read_to_string(a.join(".hale/dna").join(f)) {
                let _ = Command::new("kill").args(["-9", pid.trim()]).status();
            }
        }
    };
    if !up(&a) {
        finish(&mut host);
        panic!("the membrane did not come up");
    }
    std::thread::sleep(Duration::from_millis(500));

    // riley's ask is a signed commit the host verifies: relayed, a Task born
    let (ok, out) = hale_in(&["dna", "ask", "--no-wait", "write", "the", "changelog"], &b);
    assert!(ok, "{out}");
    // mallory's is signed with a key nobody allowed: refused, never relayed
    let (ok, out) = hale_in(&["dna", "ask", "--no-wait", "wire", "the", "money", "elsewhere"], &m);
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

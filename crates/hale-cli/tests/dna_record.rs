//! GH #566 F1, #1590 — the record is memory's. `hale habitat graft` pins the
//! record's identity in the genome and births the record in memory;
//! nothing of it is a git ref or a file under `.hale/dna`; a clone of the
//! genome reaches the same record through memory once its memory is
//! migrated; and a clone that still holds a record an earlier toolchain
//! kept on git is refused, to re-init a fresh one.

#[path = "support/vault.rs"]
mod vault;
#[path = "support/habitat.rs"]
mod habitat;
#[path = "support/scratch.rs"]
mod scratch;
#[path = "support/record.rs"]
mod record;
use std::path::{Path, PathBuf};
use std::process::Command;

fn hale(args: &[&str], cwd: &Path) -> (bool, String) {
    let out = vault::hale().args(args).current_dir(cwd).env("HALE_DNA_DISCOVER", "off").env("HALE_DNA_COMPOSE", "off").output().expect("hale");
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn git(args: &[&str], cwd: &Path) -> String {
    let out = Command::new("git").args(["-c", "user.name=riley", "-c", "user.email=r@l"]).args(args).current_dir(cwd).output().expect("git");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
fn the_record_is_memory_s_and_every_clone_reaches_it() {
    let Some(owner) = record::owner_dsn() else {
        eprintln!("dna_record: no HALE_DNA_MEMORY_DSN_OWNER; a record is memory's, so nothing was exercised");
        return;
    };
    let d = scratch::Scratch::new(format!("hale_dna_record_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    habitat::habitat();
    let (ok, out) = hale(&["habitat", "graft", "orgrec", "--no-library", "--no-pack"], &d);
    assert!(ok, "{out}");
    let app: PathBuf = d.join("orgrec");
    // the genome pins the record; nothing of the record is a ref or a file
    let id = record::identity(&app);
    assert_eq!(id.len(), 64, "hale.toml pins the record's identity: {out}");
    assert!(out.contains("born    record ") && out.contains("record.genesis; record.key"), "the record is born in memory: {out}");
    assert_eq!(git(&["for-each-ref", "refs/dna/"], &app), "", "no ref of the record");
    assert!(!app.join(".hale/dna/journal.jsonl").exists(), "no file of the record");
    let rows: Vec<serde_json::Value> = record::text(&app).lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
    assert_eq!(rows.first().map(|r| r["kind"].as_str().unwrap_or("")), Some("record.genesis"), "its first row is its genesis");
    let n = rows.len();
    assert!(n > 3, "and init seeded it: {n} rows");
    let (ok, st) = hale(&["dna", "status"], &app);
    assert!(ok && st.contains(&format!("{n} event(s), chain verified")), "{st}");

    // a clone of the genome has the pin; it reaches the record once its
    // memory is migrated (the head's DSN kept in the clone)
    git(&["add", "-A"], &app);
    git(&["commit", "-q", "-m", "the app"], &app);
    let clone = d.join("clone");
    git(&["clone", "-q", &app.to_string_lossy(), &clone.to_string_lossy()], &d);
    assert_eq!(record::identity(&clone), id, "the clone's genome pins the same record");
    let (ok, migrated) = hale(&["dna", "memory", "migrate"], &clone);
    assert!(ok, "the clone's memory: {migrated}");
    let (ok, after) = hale(&["dna", "history"], &clone);
    assert!(ok && after.contains(&format!("{n} event(s), chain verified")) && after.contains("review.requested"), "{after}");

    // init again keeps the record it has
    habitat::habitat();
    let (ok, out) = hale(&["habitat", "graft", ".", "--attach", "--no-pack"], &app);
    assert!(ok && out.contains("kept    the record"), "init keeps an existing record: {out}");

    // a clone that still holds a record an earlier toolchain kept on git is
    // refused: records are not carried forward
    let old = d.join("old");
    std::fs::create_dir_all(&old).unwrap();
    git(&["init", "-q", "-b", "main"], &old);
    std::fs::write(old.join("README.md"), "an old project\n").unwrap();
    git(&["add", "-A"], &old);
    git(&["commit", "-q", "-m", "base"], &old);
    let head = git(&["rev-parse", "HEAD"], &old);
    git(&["update-ref", "refs/dna/journal", &head], &old);
    habitat::habitat();
    let (ok, refused) = hale(&["habitat", "graft", ".", "--attach", "--no-pack"], &old);
    assert!(!ok && refused.contains("records are not carried forward: re-init a fresh record"), "init refuses a git-era record: {refused}");
    record::unmigrate(&app, &owner);
    let _ = std::fs::remove_dir_all(&d);
}

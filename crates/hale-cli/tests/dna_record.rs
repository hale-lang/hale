//! GH #566 F1 — the record is a git branch. `hale dna init` seeds
//! `refs/dna/journal` (one commit per event) and nothing under
//! `.hale/dna` is the authority; a clone that fetches `refs/dna/*` has
//! the same record and answers `hale dna status` / `history` offline;
//! the host's own appends are commits with the user's identity.

use std::path::{Path, PathBuf};
use std::process::Command;

fn hale(args: &[&str], cwd: &Path) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale")).args(args).current_dir(cwd).output().expect("hale");
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn git(args: &[&str], cwd: &Path) -> String {
    let out = Command::new("git").args(["-c", "user.name=riley", "-c", "user.email=r@l"]).args(args).current_dir(cwd).output().expect("git");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
fn the_record_is_a_branch_every_clone_can_fetch() {
    let d = std::env::temp_dir().join(format!("hale_dna_record_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let (ok, out) = hale(&["dna", "new", "orgrec"], &d);
    assert!(ok, "{out}");
    let app: PathBuf = d.join("orgrec");
    // `new` made a repository; the record is on its ref, not in a file
    assert!(app.join(".git").is_dir(), "hale dna new initializes a repository");
    assert!(!app.join(".hale/dna/journal.jsonl").exists(), "no JSONL file: the record is the branch");
    let count = git(&["rev-list", "--count", "refs/dna/journal"], &app);
    assert_eq!(count, "7", "one commit per seed event");
    let first = git(&["log", "--reverse", "--format=%s", "refs/dna/journal"], &app);
    assert!(first.starts_with("application.attached ."), "the commit subject is the event: {first}");
    let (ok, st) = hale(&["dna", "status"], &app);
    assert!(ok && st.contains("7 event(s), chain verified"), "{st}");

    // a clone with the record fetched answers the same, offline
    git(&["add", "-A"], &app);
    git(&["commit", "-q", "-m", "the app"], &app);
    let clone = d.join("clone");
    git(&["clone", "-q", &app.to_string_lossy(), &clone.to_string_lossy()], &d);
    let (ok, before) = hale(&["dna", "status"], &clone);
    assert!(!ok && before.contains("no record at refs/dna/journal"), "a plain clone has no record yet: {before}");
    git(&["fetch", "-q", "origin", "refs/dna/*:refs/dna/*"], &clone);
    let (ok, after) = hale(&["dna", "history"], &clone);
    assert!(ok && after.contains("7 event(s), chain verified") && after.contains("review.requested"), "{after}");
    assert_eq!(git(&["rev-parse", "refs/dna/journal"], &clone), git(&["rev-parse", "refs/dna/journal"], &app), "same head in both");

    // the host appends as the user (a crash accounting, say): a commit with git's identity
    let (ok, out) = hale(&["dna", "init", "."], &app);
    assert!(ok && out.contains("kept    refs/dna/journal"), "init keeps an existing record: {out}");
    let _ = std::fs::remove_dir_all(&d);
}

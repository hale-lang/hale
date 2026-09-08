//! Runs the DNA Phase 0 domain proof (`dna/tests/`, GH #526) through
//! `hale test`, and `hale verify` over its core, so a compiler change
//! that breaks the domain shapes fails the build here rather than in
//! a friction log. `dna/core` is a library seed (imported by the
//! tests); every fixture is an ordinary Hale program that exits 0.

use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p
}

#[test]
fn dna_fixtures_pass() {
    let dir = repo_root().join("dna/tests");
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("test")
        .arg(&dir)
        .output()
        .expect("invoke hale test dna/tests");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "the DNA fixtures failed.\nstdout:\n{}\nstderr:\n{}",
        stdout,
        stderr
    );
    assert!(stdout.contains(", 0 failed"), "expected a passing summary, got:\n{}", stdout);
}

#[test]
fn dna_core_verifies_clean() {
    let dir = repo_root().join("dna/core");
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("verify")
        .arg(&dir)
        .output()
        .expect("invoke hale verify dna/core");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "hale verify dna/core must report zero findings.\nstdout:\n{}\nstderr:\n{}",
        stdout,
        stderr
    );
}

/// A suite that quietly emptied would pass the check above by
/// running nothing. #526 names eight programs; seven are Hale-native.
#[test]
fn dna_fixture_set_is_complete() {
    let dir = repo_root().join("dna/tests");
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .expect("dna/tests exists")
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            n.ends_with("_test.hl").then_some(n)
        })
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            "assembly_test.hl",
            "fanout_join_test.hl",
            "journal_test.hl",
            "knowledge_test.hl",
            "performers_test.hl",
            "recursion_settlement_test.hl",
            "review_authority_test.hl",
        ]
    );
}

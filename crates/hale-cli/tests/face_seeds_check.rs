//! GH #998: the face's browser fixtures carry two Hale seeds, an
//! Organization source declaration and a Record fixture. They live under
//! `dna/face/` with the rest of the face, so they are checked here rather
//! than in `iris_seeds_check.rs`: a compiler change that breaks either
//! fails this build instead of the browser suite.

use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p
}

fn face() -> PathBuf {
    repo_root().join("dna").join("face")
}

/// Seeds that must `hale check` clean.
const CHECKED: &[&str] = &["tests/organization", "tests/record"];

#[test]
fn every_face_seed_checks_clean() {
    let mut failed = Vec::new();
    for d in CHECKED {
        let out = Command::new(env!("CARGO_BIN_EXE_hale"))
            .arg("check")
            .arg(".")
            .current_dir(face().join(d))
            .output()
            .expect("invoke hale");
        if !out.status.success() {
            failed.push(format!(
                "--- dna/face/{d}\n{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ));
        }
    }
    assert!(
        failed.is_empty(),
        "face seeds failed hale check:\n{}",
        failed.join("\n")
    );
}

#[test]
fn the_seed_list_is_complete() {
    // A new seed under dna/face/ must be added here, so the check cannot
    // quietly stop covering it.
    let mut found: Vec<String> = Vec::new();
    fn walk(dir: &std::path::Path, root: &std::path::Path, out: &mut Vec<String>) {
        let mut has_hl = false;
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            // Playwright's output and node's modules are gitignored and may
            // hold generated .hl files after a browser run; never seeds.
            let generated = p
                .file_name()
                .map(|n| n == "test-results" || n == "playwright-report" || n == "node_modules")
                .unwrap_or(false);
            if p.is_dir() && !generated {
                walk(&p, root, out);
            } else if p.extension().map(|x| x == "hl").unwrap_or(false) {
                has_hl = true;
            }
        }
        if has_hl {
            out.push(
                dir.strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .to_string(),
            );
        }
    }
    let root = face();
    walk(&root, &root, &mut found);
    found.sort();
    let mut expected: Vec<String> = CHECKED.iter().map(|s| s.to_string()).collect();
    expected.sort();
    assert_eq!(found, expected, "face seed list drifted");
}

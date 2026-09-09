//! GH #527 B2: every Hale seed under `iris/` is checked by the compiler's
//! own suite, so a compiler change that breaks the observer fails this
//! build instead of an iris handoff document. The consumer and the
//! inspector are also held to `hale verify` (their READMEs promise it).

use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p
}

fn hale(sub: &str, dir: &str) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg(sub)
        .arg(".")
        .current_dir(repo_root().join("iris").join(dir))
        .output()
        .expect("invoke hale");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), text)
}

/// Seeds that must `hale check` clean. (`examples/wasm-flower` targets
/// wasm32 and is covered by the wasm example tests, not here.)
const CHECKED: &[&str] = &[
    "consumer/fuse-hl",
    "consumer/fuse-hl/upstream-repro",
    "inspect",
    "inspect/upstream-repro",
    "observe",
    "examples/obs-smoke",
    "examples/inspect-demo",
    "examples/claims-demo/app",
    "examples/claims-demo/rogue",
];

/// The shipping consumer and the inspector are held to the discipline gate.
const VERIFIED: &[&str] = &["consumer/fuse-hl", "inspect"];

#[test]
fn every_iris_seed_checks_clean() {
    let mut failed = Vec::new();
    for d in CHECKED {
        let (ok, text) = hale("check", d);
        if !ok {
            failed.push(format!("--- iris/{d}\n{text}"));
        }
    }
    assert!(failed.is_empty(), "iris seeds failed hale check:\n{}", failed.join("\n"));
}

#[test]
fn consumer_and_inspector_verify_clean() {
    let mut failed = Vec::new();
    for d in VERIFIED {
        let (ok, text) = hale("verify", d);
        if !ok {
            failed.push(format!("--- iris/{d}\n{text}"));
        }
    }
    assert!(failed.is_empty(), "iris seeds failed hale verify:\n{}", failed.join("\n"));
}

#[test]
fn the_seed_list_is_complete() {
    // A new seed under iris/ must be added here (or excluded on
    // purpose), so the walk cannot quietly stop covering it.
    let mut found: Vec<String> = Vec::new();
    fn walk(dir: &std::path::Path, root: &std::path::Path, out: &mut Vec<String>) {
        let mut has_hl = false;
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() && !p.ends_with("handoffs") && p.file_name().map(|n| n != ".git").unwrap_or(true) {
                walk(&p, root, out);
            } else if p.extension().map(|x| x == "hl").unwrap_or(false) {
                has_hl = true;
            }
        }
        if has_hl {
            out.push(dir.strip_prefix(root).unwrap().to_string_lossy().to_string());
        }
    }
    let root = repo_root().join("iris");
    walk(&root, &root, &mut found);
    found.sort();
    let mut expected: Vec<String> = CHECKED.iter().map(|s| s.to_string()).collect();
    expected.push("consumer/fuse-hl/attach".to_string()); // FFI shim: checked as part of fuse-hl
    expected.push("examples/wasm-flower".to_string());
    expected.sort();
    assert_eq!(found, expected, "iris seed list drifted");
}

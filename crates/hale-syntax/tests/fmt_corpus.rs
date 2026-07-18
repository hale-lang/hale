//! `hale fmt` corpus anchoring: every fixture example and every
//! stdlib source must (a) format without tripping the token-stream
//! equivalence gate, and (b) be a FIXED POINT of the formatter
//! after one pass — format(format(x)) == format(x). Idempotence is
//! the property that makes fmt safe to run on save / in CI without
//! churn; the gate is the property that makes a formatter bug
//! cosmetic instead of semantic.

use std::fs;
use std::path::PathBuf;

use hale_syntax::fmt::format_source;

fn repo_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // crates/
    p.pop(); // root
    p
}

fn collect_hl(dir: &PathBuf, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            collect_hl(&p, out);
        } else if p.extension().is_some_and(|e| e == "hl") {
            out.push(p);
        }
    }
}

#[test]
fn corpus_formats_and_is_idempotent() {
    let root = repo_root();
    let mut files = Vec::new();
    collect_hl(
        &root.join("crates/hale-codegen/tests/fixtures/examples"),
        &mut files,
    );
    collect_hl(
        &root.join("crates/hale-codegen/runtime/stdlib"),
        &mut files,
    );
    assert!(
        files.len() > 80,
        "corpus discovery broke: only {} files",
        files.len()
    );

    let mut gate_failures = Vec::new();
    let mut non_idempotent = Vec::new();
    for f in &files {
        let src = fs::read_to_string(f).expect("read corpus file");
        let once = match format_source(&src) {
            Ok(o) => o,
            Err(e) => {
                gate_failures.push(format!("{}: {:?}", f.display(), e));
                continue;
            }
        };
        match format_source(&once) {
            Ok(twice) => {
                if twice != once {
                    non_idempotent.push(f.display().to_string());
                }
            }
            Err(e) => {
                gate_failures
                    .push(format!("{} (2nd pass): {:?}", f.display(), e));
            }
        }
    }
    assert!(
        gate_failures.is_empty(),
        "fmt gate failures:\n{}",
        gate_failures.join("\n")
    );
    assert!(
        non_idempotent.is_empty(),
        "fmt not idempotent on:\n{}",
        non_idempotent.join("\n")
    );
}

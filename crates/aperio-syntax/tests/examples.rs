//! Integration test: every example program in
//! `crates/aperio-codegen/tests/fixtures/examples/` parses
//! cleanly. This is the corpus the parser is empirically anchored
//! against — examples double as the language's acceptance test
//! suite, so the compiler must keep up with them.

use std::fs;
use std::path::{Path, PathBuf};

fn examples_dir() -> PathBuf {
    // Cargo runs tests with `crates/aperio-syntax` as cwd; walk up
    // to the repo root, then into the codegen crate's fixtures.
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // crates/
    p.pop(); // root
    p.push("crates");
    p.push("aperio-codegen");
    p.push("tests");
    p.push("fixtures");
    p.push("examples");
    p
}

fn ap_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if !dir.exists() {
        return out;
    }
    for entry in fs::read_dir(dir).expect("read examples dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            out.extend(ap_files(&path));
        } else if path.extension().and_then(|s| s.to_str()) == Some("ap") {
            out.push(path);
        }
    }
    out
}

#[test]
fn all_examples_parse() {
    let dir = examples_dir();
    let files = ap_files(&dir);
    assert!(!files.is_empty(), "no example .ap files found in {}", dir.display());

    let mut failures = Vec::new();
    for path in &files {
        let source = fs::read_to_string(path).expect("read .ap file");
        match aperio_syntax::parse_source(&source) {
            Ok(_) => {}
            Err(diags) => {
                let rendered: Vec<String> =
                    diags.iter().map(|d| d.render(&source)).collect();
                failures.push(format!(
                    "{}:\n  {}",
                    path.strip_prefix(&dir.parent().unwrap()).unwrap_or(path).display(),
                    rendered.join("\n  ")
                ));
            }
        }
    }
    if !failures.is_empty() {
        panic!(
            "parse failures in {} of {} example files:\n\n{}",
            failures.len(),
            files.len(),
            failures.join("\n\n")
        );
    }
    eprintln!("parsed {} example files cleanly", files.len());
}

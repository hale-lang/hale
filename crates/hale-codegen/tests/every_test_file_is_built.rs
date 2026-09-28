//! Every file under `tests/` is built, into exactly one binary.
//!
//! This crate's test targets are listed in `Cargo.toml` (`autotests =
//! false`) rather than discovered: an area root (`tests/<area>.rs`)
//! pulls its files in as modules with `#[path]`, so the crate links once
//! per area instead of once per file. The price of a list is that a new
//! `tests/foo.rs` nobody listed is not a binary at all: it compiles
//! nowhere and its tests never run, and nothing says so. This is what
//! says so.
//!
//! A file is built when it is a `[[test]]` target of its own, or when
//! exactly one area root names it in a `#[path = "<file>.rs"]`.

use std::collections::BTreeMap;
use std::path::PathBuf;

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn files_in_tests() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(crate_dir().join("tests"))
        .expect("tests dir")
        .flatten()
        .filter_map(|e| e.file_name().to_str().map(String::from))
        .filter(|n| n.ends_with(".rs"))
        .collect();
    v.sort();
    v
}

/// The `path = "tests/<file>.rs"` of every `[[test]]` in Cargo.toml.
fn test_targets() -> Vec<String> {
    let manifest = std::fs::read_to_string(crate_dir().join("Cargo.toml")).expect("Cargo.toml");
    manifest
        .lines()
        .filter_map(|l| l.trim().strip_prefix("path = \"tests/"))
        .filter_map(|l| l.strip_suffix('"'))
        .map(String::from)
        .collect()
}

/// The top-level files a root pulls in: `#[path = "<file>.rs"]`,
/// leaving out the `support/` helpers every file includes for itself.
fn modules_of(root: &str) -> Vec<String> {
    let text = std::fs::read_to_string(crate_dir().join("tests").join(root)).expect("root");
    text.lines()
        .filter_map(|l| l.trim().strip_prefix("#[path = \""))
        .filter_map(|l| l.strip_suffix("\"]"))
        .filter(|p| !p.contains('/'))
        .map(String::from)
        .collect()
}

#[test]
fn every_test_file_is_built_into_exactly_one_binary() {
    let targets = test_targets();
    let mut built: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for t in &targets {
        built.entry(t.clone()).or_default().push(format!("its own binary `{t}`"));
        for m in modules_of(t) {
            built.entry(m).or_default().push(format!("a module of `{t}`"));
        }
    }
    let files = files_in_tests();
    let missing: Vec<&String> = files.iter().filter(|f| !built.contains_key(*f)).collect();
    assert!(
        missing.is_empty(),
        "these files under tests/ are built into no binary, so their tests never run: {missing:?}\n\
         Add a `#[path = \"<file>.rs\"] mod <file>;` line to the area root it belongs to, or list it as its own [[test]] in Cargo.toml."
    );
    let twice: Vec<String> = built
        .iter()
        .filter(|(_, by)| by.len() > 1)
        .map(|(f, by)| format!("{f}: {}", by.join(" and ")))
        .collect();
    assert!(twice.is_empty(), "built more than once (its tests would run twice):\n{}", twice.join("\n"));
    let unknown: Vec<&String> = built.keys().filter(|f| !files.contains(f)).collect();
    assert!(unknown.is_empty(), "named by Cargo.toml or an area root but not there: {unknown:?}");
}

#[test]
fn the_scan_is_not_vacuous() {
    assert!(test_targets().len() >= 20, "read {} [[test]] targets from Cargo.toml", test_targets().len());
    assert!(files_in_tests().len() >= 100, "read {} files under tests/", files_in_tests().len());
}

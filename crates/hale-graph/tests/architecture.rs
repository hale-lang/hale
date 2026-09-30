//! F.40 architecture canaries: `hale-graph` depends on nothing, and
//! `hale-frontend` links no LLVM.
//!
//! The crate boundary is what keeps the AST, the checker and codegen
//! out of the core; a rule in a document would not. The same law
//! holds `hale-model` (its own tests/architecture.rs), and phase 1
//! rebuilds `hale-model` on this crate without loosening either.

use std::path::{Path, PathBuf};

/// The dependency lines of a manifest: `[dependencies]` and every
/// `[target.<cfg>.dependencies]`, comments and blanks aside.
fn dependency_lines(manifest: &str) -> Vec<String> {
    let mut in_deps = false;
    let mut out = Vec::new();
    for line in manifest.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            in_deps = t == "[dependencies]"
                || t.starts_with("[dependencies.")
                || (t.starts_with("[target.") && t.contains(".dependencies"));
            continue;
        }
        if in_deps && !t.is_empty() && !t.starts_with('#') {
            out.push(t.to_string());
        }
    }
    out
}

fn crates_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("crates/").to_path_buf()
}

#[test]
fn hale_graph_depends_on_nothing() {
    let manifest = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
        .expect("read Cargo.toml");
    let offenders = dependency_lines(&manifest);
    assert!(
        offenders.is_empty(),
        "hale-graph must stay dependency-free (generic mechanics only; \
         row meanings live with their families): {offenders:?}"
    );
}

/// `hale-frontend` is the loader the CLI and the LSP share (F.40
/// phase 2): it must build without LLVM, and it must not depend on
/// the server it serves. The walk follows every workspace path
/// dependency, so a dependency that itself pulls in codegen is caught
/// too, not just a direct one.
#[test]
fn hale_frontend_links_no_llvm_and_not_the_lsp() {
    const FORBIDDEN: &[&str] = &["hale-codegen", "hale-lsp", "hale-cli", "inkwell", "llvm-sys"];
    let crates = crates_dir();
    let mut stack = vec!["hale-frontend".to_string()];
    let mut seen = std::collections::BTreeSet::new();
    let mut offenders = Vec::new();
    while let Some(krate) = stack.pop() {
        if !seen.insert(krate.clone()) {
            continue;
        }
        let path = crates.join(&krate).join("Cargo.toml");
        let manifest = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        for dep in dependency_lines(&manifest) {
            let name = dep.split(['=', ' ']).next().unwrap_or("").trim().to_string();
            if FORBIDDEN.contains(&name.as_str()) {
                offenders.push(format!("{krate} -> {name}"));
            }
            if name.starts_with("hale-") && dep.contains("path") {
                stack.push(name);
            }
        }
    }
    assert!(
        seen.len() >= 3,
        "the walk is vacuous: it reached only {seen:?}"
    );
    assert!(
        offenders.is_empty(),
        "hale-frontend must stay LLVM-free and below the LSP (the CLI and the \
         LSP share it); these dependencies break that: {offenders:?}"
    );
}

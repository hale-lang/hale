//! Identity coverage (F.40 phase 0, step 0.4).
//!
//! Three identities name the compiler that produced something: the
//! replay identity (`HALE_TOOLCHAIN_SHA256`, `crates/hale-cli/build.rs`),
//! the toolchain cache key the DNA host and observer are cached under
//! (`HALE_COMPILER_SRC_HASH`, `crates/hale-iris/build.rs`), and the
//! stale-binary hash (`HALE_CODEGEN_SRC_HASH`, the same script and
//! `crates/hale-cli/src/shared/stale.rs`). Each walked its own list
//! of directories with its own walk, and none of the lists named
//! `hale-model`, so a model-shape change did not bust a cached host or
//! refuse a recording. A semantic producer moving between crates in
//! phase 1 must never make a later edit invisible to any of them.
//!
//! This module is the one list and the one walk. Build scripts link
//! it as a build-dependency (the crate depends on nothing, so there
//! is no cycle), `stale.rs` links it as a dependency, and the walk
//! they share is what makes the stale hash's build-time value and
//! run-time recomputation equal by construction.

use std::path::{Path, PathBuf};

/// The crates whose sources every compiler identity covers, in
/// workspace order: what shapes a compiled program or a recording.
pub const COVERED_CRATES: &[&str] = &[
    "hale-syntax",
    "hale-types",
    "hale-model",
    "hale-graph",
    "hale-codegen",
    "hale-stdlib",
];

/// The workspace members no identity covers, each with the reason.
/// `hale-cli` is covered by the replay identity alone (it does not
/// shape a cached host's binary); the rest shape nothing a program
/// runs.
pub const NOT_COVERED: &[(&str, &str)] = &[
    (
        "hale-cli",
        "the replay identity walks it; a cached host's binary does not depend on the CLI",
    ),
    (
        "hale-lsp",
        "serves diagnostics; emits no artifact and no recording",
    ),
    (
        "hale-iris",
        "its embedded observer and DNA trees ride the cache key as files through all_files(); its Rust only materializes them",
    ),
    (
        "hale-dna",
        "its embedded source tree rides the cache key as files and EMBEDDED_DIGEST names it; its Rust only carries them",
    ),
    (
        "hale-corpus",
        "test programs for the workspace's own tests",
    ),
    (
        "hale-ts-shim",
        "its library is hale-codegen/runtime/lotus_treesitter.rs, covered under hale-codegen; what else shapes a std::ts binary is the tree-sitter versions its manifest and the lock file pin, which MANIFEST_FILES carries",
    ),
];

/// Files outside the covered crates' source trees that shape a
/// compiled program: the lock file (every dependency version the
/// compiler and its shims are built with) and the ts-shim manifest
/// (the tree-sitter versions `std::ts` links). The replay identity
/// frames them beside the sources.
pub const MANIFEST_FILES: &[&str] = &["Cargo.lock", "crates/hale-ts-shim/Cargo.toml"];

/// The subdirectories of a covered crate that hold sources.
pub const SOURCE_DIRS: &[&str] = &["src", "runtime", "hl"];

/// The source extensions an identity reads.
pub const SOURCE_EXTENSIONS: &[&str] = &["rs", "c", "h", "hl"];

/// Every source file under `dir`, recursively, in path order. A
/// missing directory contributes nothing.
pub fn walk_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut items: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    items.sort();
    for p in items {
        if p.is_dir() {
            walk_sources(&p, out);
        } else if p
            .extension()
            .and_then(|s| s.to_str())
            .map(|e| SOURCE_EXTENSIONS.contains(&e))
            .unwrap_or(false)
        {
            out.push(p);
        }
    }
}

/// The source directories of the covered crates plus `extra` crates,
/// under `workspace_root`, in list order: the directories an identity
/// declares `rerun-if-changed` on and walks. Only directories that
/// exist: Cargo treats a missing `rerun-if-changed` path as always
/// stale, which rebuilt hale-iris on every cargo invocation until
/// this filter (F.40 phase-0 review).
pub fn covered_dirs(workspace_root: &Path, extra: &[&str]) -> Vec<PathBuf> {
    COVERED_CRATES
        .iter()
        .chain(extra.iter())
        .flat_map(|krate| {
            SOURCE_DIRS
                .iter()
                .map(move |d| workspace_root.join("crates").join(krate).join(d))
        })
        .filter(|d| d.is_dir())
        .collect()
}

/// The manifest files an identity frames beside the sources, those
/// that exist under `workspace_root`.
pub fn manifest_files(workspace_root: &Path) -> Vec<PathBuf> {
    MANIFEST_FILES
        .iter()
        .map(|f| workspace_root.join(f))
        .filter(|p| p.is_file())
        .collect()
}

/// The files the stale-binary hash covers: `codegen.rs`, the C runtime's
/// `lotus_arena.c`, and every `.hl` seed of the stdlib. One list for
/// `build.rs` (`HALE_CODEGEN_SRC_HASH`) and `stale.rs` (its run-time
/// recomputation), with the same path strings.
pub fn stale_hash_paths(codegen_dir: &Path) -> Vec<PathBuf> {
    let mut paths = vec![
        codegen_dir.join("src").join("codegen.rs"),
        codegen_dir.join("runtime").join("lotus_arena.c"),
    ];
    let mut seeds = Vec::new();
    walk_sources(
        &codegen_dir.join("..").join("hale-stdlib").join("hl"),
        &mut seeds,
    );
    seeds.retain(|p| p.extension().and_then(|s| s.to_str()) == Some("hl"));
    paths.extend(seeds);
    paths
}

/// Every source file of the covered crates plus `extra`, in path
/// order.
pub fn covered_files(workspace_root: &Path, extra: &[&str]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for d in covered_dirs(workspace_root, extra) {
        walk_sources(&d, &mut files);
    }
    files
}

/// A 64-bit FNV-1a fold over `(relative path, NUL, contents, NUL)` for
/// every file, in the order given: the cache key's fold. A renamed,
/// added or removed file moves it, as does one changed byte.
pub fn fold_files(root: &Path, files: &[PathBuf]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |bytes: &[u8]| {
        for b in bytes {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x100_0000_01b3);
        }
    };
    for f in files {
        let rel = f
            .strip_prefix(root)
            .unwrap_or(f)
            .to_string_lossy()
            .replace('\\', "/");
        eat(rel.as_bytes());
        eat(&[0]);
        eat(&std::fs::read(f).unwrap_or_default());
        eat(&[0]);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_covered_change_moves_the_fold_and_order_is_by_path() {
        let dir = std::env::temp_dir().join(format!("hale-graph-identity-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("b")).unwrap();
        std::fs::write(dir.join("b/two.rs"), "fn two() {}").unwrap();
        std::fs::write(dir.join("one.hl"), "fn main() {}").unwrap();
        std::fs::write(dir.join("notes.md"), "not a source").unwrap();
        let mut files = Vec::new();
        walk_sources(&dir, &mut files);
        assert_eq!(files.len(), 2, "only source extensions, recursively");
        assert!(
            files[0].ends_with("b/two.rs") && files[1].ends_with("one.hl"),
            "path order"
        );
        let a = fold_files(&dir, &files);
        std::fs::write(dir.join("one.hl"), "fn main() { }").unwrap();
        let b = fold_files(&dir, &files);
        assert_ne!(a, b, "one changed byte moves the identity");
        std::fs::write(dir.join("three.c"), "int x;").unwrap();
        let mut files2 = Vec::new();
        walk_sources(&dir, &mut files2);
        let c = fold_files(&dir, &files2);
        assert_ne!(b, c, "an added file moves the identity");
        assert_eq!(fold_files(&dir, &files2), c, "the fold is deterministic");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

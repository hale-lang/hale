use std::collections::hash_map::DefaultHasher;
use std::hash::Hasher;
use std::path::{Path, PathBuf};
use std::{env, fs};

/// Phase 2i: warn when the CLI binary's bundled codegen + runtime
/// source snapshots are stale relative to the workspace's on-disk
/// source. Both the baked-in hash (set at build time by
/// `build.rs`) and the runtime-recomputed hash use the same
/// algorithm — DefaultHasher over each file's bytes, salted with
/// the relative path — so they match exactly when the on-disk
/// tree is the one the binary was built against.
///
/// Skipped silently when:
///  - `HALE_SKIP_STALE_CHECK=1` is set,
///  - the baked codegen directory doesn't exist on this host
///    (installed binary, moved workspace),
///  - `build.rs` couldn't locate the workspace at build time
///    (the env vars are empty).
pub(crate) fn check_stale_cli() {
    if env::var_os("HALE_SKIP_STALE_CHECK")
        .filter(|v| !v.is_empty() && v != "0")
        .is_some()
    {
        return;
    }
    let baked_hash = env!("HALE_CODEGEN_SRC_HASH");
    let baked_dir = env!("HALE_CODEGEN_DIR");
    if baked_hash.is_empty() || baked_dir.is_empty() {
        return;
    }
    let codegen_dir = Path::new(baked_dir);
    if !codegen_dir.exists() {
        return;
    }
    check_stale_dna(codegen_dir);
    let current = compute_codegen_src_hash(codegen_dir);
    if current != baked_hash {
        eprintln!(
            "warning: hale CLI binary was built against an older \
             codegen+runtime source tree."
        );
        eprintln!(
            "         {} has changed since the CLI was built; the \
             emitted binary may use stale lowering.",
            codegen_dir.display()
        );
        eprintln!(
            "         Rebuild with: cargo build -p hale-cli"
        );
        eprintln!(
            "         (Set HALE_SKIP_STALE_CHECK=1 to silence \
             this warning.)"
        );
    }
}

/// GH #785: the same warning for the DNA source set. `hale dna new`,
/// `init` and `upgrade` materialize the `dna/` the binary EMBEDS
/// (`hale_dna::EMBEDDED_DIGEST`, GH #726), and every organism a
/// fixture starts runs that core — so a `dna/core` edited after the
/// last build runs nowhere, and nothing said so until `hale dna
/// status` was asked. The tree digested is the workspace the binary
/// was built from (the codegen dir's workspace), or the one
/// `HALE_STALE_DNA_ROOT` names — the regression test's way to hand
/// the check a tree it may edit.
pub(crate) fn check_stale_dna(codegen_dir: &Path) {
    let root = match env::var_os("HALE_STALE_DNA_ROOT").filter(|v| !v.is_empty()) {
        Some(v) => PathBuf::from(v),
        None => match codegen_dir.parent().and_then(|p| p.parent()) {
            Some(r) => r.to_path_buf(),
            None => return,
        },
    };
    if !root.join("dna").is_dir() {
        return;
    }
    let Ok(current) = hale_dna::digest_of_tree(&root) else {
        return;
    };
    if current != hale_dna::EMBEDDED_DIGEST {
        eprintln!(
            "warning: hale CLI binary embeds an older dna/ source set."
        );
        eprintln!(
            "         {} has changed since the CLI was built; `hale dna \
             new`, `init` and `upgrade` materialize what the binary \
             carries, and an organism a fixture starts runs that.",
            root.join("dna").display()
        );
        eprintln!("         Rebuild with: cargo build --release");
        eprintln!(
            "         (Set HALE_SKIP_STALE_CHECK=1 to silence this \
             warning.)"
        );
    }
}

/// The same walk as `build.rs`'s `HALE_CODEGEN_SRC_HASH`, with the
/// same path strings and the shared `hale_graph::identity` walk:
/// codegen.rs, lotus_arena.c and every `.hl` under
/// `crates/hale-stdlib/hl` (F.40 phase 0: the list followed the
/// stdlib when it moved out of `codegen/runtime/stdlib`; until then
/// this hash covered two files).
pub(crate) fn compute_codegen_src_hash(codegen_dir: &Path) -> String {
    let paths: Vec<PathBuf> = hale_graph::identity::stale_hash_paths(codegen_dir);
    let mut hasher = DefaultHasher::new();
    for path in &paths {
        if let Ok(bytes) = fs::read(path) {
            hasher.write(path.to_string_lossy().as_bytes());
            hasher.write(&[0u8]);
            hasher.write(&bytes);
        }
    }
    format!("{:016x}", hasher.finish())
}


use std::collections::BTreeSet;
use std::env;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Warn when the CLI binary was built from a source tree other than
/// the one on disk now: the identity-covered sources
/// (`hale_graph::identity::identity_files`, every source of the
/// covered crates and the manifests) folded with the shared fold, and
/// compared with the value `build.rs` baked by the same walk and the
/// same fold (`HALE_STALE_SRC_HASH`). See [`stale_sources`] for what an
/// ordinary invocation pays.
///
/// Skipped silently when:
///  - `HALE_SKIP_STALE_CHECK=1` is set,
///  - the baked workspace doesn't exist on this host (installed
///    binary, moved workspace),
///  - `build.rs` couldn't locate the workspace at build time
///    (the env vars are empty).
pub(crate) fn check_stale_cli() {
    if env::var_os("HALE_SKIP_STALE_CHECK")
        .filter(|v| !v.is_empty() && v != "0")
        .is_some()
    {
        return;
    }
    let baked_hash = env!("HALE_STALE_SRC_HASH");
    let baked_root = env!("HALE_STALE_ROOT");
    if baked_hash.is_empty() || baked_root.is_empty() {
        return;
    }
    let root = Path::new(baked_root);
    if !root.join("crates").is_dir() {
        return;
    }
    check_stale_dna(root);
    let baked = Baked {
        hash: baked_hash,
        files: env!("HALE_STALE_SRC_COUNT").parse().unwrap_or(0),
    };
    // A binary whose own time cannot be read is compared by its fold.
    let built_at = env::current_exe()
        .and_then(std::fs::metadata)
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH);
    if stale_sources(root, &baked, built_at) == Staleness::Stale {
        eprintln!(
            "warning: hale CLI binary was built from an older compiler \
             source tree."
        );
        eprintln!(
            "         a source under {} that the binary is built from has \
             changed since the CLI was built; what it emits may be stale.",
            root.display()
        );
        eprintln!("         Rebuild with: cargo build --release");
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
/// was built from, or the one `HALE_STALE_DNA_ROOT` names — the
/// regression test's way to hand the check a tree it may edit.
pub(crate) fn check_stale_dna(workspace_root: &Path) {
    let root = match env::var_os("HALE_STALE_DNA_ROOT").filter(|v| !v.is_empty()) {
        Some(v) => PathBuf::from(v),
        None => workspace_root.to_path_buf(),
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

/// What `build.rs` baked: the fold of the identity-covered sources
/// (`HALE_STALE_SRC_HASH`) and how many files it folded
/// (`HALE_STALE_SRC_COUNT`).
pub(crate) struct Baked<'a> {
    pub(crate) hash: &'a str,
    pub(crate) files: usize,
}

/// What [`stale_sources`] found, and how far it had to look.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Staleness {
    /// Every covered file, and every directory holding one, is no newer
    /// than the binary and the count is the baked one: nothing read.
    Fresh,
    /// Something was newer, the fold was taken, and it is the baked one.
    Unchanged,
    /// The fold differs: the binary was built from other sources.
    Stale,
}

/// The stale-binary hash (F.40 phase 4, I4): the identity-covered
/// sources under `root`, held against what was baked at `built_at`.
///
/// It reads nothing on an ordinary invocation. The walk of the shared
/// selection lists the files (a `read_dir` per directory), then each
/// file and each directory holding one is `stat`ed: an edited or added
/// file is newer than the binary, a removed or renamed one leaves its
/// directory newer, and a count other than the baked one is a removal
/// too. Only then is every file read and folded with the build's fold,
/// so a file touched and left as it was costs a fold and warns nothing.
pub(crate) fn stale_sources(root: &Path, baked: &Baked, built_at: SystemTime) -> Staleness {
    let files = hale_graph::identity::identity_files(root);
    let newer = |p: &Path| {
        std::fs::metadata(p)
            .and_then(|m| m.modified())
            .map(|t| t > built_at)
            .unwrap_or(true)
    };
    let dirs: BTreeSet<&Path> = files.iter().filter_map(|f| f.parent()).collect();
    let touched = files.len() != baked.files
        || files.iter().any(|f| newer(f))
        || dirs.iter().any(|d| newer(d));
    if !touched {
        return Staleness::Fresh;
    }
    let current = format!("{:016x}", hale_graph::identity::fold_files(root, &files));
    if current == baked.hash {
        Staleness::Unchanged
    } else {
        Staleness::Stale
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hale_graph::identity::{fold_files, identity_files, COVERED_CRATES, MANIFEST_FILES};
    use std::time::Duration;

    /// A scratch tree with every covered crate's `src`, the manifests,
    /// and a file of a crate no identity covers, each dated `at`.
    fn scratch(tag: &str, at: SystemTime) -> PathBuf {
        let root = std::env::temp_dir().join(format!("hale-cli-stale-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut files: Vec<PathBuf> = COVERED_CRATES
            .iter()
            .map(|k| root.join("crates").join(k).join("src").join("lib.rs"))
            .collect();
        files.push(root.join("crates/hale-codegen/runtime/lotus_arena.c"));
        files.push(root.join("crates/hale-stdlib/hl/core.hl"));
        files.extend(MANIFEST_FILES.iter().map(|m| root.join(m)));
        files.push(root.join("crates/hale-lsp/src/lib.rs"));
        for f in &files {
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, format!("// {}\n", f.display())).unwrap();
        }
        for f in &files {
            date(f, at);
            date(f.parent().unwrap(), at);
        }
        root
    }

    fn date(p: &Path, at: SystemTime) {
        std::fs::File::open(p).unwrap().set_modified(at).unwrap();
    }

    /// Rewrite `p` in place with one byte more, dated `at`.
    fn edit(p: &Path, at: SystemTime) {
        let mut text = std::fs::read_to_string(p).unwrap();
        text.push(' ');
        std::fs::write(p, text).unwrap();
        date(p, at);
    }

    /// What `build.rs` bakes, by the same walk and fold.
    fn bake(root: &Path) -> (String, usize) {
        let files = identity_files(root);
        (format!("{:016x}", fold_files(root, &files)), files.len())
    }

    #[test]
    fn an_unmodified_tree_is_fresh_without_a_read_and_its_fold_agrees() {
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
        let root = scratch("fresh", t0);
        let (hash, files) = bake(&root);
        let baked = Baked { hash: &hash, files };
        let built = t0 + Duration::from_secs(10);
        assert_eq!(stale_sources(&root, &baked, built), Staleness::Fresh, "nothing newer: no read");
        // Built before the files were: the fold is taken, and it agrees.
        let early = t0 - Duration::from_secs(10);
        assert_eq!(stale_sources(&root, &baked, early), Staleness::Unchanged);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// An edit to a covered file the old hash never read (a
    /// `hale-types` source: it folded `codegen.rs`, `lotus_arena.c` and
    /// the stdlib seeds) is a stale binary; an edit to a file no
    /// identity covers is not: outside the covered directories it costs
    /// no read, and beside covered sources it costs a fold.
    #[test]
    fn an_edit_to_any_covered_source_is_stale_and_an_uncovered_one_is_not() {
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
        let built = t0 + Duration::from_secs(10);
        let later = t0 + Duration::from_secs(20);

        let root = scratch("covered", t0);
        let (hash, files) = bake(&root);
        let baked = Baked { hash: &hash, files };
        edit(&root.join("crates/hale-types/src/lib.rs"), later);
        assert_eq!(stale_sources(&root, &baked, built), Staleness::Stale);
        let _ = std::fs::remove_dir_all(&root);

        let root = scratch("uncovered", t0);
        let (hash, files) = bake(&root);
        let baked = Baked { hash: &hash, files };
        edit(&root.join("crates/hale-lsp/src/lib.rs"), later);
        assert_eq!(stale_sources(&root, &baked, built), Staleness::Fresh);
        let src = root.join("crates/hale-types/src");
        std::fs::write(src.join("NOTES.md"), "not a source\n").unwrap();
        date(&src.join("NOTES.md"), later);
        date(&src, later);
        assert_eq!(stale_sources(&root, &baked, built), Staleness::Unchanged);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A covered file touched and left as it was is folded and warns
    /// nothing; a removed one, and a renamed one whose time is the
    /// build's, are stale through their directory and the count.
    #[test]
    fn a_touch_folds_quietly_and_a_removal_or_rename_is_stale() {
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
        let built = t0 + Duration::from_secs(10);
        let later = t0 + Duration::from_secs(20);

        let root = scratch("touch", t0);
        let (hash, files) = bake(&root);
        let baked = Baked { hash: &hash, files };
        date(&root.join("crates/hale-model/src/lib.rs"), later);
        assert_eq!(stale_sources(&root, &baked, built), Staleness::Unchanged);
        let _ = std::fs::remove_dir_all(&root);

        let root = scratch("remove", t0);
        let (hash, files) = bake(&root);
        let baked = Baked { hash: &hash, files };
        std::fs::remove_file(root.join("crates/hale-codegen/runtime/lotus_arena.c")).unwrap();
        date(&root.join("crates/hale-codegen/runtime"), t0);
        assert_eq!(stale_sources(&root, &baked, built), Staleness::Stale, "the count");
        let _ = std::fs::remove_dir_all(&root);

        let root = scratch("rename", t0);
        let (hash, files) = bake(&root);
        let baked = Baked { hash: &hash, files };
        let src = root.join("crates/hale-frontend/src");
        std::fs::rename(src.join("lib.rs"), src.join("frontend.rs")).unwrap();
        date(&src.join("frontend.rs"), t0);
        date(&src, later);
        assert_eq!(stale_sources(&root, &baked, built), Staleness::Stale, "the directory");
        let _ = std::fs::remove_dir_all(&root);
    }
}

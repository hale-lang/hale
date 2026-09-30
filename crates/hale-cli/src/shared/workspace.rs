use std::fs;
use std::path::{Path, PathBuf};

// The workspace helpers live in `hale-frontend` (F.40 phase 2.1a);
// `seed_inputs` stays here because it reads a library's `[ffi]`
// manifest through the CLI's `pkg`.
pub(crate) use hale_frontend::workspace::*;

use super::frontend::{collect_ap_files, collect_target_files};
use super::imports::resolve_import;

/// The transitive input set of a seed: its own `.hl` files and those
/// of every imported directory; the `hale.toml` of every such
/// directory when it has one, and every C source that manifest
/// declares under `[ffi] csrc`, exactly as the native build adds them
/// (a declared source is an input whether or not it exists yet — the
/// build reads it and fails on it). Canonical, sorted, each once. A
/// file that does not parse is still an input; only its imports go
/// unfollowed.
pub(crate) fn seed_inputs(target: &Path) -> Result<Vec<PathBuf>, String> {
    let workspace_root = find_workspace_root(target);
    let mut seen: std::collections::BTreeSet<PathBuf> = std::collections::BTreeSet::new();
    let mut dirs: std::collections::BTreeSet<PathBuf> = std::collections::BTreeSet::new();
    let mut queue: Vec<PathBuf> = collect_ap_files(target)?;
    while let Some(f) = queue.pop() {
        let canon = f.canonicalize().unwrap_or_else(|_| f.clone());
        if !seen.insert(canon.clone()) {
            continue;
        }
        let dir = canon.parent().unwrap_or_else(|| Path::new(".")).to_path_buf();
        dirs.insert(dir.clone());
        let Ok(src) = fs::read_to_string(&canon) else { continue };
        let Ok(program) = hale_syntax::parse_source(&src) else { continue };
        for imp in &program.imports {
            if let Some(t) = resolve_import(&dir, workspace_root.as_deref(), &imp.path) {
                if let Ok(files) = collect_target_files(&t) {
                    queue.extend(files);
                }
            }
        }
    }
    for dir in dirs {
        let manifest = dir.join("hale.toml");
        if !manifest.is_file() {
            continue;
        }
        seen.insert(manifest.canonicalize().unwrap_or(manifest));
        if let Ok(Some(ffi)) = crate::pkg::read_lib_ffi(&dir) {
            for csrc in ffi.csrc {
                let p = dir.join(csrc);
                seen.insert(p.canonicalize().unwrap_or(p));
            }
        }
    }
    Ok(seen.into_iter().collect())
}

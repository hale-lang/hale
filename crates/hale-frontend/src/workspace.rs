use std::fs;
use std::path::{Path, PathBuf};

use super::source::SourceProvider;

/// Walk upward from `start` looking for a `Cargo.toml`; the first
/// directory containing one is treated as the workspace root.
/// Used for the workspace-root fallback in import resolution.
/// Returns `None` if no Cargo.toml is found before hitting the
/// filesystem root (standalone-shipped binaries hit this — they
/// can still use entry-relative imports, just not the
/// workspace-fallback path).
/// Walk up from `start` looking for a workspace anchor. Hale
/// repos are anchored by `hale.toml`; hale's own dev tree
/// is also a cargo workspace, so `Cargo.toml` works as a fallback
/// anchor for compiler-side development. The first one found
/// wins. The result is the directory containing the anchor.
///
/// 2026-05-22: anchor used as the basis for path-based mangling
/// (`AliasScopes::name_library`). Two consumers in the same workspace
/// importing the same lib produce identical mangled names
/// because they compute the lib's path relative to the same
/// root.
/// `find_workspace_root` for sibling modules.
pub fn find_workspace_root_pub(start: &Path) -> Option<PathBuf> {
    find_workspace_root(start)
}

pub fn find_workspace_root(start: &Path) -> Option<PathBuf> {
    // Canonicalize first so the walk-up traverses real ancestor
    // directories regardless of whether `start` came in relative
    // (e.g., `hale build apps/a/main.hl` from the repo root).
    // Without this, relative paths walk `apps/a/main.hl` →
    // `apps/a` → `apps` → "" and never reach the actual
    // workspace root containing the hale.toml.
    let canon = start.canonicalize().unwrap_or_else(|_| start.to_path_buf());
    let mut cur = if canon.is_file() {
        canon.parent()?.to_path_buf()
    } else {
        canon
    };
    loop {
        if cur.join("hale.toml").is_file() || cur.join("Cargo.toml").is_file()
        {
            return Some(cur);
        }
        cur = match cur.parent() {
            Some(p) => p.to_path_buf(),
            None => return None,
        };
    }
}

/// GH #763: `<dir>/main.hl` and `<dir>` name the SAME library.
///
/// A seed is a directory (F.19): every `.hl` file in it shares one
/// declaration namespace, and `main.hl` is that seed's entry file,
/// not a library of its own. So `import "../lib/main"` names the
/// seed `../lib`, exactly as `import "../lib"` does, and this
/// collapses the first spelling onto the second before anything
/// downstream derives an identity from the target.
///
/// Without the collapse the two spellings produced two library
/// identities — two `lib_key`s in `resolve_imports`, two library names
/// in `name_library`, two sets of mangled symbols. The `visited`
/// set is global across the build, so whichever spelling resolved
/// second found every file already parsed, registered no rename rows
/// under its own key, and its `alias::Name` references died at
/// codegen as `unknown qualified name` while the other alias worked.
///
/// Any OTHER single file stays its own library: rule 1 of the
/// resolution order (spec `projects.md`) is a real single-file
/// library, and only the `main.hl` entry spelling is a second name
/// for the directory around it.
///
/// `import "main"` from inside the directory itself is left alone —
/// collapsing it would make a seed import itself.
pub fn seed_dir_for_entry_file(
    single: &Path,
    importer_dir: &Path,
    src: &dyn SourceProvider,
) -> Option<PathBuf> {
    if single.file_name().and_then(|s| s.to_str()) != Some("main.hl") {
        return None;
    }
    let dir = single.parent()?;
    if !src.is_dir(dir) {
        return None;
    }
    let canon_dir = dir.canonicalize().ok()?;
    let canon_importer = importer_dir.canonicalize().ok()?;
    if canon_dir == canon_importer {
        return None;
    }
    Some(dir.to_path_buf())
}

pub fn sanitize_identifier(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    // Collapse runs of underscores so deeply-nested paths don't
    // produce eye-watering `___` sequences in symbol names.
    let mut collapsed = String::with_capacity(out.len());
    let mut prev_underscore = false;
    for ch in out.chars() {
        if ch == '_' {
            if !prev_underscore {
                collapsed.push('_');
            }
            prev_underscore = true;
        } else {
            collapsed.push(ch);
            prev_underscore = false;
        }
    }
    collapsed.trim_matches('_').to_string()
}

/// The name a top-level decl introduces. Mirrors the mangler's
/// private `top_decl_name`; used only for the "a path head may name
/// this seed's own declaration" exemption below, where a miss costs
/// an exemption and never a false finding.
pub fn top_decl_ident(d: &hale_syntax::ast::TopDecl) -> Option<&str> {
    use hale_syntax::ast::TopDecl as T;
    match d {
        T::Locus(l) => Some(&l.name.name),
        T::Perspective(p) => Some(&p.name.name),
        T::Type(t) => Some(&t.name.name),
        T::Const(c) => Some(&c.name.name),
        T::Fn(f) => Some(&f.name.name),
        T::Interface(i) => Some(&i.name.name),
        T::Topic(t) => Some(&t.name.name),
        T::RingLayout(r) => Some(&r.name.name),
        T::Target(t) => Some(&t.name.name),
        T::Group(g) => Some(&g.name.name),
        T::Role(r) => Some(&r.name.name),
        T::Module(_) | T::Claims(_) | T::Constitution(_) => None,
    }
}

/// Every SEED under `root`: a directory holding one or more `.hl`
/// files directly. `check` operates on one seed and does not recurse
/// — correctly, since a directory is one compilation unit — so a
/// repository with many seeds needs something to enumerate them.
///
/// Skips `vendor` and dot-directories, matching `hale fmt`'s walk,
/// plus `target`. A seed you do not own is not yours to gate.
pub fn collect_seeds(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(root) else { return };
    let mut has_hl = false;
    let mut subdirs: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let p = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if p.is_dir() {
            if name == "vendor" || name == "target" || name.starts_with('.')
            {
                continue;
            }
            subdirs.push(p);
        } else if name.ends_with(".hl") {
            has_hl = true;
        }
    }
    if has_hl {
        out.push(root.to_path_buf());
    }
    subdirs.sort();
    for d in subdirs {
        collect_seeds(&d, out);
    }
}

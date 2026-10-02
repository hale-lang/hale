use std::fs;
use std::path::{Component, Path, PathBuf};

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
/// ([`library_basis`]). Two consumers in the same workspace
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

/// The path a library is named by: `lib` relative to the entry's
/// workspace root when it lies inside it, else relative to the entry
/// seed's directory (`..` segments included). Both anchors move with
/// the tree, so a workspace moved or cloned as a whole keeps every
/// name. All three paths are canonical. Two libraries of one load have
/// the same basis only when they are the same library: a basis inside
/// the workspace never starts with `..`, and one outside it always
/// does, since the entry's directory is inside the workspace.
pub fn library_basis(lib: &Path, workspace_root: Option<&Path>, entry_dir: &Path) -> PathBuf {
    if let Some(rel) = workspace_root.and_then(|root| lib.strip_prefix(root).ok()) {
        return rel.to_path_buf();
    }
    let ours: Vec<Component> = lib.components().collect();
    let base: Vec<Component> = entry_dir.components().collect();
    let common = ours.iter().zip(&base).take_while(|(a, b)| a == b).count();
    // No common root (another drive): the absolute path, which no
    // relative basis can spell, since it starts with a root.
    if common == 0 {
        return lib.to_path_buf();
    }
    let mut rel = PathBuf::new();
    for _ in common..base.len() {
        rel.push("..");
    }
    for c in &ours[common..] {
        rel.push(c.as_os_str());
    }
    rel
}

/// The name a library's symbols are mangled under, encoded from its
/// [`library_basis`] so that two different bases never share a name —
/// the encoding is injective, so there is no collision to resolve.
///
/// Segments are joined by `__`. Within a segment, an ASCII letter or
/// digit is kept, and so is a `_` that is not a segment's first byte
/// and is followed by a letter or digit other than `x`; every other
/// byte is `_xHH` (two lowercase hex digits). A file library's `.hl`
/// is dropped, and a directory library's name ends in one `_` (a file
/// library's name never does), so `util.hl` and `util/` stay apart.
/// Reading back, `_` is a separator before `_`, an escape before `x`,
/// the directory mark at the end, and itself otherwise.
///
/// The result is a valid C / LLVM symbol component. A single-file
/// library beside the entry, `util.hl`, is named `util`, as it always
/// was.
pub fn library_id(basis: &Path, directory: bool) -> String {
    use std::fmt::Write;
    let mut segments: Vec<&[u8]> = basis
        .components()
        .filter_map(|c| match c {
            Component::CurDir => None,
            Component::RootDir => Some(&b""[..]),
            other => Some(other.as_os_str().as_encoded_bytes()),
        })
        .collect();
    if !directory {
        if let Some(last) = segments.last_mut() {
            let file: &[u8] = last;
            *last = file.strip_suffix(b".hl").unwrap_or(file);
        }
    }
    let mut out = String::new();
    for (i, seg) in segments.iter().enumerate() {
        if i > 0 {
            out.push_str("__");
        }
        for (j, &b) in seg.iter().enumerate() {
            let plain_underscore = b == b'_'
                && j > 0
                && seg
                    .get(j + 1)
                    .is_some_and(|n| n.is_ascii_alphanumeric() && *n != b'x');
            if b.is_ascii_alphanumeric() || plain_underscore {
                out.push(b as char);
            } else {
                let _ = write!(out, "_x{b:02x}");
            }
        }
    }
    if directory {
        out.push('_');
    }
    out
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn a_library_beside_the_entry_keeps_its_name() {
        assert_eq!(library_id(Path::new("util.hl"), false), "util");
        assert_eq!(library_id(Path::new("my_util.hl"), false), "my_util");
        assert_eq!(library_id(Path::new("one/util.hl"), false), "one__util");
        assert_eq!(library_id(Path::new("toy"), true), "toy_");
        assert_eq!(library_id(Path::new("shared/messages"), true), "shared__messages_");
        assert_eq!(library_id(Path::new("../one/util.hl"), false), "_x2e_x2e__one__util");
        assert_eq!(library_id(Path::new("lib-util"), true), "lib_x2dutil_");
        assert_eq!(library_id(Path::new("lib_util"), true), "lib_util_");
        assert_eq!(library_id(Path::new("a_x"), true), "a_x5fx_");
    }

    #[test]
    fn the_basis_is_relative_to_the_workspace_else_to_the_entry() {
        let root = Path::new("/w");
        let entry = Path::new("/w/apps/a");
        assert_eq!(
            library_basis(Path::new("/w/shared/m"), Some(root), entry),
            PathBuf::from("shared/m")
        );
        assert_eq!(
            library_basis(Path::new("/elsewhere/u.hl"), Some(root), entry),
            PathBuf::from("../../../elsewhere/u.hl")
        );
        assert_eq!(
            library_basis(Path::new("/t/one/util.hl"), None, Path::new("/t/app")),
            PathBuf::from("../one/util.hl")
        );
        assert_eq!(
            library_basis(Path::new("/t/app/util.hl"), None, Path::new("/t/app")),
            PathBuf::from("util.hl")
        );
    }

    /// Every path over a small alphabet that holds each escape's
    /// trigger (`_`, `x`, a separator, a dot, a byte that is not an
    /// identifier character), as a file library and as a directory
    /// library: no two distinct libraries share a name.
    #[test]
    fn the_encoding_is_injective() {
        const ALPHABET: &[u8] = b"ax_-./";
        let mut seen: HashMap<String, (Vec<String>, bool)> = HashMap::new();
        let mut strings: Vec<Vec<u8>> = vec![Vec::new()];
        for _ in 0..6 {
            let next: Vec<Vec<u8>> = strings
                .iter()
                .flat_map(|s| {
                    ALPHABET.iter().map(move |&c| {
                        let mut t = s.clone();
                        t.push(c);
                        t
                    })
                })
                .collect();
            for s in &next {
                let text = String::from_utf8(s.clone()).unwrap();
                if text.starts_with('/') || text.ends_with('/') || text.contains("//") {
                    continue;
                }
                for directory in [false, true] {
                    let path = if directory {
                        PathBuf::from(&text)
                    } else {
                        PathBuf::from(format!("{text}.hl"))
                    };
                    // The library a path names: its components, so
                    // `a/./b` and `a/b` are one library.
                    let identity: Vec<String> = path
                        .components()
                        .filter(|c| !matches!(c, Component::CurDir))
                        .map(|c| c.as_os_str().to_string_lossy().into_owned())
                        .collect();
                    if identity.is_empty() {
                        continue;
                    }
                    let id = library_id(&path, directory);
                    assert!(
                        id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'),
                        "{id} is not a symbol component"
                    );
                    let mine = (identity, directory);
                    if let Some(prev) = seen.get(&id) {
                        assert_eq!(prev, &mine, "{id} names two libraries");
                    } else {
                        seen.insert(id, mine);
                    }
                }
            }
            strings = next;
        }
        assert!(seen.len() > 50_000, "the alphabet was enumerated: {}", seen.len());
    }
}

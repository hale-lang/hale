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
/// Each segment is one [`name_component`] (the encoding the mangler
/// also gives a file stem and a declaration name), and segments are
/// joined by `__`. A file library's `.hl` is dropped, and a directory
/// library's name ends in one `_` (a file library's name never does),
/// so `util.hl` and `util/` stay apart. A root directory is the
/// segment `/`, which no other segment can be. Since no encoded
/// segment starts or ends with `_` or holds `__`, a run of two
/// underscores is a separator and a trailing one the directory mark.
///
/// The result is a valid C / LLVM symbol component. A single-file
/// library beside the entry, `util.hl`, is named `util`.
///
/// [`name_component`]: hale_types::mangle::name_component
pub fn library_id(basis: &Path, directory: bool) -> String {
    let mut segments: Vec<&[u8]> = basis
        .components()
        .filter_map(|c| match c {
            Component::CurDir => None,
            Component::RootDir => Some(&b"/"[..]),
            other => Some(other.as_os_str().as_encoded_bytes()),
        })
        .collect();
    if !directory {
        if let Some(last) = segments.last_mut() {
            let file: &[u8] = last;
            *last = file.strip_suffix(b".hl").unwrap_or(file);
        }
    }
    let mut out = segments
        .iter()
        .map(|seg| hale_types::mangle::name_component(seg))
        .collect::<Vec<_>>()
        .join("__");
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
        T::Unit(u) => Some(&u.name.name),
        T::Api(a) => Some(&a.name.name),
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
    use hale_types::mangle::mangled;
    use std::collections::HashMap;

    #[test]
    fn a_library_beside_the_entry_keeps_its_name() {
        assert_eq!(library_id(Path::new("util.hl"), false), "util");
        assert_eq!(library_id(Path::new("my_util.hl"), false), "my_util");
        assert_eq!(library_id(Path::new("one/util.hl"), false), "one__util");
        assert_eq!(library_id(Path::new("toy"), true), "toy_");
        assert_eq!(library_id(Path::new("shared/messages"), true), "shared__messages_");
        assert_eq!(library_id(Path::new("../one/util.hl"), false), "x2e_x2e__one__util");
        assert_eq!(library_id(Path::new("lib-util"), true), "lib_x2dutil_");
        assert_eq!(library_id(Path::new("lib_util"), true), "lib_util_");
        assert_eq!(library_id(Path::new("a_x"), true), "a_x5fx_");
        assert_eq!(library_id(Path::new("xml.hl"), false), "x78ml");
        assert_eq!(library_id(Path::new("b__util.hl"), false), "b_x5f_util");
    }

    /// The full name of `name` in file `stem` of the library at `basis`.
    fn full(basis: &str, directory: bool, stem: &str, name: &str) -> String {
        mangled(&library_id(Path::new(basis), directory), stem, name)
    }

    /// The declaration names themselves, not only the library names.
    #[test]
    fn the_full_name_keeps_library_stem_and_declaration_apart() {
        // The re-review's pair: library `a` holding `b__util.hl`, and
        // library `a/b` holding `util.hl`. Their names differ (`a_`,
        // `a__b_`), and so do their declarations'.
        assert_eq!(full("a", true, "b__util", "Tag"), "__lib_a___b_x5f_util__Tag");
        assert_eq!(full("a/b", true, "util", "Tag"), "__lib_a__b___util__Tag");
        // A declaration name holding separator-like text, beside the
        // stem it could have been read as part of.
        assert_eq!(full("p", true, "q", "s__who"), "__lib_p___q__s_x5f_who");
        assert_eq!(full("p", true, "q_s", "_who"), "__lib_p___q_s__x5fwho");
        assert_eq!(full("p", true, "q", "get_x_"), "__lib_p___q__get_x5fx_x5f");
        // An escaped first byte against a literal `x` after the
        // directory mark.
        assert_eq!(full("d", true, "x2d", "T"), "__lib_d___x782d__T");
        assert_eq!(full("d.hl", false, "-", "T"), "__lib_d__x2d__T");
        // A library outside the workspace, and a stem holding `__`.
        assert_eq!(full("../one/util.hl", false, "util", "who"), "__lib_x2e_x2e__one__util__util__who");
        assert_eq!(full("my__util.hl", false, "my__util", "who"), "__lib_my_x5f_util__my_x5f_util__who");
        // The compatibility boundary: a single-file library beside the
        // entry whose file name is letters and digits keeps every
        // declaration's name that neither starts with `_` nor holds
        // `__`, as written.
        assert_eq!(full("util.hl", false, "util", "who"), "__lib_util_util_who");
        assert_eq!(full("util.hl", false, "util", "make_err"), "__lib_util_util_make_err");
        assert_eq!(full("util.hl", false, "util", "get_x_"), "__lib_util_util_get_x_");
        assert_eq!(full("xml.hl", false, "xml", "parse"), "__lib_xml_xml_parse");
        assert_eq!(full("util.hl", false, "util", "_who"), "__lib_util__util__x5fwho");
        assert_eq!(full("my_util.hl", false, "my_util", "who"), "__lib_my_util__my_util__who");
        // Read back: a name, its stem and its declaration give the one
        // library it was mangled in.
        let lib = |have: &str, stem: &str, name: &str| hale_types::mangle::mangled_library(have, stem, name);
        assert_eq!(lib("__lib_a___b_x5f_util__Tag", "b__util", "Tag").as_deref(), Some("a_"));
        assert_eq!(lib("__lib_a__b___util__Tag", "util", "Tag").as_deref(), Some("a__b_"));
        assert_eq!(lib("__lib_a__b___util__Tag", "b__util", "Tag"), None);
        assert_eq!(lib("__lib_util_util_who", "util", "who").as_deref(), Some("util"));
        assert_eq!(lib("__lib_util__util__who", "util", "who"), None);
    }

    /// Every (library, stem, declaration) tuple whose parts total at
    /// most six bytes, over alphabets that hold each escape's trigger
    /// (`_`, `x`, the hex digits `2` and `d` that spell `-` as `x2d`, a
    /// byte that is not an identifier character, and for paths `.` and
    /// `/`), each library as a file and as a directory, the directory
    /// with the empty path among them: no two distinct
    /// tuples share a full name, and every name is a symbol.
    #[test]
    fn the_full_name_is_injective() {
        const PATH: &[u8] = b"dx2_-./";
        const PART: &[u8] = b"dx2_-";
        const TOTAL: usize = 6;
        fn strings(alphabet: &[u8], max: usize) -> Vec<Vec<u8>> {
            let mut all = Vec::new();
            let mut layer: Vec<Vec<u8>> = vec![Vec::new()];
            for _ in 0..max {
                layer = layer
                    .iter()
                    .flat_map(|s| {
                        alphabet.iter().map(move |&c| {
                            let mut t = s.clone();
                            t.push(c);
                            t
                        })
                    })
                    .collect();
                all.extend(layer.iter().cloned());
            }
            all
        }
        // (identity, directory, written length, library id)
        let mut libs: Vec<(Vec<String>, bool, usize, String)> = Vec::new();
        for s in strings(PATH, TOTAL - 2) {
            let text = String::from_utf8(s).unwrap();
            if text.starts_with('/') || text.ends_with('/') || text.contains("//") {
                continue;
            }
            for directory in [false, true] {
                let path = if directory { PathBuf::from(&text) } else { PathBuf::from(format!("{text}.hl")) };
                let identity: Vec<String> = path
                    .components()
                    .filter(|c| !matches!(c, Component::CurDir))
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect();
                // `.` and `./.` are the entry's own directory, the
                // library with the empty path: its name is `_`.
                let id = library_id(&path, directory);
                libs.push((identity, directory, text.len(), id));
            }
        }
        let parts: Vec<String> =
            strings(PART, TOTAL - 2).into_iter().map(|s| String::from_utf8(s).unwrap()).collect();
        // (full name, library, stem, declaration), sorted so that equal
        // names sit side by side.
        let mut names: Vec<(String, u32, u32, u32)> = Vec::new();
        for (li, (_, _, len, id)) in libs.iter().enumerate() {
            for (si, stem) in parts.iter().enumerate() {
                if len + stem.len() >= TOTAL {
                    continue;
                }
                for (ni, name) in parts.iter().enumerate() {
                    if len + stem.len() + name.len() > TOTAL {
                        continue;
                    }
                    names.push((mangled(id, stem, name), li as u32, si as u32, ni as u32));
                }
            }
        }
        names.sort_unstable();
        let tuple = |&(_, l, s, n): &(String, u32, u32, u32)| {
            let (identity, directory, _, _) = &libs[l as usize];
            (identity.clone(), *directory, parts[s as usize].clone(), parts[n as usize].clone())
        };
        for pair in names.windows(2) {
            let (a, b) = (&pair[0], &pair[1]);
            assert!(
                a.0.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_'),
                "{} is not a symbol",
                a.0
            );
            if a.0 == b.0 {
                assert_eq!(tuple(a), tuple(b), "{} names two declarations", a.0);
            }
        }
        assert!(libs.iter().any(|(_, _, _, id)| id == "_"), "the empty path was enumerated");
        assert!(names.len() > 500_000, "the tuples were enumerated: {}", names.len());
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

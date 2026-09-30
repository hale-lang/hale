//! Where the loaders read source text from (F.40 phase 2.1a).
//!
//! Every read the loaders make — a file's text, whether a path is a
//! file or a directory, the `.hl` files a directory holds — goes
//! through a [`SourceProvider`]. The CLI reads the disk ([`Disk`]); the
//! LSP reads the editor's unsaved buffers over the disk ([`Overlay`]),
//! so a buffer the user has not saved is what gets checked, and a file
//! that exists only as a buffer is still a member of its seed.
//!
//! Paths stay the disk's: a provider answers what a path HOLDS, never
//! what it is called, so identity (`canonicalize`, the source map's
//! workspace-relative paths) is untouched by the choice of provider.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The source the loaders read.
pub trait SourceProvider {
    /// The text of the file at `path`.
    fn read(&self, path: &Path) -> io::Result<String>;
    /// `path` is a file this provider can read.
    fn exists(&self, path: &Path) -> bool;
    /// `path` is a directory: a seed, or an import target.
    fn is_dir(&self, path: &Path) -> bool;
    /// The `.hl` files directly in `dir`, sorted — the files of a seed,
    /// in merge order.
    fn hl_files(&self, dir: &Path) -> io::Result<Vec<PathBuf>>;
    /// What this provider reads over the disk, as a digest: zero for
    /// the disk itself. Part of a snapshot's key
    /// ([`crate::snapshot::SnapshotKey`]), so an edited buffer is a
    /// different snapshot.
    fn overlay_digest(&self) -> u64 {
        0
    }
}

/// The file system, as it is.
pub struct Disk;

impl SourceProvider for Disk {
    fn read(&self, path: &Path) -> io::Result<String> {
        fs::read_to_string(path)
    }

    fn exists(&self, path: &Path) -> bool {
        path.is_file()
    }

    fn is_dir(&self, path: &Path) -> bool {
        path.is_dir()
    }

    fn hl_files(&self, dir: &Path) -> io::Result<Vec<PathBuf>> {
        let mut out = Vec::new();
        for entry in fs::read_dir(dir)? {
            let p = entry?.path();
            if is_hl(&p) {
                out.push(p);
            }
        }
        out.sort();
        Ok(out)
    }
}

/// The editor's buffers over the disk: a path with a buffer reads as
/// the buffer, every other path reads as the disk does. A buffer whose
/// file is not on disk yet is still listed in its directory.
///
/// A buffer is keyed by the path its URI decodes to, which is usually
/// canonical while a loader may reach the same file through a relative
/// or `..` spelling; a lookup tries the path as given, then canonical,
/// then its canonical directory joined with its name (a file not on
/// disk has no canonical form of its own).
pub struct Overlay<'a> {
    overlays: &'a BTreeMap<PathBuf, String>,
}

impl<'a> Overlay<'a> {
    pub fn new(overlays: &'a BTreeMap<PathBuf, String>) -> Self {
        Self { overlays }
    }

    fn buffer(&self, path: &Path) -> Option<&'a String> {
        if let Some(s) = self.overlays.get(path) {
            return Some(s);
        }
        if let Ok(canon) = path.canonicalize() {
            if let Some(s) = self.overlays.get(&canon) {
                return Some(s);
            }
        }
        let name = path.file_name()?;
        let dir = canonical_dir(path.parent()?)?;
        self.overlays.get(&dir.join(name))
    }

    /// The buffers whose file sits directly in `dir`, by file name.
    fn buffers_in(&self, dir: &Path) -> Vec<&'a std::ffi::OsStr> {
        let want = canonical_dir(dir);
        self.overlays
            .keys()
            .filter(|k| is_hl(k))
            .filter(|k| {
                let parent = k.parent().unwrap_or(Path::new(""));
                parent == dir || (want.is_some() && canonical_dir(parent) == want)
            })
            .filter_map(|k| k.file_name())
            .collect()
    }
}

impl SourceProvider for Overlay<'_> {
    fn read(&self, path: &Path) -> io::Result<String> {
        match self.buffer(path) {
            Some(s) => Ok(s.clone()),
            None => Disk.read(path),
        }
    }

    fn exists(&self, path: &Path) -> bool {
        self.buffer(path).is_some() || Disk.exists(path)
    }

    fn is_dir(&self, path: &Path) -> bool {
        Disk.is_dir(path) || !self.buffers_in(path).is_empty()
    }

    fn hl_files(&self, dir: &Path) -> io::Result<Vec<PathBuf>> {
        let buffered = self.buffers_in(dir);
        let mut out = match Disk.hl_files(dir) {
            Ok(files) => files,
            Err(_) if !buffered.is_empty() => Vec::new(),
            Err(e) => return Err(e),
        };
        for name in buffered {
            if !out.iter().any(|p| p.file_name() == Some(name)) {
                out.push(dir.join(name));
            }
        }
        out.sort();
        Ok(out)
    }

    fn overlay_digest(&self) -> u64 {
        let mut d = crate::snapshot::Digest::new();
        d.count(self.overlays.len());
        for (path, text) in self.overlays {
            d.field(path.as_os_str().as_encoded_bytes());
            d.field(text.as_bytes());
        }
        d.finish()
    }
}

fn is_hl(p: &Path) -> bool {
    p.extension().and_then(|s| s.to_str()) == Some("hl")
}

fn canonical_dir(dir: &Path) -> Option<PathBuf> {
    let dir = if dir.as_os_str().is_empty() { Path::new(".") } else { dir };
    dir.canonicalize().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch directory of this test's own: the pid keeps two runs
    /// apart, the name keeps two tests of one run apart.
    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir()
            .join(format!("hale-frontend-source-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d.canonicalize().unwrap()
    }

    #[test]
    fn a_buffer_with_no_file_is_a_member_of_its_directory() {
        let d = scratch("member");
        fs::write(d.join("a.hl"), "fn a() {}\n").unwrap();
        fs::write(d.join("notes.txt"), "not a seed file\n").unwrap();
        let mut buffers = BTreeMap::new();
        buffers.insert(d.join("b.hl"), "fn b() {}\n".to_string());
        let src = Overlay::new(&buffers);
        assert_eq!(src.hl_files(&d).unwrap(), vec![d.join("a.hl"), d.join("b.hl")]);
        assert_eq!(Disk.hl_files(&d).unwrap(), vec![d.join("a.hl")]);
        assert!(src.exists(&d.join("b.hl")) && !Disk.exists(&d.join("b.hl")));
        assert_eq!(src.read(&d.join("b.hl")).unwrap(), "fn b() {}\n");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_buffer_wins_over_the_disk_however_the_path_is_spelled() {
        let d = scratch("wins");
        fs::create_dir_all(d.join("sub")).unwrap();
        fs::write(d.join("a.hl"), "on disk\n").unwrap();
        let mut buffers = BTreeMap::new();
        buffers.insert(d.join("a.hl"), "in the editor\n".to_string());
        let src = Overlay::new(&buffers);
        let roundabout = d.join("sub").join("..").join("a.hl");
        assert_eq!(src.read(&roundabout).unwrap(), "in the editor\n");
        assert_eq!(Disk.read(&roundabout).unwrap(), "on disk\n");
        // A path with no buffer reads through to the disk.
        fs::write(d.join("c.hl"), "only on disk\n").unwrap();
        assert_eq!(src.read(&d.join("c.hl")).unwrap(), "only on disk\n");
        let _ = fs::remove_dir_all(&d);
    }
}

//! A DNA test's own directory under the temp dir, as `dna::scratch_root`
//! is a fixture's: whatever a run before left at its path goes first,
//! and when the value drops — the test passing, or unwinding from a
//! failed assertion — every process named under it stops, every compose
//! project run from under it comes down ([`reap::compose_down`]), and the
//! directory goes. `HALE_DNA_KEEP_SCRATCH=1` keeps the directory and its
//! compose projects, for looking at what the test built.
//!
//! A test that runs a `hale dna` verb able to start a body or bring up
//! compose makes its directory here, never by hand:
//! `dna_fixture_roots_are_scratch.rs` refuses one that does.

#[allow(dead_code)]
#[path = "reap.rs"]
mod reap;

use std::ffi::OsStr;
use std::ops::Deref;
use std::path::{Path, PathBuf};

pub struct Scratch {
    // the directory the value stands for
    path: PathBuf,
    // the test's own directory, which goes on drop: `path` itself, or the
    // root a [`Scratch::nested`] directory sits under
    root: PathBuf,
}

impl Scratch {
    /// `<temp dir>/<name>`, cleared of what an earlier run left there. It
    /// is not created: a verb that makes its own directory is handed a
    /// path that does not exist yet.
    pub fn new(name: impl AsRef<str>) -> Scratch {
        let path = std::env::temp_dir().join(name.as_ref());
        let _ = std::fs::remove_dir_all(&path);
        Scratch { path: path.clone(), root: path }
    }

    /// `sub` under this directory, standing for it: the whole root still
    /// goes when the value drops.
    #[allow(dead_code)]
    pub fn nested(self, sub: &str) -> Scratch {
        let mut this = self;
        let path = this.path.join(sub);
        let root = std::mem::take(&mut this.root);
        std::mem::forget(this);
        Scratch { path, root }
    }
}

impl Deref for Scratch {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.path
    }
}

impl AsRef<Path> for Scratch {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

impl AsRef<OsStr> for Scratch {
    fn as_ref(&self) -> &OsStr {
        self.path.as_os_str()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        reap::reap(&self.root);
        if !std::env::var("HALE_DNA_KEEP_SCRATCH").is_ok_and(|v| !v.is_empty() && v != "0") {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
}

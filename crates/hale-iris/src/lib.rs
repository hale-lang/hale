//! `hale iris` — the iris observer, embedded (GH #527 B3).
//!
//! Iris is a Hale program (`iris/consumer/fuse-hl`), a C attach shim,
//! and a vanilla-JS renderer. Rather than rewrite it in Rust, the
//! `hale` binary carries its sources the way `hale-stdlib` carries the
//! stdlib: `include_str!` at compile time, materialized into a
//! toolchain-hashed cache directory on first use, built with the
//! compiler's own codegen, and exec'd. A compiler upgrade changes the
//! hash and rebuilds; a second launch is exec-only.
//!
//! This crate knows nothing about the CLI. It owns the file set, the
//! hash, the cache path and the materialization; `hale-cli` drives
//! the build and the exec.

use std::io;
use std::path::{Path, PathBuf};

/// One embedded file: its path relative to the materialized root, and
/// its bytes as text.
pub struct EmbeddedFile {
    pub path: &'static str,
    pub content: &'static str,
}

/// The consumer (`fuse-hl`), its FFI attach shim and the C attach
/// library it wraps, the protocol header (the HALE copy — the iris
/// tree's `emitter/protocol.h` only forwards to it, and a forward
/// cannot resolve from a cache directory), the web renderer, and the
/// artifact inspector. Paths keep the REPO's layout (`iris/…` beside
/// `dna/…`, see [`ALL_FILES`]) so every relative `#include`,
/// `hale.toml` `csrc` entry and `import "../../../dna/core"` resolves
/// unchanged.
pub const FILES: &[EmbeddedFile] = &[
    EmbeddedFile { path: "iris/consumer/fuse-hl/main.hl", content: include_str!("../../../iris/consumer/fuse-hl/main.hl") },
    EmbeddedFile { path: "iris/consumer/fuse-hl/attach/attach.hl", content: include_str!("../../../iris/consumer/fuse-hl/attach/attach.hl") },
    EmbeddedFile { path: "iris/consumer/fuse-hl/attach/glue.c", content: include_str!("../../../iris/consumer/fuse-hl/attach/glue.c") },
    EmbeddedFile { path: "iris/consumer/fuse-hl/attach/hale.toml", content: include_str!("../../../iris/consumer/fuse-hl/attach/hale.toml") },
    EmbeddedFile { path: "iris/consumer/obs_attach.c", content: include_str!("../../../iris/consumer/obs_attach.c") },
    EmbeddedFile { path: "iris/consumer/obs_attach.h", content: include_str!("../../../iris/consumer/obs_attach.h") },
    EmbeddedFile { path: "iris/emitter/protocol.h", content: include_str!("../../hale-codegen/runtime/obs_protocol.h") },
    EmbeddedFile { path: "iris/render/web/index.html", content: include_str!("../../../iris/render/web/index.html") },
    EmbeddedFile { path: "iris/render/web/app.js", content: include_str!("../../../iris/render/web/app.js") },
    EmbeddedFile { path: "iris/inspect/main.hl", content: include_str!("../../../iris/inspect/main.hl") },
];

/// Everything `hale iris` materializes: the iris tree plus the DNA
/// core the observer imports for its typed control topics (GH #527
/// B6) — one declaration of `dna.review.verdict`, bound by the
/// organism and published by the observer.
pub fn all_files() -> impl Iterator<Item = (&'static str, &'static str)> {
    FILES
        .iter()
        .map(|f| (f.path, f.content))
        .chain(hale_dna::FILES.iter().map(|f| (f.path, f.content)))
        .chain(std::iter::once((hale_dna::MEMBRANE_CLIENT.path, hale_dna::MEMBRANE_CLIENT.content)))
}

/// The seed `hale build` compiles for `hale iris` (relative to the root).
pub const FUSE_SEED: &str = "iris/consumer/fuse-hl";
/// The binary that build produces (`<dir>/<dirname>`).
pub const FUSE_BIN: &str = "iris/consumer/fuse-hl/fuse-hl";
/// The seed for `hale iris inspect`.
pub const INSPECT_SEED: &str = "iris/inspect";
pub const INSPECT_BIN: &str = "iris/inspect/inspect";
/// The web root fuse-hl serves.
pub const WEBROOT: &str = "iris/render/web";

/// FNV-1a over the compiler version and every embedded byte. Two
/// toolchains with the same iris sources and the same compiler share
/// a cache; anything else rebuilds.
pub fn toolchain_hash() -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |bytes: &[u8]| {
        for b in bytes {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x100_0000_01b3);
        }
    };
    eat(env!("CARGO_PKG_VERSION").as_bytes());
    for (path, content) in all_files() {
        eat(path.as_bytes());
        eat(&[0]);
        eat(content.as_bytes());
        eat(&[0]);
    }
    h
}

/// `$XDG_CACHE_HOME/hale/iris/<hash>` (or `~/.cache/hale/iris/<hash>`),
/// the same root the runtime objects and the LSP's stdlib copy use.
pub fn cache_dir() -> Option<PathBuf> {
    let root = match std::env::var_os("XDG_CACHE_HOME") {
        Some(x) if !x.is_empty() => PathBuf::from(x),
        _ => PathBuf::from(std::env::var_os("HOME")?).join(".cache"),
    };
    Some(root.join("hale").join("iris").join(format!("{:016x}", toolchain_hash())))
}

/// Write every embedded file under `root` unless it is already there
/// with the same content. Idempotent; a partial earlier run is
/// completed, never trusted.
pub fn materialize_into(root: &Path) -> io::Result<()> {
    for (path, content) in all_files() {
        let p = root.join(path);
        if let Ok(existing) = std::fs::read_to_string(&p) {
            if existing == content {
                continue;
            }
        }
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = p.with_extension("tmp-materialize");
        std::fs::write(&tmp, content)?;
        std::fs::rename(&tmp, &p)?;
    }
    Ok(())
}

/// Materialize into the cache directory and return it.
pub fn materialize() -> io::Result<PathBuf> {
    let dir = cache_dir().ok_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, "no cache directory: neither XDG_CACHE_HOME nor HOME is set")
    })?;
    materialize_into(&dir)?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_embedded_file_is_nonempty_and_unique() {
        let mut seen = std::collections::BTreeSet::new();
        for (path, content) in all_files() {
            assert!(!content.is_empty(), "{} is empty", path);
            assert!(seen.insert(path), "{} listed twice", path);
        }
        assert!(all_files().any(|(p, _)| p == "dna/core/topics.hl"), "the control topics ride along");
    }

    #[test]
    fn the_protocol_header_is_the_hale_copy_not_the_forwarder() {
        let h = FILES.iter().find(|f| f.path == "iris/emitter/protocol.h").unwrap();
        assert!(h.content.contains("#define OBS_PROTO_MINOR"), "must be the real header");
        assert!(!h.content.contains("crates/hale-codegen/runtime/obs_protocol.h"), "must not be the forwarder");
    }

    #[test]
    fn materialize_is_idempotent() {
        let dir = std::env::temp_dir().join(format!("hale-iris-test-{}", std::process::id()));
        materialize_into(&dir).unwrap();
        let a = std::fs::metadata(dir.join(FUSE_SEED).join("main.hl")).unwrap().modified().unwrap();
        materialize_into(&dir).unwrap();
        let b = std::fs::metadata(dir.join(FUSE_SEED).join("main.hl")).unwrap().modified().unwrap();
        assert_eq!(a, b, "unchanged files are not rewritten");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hash_is_stable_within_a_build() {
        assert_eq!(toolchain_hash(), toolchain_hash());
    }
}

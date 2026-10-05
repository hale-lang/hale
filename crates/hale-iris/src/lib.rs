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
/// `dna/…`, see [`all_files`]) so every relative `#include`,
/// `hale.toml` `csrc` entry and `import` resolves unchanged.
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
    EmbeddedFile { path: "iris/process_identity/main.hl", content: include_str!("../../../iris/process_identity/main.hl") },
];

/// Everything the toolchain cache holds: the iris tree, and beside it
/// the DNA sources `hale dna` builds from the same cache under the
/// same build lock — the host and the surface,
/// with the core, operations and pond's driver they import. Iris itself imports none of the DNA tree.
pub fn all_files() -> impl Iterator<Item = (&'static str, &'static str)> {
    FILES
        .iter()
        .map(|f| (f.path, f.content))
        .chain(hale_dna::FILES.iter().map(|f| (f.path, f.content)))
        .chain(hale_dna::POND_FILES.iter().map(|f| (f.path, f.content)))
        .chain(hale_dna::LEGS_FILES.iter().map(|f| (f.path, f.content)))
        .chain(std::iter::once((hale_dna::UI_MAIN.path, hale_dna::UI_MAIN.content)))
        .chain(std::iter::once((hale_dna::UI_HTML.path, hale_dna::UI_HTML.content)))
        .chain(hale_dna::HOST_FILES.iter().map(|f| (f.path, f.content)))
        .chain(hale_dna::OPERATION_FILES.iter().map(|f| (f.path, f.content)))
        .chain(hale_dna::ORGANIZATION_FILES.iter().map(|f| (f.path, f.content)))
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

/// FNV-1a over what the cached binaries were built from: the crate
/// version, the compiler's own source (`HALE_COMPILER_SRC_HASH`, from
/// `build.rs`: the identity-covered crates, the stdlib's seeds among
/// them, and the manifests), the options the cache's `hale build` builds
/// with, and every embedded iris and DNA byte. A change to any of them
/// rebuilds; two toolchains that agree on all of them share a cache.
///
/// `build_options` is the execution identity's options fingerprint of
/// the build the cache runs (F.40 phase 4, I5): that build is a `hale
/// build` subprocess, which inherits this process's environment and
/// turns its knobs (`HALE_DEV`, a sanitizer, LTO, ...) into options, so
/// a host built under one set is never served to another. The CLI
/// computes it, by the function the execution identity uses, from the
/// environment the subprocess inherits.
pub fn toolchain_hash(build_options: &str) -> u64 {
    toolchain_hash_of(
        env!("CARGO_PKG_VERSION"),
        env!("HALE_COMPILER_SRC_HASH"),
        build_options,
        all_files(),
    )
}

/// The stdlib is folded once, as files of the compiler's source
/// selection (`HALE_COMPILER_SRC_HASH` covers `crates/hale-stdlib/hl`);
/// it was folded a second time here from `hale_stdlib::AP_FILES` until
/// F.40 phase 4, I5.
fn toolchain_hash_of<'b>(
    version: &str,
    compiler: &str,
    build_options: &str,
    files: impl Iterator<Item = (&'b str, &'b str)>,
) -> u64 {
    let mut h = hale_graph::identity::Fnv64::new();
    let mut eat = |bytes: &[u8]| h.write(bytes);
    eat(version.as_bytes());
    eat(&[0]);
    eat(compiler.as_bytes());
    eat(&[0]);
    eat(build_options.as_bytes());
    eat(&[0]);
    for (path, content) in files {
        eat(path.as_bytes());
        eat(&[0]);
        eat(content.as_bytes());
        eat(&[0]);
    }
    h.finish()
}

/// `$XDG_CACHE_HOME/hale/iris/<hash>` (or `~/.cache/hale/iris/<hash>`),
/// the same root the runtime objects and the LSP's stdlib copy use; the
/// hash is [`toolchain_hash`] of `build_options`.
pub fn cache_dir(build_options: &str) -> Option<PathBuf> {
    let root = match std::env::var_os("XDG_CACHE_HOME") {
        Some(x) if !x.is_empty() => PathBuf::from(x),
        _ => PathBuf::from(std::env::var_os("HOME")?).join(".cache"),
    };
    Some(root.join("hale").join("iris").join(format!("{:016x}", toolchain_hash(build_options))))
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
        // a tmp name per writer: two commands materializing one cache at
        // once renamed the same tmp, and the loser's rename found nothing
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let tmp = p.with_extension(format!("tmp-materialize-{}-{}", std::process::id(), SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
        std::fs::write(&tmp, content)?;
        if let Err(e) = std::fs::rename(&tmp, &p) {
            let _ = std::fs::remove_file(&tmp);
            // the other writer won with the same bytes: fine
            if std::fs::read_to_string(&p).map(|now| now == content).unwrap_or(false) {
                continue;
            }
            return Err(e);
        }
    }
    Ok(())
}

/// Materialize into the cache directory for `build_options` (see
/// [`toolchain_hash`]) and return it.
pub fn materialize(build_options: &str) -> io::Result<PathBuf> {
    let dir = cache_dir(build_options).ok_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, "no cache directory: neither XDG_CACHE_HOME nor HOME is set")
    })?;
    materialize_into(&dir)?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Several commands materialize one cache at once on a fresh
    /// machine (a host, two nodes and a CLI in one test): every writer
    /// must succeed, and the files must be whole.
    #[test]
    fn concurrent_materialization_never_fails_a_writer() {
        let dir = std::env::temp_dir().join(format!("hale-iris-materialize-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let d = dir.clone();
                std::thread::spawn(move || materialize_into(&d))
            })
            .collect();
        for h in handles {
            h.join().unwrap().expect("a writer lost the race and failed");
        }
        for (path, content) in all_files() {
            assert_eq!(std::fs::read_to_string(dir.join(path)).unwrap(), content, "{path} is whole");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The default build's options fingerprint (`build_env.rs`'s pinned
    /// string).
    const DEFAULT_OPTIONS: &str = "target=Native;cpu=Native;dev=false;debug=false";

    fn hash_with(compiler: &str, build_options: &str) -> u64 {
        toolchain_hash_of("0.0.0", compiler, build_options, all_files())
    }

    fn workspace_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    /// The key's compiler half, `HALE_COMPILER_SRC_HASH`, as `build.rs`
    /// folds it over a tree whose stdlib seeds are `stdlib`.
    fn key_over(tag: &str, stdlib: &[(&str, &str)]) -> u64 {
        use hale_graph::identity::{fold_files, identity_files};
        let root = std::env::temp_dir().join(format!("hale-iris-key-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let hl = root.join("crates/hale-stdlib/hl");
        std::fs::create_dir_all(&hl).unwrap();
        for (name, content) in stdlib {
            std::fs::write(hl.join(name), content).unwrap();
        }
        let compiler = format!("{:016x}", fold_files(&root, &identity_files(&root)));
        let _ = std::fs::remove_dir_all(&root);
        hash_with(&compiler, DEFAULT_OPTIONS)
    }

    /// A stdlib edit that leaves the version alone must still move the
    /// key: the cached host was compiled against the old stdlib. The
    /// stdlib is folded once, as files of the compiler's selection (F.40
    /// phase 4, I5), so the edit moves the key through that half: every
    /// seed the binary embeds is a file the selection holds, with the
    /// same bytes.
    #[test]
    fn a_stdlib_byte_change_moves_the_key() {
        let root = workspace_root();
        let selected = hale_graph::identity::identity_files(&root);
        for (name, content) in hale_stdlib::AP_FILES {
            let file = root.join("crates/hale-stdlib/hl").join(name);
            assert!(selected.contains(&file), "the stdlib seed {name} is not in the compiler's selection");
            assert_eq!(&std::fs::read_to_string(&file).unwrap(), content, "{name} is the embedded seed");
        }
        let base: Vec<(&str, &str)> = hale_stdlib::AP_FILES.to_vec();
        let same = key_over("same", &base);
        assert_eq!(same, key_over("again", &base), "the key is deterministic");
        let edited = format!("{} ", base[0].1);
        let mut changed = base.clone();
        changed[0].1 = &edited;
        assert_ne!(same, key_over("changed", &changed), "one appended byte in one stdlib file");
        let mut renamed = base.clone();
        renamed[0].0 = "renamed.hl";
        assert_ne!(same, key_over("renamed", &renamed), "a stdlib file's name is part of the key");
        assert_ne!(same, key_over("dropped", &base[1..]), "a stdlib file dropped");
    }

    /// The compiler's own source moves the key too (codegen, the
    /// runtime C): the cached host is a binary it produced.
    #[test]
    fn a_compiler_change_moves_the_key() {
        assert_ne!(hash_with("0000000000000001", DEFAULT_OPTIONS), hash_with("0000000000000002", DEFAULT_OPTIONS));
        let built = env!("HALE_COMPILER_SRC_HASH");
        assert_eq!(built.len(), 16, "a 64-bit hex id from build.rs: {built}");
        assert!(built.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(
            toolchain_hash(DEFAULT_OPTIONS),
            toolchain_hash_of(env!("CARGO_PKG_VERSION"), built, DEFAULT_OPTIONS, all_files()),
            "the shipped key is composed of exactly these inputs"
        );
    }

    /// The options the cache's build inherits are part of the key (F.40
    /// phase 4, I5): a knob that moves the fingerprint moves the key, so
    /// a host built under a sanitizer or `HALE_DEV` is not served to a
    /// build without it; the same fingerprint gives the key it gave.
    #[test]
    fn a_build_knob_moves_the_key_and_the_same_options_keep_it() {
        let plain = toolchain_hash(DEFAULT_OPTIONS);
        assert_eq!(plain, toolchain_hash(DEFAULT_OPTIONS), "the unchanged environment keeps its key");
        let mut seen = std::collections::BTreeSet::from([plain]);
        for knob in [";asan", ";tsan", ";ubsan", ";lto=Thin"] {
            let options = format!("{DEFAULT_OPTIONS}{knob}");
            assert!(seen.insert(toolchain_hash(&options)), "{knob} leaves the key where another build has it");
        }
        let dev = DEFAULT_OPTIONS.replace("dev=false", "dev=true");
        assert!(seen.insert(toolchain_hash(&dev)), "HALE_DEV leaves the key where it was");
    }

    #[test]
    fn every_embedded_file_is_nonempty_and_unique() {
        let mut seen = std::collections::BTreeSet::new();
        for (path, content) in all_files() {
            assert!(!content.is_empty(), "{} is empty", path);
            assert!(seen.insert(path), "{} listed twice", path);
        }
        for seed in [hale_dna::HOST_SEED, hale_dna::UI_SEED, hale_dna::CORE_SEED] {
            let dir = format!("{seed}/");
            assert!(all_files().any(|(p, _)| p.starts_with(&dir)), "hale dna builds {seed} from this cache");
        }
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
        assert_eq!(toolchain_hash(DEFAULT_OPTIONS), toolchain_hash(DEFAULT_OPTIONS));
    }
}

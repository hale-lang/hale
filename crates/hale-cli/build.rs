//! Phase 2i — stale-CLI hash check.
//!
//! Hashes the codegen + runtime source files this CLI binary will
//! be linked against, emits the hash and the codegen-crate path
//! as `cargo:rustc-env` variables. At runtime, `main.rs` recomputes
//! the hash from the on-disk source files and warns when they
//! disagree (the user edited codegen / runtime / stdlib source
//! after building the CLI binary, so the binary's bundled
//! `include_str!` snapshots are stale relative to what the
//! workspace now shows).
//!
//! Resolves `apps/log-router/FRICTION.md` 2026-05-10
//! stale-cli-silent-drops-subscribers: agent ran
//! `cargo test -p hale-codegen` (which rebuilds codegen but
//! leaves the existing `target/debug/hale` binary linked against
//! the previous codegen.rlib), then invoked
//! `target/debug/hale build`, which emitted binaries against
//! the older lowering and silently dropped user-defined bus
//! subscribers. With the hash check in place, the same sequence
//! now prints a one-line warning pointing the agent at
//! `cargo build -p hale-cli`.

use std::collections::hash_map::DefaultHasher;
use std::env;
use std::fs;
use std::hash::Hasher;
use std::fmt::Write as _;
use std::path::PathBuf;

/// `hale mcp` docs-search: embed spec/*.md into the binary (864 KB
/// in a 66 MB binary) so an installed hale grounds language rules
/// with no sibling checkout. Generates OUT_DIR/spec_embed.rs with
/// a (name, contents) table.
fn embed_spec() {
    let manifest = PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR").expect("manifest dir"),
    );
    let spec_dir = manifest.join("../../spec");
    println!("cargo:rerun-if-changed={}", spec_dir.display());
    let mut entries: Vec<PathBuf> = fs::read_dir(&spec_dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "md"))
                .collect()
        })
        .unwrap_or_default();
    entries.sort();
    let mut out = String::from(
        "pub static SPEC_FILES: &[(&str, &str)] = &[\n",
    );
    for p in &entries {
        println!("cargo:rerun-if-changed={}", p.display());
        let name = p.file_name().unwrap().to_string_lossy();
        writeln!(
            out,
            "    ({:?}, include_str!({:?})),",
            name,
            p.canonicalize().unwrap().display().to_string()
        )
        .unwrap();
    }
    out.push_str("];\n");
    let dest = PathBuf::from(std::env::var("OUT_DIR").unwrap())
        .join("spec_embed.rs");
    fs::write(dest, out).unwrap();
}

fn main() {
    embed_spec();
    let manifest_dir = env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR set by cargo");
    // crates/hale-cli/ -> crates/ -> <repo-root>/
    let workspace_root = PathBuf::from(&manifest_dir)
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf());
    let codegen_dir = match workspace_root.as_ref() {
        Some(root) => root.join("crates").join("hale-codegen"),
        None => {
            // Manifest dir is not under the workspace shape we
            // expect; emit empty env vars so the runtime check
            // skips itself.
            println!("cargo:rustc-env=HALE_CODEGEN_SRC_HASH=");
            println!("cargo:rustc-env=HALE_CODEGEN_DIR=");
            return;
        }
    };

    // Files we hash. codegen.rs is the IR-emit; lotus_arena.c is
    // the C runtime bundled via include_str!; everything under
    // stdlib/ is the Hale stdlib seed merged into every
    // compiled program. Drift in any of these silently changes
    // what `hale build` emits.
    let mut paths: Vec<PathBuf> = vec![
        codegen_dir.join("src").join("codegen.rs"),
        codegen_dir.join("runtime").join("lotus_arena.c"),
    ];

    let stdlib_dir = codegen_dir.join("runtime").join("stdlib");
    if let Ok(entries) = fs::read_dir(&stdlib_dir) {
        let mut stdlib_files: Vec<PathBuf> = entries
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .and_then(|s| s.to_str())
                    == Some("hl")
            })
            .map(|e| e.path())
            .collect();
        // Deterministic order across filesystems.
        stdlib_files.sort();
        paths.extend(stdlib_files);
    }

    let mut hasher = DefaultHasher::new();
    for path in &paths {
        // rerun-if-changed makes Cargo invalidate this build
        // script when any tracked file changes, so the hash
        // baked into the binary stays in sync with what cargo
        // last saw on disk. This is the second line of defence;
        // the runtime check is the first.
        println!("cargo:rerun-if-changed={}", path.display());
        if let Ok(bytes) = fs::read(path) {
            // Mix path-as-bytes into the hash so renames /
            // additions / deletions also change the digest.
            hasher.write(path.to_string_lossy().as_bytes());
            hasher.write(&[0u8]);
            hasher.write(&bytes);
        }
    }
    let hash = format!("{:016x}", hasher.finish());

    println!("cargo:rustc-env=HALE_CODEGEN_SRC_HASH={}", hash);
    println!(
        "cargo:rustc-env=HALE_CODEGEN_DIR={}",
        codegen_dir.display()
    );

    // macOS: LLVM 18+ links against zstd, but the homebrew
    // `llvm@18` formula ships its libs in
    // `/opt/homebrew/Cellar/llvm@18/.../lib` while libzstd lives
    // in `/opt/homebrew/lib` (Apple Silicon) or `/usr/local/lib`
    // (Intel). The default linker search path includes neither,
    // so users hit `ld: library 'zstd' not found` on first build.
    // We inject the standard homebrew library dirs into the link
    // search path; cargo accepts paths that don't exist on the
    // host without warning, so this is a no-op on Linux.
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        for path in [
            "/opt/homebrew/lib",
            "/opt/homebrew/opt/zstd/lib",
            "/opt/homebrew/opt/llvm@18/lib",
            "/usr/local/lib",
            "/usr/local/opt/zstd/lib",
            "/usr/local/opt/llvm@18/lib",
        ] {
            if std::path::Path::new(path).is_dir() {
                println!("cargo:rustc-link-search=native={}", path);
            }
        }
    }
}

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

/// Minimal SHA-256 (no deps; build-script only). FIPS 180-4.
fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b,
        0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01,
        0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7,
        0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc,
        0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152,
        0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147,
        0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819,
        0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08,
        0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f,
        0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
        0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f,
        0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];
    let mut msg = data.to_vec();
    let bitlen = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bitlen.to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 64];
        for (i, word) in w.iter_mut().take(16).enumerate() {
            *word = u32::from_be_bytes(
                chunk[i * 4..i * 4 + 4].try_into().unwrap(),
            );
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7)
                ^ w[i - 15].rotate_right(18)
                ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17)
                ^ w[i - 2].rotate_right(19)
                ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for i in 0..64 {
            let s1 = e.rotate_right(6)
                ^ e.rotate_right(11)
                ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2)
                ^ a.rotate_right(13)
                ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }
    let mut out = [0u8; 32];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

fn walk_sources(dir: &PathBuf, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    let mut items: Vec<PathBuf> =
        entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    items.sort();
    for p in items {
        if p.is_dir() {
            walk_sources(&p, out);
        } else if matches!(
            p.extension().and_then(|s| s.to_str()),
            Some("rs") | Some("c") | Some("h") | Some("hl")
        ) {
            out.push(p);
        }
    }
}

/// GH #296 review round 3: the RECORD/REPLAY toolchain identity —
/// a length-framed SHA-256 over every compiler-side source that
/// shapes what a build emits or how a recording is produced,
/// parsed, and served: the full hale-syntax / hale-types /
/// hale-codegen / hale-cli source trees (which cover the parser,
/// effect analysis, IR emit, EVERY runtime TU incl. lotus_obs.c,
/// the stdlib seeds, and the replay CLI), plus the rustc version
/// and the git commit when available. The 64-bit stale-CLI hash
/// keeps its separate, narrower job.
fn toolchain_digest(workspace_root: &PathBuf) {
    let mut buf: Vec<u8> = Vec::new();
    let frame = |b: &[u8], buf: &mut Vec<u8>| {
        buf.extend_from_slice(&(b.len() as u64).to_le_bytes());
        buf.extend_from_slice(b);
    };
    let rustc = std::process::Command::new(
        env::var("RUSTC").unwrap_or_else(|_| "rustc".into()),
    )
    .arg("--version")
    .output()
    .map(|o| o.stdout)
    .unwrap_or_default();
    frame(&rustc, &mut buf);
    let commit = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(workspace_root)
        .output()
        .map(|o| o.stdout)
        .unwrap_or_default();
    frame(&commit, &mut buf);
    let mut files: Vec<PathBuf> = Vec::new();
    for krate in ["hale-syntax", "hale-types", "hale-codegen", "hale-cli"] {
        let root = workspace_root.join("crates").join(krate);
        walk_sources(&root.join("src"), &mut files);
        walk_sources(&root.join("runtime"), &mut files);
    }
    files.sort();
    for f in &files {
        println!("cargo:rerun-if-changed={}", f.display());
        let rel = f
            .strip_prefix(workspace_root)
            .unwrap_or(f)
            .to_string_lossy()
            .into_owned();
        frame(rel.as_bytes(), &mut buf);
        frame(&fs::read(f).unwrap_or_default(), &mut buf);
    }
    let d = sha256(&buf);
    let hex: String = d.iter().map(|b| format!("{:02x}", b)).collect();
    println!("cargo:rustc-env=HALE_TOOLCHAIN_SHA256={}", hex);
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
            println!("cargo:rustc-env=HALE_TOOLCHAIN_SHA256=");
            return;
        }
    };
    if let Some(root) = workspace_root.as_ref() {
        toolchain_digest(root);
    }

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

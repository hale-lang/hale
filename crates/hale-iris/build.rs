//! The compiler's own build id, for `toolchain_hash()`.
//!
//! The toolchain cache holds host and observer binaries built BY this
//! compiler, so its key has to move when the compiler does. The version
//! and the embedded Hale sources were the whole key, which left a
//! same-version rebuild that changed only codegen, the runtime C or the
//! stdlib reusing a stale cached host. The stdlib is hashed in the
//! crate from `hale_stdlib::AP_FILES`; this script hashes the source
//! that CANNOT be embedded here without bloating the binary by the
//! size of the compiler: the front end, the type checker, codegen and
//! its runtime C, and the stdlib crate's own tables.

use std::path::{Path, PathBuf};

const DIRS: &[&str] = &[
    "crates/hale-syntax/src",
    "crates/hale-types/src",
    "crates/hale-codegen/src",
    "crates/hale-codegen/runtime",
    "crates/hale-stdlib/src",
];

fn files_under(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("hale-iris build.rs: cannot read {}: {e}", dir.display()))
        .flatten()
        .map(|e| e.path())
        .collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            files_under(&p, out);
        } else {
            out.push(p);
        }
    }
}

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("manifest dir")).join("../..");
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |bytes: &[u8]| {
        for b in bytes {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x100_0000_01b3);
        }
    };
    for dir in DIRS {
        let d = root.join(dir);
        // A file's content, and the listing of the directory, so a
        // file added or removed re-runs this script too.
        println!("cargo:rerun-if-changed={}", d.display());
        let mut files = Vec::new();
        files_under(&d, &mut files);
        for f in files {
            let rel = f.strip_prefix(&root).unwrap_or(&f).to_string_lossy().replace('\\', "/");
            eat(rel.as_bytes());
            eat(&[0]);
            eat(&std::fs::read(&f).unwrap_or_else(|e| panic!("hale-iris build.rs: cannot read {}: {e}", f.display())));
            eat(&[0]);
        }
    }
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rustc-env=HALE_COMPILER_SRC_HASH={h:016x}");
}

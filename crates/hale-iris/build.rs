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

use std::path::PathBuf;

/// The directories this script hashes are the identity-covered
/// crates' (`hale_graph::identity::COVERED_CRATES`, F.40 phase 0 step
/// 0.4), through the shared walk and fold. The stdlib's `.hl` seeds
/// also ride the key at run time through `hale_stdlib::AP_FILES`. A
/// model-shape or graph-core change busts a cached host like a
/// codegen change does.
fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("manifest dir")).join("../..");
    let dirs = hale_graph::identity::covered_dirs(&root, &[]);
    let mut files = Vec::new();
    for d in &dirs {
        // A file's content, and the listing of the directory, so a
        // file added or removed re-runs this script too.
        println!("cargo:rerun-if-changed={}", d.display());
        hale_graph::identity::walk_sources(d, &mut files);
    }
    let h = hale_graph::identity::fold_files(&root, &files);
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rustc-env=HALE_COMPILER_SRC_HASH={h:016x}");
}

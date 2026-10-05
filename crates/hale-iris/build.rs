//! The compiler's own build id, for `toolchain_hash()`.
//!
//! The toolchain cache holds host and observer binaries built BY this
//! compiler, so its key has to move when the compiler does. The version
//! and the embedded Hale sources were the whole key, which left a
//! same-version rebuild that changed only codegen, the runtime C or the
//! stdlib reusing a stale cached host. This script hashes the source
//! that CANNOT be embedded here without bloating the binary by the
//! size of the compiler: the front end, the type checker, codegen and
//! its runtime C, the stdlib crate's tables and its `.hl` seeds, the
//! CLI whose `build` verb the cache invokes, and the manifests that pin
//! their dependencies.

use std::path::PathBuf;

/// The files this script folds are `hale_graph::identity::identity_files`
/// (F.40 phase 0 step 0.4): the identity-covered crates' sources, the
/// CLI's among them, then the lock file and the ts-shim manifest —
/// the selection the replay identity frames too, through the shared
/// walk and fold. The stdlib's `.hl` seeds ride the key here, once
/// (F.40 phase 4, I5: they were folded a second time at run time from
/// `hale_stdlib::AP_FILES`). A model-shape, graph-core, stdlib, CLI or
/// dependency-version change busts a cached host like a codegen change
/// does.
fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("manifest dir")).join("../..");
    // Each covered directory, so a file added or removed re-runs this
    // script too, and each manifest file.
    for d in hale_graph::identity::covered_dirs(&root, &[]) {
        println!("cargo:rerun-if-changed={}", d.display());
    }
    for f in hale_graph::identity::manifest_files(&root) {
        println!("cargo:rerun-if-changed={}", f.display());
    }
    let files = hale_graph::identity::identity_files(&root);
    let h = hale_graph::identity::fold_files(&root, &files);
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rustc-env=HALE_COMPILER_SRC_HASH={h:016x}");
}

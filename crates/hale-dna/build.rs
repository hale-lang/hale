//! GH #726 — the embedded DNA source set's digest, computed at the
//! toolchain's build time.
//!
//! `hale dna new` / `init` / `upgrade` materialize `vendor/dna` from
//! the source this crate embeds, so an organism runs the core the
//! BINARY carries. Two binaries with the same `hale --version` can
//! carry different source, and a fixture that mutates `dna/core`
//! without rebuilding measures the old one — twice, in a review on
//! 2026-09-17, that was reported as coverage it did not have. The
//! digest emitted here is that source set's name: `hale --version`
//! and `hale dna status` print it, the scaffold records it, and a
//! fixture compares it with the working tree's
//! (`hale dna --embedded-digest --from-tree <dir>`).
//!
//! The algorithm is `src/digest.rs`, `include!`d rather than copied:
//! this script digests the on-disk `dna/` tree, the crate digests
//! the compiled-in set, and a unit test holds the two equal — which
//! is what makes a build's snapshot coherent (source edited while a
//! build runs, or a file added without updating the crate's lists,
//! makes them disagree and fails the build).

#[allow(dead_code)]
mod dna_digest {
    include!("src/digest.rs");
}

use std::path::PathBuf;

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("manifest dir"))
        .join("../..");
    // A file's content, and the listing of every directory (so a file
    // added or removed re-runs this script too).
    for (dir, _) in dna_digest::EMBEDDED_DIRS {
        println!("cargo:rerun-if-changed={}", root.join(dir).display());
    }
    for f in dna_digest::tree_files(&root) {
        println!("cargo:rerun-if-changed={}", f.display());
    }
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/digest.rs");
    let digest = dna_digest::digest_of_tree(&root)
        .unwrap_or_else(|e| panic!("the embedded DNA source set cannot be digested: {e}"));
    println!("cargo:rustc-env=HALE_DNA_EMBEDDED_DIGEST={digest}");
}

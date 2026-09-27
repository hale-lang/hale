//! GH #785: the stale-binary warning covers the DNA source set. The
//! binary embeds `dna/**` (GH #726) and `hale dna new` / `init` /
//! `upgrade` materialize that, so a `dna/core` edited after the last
//! build runs nowhere — and nothing said so on an ordinary `hale`
//! invocation. Now the same warning the codegen tree gets fires when
//! the tree's DNA digest differs from what the binary embeds.
//!
//! The check reads the workspace the binary was built from; the test
//! hands it a copy it may edit through `HALE_STALE_DNA_ROOT`.

use std::path::{Path, PathBuf};
use std::process::Command;

const WARNING: &str = "embeds an older dna/ source set";

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap().to_path_buf()
}

/// A copy of the embedded DNA directories, exactly as digested.
fn copy_dna(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("hale_stale_dna_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&root);
    let from = repo_root();
    for (dir, _) in hale_dna::EMBEDDED_DIRS {
        let src = from.join(dir);
        let dst = root.join(dir);
        std::fs::create_dir_all(&dst).unwrap();
        for entry in std::fs::read_dir(&src).unwrap().flatten() {
            let p = entry.path();
            if p.is_file() {
                std::fs::copy(&p, dst.join(p.file_name().unwrap())).unwrap();
            }
        }
    }
    root
}

fn hale_check(root: &Path) -> String {
    let program = root.join("probe.hl");
    std::fs::write(&program, "fn main() { println(1); }\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["check", &program.to_string_lossy()])
        .env_remove("HALE_SKIP_STALE_CHECK")
        .env("HALE_STALE_DNA_ROOT", root)
        .output()
        .expect("hale");
    format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
}

#[test]
fn a_tree_the_binary_embeds_raises_nothing_and_an_edited_one_warns() {
    let root = copy_dna("edit");
    assert_eq!(
        hale_dna::digest_of_tree(&root).unwrap(),
        hale_dna::EMBEDDED_DIGEST,
        "the copy is the set this binary embeds"
    );
    let quiet = hale_check(&root);
    assert!(!quiet.contains(WARNING), "an unchanged tree raises nothing: {quiet}");

    let core = root.join("dna").join("core");
    let edited = std::fs::read_dir(&core)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().and_then(|s| s.to_str()) == Some("hl"))
        .expect("a core file");
    let mut text = std::fs::read_to_string(&edited).unwrap();
    text.push_str("\n// edited after the build\n");
    std::fs::write(&edited, text).unwrap();
    let warned = hale_check(&root);
    assert!(warned.contains(WARNING), "an edited dna/core warns: {warned}");
    assert!(warned.contains("cargo build --release"), "and says how to rebuild: {warned}");
    let _ = std::fs::remove_dir_all(&root);
}

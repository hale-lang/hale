//! GH #726 — the embedded DNA source set has a name, and the CLI
//! surface that carries it.
//!
//! `hale dna new` / `init` / `upgrade` materialize `vendor/dna` from
//! the source embedded in the binary, so an organism runs the core
//! the BINARY carries; two builds of one version can carry different
//! source, and a mutation run against a binary that predates the
//! edit proves nothing. These hold the three places a reader or a
//! fixture finds out which source that is — and hold `--version`'s
//! FIRST line to the version alone, because the body-provisioning
//! script, the DNA fixtures and the benchmark harness read field 2
//! of it.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p
}

fn hale(args: &[&str]) -> (bool, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale")).args(args).output().expect("hale");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

#[test]
fn version_names_the_embedded_dna_on_a_second_line_and_keeps_the_first() {
    let (ok, stdout, err) = hale(&["--version"]);
    assert!(ok, "{err}");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2, "two lines, no more: {stdout:?}");
    assert_eq!(lines[0], format!("hale {}", env!("CARGO_PKG_VERSION")), "the first line is the version ALONE — `awk '{{print $2}}'` over it is a version, not a digest: {stdout:?}");
    let (_, digest, _) = hale(&["dna", "--embedded-digest"]);
    let digest = digest.trim().to_string();
    assert_eq!(lines[1], format!("embedded dna: {}", &digest[..16]), "the second line is the embedded source's digest: {stdout:?}");
    // `-V` and `version` are the same surface
    for spelling in ["-V", "version"] {
        let (ok, other, _) = hale(&[spelling]);
        assert!(ok);
        assert_eq!(other, stdout, "`hale {spelling}` prints what `--version` prints");
    }
}

#[test]
fn the_embedded_digest_is_this_working_trees_dna_source() {
    let (ok, stdout, err) = hale(&["dna", "--embedded-digest"]);
    assert!(ok, "{err}");
    let mine = stdout.trim().to_string();
    assert_eq!(mine.len(), 64, "one 64-hex digest and nothing else on stdout: {stdout:?}");
    assert!(stdout.ends_with('\n') && stdout.lines().count() == 1, "nothing else on stdout: {stdout:?}");

    // the binary under test was built from this checkout, so the
    // tree's digest is its own. This is the comparison a fixture
    // makes before it believes a mutation result; unequal means the
    // binary predates the edit.
    let root = repo_root();
    let (ok, tree, err) = hale(&["dna", "--embedded-digest", "--from-tree", &root.to_string_lossy()]);
    assert!(ok, "{err}");
    assert_eq!(tree.trim(), mine, "the digest of {} is the digest this build embeds", root.display());
    // …and it is the same digest for `--from-tree=<dir>`
    let (ok, eq_form, _) = hale(&["dna", "--embedded-digest", &format!("--from-tree={}", root.display())]);
    assert!(ok);
    assert_eq!(eq_form.trim(), mine);

    // every embedded file is in it: one changed byte in a copy of the
    // tree gives a different digest, one file fewer too
    let scratch = std::env::temp_dir().join(format!("hale_dna_digest_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    copy_tree(&root, &scratch);
    let (ok, same, err) = hale(&["dna", "--embedded-digest", "--from-tree", &scratch.to_string_lossy()]);
    assert!(ok, "{err}");
    assert_eq!(same.trim(), mine, "a copy of the tree is the same source");
    let target = scratch.join("dna/core/budget.hl");
    let mut text = std::fs::read_to_string(&target).unwrap();
    text.push_str("// one comment\n");
    std::fs::write(&target, &text).unwrap();
    let (_, edited, _) = hale(&["dna", "--embedded-digest", "--from-tree", &scratch.to_string_lossy()]);
    assert_ne!(edited.trim(), mine, "an edited core file is a different source set");
    std::fs::remove_file(scratch.join("dna/host/procs.hl")).unwrap();
    let (_, removed, _) = hale(&["dna", "--embedded-digest", "--from-tree", &scratch.to_string_lossy()]);
    assert!(removed.trim() != mine && removed.trim() != edited.trim(), "a missing host file is a different source set");

    // a directory that is not a checkout is refused by name, never
    // digested as a smaller set
    let (ok, out, err) = hale(&["dna", "--embedded-digest", "--from-tree", &scratch.join("dna/tests").to_string_lossy()]);
    assert!(!ok, "a directory without dna/core is refused: {out}");
    assert!(err.contains("dna/core"), "the refusal names what was missing: {err}");
    let (ok, _, err) = hale(&["dna", "--embedded-digest", "--from-tree"]);
    assert!(!ok && err.contains("name the checkout"), "{err}");
    let _ = std::fs::remove_dir_all(&scratch);
}

/// Copy only what the digest reads: the `dna/` directories of the
/// embedded set.
fn copy_tree(from: &Path, to: &Path) {
    for (dir, _) in hale_dna::EMBEDDED_DIRS {
        let src = from.join(dir);
        let dst = to.join(dir);
        std::fs::create_dir_all(&dst).unwrap();
        for e in std::fs::read_dir(&src).unwrap().flatten() {
            if e.path().is_file() {
                std::fs::copy(e.path(), dst.join(e.file_name())).unwrap();
            }
        }
    }
}

//! `hale dna head`: the head started from an installed toolchain, with no
//! checkout. The toolchain carries what the head is built from
//! (`hale_dna::HEAD_FILES`, held equal to the tree by hale-dna's own unit
//! test) and materializes it into its cache beside the host; the verb
//! builds the four seeds there and runs the materialized launcher.
//!
//! Building the seeds takes minutes, and CI's face jobs build them from
//! the checkout already, so the proof that the embedded tree is complete
//! is the cheap one: every seed `hale check`s clean in a materialized
//! tree that holds nothing the toolchain did not carry. A seed that
//! imports a directory no list embeds fails here.

#[path = "support/scratch.rs"]
mod scratch;
use std::path::Path;
use std::process::Command;

fn scratch(tag: &str) -> scratch::Scratch {
    let d = scratch::Scratch::new(format!("hale_dna_head_{tag}_{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn check(dir: &Path) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale")).arg("check").arg(dir).output().expect("invoke hale check");
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

#[test]
fn the_materialized_head_checks_without_a_checkout() {
    let root = scratch("tree");
    hale_iris::materialize_into(&root).expect("materialize the toolchain's tree");
    let launcher = root.join(hale_dna::HEAD_LAUNCHER);
    assert!(launcher.is_file(), "the launcher is materialized at {}", launcher.display());
    for f in hale_dna::HEAD_FILES.iter().filter(|f| f.path.starts_with("dna/face/web/")) {
        assert!(root.join(f.path).is_file(), "{} is served from the materialized tree", f.path);
    }
    let mut failed = Vec::new();
    for s in [&hale_dna::HEAD_API, &hale_dna::HEAD_SERVICE, &hale_dna::HEAD_OIDC, &hale_dna::HEAD_REFLEXES] {
        let (ok, out) = check(&root.join(s.seed));
        if !ok {
            failed.push(format!("--- {}\n{out}", s.seed));
        }
    }
    assert!(failed.is_empty(), "the head's seeds do not check in the toolchain's materialized tree (an import the embedded lists do not carry?):\n{}", failed.join("\n"));
    // and the check is a proof of the closure: a materialized tree missing
    // one directory a seed imports fails it
    std::fs::remove_file(root.join("dna/oidc/stub.hl")).unwrap();
    let (ok, out) = check(&root.join(hale_dna::HEAD_OIDC.seed));
    assert!(!ok, "a seed whose import is not materialized must not check:\n{out}");
}

#[test]
fn head_help_prints_its_usage() {
    let out = Command::new(env!("CARGO_BIN_EXE_hale")).args(["dna", "head", "--help"]).output().expect("invoke hale");
    assert!(out.status.success(), "--help exits 0");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.starts_with("usage: hale dna head [project]"), "{text}");
    for flag in ["--port", "--api-port", "--oidc-port", "--commands-port", "--source-drafts", "dna/face/start.sh", "hale dna dev"] {
        assert!(text.contains(flag), "the usage names {flag}:\n{text}");
    }
    // a flag the verb does not take is refused before anything is built
    let out = Command::new(env!("CARGO_BIN_EXE_hale")).args(["dna", "head", "--api", "/bin/true"]).output().expect("invoke hale");
    assert_eq!(out.status.code(), Some(2), "{}", String::from_utf8_lossy(&out.stderr));
    // and the verb is on `hale dna`'s own usage
    let out = Command::new(env!("CARGO_BIN_EXE_hale")).args(["dna", "--help"]).output().expect("invoke hale");
    assert!(String::from_utf8_lossy(&out.stderr).contains("hale dna head [project]"));
}

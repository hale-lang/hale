//! GH #1417, R8a: the clients committed in this repository are what their
//! surfaces generate (`scripts/api-clients.sh --check`), and a client whose
//! surface moved is refused with the drift named.

use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn hale(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_hale")).args(args).env("HALE_SKIP_STALE_CHECK", "1").output().expect("run hale")
}

/// Every committed client is current: the script regenerates each one in
/// memory and compares, so a change to the rows, the generator or the
/// preamble that was not followed by `scripts/api-clients.sh` fails here.
#[test]
fn every_committed_client_is_current() {
    let out = Command::new("bash")
        .arg(root().join("scripts/api-clients.sh"))
        .arg("--check")
        .current_dir(root())
        .env("HALE_BIN", env!("CARGO_BIN_EXE_hale"))
        .output()
        .expect("run scripts/api-clients.sh");
    assert!(
        out.status.success(),
        "a committed client is stale; run scripts/api-clients.sh:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A client generated against a surface that has since moved is refused,
/// and the refusal says which digest it was made against.
#[test]
fn a_client_of_a_moved_surface_is_refused() {
    let dir = std::env::temp_dir().join(format!("hale_api_client_drift_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let program = dir.join("program.hl");
    let src = std::fs::read_to_string(root().join("tests/api-contract/program.hl")).unwrap();
    std::fs::write(&program, &src).unwrap();
    let client = dir.join("client.hl");
    let (p, c) = (program.to_str().unwrap(), client.to_str().unwrap());
    let made = hale(&["api", "client", "--surface", "Public", "--lang", "hale", "--out", c, p]);
    assert!(made.status.success(), "{}", String::from_utf8_lossy(&made.stderr));
    let text = std::fs::read_to_string(&client).unwrap();
    assert!(text.contains("const SURFACE_DIGEST: String = \"fnv1a64:a8930d6e7998e986\";"), "the client names the digest in a constant");
    assert!(text.contains("(fnv1a64:a8930d6e7998e986)"), "and in a comment");
    let ok = hale(&["api", "client", "--surface", "Public", "--lang", "hale", "--check", c, p]);
    assert!(ok.status.success(), "{}", String::from_utf8_lossy(&ok.stderr));
    // a row moves: the surface's digest moves with it
    std::fs::write(&program, src.replace("rpc Orders::cancel requires: [trader];", "rpc Orders::cancel requires: [operator];")).unwrap();
    let stale = hale(&["api", "client", "--surface", "Public", "--lang", "hale", "--check", c, p]);
    assert_eq!(stale.status.code(), Some(1), "{}", String::from_utf8_lossy(&stale.stderr));
    let err = String::from_utf8_lossy(&stale.stderr);
    assert!(err.contains("made against Public fnv1a64:a8930d6e7998e986"), "names the digest it was made against: {err}");
    assert!(err.contains("the surface is now fnv1a64:"), "and the digest now: {err}");
    // a hand edit that leaves the digest alone is drift too
    std::fs::write(&program, &src).unwrap();
    std::fs::write(&client, text.replace("fn orders_place", "fn orders_place_by_hand")).unwrap();
    let edited = hale(&["api", "client", "--surface", "Public", "--lang", "hale", "--check", c, p]);
    assert_eq!(edited.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&edited.stderr).contains("is not what it generates"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Two runs, and two checkouts, generate the same bytes.
#[test]
fn a_client_is_the_same_bytes_on_two_runs_and_two_checkouts() {
    let dir = std::env::temp_dir().join(format!("hale_api_client_det_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let elsewhere = dir.join("a/deeper/checkout");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let copy = elsewhere.join("program.hl");
    std::fs::copy(root().join("tests/api-contract/program.hl"), &copy).unwrap();
    let original = root().join("tests/api-contract/program.hl");
    let run = |program: &Path, lang: &str| {
        let out = hale(&["api", "client", "--surface", "Admin", "--lang", lang, program.to_str().unwrap()]);
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        out.stdout
    };
    for lang in ["hale", "ts"] {
        let (a, b, c) = (run(&original, lang), run(&original, lang), run(&copy, lang));
        assert!(!a.is_empty());
        assert_eq!(a, b, "{lang}: two runs");
        assert_eq!(a, c, "{lang}: two checkouts");
        assert!(!String::from_utf8_lossy(&a).contains("hale_api_client_det"), "{lang}: no path of the checkout");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

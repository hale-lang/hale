//! `std::crypto::Sha256`'s update loop inside a long-lived locus passes
//! `hale verify` with no `@unbounded` acknowledgement: `update` absorbs
//! into the locus's fixed-size context and allocates nothing per call
//! (`std::crypto::__sha256_update` is on the non-allocating list), so the
//! loop accumulates nothing the memory-bound survey has to name.

use std::path::PathBuf;
use std::process::Command;

fn app() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.extend(["tests", "fixtures", "sha256-stream", "app.hl"]);
    p
}

#[test]
fn update_loop_verifies_without_acknowledgement() {
    let src = std::fs::read_to_string(app()).expect("read the fixture");
    assert!(!src.contains("@unbounded"), "the fixture acknowledges nothing");
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("verify")
        .arg(app())
        .output()
        .expect("invoke hale verify");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "hale verify refused the update loop:\n{stdout}\n{stderr}"
    );
    assert!(
        !stderr.contains("unbounded allocation"),
        "the survey named the update loop:\n{stderr}"
    );
}

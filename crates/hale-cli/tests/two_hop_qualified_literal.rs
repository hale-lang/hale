//! WS3.4 (G34) — qualified-name struct/locus *literal* in an
//! intermediate seed of a two-hop import chain.
//!
//! `app → lib-b → lib-c`. The existing `three_hop_import` test
//! covers qualified *types* (`u::Box` as a field/param/return) and
//! qualified *fn calls* (`u::make_box(...)`) inside the middle lib.
//! It does NOT cover a qualified struct/locus *literal*
//! (`c::Box { ... }`, `c::Holder { ... }`) in expression or return
//! position inside the middle lib — the pond/jobs §11 / `_util`
//! "G34" shape (`db::Db { path: ... }`). This locks that in.
//!
//! Verified through both `hale build <dir>` and `hale run <dir>`
//! (the latter only resolves imports as of WS3.3).

use std::path::PathBuf;
use std::process::Command;

fn hale_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hale"))
}

fn fixtures_dir() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests");
    p.push("fixtures");
    p
}

fn assert_outputs(stdout: &str) {
    // Qualified struct literal through the two-hop chain.
    assert!(
        stdout.contains("answer=42 [v1]"),
        "missing wrapped struct-literal output: {:?}",
        stdout
    );
    // Qualified struct literal in return position.
    assert!(
        stdout.contains("direct=7"),
        "missing return-position struct-literal output: {:?}",
        stdout
    );
    // Qualified locus literal + method call.
    assert!(
        stdout.contains("locus-ok"),
        "missing qualified-locus-literal output: {:?}",
        stdout
    );
}

#[test]
fn two_hop_qualified_literal_builds_and_runs() {
    let app_dir = fixtures_dir().join("g34-app");
    let built_bin = app_dir.join("g34-app");
    let _ = std::fs::remove_file(&built_bin);

    let out = Command::new(hale_bin())
        .arg("build")
        .arg(&app_dir)
        .output()
        .expect("invoke hale build");
    assert!(
        out.status.success(),
        "hale build failed (two-hop qualified literal regressed): \
         stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(built_bin.exists(), "expected binary at {:?}", built_bin);

    let run_out = Command::new(&built_bin).output().expect("run g34-app");
    assert!(
        run_out.status.success(),
        "g34-app exit {:?}: stderr={}",
        run_out.status,
        String::from_utf8_lossy(&run_out.stderr)
    );
    assert_outputs(&String::from_utf8_lossy(&run_out.stdout));
    let _ = std::fs::remove_file(&built_bin);
}

#[test]
fn two_hop_qualified_literal_via_run_dir() {
    let app_dir = fixtures_dir().join("g34-app");
    let out = Command::new(hale_bin())
        .arg("run")
        .arg(&app_dir)
        .output()
        .expect("invoke hale run <dir>");
    assert!(
        out.status.success(),
        "hale run <dir> failed for two-hop qualified literal: \
         stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    assert_outputs(&String::from_utf8_lossy(&out.stdout));
}

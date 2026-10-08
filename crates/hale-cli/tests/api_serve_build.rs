//! GH #1417 (R2a): a build refuses what this compiler does not serve yet.
//! The R0 witness serves `Public` over `http::Rpc` and `Admin` over
//! `unix::Rpc`; `hale check` admits it (R1's description and the
//! serve-site laws read it as written), and `hale build` says what it
//! cannot serve yet, instead of dropping the sites (R2b: `unix::Rpc` is
//! served; R3: `http::Rpc` is). A hub binding is refused (R5).

use std::path::PathBuf;
use std::process::Command;

fn witness() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p.push("tests/api-contract/program.hl");
    p
}

#[test]
fn a_build_of_the_witness_is_refused_only_for_its_hub() {
    let mut out_path = std::env::temp_dir();
    out_path.push(format!("hale_api_serve_build_{}_witness", std::process::id()));
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("build")
        .arg(witness())
        .arg("-o")
        .arg(&out_path)
        .env("HALE_SKIP_STALE_CHECK", "1")
        .output()
        .expect("run hale build");
    let _ = std::fs::remove_file(&out_path);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "the witness builds:\n{stderr}");
    // (R2b ships `unix::Rpc`, R3 `http::Rpc`)
    assert!(!stderr.contains("`api::serve` over `http::Rpc`"), "`http::Rpc` is served:\n{stderr}");
    assert!(!stderr.contains("`api::serve` over `unix::Rpc`"), "`unix::Rpc` is served:\n{stderr}");
    assert!(stderr.contains("`Fills` is bound to the hub `self.hub`"), "the hub binding is refused too:\n{stderr}");
}

#[test]
fn a_check_admits_the_witness() {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("check")
        .arg(witness())
        .env("HALE_SKIP_STALE_CHECK", "1")
        .output()
        .expect("run hale check");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
}

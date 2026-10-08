//! GH #1417 (R2a): a build refuses a serve site over a transport this
//! compiler does not ship. The R0 witness serves `Public` over
//! `http::Rpc` and `Admin` over `unix::Rpc`; `hale check` admits it
//! (R1's description and the serve-site laws read it as written), and
//! `hale build` says what it cannot serve yet, instead of dropping the
//! sites. A hub binding is refused likewise (R5).

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
fn a_build_refuses_a_serve_site_over_a_transport_it_does_not_ship() {
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
    for transport in ["http::Rpc", "unix::Rpc"] {
        assert!(
            stderr.contains(&format!("`api::serve` over `{transport}`: this compiler serves a surface over")),
            "no refusal of `{transport}` in:\n{stderr}"
        );
    }
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

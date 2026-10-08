//! GH #1417, R8a: the generated TypeScript client compiles under
//! `tsc --strict` and speaks the contract's wire on node's `fetch` and
//! `WebSocket` (`tests/fixtures/ts-client/run.mjs`).
//!
//! It needs `node` (22 or later) and a TypeScript compiler (`TSC`, or `tsc`
//! on the PATH). Where either is missing the test says so and passes, unless
//! `HALE_REQUIRE_TS_CLIENT=1` makes that a failure; the face-browser job of
//! CI, which has node, installs TypeScript and runs the fixture directly.

use std::path::Path;
use std::process::Command;

fn has(cmd: &str) -> bool {
    Command::new(cmd).arg("--version").output().is_ok_and(|o| o.status.success())
}

#[test]
fn the_generated_typescript_client_speaks_the_contract() {
    let tsc = std::env::var("TSC").unwrap_or_else(|_| "tsc".to_string());
    if !has("node") || !has(&tsc) {
        assert!(
            std::env::var("HALE_REQUIRE_TS_CLIENT").map_or(true, |v| v != "1"),
            "HALE_REQUIRE_TS_CLIENT=1, and node or {tsc} is not installed"
        );
        eprintln!("skipped: node and a TypeScript compiler are needed (the face-browser job runs this fixture)");
        return;
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = Command::new("node")
        .arg(root.join("crates/hale-cli/tests/fixtures/ts-client/run.mjs"))
        .env("HALE_BIN", env!("CARGO_BIN_EXE_hale"))
        .env("TSC", &tsc)
        .output()
        .expect("run node");
    assert!(
        out.status.success(),
        "the TypeScript client fixture failed:\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

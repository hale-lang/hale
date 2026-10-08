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

/// A surface type named like a global the wire uses (`Response`, `Record`,
/// `Promise`) does not shadow it: the generated client still compiles under
/// `tsc --strict`, and the type keeps the name the row gives it.
#[test]
fn a_surface_type_named_like_a_global_does_not_shadow_it() {
    let tsc = std::env::var("TSC").unwrap_or_else(|_| "tsc".to_string());
    if !has(&tsc) {
        assert!(std::env::var("HALE_REQUIRE_TS_CLIENT").map_or(true, |v| v != "1"), "HALE_REQUIRE_TS_CLIENT=1, and {tsc} is not installed");
        eprintln!("skipped: a TypeScript compiler is needed");
        return;
    }
    for name in ["Response", "Record", "Promise", "Array", "MessageEvent", "AsyncIterable"] {
        let dir = std::env::temp_dir().join(format!("hale_ts_shadow_{}_{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let program = dir.join("main.hl");
        std::fs::write(
            &program,
            format!(
                "type {name} {{ id: Int; }}\nlocus Echo {{ fn echo(r: {name}) -> {name} {{ return r; }} }}\napi Public {{ rpc Echo::echo; }}\nfn main() {{ }}\n"
            ),
        )
        .unwrap();
        let client = dir.join("client.ts");
        let made = Command::new(env!("CARGO_BIN_EXE_hale"))
            .args(["api", "client", "--surface", "Public", "--lang", "ts", "--out"])
            .arg(&client)
            .arg(&program)
            .env("HALE_SKIP_STALE_CHECK", "1")
            .output()
            .expect("run hale");
        assert!(made.status.success(), "{name}: {}", String::from_utf8_lossy(&made.stderr));
        let text = std::fs::read_to_string(&client).unwrap();
        assert!(text.contains(&format!("export interface {name} {{")), "{name}: the type keeps its name");
        let checked = Command::new(&tsc)
            .args(["--strict", "--target", "es2022", "--lib", "es2022,dom", "--noEmit"])
            .arg(&client)
            .output()
            .expect("run tsc");
        assert!(
            checked.status.success(),
            "{name}: tsc refused the client:\n{}{}",
            String::from_utf8_lossy(&checked.stdout),
            String::from_utf8_lossy(&checked.stderr)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

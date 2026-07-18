//! `hale test` runs the same Stage-2 FFI pickup `hale build` does
//! (2026-07-18; closed pond FRICTION "hale test cannot link @ffi
//! libs"): a `*_test.hl` importing a lib whose `hale.toml` declares
//! an `[ffi]` csrc/link surface must link and run. Before the fix
//! every such test died at link with undefined references while the
//! same file passed under `hale build`.

use std::process::Command;

#[test]
fn test_runner_links_ffi_libs() {
    let root = std::env::temp_dir().join(format!(
        "hale_test_ffi_{}",
        std::process::id()
    ));
    let lib = root.join("vendor/shim");
    let app = root.join("app/tests");
    std::fs::create_dir_all(&lib).expect("mkdir lib");
    std::fs::create_dir_all(&app).expect("mkdir app");
    // Workspace root marker so `vendor/...` imports resolve from
    // the test file's directory upward.
    std::fs::write(root.join("hale.toml"), "").expect("marker");

    // The FFI lib: one C symbol + the Hale wrapper declaring it.
    std::fs::write(
        lib.join("glue.c"),
        "long long shim_add(long long a, long long b) { return a + b; }\n",
    )
    .expect("write glue.c");
    std::fs::write(
        lib.join("hale.toml"),
        "[ffi]\ncsrc = [\"glue.c\"]\nlink = []\n",
    )
    .expect("write hale.toml");
    std::fs::write(
        lib.join("shim.hl"),
        r#"@ffi("c") fn shim_add(a: Int, b: Int) -> Int;
fn add(a: Int, b: Int) -> Int { return shim_add(a, b); }
"#,
    )
    .expect("write shim.hl");

    // The test: imports the lib, calls through the C symbol,
    // passes iff silent + exit 0 (the spec/testing.md contract).
    std::fs::write(
        app.join("shim_test.hl"),
        r#"import "vendor/shim" as shim;
fn main() {
    if shim::add(40, 2) != 42 {
        println("ffi add wrong");
        std::process::exit(1);
    }
}
"#,
    )
    .expect("write test");

    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("test")
        .arg(&app)
        .output()
        .expect("run hale test");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "hale test failed:\nstdout:\n{}\nstderr:\n{}",
        stdout,
        stderr
    );
    assert!(
        stdout.contains("1 passed, 0 failed"),
        "unexpected summary:\n{}",
        stdout
    );

    let _ = std::fs::remove_dir_all(&root);
}

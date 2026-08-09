//! The target model, from the outside (GH #445, PR 1).
//!
//! The exit criterion for the first Windows PR is narrow and worth
//! stating as a test rather than a claim: existing targets behave exactly
//! as before, and `x86_64-pc-windows-msvc` can be parsed and described.
//!
//! The second half is the easy half to get wrong in the flattering
//! direction. A target model that accepts a Windows triple and then dies
//! somewhere inside the linker has not "supported" anything — it has
//! moved the failure further from its cause. So these tests pin the
//! refusal too: naming a target the compiler cannot build must produce a
//! precise, early, actionable error.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

/// Per-process, per-case directory. These cases build real executables,
/// and the suite runs in parallel: two tests sharing an output path is
/// the `ETXTBSY`/wrong-binary failure `harness_paths_are_unique.rs`
/// exists to prevent in the codegen suite. Same hazard, same discipline.
fn case_dir(tag: &str) -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "hale_target_model_{}_{}_{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed),
        tag
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create case dir");
    dir
}

fn run(args: &[&str]) -> (String, String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(args)
        .output()
        .expect("run hale");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn list_targets_names_every_platform_and_its_tier() {
    let (stdout, _, code) = run(&["--list-targets"]);
    assert_eq!(code, 0, "--list-targets should succeed");

    for triple in [
        "x86_64-unknown-linux-gnu",
        "aarch64-unknown-linux-gnu",
        "x86_64-apple-darwin",
        "aarch64-apple-darwin",
        "x86_64-pc-windows-msvc",
        "aarch64-pc-windows-msvc",
        "wasm32-unknown-unknown",
    ] {
        assert!(stdout.contains(triple), "missing {triple} in:\n{stdout}");
    }

    // The listing must distinguish what it can build from what it can
    // merely name, or it is advertising.
    assert!(stdout.contains("supported: builds and links"), "{stdout}");
    assert!(stdout.contains("object-only"), "{stdout}");
    assert!(stdout.contains("planned"), "{stdout}");
    assert!(
        stdout.contains("(host)"),
        "host target not marked:\n{stdout}"
    );
}

#[test]
fn windows_target_is_described_with_its_own_file_conventions() {
    let (stdout, _, _) = run(&["--list-targets"]);
    let win = stdout
        .split("\n\n")
        .find(|b| b.starts_with("x86_64-pc-windows-msvc"))
        .unwrap_or_else(|| panic!("no windows block in:\n{stdout}"));

    // The whole point of the target model: these differ from the host's,
    // and they are answered by the target rather than by `cfg!`.
    assert!(win.contains(".obj"), "{win}");
    assert!(win.contains(".exe"), "{win}");
    assert!(win.contains(".lib"), "{win}");
    assert!(win.contains(".dll"), "{win}");
    assert!(win.contains("Msvc"), "{win}");
}

#[test]
fn building_for_windows_fails_early_and_says_why() {
    let dir = case_dir("target_model_win");
    let src = dir.join("t.hl");
    std::fs::write(&src, "fn main() { println(\"hi\"); }\n").unwrap();

    let (_, stderr, code) = run(&[
        "build",
        src.to_str().unwrap(),
        "--target",
        "x86_64-pc-windows-msvc",
    ]);

    assert_ne!(code, 0, "a planned target must not report success");
    assert!(stderr.contains("not buildable yet"), "{stderr}");
    assert!(stderr.contains("x86_64-pc-windows-msvc"), "{stderr}");
    // Point at the work, not at a dead end.
    assert!(
        stderr.contains("445"),
        "error should reference the issue:\n{stderr}"
    );
    // And it must fail at argument parsing, not after emitting something.
    assert!(
        !dir.join("t.exe").exists() && !dir.join("t").exists(),
        "a rejected target still produced an artifact"
    );
}

#[test]
fn an_unknown_triple_lists_the_ones_that_exist() {
    let dir = case_dir("target_model_unknown");
    let src = dir.join("t.hl");
    std::fs::write(&src, "fn main() { }\n").unwrap();

    // A near-miss: the right OS, the wrong ABI.
    let (_, stderr, code) = run(&[
        "build",
        src.to_str().unwrap(),
        "--target",
        "x86_64-pc-windows-gnu",
    ]);
    assert_ne!(code, 0);
    assert!(stderr.contains("unknown target"), "{stderr}");
    assert!(
        stderr.contains("x86_64-pc-windows-msvc"),
        "should suggest the real one:\n{stderr}"
    );
    assert!(
        stderr.contains("native"),
        "should mention the aliases:\n{stderr}"
    );
}

#[test]
fn the_existing_aliases_are_unchanged() {
    let dir = case_dir("target_model_native");
    let src = dir.join("t.hl");
    std::fs::write(&src, "fn main() { println(\"ok\"); }\n").unwrap();

    // `native` builds and runs, exactly as before the target model existed.
    let (_, stderr, code) = run(&["build", src.to_str().unwrap(), "--target", "native"]);
    assert_eq!(code, 0, "native build failed: {stderr}");
    let bin = src.with_extension("");
    assert!(bin.exists(), "no executable at {}", bin.display());
    let out = Command::new(&bin).output().expect("run built binary");
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "ok");

    // `wasm32` still names its output `.wasm`, which is now the target's
    // convention rather than a `with_extension` at one call site.
    let (_, stderr, code) = run(&["build", src.to_str().unwrap(), "--target", "wasm32"]);
    assert_eq!(code, 0, "wasm build failed: {stderr}");
    assert!(src.with_extension("wasm").exists(), "no .wasm output");
}

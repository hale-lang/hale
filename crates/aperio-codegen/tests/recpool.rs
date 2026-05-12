//! v1.x-3 PR1 — C runtime tests for the recognition pool primitives.
//!
//! Builds `runtime/lotus_arena.c` plus the `recpool_driver.c`
//! harness into a single binary, then exec's it with one mode
//! per scenario. The driver prints `OK <mode>` on success and
//! `FAIL <mode> <reason>` on failure; we assert the OK prefix.
//!
//! No codegen path exercised — PR1 is the recpool data structure
//! in isolation. PR4 wires it through codegen.

use std::path::PathBuf;
use std::process::Command;

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn runtime_c_path() -> PathBuf {
    let mut p = manifest_dir();
    p.push("runtime");
    p.push("lotus_arena.c");
    p
}

fn driver_c_path() -> PathBuf {
    let mut p = manifest_dir();
    p.push("tests");
    p.push("recpool_driver.c");
    p
}

fn build_driver(tag: &str) -> PathBuf {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut bin = std::env::temp_dir();
    bin.push(format!(
        "aperio_recpool_driver_{}_{}_{}",
        tag,
        std::process::id(),
        nanos
    ));
    let status = Command::new("clang")
        .arg(driver_c_path())
        .arg(runtime_c_path())
        .arg("-O2")
        .arg("-Wall")
        .arg("-Wextra")
        .arg("-lpthread")
        .arg("-o")
        .arg(&bin)
        .status()
        .expect("clang invocation");
    assert!(status.success(), "clang failed building recpool driver");
    bin
}

fn run_mode(bin: &PathBuf, mode: &str) {
    let out = Command::new(bin)
        .arg(mode)
        .output()
        .unwrap_or_else(|e| panic!("spawn {mode}: {e}"));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success() && stdout.starts_with(&format!("OK {mode}")),
        "mode {mode} failed:\n  status: {}\n  stdout: {}\n  stderr: {}",
        out.status,
        stdout,
        stderr,
    );
}

#[test]
fn recpool_fixed_basic_round_trip() {
    let bin = build_driver("fixed_basic");
    run_mode(&bin, "fixed_basic");
}

#[test]
fn recpool_fixed_overflow_returns_null() {
    let bin = build_driver("fixed_overflow");
    run_mode(&bin, "fixed_overflow");
}

#[test]
fn recpool_fixed_alloc_inside_cell_and_overflow_null() {
    let bin = build_driver("fixed_alloc");
    run_mode(&bin, "fixed_alloc");
}

#[test]
fn recpool_slab_basic_shared_arena() {
    let bin = build_driver("slab_basic");
    run_mode(&bin, "slab_basic");
}

#[test]
fn recpool_slab_alloc_overflow_returns_null() {
    let bin = build_driver("slab_overflow");
    run_mode(&bin, "slab_overflow");
}

#[test]
fn recpool_slab_release_is_noop() {
    let bin = build_driver("slab_noop");
    run_mode(&bin, "slab_noop");
}

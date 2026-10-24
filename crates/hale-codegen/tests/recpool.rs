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

#[path = "support/harness.rs"]
mod harness;

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
    let bin = harness::unique_bin(&format!(
        "hale_recpool_driver_{}_{}_{}",
        tag,
        std::process::id(),
        nanos
    ));
    // Link only the production pool/arena functions this driver reaches.
    // Other runtime subsystems live in separate TUs (observation and SHM).
    let mut cc = Command::new("clang");
    cc.arg(driver_c_path()).arg(runtime_c_path())
        .args(["-O2", "-Wall", "-Wextra", "-ffunction-sections", "-fdata-sections", "-lpthread", "-lm"]);
    cc.arg(if cfg!(target_os = "macos") { "-Wl,-dead_strip" } else { "-Wl,--gc-sections" });
    let built = cc.arg("-o").arg(&bin).output().expect("clang invocation");
    assert!(built.status.success(), "clang failed building recpool driver: {}", String::from_utf8_lossy(&built.stderr));
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
fn recpool_slab_release_preserves_storage() {
    let bin = build_driver("slab_noop");
    run_mode(&bin, "slab_noop");
}

#[test]
fn recpool_slab_admission_cap_and_storage_survive_release() {
    run_mode(&build_driver("slab_admission"), "slab_admission");
}

#[test]
fn recpool_reservations_reject_wrapping_sizes_and_zero_caps() {
    run_mode(&build_driver("creation_limits"), "creation_limits");
}

#[test]
fn recpool_fixed_concurrent_reservations_and_release_are_exact() {
    run_mode(&build_driver("fixed_concurrent"), "fixed_concurrent");
}

#[test]
fn recpool_slab_concurrent_reservations_and_release_are_exact() {
    run_mode(&build_driver("slab_concurrent"), "slab_concurrent");
}

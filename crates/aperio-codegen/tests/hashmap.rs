//! v1.x-FORM-4 PR4 — C runtime tests for lotus_hashmap_*.
//!
//! Builds `runtime/lotus_arena.c` plus the `hashmap_driver.c`
//! harness into a single binary, then execs it with one mode
//! per scenario. The driver prints `OK <mode>` on success and
//! `FAIL <mode> <reason>` on failure; we assert the OK prefix.
//!
//! No codegen path exercised — PR4 is the open-addressing
//! hashmap data structure in isolation. PR5 wires it through
//! codegen so user-source `@form(hashmap)` loci compile.

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
    p.push("hashmap_driver.c");
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
        "aperio_hashmap_driver_{}_{}_{}",
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
    assert!(status.success(), "clang failed building hashmap driver");
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
fn hashmap_int_basic_round_trip() {
    let bin = build_driver("int_basic");
    run_mode(&bin, "int_basic_round_trip");
}

#[test]
fn hashmap_string_basic_round_trip() {
    let bin = build_driver("string_basic");
    run_mode(&bin, "string_basic_round_trip");
}

#[test]
fn hashmap_string_distinct_pointers_equal_bytes() {
    let bin = build_driver("string_distinct");
    run_mode(&bin, "string_distinct_pointers");
}

#[test]
fn hashmap_grow_at_load_threshold() {
    let bin = build_driver("grow");
    run_mode(&bin, "grow_at_load_threshold");
}

#[test]
fn hashmap_overwrite_on_duplicate_key() {
    let bin = build_driver("overwrite");
    run_mode(&bin, "overwrite_on_duplicate_key");
}

#[test]
fn hashmap_remove_basic() {
    let bin = build_driver("remove_basic");
    run_mode(&bin, "remove_basic");
}

#[test]
fn hashmap_remove_with_probe_chain() {
    let bin = build_driver("remove_chain");
    run_mode(&bin, "remove_with_probe_chain");
}

#[test]
fn hashmap_len_and_is_empty() {
    let bin = build_driver("len");
    run_mode(&bin, "len_and_is_empty");
}

#[test]
fn hashmap_get_missing_returns_zero() {
    let bin = build_driver("get_missing");
    run_mode(&bin, "get_missing");
}

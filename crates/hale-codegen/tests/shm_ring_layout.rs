//! Proposal B (2026-06-06) — foreign-layout SHM ring consumer test.
//!
//! Compiles `runtime/lotus_shm_ring.c` plus the
//! `shm_ring_layout_driver.c` harness into a one-off binary and
//! runs it in two modes (see the driver header):
//!
//!   1. `roundtrip` — capacity holds all records; deterministic
//!      exact in-order delivery through the `byte_records` reader.
//!   2. `wrap` — small capacity + a paced producer forces the
//!      pad-at-wrap branch; still asserts exact delivery.
//!
//! Exercises the C-ABI runtime
//! (lotus_bus_register_subscriber_shm_ring_layout + the layout
//! reader thread). The codegen side (a `layout:` binding emitting
//! this call) is covered in `shm_ring_layout_codegen.rs`.

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn build_driver(tag: &str) -> PathBuf {
    let mut driver_c = manifest_dir();
    driver_c.push("tests");
    driver_c.push("shm_ring_layout_driver.c");
    let mut ring_c = manifest_dir();
    ring_c.push("runtime");
    ring_c.push("lotus_shm_ring.c");

    let mut bin = std::env::temp_dir();
    bin.push(format!("lotus_shm_ring_layout_driver_{}", tag));
    let status = Command::new("clang")
        .arg(driver_c)
        .arg(ring_c)
        .arg("-O2")
        .arg("-lrt")
        .arg("-lpthread")
        .arg("-o")
        .arg(&bin)
        .status()
        .expect("clang invocation");
    assert!(status.success(), "clang failed building shm_ring layout driver");
    bin
}

fn unique_shm_name(tag: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("/hale-shmlayout-{}-{}-{}", tag, std::process::id(), nanos)
}

fn run_mode(mode: &str) {
    let driver = build_driver(mode);
    let name = unique_shm_name(mode);
    let out = Command::new(&driver)
        .arg(mode)
        .arg(&name)
        .output()
        .expect("run driver");
    let _ = std::fs::remove_file(&driver);
    assert!(
        out.status.success(),
        "{} driver failed: status={:?}\nstderr: {}",
        mode,
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn layout_byte_records_roundtrip() {
    run_mode("roundtrip");
}

#[test]
fn layout_byte_records_wrap() {
    run_mode("wrap");
}

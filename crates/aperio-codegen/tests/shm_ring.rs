//! Form K5 (2026-05-20) — SHM ring substrate test.
//!
//! Compiles `runtime/lotus_shm_ring.c` plus the
//! `shm_ring_driver.c` harness into a one-off binary in $TMPDIR
//! and invokes it in three modes:
//!
//!   1. `roundtrip` — publish & read each payload in succession
//!      (no wraparound). Validates the core
//!      open/claim/commit/read_slot/published API.
//!   2. `wraparound` — publish 3x slot_count without reading,
//!      verify stale seqnos return NULL and live seqnos return
//!      the right payload.
//!   3. `ipc-parent` + `ipc-child` — two separate processes
//!      attach the same SHM ring; the parent publishes, the
//!      child polls + validates. Validates cross-process
//!      delivery over POSIX SHM.
//!
//! No codegen path exercised — K5 is a C-runtime addition with
//! a stable C ABI. K3's slot-locus codegen lands in K4 + K6.

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn driver_c_path() -> PathBuf {
    let mut p = manifest_dir();
    p.push("tests");
    p.push("shm_ring_driver.c");
    p
}

fn ring_c_path() -> PathBuf {
    let mut p = manifest_dir();
    p.push("runtime");
    p.push("lotus_shm_ring.c");
    p
}

fn build_driver(tag: &str) -> PathBuf {
    let mut bin = std::env::temp_dir();
    bin.push(format!("lotus_shm_ring_driver_{}", tag));
    let status = Command::new("clang")
        .arg(driver_c_path())
        .arg(ring_c_path())
        .arg("-O2")
        .arg("-lrt")
        .arg("-o")
        .arg(&bin)
        .status()
        .expect("clang invocation");
    assert!(status.success(), "clang failed building shm_ring driver");
    bin
}

fn unique_shm_name(tag: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("/aperio-shmring-{}-{}-{}", tag, std::process::id(), nanos)
}

#[test]
fn shm_ring_roundtrip() {
    let driver = build_driver("rt");
    let name = unique_shm_name("rt");
    let out = Command::new(&driver)
        .arg("roundtrip")
        .arg(&name)
        .output()
        .expect("run driver");
    let _ = std::fs::remove_file(&driver);
    assert!(
        out.status.success(),
        "roundtrip driver failed: status={:?}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn shm_ring_wraparound() {
    let driver = build_driver("wr");
    let name = unique_shm_name("wr");
    let out = Command::new(&driver)
        .arg("wraparound")
        .arg(&name)
        .output()
        .expect("run driver");
    let _ = std::fs::remove_file(&driver);
    assert!(
        out.status.success(),
        "wraparound driver failed: status={:?}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn shm_ring_cross_process_publish_subscribe() {
    let driver = build_driver("ipc");
    let name = unique_shm_name("ipc");

    // Spawn the child first so it's polling when the parent
    // starts publishing.
    let mut child = Command::new(&driver)
        .arg("ipc-child")
        .arg(&name)
        .spawn()
        .expect("spawn child");

    // Small head-start so the child enters its retry-attach loop.
    std::thread::sleep(std::time::Duration::from_millis(20));

    let parent_out = Command::new(&driver)
        .arg("ipc-parent")
        .arg(&name)
        .output()
        .expect("run parent");

    let child_status = child.wait().expect("wait child");
    let _ = std::fs::remove_file(&driver);

    assert!(
        parent_out.status.success(),
        "parent failed: status={:?}\nstderr: {}",
        parent_out.status,
        String::from_utf8_lossy(&parent_out.stderr)
    );
    assert!(
        child_status.success(),
        "child failed: status={:?}",
        child_status
    );
}

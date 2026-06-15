//! Torn-read regression for the `post_copy` SPMC ring guard
//! (fast-protocol-I/O #5, hardening 2026-06-13).
//!
//! The pre-fix guard resynced only after the producer had lapped by a full
//! `cap`; the proof (the foreign bus ABI header) requires resyncing at the safe window
//! `cap - S` (S = min(cap/4, 1 MiB)), because a producer within `S` of
//! lapping can already be overwriting the record the reader is copying. So
//! the bug is precisely that the reader delivers a record whose lag behind
//! the cursor lies in the danger band (cap-S, cap).
//!
//! The C `clobber` driver drives that decision DETERMINISTICALLY (no thread
//! race): it plants one record at a lag in the danger band whose header seq
//! disagrees with its payload — the shape a real mid-copy tear produces —
//! and checks whether the reader delivers it.
//!
//!   correct reader (window = cap - S): resyncs past it → never delivered.
//!   pre-fix reader (window = cap):     delivers it    → driver returns rc 3.
//!
//! A consistent control record at low lag is delivered first by both readers,
//! so a correct reader's zero danger-band deliveries read as "correctly
//! skipped", not "never ran" (driver rc 4). Deterministic: fails on the bug,
//! passes on the fix, every run.

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn unique_tag() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("clobber-{}-{}", std::process::id(), nanos)
}

fn build_driver() -> PathBuf {
    let mut driver_c = manifest_dir();
    driver_c.push("tests");
    driver_c.push("shm_ring_layout_driver.c");
    let mut ring_c = manifest_dir();
    ring_c.push("runtime");
    ring_c.push("lotus_shm_ring.c");
    let mut bin = std::env::temp_dir();
    bin.push(format!("lotus_{}", unique_tag()));
    let status = Command::new("clang")
        .arg(&driver_c)
        .arg(&ring_c)
        .arg("-O2")
        .arg("-lrt")
        .arg("-lpthread")
        .arg("-o")
        .arg(&bin)
        .status()
        .expect("clang");
    assert!(status.success(), "clang failed building the clobber driver");
    bin
}

#[test]
fn post_copy_guard_resyncs_past_danger_band_records() {
    let bin = build_driver();
    let shm_name = format!("/hale-{}", unique_tag());

    let out = Command::new(&bin)
        .arg("clobber")
        .arg(&shm_name)
        .output()
        .expect("run clobber driver");

    let _ = std::fs::remove_file(&bin);

    let stderr = String::from_utf8_lossy(&out.stderr);
    // Surface the `delivered=.. torn=..` line even on success (--nocapture).
    eprint!("{stderr}");
    assert!(
        out.status.success(),
        "torn-read regression failed: status={:?}\nstderr:\n{stderr}",
        out.status,
    );
}

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
    // Salt with pid + a process-wide counter so two tests can never
    // share a driver-binary path under nextest's parallel execution
    // (the bytes_pack_read flake class). The SHM object names are
    // already unique via unique_shm_name.
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    bin.push(format!(
        "lotus_shm_ring_layout_driver_{}_{}_{}",
        tag,
        std::process::id(),
        seq
    ));
    let mut cmd = Command::new("clang");
    cmd.arg(&driver_c).arg(&ring_c);
    // Honor the same sanitizer env flags as the codegen build, so the
    // foreign-ring suite can be run under ASan / UBSan (the hostile-
    // producer cases are where OOB bugs live). UBSan adds the signed-
    // overflow / OOB checks; -O2 otherwise.
    if std::env::var("LOTUS_UBSAN").map(|v| v == "1").unwrap_or(false) {
        cmd.arg("-fsanitize=address,undefined")
            // The raw (BytesView) path calls the handler through an
            // intentionally cast, ABI-compatible function pointer; the
            // driver's view struct and the runtime's are layout-identical
            // but distinct C types, which trips UBSan's (overly strict
            // for this) function-type check. The real Hale path is clean
            // (LLVM methods carry no UBSan type descriptor). Address +
            // overflow + alignment checks stay on.
            .arg("-fno-sanitize=function")
            .arg("-fno-sanitize-recover=all")
            .arg("-O1")
            .arg("-g");
    } else if std::env::var("LOTUS_ASAN").map(|v| v == "1").unwrap_or(false) {
        cmd.arg("-fsanitize=address").arg("-O1").arg("-g");
    } else {
        cmd.arg("-O2");
    }
    let status = cmd
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

// Proposal B M3a — the PRODUCER C ABI
// (lotus_bus_register_shm_ring_layout + publish_shm_ring_layout):
// register a producer that creates the ring, publish records, and
// confirm a consumer reads them back in order. Validates that the
// producer's framing is the exact inverse of the reader's.

#[test]
fn layout_producer_roundtrip() {
    run_mode("producer");
}

#[test]
fn layout_producer_wrap() {
    run_mode("producer_wrap");
}

// --- Hardening (2026-06-08): hostile / non-conforming foreign producers
// at the boundary ring_layout exists to serve. Run under ASan+UBSan
// (LOTUS_UBSAN=1) to confirm no OOB. ---

/// A foreign record whose framed `len` is shorter than the bound
/// payload's value_size must be resynced (dropped), never dispatched —
/// dispatching it would let the handler read value_size bytes past the
/// record (OOB near the wrap). The driver writes two conforming records
/// then a short one; the consumer must receive exactly the two.
#[test]
fn layout_short_record_resynced_not_dispatched() {
    run_mode("short_record");
}

/// A conforming foreign ring with a u64 length prefix
/// (len_prefix_width == align == 8) must round-trip cleanly.
#[test]
fn layout_u64_len_prefix_roundtrips() {
    run_mode("u64_lenprefix");
}

/// A foreign producer advertising a `buffer_size` that isn't a multiple
/// of `align` must be REJECTED at attach (cap % align != 0), not read —
/// otherwise a record header could straddle the wrap. The driver exits
/// non-zero (the register path _exit(1)s with a diagnostic); the test
/// confirms it rejected (didn't reach the "BUG" line) with no sanitizer
/// error.
#[test]
fn layout_bad_buffer_size_rejected() {
    let driver = build_driver("badbuf");
    let name = unique_shm_name("badbuf");
    let out = Command::new(&driver)
        .arg("bad_bufsize")
        .arg(&name)
        .output()
        .expect("run driver");
    let _ = std::fs::remove_file(&driver);
    let stderr = String::from_utf8_lossy(&out.stderr);
    // Rejected (non-zero exit), and NOT via a sanitizer fault.
    assert!(
        !out.status.success(),
        "a non-align-multiple buffer_size must be rejected at attach, but the \
         consumer accepted it.\nstderr: {}",
        stderr
    );
    assert!(
        !stderr.contains("AddressSanitizer")
            && !stderr.contains("runtime error")
            && !stderr.contains("BUG:"),
        "rejection must be clean (no OOB / no wrongful accept).\nstderr: {}",
        stderr
    );
    // best-effort segment cleanup (the _exit(1) skips the driver's unlink)
    let stripped = name.trim_start_matches('/');
    let _ = std::fs::remove_file(format!("/dev/shm/{}", stripped));
}

/// Raw / heterogeneous foreign ring (value_size == 0): records of two
/// different sizes tagged by an i64 `kind` discriminator, consumed by a
/// single raw subscriber that receives a BytesView per record and
/// decodes both shapes. The path for real mixed-record external rings.
#[test]
fn layout_heterogeneous_raw_view() {
    run_mode("heterogeneous");
}

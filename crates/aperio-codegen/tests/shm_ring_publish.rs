//! Form K4c (2026-05-20) — end-to-end test: an Aperio publisher
//! routes `Topic <- value` through `lotus_bus_publish_shm_ring`
//! when its binding is `shm_ring(...)`, and a C reader process
//! reads + validates the payloads from the same SHM ring.
//!
//! Verifies the codegen wiring (emit_bindings_prelude emits the
//! shm_ring register call; lower_send short-circuits to
//! publish_shm_ring) AND the runtime wiring (subject->ring
//! registry; one-memcpy publish path; cross-process SHM
//! delivery).

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use aperio_codegen::build_executable;

/// Check whether a POSIX SHM object exists in the kernel
/// namespace. On Linux, POSIX SHM objects live at
/// `/dev/shm/<name>` (with the leading slash from the name
/// stripped). Used to confirm the K-cleanup atexit hook
/// actually shm_unlink'd creator-owned rings.
fn shm_object_exists(shm_name: &str) -> bool {
    let stripped = shm_name.trim_start_matches('/');
    PathBuf::from(format!("/dev/shm/{}", stripped)).exists()
}

fn unique_tag(tag: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("k4c-{}-{}-{}", tag, std::process::id(), nanos)
}

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn build_reader(tag: &str) -> PathBuf {
    let mut driver = manifest_dir();
    driver.push("tests");
    driver.push("shm_ring_publish_driver.c");

    let mut ring_c = manifest_dir();
    ring_c.push("runtime");
    ring_c.push("lotus_shm_ring.c");

    let mut bin = std::env::temp_dir();
    bin.push(format!("lotus_shm_ring_reader_{}", tag));
    let status = Command::new("clang")
        .arg(&driver)
        .arg(&ring_c)
        .arg("-O2")
        .arg("-lrt")
        .arg("-o")
        .arg(&bin)
        .status()
        .expect("clang invocation");
    assert!(status.success(), "clang failed building shm_ring reader");
    bin
}

#[test]
fn aperio_publisher_routes_through_shm_ring() {
    let tag = unique_tag("pub");
    let shm_name = format!("/aperio-{}", tag);
    let n_pub: i64 = 5;
    let slot_count: u64 = 8;

    // Aperio publisher source.
    //
    // - `type Tick { px: Int; sz: Int; }` — flat payload, both
    //   fields are 64-bit ints (matches the C reader's struct).
    // - `topic Tick { payload: Tick; }` — the topic decl.
    // - Publisher locus declares `bus { publish Tick; }` and
    //   sends N Tick values in its init() lifecycle.
    // - main locus binds `Tick: shm_ring(...)`; codegen routes
    //   the Send statements through lotus_bus_publish_shm_ring.
    let aperio_src = format!(
        r#"
        type TickPayload {{
            px: Int;
            sz: Int;
        }}
        topic Tick {{ payload: TickPayload; }}

        locus Producer {{
            bus {{ publish Tick; }}
            birth() {{
                Tick <- TickPayload {{ px: 1, sz: 7 }};
                Tick <- TickPayload {{ px: 2, sz: 14 }};
                Tick <- TickPayload {{ px: 3, sz: 21 }};
                Tick <- TickPayload {{ px: 4, sz: 28 }};
                Tick <- TickPayload {{ px: 5, sz: 35 }};
            }}
        }}

        main locus App {{
            bindings {{
                Tick: shm_ring("{shm_name}", slot_count: {slot_count}, on_overflow: drop) where zero_copy;
            }}
        }}

        fn main() {{
            App {{ }};
            Producer {{ }};
        }}
    "#,
        shm_name = shm_name,
        slot_count = slot_count,
    );

    let program = aperio_syntax::parse_source(&aperio_src).expect("parse");
    let mut publisher_bin = std::env::temp_dir();
    publisher_bin.push(format!("lotus_shm_pub_{}.bin", tag));
    build_executable(&program, &publisher_bin).expect("build publisher");

    let reader_bin = build_reader(&tag);

    // Spawn the reader first — it retry-attaches until the
    // publisher creates the SHM ring.
    let mut reader = Command::new(&reader_bin)
        .arg(&shm_name)
        .arg(slot_count.to_string())
        .arg(n_pub.to_string())
        .spawn()
        .expect("spawn reader");

    // Small head-start so the reader is polling before the
    // publisher creates the ring.
    std::thread::sleep(std::time::Duration::from_millis(20));

    let publisher_out = Command::new(&publisher_bin)
        .output()
        .expect("run publisher");

    let reader_status = reader.wait().expect("wait reader");

    let _ = std::fs::remove_file(&publisher_bin);
    let _ = std::fs::remove_file(&reader_bin);

    assert!(
        publisher_out.status.success(),
        "publisher failed: status={:?}\nstdout: {}\nstderr: {}",
        publisher_out.status,
        String::from_utf8_lossy(&publisher_out.stdout),
        String::from_utf8_lossy(&publisher_out.stderr),
    );
    assert!(
        reader_status.success(),
        "reader failed: status={:?}",
        reader_status
    );
}

/// Form K-cleanup (2026-05-20): the atexit hook installed when
/// the publisher registers an shm_ring binding must shm_unlink
/// the creator-owned ring on a clean process exit. Without this,
/// /dev/shm/ accumulates stale entries across restarts.
///
/// Builds a publisher-only Aperio binary, runs it to completion,
/// then checks that `/dev/shm/<name>` no longer exists.
#[test]
fn shm_object_unlinked_on_clean_exit() {
    let tag = unique_tag("unlink");
    let shm_name = format!("/aperio-{}", tag);
    let slot_count: u64 = 4;

    let aperio_src = format!(
        r#"
        type TickPayload {{
            px: Int;
            sz: Int;
        }}
        topic Tick {{ payload: TickPayload; }}
        locus Producer {{
            bus {{ publish Tick; }}
            birth() {{
                Tick <- TickPayload {{ px: 1, sz: 7 }};
            }}
        }}
        main locus App {{
            bindings {{
                Tick: shm_ring("{shm_name}", slot_count: {slot_count}, on_overflow: drop) where zero_copy;
            }}
        }}
        fn main() {{
            App {{ }};
            Producer {{ }};
        }}
    "#,
        shm_name = shm_name,
        slot_count = slot_count,
    );
    let program = aperio_syntax::parse_source(&aperio_src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("lotus_shm_unlink_{}.bin", tag));
    build_executable(&program, &bin).expect("build");

    // Pre-condition: name doesn't exist yet (unique per test).
    assert!(
        !shm_object_exists(&shm_name),
        "test setup error: `{}` already in /dev/shm/ before run",
        shm_name
    );

    let out = Command::new(&bin).output().expect("run publisher");
    let _ = std::fs::remove_file(&bin);

    assert!(
        out.status.success(),
        "publisher failed: status={:?}\nstdout: {}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );

    // After the publisher exits cleanly, the atexit hook should
    // have shm_unlink'd the ring.
    assert!(
        !shm_object_exists(&shm_name),
        "atexit hook failed to shm_unlink `{}` — /dev/shm/ entry persists",
        shm_name
    );
}

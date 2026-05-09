//! m57: AF_UNIX transport in the C runtime.
//!
//! Compiles `runtime/lotus_arena.c` plus the `transport_driver.c`
//! harness into a single binary, then spawns it twice:
//!
//!   1. Listener: bind / listen / accept on a unique socket path,
//!      recv one SEQPACKET message, write the bytes to stdout.
//!   2. Connector: connect-with-retry to the same path, send one
//!      SEQPACKET message containing the test payload.
//!
//! Asserting the listener's stdout matches the connector's argv
//! payload proves SOCK_SEQPACKET semantics hold end-to-end through
//! the C-runtime surface (`lotus_transport_create / send / recv /
//! destroy`). m58 will route `bus subscribe` subjects through this
//! same surface via deployment-config; m57 is the kernel-level
//! transport substrate only.
//!
//! No codegen path is exercised here — m57 is a C-runtime addition
//! with a stable C-ABI surface, not a surface-language change.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

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
    p.push("transport_driver.c");
    p
}

/// Compile the test driver + lotus_arena.c into a one-off binary
/// in $TMPDIR. Returns the path; caller is responsible for the
/// best-effort cleanup at the end of the test.
fn build_driver(name: &str) -> PathBuf {
    let mut bin = std::env::temp_dir();
    bin.push(format!("lotus_transport_driver_{}", name));
    let status = Command::new("clang")
        .arg(driver_c_path())
        .arg(runtime_c_path())
        .arg("-O2")
        .arg("-lpthread")
        .arg("-o")
        .arg(&bin)
        .status()
        .expect("clang invocation");
    assert!(status.success(), "clang failed building transport driver");
    bin
}

/// Build a unique socket path per test so parallel `cargo test`
/// invocations don't collide. AF_UNIX paths are limited to 108
/// bytes including NUL on Linux — keep this short.
fn unique_socket_path(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut p = std::env::temp_dir();
    p.push(format!("lt-m57-{}-{}-{}.sock", tag, std::process::id(), nanos));
    p
}

#[test]
fn transport_round_trip_short_message() {
    let driver = build_driver("short");
    let sock = unique_socket_path("short");
    let payload = "hello, lotus";

    // Spawn listener first — it'll bind, listen, then block on
    // accept until the connector connects. The connector retries
    // on ENOENT/ECONNREFUSED for ~1s so a tiny race here doesn't
    // matter, but starting the listener first is the natural order.
    let listener = Command::new(&driver)
        .arg("listen")
        .arg(&sock)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn listener");

    let connector = Command::new(&driver)
        .arg("connect")
        .arg(&sock)
        .arg(payload)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn connector");

    let listen_out = listener.wait_with_output().expect("listener wait");
    let connect_out = connector.wait_with_output().expect("connector wait");

    let _ = std::fs::remove_file(&sock);
    let _ = std::fs::remove_file(&driver);

    assert!(
        listen_out.status.success(),
        "listener exited non-zero: {:?}\nstderr: {}",
        listen_out.status,
        String::from_utf8_lossy(&listen_out.stderr),
    );
    assert!(
        connect_out.status.success(),
        "connector exited non-zero: {:?}\nstderr: {}",
        connect_out.status,
        String::from_utf8_lossy(&connect_out.stderr),
    );
    assert_eq!(
        String::from_utf8_lossy(&listen_out.stdout),
        payload,
        "listener stdout should be exactly the bytes sent",
    );
}

#[test]
fn transport_preserves_message_boundaries() {
    // SOCK_SEQPACKET should deliver one send as exactly one recv,
    // including a payload with embedded whitespace + a trailing
    // newline. If we accidentally fell back to SOCK_STREAM we'd
    // either need framing or risk truncation; this assertion
    // catches that regression directly.
    let driver = build_driver("boundary");
    let sock = unique_socket_path("boundary");
    let payload = "alpha beta gamma\n";

    let listener = Command::new(&driver)
        .arg("listen")
        .arg(&sock)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn listener");

    let connector = Command::new(&driver)
        .arg("connect")
        .arg(&sock)
        .arg(payload)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn connector");

    let listen_out = listener.wait_with_output().expect("listener wait");
    let connect_out = connector.wait_with_output().expect("connector wait");

    let _ = std::fs::remove_file(&sock);
    let _ = std::fs::remove_file(&driver);

    assert!(listen_out.status.success(), "listener: {:?}", listen_out);
    assert!(connect_out.status.success(), "connector: {:?}", connect_out);
    assert_eq!(
        String::from_utf8_lossy(&listen_out.stdout),
        payload,
        "exact byte-for-byte preservation of one SEQPACKET message",
    );
}

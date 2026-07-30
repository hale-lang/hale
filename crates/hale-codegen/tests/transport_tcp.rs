//! m72: AF_INET TCP transport in the C runtime.
//!
//! Builds `runtime/lotus_arena.c` plus the `transport_tcp_driver.c`
//! harness into a single binary, then spawns it twice:
//!
//!   1. Listener: bind + listen + accept on a host:port, receive
//!      framed messages until the peer closes the stream, write
//!      each message to stdout delimited by `\n----\n`.
//!   2. Connector: connect-with-retry to the same host:port, send
//!      one or more framed messages, then close.
//!
//! Asserting the listener's stdout matches the connector's argv
//! payloads — message-by-message — proves the length-prefix
//! framer correctly preserves boundaries through a SOCK_STREAM
//! transport. This is the per-transport guarantee called out in
//! `project_tcp_framing.md`: the m57 SEQPACKET test verifies the
//! same property via kernel datagram semantics; this test
//! verifies it via in-adapter framing.
//!
//! No codegen path is exercised here — m72 is a C-runtime
//! addition with a stable C-ABI surface, not a surface-language
//! change. m73 wires this up to `std::io::tcp::*` calls in `.hl`
//! source.

use std::path::PathBuf;
use std::process::{Command, Stdio};

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
    p.push("transport_tcp_driver.c");
    p
}

fn build_driver(name: &str) -> PathBuf {
    let bin = harness::unique_bin(&format!("hale_tcp_driver_{}", name));
    let status = Command::new("clang")
        .arg(driver_c_path())
        .arg(runtime_c_path())
        .arg("-O2")
        .arg("-lpthread")
        .arg("-o")
        .arg(&bin)
        .status()
        .expect("clang invocation");
    assert!(status.success(), "clang failed building tcp driver");
    bin
}

/// Find a free localhost port by binding `127.0.0.1:0`, reading
/// the OS-assigned port, then releasing the binding. The C
/// listener that follows uses SO_REUSEADDR so the brief
/// TIME_WAIT / rebind race doesn't break the test.
fn pick_free_port() -> u16 {
    let probe = std::net::TcpListener::bind("127.0.0.1:0").expect("bind probe");
    let port = probe.local_addr().expect("local_addr").port();
    drop(probe);
    port
}

#[test]
fn tcp_round_trip_short_message() {
    let driver = build_driver("short");
    let port = pick_free_port();
    let payload = "hello, hale";

    let listener = Command::new(&driver)
        .arg("listen")
        .arg("127.0.0.1")
        .arg(port.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn listener");

    let connector = Command::new(&driver)
        .arg("connect")
        .arg("127.0.0.1")
        .arg(port.to_string())
        .arg(payload)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn connector");

    let listen_out = listener.wait_with_output().expect("listener wait");
    let connect_out = connector.wait_with_output().expect("connector wait");
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
    let stdout = String::from_utf8_lossy(&listen_out.stdout);
    let messages: Vec<&str> = stdout.split("\n----\n").filter(|m| !m.is_empty()).collect();
    assert_eq!(messages.len(), 1, "expected one message; got: {:?}", stdout);
    assert_eq!(messages[0], payload);
}

#[test]
fn tcp_preserves_message_boundaries_across_three_sends() {
    // The cornerstone m72 test: TCP is a byte stream and would
    // happily deliver three sends as one merged blob without
    // framing. With the 8-byte LE length prefix the receiver
    // pulls exactly N bytes per message — three sends → three
    // recvs of identical bytes. This is the regression catch
    // that mirrors transport.rs's `transport_preserves_message
    // _boundaries` for the SEQPACKET adapter.
    let driver = build_driver("boundary");
    let port = pick_free_port();
    let payloads = ["alpha beta gamma\n", "second\n\nmessage", "third"];

    let listener = Command::new(&driver)
        .arg("listen")
        .arg("127.0.0.1")
        .arg(port.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn listener");

    let mut connector = Command::new(&driver);
    connector
        .arg("connect")
        .arg("127.0.0.1")
        .arg(port.to_string());
    for p in &payloads {
        connector.arg(p);
    }
    let connector = connector
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn connector");

    let listen_out = listener.wait_with_output().expect("listener wait");
    let connect_out = connector.wait_with_output().expect("connector wait");
    let _ = std::fs::remove_file(&driver);

    assert!(
        listen_out.status.success(),
        "listener: {:?}\nstderr: {}",
        listen_out.status,
        String::from_utf8_lossy(&listen_out.stderr)
    );
    assert!(
        connect_out.status.success(),
        "connector: {:?}\nstderr: {}",
        connect_out.status,
        String::from_utf8_lossy(&connect_out.stderr)
    );
    let stdout = String::from_utf8_lossy(&listen_out.stdout);
    let messages: Vec<&str> = stdout.split("\n----\n").filter(|m| !m.is_empty()).collect();
    assert_eq!(
        messages.len(),
        payloads.len(),
        "expected {} framed messages; got {} from stdout: {:?}",
        payloads.len(),
        messages.len(),
        stdout
    );
    for (i, expected) in payloads.iter().enumerate() {
        assert_eq!(
            messages[i], *expected,
            "message {}: expected {:?}, got {:?}",
            i, expected, messages[i]
        );
    }
}

#[test]
fn tcp_recv_rejects_messages_larger_than_caller_cap() {
    // The framed length is read first; if it exceeds the
    // caller's buffer we return -1 with errno=EMSGSIZE rather
    // than reading partial bytes into the caller's buffer. This
    // protects the caller from a malicious peer that claims an
    // arbitrarily large message — the bytes never touch caller
    // memory.
    //
    // We exercise this by giving the listener a *small* buffer
    // (BUF_CAP in the driver is 64KB) and sending a payload
    // larger than that. Connector succeeds (send is unbounded
    // until 8MB); listener's recv returns -1 → loop exits with
    // zero messages written to stdout.
    let driver = build_driver("oversize");
    let port = pick_free_port();
    let big_payload = "x".repeat(64 * 1024 + 1); // BUF_CAP + 1

    let listener = Command::new(&driver)
        .arg("listen")
        .arg("127.0.0.1")
        .arg(port.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn listener");

    let connector = Command::new(&driver)
        .arg("connect")
        .arg("127.0.0.1")
        .arg(port.to_string())
        .arg(&big_payload)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn connector");

    let listen_out = listener.wait_with_output().expect("listener wait");
    let connect_out = connector.wait_with_output().expect("connector wait");
    let _ = std::fs::remove_file(&driver);

    // Listener exits cleanly (status 0) — the recv -1 returns
    // it from the loop, not aborts the program.
    assert!(listen_out.status.success(), "listener: {:?}", listen_out.status);
    assert!(connect_out.status.success(), "connector: {:?}", connect_out.status);
    let stdout = String::from_utf8_lossy(&listen_out.stdout);
    let stderr = String::from_utf8_lossy(&listen_out.stderr);
    assert!(
        stdout.is_empty() || !stdout.contains(&big_payload[..1024]),
        "oversized payload should not have reached caller buffer; stdout: {:?}",
        stdout
    );
    assert!(
        stderr.contains("exceeds caller cap"),
        "expected EMSGSIZE diagnostic; stderr: {:?}",
        stderr
    );
}

#[test]
fn tcp_connector_retries_when_listener_is_slow_to_bind() {
    // The connector's ECONNREFUSED retry loop (~1s) lets a
    // connector that races ahead of its listener succeed once
    // the listener becomes ready. We exercise this by spawning
    // the connector first, then the listener after a 100ms gap.
    // If the retry was missing the connector would fail
    // immediately and the test would catch it.
    let driver = build_driver("retry");
    let port = pick_free_port();
    let payload = "retry-ok";

    let connector = Command::new(&driver)
        .arg("connect")
        .arg("127.0.0.1")
        .arg(port.to_string())
        .arg(payload)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn connector");

    std::thread::sleep(std::time::Duration::from_millis(100));

    let listener = Command::new(&driver)
        .arg("listen")
        .arg("127.0.0.1")
        .arg(port.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn listener");

    let listen_out = listener.wait_with_output().expect("listener wait");
    let connect_out = connector.wait_with_output().expect("connector wait");
    let _ = std::fs::remove_file(&driver);

    assert!(
        listen_out.status.success(),
        "listener: {:?}\nstderr: {}",
        listen_out.status,
        String::from_utf8_lossy(&listen_out.stderr)
    );
    assert!(
        connect_out.status.success(),
        "connector: {:?}\nstderr: {}",
        connect_out.status,
        String::from_utf8_lossy(&connect_out.stderr)
    );
    let stdout = String::from_utf8_lossy(&listen_out.stdout);
    let messages: Vec<&str> = stdout.split("\n----\n").filter(|m| !m.is_empty()).collect();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0], payload);
}

//! Phase 2g — Bytes-shaped TCP recv + Bytes/String conversions.
//!
//! Closes notes/hale-friction.md `tcp-recv-string-strlen-truncates-binary`.
//! The pre-2g `Stream.recv(max) -> String` reads bytes correctly into
//! the arena buffer but every consumer (`len`, slice, `==`) goes
//! through strlen, so any payload containing a NUL byte truncates
//! silently. WebSocket frames (mandatory client→server masking puts
//! ~1/256 NULs across the wire), HTTP/2 framing, raw market feeds,
//! file uploads, custom RPC formats — all hit this wall.
//!
//! Phase 2g adds `Stream.recv_bytes(max) -> Bytes` backed by
//! `lotus_tcp_recv_bytes` (length-prefixed, NUL-safe), plus
//! `std::bytes::from_string` / `std::str::from_bytes` for crossing
//! the shape boundary, and `std::bytes::at` / `std::bytes::slice` so
//! binary protocol parsers don't need C-side primitives for byte-
//! at-i and substring lookups.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;
use std::sync::mpsc;
use std::thread;

use hale_codegen::build_executable;

fn build_hale(name: &str, source: &str) -> std::path::PathBuf {
    let program = hale_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_test_recvbytes_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

fn pick_free_port() -> u16 {
    let probe = TcpListener::bind("127.0.0.1:0").expect("bind probe");
    probe.local_addr().expect("local_addr").port()
}

#[test]
fn recv_bytes_preserves_nul_in_payload() {
    // The headline test. Rust server sends a binary payload that
    // includes a NUL byte at offset 3. Pre-2g recv_str would
    // truncate the resulting String at offset 3 and the Hale
    // side would see len=3 / "DEA". With recv_bytes the explicit
    // length comes back through the wire-shape (length-prefixed
    // blob) and the Hale side sees the full 8 bytes.
    let port = pick_free_port();
    let listener = TcpListener::bind(("127.0.0.1", port)).expect("bind");
    let payload: [u8; 8] = [0xDE, 0xAD, 0xBE, 0x00, 0x01, 0x02, 0x03, 0xFF];
    let (done_tx, done_rx) = mpsc::channel::<()>();
    thread::spawn(move || {
        let (mut sock, _) = listener.accept().expect("accept");
        sock.write_all(&payload).expect("write payload");
        // Hold the socket open briefly so the client's read sees
        // the whole payload in one read. Letting the listener drop
        // here would FIN immediately; recv would still see the
        // bytes but timing-dependent on the kernel's TCP buffer.
        let _ = done_rx.recv_timeout(std::time::Duration::from_secs(2));
        drop(sock);
    });

    let src = format!(
        r#"
        fn main() {{
            let fd = std::io::tcp::__connect("127.0.0.1", {});
            let s = std::io::tcp::Stream {{ conn_fd: fd }};
            let b = s.recv_bytes(32) or raise;
            println("len=", len(b));
            println("b0=", std::bytes::at(b, 0));
            println("b3=", std::bytes::at(b, 3));
            println("b7=", std::bytes::at(b, 7));
        }}
        "#,
        port
    );
    let bin = build_hale("nul_preserved", &src);
    let out = Command::new(&bin).output().expect("run hale");
    let _ = done_tx.send(());
    let _ = std::fs::remove_file(&bin);

    assert!(
        out.status.success(),
        "exit: {:?}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("len=8"), "got: {:?}", stdout);
    assert!(stdout.contains("b0=222"), "byte 0 = 0xDE; got: {:?}", stdout);
    assert!(stdout.contains("b3=0"), "byte 3 = NUL; got: {:?}", stdout);
    assert!(stdout.contains("b7=255"), "byte 7 = 0xFF; got: {:?}", stdout);
}

#[test]
fn from_string_round_trip_preserves_length() {
    // std::bytes::from_string(s) → Bytes whose length matches the
    // source string's strlen. std::str::from_bytes(b) → String that
    // re-reads to the same NUL-terminated form.
    let src = r#"
        fn main() {
            let s = "hello";
            let b = std::bytes::from_string(s);
            println("blen=", len(b));
            let back = std::str::from_bytes(b);
            println("back=", back);
            println("blen_again=", len(b));
        }
    "#;
    let bin = build_hale("conv_round_trip", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("blen=5"), "got: {:?}", stdout);
    assert!(stdout.contains("back=hello"), "got: {:?}", stdout);
    assert!(stdout.contains("blen_again=5"), "got: {:?}", stdout);
}

#[test]
fn bytes_at_out_of_range_returns_minus_one() {
    // Out-of-range index is a clean sentinel — bytes never go
    // negative on read, so -1 distinguishes "no byte at i" from
    // any valid value.
    let src = r#"
        fn main() {
            let b = std::bytes::from_string("ab");
            println("at0=", std::bytes::at(b, 0));
            println("at1=", std::bytes::at(b, 1));
            println("at2=", std::bytes::at(b, 2));
            println("atneg=", std::bytes::at(b, -1));
        }
    "#;
    let bin = build_hale("at_oob", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("at0=97"), "got: {:?}", stdout); // 'a'
    assert!(stdout.contains("at1=98"), "got: {:?}", stdout); // 'b'
    assert!(stdout.contains("at2=-1"), "out-of-range; got: {:?}", stdout);
    assert!(stdout.contains("atneg=-1"), "negative; got: {:?}", stdout);
}

#[test]
fn bytes_slice_returns_subrange() {
    // Half-open [lo, hi). Out-of-range bounds clamp; hi <= lo
    // yields empty. Result is a copy with its own length.
    let src = r#"
        fn main() {
            let b = std::bytes::from_string("abcdef");
            let mid = std::bytes::slice(b, 1, 4);
            println("midlen=", len(mid));
            println("mid0=", std::bytes::at(mid, 0));
            println("mid2=", std::bytes::at(mid, 2));
            let clamped = std::bytes::slice(b, 4, 100);
            println("clamp_len=", len(clamped));
            let empty = std::bytes::slice(b, 3, 3);
            println("empty_len=", len(empty));
        }
    "#;
    let bin = build_hale("slice", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("midlen=3"), "got: {:?}", stdout);
    assert!(stdout.contains("mid0=98"), "got: {:?}", stdout); // 'b'
    assert!(stdout.contains("mid2=100"), "got: {:?}", stdout); // 'd'
    assert!(stdout.contains("clamp_len=2"), "got: {:?}", stdout); // "ef"
    assert!(stdout.contains("empty_len=0"), "got: {:?}", stdout);
}

#[test]
fn recv_bytes_on_eof_returns_zero_len() {
    // Peer closes immediately after accept. recv_bytes should see
    // EOF (read() returns 0) and return an empty (len=0) Bytes
    // rather than blocking or surfacing an error.
    let port = pick_free_port();
    let listener = TcpListener::bind(("127.0.0.1", port)).expect("bind");
    thread::spawn(move || {
        let (sock, _) = listener.accept().expect("accept");
        // Immediate drop — peer sees clean EOF.
        drop(sock);
    });

    let src = format!(
        r#"
        fn main() {{
            let fd = std::io::tcp::__connect("127.0.0.1", {});
            let s = std::io::tcp::Stream {{ conn_fd: fd }};
            let b = s.recv_bytes(64) or raise;
            println("len=", len(b));
        }}
        "#,
        port
    );
    let bin = build_hale("eof", &src);
    let out = Command::new(&bin).output().expect("run hale");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("len=0"), "got: {:?}", stdout);
}

#[test]
fn from_string_then_send_bytes_round_trips_via_tcp() {
    // End-to-end: convert a String to Bytes, ship over TCP via
    // send_bytes, server reads the full payload. Demonstrates
    // the conversion + send_bytes pair as the natural shape for
    // shipping text payloads through the binary-safe surface
    // (e.g. when the protocol layer wants explicit length on the
    // wire).
    let port = pick_free_port();
    let listener = TcpListener::bind(("127.0.0.1", port)).expect("bind");
    let (got_tx, got_rx) = mpsc::channel::<Vec<u8>>();
    thread::spawn(move || {
        let (mut sock, _) = listener.accept().expect("accept");
        let mut buf = Vec::new();
        let _ = sock.read_to_end(&mut buf);
        let _ = got_tx.send(buf);
    });

    let src = format!(
        r#"
        fn main() {{
            let fd = std::io::tcp::__connect("127.0.0.1", {});
            let s = std::io::tcp::Stream {{ conn_fd: fd }};
            let b = std::bytes::from_string("hello world");
            s.send_bytes(b) or raise;
        }}
        "#,
        port
    );
    let bin = build_hale("from_string_send", &src);
    let out = Command::new(&bin).output().expect("run hale");
    let received = got_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("server read");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(received, b"hello world");
}

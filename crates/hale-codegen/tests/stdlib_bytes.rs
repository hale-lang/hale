//! m89: Bytes codegen + std::io::fs::read_bytes +
//! Stream.send_bytes.
//!
//! Bytes is the binary-safe sibling of String. Same single-
//! pointer ABI; underlying blob is `[i64 len][u8 data[len]]`
//! so embedded NUL bytes don't truncate. These tests verify:
//!
//! - `len(bytes)` reads the explicit length prefix.
//! - `read_bytes(path)` round-trips file contents that contain
//!   embedded NULs (something `read_file` would silently
//!   truncate).
//! - `Stream.send_bytes(b)` ships the full payload over TCP
//!   regardless of NUL bytes in the body.
//! - println on a Bytes value prints `<bytes len=N>` rather
//!   than dumping potentially-binary content.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use hale_codegen::build_executable;

fn build_hale(name: &str, source: &str) -> std::path::PathBuf {
    let program = hale_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_test_bytes_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

fn unique_path(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "hale_bytes_{}_{}_{}.tmp",
        tag,
        std::process::id(),
        nanos
    ));
    p
}

fn pick_free_port() -> u16 {
    let probe = TcpListener::bind("127.0.0.1:0").expect("bind probe");
    probe.local_addr().expect("local_addr").port()
}

#[test]
fn read_bytes_returns_full_length_with_embedded_nuls() {
    // Write a file whose contents contain NUL bytes. read_file
    // (String) would truncate at the first NUL; read_bytes
    // must preserve all 10 bytes.
    let path = unique_path("with_nuls");
    let payload: [u8; 10] = [0xDE, 0xAD, 0x00, 0xBE, 0xEF, 0x00, 0x01, 0x02, 0x03, 0xFF];
    std::fs::write(&path, payload).expect("write input");

    let src = format!(
        r#"
        fn main() {{
            let b = std::io::fs::read_bytes("{}");
            println("len=", len(b));
        }}
        "#,
        path.display()
    );
    let bin = build_hale("read_with_nuls", &src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(&path);

    assert!(
        out.status.success(),
        "non-zero: {:?}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("len=10"),
        "expected len=10 (binary-safe count); got: {:?}",
        stdout
    );
}

#[test]
fn read_bytes_on_missing_file_returns_zero_len() {
    // lotus_fs_read_bytes_global returns NULL on open failure;
    // lotus_bytes_len handles NULL by returning 0. So a missing
    // file gives a Bytes value whose len() is 0 — same
    // soft-failure shape as read_file, no exception path.
    let src = r#"
        fn main() {
            let b = std::io::fs::read_bytes("/tmp/hale_definitely_does_not_exist_xyz123.bin");
            println("len=", len(b));
        }
    "#;
    let bin = build_hale("missing_file", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("len=0"), "got: {:?}", stdout);
}

#[test]
fn println_of_bytes_prints_summary_not_body() {
    // Bytes println shape is `<bytes len=N>` so logs stay
    // readable even when the body would dump unprintable
    // content.
    let path = unique_path("println");
    std::fs::write(&path, b"abc\x00def").expect("write input");
    let src = format!(
        r#"
        fn main() {{
            let b = std::io::fs::read_bytes("{}");
            println("got=", b);
        }}
        "#,
        path.display()
    );
    let bin = build_hale("println_summary", &src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(&path);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("got=<bytes len=7>"),
        "got: {:?}",
        stdout
    );
}

#[test]
fn stream_send_bytes_ships_full_body_through_tcp() {
    // The whole point of Bytes existing: send a binary payload
    // through Stream without NUL truncation. Server is a Rust
    // TcpListener that reads everything to EOF and verifies the
    // full byte sequence.
    let path = unique_path("send_input");
    let payload: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x00, 0x00, 0x00, 0x0D]; // PNG header-ish bytes with embedded NULs
    std::fs::write(&path, payload).expect("write payload");

    let port = pick_free_port();
    let listener = TcpListener::bind(("127.0.0.1", port)).expect("bind");
    let (server_done_tx, server_done_rx) = mpsc::channel::<Vec<u8>>();
    thread::spawn(move || {
        let (mut sock, _) = listener.accept().expect("accept");
        let mut buf = Vec::new();
        let _ = sock.read_to_end(&mut buf);
        let _ = server_done_tx.send(buf);
    });

    let src = format!(
        r#"
        fn main() {{
            let fd = std::io::tcp::__connect("127.0.0.1", {});
            let s = std::io::tcp::Stream {{ conn_fd: fd }};
            let body = std::io::fs::read_bytes("{}");
            s.send_bytes(body) or raise;
        }}
        "#,
        port,
        path.display()
    );
    let bin = build_hale("send_bytes", &src);
    let out = Command::new(&bin).output().expect("run hale");
    let _ = std::fs::remove_file(&bin);
    let received = server_done_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("server read");
    let _ = std::fs::remove_file(&path);

    assert!(
        out.status.success(),
        "non-zero: {:?}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        received,
        payload,
        "server didn't see the full binary payload (NUL truncation?)"
    );
}

#[test]
fn bytes_round_trips_through_helper_fn() {
    // Returning Bytes from a fn must keep the value valid for
    // the caller. Lifetime story: read_bytes anchors in the
    // global payload arena, so the returned ptr stays live
    // through the rest of main.
    let path = unique_path("round_trip");
    std::fs::write(&path, b"hello\x00world").expect("write");
    let src = format!(
        r#"
        fn load(p: String) -> Bytes {{
            return std::io::fs::read_bytes(p);
        }}

        fn main() {{
            let b = load("{}");
            println("len=", len(b));
        }}
        "#,
        path.display()
    );
    let bin = build_hale("round_trip_fn", &src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(&path);
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("len=11"), "got: {:?}", stdout);
}

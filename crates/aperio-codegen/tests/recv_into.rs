//! Phase 1: caller-provided destination at the syscall layer.
//! `std::io::tcp::recv_into(fd, buf, max_bytes)` reads directly
//! into a builder handle instead of allocating a fresh Bytes
//! blob in g_bus_payload_arena per call. Closes the recv-loop
//! leak that pond/websocket flagged.

use std::io::Write;
use std::process::{Command, Stdio};

use aperio_codegen::build_executable;

fn pick_free_port() -> u16 {
    let probe = std::net::TcpListener::bind("127.0.0.1:0").expect("bind probe");
    let port = probe.local_addr().expect("local_addr").port();
    drop(probe);
    port
}

fn build_aperio_binary(name: &str, source: &str) -> std::path::PathBuf {
    let program = aperio_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("aperio_test_recv_into_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

#[test]
fn tcp_recv_into_accumulates_across_calls() {
    // A small server binary that:
    //   1. Binds on a chosen port
    //   2. Accepts one client
    //   3. Reads two chunks via recv_into into ONE builder
    //   4. Prints the accumulated snapshot + length per step
    //   5. Frees the builder + closes the conn
    //
    // The test process plays the client, writing two distinct
    // chunks with a tiny sleep between them so the server sees
    // two separate read()s (not a coalesced single one). The
    // assertion is that the builder's accumulated contents match
    // both chunks joined — proving recv_into appends to the
    // existing buffer rather than replacing.
    let port = pick_free_port();
    let source = format!(
        r#"
        fn main() {{
            let listen = std::io::tcp::listen_socket("127.0.0.1", {}) or raise;
            let conn = std::io::tcp::accept_one(listen) or raise;
            let buf = std::bytes::BytesBuilder {{ initial_cap: 1024 }};
            let n1 = std::io::tcp::recv_into(conn, buf, 1024);
            println("after_first len=", buf.len(), " n=", n1);
            let snap1 = buf.snapshot();
            println("snap1=", std::str::from_bytes(snap1));
            let n2 = std::io::tcp::recv_into(conn, buf, 1024);
            println("after_second len=", buf.len(), " n=", n2);
            let snap2 = buf.snapshot();
            println("snap2=", std::str::from_bytes(snap2));
            std::io::tcp::close_fd(conn);
            std::io::tcp::close_fd(listen);
        }}
    "#,
        port
    );
    let bin = build_aperio_binary("tcp_accumulates", &source);

    let server_proc = Command::new(&bin)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn server");

    // Race: connect once the server is ready.
    let mut stream = None;
    for _ in 0..50 {
        match std::net::TcpStream::connect(("127.0.0.1", port)) {
            Ok(s) => {
                stream = Some(s);
                break;
            }
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(20)),
        }
    }
    let mut s = stream.expect("connect");
    s.write_all(b"hello").expect("write 1");
    s.flush().expect("flush 1");
    // Small pause so the server sees two distinct read()s.
    std::thread::sleep(std::time::Duration::from_millis(80));
    s.write_all(b" world").expect("write 2");
    s.flush().expect("flush 2");
    drop(s);

    let out = server_proc.wait_with_output().expect("server wait");
    let _ = std::fs::remove_file(&bin);

    assert!(
        out.status.success(),
        "non-zero: {:?}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("after_first len=5 n=5"), "got: {:?}", stdout);
    assert!(stdout.contains("snap1=hello"), "got: {:?}", stdout);
    assert!(
        stdout.contains("after_second len=11 n=6"),
        "got: {:?}",
        stdout
    );
    assert!(stdout.contains("snap2=hello world"), "got: {:?}", stdout);
}

#[test]
fn tcp_recv_into_zero_on_peer_close() {
    // The receiver's third recv_into after the peer disconnects
    // should return 0 (clean EOF), not error. Builder unchanged
    // on the EOF return.
    let port = pick_free_port();
    let source = format!(
        r#"
        fn main() {{
            let listen = std::io::tcp::listen_socket("127.0.0.1", {}) or raise;
            let conn = std::io::tcp::accept_one(listen) or raise;
            let buf = std::bytes::BytesBuilder {{ initial_cap: 1024 }};
            let n1 = std::io::tcp::recv_into(conn, buf, 1024);
            println("n1=", n1, " len=", buf.len());
            // Wait for the test process to close the socket.
            let n2 = std::io::tcp::recv_into(conn, buf, 1024);
            println("n2=", n2, " len=", buf.len());
            std::io::tcp::close_fd(conn);
            std::io::tcp::close_fd(listen);
        }}
    "#,
        port
    );
    let bin = build_aperio_binary("tcp_eof", &source);

    let server_proc = Command::new(&bin)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn server");

    let mut stream = None;
    for _ in 0..50 {
        match std::net::TcpStream::connect(("127.0.0.1", port)) {
            Ok(s) => {
                stream = Some(s);
                break;
            }
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(20)),
        }
    }
    let mut s = stream.expect("connect");
    s.write_all(b"bye").expect("write");
    s.flush().expect("flush");
    // Half-close write side so the server's second read returns 0.
    drop(s);

    let out = server_proc.wait_with_output().expect("server wait");
    let _ = std::fs::remove_file(&bin);

    assert!(
        out.status.success(),
        "non-zero: {:?}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("n1=3 len=3"), "got: {:?}", stdout);
    assert!(stdout.contains("n2=0 len=3"), "got: {:?}", stdout);
}

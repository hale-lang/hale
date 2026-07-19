//! std::http connection takeover (2026-07-19) — the Upgrade
//! surface that unblocks WebSocket-class protocols. A Handler
//! reads `req.conn_fd`, answers `Response { takeover: true }`
//! (status line + its own headers only — no Content-Length, no
//! Connection: close, no body), and the Server returns without
//! closing the fd: `Stream.release_fd()` disarms the
//! per-connection scope close. The handler owns the live
//! connection from that moment.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::Command;
use std::thread;
use std::time::Duration;

use hale_codegen::build_executable;

fn pick_free_port() -> u16 {
    let probe = std::net::TcpListener::bind("127.0.0.1:0").expect("bind probe");
    probe.local_addr().expect("local_addr").port()
}

#[test]
fn takeover_answers_101_and_keeps_the_connection_live() {
    let port = pick_free_port();
    let src = format!(
        r#"
        locus TakeoverHandler {{
            params {{ taken_fd: Int = -1; }}
            fn handle(req: std::http::Request) -> std::http::Response {{
                self.taken_fd = req.conn_fd;
                return std::http::Response {{
                    status: 101,
                    headers: "Upgrade: echo\r\nConnection: Upgrade",
                    body: "",
                    takeover: true
                }};
            }}
        }}
        fn main() {{
            let h = TakeoverHandler {{ }};
            std::http::Server {{
                // 2 accepts: the test's readiness probe consumes
                // one (connect-and-drop), the real client the other.
                port: {port}, max_accepts: 2, ready_signal: "READY",
                handler: h
            }};
            // The accept loop is done; the connection is still
            // live and stashed on the handler. Drive it raw.
            let s = std::io::tcp::Stream {{ conn_fd: h.taken_fd }};
            let ping = s.recv(64) or "";
            s.send("echo:" + ping) or discard;
        }}
    "#
    );
    let program = hale_syntax::parse_source(&src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_http_upgrade_{}", std::process::id()));
    build_executable(&program, &bin).expect("build");
    let mut child = Command::new(&bin)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("spawn server");

    let mut ready = false;
    for _ in 0..50 {
        thread::sleep(Duration::from_millis(100));
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            ready = true;
            break;
        }
    }
    assert!(ready, "server never started listening");

    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    s.set_read_timeout(Some(Duration::from_secs(5))).ok();
    write!(
        s,
        "GET /ws HTTP/1.1\r\nHost: t\r\nUpgrade: echo\r\nConnection: Upgrade\r\n\r\n"
    )
    .expect("send request");

    // Read exactly the 101 header block (ends at CRLFCRLF) — the
    // stream stays open, so read_to_string would hang.
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        let n = s.read(&mut byte).expect("read 101");
        assert!(n > 0, "connection closed before 101 completed");
        head.push(byte[0]);
        assert!(head.len() < 4096, "runaway header block");
    }
    let head = String::from_utf8_lossy(&head);
    assert!(head.starts_with("HTTP/1.1 101 Switching Protocols"), "{}", head);
    assert!(head.contains("Upgrade: echo"), "{}", head);
    assert!(head.contains("Connection: Upgrade"), "{}", head);
    // Takeover responses carry none of the normal-path framing.
    assert!(!head.contains("Content-Length"), "{}", head);
    assert!(!head.contains("Connection: close"), "{}", head);

    // Same connection, raw bytes now.
    write!(s, "ping-payload").expect("send ping");
    let mut buf = [0u8; 64];
    let n = s.read(&mut buf).expect("read echo");
    let echo = String::from_utf8_lossy(&buf[..n]);
    assert_eq!(echo, "echo:ping-payload");

    let _ = child.wait();
    let _ = std::fs::remove_file(&bin);
}

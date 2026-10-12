//! F.35 Slice 3: end-to-end park/resume test.
//!
//! Verifies that a locus running on an `async_io` pool can hold
//! multiple concurrent connections without blocking the OS thread.
//! Pre-F.35, the pool would serialize: one connection at a time
//! per worker thread, with new connections queued behind the
//! current one's blocking read. With F.35, recv_bytes parks on
//! EAGAIN; the pool drain advances other cells; epoll wakes the
//! parked coro when data arrives.
//!
//! Test shape: a main locus with two parallel TCP listeners on
//! separate fds, both placed on a single-pool async_io cooperative
//! pool. We connect to both and verify both connections receive
//! their response — which is impossible if the pool serializes
//! reads (the first listener's accept-loop would block the worker
//! thread, the second listener would never accept). Smoke-level
//! validation that async_io is actually multiplexing.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::Command;
use std::thread;
use std::time::Duration;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;
#[path = "support/child.rs"]
mod child;

#[test]
fn async_io_pool_multiplexes_two_listeners() {
    // The two listeners bind port 0, each on a loopback address of its
    // own, and the test reads back the port the kernel gave each: a port
    // drawn here and handed over is free, not reserved.
    // A minimal program: two listeners on the same async_io pool,
    // each accepts ONE connection, reads a request line, writes a
    // tagged response, then exits.
    //
    // If async_io works: both listeners can be in accept() at once
    // (one parked, the other running). Connecting to either fires
    // the appropriate one. We connect to BOTH, sequentially, and
    // both clients receive their tagged response.
    //
    // If async_io doesn't work (pre-F.35 shape or Slice 3 not
    // wired): only one listener can be in accept() at a time —
    // we'd see the first connection succeed but the second hang
    // until the first completes; the test would time out before
    // both clients get responses.
    let src = format!(
        r#"
        fn handle_a(s: std::io::tcp::Stream) {{
            let _req = s.recv_bytes(64) or raise;
            let resp = std::bytes::from_string("A-OK\n");
            s.send_bytes(resp) or raise;
        }}
        fn handle_b(s: std::io::tcp::Stream) {{
            let _req = s.recv_bytes(64) or raise;
            let resp = std::bytes::from_string("B-OK\n");
            s.send_bytes(resp) or raise;
        }}

        main locus App {{
            params {{
                la: std::io::tcp::Listener = std::io::tcp::Listener {{
                    host:         "127.0.0.1",
                    port:         0,
                    max_accepts:  1,
                    on_connection: handle_a,
                }};
                lb: std::io::tcp::Listener = std::io::tcp::Listener {{
                    host:         "127.0.0.2",
                    port:         0,
                    max_accepts:  1,
                    on_connection: handle_b,
                }};
            }}
            placement {{
                la: cooperative(pool = io) where async_io;
                lb: cooperative(pool = io) where async_io;
            }}
            run() {{
                // Keep main alive long enough for both clients to
                // connect. The listeners on the io pool run their
                // accept loops independently via async_io park —
                // pre-F.35 they would have serialized on the worker
                // thread and the second connect would hang.
                std::time::sleep(2s);
            }}
        }}

        fn main() {{
            App {{ }};
        }}
    "#
    );
    let bin = harness::unique_bin("hale_test_async_io_park_resume");
    build_opts::build_source(&src, &bin, &build_opts::options()).expect("build");
    // Spawn the binary in the background.
    let mut child = Command::new(&bin)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn");
    // Both listen once the kernel has given each its port.
    let port_a = child::tcp_port_on(&mut child, std::net::Ipv4Addr::new(127, 0, 0, 1));
    let port_b = child::tcp_port_on(&mut child, std::net::Ipv4Addr::new(127, 0, 0, 2));
    // Connect to A and B. Each connection: send a byte, read the
    // response. With async_io, both should succeed.
    let connect = |host: &str, port: u16, send_byte: u8| -> Vec<u8> {
        let mut s = TcpStream::connect((host, port)).unwrap_or_else(|e| panic!("connect {host}:{port}: {e:?}"));
        s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        s.write_all(&[send_byte]).expect("write");
        let mut buf = vec![0u8; 16];
        let n = s.read(&mut buf).unwrap_or(0);
        buf.truncate(n);
        buf
    };
    let resp_a = connect("127.0.0.1", port_a, b'a');
    let resp_b = connect("127.0.0.2", port_b, b'b');
    // Give the binary a moment to finish its accept counters and exit.
    thread::sleep(Duration::from_millis(100));
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_file(&bin);
    assert!(
        resp_a.starts_with(b"A-OK"),
        "listener A didn't respond: got {:?}",
        resp_a
    );
    assert!(
        resp_b.starts_with(b"B-OK"),
        "listener B didn't respond (async_io pool likely serialized \
         the accept-loops instead of multiplexing them): got {:?}",
        resp_b
    );
}

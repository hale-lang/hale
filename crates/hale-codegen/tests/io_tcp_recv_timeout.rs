//! 2026-05-27 — `std::io::tcp::set_recv_timeout(fd, d)` and
//! `set_send_timeout(fd, d)`. Mirror the udp P4 surface so
//! recv loops over TCP can do periodic work on a quiet
//! connection (silence detection, heartbeats, watchdog
//! timers). Wraps `SO_RCVTIMEO` / `SO_SNDTIMEO` via the
//! shared fd-generic `sock_set_timeout_ns` helper in the C
//! runtime.
//!
//! Test shape: connect to a listening socket that never
//! sends, set a short recv timeout, observe that the recv
//! returns within a bounded window instead of blocking
//! forever.

use std::process::Command;

use hale_codegen::build_executable;

fn build_and_run(name: &str, src: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_test_io_tcp_recv_timeout_{}", name));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status,
    )
}

#[test]
fn tcp_recv_blocks_under_timeout_returns_promptly() {
    // The .hl program:
    //   1. opens a listen socket on an ephemeral port
    //   2. connects to itself (loopback)
    //   3. accepts the connection
    //   4. sets a 100ms recv timeout on the client fd
    //   5. calls recv_bytes (no data will ever arrive — server side never sends)
    //   6. prints the elapsed milliseconds + whatever the recv returned
    //
    // Without the timeout, step 5 would block forever. With
    // the timeout, recv_bytes returns within ~100ms (zero
    // bytes on Linux EAGAIN) and the loop prints + exits.
    let src = r#"
        fn main() {
            let listen_fd = std::io::tcp::listen_socket("127.0.0.1", 47831) or raise;
            let client_fd = std::io::tcp::connect("127.0.0.1", 47831) or raise;
            let _peer_fd  = std::io::tcp::accept_one(listen_fd) or raise;

            // Set 100ms recv timeout; without this the next
            // recv_bytes blocks forever.
            std::io::tcp::set_recv_timeout(client_fd, 100ms) or raise;

            let client = std::io::tcp::Stream { conn_fd: client_fd };
            let start  = std::time::monotonic_ns();
            let got    = client.recv_bytes(16);
            let elapsed_ms = (std::time::monotonic_ns() - start) / 1000000;
            println("elapsed_ms=", elapsed_ms);
            println("got_len=", len(got));
        }
    "#;
    let (out, status) = build_and_run("recv_timeout", src);
    assert!(status.success(), "non-zero: {:?}\nstdout: {}", status, out);

    // Parse elapsed_ms — must be roughly in the [80, 1500] ms
    // window. Lower bound covers a slight kernel-side
    // discretization; upper bound generous for slow CI hosts
    // while still well under any realistic block-forever
    // failure mode.
    let line = out
        .lines()
        .find(|l| l.starts_with("elapsed_ms="))
        .unwrap_or_else(|| panic!("no elapsed_ms in:\n{}", out));
    let ms: i64 = line.trim_start_matches("elapsed_ms=").trim().parse()
        .unwrap_or_else(|e| panic!("parse elapsed_ms: {} from {:?}", e, line));
    assert!(
        ms >= 80 && ms <= 1500,
        "recv should return within ~100ms of the timeout; got {} ms\n{}",
        ms, out
    );
}

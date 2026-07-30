//! 2026-05-26 — UDP P4: `recv_with_source` + thread-local
//! `last_source_*` getters, plus `set_recv_timeout` /
//! `set_send_timeout` (Duration-based, can't ride the
//! `set_option_int` pass-through because SO_RCVTIMEO takes a
//! struct timeval, not an int).
//!
//! Each test uses loopback so they're self-contained — no
//! external rig.

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn unique_path(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let p = harness::unique_bin(&format!(
        "lt-udp-p4-{}-{}-{}.bin",
        tag,
        std::process::id(),
        nanos,
    ));
    p
}

fn build_and_run(name: &str, src: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = unique_path(name);
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status,
    )
}

#[test]
fn recv_with_source_captures_loopback_sender() {
    // Receiver binds to a fixed port; sender uses an ephemeral
    // port (the kernel picks). After recv_with_source returns,
    // last_source_host = 127.0.0.1 and last_source_port = the
    // sender's ephemeral port.
    let src = r#"
        fn main() {
            let rx = std::io::udp::bind("127.0.0.1", 0) or raise;
            // Read back the bound port so the sender knows where
            // to send.
            let rx_port = std::io::udp::get_option_int(
                rx,
                std::io::sockopt::SOL_SOCKET(),
                std::io::sockopt::SO_RCVBUF()
            ) or raise;
            // (the get_option_int above is just exercising the
            // pass-through; the actual port comes from a
            // hardcoded bind below since std::io::udp doesn't
            // yet expose getsockname.)
            // For this test we'll bind to a fixed port; if it's
            // in use, the test is racy but rerunning fixes it.
            let _ = std::io::udp::close(rx);

            let rx = std::io::udp::bind("127.0.0.1", 56821) or raise;
            std::io::udp::set_recv_timeout(rx, 500ms) or raise;
            let tx = std::io::udp::bind("0.0.0.0", 0) or raise;
            std::io::udp::send(tx, "127.0.0.1", 56821, "hi") or raise;
            let data = std::io::udp::recv_with_source(rx, 1500) or raise;
            let host = std::io::udp::last_source_host();
            let port = std::io::udp::last_source_port();
            print("len="); println(len(data));
            print("host="); println(host);
            // Don't print the exact port — it's ephemeral —
            // just check it's > 0.
            if port <= 0 { println("FAIL: port not captured"); }
            else { println("port_captured"); }
            let _ = std::io::udp::close(rx);
            let _ = std::io::udp::close(tx);
        }
    "#;
    let (stdout, status) = build_and_run("recv_source", src);
    assert!(status.success(), "non-zero exit: {:?}\nstdout: {}", status, stdout);
    assert!(stdout.contains("len=2"), "expected len=2 (hi); got: {:?}", stdout);
    assert!(stdout.contains("host=127.0.0.1"), "got: {:?}", stdout);
    assert!(stdout.contains("port_captured"), "got: {:?}", stdout);
    assert!(!stdout.contains("FAIL"), "stdout: {:?}", stdout);
}

#[test]
fn last_source_returns_zero_on_no_recv() {
    // Calling last_source_* without a prior recv_with_source
    // returns the documented zero-state (host="0.0.0.0",
    // port=0).
    let src = r#"
        fn main() {
            let host = std::io::udp::last_source_host();
            let port = std::io::udp::last_source_port();
            print("host="); println(host);
            print("port="); println(port);
        }
    "#;
    let (stdout, status) = build_and_run("zero_state", src);
    assert!(status.success(), "non-zero exit: {:?}\nstdout: {}", status, stdout);
    assert!(stdout.contains("host=0.0.0.0"), "got: {:?}", stdout);
    assert!(stdout.contains("port=0"), "got: {:?}", stdout);
}

#[test]
fn set_recv_timeout_fires_on_silent_socket() {
    // 50ms recv timeout. Nobody sends — recv_with_source returns
    // err, and last_source_* are reset to their zero-state.
    let src = r#"
        fn handle(_e: IoError) -> Bytes {
            println("recv_timed_out");
            return std::bytes::from_string("");
        }
        fn main() {
            let rx = std::io::udp::bind("127.0.0.1", 0) or raise;
            std::io::udp::set_recv_timeout(rx, 50ms) or raise;
            let _bytes = std::io::udp::recv_with_source(rx, 1500) or handle(err);
            let host = std::io::udp::last_source_host();
            let port = std::io::udp::last_source_port();
            print("host="); println(host);
            print("port="); println(port);
            let _ = std::io::udp::close(rx);
        }
    "#;
    let (stdout, status) = build_and_run("recv_timeout", src);
    assert!(status.success(), "non-zero exit: {:?}\nstdout: {}", status, stdout);
    assert!(stdout.contains("recv_timed_out"), "got: {:?}", stdout);
    // Post-error: source TLS reset.
    assert!(stdout.contains("host=0.0.0.0"), "got: {:?}", stdout);
    assert!(stdout.contains("port=0"), "got: {:?}", stdout);
}

#[test]
fn set_send_timeout_round_trips() {
    // SO_SNDTIMEO on a UDP socket isn't typically observable
    // (sendto blocks only when the kernel send buffer is full,
    // which doesn't happen on loopback at low rate), so we
    // can't easily provoke the timeout firing. Just verify the
    // setsockopt call succeeds.
    let src = r#"
        fn main() {
            let tx = std::io::udp::bind("0.0.0.0", 0) or raise;
            std::io::udp::set_send_timeout(tx, 200ms) or raise;
            println("send_timeout_ok");
            let _ = std::io::udp::close(tx);
        }
    "#;
    let (stdout, status) = build_and_run("send_timeout", src);
    assert!(status.success(), "non-zero exit: {:?}\nstdout: {}", status, stdout);
    assert!(stdout.contains("send_timeout_ok"), "got: {:?}", stdout);
}

//! #209 — `Stream.send` / `send_bytes` / `recv` / `recv_bytes` are
//! `fallible(IoError)`. The contract pinned here:
//!   - a genuine I/O error (bad fd, peer reset / broken pipe) FAILS
//!     with a real IoError carrying kind + errno;
//!   - clean EOF is NOT an error — recv returns empty;
//!   - a `set_recv_timeout` expiry is NOT an error — recv returns
//!     empty (timeout stays a liveness signal);
//!   - send/send_bytes succeed with Unit, so `or discard` works.

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use hale_codegen::build_executable;

fn pick_free_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind probe");
    l.local_addr().expect("local_addr").port()
}

fn unique_path(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "hale-stream-fallible-{}-{}-{}.bin",
        tag,
        std::process::id(),
        nanos,
    ));
    p
}

fn build_and_run(
    name: &str,
    src: &str,
    argv: &[&str],
) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = unique_path(name);
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).args(argv).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (String::from_utf8_lossy(&out.stdout).to_string(), out.status)
}

#[test]
fn bad_fd_send_and_recv_fail_with_io_error() {
    let src = r#"
        fn on_send_err(e: IoError) {
            println("send-err errno=", e.errno, " kind=", e.kind);
        }
        fn on_recv_err(e: IoError) -> String {
            println("recv-err errno=", e.errno);
            return "FELL-BACK";
        }
        fn main() {
            let s = std::io::tcp::Stream { conn_fd: 0 - 1, owns_fd: false };
            s.send("x") or on_send_err(err);
            let got = s.recv(64) or on_recv_err(err);
            println("got=[", got, "]");
        }
    "#;
    let (out, status) = build_and_run("bad_fd", src, &[]);
    assert!(status.success(), "non-zero: {:?}\nstdout: {}", status, out);
    // EBADF = 9 on Linux.
    assert!(out.contains("send-err errno=9"), "stdout: {}", out);
    assert!(out.contains("recv-err errno=9"), "stdout: {}", out);
    assert!(out.contains("got=[FELL-BACK]"), "stdout: {}", out);
}

#[test]
fn eof_is_empty_success_not_error() {
    // Peer closes cleanly: recv must return "" WITHOUT taking the
    // error path — `or "ERR"` must not fire.
    let port = pick_free_port();
    let src = r#"
        fn main() {
            let port = std::str::parse_int(std::env::arg(1)) or raise;
            let lfd = std::io::tcp::__listen_socket("127.0.0.1", port);
            let cfd = std::io::tcp::__connect("127.0.0.1", port);
            let afd = std::io::tcp::__accept_one(lfd);
            std::io::tcp::__close_fd(afd);
            let c = std::io::tcp::Stream { conn_fd: cfd, owns_fd: false };
            let at_eof = c.recv(64) or "ERR";
            println("eof=[", at_eof, "] len=", len(at_eof));
            let b = c.recv_bytes(64) or std::bytes::from_string("ERR");
            println("eof_bytes_len=", len(b));
            std::io::tcp::__close_fd(cfd);
        }
    "#;
    let (out, status) = build_and_run("eof", src, &[&port.to_string()]);
    assert!(status.success(), "non-zero: {:?}\nstdout: {}", status, out);
    assert!(out.contains("eof=[] len=0"), "EOF must be empty success: {}", out);
    assert!(out.contains("eof_bytes_len=0"), "stdout: {}", out);
}

#[test]
fn recv_timeout_is_empty_success_not_error() {
    // Silent peer + 200ms SO_RCVTIMEO: recv returns "" at ~deadline
    // without raising — the timeout keeps its liveness-signal shape.
    let port = pick_free_port();
    let src = r#"
        fn main() {
            let port = std::str::parse_int(std::env::arg(1)) or raise;
            let lfd = std::io::tcp::__listen_socket("127.0.0.1", port);
            let cfd = std::io::tcp::__connect("127.0.0.1", port);
            let _afd = std::io::tcp::__accept_one(lfd);
            std::io::tcp::set_recv_timeout(cfd, 200ms) or raise;
            let c = std::io::tcp::Stream { conn_fd: cfd, owns_fd: false };
            let got = c.recv(64) or raise;
            println("timeout_len=", len(got));
        }
    "#;
    let (out, status) = build_and_run("timeout", src, &[&port.to_string()]);
    assert!(status.success(), "non-zero: {:?}\nstdout: {}", status, out);
    assert!(out.contains("timeout_len=0"), "stdout: {}", out);
}

#[test]
fn send_to_closed_peer_eventually_breaks_pipe() {
    // First send may land in the kernel buffer before the RST is
    // observed (discarded); the second write hits EPIPE/ECONNRESET
    // and must fail with the mapped kind.
    let port = pick_free_port();
    let src = r#"
        fn on_err(e: IoError) {
            println("late-send kind=", e.kind);
        }
        fn main() {
            let port = std::str::parse_int(std::env::arg(1)) or raise;
            let lfd = std::io::tcp::__listen_socket("127.0.0.1", port);
            let cfd = std::io::tcp::__connect("127.0.0.1", port);
            let afd = std::io::tcp::__accept_one(lfd);
            std::io::tcp::__close_fd(afd);
            let c = std::io::tcp::Stream { conn_fd: cfd, owns_fd: false };
            c.send("a") or discard;
            c.send("b") or on_err(err);
            println("done");
            std::io::tcp::__close_fd(cfd);
        }
    "#;
    let (out, status) = build_and_run("broken_pipe", src, &[&port.to_string()]);
    assert!(status.success(), "non-zero: {:?}\nstdout: {}", status, out);
    assert!(
        out.contains("late-send kind=broken_pipe")
            || out.contains("late-send kind=connection_reset"),
        "expected a broken_pipe/connection_reset failure: {}",
        out
    );
    assert!(out.contains("done"), "stdout: {}", out);
}

#[test]
fn or_discard_works_on_unit_success_send() {
    // send's Unit success makes `or discard` the fire-and-forget
    // disposition — even on a guaranteed-failing fd.
    let src = r#"
        fn main() {
            let s = std::io::tcp::Stream { conn_fd: 0 - 1, owns_fd: false };
            s.send("x") or discard;
            s.send_bytes(std::bytes::from_string("y")) or discard;
            println("survived");
        }
    "#;
    let (out, status) = build_and_run("discard", src, &[]);
    assert!(status.success(), "non-zero: {:?}\nstdout: {}", status, out);
    assert!(out.contains("survived"), "stdout: {}", out);
}

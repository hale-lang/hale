//! downstream handoff 2026-07-14 finding 1 (P1): on a `where async_io`
//! pool, `recv_into` returned the `-2` retryable sentinel instantly
//! on EAGAIN (the pool's fds are nonblocking) instead of parking the
//! coroutine — so pond/websocket's `-2`-based liveness declared
//! every idle connection dead within microseconds. `recv_into` (and
//! `recv_stamped` / udp / tls siblings) now do a TIMED PARK: park on
//! EPOLLIN until readable, or until the fd's `set_recv_timeout`
//! deadline expires — `-2` again means "deadline expired", on every
//! pool type. `recv_bytes` gains the deadline for free (it parked
//! indefinitely before, silently ignoring `set_recv_timeout`).

use std::io::Write;
use std::net::TcpStream;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;
#[path = "support/child.rs"]
mod child;

fn build(name: &str, src: &str) -> std::path::PathBuf {
    let bin = harness::unique_bin(&format!(
        "hale_async_recv_into_park_{}_{}",
        name,
        std::process::id()
    ));
    build_opts::build_source(src, &bin, &build_opts::options()).expect("build");
    bin
}

/// A listener on an async_io pool whose connection handler does ONE
/// `recv_into` and reports the result. `timeout_ms` argv arms
/// `set_recv_timeout` on the conn fd first (0 = leave unset). It binds
/// port 0 and the test reads back the port the kernel gave it
/// ([`child::tcp_port`]): a port drawn here and handed over is free, not
/// reserved, and another test of a parallel run can bind it first.
fn server_src() -> String {
    format!(
        r#"
        fn handle(s: std::io::tcp::Stream) {{
            let tmo = std::str::parse_int(std::env::arg(2)) or 0;
            if tmo > 0 {{
                let _ = std::io::tcp::__set_recv_timeout_ns(
                    s.conn_fd, tmo * 1000000);
            }}
            let b = std::bytes::BytesBuilder {{ }};
            let got = std::io::tcp::recv_into(s.conn_fd, b, 256);
            println("got=", got);
            let resp = std::bytes::from_string("DONE\n");
            s.send_bytes(resp) or raise;
        }}

        main locus App {{
            params {{
                l: std::io::tcp::Listener = std::io::tcp::Listener {{
                    host:          "127.0.0.1",
                    port:          0,
                    max_accepts:   1,
                    on_connection: handle,
                }};
            }}
            placement {{
                l: cooperative(pool = io) where async_io;
            }}
            run() {{
                std::time::sleep(4s);
            }}
        }}

        fn main() {{ App {{ }}; }}
    "#
    )
}

#[test]
fn recv_into_parks_until_readable_on_async_io() {
    // No recv timeout set: recv_into must park (indefinitely) until
    // the client's late bytes arrive — pre-fix it returned -2
    // instantly and the handler reported got=-2.
    let bin = build("parks", &server_src());
    let mut child = Command::new(&bin)
        .arg("0")
        .arg("0")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn");
    let mut s = connect(child::tcp_port(&mut child));
    // Idle for 300ms — the parked window — then send 5 bytes.
    thread::sleep(Duration::from_millis(300));
    s.write_all(b"hello").expect("write");
    let mut resp = Vec::new();
    s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    use std::io::Read;
    let _ = s.read_to_end(&mut resp);
    drop(s);
    let _ = child.kill();
    let out = child.wait_with_output().expect("wait");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("got=5"),
        "recv_into must park until the 5 late bytes arrive (pre-fix: \
         instant got=-2); stdout: {:?}",
        stdout
    );
}

#[test]
fn recv_into_honors_recv_deadline_on_async_io() {
    // 300ms recv timeout armed, client stays silent: recv_into must
    // return -2 AFTER ~300ms (genuine deadline expiry) — neither
    // instantly (the pre-fix bug) nor never (an unbounded park).
    let bin = build("deadline", &server_src());
    let mut child = Command::new(&bin)
        .arg("0")
        .arg("300")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn");
    let port = child::tcp_port(&mut child);
    let started = Instant::now();
    let mut s = connect(port);
    // Send nothing; wait for the handler's DONE response, which it
    // writes right after recv_into returns.
    let mut resp = Vec::new();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    use std::io::Read;
    let _ = s.read_to_end(&mut resp);
    let elapsed = started.elapsed();
    drop(s);
    let _ = child.kill();
    let out = child.wait_with_output().expect("wait");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("got=-2"),
        "silent peer + 300ms deadline must yield the -2 sentinel; \
         stdout: {:?}",
        stdout
    );
    assert!(
        elapsed >= Duration::from_millis(250),
        "-2 arrived after {:?} — an instant would-block, not a \
         deadline expiry",
        elapsed
    );
    assert!(
        elapsed < Duration::from_secs(4),
        "-2 took {:?} — the deadline never fired",
        elapsed
    );
}

#[test]
fn udp_recv_into_returns_minus_two_on_timeout() {
    // The udp sibling's contract alignment (blocking pool): a
    // SO_RCVTIMEO expiry now returns -2 retryable — it previously
    // fell into -1 fatal.
    // Nothing is sent to the socket, so it binds port 0.
    let src = r#"
        fn main() {
            let port = std::str::parse_int(std::env::arg(1)) or raise;
            let fd = std::io::udp::bind("127.0.0.1", port) or raise;
            std::io::udp::set_recv_timeout(fd, 200ms) or raise;
            let b = std::bytes::BytesBuilder { };
            let got = std::io::udp::recv_into(fd, b, 256);
            println("got=", got);
            std::io::udp::close(fd);
        }
    "#;
    let bin = build("udp", src);
    let out = Command::new(&bin)
        .arg("0")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "exit: {:?}\nstdout: {}", out.status, stdout);
    assert!(
        stdout.contains("got=-2"),
        "udp recv_into on SO_RCVTIMEO expiry must return -2 (was -1); \
         got: {:?}",
        stdout
    );
}

#[test]
fn recv_bytes_honors_recv_deadline_on_async_io() {
    // recv_bytes parked indefinitely pre-fix, silently ignoring
    // set_recv_timeout on async_io pools. With the timed park it
    // returns the empty Bytes at ~deadline (its documented timeout
    // shape on blocking pools).
    let src = format!(
        r#"
        fn handle(s: std::io::tcp::Stream) {{
            let _ = std::io::tcp::__set_recv_timeout_ns(
                s.conn_fd, 300000000);
            let got = s.recv_bytes(256) or raise;
            println("len=", len(got));
            let resp = std::bytes::from_string("DONE\n");
            s.send_bytes(resp) or raise;
        }}

        main locus App {{
            params {{
                l: std::io::tcp::Listener = std::io::tcp::Listener {{
                    host:          "127.0.0.1",
                    port:          0,
                    max_accepts:   1,
                    on_connection: handle,
                }};
            }}
            placement {{
                l: cooperative(pool = io) where async_io;
            }}
            run() {{
                std::time::sleep(4s);
            }}
        }}

        fn main() {{ App {{ }}; }}
    "#
    );
    let bin = build("recv_bytes_deadline", &src);
    let mut child = Command::new(&bin)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn");
    let port = child::tcp_port(&mut child);
    let started = Instant::now();
    let mut s = connect(port);
    let mut resp = Vec::new();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    use std::io::Read;
    let _ = s.read_to_end(&mut resp);
    let elapsed = started.elapsed();
    drop(s);
    let _ = child.kill();
    let out = child.wait_with_output().expect("wait");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("len=0"),
        "silent peer + deadline must yield the empty Bytes; stdout: {:?}",
        stdout
    );
    assert!(
        elapsed >= Duration::from_millis(250) && elapsed < Duration::from_secs(4),
        "recv_bytes deadline mistimed: {:?}",
        elapsed
    );
}

/// Connect to the program's listener, which is up: the port was read
/// from its listening socket.
fn connect(port: u16) -> TcpStream {
    TcpStream::connect(("127.0.0.1", port)).unwrap_or_else(|e| panic!("connect 127.0.0.1:{port}: {e:?}"))
}

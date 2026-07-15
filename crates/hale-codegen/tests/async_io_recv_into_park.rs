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

use hale_codegen::build_executable;

fn pick_free_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind probe");
    l.local_addr().expect("local_addr").port()
}

fn build(name: &str, src: &str) -> std::path::PathBuf {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!(
        "hale_async_recv_into_park_{}_{}",
        name,
        std::process::id()
    ));
    build_executable(&program, &bin).expect("build");
    bin
}

/// A listener on an async_io pool whose connection handler does ONE
/// `recv_into` and reports the result. `timeout_ms` argv arms
/// `set_recv_timeout` on the conn fd first (0 = leave unset).
fn server_src(port: u16) -> String {
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
            s.send_bytes(resp);
        }}

        main locus App {{
            params {{
                l: std::io::tcp::Listener = std::io::tcp::Listener {{
                    host:          "127.0.0.1",
                    port:          {port},
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
    let port = pick_free_port();
    let bin = build("parks", &server_src(port));
    let mut child = Command::new(&bin)
        .arg(port.to_string())
        .arg("0")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn");
    thread::sleep(Duration::from_millis(150));
    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
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
    let port = pick_free_port();
    let bin = build("deadline", &server_src(port));
    let mut child = Command::new(&bin)
        .arg(port.to_string())
        .arg("300")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn");
    thread::sleep(Duration::from_millis(150));
    let started = Instant::now();
    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
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
    let port = pick_free_port();
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
        .arg(port.to_string())
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
    let port = pick_free_port();
    let src = format!(
        r#"
        fn handle(s: std::io::tcp::Stream) {{
            let _ = std::io::tcp::__set_recv_timeout_ns(
                s.conn_fd, 300000000);
            let got = s.recv_bytes(256);
            println("len=", len(got));
            let resp = std::bytes::from_string("DONE\n");
            s.send_bytes(resp);
        }}

        main locus App {{
            params {{
                l: std::io::tcp::Listener = std::io::tcp::Listener {{
                    host:          "127.0.0.1",
                    port:          {port},
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
    thread::sleep(Duration::from_millis(150));
    let started = Instant::now();
    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
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

//! GH #1081 — shutting a `std::http::Server` down on purpose is silent.
//!
//! The server's `shutdown()` does `shutdown(SHUT_RDWR)` on its listen
//! fd; the pool worker parked on that fd wakes, and its `accept` fails
//! EINVAL. That is the wake that was asked for, but b35a4494 removed the
//! guard around the `perror` in `lotus_tcp_accept_one` and left the
//! call, so every deliberate shutdown ended with `lotus_tcp_accept_one:
//! accept: Invalid argument` on stderr. The runtime now marks a listen
//! fd it shut down, and stays silent for that fd and for a worker whose
//! pool is shutting down. An accept failure nobody asked for is still
//! reported.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

#[test]
fn a_deliberate_server_shutdown_is_silent() {
    let port = harness::free_port();
    let src = format!(
        r#"
locus Hello {{ fn handle(req: std::http::Request) -> std::http::Response {{ return std::http::Response {{ status: 200, body: "hi" }}; }} }}
main locus App {{
    params {{ srv: std::http::Server; }}
    placement {{ srv: cooperative(pool = io); }}
    run() {{ while !self.draining {{ std::time::sleep(20ms); }} self.srv.shutdown(); }}
}}
fn main() {{ App {{ srv: std::http::Server {{ host: "127.0.0.1", port: {port}, ready_signal: "up", handler: Hello {{ }} }} }}; }}
"#
    );
    let program = hale_syntax::parse_source(&src).expect("parse");
    let bin = harness::unique_bin("hale_1081_accept_shutdown");
    build_executable(&program, &bin).expect("build");
    let child = Command::new(&bin)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    // up once the port answers
    let t0 = Instant::now();
    while std::net::TcpStream::connect(("127.0.0.1", port)).is_err()
        && t0.elapsed() < Duration::from_secs(10)
    {
        std::thread::sleep(Duration::from_millis(20));
    }
    let killed = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .expect("kill");
    assert!(killed.success(), "sent SIGTERM");
    let out = child.wait_with_output().expect("wait");
    let _ = std::fs::remove_file(&bin);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "the server drains to exit 0: {:?}; stderr: {stderr}", out.status);
    assert!(
        !stderr.contains("lotus_tcp_accept_one"),
        "a deliberate shutdown prints nothing about accept: {stderr}"
    );
}

#[test]
fn an_accept_failure_nobody_asked_for_is_still_reported() {
    let program = hale_syntax::parse_source(
        "fn main() { let c = std::io::tcp::accept_one(-1) or - 1; println(to_string(c)); }\n",
    )
    .expect("parse");
    let bin = harness::unique_bin("hale_1081_accept_bad_fd");
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("lotus_tcp_accept_one: accept: Bad file descriptor"),
        "stderr: {stderr}"
    );
}

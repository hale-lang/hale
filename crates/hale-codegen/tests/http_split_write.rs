//! downstream handoff 2026-07-14 finding 2: `std::http::Server` read each
//! request with a single `recv(8192)`, so clients that write headers
//! and body in separate TCP segments (python urllib does exactly
//! this) got their body truncated — a valid POST failed while curl
//! (single write) worked. `__http_handle_one_conn` now reassembles:
//! accumulate to `\r\n\r\n`, then to `Content-Length` body bytes,
//! with a 1 MiB cap (413) and a 5s recv timeout.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use hale_codegen::build_executable;

// Echo server: responds 200 with the request body it parsed, so the
// client can observe exactly what the server-side reassembly saw.
const ECHO_SERVER: &str = r#"
    locus Echo {
        fn handle(req: std::http::Request) -> std::http::Response {
            return std::http::Response { status: 200, body: req.body };
        }
    }
    fn main() {
        let port = std::str::parse_int(std::env::arg(1)) or 0;
        let accepts = std::str::parse_int(std::env::arg(2)) or 1;
        std::http::Server {
            host:        "127.0.0.1",
            port:        port,
            max_accepts: accepts,
            handler:     Echo { },
        };
    }
"#;

fn pick_free_port() -> u16 {
    let probe = std::net::TcpListener::bind("127.0.0.1:0").expect("bind probe");
    probe.local_addr().expect("local_addr").port()
}

fn build_echo(name: &str) -> PathBuf {
    let program = hale_syntax::parse_source(ECHO_SERVER).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_http_split_write_{}_{}", name, std::process::id()));
    build_executable(&program, &bin).expect("build");
    bin
}

fn connect_with_retry(port: u16) -> TcpStream {
    for _ in 0..100 {
        if let Ok(s) = TcpStream::connect(("127.0.0.1", port)) {
            return s;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("failed to connect after 2s on port {}", port);
}

fn run_one(name: &str, client: impl FnOnce(&mut TcpStream)) -> String {
    let bin = build_echo(name);
    let port = pick_free_port();
    let child = Command::new(&bin)
        .arg(port.to_string())
        .arg("1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn echo server");

    let mut sock = connect_with_retry(port);
    client(&mut sock);
    let mut buf = Vec::new();
    let _ = sock.read_to_end(&mut buf);
    drop(sock);

    let out = child.wait_with_output().expect("wait child");
    let _ = std::fs::remove_file(&bin);
    assert!(
        out.status.success(),
        "server exited non-zero: {:?}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&buf).to_string()
}

#[test]
fn split_headers_then_body_is_reassembled() {
    // The exact urllib shape: headers in one segment, body in a
    // later one. Pre-fix the server parsed an empty body.
    let resp = run_one("head_body", |sock| {
        sock.write_all(
            b"POST /x HTTP/1.1\r\nHost: h\r\nContent-Length: 11\r\n\r\n",
        )
        .expect("write head");
        thread::sleep(Duration::from_millis(150));
        sock.write_all(b"hello world").expect("write body");
    });
    assert!(
        resp.starts_with("HTTP/1.1 200 OK\r\n"),
        "wrong status line; got: {:?}",
        resp
    );
    assert!(
        resp.ends_with("hello world"),
        "body was truncated; got: {:?}",
        resp
    );
}

#[test]
fn three_way_split_is_reassembled() {
    // Request line / rest of headers + terminator / body, three
    // segments — catches "terminator arrives in the first chunk"
    // assumptions.
    let resp = run_one("three_way", |sock| {
        sock.write_all(b"POST /y HTTP/1.1\r\n").expect("write line");
        thread::sleep(Duration::from_millis(80));
        sock.write_all(b"Host: h\r\nContent-Length: 9\r\n\r\n")
            .expect("write headers");
        thread::sleep(Duration::from_millis(80));
        sock.write_all(b"abcdefghi").expect("write body");
    });
    assert!(
        resp.starts_with("HTTP/1.1 200 OK\r\n"),
        "wrong status line; got: {:?}",
        resp
    );
    assert!(resp.ends_with("abcdefghi"), "body was truncated; got: {:?}", resp);
}

#[test]
fn single_write_get_answers_immediately() {
    // No Content-Length → need = 0 is satisfied as soon as the
    // header terminator arrives; the server must not wait for a
    // body that never comes.
    let resp = run_one("single_get", |sock| {
        sock.write_all(b"GET /z HTTP/1.1\r\nHost: h\r\n\r\n")
            .expect("write get");
    });
    assert!(
        resp.starts_with("HTTP/1.1 200 OK\r\n"),
        "wrong status line; got: {:?}",
        resp
    );
}

#[test]
fn oversized_declared_content_length_gets_413() {
    // Content-Length over the 1 MiB cap: refused up front, before
    // the server buffers anything like that much.
    let resp = run_one("oversized", |sock| {
        sock.write_all(
            b"POST /big HTTP/1.1\r\nHost: h\r\nContent-Length: 2000000\r\n\r\n",
        )
        .expect("write head");
        let _ = sock.write_all(b"tiny");
    });
    assert!(
        resp.starts_with("HTTP/1.1 413 Payload Too Large\r\n"),
        "expected 413; got: {:?}",
        resp
    );
}

#[test]
fn silent_client_after_headers_is_bounded_by_the_recv_timeout() {
    // Headers declare a body that never arrives: the 5s recv
    // timeout fires, the server serves what it has (truncated
    // body) instead of hanging the accept loop forever.
    let started = std::time::Instant::now();
    let resp = run_one("silent", |sock| {
        sock.write_all(
            b"POST /w HTTP/1.1\r\nHost: h\r\nContent-Length: 11\r\n\r\n",
        )
        .expect("write head");
        // ... and go silent.
    });
    let elapsed = started.elapsed();
    assert!(
        resp.starts_with("HTTP/1.1 200 OK\r\n"),
        "expected a served (truncated) response; got: {:?}",
        resp
    );
    assert!(
        elapsed < Duration::from_secs(15),
        "server should give up within the recv timeout; took {:?}",
        elapsed
    );
}

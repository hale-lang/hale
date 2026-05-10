//! m86: Phase 3 capstone — integration test for
//! examples/http-hello/main.ap.
//!
//! Builds the example, runs it on a picked port with a
//! bounded max_accepts, sends a real HTTP/1.1 request via
//! TcpStream, and asserts on both the response wire format
//! and the example's stderr/stdout. End-to-end exercise of
//! m82 + m83 + m84 + m85 composed.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use aperio_codegen::build_executable;

fn examples_dir() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p.push("examples");
    p
}

fn pick_free_port() -> u16 {
    let probe = std::net::TcpListener::bind("127.0.0.1:0").expect("bind probe");
    probe.local_addr().expect("local_addr").port()
}

fn build_http_hello() -> PathBuf {
    let src_path = examples_dir().join("http-hello").join("main.ap");
    let src = std::fs::read_to_string(&src_path).expect("read example");
    let program = aperio_syntax::parse_source(&src).expect("parse example");
    let mut bin = std::env::temp_dir();
    bin.push(format!(
        "aperio_http_hello_{}",
        std::process::id()
    ));
    build_executable(&program, &bin).expect("build example");
    bin
}

#[test]
fn http_hello_responds_to_get_with_html_body() {
    // One request, max_accepts=1 so the server exits after
    // handling it. Verifies the full chain: parse_request
    // pulls "GET /hello", handle_request builds an HTML
    // Response with the path embedded, write_response ships
    // it, and the client sees a well-formed HTTP/1.1
    // response.
    let bin = build_http_hello();
    let port = pick_free_port();
    let child = Command::new(&bin)
        .arg(port.to_string())
        .arg("1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn http-hello");

    // Retry-connect (no bind probe — burning a separate
    // accept on a probe inflates the server's max_accepts
    // budget for no visible reason in test code).
    let mut sock = {
        let mut s = None;
        for _ in 0..100 {
            if let Ok(c) = TcpStream::connect(("127.0.0.1", port)) {
                s = Some(c);
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        s.expect("server didn't come up")
    };
    sock.write_all(b"GET /hello HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
        .expect("client write");
    let mut buf = Vec::new();
    let _ = sock.read_to_end(&mut buf);
    drop(sock);

    let out = child.wait_with_output().expect("wait child");
    let _ = std::fs::remove_file(&bin);

    let response = String::from_utf8_lossy(&buf);
    assert!(
        response.starts_with("HTTP/1.1 200 OK\r\n"),
        "wrong status line; got: {:?}",
        response
    );
    assert!(
        response.contains("Content-Type: text/html"),
        "missing html content-type; got: {:?}",
        response
    );
    assert!(
        response.contains("<h1>Hello from Aperio</h1>"),
        "missing greeting in body; got: {:?}",
        response
    );
    assert!(
        response.contains("You requested: /hello"),
        "path didn't echo into body; got: {:?}",
        response
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("http-hello: GET /hello"),
        "missing request log on server stdout; got: {:?}",
        stdout
    );
    assert!(
        out.status.success(),
        "server exited non-zero: {:?}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn http_hello_handles_three_requests_in_sequence() {
    // max_accepts=3 with no bind-probe — each test client
    // retries connect itself until success, so accept-budget
    // is exactly 3 == iterations. Each iteration's Stream
    // dissolves between accepts so fds don't bleed.
    let bin = build_http_hello();
    let port = pick_free_port();
    let child = Command::new(&bin)
        .arg(port.to_string())
        .arg("3")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn http-hello");

    let connect_with_retry = |port: u16| -> TcpStream {
        for _ in 0..100 {
            if let Ok(s) = TcpStream::connect(("127.0.0.1", port)) {
                return s;
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!("failed to connect after 2s on port {}", port);
    };

    let mut paths_seen = Vec::new();
    let mut responses: Vec<String> = Vec::new();
    for path in &["/one", "/two", "/three"] {
        let mut sock = connect_with_retry(port);
        sock.write_all(
            format!("GET {} HTTP/1.1\r\n\r\n", path).as_bytes(),
        )
        .expect("client write");
        let mut buf = Vec::new();
        let _ = sock.read_to_end(&mut buf);
        responses.push(String::from_utf8_lossy(&buf).to_string());
        paths_seen.push(path.to_string());
    }

    let out = child.wait_with_output().expect("wait child");
    let _ = std::fs::remove_file(&bin);

    for (path, resp) in paths_seen.iter().zip(responses.iter()) {
        assert!(
            resp.starts_with("HTTP/1.1 200 OK\r\n"),
            "iter for {} got: {:?}\nserver stdout: {}\nserver stderr: {}",
            path,
            resp,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        );
        assert!(
            resp.contains(&format!("You requested: {}", path)),
            "iter for {} missing echoed path; got: {:?}",
            path,
            resp
        );
    }

    let stdout = String::from_utf8_lossy(&out.stdout);
    for path in &paths_seen {
        assert!(
            stdout.contains(&format!("http-hello: GET {}", path)),
            "missing log for {}; stdout: {:?}",
            path,
            stdout
        );
    }
}

//! m86: Phase 3 capstone — integration test for
//! examples/http-hello/main.hl.
//!
//! Builds the example, runs it on a port the kernel picks with a
//! bounded max_accepts, sends a real HTTP/1.1 request via
//! TcpStream, and asserts on both the response wire format
//! and the example's stderr/stdout. End-to-end exercise of
//! m82 + m83 + m84 + m85 composed.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;
#[path = "support/child.rs"]
mod child;

fn examples_dir() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests");
    p.push("fixtures");
    p.push("examples");
    p
}

/// The example, built to listen on port 0 when told port 0. As written
/// it takes only a positive port from its argv and otherwise serves on
/// 8080, so the test turns that default into 0 and the kernel picks the
/// port, which the test reads back (`child::tcp_port`): a port drawn here
/// and handed over is free, not reserved, and another test of a parallel
/// run can bind it before the server does.
fn build_http_hello() -> PathBuf {
    let src_path = examples_dir().join("http-hello").join("main.hl");
    let src = std::fs::read_to_string(&src_path).expect("read example");
    let default = "let mut port: Int = 8080;";
    assert!(src.contains(default), "the example's default port moved: {}", src_path.display());
    let src = src.replacen(default, "let mut port: Int = 0;", 1);
    let bin = harness::unique_bin(&format!(
        "hale_http_hello_{}",
        std::process::id()
    ));
    build_opts::build_source(&src, &bin, &build_opts::options()).expect("build example");
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
    let mut child = Command::new(&bin)
        .arg("0")
        .arg("1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn http-hello");
    // The port is read once the server listens, and reading it costs no
    // accept from the max_accepts budget.
    let port = child::tcp_port(&mut child);

    let mut sock = TcpStream::connect(("127.0.0.1", port)).expect("connect");
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
        response.contains("<h1>Hello from Hale</h1>"),
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
    // max_accepts=3 with no bind-probe — the port is read from
    // /proc, not by a connect, so the accept budget is exactly
    // 3 == iterations. Each iteration's Stream
    // dissolves between accepts so fds don't bleed.
    let bin = build_http_hello();
    let mut child = Command::new(&bin)
        .arg("0")
        .arg("3")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn http-hello");
    // The port is read once the server listens, and reading it costs no
    // accept from the max_accepts budget.
    let port = child::tcp_port(&mut child);

    let mut paths_seen = Vec::new();
    let mut responses: Vec<String> = Vec::new();
    for path in &["/one", "/two", "/three"] {
        let mut sock = TcpStream::connect(("127.0.0.1", port)).expect("connect");
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

//! m85: std::http::write_response — Response → HTTP/1.1
//! wire bytes over a Stream.
//!
//! Each test runs an Hale program that listens on a port,
//! accepts one connection, writes a known Response, and
//! exits. A Rust client connects, reads the bytes, and
//! verifies the wire format: status line, headers,
//! body separator, body content.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;
#[path = "support/child.rs"]
mod child;

fn build_hale(name: &str, source: &str) -> std::path::PathBuf {
    let bin = harness::unique_bin(&format!("hale_test_http_resp_{}", name));
    build_opts::build_source(source, &bin, &build_opts::options()).expect("build");
    bin
}

/// Start the listener and read back the port it was given. The programs
/// bind port 0 and the kernel picks: a port drawn here and handed over is
/// free, not reserved, and another test of a parallel run can bind it
/// before the program does. Reading it from `/proc` costs no accept, so
/// a `max_accepts: 1` listener still serves the test's one connection.
fn spawn_server(bin: &std::path::Path) -> (Child, u16) {
    let mut child = Command::new(bin)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run listener");
    let port = child::tcp_port(&mut child);
    (child, port)
}

fn run_server_collect_response(
    bin: &std::path::Path,
    request: &[u8],
) -> String {
    let (server, port) = spawn_server(bin);

    let mut client = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    client.write_all(request).expect("client write");
    let mut buf = Vec::new();
    let _ = client.read_to_end(&mut buf);
    drop(client);

    let _ = server.wait_with_output();
    String::from_utf8_lossy(&buf).to_string()
}

#[test]
fn write_response_emits_http_1_1_status_line() {
    // 200 OK with text/plain body. Verifies: status line shape,
    // Content-Type header, Content-Length matches body bytes,
    // Connection: close header, blank-line separator, body.
    let src = format!(
        r#"
        fn handler(s: std::io::tcp::Stream) {{
            let resp = std::http::Response {{
                status: 200,
                content_type: "text/plain",
                body: "hello world"
            }};
            std::http::write_response(s, resp);
        }}

        fn main() {{
            std::io::tcp::Listener {{
                host: "127.0.0.1",
                port: 0,
                max_accepts: 1,
                on_connection: handler
            }};
        }}
        "#
    );
    let bin = build_hale("status_200", &src);
    let response = run_server_collect_response(
        &bin,
        b"GET / HTTP/1.1\r\n\r\n",
    );
    let _ = std::fs::remove_file(&bin);

    assert!(
        response.starts_with("HTTP/1.1 200 OK\r\n"),
        "wrong status line; got: {:?}",
        response
    );
    assert!(
        response.contains("Content-Type: text/plain\r\n"),
        "missing content-type; got: {:?}",
        response
    );
    assert!(
        response.contains("Content-Length: 11\r\n"),
        "wrong content-length; got: {:?}",
        response
    );
    assert!(
        response.contains("Connection: close\r\n"),
        "missing connection-close; got: {:?}",
        response
    );
    assert!(
        response.ends_with("\r\n\r\nhello world"),
        "wrong body or separator; got: {:?}",
        response
    );
}

#[test]
fn write_response_404_with_canonical_phrase() {
    // 404 Not Found — verifies the status-phrase table covers
    // the most common error path. Body and Content-Length still
    // match conventional expectations.
    let src = format!(
        r#"
        fn handler(s: std::io::tcp::Stream) {{
            let resp = std::http::Response {{
                status: 404,
                content_type: "text/plain",
                body: "no such page"
            }};
            std::http::write_response(s, resp);
        }}

        fn main() {{
            std::io::tcp::Listener {{
                host: "127.0.0.1",
                port: 0,
                max_accepts: 1,
                on_connection: handler
            }};
        }}
        "#
    );
    let bin = build_hale("status_404", &src);
    let response = run_server_collect_response(
        &bin,
        b"GET /missing HTTP/1.1\r\n\r\n",
    );
    let _ = std::fs::remove_file(&bin);

    assert!(
        response.starts_with("HTTP/1.1 404 Not Found\r\n"),
        "wrong 404 status line; got: {:?}",
        response
    );
    assert!(
        response.contains("Content-Length: 12\r\n"),
        "wrong content-length for body 'no such page'; got: {:?}",
        response
    );
    assert!(
        response.ends_with("\r\n\r\nno such page"),
        "missing body; got: {:?}",
        response
    );
}

#[test]
fn write_response_with_html_content_type() {
    // text/html — the doc server's primary content type.
    // Body has a `<` character to confirm string content with
    // html-shape characters round-trips through the wire.
    let src = format!(
        r#"
        fn handler(s: std::io::tcp::Stream) {{
            let resp = std::http::Response {{
                status: 200,
                content_type: "text/html; charset=utf-8",
                body: "<h1>Hale</h1>"
            }};
            std::http::write_response(s, resp);
        }}

        fn main() {{
            std::io::tcp::Listener {{
                host: "127.0.0.1",
                port: 0,
                max_accepts: 1,
                on_connection: handler
            }};
        }}
        "#
    );
    let bin = build_hale("html", &src);
    let response = run_server_collect_response(
        &bin,
        b"GET / HTTP/1.1\r\n\r\n",
    );
    let _ = std::fs::remove_file(&bin);

    assert!(
        response.contains("Content-Type: text/html; charset=utf-8\r\n"),
        "missing html content-type; got: {:?}",
        response
    );
    assert!(
        response.ends_with("\r\n\r\n<h1>Hale</h1>"),
        "html body wrong; got: {:?}",
        response
    );
}

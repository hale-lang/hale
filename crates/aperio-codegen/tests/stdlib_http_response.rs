//! m85: std::http::write_response — Response → HTTP/1.1
//! wire bytes over a Stream.
//!
//! Each test runs an Aperio program that listens on a port,
//! accepts one connection, writes a known Response, and
//! exits. A Rust client connects, reads the bytes, and
//! verifies the wire format: status line, headers,
//! body separator, body content.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::Command;
use std::thread;
use std::time::Duration;

use aperio_codegen::build_executable;

fn build_aperio(name: &str, source: &str) -> std::path::PathBuf {
    let program = aperio_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("aperio_test_http_resp_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

fn pick_free_port() -> u16 {
    let probe = std::net::TcpListener::bind("127.0.0.1:0").expect("bind probe");
    probe.local_addr().expect("local_addr").port()
}

fn run_server_collect_response(
    bin: &std::path::Path,
    port: u16,
    request: &[u8],
) -> String {
    let bin_path = bin.to_path_buf();
    let server_handle = thread::spawn(move || {
        Command::new(&bin_path).output().expect("run listener")
    });
    thread::sleep(Duration::from_millis(150));

    let mut client = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    client.write_all(request).expect("client write");
    let mut buf = Vec::new();
    let _ = client.read_to_end(&mut buf);
    drop(client);

    let _ = server_handle.join();
    String::from_utf8_lossy(&buf).to_string()
}

#[test]
fn write_response_emits_http_1_1_status_line() {
    // 200 OK with text/plain body. Verifies: status line shape,
    // Content-Type header, Content-Length matches body bytes,
    // Connection: close header, blank-line separator, body.
    let port = pick_free_port();
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
                port: {},
                max_accepts: 1,
                on_connection: handler
            }};
        }}
        "#,
        port
    );
    let bin = build_aperio("status_200", &src);
    let response = run_server_collect_response(
        &bin,
        port,
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
    let port = pick_free_port();
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
                port: {},
                max_accepts: 1,
                on_connection: handler
            }};
        }}
        "#,
        port
    );
    let bin = build_aperio("status_404", &src);
    let response = run_server_collect_response(
        &bin,
        port,
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
    let port = pick_free_port();
    let src = format!(
        r#"
        fn handler(s: std::io::tcp::Stream) {{
            let resp = std::http::Response {{
                status: 200,
                content_type: "text/html; charset=utf-8",
                body: "<h1>Aperio</h1>"
            }};
            std::http::write_response(s, resp);
        }}

        fn main() {{
            std::io::tcp::Listener {{
                host: "127.0.0.1",
                port: {},
                max_accepts: 1,
                on_connection: handler
            }};
        }}
        "#,
        port
    );
    let bin = build_aperio("html", &src);
    let response = run_server_collect_response(
        &bin,
        port,
        b"GET / HTTP/1.1\r\n\r\n",
    );
    let _ = std::fs::remove_file(&bin);

    assert!(
        response.contains("Content-Type: text/html; charset=utf-8\r\n"),
        "missing html content-type; got: {:?}",
        response
    );
    assert!(
        response.ends_with("\r\n\r\n<h1>Aperio</h1>"),
        "html body wrong; got: {:?}",
        response
    );
}

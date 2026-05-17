//! C11 (pond follow-up) — `std::http::Response.headers` field
//! + symmetric `std::http::header(resp, name)` lookup.
//!
//! Two surfaces to exercise:
//!
//! 1. Wire-side: a Response constructed with non-empty
//!    `headers` must emit those header lines between the fixed
//!    `Connection: close` line and the blank-line separator,
//!    so consumers see them on the wire.
//!
//! 2. Lookup-side: `std::http::header(resp, "X-Custom")` must
//!    fork on receiver type and route through
//!    `__http_response_header` — mirroring the Request-side
//!    `header(req, name)` path-call.
//!
//! The wire test mirrors the shape of `stdlib_http_response.rs`
//! (Aperio listener + Rust TCP client). The lookup test runs
//! a pure-stdlib program that prints the result of the
//! Response-side `header` call.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::Command;
use std::thread;
use std::time::Duration;

use aperio_codegen::build_executable;

fn build_aperio(name: &str, source: &str) -> std::path::PathBuf {
    let program = aperio_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("aperio_test_http_resp_headers_{}", name));
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
fn write_response_emits_user_supplied_headers_on_wire() {
    // C11: `Response.headers` carries CRLF-joined user lines.
    // The wire must include both X-Custom and Set-Cookie in
    // the same block as Content-Type / Content-Length /
    // Connection. The blank-line separator before the body
    // must remain exactly `\r\n\r\n` (no missing terminator,
    // no extra CRLF).
    let port = pick_free_port();
    let src = format!(
        r#"
        fn handler(s: std::io::tcp::Stream) {{
            let resp = std::http::Response {{
                status: 200,
                content_type: "text/plain",
                headers: "X-Custom: hi\r\nSet-Cookie: a=b",
                body: "ok"
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
    let bin = build_aperio("wire", &src);
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
        response.contains("Content-Length: 2\r\n"),
        "wrong content-length for body 'ok'; got: {:?}",
        response
    );
    assert!(
        response.contains("Connection: close\r\n"),
        "missing connection-close; got: {:?}",
        response
    );
    assert!(
        response.contains("X-Custom: hi\r\n"),
        "missing user header X-Custom; got: {:?}",
        response
    );
    assert!(
        response.contains("Set-Cookie: a=b\r\n"),
        "missing user header Set-Cookie; got: {:?}",
        response
    );
    assert!(
        response.ends_with("\r\n\r\nok"),
        "wrong body or separator (expect \\r\\n\\r\\n before body); \
         got: {:?}",
        response
    );
}

#[test]
fn empty_headers_field_preserves_v0_wire_shape() {
    // Constraint from the C11 brief: with `headers == ""` the
    // wire bytes must be byte-identical to the v0 implementation
    // (no trailing extra CRLF after Connection: close). This
    // pins the empty-headers path so a future refactor can't
    // regress it.
    let port = pick_free_port();
    let src = format!(
        r#"
        fn handler(s: std::io::tcp::Stream) {{
            let resp = std::http::Response {{
                status: 200,
                content_type: "text/plain",
                body: "hi"
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
    let bin = build_aperio("empty_headers", &src);
    let response = run_server_collect_response(
        &bin,
        port,
        b"GET / HTTP/1.1\r\n\r\n",
    );
    let _ = std::fs::remove_file(&bin);

    // Exact wire-format check: the four fixed headers, single
    // blank-line separator, then body — no extra CRLFs.
    let expected = "HTTP/1.1 200 OK\r\n\
                    Content-Type: text/plain\r\n\
                    Content-Length: 2\r\n\
                    Connection: close\r\n\
                    \r\n\
                    hi";
    assert_eq!(
        response, expected,
        "empty-headers wire shape changed; expected v0 byte-identical output"
    );
}

#[test]
fn header_lookup_on_response_returns_value() {
    // C11 lookup-side: `std::http::header(resp, name)` must
    // dispatch on receiver type and route to the Response-side
    // getter. Construct a Response with attached headers and
    // assert single-header / multi-header / absent lookups all
    // resolve correctly.
    let src = r#"
        fn main() {
            let resp = std::http::Response {
                status: 200,
                content_type: "text/plain",
                headers: "X-Custom: hi\r\nSet-Cookie: a=b",
                body: "ok"
            };
            println("custom=", std::http::header(resp, "X-Custom"));
            println("cookie=", std::http::header(resp, "Set-Cookie"));
            // Case-insensitive: same fold as Request-side.
            println("custom_lower=", std::http::header(resp, "x-custom"));
            // Absent header returns empty.
            println("absent=[", std::http::header(resp, "X-Not-Here"), "]");
        }
    "#;
    let bin = build_aperio("lookup", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("custom=hi"),
        "Response-side header(\"X-Custom\") missing; got: {:?}",
        stdout
    );
    assert!(
        stdout.contains("cookie=a=b"),
        "Response-side header(\"Set-Cookie\") missing; got: {:?}",
        stdout
    );
    assert!(
        stdout.contains("custom_lower=hi"),
        "case-insensitive Response-side lookup failed; got: {:?}",
        stdout
    );
    assert!(
        stdout.contains("absent=[]"),
        "absent Response-side header should return empty; got: {:?}",
        stdout
    );
}

#[test]
fn header_lookup_still_works_on_request_after_c11() {
    // Regression guard: C11 reshuffled the dispatch arm in
    // codegen.rs. The Request-side path-call must still resolve
    // to `__http_request_header` after the receiver-type fork.
    let src = r#"
        fn main() {
            let raw = "GET / HTTP/1.1\r\nHost: example.com\r\n\r\n";
            let r = std::http::parse_request(raw);
            println("host=", std::http::header(r, "Host"));
        }
    "#;
    let bin = build_aperio("req_regression", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("host=example.com"),
        "Request-side header lookup regressed; got: {:?}",
        stdout
    );
}

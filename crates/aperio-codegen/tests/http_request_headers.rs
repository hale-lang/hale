//! ws-echo friction `http-request-headers-absent` —
//! `std::http::header(req, name)` reads a header by name off
//! a parsed Request. Pre-fix `parse_request` discarded the
//! header lines entirely; the Request type had no `headers`
//! field and no accessor.

use std::process::Command;

use aperio_codegen::build_executable;

fn build(name: &str, src: &str) -> std::path::PathBuf {
    let program = aperio_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("aperio_test_http_headers_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

#[test]
fn header_lookup_returns_value_or_empty() {
    let src = r#"
        fn main() {
            let raw = "GET /ws HTTP/1.1\r\nHost: example.com\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nUpgrade: websocket\r\n\r\n";
            let r = std::http::parse_request(raw);
            println("host=", std::http::header(r, "Host"));
            println("key=", std::http::header(r, "Sec-WebSocket-Key"));
            println("up=", std::http::header(r, "Upgrade"));
            println("absent=[", std::http::header(r, "X-Not-Here"), "]");
        }
    "#;
    let bin = build("ws_handshake_req", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("host=example.com"), "got: {:?}", stdout);
    assert!(
        stdout.contains("key=dGhlIHNhbXBsZSBub25jZQ=="),
        "got: {:?}", stdout
    );
    assert!(stdout.contains("up=websocket"), "got: {:?}", stdout);
    assert!(stdout.contains("absent=[]"), "got: {:?}", stdout);
}

#[test]
fn parse_request_still_returns_method_path_body() {
    // Pin the existing surface — adding the `headers` field
    // shouldn't break callers that ignore it.
    let src = r#"
        fn main() {
            let raw = "POST /api HTTP/1.1\r\nContent-Length: 5\r\n\r\nhello";
            let r = std::http::parse_request(raw);
            println("m=", r.method);
            println("p=", r.path);
            println("b=", r.body);
        }
    "#;
    let bin = build("backcompat", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("m=POST"), "got: {:?}", stdout);
    assert!(stdout.contains("p=/api"), "got: {:?}", stdout);
    assert!(stdout.contains("b=hello"), "got: {:?}", stdout);
}

//! ws-echo friction `http-request-headers-absent` —
//! `std::http::header(req, name)` reads a header by name off
//! a parsed Request. Pre-fix `parse_request` discarded the
//! header lines entirely; the Request type had no `headers`
//! field and no accessor.

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn build(name: &str, src: &str) -> std::path::PathBuf {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(&format!("hale_test_http_headers_{}", name));
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
fn header_lookup_is_case_insensitive() {
    // RFC 7230 §3.2 says header names are case-insensitive.
    // Pre-v1.x lookup was case-sensitive — only worked because
    // WebSocket-upgrade clients send fixed-case names. Now that
    // std::str::lower exists, the lookup folds both sides.
    let src = r#"
        fn main() {
            let raw = "GET / HTTP/1.1\r\nContent-Type: application/json\r\nUser-Agent: curl/8\r\n\r\n";
            let r = std::http::parse_request(raw);
            // Same name, different casing — should all match.
            println("ct1=", std::http::header(r, "Content-Type"));
            println("ct2=", std::http::header(r, "content-type"));
            println("ct3=", std::http::header(r, "CONTENT-TYPE"));
            println("ua=",  std::http::header(r, "user-agent"));
        }
    "#;
    let bin = build("case_insensitive", src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("ct1=application/json"), "got: {:?}", stdout);
    assert!(stdout.contains("ct2=application/json"), "got: {:?}", stdout);
    assert!(stdout.contains("ct3=application/json"), "got: {:?}", stdout);
    assert!(stdout.contains("ua=curl/8"), "got: {:?}", stdout);
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

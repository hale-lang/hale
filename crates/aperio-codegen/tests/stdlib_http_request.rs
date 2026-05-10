//! m84: std::http::parse_request — raw request bytes →
//! Request { method, path, version, body }.
//!
//! The parser is intentionally minimal: split on the first
//! `\r\n` to get the request line, split that on the first
//! two spaces to get METHOD/PATH/VERSION, and pull the body
//! from after the first `\r\n\r\n`. Headers between the
//! request line and the blank line are skipped — they aren't
//! surfaced on Request in v0.
//!
//! Tests cover the canonical GET/POST shape, body extraction,
//! and a few malformed inputs that should yield empty fields
//! rather than crashing.

use std::process::Command;

use aperio_codegen::build_executable;

fn build_and_run(name: &str, source: &str) -> (String, std::process::ExitStatus) {
    let program = aperio_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("aperio_test_http_req_{}", name));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (String::from_utf8_lossy(&output.stdout).to_string(), output.status)
}

#[test]
fn parse_get_request_line_no_headers() {
    // Minimal GET — no headers, no body. The double CRLF
    // terminator is conventional but absent here; the parser
    // should still extract method/path/version cleanly from
    // the single-line input.
    let src = r#"
        fn main() {
            let raw = "GET /hello HTTP/1.1\r\n\r\n";
            let req = std::http::parse_request(raw);
            println("method=", req.method);
            println("path=", req.path);
            println("version=", req.version);
            println("body=[", req.body, "]");
        }
    "#;
    let (stdout, status) = build_and_run("get_minimal", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(
        stdout.contains("method=GET"),
        "got: {:?}",
        stdout
    );
    assert!(
        stdout.contains("path=/hello"),
        "got: {:?}",
        stdout
    );
    assert!(
        stdout.contains("version=HTTP/1.1"),
        "got: {:?}",
        stdout
    );
    assert!(
        stdout.contains("body=[]"),
        "expected empty body; got: {:?}",
        stdout
    );
}

#[test]
fn parse_post_with_body_extracts_body_after_blank_line() {
    // POST with headers + body. The parser skips headers
    // (Content-Type / Content-Length) and pulls everything
    // after the `\r\n\r\n` terminator into body.
    let src = r#"
        fn main() {
            let raw = "POST /submit HTTP/1.1\r\nContent-Type: text/plain\r\nContent-Length: 5\r\n\r\nhello";
            let req = std::http::parse_request(raw);
            println("method=", req.method);
            println("path=", req.path);
            println("body=[", req.body, "]");
        }
    "#;
    let (stdout, status) = build_and_run("post_body", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(
        stdout.contains("method=POST"),
        "got: {:?}",
        stdout
    );
    assert!(
        stdout.contains("path=/submit"),
        "got: {:?}",
        stdout
    );
    assert!(
        stdout.contains("body=[hello]"),
        "got: {:?}",
        stdout
    );
}

#[test]
fn parse_request_with_query_string_keeps_query_in_path() {
    // The parser treats path as everything between method and
    // version — including query string. Splitting on `?` is a
    // future router concern, not a parser concern.
    let src = r#"
        fn main() {
            let raw = "GET /search?q=lotus&page=2 HTTP/1.1\r\n\r\n";
            let req = std::http::parse_request(raw);
            println("path=", req.path);
        }
    "#;
    let (stdout, status) = build_and_run("with_query", src);
    assert!(status.success());
    assert!(
        stdout.contains("path=/search?q=lotus&page=2"),
        "got: {:?}",
        stdout
    );
}

#[test]
fn parse_malformed_no_crlf_yields_empty_fields() {
    // No `\r\n` at all — parser returns a fully-empty Request.
    // User code can detect this via `req.method == ""` and
    // emit a 400.
    let src = r#"
        fn main() {
            let raw = "garbage with no crlf at all";
            let req = std::http::parse_request(raw);
            println("method=[", req.method, "]");
            println("path=[", req.path, "]");
            println("version=[", req.version, "]");
        }
    "#;
    let (stdout, status) = build_and_run("malformed_no_crlf", src);
    assert!(status.success());
    assert!(
        stdout.contains("method=[]"),
        "got: {:?}",
        stdout
    );
    assert!(
        stdout.contains("path=[]"),
        "got: {:?}",
        stdout
    );
    assert!(
        stdout.contains("version=[]"),
        "got: {:?}",
        stdout
    );
}

#[test]
fn parse_malformed_no_space_in_request_line() {
    // Has CRLF but the request line has no space — method ends
    // up empty, no path/version.
    let src = r#"
        fn main() {
            let raw = "JUSTONEWORD\r\n\r\n";
            let req = std::http::parse_request(raw);
            println("method=[", req.method, "]");
            println("path=[", req.path, "]");
        }
    "#;
    let (stdout, status) = build_and_run("malformed_no_space", src);
    assert!(status.success());
    // sp1 < 0 path: returns method="", path="", version="".
    assert!(
        stdout.contains("method=[]"),
        "got: {:?}",
        stdout
    );
    assert!(
        stdout.contains("path=[]"),
        "got: {:?}",
        stdout
    );
}

#[test]
fn parse_request_line_with_one_space_only() {
    // METHOD + space + PATH but no version. After-method has
    // no space; version stays empty, path captures everything
    // after the first space.
    let src = r#"
        fn main() {
            let raw = "GET /no-version\r\n\r\n";
            let req = std::http::parse_request(raw);
            println("method=", req.method);
            println("path=", req.path);
            println("version=[", req.version, "]");
        }
    "#;
    let (stdout, status) = build_and_run("one_space", src);
    assert!(status.success());
    assert!(stdout.contains("method=GET"), "got: {:?}", stdout);
    assert!(stdout.contains("path=/no-version"), "got: {:?}", stdout);
    assert!(
        stdout.contains("version=[]"),
        "got: {:?}",
        stdout
    );
}

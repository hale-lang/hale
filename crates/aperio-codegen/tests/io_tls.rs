//! `std::io::tls::*` — system-OpenSSL-backed client-side TLS.
//!
//! Exercises the four primitives end-to-end against a real public
//! HTTPS endpoint:
//!   - `connect(host, port) -> Int fallible(IoError)`
//!   - `send_bytes(handle, b: Bytes) -> Int`
//!   - `recv_bytes(handle, max: Int) -> Bytes`
//!   - `close(handle) -> Int`
//!
//! Marked `#[ignore]` so it doesn't fire on every workspace test
//! run: requires network, DNS, and the system trust store. Run
//! explicitly with `cargo test --release -p aperio-codegen --test
//! io_tls -- --ignored --test-threads=1`.
//!
//! The build path itself is exercised by every build via the
//! unconditional `-lssl -lcrypto` link line and the `lotus_tls.c`
//! translation unit being compiled into every Aperio binary, so a
//! basic syntax / link regression would surface in the broader
//! suite even if this network test is skipped.

use std::process::Command;

use aperio_codegen::build_executable;

fn build_and_run(name: &str, src: &str) -> (String, std::process::ExitStatus) {
    let program = aperio_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("aperio_io_tls_{}_{}", name, std::process::id()));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status,
    )
}

#[test]
#[ignore = "requires network + DNS + system trust store"]
fn https_get_example_com_returns_200() {
    let src = r#"
        fn main() {
            let h = std::io::tls::connect("example.com", 443) or raise;
            let req = std::bytes::from_string(
                "GET / HTTP/1.0\r\nHost: example.com\r\nConnection: close\r\n\r\n"
            );
            let _ = std::io::tls::send_bytes(h, req);
            let resp = std::io::tls::recv_bytes(h, 256);
            let s = std::str::from_bytes(resp);
            let nl = std::str::index_of(s, "\r\n");
            if nl > 0 {
                println(s[0..nl]);
            }
            std::io::tls::close(h);
        }
    "#;
    let (stdout, status) = build_and_run("get_example", src);
    assert!(status.success(), "non-zero: {:?}", status);
    // example.com's status line is "HTTP/1.1 200 OK" — assert on
    // the 200 since the rest is server-dependent.
    assert!(
        stdout.contains("200"),
        "expected 200 status in first line; got: {:?}",
        stdout
    );
}

#[test]
#[ignore = "requires network + DNS + system trust store"]
fn handshake_failure_returns_fallible_error() {
    // Connecting on the wrong port (e.g. port 80 expecting plain
    // HTTP) causes the TLS handshake to fail. The fallible(IoError)
    // surface should fire on the `or raise`.
    let src = r#"
        fn main() {
            let h = std::io::tls::connect("example.com", 80) or {
                println("connect_err");
                return;
            };
            println("unexpected ok handle=" + h);
        }
    "#;
    let (stdout, status) = build_and_run("bad_port", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(
        stdout.contains("connect_err"),
        "expected fallible-err branch; got: {:?}",
        stdout
    );
    assert!(
        !stdout.contains("unexpected ok"),
        "should NOT have reached success branch: {:?}",
        stdout
    );
}

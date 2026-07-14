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
//! explicitly with `cargo test --release -p hale-codegen --test
//! io_tls -- --ignored --test-threads=1`.
//!
//! The build path itself is exercised by every build via the
//! unconditional `-lssl -lcrypto` link line and the `lotus_tls.c`
//! translation unit being compiled into every Hale binary, so a
//! basic syntax / link regression would surface in the broader
//! suite even if this network test is skipped.

use std::process::Command;

use hale_codegen::build_executable;

fn build_and_run(name: &str, src: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_io_tls_{}_{}", name, std::process::id()));
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
fn upgrade_bad_fd_returns_fallible_error() {
    // `upgrade` wraps an already-connected fd in a TLS session. A
    // negative fd (-1) hits `lotus_tls_upgrade`'s `raw_fd < 0` arg-
    // validation guard and returns EINVAL *before* any SSL_new /
    // SSL_connect work happens — it does NOT exercise the handshake-
    // failure path (SSL_new OK, BIO wired, SSL_connect fails); see
    // `upgrade_failure_leaves_fd_open` for that. This test only
    // pins down that the fallible(IoError) surface fires correctly
    // for the argument-guard path. No network / DNS / trust store
    // needed, so this runs in the default (non-ignored) suite and is
    // the fast regression guard for the `upgrade` plumbing/lowering.
    let src = r#"
        fn main() {
            let h = std::io::tls::upgrade(-1, "example.com", true) or {
                println("upgrade_err");
                return;
            };
            println("unexpected ok handle=" + h);
        }
    "#;
    let (stdout, status) = build_and_run("upgrade_bad_fd", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(
        stdout.contains("upgrade_err"),
        "expected fallible-err branch; got: {:?}",
        stdout
    );
    assert!(
        !stdout.contains("unexpected ok"),
        "should NOT have reached success branch: {:?}",
        stdout
    );
}

#[test]
fn upgrade_failure_leaves_fd_open() {
    // Pin-down test for `lotus_tls_upgrade`'s headline fd-ownership
    // contract (see the fn header comment in lotus_tls.c, ~line
    // 285-291): "upgrade does NOT close raw_fd on failure — the
    // caller already owned the fd before this call ... so ownership
    // and teardown stay with the caller." That contract previously
    // had zero test coverage.
    //
    // `upgrade_bad_fd_returns_fallible_error` above only exercises
    // the `raw_fd < 0` EINVAL argument guard, which returns before
    // any TLS/socket work begins. To drive the REAL handshake-
    // failure path (SSL_new OK, BIO wired, SSL_connect fails,
    // SSL_free, fd left open) without any network access, use
    // `std::io::tcp::listen_socket` to get a valid fd that is bound
    // and listening but never `connect()`-ed and never accepted on —
    // i.e. a valid fd that is not a TLS peer. `SSL_connect`'s
    // ClientHello send hits `send(2)` on that fd, which fails with
    // ENOTCONN (the socket was never connected), so the handshake
    // fails for a genuine I/O reason rather than an argument-
    // validation short-circuit.
    //
    // After the fallible branch fires, close the fd ourselves and
    // assert the close succeeds (0) — proving `upgrade` left it
    // open rather than closing it out from under the caller (had
    // upgrade already closed it, closing again would fail).
    let src = r#"
        fn main() {
            let listen_fd = std::io::tcp::listen_socket("127.0.0.1", 47847) or raise;
            let h = std::io::tls::upgrade(listen_fd, "x", false) or {
                println("upgrade_err");
                let close_ret = std::io::tcp::close_fd(listen_fd);
                println("close_ret=" + close_ret);
                return;
            };
            println("unexpected ok handle=" + h);
        }
    "#;
    let (stdout, status) = build_and_run("upgrade_failure_leaves_fd_open", src);
    assert!(status.success(), "non-zero: {:?}\nstdout: {}", status, stdout);
    assert!(
        stdout.contains("upgrade_err"),
        "expected the fallible-err branch (real handshake failure via a \
         listening, never-connected fd — not the bad-fd arg guard); got: {:?}",
        stdout
    );
    assert!(
        !stdout.contains("unexpected ok"),
        "should NOT have reached success branch: {:?}",
        stdout
    );
    // The headline assertion: closing the fd AFTER upgrade's failure
    // path succeeds, proving upgrade did not already close it.
    assert!(
        stdout.contains("close_ret=0"),
        "expected close_fd(listen_fd) to succeed (0) after upgrade's failure \
         path — a non-zero/negative result would mean upgrade already closed \
         the fd out from under the caller; got: {:?}",
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

#[test]
#[ignore = "requires network + DNS + system trust store"]
fn upgrade_verify_true_reproduces_connect_200() {
    // Manually dial a plain TCP socket with std::io::tcp::connect,
    // then upgrade that fd to a *verified* TLS session. connect is now
    // internally dial + upgrade(verify=1), so getting the same 200-OK
    // result here regression-proves the refactor is behavior-
    // preserving and that upgrade+verify goes through the system trust
    // store exactly as connect did.
    let src = r#"
        fn main() {
            let fd = std::io::tcp::connect("example.com", 443) or raise;
            let h = std::io::tls::upgrade(fd, "example.com", true) or {
                std::io::tcp::close_fd(fd);
                println("upgrade_err");
                return;
            };
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
    let (stdout, status) = build_and_run("upgrade_verify_true", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(
        !stdout.contains("upgrade_err"),
        "verified upgrade should have handshaked; got: {:?}",
        stdout
    );
    assert!(
        stdout.contains("200"),
        "expected 200 status in first line; got: {:?}",
        stdout
    );
}

#[test]
#[ignore = "requires network + DNS + system trust store"]
fn upgrade_verify_false_handshakes() {
    // verify=false skips peer authentication (sslmode=require
    // semantics — encrypt without checking the cert chain against the
    // trust store), the mode used against endpoints whose CA is not in
    // the system store (e.g. AWS RDS). It must still complete the
    // handshake and exchange application data; SNI is still sent.
    let src = r#"
        fn main() {
            let fd = std::io::tcp::connect("example.com", 443) or raise;
            let h = std::io::tls::upgrade(fd, "example.com", false) or {
                std::io::tcp::close_fd(fd);
                println("upgrade_err");
                return;
            };
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
    let (stdout, status) = build_and_run("upgrade_verify_false", src);
    assert!(status.success(), "non-zero: {:?}", status);
    assert!(
        !stdout.contains("upgrade_err"),
        "unverified upgrade should still handshake; got: {:?}",
        stdout
    );
    assert!(
        stdout.contains("200"),
        "expected 200 status in first line; got: {:?}",
        stdout
    );
}

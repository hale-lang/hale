//! v1.x polish (2026-05-20): `std::http::Server` should bail
//! cleanly on `__listen_socket` failure (e.g. EADDRINUSE)
//! instead of proceeding with `listen_fd = -1` and looping
//! on `accept: Bad file descriptor` at full speed.
//!
//! Test strategy: a Rust-side TcpListener takes the port
//! WITHOUT SO_REUSEPORT, so the Aperio Server's bind (which
//! sets SO_REUSEPORT) cannot share it. Aperio's birth()
//! detects `listen_fd < 0` and violates `listen_failed`,
//! the unhandled violation bubbles past `main`, and the
//! process exits non-zero with the closure-violation
//! diagnostic on stderr.

use std::net::TcpListener;
use std::process::Command;

use aperio_codegen::build_executable;

#[test]
fn http_server_violates_on_bind_failure() {
    // Take a port. The Rust listener doesn't set SO_REUSEPORT,
    // so the Aperio Server's bind on the same port fails.
    let listener = TcpListener::bind("127.0.0.1:0").expect("rust bind");
    let port = listener.local_addr().expect("local_addr").port();

    let src = format!(
        r#"
        locus EchoHandler {{
            params {{ }}
            fn handle(req: std::http::Request) -> std::http::Response {{
                let _ = req;
                return std::http::Response {{ status: 200, body: "ok" }};
            }}
        }}
        fn main() {{
            std::http::Server {{
                host: "127.0.0.1",
                port: {port},
                handler: EchoHandler {{ }},
                max_accepts: 1,
            }};
        }}
    "#,
        port = port,
    );

    let prog = aperio_syntax::parse_source(&src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!(
        "aperio_test_http_bind_fail_{}_{}",
        std::process::id(),
        port,
    ));
    build_executable(&prog, &bin).expect("build");

    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    drop(listener); // release port

    // The Server should fail to bind and violate. The
    // unaddressed violation propagates past main → process
    // exits non-zero.
    assert!(
        !out.status.success(),
        "Server should violate on bind failure, but exited 0. \
         stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    // The closure-violation root panic format includes the
    // closure name. We also want NO `accept: Bad file
    // descriptor` spam (the old failure mode).
    assert!(
        stderr.contains("listen_failed"),
        "expected listen_failed violation diagnostic, got stderr:\n{}",
        stderr
    );
    assert!(
        !stderr.contains("accept: Bad file descriptor"),
        "Server should NOT have entered the EBADF accept loop. \
         stderr:\n{}",
        stderr
    );
}

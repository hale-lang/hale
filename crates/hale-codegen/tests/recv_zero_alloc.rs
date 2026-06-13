//! recv plaintext-allocation audit + gate (#6 of the fast-protocol-I/O
//! substrate plan).
//!
//! Audit finding (by code, runtime/lotus_tls.c + lotus_arena.c):
//! `lotus_tls_recv_into` and `lotus_tcp_recv_into` are zero-alloc on the
//! Hale side — `SSL_read` / `read` write straight into the caller's
//! reserved buffer; there is no per-record malloc in the binding. The only
//! per-record TLS allocation is OpenSSL-internal, governed by
//! `SSL_MODE_RELEASE_BUFFERS` (set in the process SSL_CTX): a deliberate
//! memory-frugality tradeoff (no ~32 KiB resident per idle connection, at
//! the cost of a re-malloc when a released buffer is next read). To get
//! zero per-record malloc on an always-busy latency-critical connection,
//! clear that mode (retain the buffers) — the lever a future
//! `std::io::tls` knob would expose; not built here (no consumer yet).
//!
//! This gate pins the binding's zero-alloc property: a `recv_into`
//! path-call (no locus-method scratch frame) on a quiet loopback socket,
//! into a builder already at capacity, does zero heap allocations per call
//! — measured with the `std::diag` gate counter.
//!
//! The real per-record TLS measurement needs a network host + a trusted
//! cert (loopback TLS is blocked by mandatory SSL_VERIFY_PEER). Manual
//! recipe: connect to an HTTPS host, GET, and read
//! `std::diag::heap_alloc_count()` around each `recv_into` — it stays a
//! small bounded constant per record (the OpenSSL RELEASE_BUFFERS
//! re-malloc), independent of record count.

use hale_codegen::build_executable;
use std::process::Command;

fn build_and_run_argv(name: &str, src: &str, argv: &[&str]) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_recv0_{}", name));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).args(argv).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (String::from_utf8_lossy(&out.stdout).to_string(), out.status)
}

fn pick_free_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind probe");
    l.local_addr().expect("local_addr").port()
}

#[test]
fn recv_into_binding_is_zero_alloc() {
    let port = pick_free_port();
    // Loopback, quiet socket (nothing sent), short recv timeout. The
    // builder starts already large enough for `max`, so `recv_into`'s
    // reserve never reallocs; each recv times out (-2) without advancing,
    // so used never grows. The binding does no malloc of its own, so
    // heap_alloc_count must not move across 300 recv_into calls.
    let src = r#"
        fn main() {
            let port = std::str::parse_int(std::env::arg(1)) or raise;
            let lfd = std::io::tcp::__listen_socket("127.0.0.1", port);
            let cfd = std::io::tcp::__connect("127.0.0.1", port);
            let afd = std::io::tcp::__accept_one(lfd);
            std::io::tcp::set_recv_timeout(afd, 5ms) or raise;
            let bld = std::bytes::BytesBuilder { initial_cap: 65536 };
            // Warm-up so any first-touch is outside the measured window.
            let _ = std::io::tcp::recv_into(afd, bld, 16384);
            let a0 = std::diag::heap_alloc_count();
            let mut i = 0;
            while i < 300 {
                let _ = std::io::tcp::recv_into(afd, bld, 16384);
                i = i + 1;
            }
            let a1 = std::diag::heap_alloc_count();
            println("recv_alloc_delta=", a1 - a0, " avail=", a0 >= 0);
        }
    "#;
    let (out, status) = build_and_run_argv("tcp", src, &[&port.to_string()]);
    assert!(status.success(), "exit {:?}\n{}", status, out);
    assert!(out.contains("avail=true"), "gate must be available; got: {:?}", out);
    assert!(
        out.contains("recv_alloc_delta=0"),
        "the recv_into binding must do zero heap allocations per call; got: {:?}",
        out
    );
}

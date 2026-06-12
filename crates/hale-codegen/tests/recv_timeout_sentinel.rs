//! Part A of the WsClient-liveness fix: a recv timeout must make
//! `recv_into` return a *distinguishable, non-fatal* sentinel (-2)
//! on a `SO_RCVTIMEO` timeout, rather than -1 ("fatal").
//!
//! The TCP path is tested end-to-end on loopback (no certs needed):
//! set a short recv timeout on a connected socket, `recv_into` with
//! no data arriving, and assert it returns -2 after the timeout —
//! not -1 (which a caller would treat as a dead connection) and not
//! a hang. The TLS siblings (`std::io::tls::set_recv_timeout` +
//! `recv_into`'s `SSL_ERROR_WANT_READ → -2`) share the same C
//! helper / sentinel pattern and are verified to compile + link.

use std::process::Command;

use hale_codegen::build_executable;

fn pick_free_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind probe");
    l.local_addr().expect("local_addr").port()
}

fn build_and_run_argv(name: &str, src: &str, argv: &[&str]) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_recv_timeout_{}", name));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).args(argv).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status,
    )
}

#[test]
fn tcp_recv_into_returns_minus_two_on_timeout() {
    let port = pick_free_port();
    // Loopback: listen, connect, accept (single-threaded — connect
    // completes via the listen backlog before accept returns). Set a
    // 200ms recv timeout on the connected fd, then recv with nothing
    // sent → SO_RCVTIMEO fires → recv_into returns -2.
    let src = r#"
        fn main() {
            let port = std::str::parse_int(std::env::arg(1)) or raise;
            let lfd = std::io::tcp::__listen_socket("127.0.0.1", port);
            let cfd = std::io::tcp::__connect("127.0.0.1", port);
            let afd = std::io::tcp::__accept_one(lfd);
            std::io::tcp::set_recv_timeout(cfd, 200ms) or raise;
            let b = std::bytes::BytesBuilder { };
            let got = std::io::tcp::recv_into(cfd, b, 256);
            println("got=", got);
        }
    "#;
    let (stdout, status) = build_and_run_argv("tcp_timeout", src, &[&port.to_string()]);
    assert!(status.success(), "exit: {:?}\nstdout: {}", status, stdout);
    assert!(
        stdout.contains("got=-2"),
        "expected recv_into to return the -2 timeout sentinel (not -1/fatal, \
         not a hang); got: {:?}",
        stdout
    );
}

#[test]
fn tls_recv_timeout_surface_compiles_and_links() {
    // Can't open a real TLS connection in a unit test, but the
    // dispatch (`std::io::tls::set_recv_timeout` fallible path) +
    // the `lotus_tls_set_recv_timeout_ns` extern must resolve and
    // link. A program that references both — gated behind a handle
    // that's never valid at runtime so it doesn't actually fire —
    // proves the surface exists end to end.
    let src = r#"
        fn main() {
            let h = std::str::parse_int(std::env::arg(1)) or 0;
            if h > 0 {
                std::io::tls::set_recv_timeout(h, 5s) or raise;
                std::io::tls::set_send_timeout(h, 5s) or raise;
            }
            println("ok");
        }
    "#;
    let (stdout, status) = build_and_run_argv("tls_compile", src, &["0"]);
    assert!(status.success(), "exit: {:?}\nstdout: {}", status, stdout);
    assert!(stdout.contains("ok"), "got: {:?}", stdout);
}

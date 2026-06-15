//! TLS fast-path siblings (2026-06-14): `std::io::tls::set_nodelay`,
//! `set_rx_timestamps`, `recv_stamped_into`, `last_recv_{kernel,user}_ns`.
//!
//! These mirror the tcp fast-path knobs over a TLS handle — the handle
//! resolves to the underlying socket fd, so `set_nodelay` / SO_TIMESTAMPNS
//! reuse the plain-tcp primitives, and `recv_stamped_into` is `recv_into`
//! plus a kernel-timestamp capture. The kernel stamp rides the *socket*
//! recvmsg but `SSL_read` sits in front, so rather than swap OpenSSL's BIO
//! (which would touch every TLS read), the stamp is pulled with
//! `SIOCGSTAMPNS` after the ordinary `SSL_read` socket read.
//!
//! A real TLS connection needs network + a trusted cert (loopback TLS is
//! blocked by mandatory `SSL_VERIFY_PEER`), so the CI test only proves the
//! surface compiles + links; the real per-record probe is `#[ignore]`.

use hale_codegen::build_executable;
use std::process::Command;

fn build_and_run_argv(name: &str, src: &str, argv: &[&str]) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_tlsfast_{}", name));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).args(argv).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (String::from_utf8_lossy(&out.stdout).to_string(), out.status)
}

#[test]
fn tls_fast_io_surface_compiles_and_links() {
    // Gate the calls behind a handle that's never valid at runtime so the
    // program links every TLS fast-path extern without opening a connection.
    let src = r#"
        fn main() {
            let h = std::str::parse_int(std::env::arg(1)) or 0;
            if h > 0 {
                std::io::tls::set_nodelay(h, true) or raise;
                std::io::tls::set_rx_timestamps(h, true) or raise;
                let bld = std::bytes::BytesBuilder { };
                let n = std::io::tls::recv_stamped_into(h, bld, 4096);
                let k = std::io::tls::last_recv_kernel_ns();
                let u = std::io::tls::last_recv_user_ns();
                println("n=", n, " k=", k, " u=", u);
            }
            println("ok");
        }
    "#;
    let (out, status) = build_and_run_argv("surface", src, &["0"]);
    assert!(status.success(), "exit {:?}\n{}", status, out);
    assert!(out.contains("ok"), "got: {:?}", out);
}

#[test]
#[ignore = "requires network + DNS + system trust store; real TLS recv_stamped probe"]
fn tls_recv_stamped_over_real_host() {
    // Connect to a real HTTPS host, enable nodelay + RX timestamps, GET, and
    // recv_stamped the response. Confirms the recv_stamped path doesn't break
    // TLS (decrypts the response) and captures the userspace stamp; the
    // kernel stamp is best-effort (0 unless the path supports RX timestamping).
    let src = r#"
        fn main() {
            let h = std::io::tls::connect("example.com", 443) or { println("connect_fail"); return; };
            std::io::tls::set_nodelay(h, true) or raise;
            std::io::tls::set_rx_timestamps(h, true) or raise;
            let req = std::bytes::from_string("GET / HTTP/1.0\r\nHost: example.com\r\nConnection: close\r\n\r\n");
            let _ = std::io::tls::send_bytes(h, req);
            let bld = std::bytes::BytesBuilder { initial_cap: 65536 };
            let n = std::io::tls::recv_stamped_into(h, bld, 16384);
            let u = std::io::tls::last_recv_user_ns();
            std::io::tls::close(h);
            println("recv_n=", n, " user_ok=", u > 0);
        }
    "#;
    let (out, status) = build_and_run_argv("realhost", src, &[]);
    assert!(status.success(), "exit {:?}\n{}", status, out);
    assert!(out.contains("recv_n="), "got: {:?}", out);
    assert!(out.contains("user_ok=true"), "userspace stamp must be captured; got: {:?}", out);
}

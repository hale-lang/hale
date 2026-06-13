//! `std::io::tcp::recv_stamped_into` (#1 of the fast-protocol-I/O substrate
//! plan): one recvmsg(2) that delivers the payload *and* the kernel's
//! SCM_TIMESTAMPNS RX timestamp, with no extra syscall on the hot path
//! (SO_TIMESTAMPNS is the one-time `set_rx_timestamps` opt-in).
//!
//! End-to-end on loopback: enable RX timestamps on the accepted fd, send a
//! message, recv it stamped, and assert — via the gate counter from #137 —
//! that it was exactly one recvmsg, the bytes arrived, and the userspace
//! stamp was taken.
//!
//! The *kernel* timestamp is best-effort: loopback TCP does not generate a
//! software RX timestamp in many kernels (verified — `SO_TIMESTAMPNS` and
//! `SO_TIMESTAMPING` both deliver an empty control message over loopback),
//! so `last_recv_kernel_ns()` is `0` here and populated only on a real NIC
//! path with RX timestamping. We assert it is non-negative (graceful 0,
//! never garbage), not that it is positive.

use std::process::Command;

use hale_codegen::build_executable;

fn pick_free_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind probe");
    l.local_addr().expect("local_addr").port()
}

fn build_and_run_argv(name: &str, src: &str, argv: &[&str]) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_recv_stamped_{}", name));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).args(argv).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status,
    )
}

#[test]
fn recv_stamped_one_recvmsg_with_kernel_timestamp() {
    let port = pick_free_port();
    let src = r#"
        fn main() {
            let port = std::str::parse_int(std::env::arg(1)) or raise;
            let lfd = std::io::tcp::__listen_socket("127.0.0.1", port);
            let cfd = std::io::tcp::__connect("127.0.0.1", port);
            let afd = std::io::tcp::__accept_one(lfd);
            std::io::tcp::set_rx_timestamps(afd, true) or raise;
            std::io::tcp::set_recv_timeout(afd, 500ms) or raise;
            std::io::tcp::__send(cfd, "hello-stamped");
            let m0 = std::diag::syscall_count("recvmsg");
            let bld = std::bytes::BytesBuilder { };
            let got = std::io::tcp::recv_stamped_into(afd, bld, 256);
            let m1 = std::diag::syscall_count("recvmsg");
            let kns = std::io::tcp::last_recv_kernel_ns();
            let uns = std::io::tcp::last_recv_user_ns();
            println("got=", got, " recvmsgs=", m1 - m0,
                    " kernel_nonneg=", kns >= 0, " user_ok=", uns > 0);
        }
    "#;
    let (stdout, status) = build_and_run_argv("basic", src, &[&port.to_string()]);
    assert!(status.success(), "exit: {:?}\nstdout: {}", status, stdout);
    assert!(stdout.contains("got=13"), "should receive the 13-byte message; got: {:?}", stdout);
    assert!(
        stdout.contains("recvmsgs=1"),
        "one recv_stamped_into must be exactly one recvmsg syscall; got: {:?}",
        stdout
    );
    assert!(
        stdout.contains("kernel_nonneg=true"),
        "the kernel RX timestamp must be a graceful 0 when undelivered (loopback), never garbage; got: {:?}",
        stdout
    );
    assert!(
        stdout.contains("user_ok=true"),
        "the userspace stamp must be set; got: {:?}",
        stdout
    );
}

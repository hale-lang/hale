//! `std::io::tcp::set_nodelay(fd, on)` toggles Nagle (TCP_NODELAY) on a
//! connected socket, and `std::io::sockopt::TCP_NODELAY()` names the option
//! so the effect can be read back via the fd-generic `get_option_int`.
//!
//! End-to-end on loopback (no certs): listen, connect, accept, then flip
//! TCP_NODELAY on and off on the connected fd and read the kernel's view
//! back each time. Asserts the round-trip — proving the new constant, the
//! new setter, and the actual setsockopt all line up, not just that the
//! call returned 0.

use std::process::Command;

use hale_codegen::build_executable;

fn pick_free_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind probe");
    l.local_addr().expect("local_addr").port()
}

fn build_and_run_argv(name: &str, src: &str, argv: &[&str]) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_tcp_nodelay_{}", name));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).args(argv).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status,
    )
}

#[test]
fn set_nodelay_toggles_and_reads_back() {
    let port = pick_free_port();
    let src = r#"
        fn main() {
            let port = std::str::parse_int(std::env::arg(1)) or raise;
            let lfd = std::io::tcp::__listen_socket("127.0.0.1", port);
            let cfd = std::io::tcp::__connect("127.0.0.1", port);
            let afd = std::io::tcp::__accept_one(lfd);
            let lvl = std::io::sockopt::IPPROTO_TCP();
            let opt = std::io::sockopt::TCP_NODELAY();
            std::io::tcp::set_nodelay(cfd, true) or raise;
            let enabled = std::io::udp::get_option_int(cfd, lvl, opt) or raise;
            std::io::tcp::set_nodelay(cfd, false) or raise;
            let disabled = std::io::udp::get_option_int(cfd, lvl, opt) or raise;
            println("nodelay on=", enabled, " off=", disabled);
        }
    "#;
    let (stdout, status) = build_and_run_argv("toggle", src, &[&port.to_string()]);
    assert!(status.success(), "exit: {:?}\nstdout: {}", status, stdout);
    assert!(
        stdout.contains("nodelay on=1 off=0"),
        "expected TCP_NODELAY to read back 1 after set_nodelay(true) and 0 after \
         set_nodelay(false); got: {:?}",
        stdout
    );
}

//! Item 4 (downstream handoff 2026-07-15) — a TCP listener binds
//! EXCLUSIVELY: two live binds of the same host:port must NOT both
//! succeed.
//!
//! `lotus_tcp_listen_socket` used to set SO_REUSEPORT unconditionally,
//! which lets two live processes both bind the same port and have the
//! kernel round-robin connections between them — a second server booted
//! by accident got no error and clients were silently split-brained
//! across two processes with divergent state. Dropping SO_REUSEPORT
//! (SO_REUSEADDR alone still covers the restart-within-TIME_WAIT case)
//! makes the second bind fail loudly, matching the Go/Rust reference
//! backends.
//!
//! Two live listen sockets on the same port in ONE process is the same
//! kernel refusal a second process would hit — SO_REUSEADDR does not
//! permit it, only SO_REUSEPORT did. So the first `__listen_socket`
//! succeeds and the second returns the -1 error sentinel.

use std::process::Command;

use hale_codegen::build_executable;

fn free_tcp_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind probe");
    l.local_addr().expect("local_addr").port()
}

#[test]
fn second_bind_of_same_port_fails() {
    let port = free_tcp_port();
    let src = format!(
        r#"
        fn main() {{
            let p = {port};
            let fd1 = std::io::tcp::__listen_socket("127.0.0.1", p);
            println("fd1_ok=", fd1 >= 0);
            // Second live bind of the same port: with SO_REUSEPORT gone
            // this must be refused by the kernel (EADDRINUSE → -1).
            let fd2 = std::io::tcp::__listen_socket("127.0.0.1", p);
            println("fd2_ok=", fd2 >= 0);
        }}
        "#,
    );
    let program = hale_syntax::parse_source(&src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale-listener-exclusive-{}", std::process::id()));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);

    assert!(
        stdout.contains("fd1_ok=true"),
        "first bind should succeed; got:\n{}",
        stdout
    );
    assert!(
        stdout.contains("fd2_ok=false"),
        "second live bind of the same port must be refused (exclusive bind — \
         SO_REUSEPORT must not be set); got:\n{}",
        stdout
    );
}

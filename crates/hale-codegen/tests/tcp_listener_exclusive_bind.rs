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

#[path = "support/harness.rs"]
mod harness;

fn free_tcp_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind probe");
    l.local_addr().expect("local_addr").port()
}

#[test]
fn second_bind_of_same_port_fails() {
    // Acquiring a port and binding it in a CHILD are two steps, and
    // nothing holds the port between them: `free_tcp_port` binds
    // 127.0.0.1:0, reads the assigned port, and drops the listener so
    // the child can bind it. In a parallel run another test can take
    // that port in the gap, and the child's FIRST bind fails — which
    // looks like this test's subject failing when it is really port
    // allocation losing a race. Observed in CI 2026-08-03.
    //
    // The gap cannot be closed: holding the port is exactly what
    // stops the child binding it. So the acquire-and-bind pair is
    // retried as a unit. The property under test — a SECOND bind of
    // the same live port is refused — is unaffected by which port we
    // end up on.
    let mut last = String::new();
    for _ in 0..8 {
        let stdout = run_double_bind(free_tcp_port());
        if stdout.contains("fd1_ok=true") {
            assert!(
                stdout.contains("fd2_ok=false"),
                "a second bind of a live port must be refused \
                 (SO_REUSEPORT is deliberately not set); got:\n{}",
                stdout
            );
            return;
        }
        last = stdout;
    }
    panic!(
        "could not acquire a bindable port in 8 attempts — this is \
         port-allocation contention, not the property under test. \
         Last output:\n{}",
        last
    );
}

/// Build and run a program that binds `port` twice; return stdout.
fn run_double_bind(port: u16) -> String {
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
    let bin = harness::unique_bin(&format!("hale-listener-exclusive-{}", std::process::id()));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    String::from_utf8_lossy(&out.stdout).to_string()
}

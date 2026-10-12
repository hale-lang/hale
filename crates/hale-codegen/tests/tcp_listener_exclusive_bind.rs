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

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;
#[path = "support/child.rs"]
mod child;

#[test]
fn second_bind_of_same_port_fails() {
    // The program binds one port twice, so it is told the port before
    // it binds. A port drawn and released could be bound by another
    // test of a parallel run before the program's FIRST bind (observed
    // in CI 2026-08-03), so the address is reserved for the test's
    // length instead: the first bind is then always this program's,
    // and only the second is the property under test.
    let at = child::Reserved::new();
    let stdout = run_double_bind(&at.host(), at.port());
    assert!(stdout.contains("fd1_ok=true"), "the reserved address binds once; got:\n{stdout}");
    assert!(
        stdout.contains("fd2_ok=false"),
        "a second bind of a live port must be refused \
         (SO_REUSEPORT is deliberately not set); got:\n{}",
        stdout
    );
}

/// Build and run a program that binds `host:port` twice; return stdout.
fn run_double_bind(host: &str, port: u16) -> String {
    let src = format!(
        r#"
        fn main() {{
            let p = {port};
            let fd1 = std::io::tcp::__listen_socket("{host}", p);
            println("fd1_ok=", fd1 >= 0);
            // Second live bind of the same port: with SO_REUSEPORT gone
            // this must be refused by the kernel (EADDRINUSE → -1).
            let fd2 = std::io::tcp::__listen_socket("{host}", p);
            println("fd2_ok=", fd2 >= 0);
        }}
        "#,
    );
    let bin = harness::unique_bin(&format!("hale-listener-exclusive-{}", std::process::id()));
    build_opts::build_source(&src, &bin, &build_opts::options()).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    String::from_utf8_lossy(&out.stdout).to_string()
}

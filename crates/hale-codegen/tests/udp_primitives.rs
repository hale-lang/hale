//! std::io::udp::* raw networking primitives.
//!
//! Smoke test: bind a UDP socket on loopback, send a datagram
//! to it from a separately-bound sender socket, verify the
//! receiver gets the bytes. Round-trip over loopback is reliable
//! enough for a non-flaky test (the "best-effort" semantics of
//! UDP show up on contended networks, not on localhost between
//! two sockets in the same process).

use std::process::Command;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;
#[path = "support/child.rs"]
mod child;

fn build_and_run(name: &str, source: &str) -> (String, String, std::process::ExitStatus) {
    let bin = harness::unique_bin(name);
    build_opts::build_source(source, &bin, &build_opts::options()).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status,
    )
}

#[test]
fn send_recv_round_trip_over_loopback() {
    // The program sends to its own receiver, so the receiver's address is
    // in the source before it binds: a reserved one, which no other test
    // of a parallel run can bind in between. (A port drawn here and handed
    // over is free, not reserved; this test used to retry on a fresh one
    // when another test took it first.)
    let reserved = child::Reserved::new();
    let (host, port) = (reserved.host(), reserved.port());
    // Receiver binds first, sender binds ephemeral port (port 0
    // = OS-assigned), then sends. Receiver loops on recv,
    // converts bytes back to a String for comparison.
    let src = format!(
        r#"
        fn main() {{
            let recv_fd = std::io::udp::bind("{host}", {port}) or raise;
            let send_fd = std::io::udp::bind("", 0) or raise;
            std::io::udp::send(send_fd, "{host}", {port}, "hello, hale udp") or raise;
            let blob = std::io::udp::recv(recv_fd, 1024) or raise;
            let s = std::str::from_bytes(blob);
            print("got=");
            println(s);
            std::io::udp::close(send_fd);
            std::io::udp::close(recv_fd);
        }}
    "#
    );
    let (stdout, stderr, status) = build_and_run("rt", &src);
    assert!(status.success(), "exit: {:?}\nstderr: {}\nstdout: {:?}", status, stderr, stdout);
    assert!(stdout.contains("got=hello, hale udp"), "stdout: {:?}", stdout);
}

#[test]
fn bind_with_invalid_host_surfaces_invalid_kind() {
    let src = r#"
        fn try_bind() -> Int fallible(IoError) {
            let fd = std::io::udp::bind("not.a.valid.host", 0) or raise;
            return fd;
        }

        fn main() {
            let r = try_bind() or {
                println("kind=", err.kind);
                -1
            };
            println("r=", r);
        }
    "#;
    let (stdout, stderr, status) = build_and_run("badhost", src);
    assert!(status.success(), "exit: {:?}\nstderr: {}", status, stderr);
    assert!(
        stdout.contains("kind=invalid"),
        "expected kind=invalid for non-numeric host; got: {:?}",
        stdout,
    );
}

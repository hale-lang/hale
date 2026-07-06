//! std::io::udp::* raw networking primitives.
//!
//! Smoke test: bind a UDP socket on 127.0.0.1, send a datagram
//! to it from a separately-bound sender socket, verify the
//! receiver gets the bytes. Round-trip over loopback is reliable
//! enough for a non-flaky test (the "best-effort" semantics of
//! UDP show up on contended networks, not on localhost between
//! two sockets in the same process).

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use hale_codegen::build_executable;

fn pick_free_port() -> u16 {
    // We can't bind a UDP socket here easily without pulling in
    // a UDP crate, but TcpListener also surfaces an OS-assigned
    // port. Race risk: between drop(probe) and the test's UDP
    // bind, another process could claim the port. SO_REUSEADDR
    // on the UDP bind side reduces the rebind window.
    let probe = std::net::TcpListener::bind("127.0.0.1:0").expect("bind probe");
    let port = probe.local_addr().expect("local_addr").port();
    drop(probe);
    port
}

fn build_and_run(name: &str, source: &str) -> (String, String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!(
        "hale_udp_test_{}_{}_{}",
        name,
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    ));
    build_executable(&program, &bin).expect("build");
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
    // `pick_free_port` has an unavoidable TOCTOU window: between
    // dropping the probe and the compiled program's UDP bind, a
    // concurrent test process (nextest runs test binaries in
    // parallel) can claim the port, so the bind fails and the
    // program exits nonzero. That's a pure port race, not a logic
    // bug — retry with a FRESH port a few times. The round-trip is
    // deterministic once the bind succeeds, so a real failure still
    // surfaces (all attempts fail identically), while the race
    // needs several consecutive collisions to leak through.
    let mut last: Option<(String, String, std::process::ExitStatus)> = None;
    for attempt in 0..5 {
        let port = pick_free_port();
        // Receiver binds first, sender binds ephemeral port (port 0
        // = OS-assigned), then sends. Receiver loops on recv,
        // converts bytes back to a String for comparison.
        let src = format!(
            r#"
        fn main() {{
            let recv_fd = std::io::udp::bind("127.0.0.1", {port}) or raise;
            let send_fd = std::io::udp::bind("", 0) or raise;
            std::io::udp::send(send_fd, "127.0.0.1", {port}, "hello, hale udp") or raise;
            let blob = std::io::udp::recv(recv_fd, 1024) or raise;
            let s = std::str::from_bytes(blob);
            print("got=");
            println(s);
            std::io::udp::close(send_fd);
            std::io::udp::close(recv_fd);
        }}
    "#,
            port = port,
        );
        let (stdout, stderr, status) = build_and_run("rt", &src);
        if status.success() && stdout.contains("got=hello, hale udp") {
            return; // clean round-trip
        }
        eprintln!(
            "udp round-trip attempt {} failed (likely port {} race), retrying",
            attempt, port
        );
        last = Some((stdout, stderr, status));
    }
    let (stdout, stderr, status) = last.expect("at least one attempt");
    panic!(
        "udp round-trip failed after 5 attempts; \
         exit: {:?}\nstderr: {}\nstdout: {:?}",
        status, stderr, stdout
    );
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

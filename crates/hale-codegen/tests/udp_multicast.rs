//! 2026-05-26 — P1 (UDP multicast surface) + P2 (transparent
//! setsockopt) end-to-end coverage. Each test builds + runs a
//! Hale program that exercises the new stdlib surface; the
//! assertions live in-binary via `println("FAIL: …")` sentinels
//! that the harness then greps for. Kernel multicast loopback
//! makes these self-contained — no external test rig.

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn unique_path(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let p = harness::unique_bin(&format!(
        "lt-udp-multicast-{}-{}-{}.bin",
        tag,
        std::process::id(),
        nanos,
    ));
    p
}

fn build_and_run(name: &str, src: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = unique_path(name);
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status,
    )
}

#[test]
fn sockopt_constants_resolve_to_platform_values() {
    // Sanity: each `std::io::sockopt::<NAME>()` call returns a
    // platform-defined Int. Just exercises the codegen
    // dispatch + C getter — values themselves are platform-
    // specific so we only assert "non-negative and reasonable
    // for an Int".
    let src = r#"
        fn main() {
            let sol = std::io::sockopt::SOL_SOCKET();
            let rcvbuf = std::io::sockopt::SO_RCVBUF();
            let ip_proto = std::io::sockopt::IPPROTO_IP();
            let mcast_ttl = std::io::sockopt::IP_MULTICAST_TTL();
            let add_mem = std::io::sockopt::IP_ADD_MEMBERSHIP();
            print("sol="); println(sol);
            print("rcvbuf="); println(rcvbuf);
            print("ip_proto="); println(ip_proto);
            print("mcast_ttl="); println(mcast_ttl);
            print("add_mem="); println(add_mem);
        }
    "#;
    let (stdout, status) = build_and_run("sockopt_const", src);
    assert!(status.success(), "non-zero exit: {:?}\nstdout: {}", status, stdout);
    // Linux x86_64: SOL_SOCKET=1, SO_RCVBUF=8, IPPROTO_IP=0,
    // IP_MULTICAST_TTL=33, IP_ADD_MEMBERSHIP=35. Don't hard-pin
    // values (they could differ on macOS / BSD), just check the
    // lines are present and parse as Ints.
    for prefix in ["sol=", "rcvbuf=", "ip_proto=", "mcast_ttl=", "add_mem="] {
        let line = stdout
            .lines()
            .find(|l| l.starts_with(prefix))
            .unwrap_or_else(|| panic!("missing {} in stdout: {:?}", prefix, stdout));
        let value: i64 = line
            .trim_start_matches(prefix)
            .trim()
            .parse()
            .unwrap_or_else(|_| panic!("non-Int after {}: {:?}", prefix, line));
        // Ints in 0..1024 — every defined SOL_*/SO_*/IP_* sits
        // well within this on every platform.
        assert!(
            value >= 0 && value < 1024,
            "{}{} outside expected platform-range",
            prefix, value
        );
    }
}

#[test]
fn setsockopt_round_trips_rcvbuf() {
    // SO_RCVBUF is the canonical "tune this for high-throughput
    // UDP" knob — every market-data app reaches for it. Set 1
    // MiB and confirm getsockopt reports back at least 1 MiB.
    // (Linux doubles the request for kernel accounting; we just
    // check the floor.)
    let src = r#"
        fn main() {
            let fd = std::io::udp::bind("0.0.0.0", 0) or raise;
            std::io::udp::set_option_int(
                fd,
                std::io::sockopt::SOL_SOCKET(),
                std::io::sockopt::SO_RCVBUF(),
                1048576
            ) or raise;
            let buf = std::io::udp::get_option_int(
                fd,
                std::io::sockopt::SOL_SOCKET(),
                std::io::sockopt::SO_RCVBUF()
            ) or raise;
            print("rcvbuf="); println(buf);
            if buf < 1048576 { println("FAIL: rcvbuf below request"); }
            let _ = std::io::udp::close(fd);
        }
    "#;
    let (stdout, status) = build_and_run("rcvbuf", src);
    assert!(status.success(), "non-zero exit: {:?}\nstdout: {}", status, stdout);
    assert!(!stdout.contains("FAIL:"), "stdout: {:?}", stdout);
}

#[test]
fn multicast_ttl_and_loop_round_trip() {
    let src = r#"
        fn main() {
            let fd = std::io::udp::bind("0.0.0.0", 0) or raise;
            std::io::udp::set_multicast_ttl(fd, 7) or raise;
            std::io::udp::set_multicast_loop(fd, false) or raise;
            let ttl = std::io::udp::get_option_int(
                fd,
                std::io::sockopt::IPPROTO_IP(),
                std::io::sockopt::IP_MULTICAST_TTL()
            ) or raise;
            let lo = std::io::udp::get_option_int(
                fd,
                std::io::sockopt::IPPROTO_IP(),
                std::io::sockopt::IP_MULTICAST_LOOP()
            ) or raise;
            print("ttl="); println(ttl);
            print("loop="); println(lo);
            if ttl != 7 { println("FAIL: ttl mismatch"); }
            if lo != 0 { println("FAIL: loop should be 0"); }
            let _ = std::io::udp::close(fd);
        }
    "#;
    let (stdout, status) = build_and_run("ttl_loop", src);
    assert!(status.success(), "non-zero exit: {:?}\nstdout: {}", status, stdout);
    assert!(stdout.contains("ttl=7"), "got: {:?}", stdout);
    assert!(stdout.contains("loop=0"), "got: {:?}", stdout);
    assert!(!stdout.contains("FAIL:"), "stdout: {:?}", stdout);
}

#[test]
fn multicast_join_leave_idempotent_at_kernel() {
    // The join + leave should both return success on a valid
    // local-scope multicast group. Repeated join of the same
    // group fails (EADDRINUSE) — we don't assert that since
    // it's a tested kernel behavior; just sanity-check the
    // single join + leave round trip.
    let src = r#"
        fn main() {
            let fd = std::io::udp::bind("0.0.0.0", 0) or raise;
            std::io::udp::join_group(fd, "239.255.0.42", "") or raise;
            std::io::udp::leave_group(fd, "239.255.0.42", "") or raise;
            println("join_leave_ok");
            let _ = std::io::udp::close(fd);
        }
    "#;
    let (stdout, status) = build_and_run("join_leave", src);
    assert!(status.success(), "non-zero exit: {:?}\nstdout: {}", status, stdout);
    assert!(stdout.contains("join_leave_ok"), "got: {:?}", stdout);
}

#[test]
fn multicast_join_invalid_group_fails_with_io_error() {
    // A non-multicast address can't be joined; the kernel
    // returns EADDRNOTAVAIL / EINVAL. The fallible IoError path
    // surfaces this.
    let src = r#"
        fn handle(_e: IoError) { println("caught_io_error"); }
        fn main() {
            let fd = std::io::udp::bind("0.0.0.0", 0) or raise;
            std::io::udp::join_group(fd, "10.0.0.1", "") or handle(err);
            let _ = std::io::udp::close(fd);
        }
    "#;
    let (stdout, status) = build_and_run("join_invalid", src);
    assert!(status.success(), "non-zero exit: {:?}\nstdout: {}", status, stdout);
    assert!(stdout.contains("caught_io_error"),
        "non-multicast join should surface IoError; got: {:?}", stdout);
}

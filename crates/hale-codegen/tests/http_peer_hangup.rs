//! A client that hangs up before it reads the response is not a fault
//! of the program (downstream handoff, 2026-11-06): the runtime wrote an
//! ERROR record on stderr for every such write (`lotus.tcp` "write
//! failed", EPIPE or ECONNRESET), outside the program's `std::log` sinks
//! and under no level filter. The peer hanging up is now a DEBUG record,
//! written only under `HALE_LOG=debug`; any other errno stays ERROR.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Command, Stdio};
use std::time::Duration;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;
#[path = "support/child.rs"]
mod child;

// A handler that answers late, so the client is gone when the response
// is written.
const SLOW_SERVER: &str = r#"
    locus Slow {
        fn handle(req: std::http::Request) -> std::http::Response {
            std::time::sleep(300ms);
            return std::http::Response { status: 200, body: std::str::repeat("x", 65536) };
        }
    }
    fn main() {
        std::http::Server { host: "127.0.0.1", port: 0, handler: Slow { }, ready_signal: "ready" };
    }
"#;

/// Close `s` with a reset (SO_LINGER 0): the server's next write to it
/// fails with ECONNRESET or EPIPE at once.
fn hang_up(s: TcpStream) {
    use std::os::unix::io::AsRawFd;
    let l = libc::linger { l_onoff: 1, l_linger: 0 };
    let r = unsafe {
        libc::setsockopt(
            s.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_LINGER,
            &l as *const libc::linger as *const libc::c_void,
            std::mem::size_of::<libc::linger>() as libc::socklen_t,
        )
    };
    assert_eq!(r, 0, "SO_LINGER");
    drop(s);
}

/// Serve, take one request from a client that hangs up before the
/// answer, and return what the server wrote on stderr.
fn stderr_after_a_hangup(bin: &std::path::Path, env: &[(&str, &str)]) -> String {
    let mut cmd = Command::new(bin);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().expect("spawn the server");
    let stdout = child.stdout.take().unwrap();
    let mut ready = String::new();
    let _ = BufReader::new(stdout).read_line(&mut ready);
    assert_eq!(ready.trim(), "ready", "the server did not start");
    let port = child::tcp_port(&mut child);

    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    s.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n").expect("write the request");
    hang_up(s);
    // the handler answers after 300 ms, to nobody
    std::thread::sleep(Duration::from_millis(800));
    // a second request, served whole: the server is still fine
    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect again");
    s.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n").expect("write the second request");
    let mut resp = Vec::new();
    s.read_to_end(&mut resp).expect("read the second response");
    assert!(String::from_utf8_lossy(&resp).starts_with("HTTP/1.1 200"), "the server did not serve after the hang-up");

    let mut stderr = child.stderr.take().unwrap();
    let _ = child.kill();
    let _ = child.wait();
    let mut err = String::new();
    let _ = stderr.read_to_string(&mut err);
    err
}

#[test]
fn a_client_hanging_up_is_not_an_error_record() {
    let bin = harness::unique_bin("http_peer_hangup");
    build_opts::build_source(SLOW_SERVER, &bin, &build_opts::options()).expect("build");

    let err = stderr_after_a_hangup(&bin, &[]);
    assert!(!err.contains("level=error"), "an ERROR record for a peer that hung up:\n{err}");
    assert!(!err.contains("write failed"), "the hang-up was written by default:\n{err}");

    // under HALE_LOG=debug the same condition is a DEBUG record, so it
    // can be seen when asked for
    let err = stderr_after_a_hangup(&bin, &[("HALE_LOG", "debug")]);
    let _ = std::fs::remove_file(&bin);
    let line = err
        .lines()
        .find(|l| l.contains("write failed"))
        .unwrap_or_else(|| panic!("no record of the hang-up under HALE_LOG=debug:\n{err}"));
    assert!(line.contains("level=debug") && line.contains("logger=lotus.tcp"), "{line}");
    assert!(line.contains("errno=32") || line.contains("errno=104"), "EPIPE or ECONNRESET: {line}");
}

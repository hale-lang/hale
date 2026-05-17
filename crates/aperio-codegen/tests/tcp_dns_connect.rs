//! C6 (pond follow-up) — DNS fallback in `std::io::tcp::connect`.
//!
//! Pre-C6 `lotus_tcp_connect` only accepted IPv4 dotted-quad hosts
//! via `inet_pton`; a hostname like `"localhost"` or `"httpbin.org"`
//! failed with `errno = EINVAL`. C6 keeps the numeric fast-path
//! bit-for-bit identical and falls back to `getaddrinfo` (AF_INET +
//! SOCK_STREAM) when `inet_pton` rejects the host. The fallible
//! signature is unchanged: `connect(host, port) -> Int
//! fallible(IoError)`. gai errors map onto the existing IoError
//! taxonomy without introducing a new kind: EAI_NONAME → ENOENT
//! ("not_found"), everything else → EHOSTUNREACH ("host_unreachable").
//!
//! Driver: `pond/http/client` (`FRICTION.md` § "No DNS").

use std::io::Read;
use std::net::TcpListener;
use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use aperio_codegen::build_executable;

fn build_aperio(name: &str, source: &str) -> std::path::PathBuf {
    let program = aperio_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!(
        "aperio_test_tcp_dns_{}_{}",
        name,
        std::process::id()
    ));
    build_executable(&program, &bin).expect("build");
    bin
}

fn pick_free_port() -> u16 {
    let probe = TcpListener::bind("127.0.0.1:0").expect("bind probe");
    probe.local_addr().expect("local_addr").port()
}

#[test]
fn connect_numeric_host_still_uses_fast_path() {
    // Fast-path regression guard: `127.0.0.1` is dotted-quad, so
    // inet_pton matches and getaddrinfo is never called. If this
    // breaks, the C6 refactor lost the numeric branch.
    let port = pick_free_port();
    let listener = TcpListener::bind(("127.0.0.1", port)).expect("bind");
    let (got_tx, got_rx) = mpsc::channel::<Vec<u8>>();
    thread::spawn(move || {
        let (mut sock, _) = listener.accept().expect("accept");
        let mut buf = Vec::new();
        let _ = sock.read_to_end(&mut buf);
        let _ = got_tx.send(buf);
    });

    let src = format!(
        r#"
        fn main() {{
            let fd = std::io::tcp::connect("127.0.0.1", {}) or raise;
            let s = std::io::tcp::Stream {{ conn_fd: fd }};
            s.send("hello-numeric");
        }}
        "#,
        port
    );
    let bin = build_aperio("numeric", &src);
    let out = Command::new(&bin).output().expect("run aperio");
    let received = got_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("server read");
    let _ = std::fs::remove_file(&bin);

    assert!(
        out.status.success(),
        "exit: {:?}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(received, b"hello-numeric");
}

#[test]
fn connect_localhost_resolves_via_getaddrinfo() {
    // DNS-fallback path: "localhost" isn't a dotted quad, so
    // inet_pton returns 0 and getaddrinfo resolves it (typically
    // to 127.0.0.1 from /etc/hosts; if a sandbox has no nsswitch
    // hosts entry this would fail — but that's the exact case
    // we want to surface as IoError below).
    let port = pick_free_port();
    let listener = TcpListener::bind(("127.0.0.1", port)).expect("bind");
    let (got_tx, got_rx) = mpsc::channel::<Vec<u8>>();
    thread::spawn(move || {
        let (mut sock, _) = listener.accept().expect("accept");
        let mut buf = Vec::new();
        let _ = sock.read_to_end(&mut buf);
        let _ = got_tx.send(buf);
    });

    let src = format!(
        r#"
        fn main() {{
            let fd = std::io::tcp::connect("localhost", {}) or raise;
            let s = std::io::tcp::Stream {{ conn_fd: fd }};
            s.send("hello-dns");
        }}
        "#,
        port
    );
    let bin = build_aperio("localhost", &src);
    let out = Command::new(&bin).output().expect("run aperio");
    let received = got_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("server read");
    let _ = std::fs::remove_file(&bin);

    assert!(
        out.status.success(),
        "exit: {:?}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(received, b"hello-dns");
}

#[test]
fn connect_unresolvable_host_surfaces_ioerror_kind() {
    // EAI_NONAME → ENOENT → kind "not_found". The .invalid TLD is
    // reserved by RFC 2606 specifically for this kind of negative-
    // resolution test (DNS servers MUST NOT resolve it).
    //
    // Tolerance: some misconfigured resolvers return SERVFAIL
    // (mapped to host_unreachable) instead of NXDOMAIN. Accept
    // both so the test doesn't depend on local DNS quirks.
    let src = r#"
        fn report(e: IoError) -> Int {
            println("kind=", e.kind);
            return -1;
        }
        fn main() {
            let fd = std::io::tcp::connect("nonexistent.invalid", 80)
                or report(err);
            println("fd=", fd);
        }
    "#;
    let bin = build_aperio("unresolvable", src);
    let out = Command::new(&bin).output().expect("run aperio");
    let _ = std::fs::remove_file(&bin);
    assert!(
        out.status.success(),
        "exit: {:?}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("kind=not_found") || stdout.contains("kind=host_unreachable"),
        "expected one of {{not_found, host_unreachable}}; got: {:?}",
        stdout
    );
}

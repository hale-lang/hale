//! TLS parks on an `async_io` pool (friction sweep 2026-10-12, downstream
//! handoff): only `tls::recv_into` / `recv_stamped_into` parked; the
//! handshake (`connect`, `upgrade`) and `recv_bytes` blocked the worker
//! thread and `send_bytes` was a plain `SSL_write` on it, so the
//! `std::http` https client — which uses exactly those — had to stay off
//! `async_io` pools. Now every TLS wait is a park there: the handshake and
//! the writes and reads run on the non-blocking socket, and each
//! `SSL_ERROR_WANT_READ` / `WANT_WRITE` parks the coroutine until the
//! socket is ready.
//!
//! The peer is a TLS server in this process (the `openssl` crate the
//! runtime's system OpenSSL already is): a self-signed certificate for
//! `localhost` made here, written to a file the child trusts through
//! OpenSSL's own `SSL_CERT_FILE` (the client's default verify paths read
//! it), so `tls::connect`'s full verification — trust store and hostname —
//! runs unchanged. The server delays its handshake or its response on
//! purpose; a parked client keeps the pool's other coroutines running
//! through those delays, a blocking one stalls them.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use openssl::asn1::Asn1Time;
use openssl::bn::BigNum;
use openssl::ec::{EcGroup, EcKey};
use openssl::hash::MessageDigest;
use openssl::nid::Nid;
use openssl::pkey::{PKey, Private};
use openssl::ssl::{SslAcceptor, SslMethod};
use openssl::x509::extension::SubjectAlternativeName;
use openssl::x509::{X509NameBuilder, X509};

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

/// A self-signed P-256 certificate for `localhost`, valid for a day.
fn self_signed() -> (PKey<Private>, X509) {
    let group = EcGroup::from_curve_name(Nid::X9_62_PRIME256V1).unwrap();
    let key = PKey::from_ec_key(EcKey::generate(&group).unwrap()).unwrap();
    let mut name = X509NameBuilder::new().unwrap();
    name.append_entry_by_text("CN", "localhost").unwrap();
    let name = name.build();
    let mut b = X509::builder().unwrap();
    b.set_version(2).unwrap();
    b.set_serial_number(&BigNum::from_u32(1).unwrap().to_asn1_integer().unwrap()).unwrap();
    b.set_subject_name(&name).unwrap();
    b.set_issuer_name(&name).unwrap();
    b.set_pubkey(&key).unwrap();
    b.set_not_before(&Asn1Time::days_from_now(0).unwrap()).unwrap();
    b.set_not_after(&Asn1Time::days_from_now(1).unwrap()).unwrap();
    let san = SubjectAlternativeName::new().dns("localhost").build(&b.x509v3_context(None, None)).unwrap();
    b.append_extension(san).unwrap();
    b.sign(&key, MessageDigest::sha256()).unwrap();
    (key, b.build())
}

/// How the server answers: it waits `handshake_delay` before it takes
/// the handshake and `response_delay` between the request and the
/// response. `speak: false` accepts the TCP connection and never says a
/// word (a handshake that never completes); `read: false` completes the
/// handshake and then never reads (a peer whose receive window fills).
#[derive(Clone, Copy)]
struct Behaviour {
    handshake_delay: Duration,
    response_delay: Duration,
    speak: bool,
    read: bool,
}

/// A TLS (or, unspoken, a silent TCP) server on 127.0.0.1, a thread per
/// connection. The kernel picks its port. Dropping it stops the accept
/// loop and removes the certificate file.
struct Peer {
    port: u16,
    cert: PathBuf,
    stop: Arc<AtomicBool>,
}

impl Peer {
    fn start(how: Behaviour) -> Peer {
        let (key, cert) = self_signed();
        let cert_path = harness::unique_bin("tls_park_cert").with_extension("pem");
        std::fs::write(&cert_path, cert.to_pem().unwrap()).unwrap();
        let mut acc = SslAcceptor::mozilla_intermediate_v5(SslMethod::tls()).unwrap();
        acc.set_private_key(&key).unwrap();
        acc.set_certificate(&cert).unwrap();
        let acceptor = Arc::new(acc.build());
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        thread::spawn(move || {
            let mut held = Vec::new();
            for conn in listener.incoming() {
                if stopping.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(tcp) = conn else { continue };
                if !how.speak {
                    held.push(tcp);
                    continue;
                }
                let acceptor = acceptor.clone();
                thread::spawn(move || {
                    thread::sleep(how.handshake_delay);
                    let Ok(mut s) = acceptor.accept(tcp) else { return };
                    if !how.read {
                        thread::sleep(Duration::from_secs(20));
                        return;
                    }
                    let mut req = Vec::new();
                    let mut chunk = [0u8; 1024];
                    while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                        match s.read(&mut chunk) {
                            Ok(0) | Err(_) => return,
                            Ok(n) => req.extend_from_slice(&chunk[..n]),
                        }
                    }
                    thread::sleep(how.response_delay);
                    let body = b"hello-tls";
                    let head = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = s.write_all(head.as_bytes());
                    let _ = s.write_all(body);
                    let _ = s.shutdown();
                });
            }
        });
        Peer { port, cert: cert_path, stop }
    }
}

impl Drop for Peer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        let _ = std::fs::remove_file(&self.cert);
    }
}

/// Build `src` (with `options`), run it with the port as its argument and
/// the peer's certificate as its trust store; its stdout.
fn run(name: &str, src: &str, peer: &Peer, options: &hale_codegen::BuildOptions) -> (String, String) {
    let bin = harness::unique_bin(name);
    build_opts::build_source(src, &bin, options).expect("build");
    let out = Command::new("timeout")
        .arg("60")
        .arg(&bin)
        .arg(peer.port.to_string())
        .env("SSL_CERT_FILE", &peer.cert)
        .env("ASAN_OPTIONS", "detect_leaks=0")
        .env("LOTUS_NO_CHUNK_POOL", "1")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(out.status.success(), "{name}: {:?}; stdout: {stdout}; stderr: {stderr}", out.status);
    (stdout, stderr)
}

/// The `<key>=<v>` value on the line that starts with `tag`.
fn field(out: &str, tag: &str, key: &str) -> String {
    let line = out
        .lines()
        .find(|l| l.starts_with(tag))
        .unwrap_or_else(|| panic!("no `{tag}` line in {out:?}"));
    line.split_whitespace()
        .find_map(|w| w.strip_prefix(&format!("{key}=")))
        .unwrap_or_else(|| panic!("no `{key}=` on {line:?}"))
        .to_string()
}

fn ms(out: &str, tag: &str) -> u64 {
    field(out, tag, "ms").parse().expect("ms is a number")
}

/// A host on one `async_io` pool that starts a `Job` coroutine per
/// message: kind 0 ticks (`ticks` 50 ms sleeps, then prints when it
/// finished), kind 1 makes an https GET to `localhost:<port>`, kind 2
/// dials `tls::connect_within(.., 500ms)`. `App` publishes `jobs` and
/// waits `wait`.
fn pool_program(ticks: u32, jobs: &str, wait: &str) -> String {
    format!(
        r#"
        type Go {{ kind: Int; port: Int; n: Int; }}

        locus Job {{
            params {{ kind: Int = 0; port: Int = 0; n: Int = 0; t0: Int = 0; }}
            run() {{
                if self.kind == 0 {{
                    let mut i = 0;
                    while i < {ticks} {{
                        std::time::sleep(50ms);
                        i = i + 1;
                    }}
                    println("ticks done ms=", (std::time::monotonic_ns() - self.t0) / 1000000);
                }} else if self.kind == 1 {{
                    let r = std::http::get("https://localhost:" + to_string(self.port) + "/") or {{
                        println("failed n=", self.n, " kind=", err.kind, " detail=", err.detail);
                        std::http::ClientResponse {{ status: 0, headers: "", body: b"" }}
                    }};
                    println("https n=", self.n, " status=", r.status, " body=", std::str::from_bytes(r.body), " ms=", (std::time::monotonic_ns() - self.t0) / 1000000);
                }} else {{
                    let mut kind = "";
                    let h = std::io::tls::connect_within("localhost", self.port, 500ms) or {{ kind = err.kind; - 1 }};
                    println("dial h=", h, " kind=", kind, " ms=", (std::time::monotonic_ns() - self.t0) / 1000000);
                }}
            }}
        }}

        locus Host {{
            params {{ t0: Int = 0; }}
            bus {{ subscribe "go" as on_go of type Go; }}
            accept(c: Job) {{ }}
            fn on_go(g: Go) {{
                Job {{ kind: g.kind, port: g.port, n: g.n, t0: self.t0 }};
            }}
        }}

        main locus App {{
            params {{
                t0: Int = 0;
                host: Host = Host {{ }};
            }}
            placement {{
                host: cooperative(pool = io) where async_io;
            }}
            bus {{ publish "go" of type Go; }}
            run() {{
                let port = std::str::parse_int(std::env::arg(1)) or 0;
                self.host.t0 = self.t0;
                {jobs}
                std::time::sleep({wait});
            }}
        }}

        fn main() {{
            App {{ t0: std::time::monotonic_ns() }};
            return 0;
        }}
    "#
    )
}

/// Off any pool the client is what it was: the blocking handshake with
/// full verification (the test's certificate through `SSL_CERT_FILE`),
/// `send_bytes`, `recv_bytes`; and a handshake the peer never answers ends
/// at `connect_within`'s deadline with kind `timeout`.
#[test]
fn off_pool_the_handshake_verifies_and_a_silent_peer_ends_at_the_deadline() {
    let peer = Peer::start(Behaviour { handshake_delay: Duration::ZERO, response_delay: Duration::ZERO, speak: true, read: true });
    let silent = Peer::start(Behaviour { handshake_delay: Duration::ZERO, response_delay: Duration::ZERO, speak: false, read: true });
    let src = format!(
        r#"
        fn main() {{
            let port = std::str::parse_int(std::env::arg(1)) or 0;
            let h = std::io::tls::connect("localhost", port) or raise;
            let sent = std::io::tls::send_bytes(h, std::bytes::from_string("GET / HTTP/1.0\r\nHost: localhost\r\n\r\n"));
            let mut got = "";
            let mut piece = std::str::from_bytes(std::io::tls::recv_bytes(h, 4096));
            while len(piece) > 0 {{
                got = got + piece;
                piece = std::str::from_bytes(std::io::tls::recv_bytes(h, 4096));
            }}
            std::io::tls::close(h);
            println("plain sent=", sent, " ok=", std::str::starts_with(got, "HTTP/1.1 200"), " body=", std::str::ends_with(got, "hello-tls"));
            let t = std::time::monotonic_ns();
            let mut kind = "";
            let s = std::io::tls::connect_within("localhost", {silent}, 300ms) or {{ kind = err.kind; - 1 }};
            println("silent h=", s, " kind=", kind, " ms=", (std::time::monotonic_ns() - t) / 1000000);
        }}
    "#,
        silent = silent.port
    );
    let (out, _) = run("hale_tls_off_pool", &src, &peer, &build_opts::options());
    assert_eq!(field(&out, "plain", "sent"), "0", "{out}");
    assert_eq!(field(&out, "plain", "ok"), "true", "{out}");
    assert_eq!(field(&out, "plain", "body"), "true", "{out}");
    assert_eq!(field(&out, "silent", "kind"), "timeout", "{out}");
    let took = ms(&out, "silent");
    assert!((280..1300).contains(&took), "the handshake must end at its 300 ms deadline; took {took}: {out}");
}

/// The friction's own shape: an https `std::http::get` on an async_io
/// pool beside a coroutine that ticks. The peer holds the handshake
/// 400 ms and the response another 400 ms. A handshake or read that
/// held the worker would freeze the ticker through both; parked, the
/// ticker's twelve 50 ms sleeps finish before the response arrives.
#[test]
fn an_https_request_parks_through_its_handshake_and_its_read() {
    let peer = Peer::start(Behaviour {
        handshake_delay: Duration::from_millis(400),
        response_delay: Duration::from_millis(400),
        speak: true,
        read: true,
    });
    let src = pool_program(
        12,
        r#""go" <- Go { kind: 1, port: port, n: 0 };
                "go" <- Go { kind: 0, port: 0, n: 0 };"#,
        "2500ms",
    );
    let (out, stderr) = run("hale_tls_park_https", &src, &peer, &build_opts::options());
    assert_eq!(field(&out, "https", "status"), "200", "{out}\n{stderr}");
    assert_eq!(field(&out, "https", "body"), "hello-tls", "{out}");
    let req = ms(&out, "https");
    let ticks = ms(&out, "ticks");
    assert!(req >= 780, "the peer's delays are 800 ms: {out}");
    assert!(
        ticks < req,
        "the ticker must finish its 600 ms of sleeps while the request \
         waits on the peer (a blocking handshake or read freezes it until \
         the response): ticks done at {ticks} ms, response at {req} ms: {out}"
    );
}

/// The handshake's deadline on an async_io pool is a timed park: a peer
/// that never answers ends `tls::connect_within` at 500 ms with kind
/// `timeout`, and the ticker beside it keeps running meanwhile.
#[test]
fn a_tls_dial_deadline_is_a_timed_park_on_async_io() {
    let silent = Peer::start(Behaviour { handshake_delay: Duration::ZERO, response_delay: Duration::ZERO, speak: false, read: true });
    let src = pool_program(
        6,
        r#""go" <- Go { kind: 2, port: port, n: 0 };
                "go" <- Go { kind: 0, port: 0, n: 0 };"#,
        "1500ms",
    );
    let (out, _) = run("hale_tls_park_deadline", &src, &silent, &build_opts::options());
    assert_eq!(field(&out, "dial", "kind"), "timeout", "{out}");
    let dial = ms(&out, "dial");
    let ticks = ms(&out, "ticks");
    assert!((480..1400).contains(&dial), "the dial ends at its 500 ms deadline: {out}");
    assert!(ticks < dial, "the ticker runs through the pending handshake: {out}");
}

/// Eight https requests on one async_io pool, each answered 300 ms after
/// it is read. Parked, they wait together and all answer in about 300 ms;
/// held, the worker serves them one after another (2.4 s).
fn concurrent(name: &str, options: &hale_codegen::BuildOptions) -> String {
    let peer = Peer::start(Behaviour {
        handshake_delay: Duration::ZERO,
        response_delay: Duration::from_millis(300),
        speak: true,
        read: true,
    });
    let jobs: String = (0..8)
        .map(|n| format!("\"go\" <- Go {{ kind: 1, port: port, n: {n} }};\n                "))
        .collect();
    let src = pool_program(1, &jobs, "4000ms");
    let (out, stderr) = run(name, &src, &peer, options);
    assert!(!stderr.contains("AddressSanitizer"), "{stderr}");
    let lines: Vec<&str> = out.lines().filter(|l| l.starts_with("https ")).collect();
    assert_eq!(lines.len(), 8, "eight answers: {out}\n{stderr}");
    for l in &lines {
        assert!(l.contains("status=200") && l.contains("body=hello-tls"), "{l}: {out}");
    }
    out
}

#[test]
fn eight_https_requests_on_one_async_io_pool_wait_together() {
    let out = concurrent("hale_tls_park_concurrent", &build_opts::options());
    let slowest = out
        .lines()
        .filter(|l| l.starts_with("https "))
        .map(|l| field(l, "https", "ms").parse::<u64>().unwrap())
        .max()
        .unwrap();
    assert!(
        slowest < 1200,
        "eight parked requests answer together (~300 ms); one after another \
         would take 2400 ms: the slowest took {slowest} ms: {out}"
    );
}

/// The same eight requests under AddressSanitizer (chunk recycling off):
/// a park swaps coroutines in the middle of `SSL_connect`, `SSL_write` and
/// `SSL_read`, and nothing the swap leaves behind may be read after it is
/// freed.
#[test]
fn eight_parked_https_requests_are_clean_under_asan() {
    let options = hale_codegen::BuildOptions { asan: true, ..build_opts::options() };
    concurrent("hale_tls_park_asan", &options);
}

// ---- the socket's own timeouts bound the parked waits (PR #1538 review) ----
//
// A non-blocking socket ignores SO_SNDTIMEO / SO_RCVTIMEO: the kernel
// applies them only to a call that blocks. The parks read them back and
// bound themselves, so `set_send_timeout` still ends a send to a peer that
// stopped reading, and a recv timeout armed before `upgrade` still ends a
// handshake the peer never answers — as each did on a blocking socket.

/// A TCP listener on 127.0.0.1 that accepts and never reads: a send to it
/// fills the receive window and then the send buffer.
struct Deaf {
    port: u16,
}

impl Deaf {
    fn start() -> Deaf {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            let mut held = Vec::new();
            for conn in listener.incoming().flatten() {
                held.push(conn);
            }
        });
        Deaf { port }
    }
}

const BIG: &str = r#"
        let mut s = "xxxxxxxxxxxxxxxx";
        let mut i = 0;
        while i < 20 {
            s = s + s;
            i = i + 1;
        }
        let big = std::bytes::from_string(s);
"#;

#[test]
fn a_send_to_a_peer_that_stopped_reading_ends_at_its_send_timeout_on_async_io() {
    let stalled = Peer::start(Behaviour {
        handshake_delay: Duration::ZERO,
        response_delay: Duration::ZERO,
        speak: true,
        read: false,
    });
    let deaf = Deaf::start();
    let src = format!(
        r#"
        type Go {{ kind: Int; port: Int; }}

        locus Job {{
            params {{ kind: Int = 0; port: Int = 0; }}
            run() {{
                {BIG}
                if self.kind == 0 {{
                    let h = std::io::tls::connect("localhost", self.port) or raise;
                    std::io::tls::set_send_timeout(h, 20ms) or raise;
                    let t = std::time::monotonic_ns();
                    let sent = std::io::tls::send_bytes(h, big);
                    println("tls sent=", sent, " ms=", (std::time::monotonic_ns() - t) / 1000000);
                    std::io::tls::close(h);
                }} else {{
                    let fd = std::io::tcp::connect("127.0.0.1", self.port) or raise;
                    std::io::tcp::set_send_timeout(fd, 20ms) or raise;
                    let t = std::time::monotonic_ns();
                    let mut kind = "sent";
                    std::io::tcp::send_fd(fd, big) or {{ kind = err.kind; }};
                    println("tcp kind=", kind, " ms=", (std::time::monotonic_ns() - t) / 1000000);
                    std::io::tcp::close_fd(fd);
                }}
            }}
        }}

        locus Host {{
            bus {{ subscribe "go" as on_go of type Go; }}
            accept(c: Job) {{ }}
            fn on_go(g: Go) {{
                Job {{ kind: g.kind, port: g.port }};
            }}
        }}

        main locus App {{
            params {{ host: Host = Host {{ }}; }}
            placement {{
                host: cooperative(pool = io) where async_io;
            }}
            bus {{ publish "go" of type Go; }}
            run() {{
                let port = std::str::parse_int(std::env::arg(1)) or 0;
                "go" <- Go {{ kind: 0, port: port }};
                "go" <- Go {{ kind: 1, port: {deaf} }};
                std::time::sleep(5s);
            }}
        }}

        fn main() {{
            App {{ }};
            return 0;
        }}
    "#,
        deaf = deaf.port
    );
    let (out, stderr) = run("hale_tls_park_send_timeout", &src, &stalled, &build_opts::options());
    assert_eq!(
        field(&out, "tls", "sent"),
        "-1",
        "a TLS send to a peer that stopped reading must fail once SO_SNDTIMEO passes \
         with no progress, not park forever: {out}\n{stderr}"
    );
    assert!(ms(&out, "tls") < 3000, "{out}");
    assert_eq!(
        field(&out, "tcp", "kind"),
        "would_block",
        "a TCP send's SO_SNDTIMEO expiry on an async_io pool is the blocking send's \
         error (EAGAIN): {out}\n{stderr}"
    );
    assert!(ms(&out, "tcp") < 3000, "{out}");
}

#[test]
fn a_recv_timeout_armed_before_upgrade_ends_a_silent_handshake_on_and_off_a_pool() {
    let silent = Peer::start(Behaviour {
        handshake_delay: Duration::ZERO,
        response_delay: Duration::ZERO,
        speak: false,
        read: true,
    });
    let src = r#"
        fn up(port: Int, tag: String) {
            let fd = std::io::tcp::connect("127.0.0.1", port) or raise;
            std::io::tcp::set_recv_timeout(fd, 200ms) or raise;
            let t = std::time::monotonic_ns();
            let mut kind = "";
            let h = std::io::tls::upgrade(fd, "localhost", true) or { kind = err.kind; - 1 };
            println(tag, " h=", h, " kind=", kind, " ms=", (std::time::monotonic_ns() - t) / 1000000);
            std::io::tcp::close_fd(fd);
        }

        type Go { port: Int; }

        locus Job {
            params { port: Int = 0; }
            run() { up(self.port, "pool"); }
        }

        locus Host {
            bus { subscribe "go" as on_go of type Go; }
            accept(c: Job) { }
            fn on_go(g: Go) { Job { port: g.port }; }
        }

        main locus App {
            params { host: Host = Host { }; }
            placement {
                host: cooperative(pool = io) where async_io;
            }
            bus { publish "go" of type Go; }
            run() {
                let port = std::str::parse_int(std::env::arg(1)) or 0;
                up(port, "main");
                "go" <- Go { port: port };
                std::time::sleep(1500ms);
            }
        }

        fn main() {
            App { };
            return 0;
        }
    "#;
    let (out, stderr) = run("hale_tls_upgrade_recv_timeout", src, &silent, &build_opts::options());
    for tag in ["main", "pool"] {
        assert_eq!(field(&out, tag, "kind"), "timeout", "{tag}: {out}\n{stderr}");
        let took = ms(&out, tag);
        assert!((180..1300).contains(&took), "{tag}: the handshake ends at the 200 ms recv timeout; took {took}: {out}");
    }
}

/// A TLS server that answers every request with a Server-Sent Events
/// stream in chunked transfer encoding: one event, a 600 ms gap, then a
/// second event and the last chunk, or, with `hold`, nothing more for
/// 5 s (the connection open).
fn start_streaming(
    hold: bool,
    key: &PKey<Private>,
    cert: &X509,
    cert_path: &std::path::Path,
) -> Peer {
    let mut acc = SslAcceptor::mozilla_intermediate_v5(SslMethod::tls()).unwrap();
    acc.set_private_key(key).unwrap();
    acc.set_certificate(cert).unwrap();
    let acceptor = Arc::new(acc.build());
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    let stop = Arc::new(AtomicBool::new(false));
    let stopping = stop.clone();
    thread::spawn(move || {
        for conn in listener.incoming() {
            if stopping.load(Ordering::SeqCst) {
                break;
            }
            let Ok(tcp) = conn else { continue };
            let acceptor = acceptor.clone();
            thread::spawn(move || {
                let Ok(mut s) = acceptor.accept(tcp) else { return };
                let mut req = Vec::new();
                let mut chunk = [0u8; 1024];
                while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                    match s.read(&mut chunk) {
                        Ok(0) | Err(_) => return,
                        Ok(n) => req.extend_from_slice(&chunk[..n]),
                    }
                }
                let frame = |data: &str| format!("{:x}\r\n{data}\r\n", data.len());
                let _ = s.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\
                      Transfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
                );
                let _ = s.write_all(frame("event: tick\ndata: first\n\n").as_bytes());
                if hold {
                    thread::sleep(Duration::from_secs(5));
                    return;
                }
                thread::sleep(Duration::from_millis(600));
                let _ = s.write_all(frame("data: second\n\n").as_bytes());
                let _ = s.write_all(b"0\r\n\r\n");
                let _ = s.shutdown();
            });
        }
    });
    Peer { port, cert: cert_path.to_path_buf(), stop }
}

/// GH #1698 over https: `std::http::SseReader` hands on the first event
/// as it arrives, 600 ms before the server sends the second, and then the
/// end; a stream the server holds open ends at the reader's `timeout_ms`
/// with the whole-body read's outcome, its reads bounded by the deadline
/// itself (`tls::__recv_into_until`), off any pool and on an async_io one.
#[test]
fn an_https_event_stream_is_read_as_it_arrives_and_its_deadline_holds() {
    // one certificate for both servers, the one the child trusts
    let (key, cert) = self_signed();
    let cert_path = harness::unique_bin("tls_stream_cert").with_extension("pem");
    std::fs::write(&cert_path, cert.to_pem().unwrap()).unwrap();
    let peer = start_streaming(false, &key, &cert, &cert_path);
    let held = start_streaming(true, &key, &cert, &cert_path);
    let body = |port: String| {
        format!(
            r#"
            let t0 = std::time::monotonic_ns();
            let url = std::http::parse_url("https://localhost:" + {port} + "/") or std::http::Url {{ }};
            let s = std::http::SseReader {{ req: std::http::ClientRequest {{ url: url, body: b"" }} }};
            let a = s.event() or std::http::SseEvent {{ data: "!" }};
            let ta = (std::time::monotonic_ns() - t0) / 1000000;
            let b = s.event() or std::http::SseEvent {{ data: "!" }};
            let tb = (std::time::monotonic_ns() - t0) / 1000000;
            let mut kind = "";
            s.event() or {{ kind = err.kind; std::http::SseEvent {{ data: "" }} }};
            println("first status=", s.body.status, " event=", a.event, " data=", a.data, " ms=", ta);
            println("second data=", b.data, " ms=", tb);
            println("after kind=", kind);
            let t1 = std::time::monotonic_ns();
            let hurl = std::http::parse_url("https://localhost:" + std::env::arg(2) + "/") or std::http::Url {{ }};
            let h = std::http::SseReader {{ req: std::http::ClientRequest {{ url: hurl, body: b"" }}, timeout_ms: 800 }};
            let c = h.event() or std::http::SseEvent {{ data: "!" }};
            let mut hkind = "";
            let mut hdetail = "";
            h.event() or {{ hkind = err.kind; hdetail = err.detail; std::http::SseEvent {{ data: "" }} }};
            println("held data=", c.data, " kind=", hkind, " timeout=", std::str::contains(hdetail, "timeout: no complete response within 800 ms"), " ms=", (std::time::monotonic_ns() - t1) / 1000000);
"#
        )
    };
    let off_pool = format!("fn main() {{ {} }}", body("std::env::arg(1)".to_string()));
    let on_pool = format!(
        r#"
        type Go {{ n: Int; }}
        locus Job {{
            params {{ n: Int = 0; }}
            run() {{ {} }}
        }}
        locus Host {{
            bus {{ subscribe "go" as on_go of type Go; }}
            accept(c: Job) {{ }}
            fn on_go(g: Go) {{ Job {{ n: g.n }}; }}
        }}
        main locus App {{
            params {{ host: Host = Host {{ }}; }}
            placement {{ host: cooperative(pool = io) where async_io; }}
            bus {{ publish "go" of type Go; }}
            run() {{
                "go" <- Go {{ n: 1 }};
                std::time::sleep(4s);
            }}
        }}
        fn main() {{ App {{ }}; }}
"#,
        body("std::env::arg(1)".to_string())
    );
    for (tag, src) in [("off_pool", off_pool), ("async_io", on_pool)] {
        let bin = harness::unique_bin(&format!("hale_tls_sse_{tag}"));
        build_opts::build_source(&src, &bin, &build_opts::options()).expect("build");
        let out = Command::new("timeout")
            .arg("60")
            .arg(&bin)
            .arg(peer.port.to_string())
            .arg(held.port.to_string())
            .env("SSL_CERT_FILE", &peer.cert)
            .env("LOTUS_NO_CHUNK_POOL", "1")
            .output()
            .expect("run");
        let _ = std::fs::remove_file(&bin);
        let out = String::from_utf8_lossy(&out.stdout).into_owned();
        assert_eq!(field(&out, "first", "status"), "200", "{tag}: {out}");
        assert_eq!(field(&out, "first", "event"), "tick", "{tag}: {out}");
        assert_eq!(field(&out, "first", "data"), "first", "{tag}: {out}");
        assert_eq!(field(&out, "second", "data"), "second", "{tag}: {out}");
        assert_eq!(field(&out, "after", "kind"), "end", "{tag}: {out}");
        let (ta, tb) = (ms(&out, "first"), ms(&out, "second"));
        assert!(tb >= ta + 500, "{tag}: the first event is read 600 ms before the second is sent: {out}");
        assert_eq!(field(&out, "held", "data"), "first", "{tag}: {out}");
        assert_eq!(field(&out, "held", "kind"), "recv_failed", "{tag}: {out}");
        assert_eq!(field(&out, "held", "timeout"), "true", "{tag}: {out}");
        let took = ms(&out, "held");
        assert!((700..1500).contains(&took), "{tag}: the held stream ends at its 800 ms deadline; took {took}: {out}");
    }
}

//! A connect deadline (friction sweep 2026-10-12, downstream handoff):
//! `std::io::tcp::connect` and `std::io::tls::connect` took no timeout, and
//! `connect_wait` retried only a REFUSED connect, so a host that never
//! answers held the caller until the kernel gave up (about two minutes of
//! SYN retransmits). `connect_within(host, port, timeout)` bounds the one
//! attempt (tls: the dial and the handshake), `connect_wait`'s `wait`
//! bounds the whole dial, `std::http::Client`'s `timeout_ms` bounds each
//! attempt's dial, and every expiry is the IoError kind `timeout`. On an
//! `async_io` pool the pending connect parks the coroutine.
//!
//! The host that never answers is made here, deterministically, without
//! a non-routable address (whose behaviour is the network's, not the
//! test's: a route may answer `ENETUNREACH` at once, or a firewall a
//! reset): a loopback socket listening with a backlog of 0, its one
//! accept-queue slot taken by a connection the test holds and never
//! accepts. Linux drops every further SYN to a listener whose accept
//! queue is full (`tcp_conn_request`: `sk_acceptq_is_full` → drop, with
//! `tcp_abort_on_overflow` at its default 0), so a connect to it is
//! neither accepted nor refused until the client gives up — exactly the
//! blackholed peer. The kernel picks the port (bind to `:0`, read back).
//! macOS's accept-queue overflow behaviour is not the same contract, so
//! these tests are Linux's alone.

use std::net::TcpStream;
use std::process::Command;
use std::time::{Duration, Instant};

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;
#[path = "support/child.rs"]
mod child;

/// A loopback listener whose accept queue is full: a connect to its port
/// pends until the caller's deadline. Dropping it closes both sockets.
struct Blackhole {
    fd: i32,
    port: u16,
    _held: TcpStream,
}

impl Blackhole {
    fn new() -> Blackhole {
        // SAFETY: plain socket syscalls on a descriptor this fn owns; the
        // sockaddr structs are zeroed and sized by the kernel's types.
        unsafe {
            let fd = libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0);
            assert!(fd >= 0, "socket: {}", std::io::Error::last_os_error());
            let mut addr: libc::sockaddr_in = std::mem::zeroed();
            addr.sin_family = libc::AF_INET as libc::sa_family_t;
            addr.sin_port = 0;
            addr.sin_addr.s_addr = u32::from_ne_bytes([127, 0, 0, 1]);
            let len = std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t;
            assert_eq!(
                libc::bind(fd, &addr as *const _ as *const libc::sockaddr, len),
                0,
                "bind: {}",
                std::io::Error::last_os_error()
            );
            assert_eq!(libc::listen(fd, 0), 0, "listen: {}", std::io::Error::last_os_error());
            let mut got: libc::sockaddr_in = std::mem::zeroed();
            let mut glen = len;
            assert_eq!(
                libc::getsockname(fd, &mut got as *mut _ as *mut libc::sockaddr, &mut glen),
                0
            );
            let port = u16::from_be(got.sin_port);
            // The one slot a backlog of 0 leaves: taken, never accepted.
            let held = TcpStream::connect_timeout(
                &format!("127.0.0.1:{port}").parse().unwrap(),
                Duration::from_secs(2),
            )
            .expect("the first connect fills the accept queue");
            Blackhole { fd, port, _held: held }
        }
    }
}

impl Drop for Blackhole {
    fn drop(&mut self) {
        // SAFETY: closes the descriptor `new` opened, once.
        unsafe {
            libc::close(self.fd);
        }
    }
}

fn run(name: &str, src: &str, args: &[&str]) -> String {
    let bin = harness::unique_bin(name);
    build_opts::build_source(src, &bin, &build_opts::options()).expect("build");
    let out = Command::new(&bin).args(args).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "{name}: {:?}; stdout: {stdout}; stderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    stdout
}

/// The `<key>=<n>` value on the line that starts with `tag`.
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

#[test]
#[cfg_attr(not(target_os = "linux"), ignore = "the full-accept-queue blackhole is Linux's SYN-drop contract")]
fn a_connect_nobody_answers_ends_at_its_deadline_with_kind_timeout() {
    let hole = Blackhole::new();
    let src = r#"
        fn main() {
            let port = std::str::parse_int(std::env::arg(1)) or 0;
            let mut t = std::time::monotonic_ns();
            let mut kind = "";
            let fd = std::io::tcp::connect_within("127.0.0.1", port, 300ms) or { kind = err.kind; - 1 };
            println("within fd=", fd, " kind=", kind, " ms=", (std::time::monotonic_ns() - t) / 1000000);
            t = std::time::monotonic_ns();
            kind = "";
            let fw = std::io::tcp::connect_wait("127.0.0.1", port, 250ms) or { kind = err.kind; - 1 };
            println("wait fd=", fw, " kind=", kind, " ms=", (std::time::monotonic_ns() - t) / 1000000);
            t = std::time::monotonic_ns();
            kind = "";
            let h = std::io::tls::connect_within("127.0.0.1", port, 200ms) or { kind = err.kind; - 1 };
            println("tls h=", h, " kind=", kind, " ms=", (std::time::monotonic_ns() - t) / 1000000);
        }
    "#;
    let port = hole.port.to_string();
    let out = run("hale_tcp_connect_deadline", src, &[&port]);
    for (tag, deadline) in [("within", 300), ("wait", 250), ("tls", 200)] {
        assert_eq!(field(&out, tag, "kind"), "timeout", "{tag}: {out}");
        let took = ms(&out, tag);
        assert!(
            took + 20 >= deadline && took < deadline + 1000,
            "{tag}: the dial must end at its {deadline} ms deadline, not \
             before it and not at the kernel's give-up; took {took} ms: {out}"
        );
    }
}

#[test]
fn connect_within_reaches_a_listener_and_a_refusal_is_still_an_answer() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let open = listener.local_addr().unwrap().port().to_string();
    let nobody = child::Refusing::new();
    let src = format!(
        r#"
        fn main() {{
            let port = std::str::parse_int(std::env::arg(1)) or 0;
            let fd = std::io::tcp::connect_within("127.0.0.1", port, 2s) or - 1;
            println("open ok=", fd >= 0);
            std::io::tcp::close_fd(fd);
            let t = std::time::monotonic_ns();
            let mut kind = "";
            let r = std::io::tcp::connect_within("{host}", {refusing}, 2s) or {{ kind = err.kind; - 1 }};
            println("refused fd=", r, " kind=", kind, " ms=", (std::time::monotonic_ns() - t) / 1000000);
        }}
    "#,
        host = nobody.host(),
        refusing = nobody.port()
    );
    let out = run("hale_tcp_connect_within_open", &src, &[&open]);
    assert_eq!(field(&out, "open", "ok"), "true", "{out}");
    assert_eq!(field(&out, "refused", "kind"), "connection_refused", "{out}");
    assert!(ms(&out, "refused") < 500, "a refusal answers at once, not at the deadline: {out}");
}

#[test]
#[cfg_attr(not(target_os = "linux"), ignore = "the full-accept-queue blackhole is Linux's SYN-drop contract")]
fn the_http_client_timeout_bounds_the_connect_and_the_error_names_it() {
    let hole = Blackhole::new();
    let src = r#"
        fn main() {
            let port = std::str::parse_int(std::env::arg(1)) or 0;
            let c = std::http::Client { timeout_ms: 200, max_retries: 0 };
            let t = std::time::monotonic_ns();
            let r = c.get("http://127.0.0.1:" + to_string(port) + "/") or {
                println("err kind=", err.kind, " ms=", (std::time::monotonic_ns() - t) / 1000000);
                println("detail ", err.detail);
                std::http::ClientResponse { status: 0, headers: "", body: b"" }
            };
            let k = std::http::Client { timeout_ms: 200, max_retries: 0, keep_alive: true };
            let r2 = k.get("http://127.0.0.1:" + to_string(port) + "/") or {
                println("pooled kind=", err.kind);
                println("pooled_detail ", err.detail);
                std::http::ClientResponse { status: 0, headers: "", body: b"" }
            };
            println("statuses ", r.status + r2.status);
        }
    "#;
    let port = hole.port.to_string();
    let out = run("hale_http_client_connect_timeout", src, &[&port]);
    assert_eq!(field(&out, "err", "kind"), "retries_exhausted", "{out}");
    let took = ms(&out, "err");
    assert!(took >= 180 && took < 1200, "the client's dial must end at timeout_ms; took {took} ms: {out}");
    for tag in ["detail ", "pooled_detail "] {
        let line = out.lines().find(|l| l.starts_with(tag)).expect(tag);
        assert!(
            line.contains("timeout: no connection within 200 ms"),
            "the error must name the timeout: {line:?}"
        );
    }
}

#[test]
#[cfg_attr(not(target_os = "linux"), ignore = "the full-accept-queue blackhole is Linux's SYN-drop contract")]
fn a_pending_connect_parks_its_coroutine_on_an_async_io_pool() {
    // Two coroutines on one async_io pool: a dial to the blackhole with a
    // 600 ms deadline, and a ticker sleeping 50 ms ten times. A connect
    // that held the worker would stall the ticker for the whole 600 ms
    // (its ticks would start after the dial ended); a parked one leaves
    // it ticking through the dial.
    let hole = Blackhole::new();
    let src = r#"
        type Go { kind: Int; port: Int; }

        locus Job {
            params { kind: Int = 0; port: Int = 0; t0: Int = 0; }
            run() {
                if self.kind == 0 {
                    let mut i = 0;
                    while i < 10 {
                        std::time::sleep(50ms);
                        i = i + 1;
                    }
                    println("ticks done ms=", (std::time::monotonic_ns() - self.t0) / 1000000);
                } else {
                    let mut kind = "";
                    let fd = std::io::tcp::connect_within("127.0.0.1", self.port, 600ms) or { kind = err.kind; - 1 };
                    println("dial fd=", fd, " kind=", kind, " ms=", (std::time::monotonic_ns() - self.t0) / 1000000);
                }
            }
        }

        locus Host {
            params { t0: Int = 0; }
            bus { subscribe "go" as on_go of type Go; }
            accept(c: Job) { }
            fn on_go(g: Go) {
                Job { kind: g.kind, port: g.port, t0: self.t0 };
            }
        }

        main locus App {
            params {
                t0: Int = 0;
                host: Host = Host { };
            }
            placement {
                host: cooperative(pool = io) where async_io;
            }
            bus { publish "go" of type Go; }
            run() {
                let port = std::str::parse_int(std::env::arg(1)) or 0;
                self.host.t0 = self.t0;
                "go" <- Go { kind: 1, port: port };
                "go" <- Go { kind: 0, port: 0 };
                std::time::sleep(1500ms);
            }
        }

        fn main() {
            App { t0: std::time::monotonic_ns() };
            return 0;
        }
    "#;
    let port = hole.port.to_string();
    let started = Instant::now();
    let out = run("hale_tcp_connect_parks", src, &[&port]);
    assert!(started.elapsed() < Duration::from_secs(20));
    assert_eq!(field(&out, "dial", "kind"), "timeout", "{out}");
    let dial = ms(&out, "dial");
    let ticks = ms(&out, "ticks");
    assert!(dial >= 580 && dial < 1400, "the dial ends at its deadline: {out}");
    assert!(
        ticks < dial,
        "the ticker must finish its ten 50 ms sleeps while the dial is \
         pending (a connect that holds the worker stalls it until the \
         dial ends): ticks done at {ticks} ms, dial ended at {dial} ms: {out}"
    );
}

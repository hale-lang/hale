//! The api runtime's socket transports on the TCP connection substrate
//! (spec/stdlib.md § "The TCP connection substrate", spec/api.md § The
//! runtime boundaries).
//!
//! `http::Rpc` and `unix::Rpc` listeners are substrate listeners: each holds
//! a relay that accepts and hands every connection over the bus as a
//! `std::io::tcp::Accepted`, and the consumer on its route serves each on a
//! coroutine of its own. One program serves one surface of four receivers
//! over both transports. These tests hold what that adds:
//!
//! * a connection relayed on a route of an api listener that no consumer
//!   takes is closed by that listener (the substrate's rule), for both
//!   address families;
//! * calls to different receivers run at once over either transport, and
//!   calls to one receiver run one at a time (the exposure hands a receiver
//!   one call at a time: § The runtime boundaries, dispatch);
//! * a process drain answers the calls in flight, then releases the port
//!   and the socket file.

#![cfg(target_os = "linux")]

use std::io::Read;
use std::net::TcpListener;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;
#[path = "support/ports.rs"]
mod ports;
#[path = "support/http_rpc.rs"]
mod http_rpc;

use http_rpc::*;

/// Four receivers, each on the async_io pool `work`, served over HTTP on
/// `$BIND` and over a Unix socket at `$SOCK`. A call holds its receiver for
/// `$SLOW_MS`. With `$STRAY_PORT` and `$STRAY_UNIX_PORT` set, once `$GO`
/// appears the program connects to each and relays the descriptor on route
/// `#1` of the HTTP and the Unix listener, which no consumer takes.
const BENCH: &str = r##"
type Ping { n: Int; }
type Pong { n: Int; }

api Bench {
    rpc W0::work;
    rpc W1::work;
    rpc W2::work;
    rpc W3::work;
}

@unbounded
fn pause() {
    let ms = std::str::parse_int(std::env::var("SLOW_MS")) or 0;
    let mut i = 0;
    while i < ms {
        std::time::sleep(1ms);
        i = i + 1;
    }
}

locus W0 { fn work(p: Ping) -> Pong { pause(); return Pong { n: p.n }; } }
locus W1 { fn work(p: Ping) -> Pong { pause(); return Pong { n: p.n + 1 }; } }
locus W2 { fn work(p: Ping) -> Pong { pause(); return Pong { n: p.n + 2 }; } }
locus W3 { fn work(p: Ping) -> Pong { pause(); return Pong { n: p.n + 3 }; } }

locus Tokens {
    fn principal(token: String) -> std::api::Principal {
        if token == "t" { return std::api::Principal { mode: "bearer", name: "alice" }; }
        return std::api::Principal { mode: "bearer", name: "" };
    }
    fn refused() -> String { return "no such token"; }
}

locus Nobody {
    fn holds(p: std::api::Principal, r: String) -> Bool { return false; }
}

main locus Desk {
    params {
        bearer: Tokens = Tokens { };
        roles: Nobody = Nobody { };
        w0: W0 = W0 { };
        w1: W1 = W1 { };
        w2: W2 = W2 { };
        w3: W3 = W3 { };
    }
    placement {
        w0: cooperative(pool = work) where async_io;
        w1: cooperative(pool = work) where async_io;
        w2: cooperative(pool = work) where async_io;
        w3: cooperative(pool = work) where async_io;
    }
    bus {
        publish __TcpAcceptedT;
    }
    run() {
        let h = api::serve(Bench, http::Rpc { bind: std::env::var("BIND"), codec: json, principals: self.bearer, roles: self.roles }, as: "bench", bound: 64, on_full: refuse);
        let u = api::serve(Bench, unix::Rpc { path: std::env::var("SOCK"), roles: self.roles }, as: "ubench", bound: 64, on_full: refuse);
        let stray = std::str::parse_int(std::env::var("STRAY_PORT")) or 0;
        if stray > 0 {
            let go = std::env::var("GO");
            while !self.draining && (std::io::fs::read_file(go) or "") == "" {
                std::time::sleep(5ms);
            }
            self.stray(stray, std::env::var("BIND"));
            self.stray(std::str::parse_int(std::env::var("STRAY_UNIX_PORT")) or 0, "unix:" + std::env::var("SOCK"));
            println("relayed");
        }
        let trigger = std::env::var("TRIGGER");
        while !self.draining && (std::io::fs::read_file(trigger) or "") == "" {
            std::time::sleep(10ms);
        }
        h.stop();
        u.stop();
        println("stopped");
    }
    // A connection relayed on a route of `listener` that no consumer takes.
    fn stray(port: Int, listener: String) {
        let fd = std::io::tcp::__connect("127.0.0.1", port);
        __TcpAcceptedT <- std::io::tcp::Accepted { listener: listener, route: listener + "#1", fd: fd, seq: 0 };
    }
}

fn main() { Desk { }; }
"##;

fn bench() -> PathBuf {
    static BUILT: OnceLock<PathBuf> = OnceLock::new();
    BUILT
        .get_or_init(|| {
            let bin = harness::unique_bin("api_transport_bench");
            build_opts::build_source(BENCH, &bin, &build_opts::options()).expect("build the bench program");
            bin
        })
        .clone()
}

fn http_work(port: u16, k: usize) -> Response {
    request(port, "POST", &format!("/call/W{k}::work"), &[bearer("t")], Some("{\"n\":10}"))
}

fn unix_work(server: &Server, k: usize) -> String {
    unix_ask(&server.sock(), &format!("{{\"call\":\"W{k}::work\",\"payload\":{{\"n\":10}},\"id\":{k}}}"))
}

/// Run `calls` at once, call `i` to receiver `pick(i)`, and how long the
/// last took to be answered.
fn at_once(calls: usize, pick: fn(usize) -> usize, call: impl Fn(usize) -> bool + Sync) -> Duration {
    let t0 = Instant::now();
    std::thread::scope(|s| {
        let call = &call;
        let hs: Vec<_> = (0..calls).map(|i| s.spawn(move || call(pick(i)))).collect();
        for (i, h) in hs.into_iter().enumerate() {
            assert!(h.join().expect("a client thread"), "call {i} was not answered with a result");
        }
    });
    t0.elapsed()
}

#[test]
fn a_connection_no_consumer_takes_is_closed_by_the_api_listener() {
    // the test's own listeners, which the program dials: the far ends of the
    // two stray descriptors
    let tcp_end = TcpListener::bind("127.0.0.1:0").expect("bind");
    let unix_end = TcpListener::bind("127.0.0.1:0").expect("bind");
    let server = Server::start(
        &bench(),
        &[
            ("STRAY_PORT", &tcp_end.local_addr().unwrap().port().to_string()),
            ("STRAY_UNIX_PORT", &unix_end.local_addr().unwrap().port().to_string()),
            ("GO", &server_go()),
        ],
    );
    server.ready();
    wait_accepting_unix(&server.sock());
    std::fs::write(server_go(), "go").expect("write GO");
    for (what, end) in [("the HTTP listener", &tcp_end), ("the Unix listener", &unix_end)] {
        end.set_nonblocking(false).unwrap();
        let (mut far, _) = end.accept().expect("the program dials");
        far.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let t0 = Instant::now();
        let mut buf = [0u8; 16];
        match far.read(&mut buf) {
            Ok(0) => {}
            other => panic!("{what}: a stray connection on its route is closed, got {other:?}"),
        }
        assert!(t0.elapsed() < Duration::from_secs(3), "{what}: closed promptly");
    }
    // and the listeners serve on
    assert_eq!(http_work(server.port, 1).status, 200);
    assert!(unix_work(&server, 2).contains("\"ok\":true"));
    let done = server.finish();
    assert!(done.status.success(), "{:?}\n{}", done.status, done.stderr);
    assert!(done.stdout.contains("relayed"), "{}", done.stdout);
}

/// The trigger file the stray test's program waits on: one per process.
fn server_go() -> String {
    static GO: OnceLock<PathBuf> = OnceLock::new();
    GO.get_or_init(|| harness::unique_dir("api_transport_go").join("go")).display().to_string()
}

#[test]
fn calls_to_different_receivers_run_at_once_over_both_transports() {
    let server = Server::start(&bench(), &[("SLOW_MS", "500")]);
    server.ready();
    wait_accepting_unix(&server.sock());
    // eight calls over four receivers: two rounds of 500 ms, not eight
    let http = at_once(8, |i| i % 4, |k| http_work(server.port, k).status == 200);
    assert!(http < Duration::from_millis(2000), "eight HTTP calls over four receivers took {http:?}");
    let unix = at_once(8, |i| i % 4, |k| unix_work(&server, k).contains("\"ok\":true"));
    assert!(unix < Duration::from_millis(2000), "eight Unix calls over four receivers took {unix:?}");
    // three calls to one receiver: one at a time, as the exposure hands them
    let one = at_once(3, |_| 0, |k| http_work(server.port, k).status == 200);
    assert!(one >= Duration::from_millis(1400), "three calls to one receiver ran at once ({one:?}): the dispatch window is one call");
    let done = server.finish();
    assert!(done.status.success(), "{:?}\n{}", done.status, done.stderr);
}

#[test]
fn a_drain_answers_the_calls_in_flight_and_releases_the_listeners() {
    let mut server = Server::start(&bench(), &[("SLOW_MS", "400")]);
    server.ready();
    wait_accepting_unix(&server.sock());
    let port = server.port;
    let pid = server.pid();
    let answered = std::thread::scope(|s| {
        let calls: Vec<_> = (0..4).map(|k| s.spawn(move || http_work(port, k).status)).collect();
        std::thread::sleep(Duration::from_millis(150));
        let _ = std::process::Command::new("kill").args(["-TERM", &pid.to_string()]).status();
        calls.into_iter().map(|h| h.join().expect("a client thread")).collect::<Vec<_>>()
    });
    assert_eq!(answered, [200, 200, 200, 200], "the executing calls complete and are answered");
    let done = server.wait(Duration::from_secs(30));
    assert!(done.status.success(), "{:?}\n{}", done.status, done.stderr);
    assert!(done.stdout.contains("stopped"), "{}", done.stdout);
    assert!(closed(port), "the HTTP listener's port is released");
    assert!(!server.sock().exists(), "the Unix listener's socket file is removed");
    assert!(UnixStream::connect(server.sock()).is_err(), "nothing accepts on the path");
}

#[test]
fn the_bench_program_runs_clean_under_asan() {
    // the substrate's relay and consumer, the peers as flow children of the
    // transports' handlers, a stray connection closed by its listener and a
    // stop: a use after free in any of them would be reported here (the
    // arena's chunk recycling off, so a freed chunk is not handed back out)
    let bin = harness::unique_bin("api_transport_bench_asan");
    harness::build_source_asan(BENCH, &bin);
    let tcp_end = TcpListener::bind("127.0.0.1:0").expect("bind");
    let unix_end = TcpListener::bind("127.0.0.1:0").expect("bind");
    let go = harness::unique_dir("api_transport_go_asan").join("go");
    let server = Server::start(
        &bin,
        &[
            ("LOTUS_NO_CHUNK_POOL", "1"),
            ("SLOW_MS", "50"),
            ("STRAY_PORT", &tcp_end.local_addr().unwrap().port().to_string()),
            ("STRAY_UNIX_PORT", &unix_end.local_addr().unwrap().port().to_string()),
            ("GO", &go.display().to_string()),
        ],
    );
    server.ready();
    wait_accepting_unix(&server.sock());
    std::fs::write(&go, "go").expect("write GO");
    for end in [&tcp_end, &unix_end] {
        let (mut far, _) = end.accept().expect("the program dials");
        far.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        assert!(matches!(far.read(&mut [0u8; 16]), Ok(0)), "the stray connection is closed");
    }
    at_once(8, |i| i % 4, |k| http_work(server.port, k).status == 200);
    at_once(8, |i| i % 4, |k| unix_work(&server, k).contains("\"ok\":true"));
    let done = server.finish();
    assert!(done.status.success(), "{:?}\n{}", done.status, done.stderr);
    assert!(!done.stderr.contains("AddressSanitizer"), "{}", done.stderr);
}

#[test]
fn the_unix_listener_removes_only_the_socket_file_it_bound() {
    // a replacement binds a file of its own at the path while the api
    // listener still holds the old one (unlinked under it): the listener's
    // release at stop leaves the replacement's file alone
    let mut server = Server::start(&bench(), &[]);
    server.ready();
    let sock = server.sock();
    wait_accepting_unix(&sock);
    std::fs::remove_file(&sock).expect("unlink the bound file");
    let replacement = std::os::unix::net::UnixListener::bind(&sock).expect("the replacement binds the path");
    // (the server's own directory goes when it is dropped: ask while it lives)
    server.trigger();
    let done = server.wait(Duration::from_secs(30));
    assert!(done.status.success(), "{:?}\n{}", done.status, done.stderr);
    assert!(sock.exists(), "the replacement's socket file survives the api listener's release");
    replacement.set_nonblocking(true).unwrap();
    let _client = UnixStream::connect(&sock).expect("the replacement still accepts");
    assert!(replacement.accept().is_ok());
}

//! The HTTP/2 server's listener on the TCP connection substrate
//! (spec/stdlib.md § "The TCP connection substrate", § `std::io::h2`).
//!
//! `std::io::h2::Listener` (the one every `grpc::Rpc` serve site holds) is
//! a substrate listener: it holds a relay that accepts and hands every
//! connection over the bus as a `std::io::tcp::Accepted`, and the consumer
//! on the route of its name (`127.0.0.1:<port>`) starts a `Conn` for each, a
//! flow child on the listener's `async_io` pool. These tests hold what that
//! adds:
//!
//! * a connection relayed on a route of the listener that no consumer takes
//!   is closed by that listener;
//! * `std::io::tcp::stop(name)` stops the listener, and the connections it
//!   accepted before are served on (a drain, never a cut);
//! * slow connections (a client that has sent nothing, one that stalls in
//!   its preface) are served beside one another and beside the fast ones.

#![cfg(target_os = "linux")]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

#[path = "support/build.rs"]
mod build_opts;
#[path = "support/h2_client.rs"]
mod h2_client;
#[path = "support/harness.rs"]
mod harness;
#[path = "support/http_rpc.rs"]
mod http_rpc;
#[path = "support/ports.rs"]
mod ports;

use h2_client::*;
use http_rpc::*;

/// An HTTP/2 listener on `$BIND` and a transport that answers `/hello`.
/// `$GO` appearing relays a connection to `$STRAY_PORT` on route `#1` of the
/// listener, which no consumer takes; `$STOPNAME` appearing stops the
/// listener by its name (the bound address); `$TRIGGER` stops the transport.
const ECHO: &str = r##"
@unbounded
fn written(path: String) -> Bool {
    return len(path) > 0 && (std::io::fs::read_file(path) or "") != "";
}

locus Echo {
    bus {
        subscribe "__api.h2.event" as on_event of type std::io::h2::Event;
        subscribe "__api.h2.cmd" as on_cmd of type std::io::h2::Cmd;
        publish "__api.h2.cmd" of type std::io::h2::Cmd;
    }
    fn on_cmd(c: std::io::h2::Cmd) {
        if c.kind == 4 && c.key == 1 {
            "__api.h2.cmd" <- std::io::h2::Cmd { key: 1, kind: 3, stream: 0, code: 0, headers: std::bytes::from_string(""), body: std::bytes::from_string(""), trailers: std::bytes::from_string("") };
        }
    }
    fn stop() {
        "__api.h2.cmd" <- std::io::h2::Cmd { key: 1, kind: 5, stream: 0, code: 0, headers: std::bytes::from_string(""), body: std::bytes::from_string(""), trailers: std::bytes::from_string("") };
    }
    fn on_event(e: std::io::h2::Event) {
        if e.kind == 1 {
            "__api.h2.cmd" <- std::io::h2::Cmd { key: e.key * 4294967296 + e.conn, kind: 0, stream: e.stream, code: 0, headers: std::bytes::from_string(":status: 200\n"), body: std::bytes::from_string("hello"), trailers: std::bytes::from_string("") };
        }
    }
}

main locus App {
    params {
        echo: Echo = Echo { };
        server: __StdIoH2Listener = __StdIoH2Listener { tid: 1, bind: std::env::var("BIND"), what: "the h2 test" };
    }
    placement { server: cooperative(pool = h2) where async_io; }
    bus {
        publish __TcpAcceptedT;
    }
    // A connection relayed on a route of the listener that no consumer takes.
    fn stray() {
        let listener = std::env::var("BIND");
        let fd = std::io::tcp::__connect("127.0.0.1", std::str::parse_int(std::env::var("STRAY_PORT")) or 0);
        __TcpAcceptedT <- std::io::tcp::Accepted { listener: listener, route: listener + "#1", fd: fd, seq: 0 };
        println("relayed");
    }
    run() {
        let all = std::env::var("TRIGGER");
        let mut strayed = false;
        let mut named = false;
        while !self.draining && !written(all) {
            if !strayed && written(std::env::var("GO")) {
                strayed = true;
                self.stray();
            }
            if !named && written(std::env::var("STOPNAME")) {
                named = true;
                println("named=", std::io::tcp::stop(std::env::var("BIND")));
            }
            std::time::sleep(10ms);
        }
        self.echo.stop();
        std::time::sleep(300ms);
        println("stopped");
    }
}

fn main() { App { }; }
"##;

fn echo() -> PathBuf {
    static BUILT: OnceLock<PathBuf> = OnceLock::new();
    BUILT
        .get_or_init(|| {
            let bin = harness::unique_bin("io_h2_substrate");
            build_opts::build_source(ECHO, &bin, &build_opts::options()).expect("build the h2 program");
            bin
        })
        .clone()
}

const WAIT: Duration = Duration::from_secs(10);

/// The files the program watches, in a directory of the test's own.
fn controls(what: &str) -> (PathBuf, PathBuf) {
    let dir = harness::unique_dir(what);
    (dir.join("go"), dir.join("stopname"))
}

fn hello(c: &mut Client, stream: u32) -> u16 {
    c.headers(stream, &[(":method", "GET"), (":scheme", "http"), (":path", "/hello"), (":authority", "t")], true);
    let r = c.response(stream, WAIT);
    assert_eq!(r.data, b"hello", "{r:?}");
    r.status()
}

#[test]
fn a_connection_no_consumer_takes_is_closed_by_the_h2_listener() {
    let end = TcpListener::bind("127.0.0.1:0").expect("bind");
    let (go, stopname) = controls("io_h2_substrate_stray");
    let server = Server::start(
        &echo(),
        &[
            ("STRAY_PORT", &end.local_addr().unwrap().port().to_string()),
            ("GO", &go.display().to_string()),
            ("STOPNAME", &stopname.display().to_string()),
        ],
    );
    server.ready();
    std::fs::write(&go, "go").expect("write GO");
    let (mut far, _) = end.accept().expect("the program dials");
    far.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let t0 = Instant::now();
    match far.read(&mut [0u8; 16]) {
        Ok(0) => {}
        other => panic!("a stray connection on the listener's route is closed, got {other:?}"),
    }
    assert!(t0.elapsed() < Duration::from_secs(3), "closed promptly");
    // and the listener serves on
    let mut c = Client::connect(server.port);
    assert_eq!(hello(&mut c, 1), 200);
    let done = server.finish();
    assert!(done.status.success(), "{:?}\n{}\n{}", done.status, done.stdout, done.stderr);
    assert!(done.stdout.contains("relayed"), "{}", done.stdout);
}

#[test]
fn stop_by_name_closes_the_h2_listener_and_its_connections_are_served_on() {
    let (go, stopname) = controls("io_h2_substrate_stop");
    let server = Server::start(&echo(), &[("GO", &go.display().to_string()), ("STOPNAME", &stopname.display().to_string())]);
    server.ready();
    let mut c = Client::connect(server.port);
    assert_eq!(hello(&mut c, 1), 200);
    std::fs::write(&stopname, "stop").expect("write STOPNAME");
    let start = Instant::now();
    while !closed(server.port) {
        assert!(start.elapsed() < WAIT, "stop(name) closes the listener");
        std::thread::sleep(Duration::from_millis(10));
    }
    // the connection accepted before is not cut: it carries more calls
    for s in [3u32, 5, 7] {
        assert_eq!(hello(&mut c, s), 200, "stream {s} after the listener stopped");
    }
    assert!(c.goaway.is_none() && !c.eof, "the connection is whole");
    // the transport's own stop still says GOAWAY to it
    server.trigger();
    let go = c.goaway_within(WAIT).expect("stop says GOAWAY");
    assert_eq!(go.1, 0, "NO_ERROR");
    let mut server = server;
    let done = server.wait(Duration::from_secs(30));
    assert!(done.status.success(), "{:?}\n{}\n{}", done.status, done.stdout, done.stderr);
    assert!(done.stdout.contains("named=true"), "stop(name) found the listener by its name:\n{}", done.stdout);
}

#[test]
fn slow_connections_are_served_beside_one_another_and_the_fast_ones() {
    let server = Server::start(&echo(), &[]);
    server.ready();
    let port = server.port;
    // eight connections that send nothing at all, held open throughout
    let silent: Vec<TcpStream> = (0..8).map(|_| TcpStream::connect(("127.0.0.1", port)).expect("connect")).collect();
    let stall = Duration::from_millis(800);
    let t0 = Instant::now();
    std::thread::scope(|s| {
        // eight that stall half way through the preface, each its own connection
        let slow: Vec<_> = (0..8)
            .map(|_| {
                s.spawn(move || {
                    let mut raw = TcpStream::connect(("127.0.0.1", port)).expect("connect");
                    raw.write_all(b"PRI * HTTP/2.0\r\n").expect("half the preface");
                    std::thread::sleep(stall);
                    raw.write_all(b"\r\nSM\r\n\r\n\x00\x00\x00\x04\x00\x00\x00\x00\x00").expect("the rest, and SETTINGS");
                    // the server's SETTINGS frame comes back: a frame header, type 4
                    raw.set_read_timeout(Some(WAIT)).unwrap();
                    let mut head = [0u8; 9];
                    raw.read_exact(&mut head).expect("the server's first frame");
                    head[3]
                })
            })
            .collect();
        std::thread::sleep(Duration::from_millis(100));
        // meanwhile eight fast clients connect, all at once, and are answered
        let fast = Instant::now();
        let mut clients: Vec<Client> = (0..8).map(|_| Client::connect(port)).collect();
        for c in clients.iter_mut() {
            assert_eq!(hello(c, 1), 200);
        }
        assert!(fast.elapsed() < stall, "a fast connection waits for no slow one ({:?})", fast.elapsed());
        for (i, h) in slow.into_iter().enumerate() {
            assert_eq!(h.join().expect("a slow client"), 4, "slow connection {i}: SETTINGS");
        }
    });
    let all = t0.elapsed();
    assert!(all < stall * 3, "eight slow prefaces took {all:?}");
    drop(silent);
    let done = server.finish();
    assert!(done.status.success(), "{:?}\n{}\n{}", done.status, done.stdout, done.stderr);
}

#[test]
fn the_h2_listener_runs_clean_under_asan() {
    let bin = harness::unique_bin("io_h2_substrate_asan");
    harness::build_source_asan(ECHO, &bin);
    let end = TcpListener::bind("127.0.0.1:0").expect("bind");
    let (go, stopname) = controls("io_h2_substrate_asan");
    let server = Server::start(
        &bin,
        &[
            ("LOTUS_NO_CHUNK_POOL", "1"),
            ("STRAY_PORT", &end.local_addr().unwrap().port().to_string()),
            ("GO", &go.display().to_string()),
            ("STOPNAME", &stopname.display().to_string()),
        ],
    );
    server.ready();
    std::fs::write(&go, "go").expect("write GO");
    let (mut far, _) = end.accept().expect("the program dials");
    far.set_read_timeout(Some(WAIT)).unwrap();
    assert!(matches!(far.read(&mut [0u8; 16]), Ok(0)), "the stray connection is closed");
    let mut c = Client::connect(server.port);
    assert_eq!(hello(&mut c, 1), 200);
    {
        let mut gone = Client::connect(server.port);
        assert_eq!(hello(&mut gone, 1), 200);
    }
    std::fs::write(&stopname, "stop").expect("write STOPNAME");
    let start = Instant::now();
    while !closed(server.port) {
        assert!(start.elapsed() < WAIT, "stop(name) closes the listener");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(hello(&mut c, 3), 200);
    let done = server.finish();
    for bad in ["AddressSanitizer", "LeakSanitizer", "heap-use-after-free", "SUMMARY:"] {
        assert!(!done.stderr.contains(bad), "{bad} in:\n{}", done.stderr);
    }
    assert!(done.status.success(), "{:?}\n{}\n{}", done.status, done.stdout, done.stderr);
}

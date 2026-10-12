//! The WebSocket hub's listener on the TCP connection substrate
//! (spec/stdlib.md § "The TCP connection substrate", spec/api.md § Streams).
//!
//! A `ws::Hub`'s listener (`std::api::ws::Listener`) is a substrate
//! listener: it holds a relay that accepts and hands every connection over
//! the bus as a `std::io::tcp::Accepted`, and the consumer on the route of
//! its name (`127.0.0.1:<port>`) starts the hub's peer for each, a flow child
//! on the hub's `async_io` pool. These tests hold what that adds:
//!
//! * a connection relayed on a route of the hub's listener that no consumer
//!   takes is closed by that listener;
//! * `std::io::tcp::stop(name)` stops the listener, and the connections it
//!   accepted before are served on (a drain, never a cut) until the hub's
//!   own `stop()` closes them;
//! * slow connections (a client that stalls half way through its upgrade)
//!   are served at once, beside one another and beside the fast ones.

#![cfg(target_os = "linux")]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;
#[path = "support/ports.rs"]
mod ports;
#[path = "support/ws_hub.rs"]
mod ws_hub;
#[path = "support/ws_client.rs"]
mod ws_client;

use ws_client::*;
use ws_hub::*;

/// A hub named `fills` on `$BIND` that binds `Fills` (`requires:
/// [operator]`; dave is the operator). Driven by `CTL/cmd`: `publish N`,
/// `stray` (connect to `$STRAY_PORT` and relay the descriptor on route `#1`
/// of the hub's listener, which no consumer takes), `stopname`
/// (`std::io::tcp::stop` of the listener's name, the bound address) and
/// `stop` (the hub's own `stop()`).
const HUB: &str = r##"
role operator;

type Fill { qty: Int; price: Int; }
topic Fills { payload: Fill; subject: "desk.fills"; }

locus Tokens {
    fn principal(token: String) -> std::api::Principal {
        if token == "t-dave" { return std::api::Principal { mode: "bearer", name: "dave" }; }
        return std::api::Principal { mode: "bearer", name: "" };
    }
    fn refused() -> String { return "no such token"; }
}

locus Grants {
    params { operator: String = ""; }
    fn holds(p: std::api::Principal, r: String) -> Bool {
        if r == "operator" { return len(self.operator) > 0 && p.name == self.operator; }
        return false;
    }
}

locus Maker {
    params { made: Int = 0; }
    bus { publish Fills; }
    fn make(n: Int) {
        self.made = self.made + 1;
        Fills <- Fill { qty: n, price: n * 10 };
    }
}

main locus Desk {
    params {
        bearer: Tokens = Tokens { };
        hub_roles: Grants = Grants { operator: "dave" };
        hub: ws::Hub = ws::Hub { bind: std::env::var("BIND"), principals: self.bearer, roles: self.hub_roles, as: "fills" };
        maker: Maker = Maker { };
        named: Int = 0;
    }
    bindings {
        Fills: self.hub requires: [operator], bound: 64, on_full: drop_old;
    }
    bus {
        publish __TcpAcceptedT;
    }
    // A connection relayed on a route of the hub's listener that no consumer takes.
    fn stray() {
        let listener = std::env::var("BIND");
        let fd = std::io::tcp::__connect("127.0.0.1", std::str::parse_int(std::env::var("STRAY_PORT")) or 0);
        __TcpAcceptedT <- std::io::tcp::Accepted { listener: listener, route: listener + "#1", fd: fd, seq: 0 };
    }
    fn stats(ctl: String) {
        std::io::fs::write_file(ctl + "/stats.tmp", "made=" + to_string(self.maker.made) + "\nevents=" + to_string(self.hub.events()) + "\nsubs=" + to_string(self.hub.subscribers()) + "\nconns=" + to_string(self.hub.connections()) + "\nnamed=" + to_string(self.named) + "\n") or discard;
        std::io::fs::rename(ctl + "/stats.tmp", ctl + "/stats") or discard;
    }
    run() {
        let ctl = std::env::var("CTL");
        let mut done = false;
        while !done && !self.draining {
            let cmd = std::str::trim(std::io::fs::read_file(ctl + "/cmd") or "");
            if len(cmd) > 0 {
                std::io::fs::unlink(ctl + "/cmd") or discard;
                if cmd == "stop" { done = true; }
                if cmd == "stray" { self.stray(); }
                if cmd == "stopname" {
                    if std::io::tcp::stop(std::env::var("BIND")) { self.named = 1; } else { self.named = 2; }
                }
                if std::str::starts_with(cmd, "publish ") {
                    let n = std::str::parse_int(cmd[8..len(cmd)]) or 0;
                    let mut i = 1;
                    while i <= n {
                        self.maker.make(i);
                        i = i + 1;
                    }
                }
            }
            self.stats(ctl);
            std::time::sleep(5ms);
        }
        println("events=", self.hub.events(), " made=", self.maker.made);
        self.hub.stop();
    }
}

fn main() { Desk { }; }
"##;

fn build() -> PathBuf {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| {
        let bin = harness::unique_bin("api_hub_substrate");
        build_opts::build_source(HUB, &bin, &build_opts::options()).expect("build the hub program");
        bin
    })
    .clone()
}

fn event(seq: i64, n: i64) -> String {
    format!("{{\"type\":\"event\",\"topic\":\"Fills\",\"seq\":{seq},\"payload\":{{\"qty\":{n},\"price\":{}}}}}", n * 10)
}

/// Connect as dave, subscribe to `Fills`, and expect to be admitted.
fn admitted(port: u16) -> Ws {
    let mut ws = Ws::connect(port, Some("t-dave"));
    ws.subscribe("Fills");
    assert_eq!(ws.text(), "{\"type\":\"subscribed\",\"topic\":\"Fills\"}");
    ws
}

/// The stray connection's far end: read until the program closes it.
fn closed_promptly(end: &TcpListener, within: Duration) {
    let (mut far, _) = end.accept().expect("the program dials");
    far.set_read_timeout(Some(within)).unwrap();
    let t0 = Instant::now();
    match far.read(&mut [0u8; 16]) {
        Ok(0) => {}
        other => panic!("a stray connection on the hub listener's route is closed, got {other:?}"),
    }
    assert!(t0.elapsed() < within, "closed promptly");
}

/// An upgrade that stalls half way: the request line and the first headers
/// now, the rest after `stall`; the head of the answer.
fn slow_upgrade(port: u16, stall: Duration) -> String {
    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    s.write_all(b"GET / HTTP/1.1\r\nHost: hub\r\nUpgrade: websocket\r\n").expect("half the upgrade");
    std::thread::sleep(stall);
    let rest = format!("Connection: Upgrade\r\nSec-WebSocket-Key: {RFC_KEY}\r\nSec-WebSocket-Version: 13\r\nAuthorization: Bearer t-dave\r\n\r\n");
    s.write_all(rest.as_bytes()).expect("the rest of the upgrade");
    let mut head = Vec::new();
    let mut b = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        match s.read(&mut b) {
            Ok(1) => head.push(b[0]),
            other => panic!("the slow upgrade was not answered: {other:?} after {:?}", String::from_utf8_lossy(&head)),
        }
    }
    String::from_utf8_lossy(&head).into_owned()
}

#[test]
fn a_connection_no_consumer_takes_is_closed_by_the_hubs_listener() {
    let end = TcpListener::bind("127.0.0.1:0").expect("bind");
    let server = Server::start(&build(), &[("STRAY_PORT", &end.local_addr().unwrap().port().to_string())]);
    server.command("stray");
    closed_promptly(&end, Duration::from_secs(3));
    // and the hub serves on
    let mut dave = admitted(server.port);
    server.await_stat("subs", 1);
    server.command("publish 1");
    assert_eq!(dave.text(), event(1, 1));
    let done = server.finish();
    assert!(done.status.success(), "{}{}", done.stdout, done.stderr);
}

#[test]
fn stop_by_name_closes_the_hubs_listener_and_its_connections_are_served_on() {
    let server = Server::start(&build(), &[]);
    let port = server.port;
    let mut dave = admitted(port);
    server.await_stat("subs", 1);
    server.command("stopname");
    server.await_stat("named", 1);
    let start = Instant::now();
    while TcpStream::connect(("127.0.0.1", port)).is_ok() {
        assert!(start.elapsed() < Duration::from_secs(10), "stop(name) closes the hub's listener");
        std::thread::sleep(Duration::from_millis(10));
    }
    // the connection accepted before is not cut: its subscription delivers
    server.command("publish 2");
    assert_eq!(dave.text(), event(1, 1));
    assert_eq!(dave.text(), event(2, 2));
    assert_eq!(server.stat("subs"), Some(1));
    // and the hub's own stop() still closes it, with `closed`
    let done = server.finish();
    assert!(done.status.success(), "{}{}", done.stdout, done.stderr);
    assert_eq!(dave.text(), "{\"type\":\"closed\",\"reason\":\"shutting_down\"}");
    assert!(matches!(dave.recv_within(5000), Frame::Close(_)));
}

#[test]
fn slow_connections_are_served_at_once_beside_the_fast_ones() {
    let server = Server::start(&build(), &[]);
    let port = server.port;
    let stall = Duration::from_millis(800);
    let t0 = Instant::now();
    std::thread::scope(|s| {
        // eight upgrades that stall half way, each its own connection
        let slow: Vec<_> = (0..8).map(|_| s.spawn(move || slow_upgrade(port, stall))).collect();
        std::thread::sleep(Duration::from_millis(100));
        // meanwhile a fast client is upgraded, admitted and delivered to
        let fast = Instant::now();
        let mut dave = admitted(port);
        server.await_stat("subs", 1);
        server.command("publish 1");
        assert_eq!(dave.text(), event(1, 1));
        assert!(fast.elapsed() < stall, "a fast connection waits for no slow one ({:?})", fast.elapsed());
        for (i, h) in slow.into_iter().enumerate() {
            let head = h.join().expect("a slow client");
            assert!(head.starts_with("HTTP/1.1 101"), "slow connection {i}: {head}");
        }
    });
    // eight stalls, served together: one stall and change, not eight
    let all = t0.elapsed();
    assert!(all < stall * 3, "eight slow upgrades took {all:?}");
    let done = server.finish();
    assert!(done.status.success(), "{}{}", done.stdout, done.stderr);
}

#[test]
fn the_hub_listener_runs_clean_under_asan() {
    // the relay, the consumer, the peers as flow children of the hub's
    // handler, a stray connection closed by the listener and stop(name)
    let bin = harness::unique_bin("api_hub_substrate_asan");
    harness::build_source_asan(HUB, &bin);
    let end = TcpListener::bind("127.0.0.1:0").expect("bind");
    let server = Server::start(&bin, &[("LOTUS_NO_CHUNK_POOL", "1"), ("STRAY_PORT", &end.local_addr().unwrap().port().to_string())]);
    server.command("stray");
    closed_promptly(&end, Duration::from_secs(10));
    let mut dave = admitted(server.port);
    let other = admitted(server.port);
    server.await_stat("subs", 2);
    other.abandon();
    server.await_stat("subs", 1);
    server.command("publish 2");
    assert_eq!(dave.text(), event(1, 1));
    assert_eq!(dave.text(), event(2, 2));
    server.command("stopname");
    server.await_stat("named", 1);
    server.command("publish 1");
    assert_eq!(dave.text(), event(3, 1));
    let done = server.finish();
    assert!(done.status.success(), "{:?}\n{}\n{}", done.status, done.stdout, done.stderr);
    for bad in ["AddressSanitizer", "LeakSanitizer", "heap-use-after-free", "SUMMARY:"] {
        assert!(!done.stderr.contains(bad), "{bad} in:\n{}", done.stderr);
    }
}

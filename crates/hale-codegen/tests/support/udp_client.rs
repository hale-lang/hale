//! A datagram client for the UDP hub tests (GH #1417, R5), and the start of a
//! program whose hub is a `udp::Hub`: no connection to wait for, so the program
//! is up when it has written its first statistics.

#![allow(dead_code)]

use std::net::UdpSocket;
use std::path::Path;
use std::time::{Duration, Instant};

use super::harness;
use super::ws_hub::Server;

#[path = "child.rs"]
mod child;

/// Where the hub listens: a [`child::Reserved`] address, on
/// [`child::reserved_host`]. The hub's port is in its `BIND` before it
/// starts, and the reservation keeps every other test of a parallel run
/// off it until the hub has bound it. (A port drawn here and handed over
/// is free, not reserved: another test could take it first, and `start`
/// used to draw again when one did.)
pub fn hub_host() -> std::net::Ipv4Addr {
    child::reserved_host()
}

/// A reserved address for a hub, as `BIND` states it, and the port.
pub fn reserved_bind() -> (child::Reserved, String) {
    let r = child::Reserved::new();
    let bind = r.addr.to_string();
    (r, bind)
}

/// Run `bin` with its hub on a reserved UDP address, and wait until it has
/// bound it.
pub fn start(bin: &Path, env: &[(&str, &str)]) -> Server {
    let (reserved, bind) = reserved_bind();
    let mut all: Vec<(&str, &str)> = env.to_vec();
    all.push(("BIND", &bind));
    let mut s = Server::start_on(bin, &all, reserved.port(), harness::unique_dir("udphub"));
    let begin = Instant::now();
    while s.stat("made").is_none() {
        if s.exited() {
            let done = s.wait(Duration::from_secs(5));
            panic!("the program never bound its datagram port: {:?}\n{}{}", done.status, done.stdout, done.stderr);
        }
        assert!(begin.elapsed() < Duration::from_secs(30), "the program never bound its datagram port within 30 s");
        std::thread::sleep(Duration::from_millis(10));
    }
    s
}

pub struct Udp {
    s: UdpSocket,
    pub port: u16,
}

impl Udp {
    /// A client socket, sending to the hub on `port` (at [`hub_host`]).
    pub fn to(port: u16) -> Udp {
        let s = UdpSocket::bind("127.0.0.1:0").expect("bind a client socket");
        s.connect((hub_host(), port)).expect("connect");
        Udp { s, port }
    }

    pub fn send(&self, text: &str) {
        self.s.send(text.as_bytes()).expect("send a datagram");
    }

    /// `{"type":"subscribe","topic":T,"id":ID,"token":TOK}`, `id` as the JSON it is.
    pub fn subscribe(&self, topic: &str, id: &str, token: Option<&str>) {
        let token = token.map(|t| format!(",\"token\":\"{t}\"")).unwrap_or_default();
        self.send(&format!("{{\"type\":\"subscribe\",\"topic\":\"{topic}\",\"id\":{id}{token}}}"));
    }

    /// The next datagram, waiting at most `ms`.
    pub fn recv_within(&self, ms: u64) -> Option<String> {
        self.s.set_read_timeout(Some(Duration::from_millis(ms.max(1)))).expect("timeout");
        let mut buf = vec![0u8; 70000];
        match self.s.recv(&mut buf) {
            Ok(n) => Some(String::from_utf8_lossy(&buf[..n]).into_owned()),
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => None,
            Err(e) => panic!("recv: {e}"),
        }
    }

    pub fn text(&self) -> String {
        self.recv_within(10_000).expect("a datagram within ten seconds")
    }

    pub fn silence(&self, ms: u64) {
        if let Some(t) = self.recv_within(ms) {
            panic!("expected silence, got {t}");
        }
    }
}

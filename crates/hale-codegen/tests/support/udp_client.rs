//! A datagram client for the UDP hub tests (GH #1417, R5), and the start of a
//! program whose hub is a `udp::Hub`: no connection to wait for, so the program
//! is up when it has written its first statistics.

#![allow(dead_code)]

use std::net::UdpSocket;
use std::path::Path;
use std::time::{Duration, Instant};

use super::harness;
use super::ws_hub::Server;

/// A UDP port nothing holds (a draw another process takes first is drawn again by
/// [`start`]).
pub fn free_udp_port() -> u16 {
    UdpSocket::bind("127.0.0.1:0").expect("bind an ephemeral port").local_addr().expect("addr").port()
}

/// Run `bin` with its hub on a free UDP port, and wait until it has bound it.
pub fn start(bin: &Path, env: &[(&str, &str)]) -> Server {
    for _ in 0..10 {
        let port = free_udp_port();
        let dir = harness::unique_dir("udphub");
        let mut s = Server::start_on(bin, env, port, dir);
        let begin = Instant::now();
        loop {
            if s.stat("made").is_some() {
                return s;
            }
            if s.exited() || begin.elapsed() > Duration::from_secs(30) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    panic!("the program never bound its datagram port");
}

pub struct Udp {
    s: UdpSocket,
    pub port: u16,
}

impl Udp {
    /// A client socket, sending to the hub on `port`.
    pub fn to(port: u16) -> Udp {
        let s = UdpSocket::bind("127.0.0.1:0").expect("bind a client socket");
        s.connect(("127.0.0.1", port)).expect("connect");
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

//! A WebSocket client for the hub tests (GH #1417, R5): the handshake of
//! RFC 6455, masked text frames out, and frames in, with every wait bounded.

#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

/// What a read of a connection found.
#[derive(Debug, PartialEq, Clone)]
pub enum Frame {
    Text(String),
    Binary(Vec<u8>),
    Close(Vec<u8>),
    Ping(Vec<u8>),
    Pong(Vec<u8>),
    /// the server closed the connection (no close frame, or after one)
    Eof,
    /// nothing within the wait
    Timeout,
}

/// The accept key RFC 6455 § 1.3 derives from this key.
pub const RFC_KEY: &str = "dGhlIHNhbXBsZSBub25jZQ==";
pub const RFC_ACCEPT: &str = "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=";

pub struct Ws {
    s: TcpStream,
    buf: Vec<u8>,
    /// the response's head, as the server wrote it
    pub head: String,
}

impl Ws {
    /// Connect with `Authorization: Bearer <token>` (or none), and expect
    /// the upgrade.
    pub fn connect(port: u16, token: Option<&str>) -> Ws {
        let (ws, status) = Ws::try_connect(port, &headers(token), "/");
        assert!(status.starts_with("HTTP/1.1 101"), "the upgrade was refused: {}", ws.head);
        ws
    }

    /// Connect presenting the credential as `?access_token=`.
    pub fn connect_with_query(port: u16, token: &str) -> Ws {
        let (ws, status) = Ws::try_connect(port, &headers(None), &format!("/?access_token={token}"));
        assert!(status.starts_with("HTTP/1.1 101"), "the upgrade was refused: {}", ws.head);
        ws
    }

    /// Send a handshake with `extra` header lines and read the response head.
    pub fn try_connect(port: u16, extra: &str, path: &str) -> (Ws, String) {
        let mut s = connect_tcp(port);
        let req = format!(
            "GET {path} HTTP/1.1\r\nHost: hub\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {RFC_KEY}\r\nSec-WebSocket-Version: 13\r\n{extra}\r\n"
        );
        s.write_all(req.as_bytes()).expect("write the handshake");
        let mut ws = Ws { s, buf: Vec::new(), head: String::new() };
        let end = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(at) = find(&ws.buf, b"\r\n\r\n") {
                let head: Vec<u8> = ws.buf.drain(..at + 4).collect();
                ws.head = String::from_utf8_lossy(&head).into_owned();
                break;
            }
            assert!(Instant::now() < end, "no response to the handshake");
            let mut chunk = [0u8; 4096];
            ws.s.set_read_timeout(Some(Duration::from_secs(5))).expect("timeout");
            match ws.s.read(&mut chunk) {
                Ok(0) => panic!("closed during the handshake: {:?}", String::from_utf8_lossy(&ws.buf)),
                Ok(n) => ws.buf.extend_from_slice(&chunk[..n]),
                Err(e) => panic!("handshake read: {e}"),
            }
        }
        let status = ws.head.lines().next().unwrap_or("").to_string();
        (ws, status)
    }

    pub fn accept_key(&self) -> String {
        self.head
            .lines()
            .find_map(|l| l.strip_prefix("Sec-WebSocket-Accept: "))
            .unwrap_or("")
            .to_string()
    }

    fn write_frame(&mut self, op: u8, payload: &[u8]) {
        let key = [0x37u8, 0xfa, 0x21, 0x3d];
        let mut out = vec![0x80 | op];
        let n = payload.len();
        if n < 126 {
            out.push(0x80 | n as u8);
        } else if n < 65536 {
            out.push(0x80 | 126);
            out.extend_from_slice(&(n as u16).to_be_bytes());
        } else {
            out.push(0x80 | 127);
            out.extend_from_slice(&(n as u64).to_be_bytes());
        }
        out.extend_from_slice(&key);
        out.extend(payload.iter().enumerate().map(|(i, b)| b ^ key[i % 4]));
        self.s.write_all(&out).expect("write a frame");
    }

    pub fn send_text(&mut self, text: &str) {
        self.write_frame(1, text.as_bytes());
    }

    pub fn send_ping(&mut self, payload: &[u8]) {
        self.write_frame(9, payload);
    }

    pub fn send_close(&mut self) {
        self.write_frame(8, &[0x03, 0xe8]);
    }

    pub fn subscribe(&mut self, topic: &str) {
        self.send_text(&format!("{{\"type\":\"subscribe\",\"topic\":\"{topic}\"}}"));
    }

    /// Drop the connection with no closing handshake.
    pub fn abandon(self) {
        let _ = self.s.shutdown(std::net::Shutdown::Both);
    }

    /// The next frame, waiting at most `ms`.
    pub fn recv_within(&mut self, ms: u64) -> Frame {
        let end = Instant::now() + Duration::from_millis(ms);
        loop {
            if let Some((frame, used)) = parse(&self.buf) {
                self.buf.drain(..used);
                return frame;
            }
            let now = Instant::now();
            if now >= end {
                return Frame::Timeout;
            }
            self.s.set_read_timeout(Some(end - now)).expect("timeout");
            let mut chunk = [0u8; 65536];
            match self.s.read(&mut chunk) {
                Ok(0) => return Frame::Eof,
                Ok(n) => self.buf.extend_from_slice(&chunk[..n]),
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                    return Frame::Timeout
                }
                Err(_) => return Frame::Eof,
            }
        }
    }

    /// The next text frame; any other outcome fails the test.
    pub fn text(&mut self) -> String {
        match self.recv_within(10_000) {
            Frame::Text(t) => t,
            other => panic!("expected a text frame, got {other:?}"),
        }
    }

    /// Every text frame that arrives until `quiet_ms` pass with none.
    pub fn texts_until_quiet(&mut self, quiet_ms: u64) -> Vec<String> {
        let mut out = Vec::new();
        loop {
            match self.recv_within(quiet_ms) {
                Frame::Text(t) => out.push(t),
                Frame::Timeout | Frame::Eof => return out,
                other => panic!("unexpected {other:?} after {out:?}"),
            }
        }
    }

    /// Ping with `payload`, then every text frame that arrives before its
    /// pong. The pong is written after the frames queued before the ping,
    /// so what comes back is the connection's queue, drained.
    pub fn texts_until_pong(&mut self, payload: &[u8]) -> Vec<String> {
        self.send_ping(payload);
        let mut out = Vec::new();
        loop {
            match self.recv_within(30_000) {
                Frame::Text(t) => out.push(t),
                Frame::Pong(p) if p == payload => return out,
                other => panic!("expected a text frame or the pong, got {other:?} after {} frames", out.len()),
            }
        }
    }

    /// The test expects nothing for `ms`.
    pub fn silence(&mut self, ms: u64) {
        match self.recv_within(ms) {
            Frame::Timeout => {}
            other => panic!("expected silence, got {other:?}"),
        }
    }
}

fn headers(token: Option<&str>) -> String {
    match token {
        Some(t) => format!("Authorization: Bearer {t}\r\n"),
        None => String::new(),
    }
}

fn connect_tcp(port: u16) -> TcpStream {
    let start = Instant::now();
    loop {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(s) => return s,
            Err(e) => {
                assert!(start.elapsed() < Duration::from_secs(20), "could not connect to {port}: {e}");
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// One server frame at the front of `buf`, and how many bytes it took.
fn parse(buf: &[u8]) -> Option<(Frame, usize)> {
    if buf.len() < 2 {
        return None;
    }
    let op = buf[0] & 15;
    let masked = buf[1] & 128 != 0;
    let mut n = (buf[1] & 127) as usize;
    let mut at = 2;
    if n == 126 {
        if buf.len() < 4 {
            return None;
        }
        n = u16::from_be_bytes([buf[2], buf[3]]) as usize;
        at = 4;
    } else if n == 127 {
        if buf.len() < 10 {
            return None;
        }
        n = u64::from_be_bytes(buf[2..10].try_into().unwrap()) as usize;
        at = 10;
    }
    assert!(!masked, "a server frame is not masked");
    if buf.len() < at + n {
        return None;
    }
    let payload = buf[at..at + n].to_vec();
    let frame = match op {
        1 => Frame::Text(String::from_utf8_lossy(&payload).into_owned()),
        2 => Frame::Binary(payload),
        8 => Frame::Close(payload),
        9 => Frame::Ping(payload),
        10 => Frame::Pong(payload),
        other => panic!("opcode {other}"),
    };
    Some((frame, at + n))
}

/// `GET /.description` with the credential (or none): the status and body.
pub fn get_description(port: u16, token: Option<&str>) -> (u16, String) {
    let mut s = connect_tcp(port);
    s.set_read_timeout(Some(Duration::from_secs(20))).expect("timeout");
    let req = format!("GET /.description HTTP/1.1\r\nHost: hub\r\n{}Connection: close\r\n\r\n", headers(token));
    s.write_all(req.as_bytes()).expect("write");
    let mut all = Vec::new();
    s.read_to_end(&mut all).expect("read the response");
    let text = String::from_utf8_lossy(&all).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let status = head.split_whitespace().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    (status, body.to_string())
}

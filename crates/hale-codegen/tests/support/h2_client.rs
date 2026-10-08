//! A bare HTTP/2 client for the tests of the HTTP/2 server and `grpc::Rpc`
//! (GH #1417, R7): frames written and read by hand over a `TcpStream`, no
//! library. The client says what a test says and no more: a connection
//! (preface, SETTINGS, a window large enough that the server never waits on
//! it), a request's HEADERS and DATA, a stream's reset, a PING, and what the
//! server sent back, as headers, data and trailers per stream.
//!
//! HPACK is written out: requests use literals without indexing (any server
//! must accept them), and the decoder reads what nghttp2 sends, which is
//! the static table, a dynamic table and Huffman strings. The static table
//! and the Huffman codes are not typed here: they are read out of the
//! vendored library's own sources, so the client and the server cannot
//! disagree about them.

#![allow(dead_code)]

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

const HUFFMAN_SOURCE: &str = include_str!("../../runtime/third_party/nghttp2/nghttp2_hd_huffman_data.c");
const HD_SOURCE: &str = include_str!("../../runtime/third_party/nghttp2/nghttp2_hd.c");

pub const DATA: u8 = 0;
pub const HEADERS: u8 = 1;
pub const RST_STREAM: u8 = 3;
pub const SETTINGS: u8 = 4;
pub const PING: u8 = 6;
pub const GOAWAY: u8 = 7;
pub const WINDOW_UPDATE: u8 = 8;
pub const END_STREAM: u8 = 1;
pub const END_HEADERS: u8 = 4;

pub struct Frame {
    pub ty: u8,
    pub flags: u8,
    pub stream: u32,
    pub payload: Vec<u8>,
}

/// What the server sent on one stream.
#[derive(Default, Debug, Clone)]
pub struct Response {
    /// The first HEADERS block (`:status` first).
    pub headers: Vec<(String, String)>,
    /// The DATA, whole.
    pub data: Vec<u8>,
    /// The HEADERS block after the data, if there was one.
    pub trailers: Vec<(String, String)>,
    /// The error code of an RST_STREAM.
    pub reset: Option<u32>,
    /// Whether END_STREAM arrived.
    pub ended: bool,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().chain(self.trailers.iter()).find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }
    pub fn status(&self) -> u16 {
        self.header(":status").and_then(|s| s.parse().ok()).unwrap_or(0)
    }
}

struct Hpack {
    /// (bits, code right-aligned) -> symbol
    huffman: HashMap<(u8, u32), u16>,
    stat: Vec<(String, String)>,
    dynamic: Vec<(String, String)>,
    size: usize,
    max: usize,
}

fn prefix_int(buf: &[u8], at: &mut usize, bits: u8) -> usize {
    let mask = (1usize << bits) - 1;
    let mut v = (buf[*at] as usize) & mask;
    *at += 1;
    if v < mask {
        return v;
    }
    let mut shift = 0;
    loop {
        let b = buf[*at] as usize;
        *at += 1;
        v += (b & 0x7f) << shift;
        shift += 7;
        if b & 0x80 == 0 {
            return v;
        }
    }
}

fn write_prefix_int(out: &mut Vec<u8>, first: u8, bits: u8, mut v: usize) {
    let mask = (1usize << bits) - 1;
    if v < mask {
        out.push(first | v as u8);
        return;
    }
    out.push(first | mask as u8);
    v -= mask;
    while v >= 128 {
        out.push((v % 128) as u8 | 0x80);
        v /= 128;
    }
    out.push(v as u8);
}

impl Hpack {
    fn new() -> Hpack {
        let mut huffman = HashMap::new();
        let start = HUFFMAN_SOURCE.find("huff_sym_table[] = {").expect("the huffman table") + 20;
        let body = &HUFFMAN_SOURCE[start..];
        let body = &body[..body.find("};").expect("the table ends")];
        let mut sym = 0u16;
        for entry in body.split('{').skip(1) {
            let entry = &entry[..entry.find('}').unwrap()];
            let mut parts = entry.split(',');
            let bits: u8 = parts.next().unwrap().trim().parse().unwrap();
            let code = parts.next().unwrap().trim().trim_end_matches('u');
            let code = u32::from_str_radix(code.trim_start_matches("0x"), 16).unwrap();
            huffman.insert((bits, code >> (32 - bits as u32)), sym);
            sym += 1;
        }
        assert_eq!(sym, 257, "the huffman table has 256 symbols and EOS");
        let mut stat = Vec::new();
        for line in HD_SOURCE.lines() {
            if let Some(rest) = line.trim().strip_prefix("MAKE_STATIC_ENT(\"") {
                let (name, rest) = rest.split_once("\", \"").unwrap();
                let value = &rest[..rest.find('"').unwrap()];
                stat.push((name.to_string(), value.to_string()));
            }
        }
        assert_eq!(stat.len(), 61, "the static table");
        Hpack { huffman, stat, dynamic: Vec::new(), size: 0, max: 4096 }
    }

    fn string(&self, buf: &[u8], at: &mut usize) -> String {
        let huff = buf[*at] & 0x80 != 0;
        let n = prefix_int(buf, at, 7);
        let raw = &buf[*at..*at + n];
        *at += n;
        if !huff {
            return String::from_utf8_lossy(raw).into_owned();
        }
        let mut out = Vec::new();
        let (mut code, mut bits) = (0u32, 0u8);
        for byte in raw {
            for k in (0..8).rev() {
                code = (code << 1) | ((byte >> k) as u32 & 1);
                bits += 1;
                if let Some(s) = self.huffman.get(&(bits, code)) {
                    assert!(*s < 256, "EOS in a string");
                    out.push(*s as u8);
                    code = 0;
                    bits = 0;
                }
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    fn entry(&self, idx: usize) -> (String, String) {
        if idx >= 1 && idx <= 61 {
            return self.stat[idx - 1].clone();
        }
        self.dynamic[idx - 62].clone()
    }

    fn insert(&mut self, name: String, value: String) {
        self.size += name.len() + value.len() + 32;
        self.dynamic.insert(0, (name, value));
        while self.size > self.max {
            let (n, v) = self.dynamic.pop().expect("an entry to evict");
            self.size -= n.len() + v.len() + 32;
        }
    }

    fn decode(&mut self, buf: &[u8]) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let mut at = 0;
        while at < buf.len() {
            let b = buf[at];
            if b & 0x80 != 0 {
                let idx = prefix_int(buf, &mut at, 7);
                out.push(self.entry(idx));
            } else if b & 0x40 != 0 {
                let idx = prefix_int(buf, &mut at, 6);
                let name = if idx == 0 { self.string(buf, &mut at) } else { self.entry(idx).0 };
                let value = self.string(buf, &mut at);
                self.insert(name.clone(), value.clone());
                out.push((name, value));
            } else if b & 0x20 != 0 {
                self.max = prefix_int(buf, &mut at, 5);
                while self.size > self.max {
                    let (n, v) = self.dynamic.pop().expect("an entry to evict");
                    self.size -= n.len() + v.len() + 32;
                }
            } else {
                let idx = prefix_int(buf, &mut at, 4);
                let name = if idx == 0 { self.string(buf, &mut at) } else { self.entry(idx).0 };
                let value = self.string(buf, &mut at);
                out.push((name, value));
            }
        }
        out
    }
}

pub struct Client {
    sock: TcpStream,
    hpack: Hpack,
    inbox: Vec<u8>,
    /// The last GOAWAY the server sent: (last stream, error code).
    pub goaway: Option<(u32, u32)>,
    /// Whether the server closed the connection.
    pub eof: bool,
    /// The streams' responses so far.
    pub streams: HashMap<u32, Response>,
    /// How many PING acknowledgements have come back.
    pub pongs: usize,
    /// The server's SETTINGS (id, value), as last sent.
    pub settings: Vec<(u16, u32)>,
}

impl Client {
    /// Connect, send the preface and a SETTINGS that gives the server a window
    /// it never has to wait on, and read until the server's SETTINGS is in and
    /// acknowledged.
    pub fn connect(port: u16) -> Client {
        let sock = TcpStream::connect(("127.0.0.1", port)).expect("connect");
        sock.set_nodelay(true).ok();
        let mut c = Client { sock, hpack: Hpack::new(), inbox: Vec::new(), goaway: None, eof: false, streams: HashMap::new(), pongs: 0, settings: Vec::new() };
        c.write_all(b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n");
        // INITIAL_WINDOW_SIZE = 16 MiB
        c.frame(SETTINGS, 0, 0, &[0, 4, 1, 0, 0, 0]);
        c.frame(WINDOW_UPDATE, 0, 0, &(16u32 * 1024 * 1024).to_be_bytes());
        let until = Instant::now() + Duration::from_secs(10);
        while c.settings.is_empty() {
            assert!(Instant::now() < until, "the server's SETTINGS");
            c.pump(Duration::from_millis(200));
        }
        c
    }

    /// Close the write direction only: the read side stays open, and what the
    /// server has sent stays unread.
    pub fn half_close(&mut self) {
        self.sock.shutdown(std::net::Shutdown::Write).expect("half-close");
    }

    fn write_all(&mut self, bytes: &[u8]) {
        let _ = self.sock.write_all(bytes);
    }

    pub fn frame(&mut self, ty: u8, flags: u8, stream: u32, payload: &[u8]) {
        let mut f = Vec::with_capacity(9 + payload.len());
        f.extend_from_slice(&(payload.len() as u32).to_be_bytes()[1..]);
        f.push(ty);
        f.push(flags);
        f.extend_from_slice(&stream.to_be_bytes());
        f.extend_from_slice(payload);
        self.write_all(&f);
    }

    /// A HEADERS frame (END_HEADERS), each header a literal without indexing.
    pub fn headers(&mut self, stream: u32, headers: &[(&str, &str)], end_stream: bool) {
        let mut block = Vec::new();
        for (n, v) in headers {
            block.push(0);
            write_prefix_int(&mut block, 0, 7, n.len());
            block.extend_from_slice(n.as_bytes());
            write_prefix_int(&mut block, 0, 7, v.len());
            block.extend_from_slice(v.as_bytes());
        }
        self.frame(HEADERS, END_HEADERS | if end_stream { END_STREAM } else { 0 }, stream, &block);
    }

    pub fn data(&mut self, stream: u32, body: &[u8], end_stream: bool) {
        for (i, chunk) in body.chunks(16384).enumerate() {
            let last = (i + 1) * 16384 >= body.len();
            self.frame(DATA, if last && end_stream { END_STREAM } else { 0 }, stream, chunk);
        }
        if body.is_empty() && end_stream {
            self.frame(DATA, END_STREAM, stream, &[]);
        }
    }

    pub fn reset(&mut self, stream: u32, code: u32) {
        self.frame(RST_STREAM, 0, stream, &code.to_be_bytes());
    }

    pub fn ping(&mut self, token: [u8; 8]) {
        self.frame(PING, 0, 0, &token);
    }

    /// Bytes straight onto the connection.
    pub fn raw(&mut self, bytes: &[u8]) {
        self.write_all(bytes);
    }

    /// Read what has arrived (waiting up to `wait` for the first byte) and
    /// handle every whole frame in it.
    pub fn pump(&mut self, wait: Duration) {
        self.pump_at_most(wait, 65536);
    }

    /// `pump`, taking at most `cap` bytes from the socket: a client that
    /// reads slowly, so the server's sends meet a full buffer.
    pub fn pump_at_most(&mut self, wait: Duration, cap: usize) {
        self.sock.set_read_timeout(Some(wait.max(Duration::from_millis(1)))).ok();
        let mut buf = vec![0u8; cap.clamp(1, 65536)];
        match self.sock.read(&mut buf) {
            Ok(0) => self.eof = true,
            Ok(n) => self.inbox.extend_from_slice(&buf[..n]),
            Err(_) => {}
        }
        while self.inbox.len() >= 9 {
            let n = u32::from_be_bytes([0, self.inbox[0], self.inbox[1], self.inbox[2]]) as usize;
            if self.inbox.len() < 9 + n {
                break;
            }
            let ty = self.inbox[3];
            let flags = self.inbox[4];
            let stream = u32::from_be_bytes([self.inbox[5] & 0x7f, self.inbox[6], self.inbox[7], self.inbox[8]]);
            let payload: Vec<u8> = self.inbox[9..9 + n].to_vec();
            self.inbox.drain(..9 + n);
            self.handle(Frame { ty, flags, stream, payload });
        }
    }

    fn handle(&mut self, f: Frame) {
        match f.ty {
            SETTINGS if f.flags & 1 == 0 => {
                self.settings = f.payload.chunks(6).map(|c| (u16::from_be_bytes([c[0], c[1]]), u32::from_be_bytes([c[2], c[3], c[4], c[5]]))).collect();
                if self.settings.is_empty() {
                    // an empty SETTINGS is still a SETTINGS
                    self.settings.push((0, 0));
                }
                self.frame(SETTINGS, 1, 0, &[]);
            }
            PING => {
                if f.flags & 1 == 0 {
                    self.frame(PING, 1, 0, &f.payload);
                } else {
                    self.pongs += 1;
                }
            }
            GOAWAY => {
                let last = u32::from_be_bytes([f.payload[0] & 0x7f, f.payload[1], f.payload[2], f.payload[3]]);
                let code = u32::from_be_bytes([f.payload[4], f.payload[5], f.payload[6], f.payload[7]]);
                self.goaway = Some((last, code));
            }
            HEADERS => {
                let mut at = 0;
                let mut payload = &f.payload[..];
                if f.flags & 0x8 != 0 {
                    let pad = payload[0] as usize;
                    payload = &payload[1..payload.len() - pad];
                    at = 0;
                }
                if f.flags & 0x20 != 0 {
                    at += 5;
                }
                let block = self.hpack.decode(&payload[at..]);
                let r = self.streams.entry(f.stream).or_default();
                if r.headers.is_empty() {
                    r.headers = block;
                } else {
                    r.trailers = block;
                }
                if f.flags & END_STREAM != 0 {
                    r.ended = true;
                }
            }
            DATA => {
                let mut payload = &f.payload[..];
                if f.flags & 0x8 != 0 {
                    let pad = payload[0] as usize;
                    payload = &payload[1..payload.len() - pad];
                }
                let r = self.streams.entry(f.stream).or_default();
                r.data.extend_from_slice(payload);
                if f.flags & END_STREAM != 0 {
                    r.ended = true;
                }
            }
            RST_STREAM => {
                let code = u32::from_be_bytes([f.payload[0], f.payload[1], f.payload[2], f.payload[3]]);
                let r = self.streams.entry(f.stream).or_default();
                r.reset = Some(code);
                r.ended = true;
            }
            _ => {}
        }
    }

    /// Read until `stream` has ended (END_STREAM or a reset) or `within`
    /// passes; whatever it holds then.
    pub fn response(&mut self, stream: u32, within: Duration) -> Response {
        let until = Instant::now() + within;
        while Instant::now() < until && !self.eof {
            if self.streams.get(&stream).is_some_and(|r| r.ended) {
                break;
            }
            self.pump(Duration::from_millis(50));
        }
        self.streams.get(&stream).cloned().unwrap_or_default()
    }

    /// Read until every stream in `streams` has ended or `within` passes,
    /// `cap` bytes and then a pause of `pause` at a time.
    pub fn slowly(&mut self, streams: &[u32], within: Duration, cap: usize, pause: Duration) {
        let until = Instant::now() + within;
        while Instant::now() < until && !self.eof && !streams.iter().all(|s| self.streams.get(s).is_some_and(|r| r.ended)) {
            std::thread::sleep(pause);
            self.pump_at_most(Duration::from_millis(50), cap);
        }
    }

    /// Read until the server closes the connection or `within` passes.
    pub fn closed(&mut self, within: Duration) -> bool {
        let until = Instant::now() + within;
        while Instant::now() < until && !self.eof {
            self.pump(Duration::from_millis(50));
        }
        self.eof
    }

    /// Read until a GOAWAY has come or `within` passes.
    pub fn goaway_within(&mut self, within: Duration) -> Option<(u32, u32)> {
        let until = Instant::now() + within;
        while Instant::now() < until && self.goaway.is_none() && !self.eof {
            self.pump(Duration::from_millis(50));
        }
        self.goaway
    }

    /// Let the connection run for `wait`, reading whatever comes.
    pub fn idle(&mut self, wait: Duration) {
        let until = Instant::now() + wait;
        while Instant::now() < until && !self.eof {
            self.pump(Duration::from_millis(20));
        }
    }
}

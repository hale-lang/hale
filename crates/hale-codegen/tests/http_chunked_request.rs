//! `std::http::Server` reads a `Transfer-Encoding: chunked` request body
//! (downstream handoff, 2026-11-06): the handler saw the raw chunk
//! framing, or an empty body, since the per-connection loop read
//! `Content-Length` bodies only. The body is read through the framing
//! as it arrives (`__http_chunk_feed`, a state machine fed one read at
//! a time) and the handler sees the de-chunked body under its own
//! `Content-Length`, the chunked framing's headers removed; malformed
//! framing, or a body cut off before its last chunk, answers 400; a
//! chunked request over `max_request_bytes` answers `too_large` as an
//! over-cap `Content-Length` does.
//!
//! The framing is read as RFC 9112 has a server read it (review of
//! #1869, 2026-11-07): a chunk size is one to fifteen hex digits followed
//! by nothing but whitespace and a `;` extension (`10000000000000000`
//! wrapped to 0 and read the rest of the body as trailers, `0x5` was 0,
//! `5zz` was 5); `Transfer-Encoding` is the comma-separated list of all
//! its lines and must be exactly `chunked`; `Transfer-Encoding` beside a
//! `Content-Length`, and whitespace before a header's colon, answer 400
//! however the bytes were segmented.
//!
//! The tests read the server's port from `/proc`, so they are Linux-only.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Command, Stdio};
use std::time::Duration;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;
#[path = "support/child.rs"]
mod child;

// The repro's echo: the body's length and bytes, and the framing headers,
// as the handler saw them.
const ECHO_SERVER: &str = r#"
    locus Echo {
        fn handle(req: std::http::Request) -> std::http::Response {
            return std::http::Response {
                status: 200,
                content_type: "text/plain",
                body: "len=" + to_string(len(req.body)) + " body=[" + req.body + "]"
                    + " te=[" + std::http::header(req, "Transfer-Encoding") + "]"
                    + " cl=[" + std::http::header(req, "Content-Length") + "]",
            };
        }
    }
    fn main() {
        std::http::Server {
            host: "127.0.0.1",
            port: 0,
            handler: Echo { },
            max_request_bytes: 4096,
            ready_signal: "ready"
        };
    }
"#;

struct Stopped(std::process::Child);
impl Drop for Stopped {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn serve() -> (Stopped, u16, std::path::PathBuf) {
    let bin = harness::unique_bin("http_chunked_request");
    build_opts::build_source(ECHO_SERVER, &bin, &build_opts::options()).expect("build");
    // port 0, read back once the server says it listens
    let mut child = Command::new(&bin).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().expect("spawn the server");
    let stdout = child.stdout.take().unwrap();
    let mut child = Stopped(child);
    let mut ready = String::new();
    let _ = BufReader::new(stdout).read_line(&mut ready);
    assert_eq!(ready.trim(), "ready", "the server did not start");
    let port = child::tcp_port(&mut child.0);
    (child, port, bin)
}

/// One raw request on its own connection: the status line and the body.
fn raw(port: u16, request: &[u8]) -> (String, String) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    s.write_all(request).expect("write the request");
    let mut resp = Vec::new();
    s.read_to_end(&mut resp).expect("read the response");
    let text = String::from_utf8_lossy(&resp).to_string();
    let status = text.lines().next().unwrap_or("").to_string();
    let body = text.split_once("\r\n\r\n").map(|(_, b)| b.to_string()).unwrap_or_default();
    (status, body)
}

#[test]
fn a_chunked_request_body_reaches_the_handler_de_chunked() {
    let (child, port, bin) = serve();

    // the repro's request, headers and body in one write
    let (status, body) = raw(port, b"POST / HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n");
    assert!(status.starts_with("HTTP/1.1 200"), "{status}");
    assert_eq!(body, "len=5 body=[hello] te=[] cl=[5]");

    // several chunks, a chunk extension and trailers, headers and body
    // in separate segments
    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    s.write_all(b"POST / HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n").unwrap();
    std::thread::sleep(Duration::from_millis(50));
    s.write_all(b"3\r\nabc\r\n").unwrap();
    std::thread::sleep(Duration::from_millis(50));
    s.write_all(b"4\r\ndefg\r\n0\r\nX-Trailer: t\r\n\r\n").unwrap();
    let mut resp = Vec::new();
    s.read_to_end(&mut resp).unwrap();
    let text = String::from_utf8_lossy(&resp);
    assert!(text.starts_with("HTTP/1.1 200"), "{text}");
    assert!(text.ends_with("len=7 body=[abcdefg] te=[] cl=[7]"), "{text}");

    // a body whose chunk data holds a blank line of its own
    let (status, body) = raw(port, b"POST / HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n6\r\na\r\n\r\nb\r\n0\r\n\r\n");
    assert!(status.starts_with("HTTP/1.1 200"), "{status}");
    assert_eq!(body, "len=6 body=[a\r\n\r\nb] te=[] cl=[6]");

    // a Content-Length body is served as before
    let (status, body) = raw(port, b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 3\r\n\r\nxyz");
    assert!(status.starts_with("HTTP/1.1 200"), "{status}");
    assert_eq!(body, "len=3 body=[xyz] te=[] cl=[3]");

    drop(child);
    let _ = std::fs::remove_file(&bin);
}

#[test]
fn malformed_or_over_cap_chunked_framing_is_refused() {
    let (child, port, bin) = serve();

    // a chunk whose data is not followed by CRLF: 400
    let (status, _) = raw(port, b"POST / HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhelloXX0\r\n\r\n");
    assert!(status.starts_with("HTTP/1.1 400"), "{status}");

    // a chunk size that is not hex: 400
    let (status, _) = raw(port, b"POST / HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\nzz\r\nhello\r\n0\r\n\r\n");
    assert!(status.starts_with("HTTP/1.1 400"), "{status}");

    // a chunked body over max_request_bytes (4096): too_large, as an
    // over-cap Content-Length answers
    let mut big = b"POST / HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
    big.extend_from_slice(format!("{:x}\r\n", 5000).as_bytes());
    big.extend(std::iter::repeat(b'z').take(5000));
    big.extend_from_slice(b"\r\n0\r\n\r\n");
    let (status, _) = raw(port, &big);
    assert!(status.starts_with("HTTP/1.1 413"), "{status}");

    drop(child);
    let _ = std::fs::remove_file(&bin);
}

/// One raw request on its own connection, written in two segments with a
/// pause between them.
fn raw_split(port: u16, request: &[u8], at: usize) -> (String, String) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    s.write_all(&request[..at]).expect("write the first segment");
    std::thread::sleep(Duration::from_millis(50));
    s.write_all(&request[at..]).expect("write the second segment");
    let mut resp = Vec::new();
    s.read_to_end(&mut resp).expect("read the response");
    let text = String::from_utf8_lossy(&resp).to_string();
    let status = text.lines().next().unwrap_or("").to_string();
    let body = text.split_once("\r\n\r\n").map(|(_, b)| b.to_string()).unwrap_or_default();
    (status, body)
}

const CHUNKED_HEAD: &[u8] = b"POST / HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n";

/// A chunk size is one to fifteen hex digits, then at most whitespace
/// and a `;` extension (RFC 9112 § 7.1). The first cut's `v * 16 + d`
/// had no digit limit and stopped at the first non-hex byte, so a
/// 17-digit size wrapped to 0 and the rest of the body, a smuggled
/// request line included, was read as trailers (200, empty body); `0x5`
/// was 0 and `5zz` was 5.
#[test]
fn a_chunk_size_is_at_most_fifteen_hex_digits_then_an_extension() {
    let (child, port, bin) = serve();
    let req = |framing: &[u8]| {
        let mut r = CHUNKED_HEAD.to_vec();
        r.extend_from_slice(framing);
        r
    };

    // the wraparound: 2^64 as a size
    let (status, body) = raw(port, &req(b"10000000000000000\r\nGET /smuggled HTTP/1.1\r\n\r\n"));
    assert!(status.starts_with("HTTP/1.1 400"), "{status} {body}");
    // sixteen digits, even of zeros: over the limit
    let (status, _) = raw(port, &req(b"0000000000000005\r\nhello\r\n0\r\n\r\n"));
    assert!(status.starts_with("HTTP/1.1 400"), "{status}");
    // fifteen is a size
    let (status, body) = raw(port, &req(b"000000000000005\r\nhello\r\n0\r\n\r\n"));
    assert!(status.starts_with("HTTP/1.1 200"), "{status}");
    assert_eq!(body, "len=5 body=[hello] te=[] cl=[5]");
    // a byte after the digits that is not whitespace or `;`
    for framing in [&b"0x5\r\nhello\r\n0\r\n\r\n"[..], b"5zz\r\nhello\r\n0\r\n\r\n", b"5-\r\nhello\r\n0\r\n\r\n", b"5\r\nhello\r\n0x\r\n\r\n"] {
        let (status, body) = raw(port, &req(framing));
        assert!(status.starts_with("HTTP/1.1 400"), "{status} {body} for {:?}", String::from_utf8_lossy(framing));
        assert_eq!(body, "malformed chunked request body");
    }
    // an extension, with and without whitespace before it; trailing whitespace
    for framing in [&b"5;ext=1\r\nhello\r\n0\r\n\r\n"[..], b"5 ; ext=1\r\nhello\r\n0\r\n\r\n", b"5 \r\nhello\r\n0\t\r\n\r\n", b"5\t;x\r\nhello\r\n0\r\n\r\n"] {
        let (status, body) = raw(port, &req(framing));
        assert!(status.starts_with("HTTP/1.1 200"), "{status} for {:?}", String::from_utf8_lossy(framing));
        assert_eq!(body, "len=5 body=[hello] te=[] cl=[5]");
    }

    drop(child);
    let _ = std::fs::remove_file(&bin);
}

/// `Transfer-Encoding` is the comma-separated list of every line carrying
/// it, accepted only when it is exactly `chunked` (RFC 9112 § 6.1: the
/// final coding must be chunked, and the Server applies no other). The
/// first cut matched `chunked` as a substring of the first line, so
/// `xchunked` and `chunked, gzip` were de-chunked, `gzip, chunked` handed
/// the handler gzip bytes, and a `gzip` line before a `chunked` line, or
/// whitespace before the colon, handed it the raw chunk framing.
#[test]
fn transfer_encoding_is_the_list_of_all_its_lines_and_exactly_chunked() {
    let (child, port, bin) = serve();
    let req = |headers: &str| format!("POST / HTTP/1.1\r\nHost: x\r\n{headers}\r\n5\r\nhello\r\n0\r\n\r\n").into_bytes();

    for (headers, answer) in [
        ("Transfer-Encoding: xchunked\r\n", "unsupported transfer encoding"),
        ("Transfer-Encoding: chunked, gzip\r\n", "unsupported transfer encoding"),
        ("Transfer-Encoding: gzip, chunked\r\n", "unsupported transfer encoding"),
        ("Transfer-Encoding: gzip\r\nTransfer-Encoding: chunked\r\n", "unsupported transfer encoding"),
        ("Transfer-Encoding: chunked\r\nTransfer-Encoding: chunked\r\n", "unsupported transfer encoding"),
        ("Transfer-Encoding: \r\n", "unsupported transfer encoding"),
        ("Transfer-Encoding : chunked\r\n", "whitespace before a header's colon"),
        ("Transfer-Encoding\t: chunked\r\n", "whitespace before a header's colon"),
        ("X-Other : 1\r\nTransfer-Encoding: chunked\r\n", "whitespace before a header's colon"),
    ] {
        let (status, body) = raw(port, &req(headers));
        assert!(status.starts_with("HTTP/1.1 400"), "{status} for {headers:?}");
        assert_eq!(body, answer, "for {headers:?}");
    }
    // the coding's name is case-insensitive, and empty list members are
    // skipped (RFC 9110 § 5.6.1)
    for headers in ["Transfer-Encoding: Chunked\r\n", "transfer-encoding: CHUNKED\r\n", "Transfer-Encoding: , chunked ,\r\n", "Transfer-Encoding:chunked\r\n"] {
        let (status, body) = raw(port, &req(headers));
        assert!(status.starts_with("HTTP/1.1 200"), "{status} for {headers:?}");
        assert_eq!(body, "len=5 body=[hello] te=[] cl=[5]", "for {headers:?}");
    }

    drop(child);
    let _ = std::fs::remove_file(&bin);
}

/// `Transfer-Encoding` beside a `Content-Length` is refused (RFC 9112
/// § 6.3: a server rejects the pair) whatever the segmentation of the
/// bytes across reads: the first cut's answer depended on it (200 in one
/// write, the cap's 413 when split).
#[test]
fn transfer_encoding_with_content_length_is_refused_however_segmented() {
    let (child, port, bin) = serve();
    let request = b"POST / HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\nContent-Length: 5\r\n\r\n5\r\nhello\r\n0\r\n\r\n";
    let head = request.iter().collect::<Vec<_>>().windows(4).position(|w| w == [&b'\r', &b'\n', &b'\r', &b'\n']).unwrap() + 4;

    let (status, body) = raw(port, request);
    assert!(status.starts_with("HTTP/1.1 400"), "{status}");
    assert_eq!(body, "Transfer-Encoding with Content-Length");
    // headers and body in separate segments
    let (status, body) = raw_split(port, request, head);
    assert!(status.starts_with("HTTP/1.1 400"), "{status}");
    assert_eq!(body, "Transfer-Encoding with Content-Length");
    // the split inside the header block, between the two framing lines
    let (status, body) = raw_split(port, request, 50);
    assert!(status.starts_with("HTTP/1.1 400"), "{status}");
    assert_eq!(body, "Transfer-Encoding with Content-Length");
    // the Content-Length first, and a declared length over the cap
    let (status, body) = raw(port, b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 999999\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n");
    assert!(status.starts_with("HTTP/1.1 400"), "{status}");
    assert_eq!(body, "Transfer-Encoding with Content-Length");

    drop(child);
    let _ = std::fs::remove_file(&bin);
}

/// The handler sees the de-chunked body under its own `Content-Length`,
/// the request's `Transfer-Encoding` and any `Content-Length` line gone,
/// so a handler copying the headers onto an upstream request frames what
/// it forwards. The other headers, and the request line, are as they came.
#[test]
fn the_handler_sees_the_de_chunked_body_under_its_own_content_length() {
    const HEADER_ECHO: &str = r#"
        locus Echo {
            fn handle(req: std::http::Request) -> std::http::Response {
                return std::http::Response {
                    status: 200,
                    content_type: "text/plain",
                    body: req.method + " " + req.path + "\n" + req.headers + "\n[" + req.body + "]",
                };
            }
        }
        fn main() {
            std::http::Server {
                host: "127.0.0.1",
                port: 0,
                handler: Echo { },
                max_request_bytes: 4096,
                ready_signal: "ready"
            };
        }
    "#;
    let bin = harness::unique_bin("http_chunked_headers");
    build_opts::build_source(HEADER_ECHO, &bin, &build_opts::options()).expect("build");
    let mut child = Command::new(&bin).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().expect("spawn the server");
    let stdout = child.stdout.take().unwrap();
    let mut child = Stopped(child);
    let mut ready = String::new();
    let _ = BufReader::new(stdout).read_line(&mut ready);
    assert_eq!(ready.trim(), "ready", "the server did not start");
    let port = child::tcp_port(&mut child.0);

    let (status, body) = raw(port, b"POST /up?x=1 HTTP/1.1\r\nHost: x\r\nX-Before: a\r\nTransfer-Encoding: chunked\r\nX-After: b\r\n\r\n3\r\nabc\r\n4\r\ndefg\r\n0\r\n\r\n");
    assert!(status.starts_with("HTTP/1.1 200"), "{status}");
    assert_eq!(body, "POST /up?x=1\nHost: x\r\nX-Before: a\r\nX-After: b\r\nContent-Length: 7\n[abcdefg]");
    // a Content-Length request's headers are as they came
    let (status, body) = raw(port, b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 3\r\nX-After: b\r\n\r\nxyz");
    assert!(status.starts_with("HTTP/1.1 200"), "{status}");
    assert_eq!(body, "POST /\nHost: x\r\nContent-Length: 3\r\nX-After: b\n[xyz]");

    drop(child);
    let _ = std::fs::remove_file(&bin);
}

// A server taking requests of up to 16 MiB, answering with the length
// of the body it got (`a_large_request_costs_memory_linear_in_its_size`'s).
const LARGE_ECHO_SERVER: &str = r#"
    locus Count {
        fn handle(req: std::http::Request) -> std::http::Response {
            return std::http::Response { status: 200, body: to_string(len(req.body)) };
        }
    }
    fn main() {
        std::http::Server {
            host: "127.0.0.1",
            port: 0,
            handler: Count { },
            max_request_bytes: 16777216,
            ready_signal: "ready"
        };
    }
"#;

fn peak_kb(pid: u32) -> u64 {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap_or_default();
    status
        .lines()
        .find_map(|l| l.strip_prefix("VmHWM:"))
        .and_then(|v| v.split_whitespace().next())
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

/// One 12 MiB chunk whose data carries a blank line every 4 KiB, written
/// 4 KiB at a time (review of the first cut, downstream handoff): the
/// framing is consumed as it arrives, so the server's peak resident set
/// grows by a small multiple of the body and the handler sees all of it.
/// The first cut scanned a snapshot of everything so far on every read
/// whose bytes ended in a blank line: about 9 GiB of prefix copies for
/// this request.
#[cfg(target_os = "linux")]
#[test]
fn a_large_chunk_in_small_writes_costs_memory_linear_in_its_size() {
    let bin = harness::unique_bin("http_chunked_large");
    build_opts::build_source(LARGE_ECHO_SERVER, &bin, &build_opts::options()).expect("build");
    let mut child = Command::new(&bin).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().expect("spawn the server");
    let stdout = child.stdout.take().unwrap();
    let mut child = Stopped(child);
    let mut ready = String::new();
    let _ = BufReader::new(stdout).read_line(&mut ready);
    assert_eq!(ready.trim(), "ready", "the server did not start");
    let port = child::tcp_port(&mut child.0);
    let pid = child.0.id();
    let before = peak_kb(pid);

    // 4 KiB pieces, each ending in a blank line
    let piece: Vec<u8> = {
        let mut p = vec![b'x'; 4096 - 4];
        p.extend_from_slice(b"\r\n\r\n");
        p
    };
    let pieces = 3 * 1024;
    let size = piece.len() * pieces;
    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    s.set_read_timeout(Some(Duration::from_secs(120))).unwrap();
    s.write_all(format!("POST / HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n{size:x}\r\n").as_bytes()).unwrap();
    for _ in 0..pieces {
        s.write_all(&piece).unwrap();
    }
    s.write_all(b"\r\n0\r\n\r\n").unwrap();
    let mut reply = Vec::new();
    let _ = s.read_to_end(&mut reply);
    let reply = String::from_utf8_lossy(&reply);
    let peak = peak_kb(pid);
    drop(child);
    let _ = std::fs::remove_file(&bin);
    assert!(reply.starts_with("HTTP/1.1 200") && reply.trim_end().ends_with(&size.to_string()), "{reply}");
    eprintln!("peak RSS grew {} KB for a 12 MiB chunked request", peak.saturating_sub(before));
    // a linear assembly holds a few copies of 12 MiB; the per-read
    // snapshot held gigabytes
    assert!(peak.saturating_sub(before) < 96 * 1024, "peak RSS grew {} KB for a 12 MiB chunked request ({before} KB to {peak} KB)", peak.saturating_sub(before));
}

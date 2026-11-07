//! `std::http::Server` reads a `Transfer-Encoding: chunked` request body
//! (downstream handoff, 2026-11-06): the handler saw the raw chunk
//! framing, or an empty body, since the per-connection loop read
//! `Content-Length` bodies only. The body is now read to its last chunk
//! and de-chunked with the client's scanner before the handler sees it;
//! malformed framing, or a body cut off before its last chunk, answers
//! 400; a chunked request over `max_request_bytes` answers `too_large`
//! as an over-cap `Content-Length` does.

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

// The repro's echo: the body's length and bytes, as the handler saw them.
const ECHO_SERVER: &str = r#"
    locus Echo {
        fn handle(req: std::http::Request) -> std::http::Response {
            return std::http::Response {
                status: 200,
                content_type: "text/plain",
                body: "len=" + to_string(len(req.body)) + " body=[" + req.body + "]",
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
    assert_eq!(body, "len=5 body=[hello]");

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
    assert!(text.ends_with("len=7 body=[abcdefg]"), "{text}");

    // a body whose chunk data holds a blank line of its own
    let (status, body) = raw(port, b"POST / HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n6\r\na\r\n\r\nb\r\n0\r\n\r\n");
    assert!(status.starts_with("HTTP/1.1 200"), "{status}");
    assert_eq!(body, "len=6 body=[a\r\n\r\nb]");

    // a Content-Length body is served as before
    let (status, body) = raw(port, b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 3\r\n\r\nxyz");
    assert!(status.starts_with("HTTP/1.1 200"), "{status}");
    assert_eq!(body, "len=3 body=[xyz]");

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

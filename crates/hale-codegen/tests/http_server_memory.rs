//! GH #578: `std::http::Server` handled each connection inline in its
//! `run()`, which never returns, so every request's allocations — the
//! reassembled request, the parsed `Request`, the handler's `Response`
//! and its body — stayed in `run()`'s region for the life of the
//! process. A handler answering with a 450 KB body grew the server's
//! RSS by about that much per request (55 MB to 230 MB over 500), which
//! is how iris's `fuse-hl`, answering `/snapshot`, kept growing. Each
//! connection is now served in a child locus that dissolves with it.
//!
//! The oracle is the server's resident set, read from `/proc`: after a
//! warm-up it stays within a small margin over many more requests.

#![cfg(target_os = "linux")]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Command, Stdio};
use std::time::Duration;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

// A handler answering every request with the same ~450 KB body, held in
// its params: nothing grows but what a request leaves behind.
const BIG_BODY_SERVER: &str = r#"
    locus Big {
        params { body: String = ""; }
        fn handle(req: std::http::Request) -> std::http::Response {
            return std::http::Response { status: 200, body: self.body };
        }
    }
    fn body() -> String {
        let b = std::str::builder_new();
        let mut i = 0;
        while i < 20000 { std::str::builder_append(b, "{\"id\":" + i + ",\"t\":\"Kid\"},"); i = i + 1; }
        return std::str::builder_finish(b);
    }
    fn main() {
        let port = std::str::parse_int(std::env::arg(1)) or 0;
        std::http::Server {
            host: "127.0.0.1",
            port: port,
            handler: Big { body: body() },
            ready_signal: "ready"
        };
    }
"#;

fn rss_kb(pid: u32) -> u64 {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap_or_default();
    status
        .lines()
        .find_map(|l| l.strip_prefix("VmRSS:"))
        .and_then(|v| v.split_whitespace().next())
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

fn get(port: u16) -> usize {
    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    s.write_all(b"GET / HTTP/1.0\r\nHost: x\r\n\r\n").unwrap();
    let mut buf = Vec::new();
    let _ = s.read_to_end(&mut buf);
    buf.len()
}

#[test]
fn a_server_answering_large_bodies_keeps_its_memory_flat() {
    let program = hale_syntax::parse_source(BIG_BODY_SERVER).expect("parse");
    let bin = harness::unique_bin("http_server_memory");
    build_executable(&program, &bin).expect("build");
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let mut child = Command::new(&bin)
        .arg(port.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn the server");
    // the server's own word that it listens, never a guess
    let mut ready = String::new();
    let _ = BufReader::new(child.stdout.take().unwrap()).read_line(&mut ready);
    assert_eq!(ready.trim(), "ready", "the server did not start on {port}");
    let pid = child.id();

    for _ in 0..50 {
        assert!(get(port) > 400_000, "a full body");
    }
    let warm = rss_kb(pid);
    for _ in 0..400 {
        get(port);
    }
    let after = rss_kb(pid);
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_file(&bin);
    // 400 bodies of ~450 KB would be ~175 MB kept; a flat server moves
    // by at most a few chunks
    assert!(
        after <= warm + 16 * 1024,
        "RSS grew {} KB over 400 requests (from {warm} KB to {after} KB)",
        after.saturating_sub(warm)
    );
}

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

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;
#[path = "support/child.rs"]
mod child;

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

// The same server placed on an async_io pool, where each connection is a
// flow child of its own (reclaimed as its run() returns) rather than an
// inline child: the per-request region still goes with the connection.
const BIG_BODY_ASYNC_SERVER: &str = r#"
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
    main locus App {
        params {
            port: Int = std::str::parse_int(std::env::arg(1)) or 0;
            web: std::http::Server = std::http::Server {
                host: "127.0.0.1",
                port: self.port,
                handler: Big { body: body() },
                ready_signal: "ready"
            };
        }
        placement { web: cooperative(pool = web) where async_io; }
        run() {
            while !self.draining { std::time::sleep(20ms); }
        }
    }
    fn main() { App { }; }
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

/// The server, stopped however the test ends.
struct Stopped(std::process::Child);
impl Drop for Stopped {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
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
    memory_stays_flat(BIG_BODY_SERVER, "http_server_memory", 1);
}

#[test]
fn a_concurrent_server_answering_large_bodies_keeps_its_memory_flat() {
    // eight clients at a time, so connections are parked mid-response
    // while others complete and are reclaimed
    memory_stays_flat(BIG_BODY_ASYNC_SERVER, "http_server_memory_async", 8);
}

/// Warm the server with 50 requests, then send 400 more, `parallel` at a
/// time: its resident set moves by at most a few chunks.
fn memory_stays_flat(source: &str, name: &str, parallel: usize) {
    let bin = harness::unique_bin(name);
    build_opts::build_source(source, &bin, &build_opts::options()).expect("build");
    // port 0, read back once the server says it listens: a port drawn
    // here and handed over is free, not reserved, and another test of a
    // parallel run can bind it first
    let mut child = Command::new(&bin)
        .arg("0")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn the server");
    // the server's own word that it listens, never a guess
    let stdout = child.stdout.take().unwrap();
    let mut child = Stopped(child);
    let mut ready = String::new();
    let _ = BufReader::new(stdout).read_line(&mut ready);
    assert_eq!(ready.trim(), "ready", "the server did not start");
    let port = child::tcp_port(&mut child.0);
    let pid = child.0.id();

    let round = |n: usize| -> Vec<usize> {
        let mut sizes = Vec::new();
        for _ in 0..(n / parallel) {
            let hs: Vec<_> = (0..parallel).map(|_| std::thread::spawn(move || get(port))).collect();
            sizes.extend(hs.into_iter().map(|h| h.join().unwrap()));
        }
        sizes
    };
    for size in round(50) {
        assert!(size > 400_000, "a full body");
    }
    let warm = rss_kb(pid);
    for size in round(400) {
        assert!(size > 400_000, "a full body");
    }
    let after = rss_kb(pid);
    drop(child);
    let _ = std::fs::remove_file(&bin);
    // 400 bodies of ~450 KB would be ~175 MB kept; a flat server moves
    // by at most a few chunks
    assert!(
        after <= warm + 16 * 1024,
        "RSS grew {} KB over 400 requests (from {warm} KB to {after} KB)",
        after.saturating_sub(warm)
    );
}

// A server that takes requests of up to 16 MiB and answers each with the
// length of the body it got.
const LARGE_REQUEST_SERVER: &str = r#"
    locus Count {
        fn handle(req: std::http::Request) -> std::http::Response {
            return std::http::Response { status: 200, body: to_string(len(req.body)) };
        }
    }
    fn main() {
        let port = std::str::parse_int(std::env::arg(1)) or 0;
        std::http::Server {
            host: "127.0.0.1",
            port: port,
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

/// A request's bytes were assembled by appending each 8 KiB read to a
/// String, which kept every prefix alive until the connection was served:
/// memory quadratic in the body before any handler or auth ran (a 12 MiB
/// body held about 9.5 GB; a downstream handoff measured it). Assembled
/// in a builder, a 12 MiB request leaves the server's peak resident set
/// within a small multiple of the body, and the handler gets all of it.
#[test]
fn a_large_request_costs_memory_linear_in_its_size() {
    let bin = harness::unique_bin("http_large_request");
    build_opts::build_source(LARGE_REQUEST_SERVER, &bin, &build_opts::options()).expect("build");
    let mut child = Command::new(&bin)
        .arg("0")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn the server");
    let stdout = child.stdout.take().unwrap();
    let mut child = Stopped(child);
    let mut ready = String::new();
    let _ = BufReader::new(stdout).read_line(&mut ready);
    assert_eq!(ready.trim(), "ready", "the server did not start");
    let port = child::tcp_port(&mut child.0);
    let pid = child.0.id();
    let before = peak_kb(pid);

    let body = vec![b'x'; 12 * 1024 * 1024];
    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    s.set_read_timeout(Some(Duration::from_secs(60))).unwrap();
    s.write_all(format!("POST / HTTP/1.0\r\nHost: x\r\nContent-Length: {}\r\n\r\n", body.len()).as_bytes()).unwrap();
    s.write_all(&body).unwrap();
    let mut reply = Vec::new();
    let _ = s.read_to_end(&mut reply);
    let reply = String::from_utf8_lossy(&reply);
    let peak = peak_kb(pid);
    drop(child);
    let _ = std::fs::remove_file(&bin);
    assert!(reply.starts_with("HTTP/1.1 200") && reply.trim_end().ends_with(&body.len().to_string()), "{reply}");
    eprintln!("peak RSS grew {} KB for a 12 MiB request", peak.saturating_sub(before));
    // a linear assembly holds a few copies of 12 MiB; the quadratic one
    // held about 9.5 GB
    assert!(peak.saturating_sub(before) < 64 * 1024, "peak RSS grew {} KB for a 12 MiB request ({before} KB to {peak} KB)", peak.saturating_sub(before));
}

//! The TCP connection substrate (spec/stdlib.md § "The TCP connection
//! substrate"): a listener that only binds, accepts and relays each
//! connection over the bus, and consumers that serve what they are handed.
//!
//! Here a pinned `std::io::tcp::Acceptor` deals its connections over two
//! `std::io::tcp::Connections`, each on an async_io pool of its own and each
//! serving HTTP through `std::http::Responder` with a handler that sleeps a
//! second and answers with its replica's tag: the routing is the listener's
//! shards, the concurrency each consumer's placement. A connection on a
//! route no consumer takes is closed by its listener, never leaked; and the
//! program stops its listener across pools with `std::io::tcp::stop`.

#![cfg(target_os = "linux")]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;
#[path = "support/ports.rs"]
mod ports;

// Two replicas behind one listener of two shards.
const REPLICAS: &str = r#"
    locus Api {
        params { tag: String = ""; }
        fn handle(req: std::http::Request) -> std::http::Response {
            std::time::sleep(1000ms);
            return std::http::Response { status: 200, body: self.tag };
        }
    }
    main locus App {
        params {
            port: Int = std::str::parse_int(std::env::arg(1)) or 0;
            zero: std::io::tcp::Connections = std::io::tcp::Connections { listener: "web", shard: 0, handler: std::http::Responder { handler: Api { tag: "zero" } } };
            one: std::io::tcp::Connections = std::io::tcp::Connections { listener: "web", shard: 1, handler: std::http::Responder { handler: Api { tag: "one" } } };
            acceptor: std::io::tcp::Acceptor = std::io::tcp::Acceptor { port: self.port, name: "web", shards: 2, ready_signal: "ready" };
        }
        placement {
            zero: cooperative(pool = zero) where async_io;
            one: cooperative(pool = one) where async_io;
            acceptor: pinned;
        }
        run() {
            while !self.draining { std::time::sleep(20ms); }
            std::io::tcp::stop("web");
        }
    }
    fn main() { App { }; }
"#;

// The same listener with the shard-1 consumer missing: every second
// connection is on a route nobody takes.
const HALF: &str = r#"
    locus Api {
        fn handle(req: std::http::Request) -> std::http::Response {
            return std::http::Response { status: 200, body: "zero" };
        }
    }
    main locus App {
        params {
            port: Int = std::str::parse_int(std::env::arg(1)) or 0;
            zero: std::io::tcp::Connections = std::io::tcp::Connections { listener: "web", shard: 0, handler: std::http::Responder { handler: Api { } } };
            acceptor: std::io::tcp::Acceptor = std::io::tcp::Acceptor { port: self.port, name: "web", shards: 2, ready_signal: "ready" };
        }
        placement {
            zero: cooperative(pool = zero) where async_io;
            acceptor: pinned;
        }
        run() {
            while !self.draining { std::time::sleep(20ms); }
            std::io::tcp::stop("web");
        }
    }
    fn main() { App { }; }
"#;

struct Running {
    child: Child,
    port: u16,
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Running {
    fn start(bin: &Path) -> Running {
        let port = ports::free_port();
        let mut child = Command::new(bin)
            .arg(port.to_string())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn");
        let mut line = String::new();
        let _ = BufReader::new(child.stdout.take().unwrap()).read_line(&mut line);
        assert_eq!(line.trim(), "ready", "the listener did not come up on {port}");
        Running { child, port }
    }

    /// SIGTERM, and the program's exit and stderr.
    fn term(mut self) -> (std::process::ExitStatus, String) {
        let _ = Command::new("kill").args(["-TERM", &self.child.id().to_string()]).status();
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(status) = self.child.try_wait().expect("wait") {
                let mut err = String::new();
                if let Some(mut e) = self.child.stderr.take() {
                    let _ = e.read_to_string(&mut err);
                }
                return (status, err);
            }
            assert!(Instant::now() < deadline, "the program did not end on SIGTERM");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn open_fds(&self) -> usize {
        std::fs::read_dir(format!("/proc/{}/fd", self.child.id())).map(|d| d.count()).unwrap_or(0)
    }
}

/// One request: the status code and the body, or None when the connection
/// was closed without a response.
fn get(port: u16) -> Option<(u16, String)> {
    let mut s = TcpStream::connect(("127.0.0.1", port)).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
    s.write_all(b"GET / HTTP/1.0\r\nHost: x\r\n\r\n").ok()?;
    let mut buf = String::new();
    let _ = s.read_to_string(&mut buf);
    let code = buf.split_whitespace().nth(1)?.parse().ok()?;
    Some((code, buf.split("\r\n\r\n").nth(1).unwrap_or("").to_string()))
}

fn build(name: &str, source: &str) -> PathBuf {
    let bin = harness::unique_bin(name);
    build_opts::build_source(source, &bin, &build_opts::options()).expect("build");
    bin
}

#[test]
fn two_replicas_share_one_listener_and_serve_concurrently() {
    let bin = build("tcp_substrate_replicas", REPLICAS);
    let server = Running::start(&bin);
    let port = server.port;
    let t0 = Instant::now();
    let hs: Vec<_> = (0..10).map(|_| std::thread::spawn(move || get(port))).collect();
    let answers: Vec<_> = hs.into_iter().map(|h| h.join().unwrap()).collect();
    let took = t0.elapsed();
    let count = |tag: &str| answers.iter().filter(|a| a.as_ref().is_some_and(|(c, b)| *c == 200 && b == tag)).count();
    assert_eq!((count("zero"), count("one")), (5, 5), "the listener deals connections round-robin over its shards: {answers:?}");
    assert!(took < Duration::from_millis(2500), "ten 1 s requests over two async_io replicas took {took:?}");
    // the main locus stops the pinned listener across pools by its name
    let (status, stderr) = server.term();
    assert!(status.success(), "stop(\"web\") ends the accept and the program exits: {status:?}\n{stderr}");
    assert!(!stderr.contains("lotus_tcp_accept_one"), "a deliberate stop is silent: {stderr}");
    let _ = std::fs::remove_file(&bin);
}

#[test]
fn a_connection_no_consumer_takes_is_closed_by_its_listener() {
    let bin = build("tcp_substrate_unclaimed", HALF);
    let server = Running::start(&bin);
    let port = server.port;
    // a warm-up pair, then many: the shard-1 half is closed unanswered,
    // promptly, and the process's descriptors do not grow with it
    let _ = (get(port), get(port));
    let before = server.open_fds();
    let mut served = 0;
    let mut closed = 0;
    for _ in 0..40 {
        let t0 = Instant::now();
        match get(port) {
            Some((200, b)) if b == "zero" => served += 1,
            None => closed += 1,
            other => panic!("unexpected answer {other:?}"),
        }
        assert!(t0.elapsed() < Duration::from_secs(2), "an unclaimed connection is closed, not left hanging");
    }
    assert_eq!((served, closed), (20, 20), "the shard-0 half is served, the shard-1 half closed");
    std::thread::sleep(Duration::from_millis(200));
    let after = server.open_fds();
    assert!(after <= before + 2, "unclaimed connections leak no descriptor: {before} -> {after}");
    let (status, stderr) = server.term();
    assert!(status.success(), "{status:?}\n{stderr}");
    let _ = std::fs::remove_file(&bin);
}

#[test]
fn the_replicas_run_clean_under_asan() {
    // handles cross threads (pinned listener, two async pools) and each
    // connection's flow is reclaimed as it ends; with the chunk pool off a
    // reclaimed region read again would be reported
    let bin = harness::unique_bin("tcp_substrate_replicas_asan");
    harness::build_source_asan(REPLICAS, &bin);
    let port = ports::free_port();
    let mut child = Command::new(&bin)
        .arg(port.to_string())
        .env("LOTUS_NO_CHUNK_POOL", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    let mut line = String::new();
    let _ = BufReader::new(child.stdout.take().unwrap()).read_line(&mut line);
    assert_eq!(line.trim(), "ready");
    let server = Running { child, port };
    let hs: Vec<_> = (0..6).map(|_| std::thread::spawn(move || get(port))).collect();
    for h in hs {
        assert_eq!(h.join().unwrap().map(|(c, _)| c), Some(200));
    }
    let (status, stderr) = server.term();
    for bad in ["AddressSanitizer", "LeakSanitizer", "heap-use-after-free", "SUMMARY:"] {
        assert!(!stderr.contains(bad), "{bad} in:\n{stderr}");
    }
    assert!(status.success(), "{status:?}\n{stderr}");
    let _ = std::fs::remove_file(&bin);
}

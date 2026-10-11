//! `std::http::Server` placed on a `where async_io` pool serves its
//! connections concurrently, a coroutine each.
//!
//! It used to serve one connection at a time on every placement: its
//! accept loop built the per-connection child inline, and the child's
//! `birth()` handled the whole connection before the loop accepted the
//! next. On an `async_io` pool a handler's `sleep` parked the coroutine
//! that WAS the accept loop, so five concurrent requests to a handler that
//! sleeps a second took five seconds (measured on main: 5.0 s), and one
//! slow request held every other one, health checks included — a
//! downstream handoff whose requests take a minute each. Each connection
//! is now an accept'd flow child whose `run()` serves it, so a park yields
//! to the other connections and to the accept loop.
//!
//! The oracles are wall-clock bounds far from both shapes (the serial
//! server takes N seconds for N requests) and the responses themselves.

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

// The server on an async_io pool, serving until the process drains.
const SERVE: &str = r#"
    // a second on `/slow`; `/health` at once
    locus Slow {
        params { hits: Int = 0; }
        fn handle(req: std::http::Request) -> std::http::Response {
            if req.path == "/health" {
                return std::http::Response { status: 200, body: "ok" };
            }
            std::time::sleep(1000ms);
            self.hits = self.hits + 1;
            return std::http::Response { status: 200, body: std::str::upper("slow") };
        }
    }
    main locus App {
        params {
            port: Int = std::str::parse_int(std::env::arg(1)) or 0;
            web: std::http::Server = std::http::Server { host: "127.0.0.1", port: self.port, handler: Slow { }, ready_signal: "ready" };
        }
        placement { web: cooperative(pool = web) where async_io; }
        run() {
            while !self.draining { std::time::sleep(20ms); }
        }
    }
    fn main() { App { }; }
"#;

// The same server, stopped half a second in (with the slow requests in
// flight); the program ends two seconds later. The main locus stops the
// server on another pool by its name, `std::io::tcp::stop(name)`, which a
// checked program may call (a method call across pools is refused, F.31).
const STOP: &str = r#"
    // a second on `/slow`; `/health` at once
    locus Slow {
        params { hits: Int = 0; }
        fn handle(req: std::http::Request) -> std::http::Response {
            if req.path == "/health" {
                return std::http::Response { status: 200, body: "ok" };
            }
            std::time::sleep(1000ms);
            self.hits = self.hits + 1;
            return std::http::Response { status: 200, body: std::str::upper("slow") };
        }
    }
    main locus App {
        params {
            port: Int = std::str::parse_int(std::env::arg(1)) or 0;
            web: std::http::Server = std::http::Server { host: "127.0.0.1", port: self.port, handler: Slow { }, ready_signal: "ready", name: "web" };
        }
        placement { web: cooperative(pool = web) where async_io; }
        run() {
            std::time::sleep(500ms);
            std::io::tcp::stop("web");
            println("shutdown");
            std::time::sleep(2s);
            println("done");
        }
    }
    fn main() { App { }; }
"#;

/// The server, stopped however the test ends.
struct Running {
    child: Child,
    port: u16,
    lines: std::sync::mpsc::Receiver<String>,
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Running {
    fn start(bin: &Path, env: &[(&str, &str)]) -> Running {
        let port = ports::free_port();
        let mut cmd = Command::new(bin);
        cmd.arg(port.to_string()).stdout(Stdio::piped()).stderr(Stdio::piped());
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().expect("spawn the server");
        let stdout = child.stdout.take().unwrap();
        let (tx, lines) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let _ = tx.send(line);
            }
        });
        let running = Running { child, port, lines };
        running.expect_line("ready");
        running
    }

    fn expect_line(&self, want: &str) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            match self.lines.recv_timeout(Duration::from_millis(100)) {
                Ok(l) if l.trim() == want => return,
                Ok(_) | Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => break,
            }
        }
        panic!("the server never printed `{want}`");
    }

    /// The program's exit, and what it wrote to stderr.
    fn finish(mut self) -> (std::process::ExitStatus, String) {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(status) = self.child.try_wait().expect("wait") {
                let mut err = String::new();
                if let Some(mut e) = self.child.stderr.take() {
                    let _ = e.read_to_string(&mut err);
                }
                return (status, err);
            }
            assert!(Instant::now() < deadline, "the program did not end");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

/// One request: the status line's code and the body, or None when the
/// connection is refused or closed without a response.
fn get(port: u16, path: &str) -> Option<(u16, String)> {
    let mut s = TcpStream::connect(("127.0.0.1", port)).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
    s.write_all(format!("GET {path} HTTP/1.0\r\nHost: x\r\n\r\n").as_bytes()).ok()?;
    let mut buf = String::new();
    let _ = s.read_to_string(&mut buf);
    let code = buf.split_whitespace().nth(1)?.parse().ok()?;
    let body = buf.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
    Some((code, body))
}

/// `n` concurrent `/slow` requests: every answer, and how long the last
/// took.
fn burst(port: u16, n: usize) -> (Vec<Option<(u16, String)>>, Duration) {
    let t0 = Instant::now();
    let handles: Vec<_> = (0..n).map(|_| std::thread::spawn(move || get(port, "/slow"))).collect();
    let answers = handles.into_iter().map(|h| h.join().unwrap()).collect();
    (answers, t0.elapsed())
}

fn build(name: &str, source: &str, asan: bool) -> PathBuf {
    let bin = harness::unique_bin(name);
    if asan {
        harness::build_source_asan(source, &bin);
    } else {
        build_opts::build_source(source, &bin, &build_opts::options()).expect("build");
    }
    bin
}

#[test]
fn concurrent_slow_requests_take_one_handler_time_not_n() {
    let bin = build("http_server_concurrent", SERVE, false);
    let server = Running::start(&bin, &[]);
    for n in [5usize, 50] {
        let (answers, took) = burst(server.port, n);
        for a in &answers {
            assert_eq!(a.as_ref().map(|(c, b)| (*c, b.as_str())), Some((200, "SLOW")), "every request answered");
        }
        // served one at a time this is n seconds; concurrently, one
        assert!(took < Duration::from_millis(2500), "{n} concurrent 1 s requests took {took:?}");
    }
    drop(server);
    let _ = std::fs::remove_file(&bin);
}

#[test]
fn a_health_check_is_answered_while_slow_requests_are_in_flight() {
    let bin = build("http_server_concurrent_health", SERVE, false);
    let server = Running::start(&bin, &[]);
    let port = server.port;
    let slow: Vec<_> = (0..5).map(|_| std::thread::spawn(move || get(port, "/slow"))).collect();
    std::thread::sleep(Duration::from_millis(150));
    let t0 = Instant::now();
    let health = get(port, "/health");
    let took = t0.elapsed();
    assert_eq!(health, Some((200, "ok".to_string())));
    assert!(took < Duration::from_millis(500), "the health check waited {took:?} behind the slow requests");
    for h in slow {
        assert_eq!(h.join().unwrap().map(|(c, _)| c), Some(200));
    }
    drop(server);
    let _ = std::fs::remove_file(&bin);
}

/// `shutdown()` with requests in flight is a drain: every request accepted
/// before it is answered, nothing is accepted after it, and the program
/// ends cleanly. `bin` is the plain or the sanitized build.
fn shutdown_drains(bin: &Path, env: &[(&str, &str)]) -> String {
    let server = Running::start(bin, env);
    let port = server.port;
    let slow: Vec<_> = (0..8).map(|_| std::thread::spawn(move || get(port, "/slow"))).collect();
    server.expect_line("shutdown");
    // the listener is shut: a new connection is refused or closed unanswered
    assert_eq!(get(port, "/health"), None, "a request after shutdown() is not served");
    for h in slow {
        assert_eq!(
            h.join().unwrap(),
            Some((200, "SLOW".to_string())),
            "a request in flight at shutdown() is answered"
        );
    }
    server.expect_line("done");
    let (status, stderr) = server.finish();
    assert!(status.success(), "the program ends cleanly: {status:?}\n{stderr}");
    stderr
}

#[test]
fn shutdown_under_load_answers_the_requests_in_flight() {
    let bin = build("http_server_concurrent_stop", STOP, false);
    let stderr = shutdown_drains(&bin, &[]);
    assert!(!stderr.contains("lotus_tcp_accept_one"), "a deliberate shutdown is silent: {stderr}");
    let _ = std::fs::remove_file(&bin);
}

#[test]
fn concurrent_connections_and_their_drain_run_clean_under_asan() {
    // the flows' regions are reclaimed as each run() returns, while the
    // other connections are parked mid-request; the chunk pool is off so
    // a reclaimed region read again would be reported
    let bin = build("http_server_concurrent_asan", STOP, true);
    let stderr = shutdown_drains(&bin, &[("LOTUS_NO_CHUNK_POOL", "1")]);
    for bad in ["AddressSanitizer", "LeakSanitizer", "heap-use-after-free", "SUMMARY:"] {
        assert!(!stderr.contains(bad), "{bad} in:\n{stderr}");
    }
    let _ = std::fs::remove_file(&bin);
}

//! `std::io::tcp::Listener` on the TCP connection substrate (spec/stdlib.md
//! § "The TCP connection substrate"): a listener and a consumer in one
//! locus whose connection handler calls the `on_connection` callback with a
//! `Stream` over each connection. Placed on an async_io pool it serves its
//! connections concurrently, a coroutine each; anywhere else one at a time.
//! Its name is its bound address, so `std::io::tcp::stop` reaches it; and a
//! socket it cannot bind is no violation, as before it moved onto the
//! substrate: it accepts nothing and its run returns.

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
#[path = "support/child.rs"]
mod child;

use child::{listening_ports, Stderr};

// An echo listener that serves until the main locus stops it by the name
// written to its stdin (the listener's bound address).
const ECHO: &str = r#"
    fn echo(s: std::io::tcp::Stream) {
        let line = s.recv(64) or "";
        s.send("echo " + line) or discard;
    }
    main locus App {
        params {
            l: std::io::tcp::Listener = std::io::tcp::Listener { port: 0, max_accepts: -1, on_connection: echo };
        }
        placement { l: PLACEMENT; }
        run() {
            let name = std::io::stdin::read_line();
            println("stopped " + to_string(std::io::tcp::stop(name)));
        }
    }
    fn main() { App { }; }
"#;

fn build(name: &str, source: &str) -> PathBuf {
    let bin = harness::unique_bin(name);
    build_opts::build_source(source, &bin, &build_opts::options()).expect("build");
    bin
}

/// The listener's program, its stdout as lines, its stderr and its port.
struct Running {
    child: Child,
    lines: std::sync::mpsc::Receiver<String>,
    err: Stderr,
    port: u16,
}

impl Running {
    fn start(bin: &Path, args: &[&str]) -> Running {
        let mut child = Command::new(bin)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn");
        let err = Stderr::of(&mut child);
        let out = child.stdout.take().unwrap();
        let (tx, lines) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                let _ = tx.send(line);
            }
        });
        let mut r = Running { child, lines, err, port: 0 };
        r.wait_line("__StdIoTcpListener.birth host=127.0.0.1 port=");
        r
    }

    /// The port the listener bound, once it has (birth prints first).
    fn bound(&mut self) -> u16 {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let [port] = listening_ports(self.child.id())[..] {
                self.port = port;
                return port;
            }
            assert!(Instant::now() < deadline, "the listener never bound; stderr:\n{}", self.err.text());
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Read stdout until a line starting with `want` (or 20 s).
    fn wait_line(&self, want: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(left) {
                Ok(line) if line.starts_with(want) => return line,
                Ok(_) => {}
                Err(_) => panic!("no `{want}` line; stderr:\n{}", self.err.text()),
            }
        }
    }

    /// The program's exit, within 20 s.
    fn exit(&mut self) -> std::process::ExitStatus {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(status) = self.child.try_wait().expect("wait") {
                return status;
            }
            assert!(Instant::now() < deadline, "the program did not end; stderr:\n{}", self.err.text());
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn echo(port: u16, line: &str) -> String {
    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
    s.write_all(line.as_bytes()).unwrap();
    let mut buf = String::new();
    let _ = s.read_to_string(&mut buf);
    buf
}

/// Echo through the listener `placement` places, holding `silent`
/// connections open meanwhile; then stop it by its name.
fn echo_case(tag: &str, placement: &str, silent: usize) {
    let bin = build(tag, &ECHO.replace("PLACEMENT", placement));
    let mut r = Running::start(&bin, &[]);
    let port = r.bound();
    // connections that send nothing: on async_io each parks its own
    // coroutine in recv, and the next connection is served meanwhile
    let held: Vec<TcpStream> = (0..silent).map(|_| TcpStream::connect(("127.0.0.1", port)).expect("connect")).collect();
    let t0 = Instant::now();
    assert_eq!(echo(port, "hi"), "echo hi", "{tag}: the callback answers");
    assert!(t0.elapsed() < Duration::from_secs(5), "{tag}: answered behind {silent} silent connections in {:?}", t0.elapsed());
    drop(held);
    assert_eq!(echo(port, "again"), "echo again", "{tag}: the listener keeps serving");
    // the main locus stops the listener on another thread by its name
    let mut stdin = r.child.stdin.take().unwrap();
    stdin.write_all(format!("127.0.0.1:{port}\n").as_bytes()).unwrap();
    drop(stdin);
    assert_eq!(r.wait_line("stopped "), "stopped true", "{tag}: its name is its bound address");
    r.wait_line("__StdIoTcpListener.dissolve host=127.0.0.1 port=0");
    let status = r.exit();
    assert!(status.success(), "{tag}: the program ends once the listener is stopped: {status:?}\n{}", r.err.finished());
    let _ = std::fs::remove_file(&bin);
}

#[test]
fn an_async_io_listener_serves_its_connections_concurrently_and_stops_by_name() {
    echo_case("tcp_listener_async", "cooperative(pool = io) where async_io", 4);
}

#[test]
fn a_pinned_listener_serves_and_stops_by_name() {
    echo_case("tcp_listener_pinned", "pinned", 0);
}

// The Listener at statement position in `fn main` on a port the test holds:
// the bind fails, the Listener accepts nothing and `main` goes on.
const BIND_FAILS: &str = r#"
    fn main() {
        let port = std::str::parse_int(std::env::arg(1)) or 0;
        std::io::tcp::Listener { port: port };
        println("main goes on");
    }
"#;

#[test]
fn a_listener_that_cannot_bind_accepts_nothing() {
    let held = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = held.local_addr().unwrap().port().to_string();
    let bin = build("tcp_listener_bind_fails", BIND_FAILS);
    let mut r = Running::start(&bin, &[&port]);
    r.wait_line(&format!("__StdIoTcpListener.dissolve host=127.0.0.1 port={port}"));
    r.wait_line("main goes on");
    let status = r.exit();
    assert!(status.success(), "a socket it cannot bind is no violation: {status:?}\n{}", r.err.finished());
    drop(held);
    let _ = std::fs::remove_file(&bin);
}

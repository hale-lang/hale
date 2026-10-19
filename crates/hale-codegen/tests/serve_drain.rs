//! A served program and the drain, and a main thread that serves.
//!
//! - A SIGTERM's drain stops the process's listeners: a `std::http::Server`
//!   stops accepting, answers what it accepted, and its `run()` returns.
//!   Its accept loop never saw the drain, so a server on `main` or on a
//!   pinned thread held the drain for the whole grace (5 s) and the
//!   process died by the signal; a request in flight is still answered.
//! - An accept on the main thread waits on main's bus queue too: a cell
//!   another thread enqueues for a subscriber on main (a pinned locus's
//!   `std::log` line to a sink on main) is delivered when it arrives. It
//!   waited for the next connection, so on an idle server it never came.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;
#[path = "support/child.rs"]
mod child;

const HELLO: &str = r#"
locus Hello {
    params { n: Int = 0; }
    fn handle(req: std::http::Request) -> std::http::Response {
        if req.path == "/slow" {
            // busy, not asleep: a drain ends a sleep early
            let t = std::time::monotonic_ns();
            while std::time::monotonic_ns() - t < 400000000 { }
        }
        return std::http::Response { status: 200, body: "hi" };
    }
}
"#;

/// Build `src`, start it, wait for its listener, and return the child and
/// the port.
fn serve(tag: &str, src: &str) -> (std::process::Child, u16, std::path::PathBuf) {
    let bin = harness::unique_bin(tag);
    build_opts::build_source(&format!("{HELLO}{src}"), &bin, &build_opts::options()).expect("build");
    let mut c = Command::new(&bin)
        .env("LOTUS_DRAIN_GRACE_MS", "5000")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    let port = child::tcp_port(&mut c);
    (c, port, bin)
}

fn term_and_wait(mut c: std::process::Child) -> (std::process::ExitStatus, Duration, String) {
    let sent = Instant::now();
    Command::new("kill").args(["-TERM", &c.id().to_string()]).status().expect("kill");
    let status = c.wait().expect("wait");
    let took = sent.elapsed();
    let mut err = String::new();
    if let Some(mut e) = c.stderr.take() {
        let _ = e.read_to_string(&mut err);
    }
    (status, took, err)
}

#[test]
fn a_server_ends_on_the_drain_wherever_it_runs() {
    let shapes = [
        (
            "main",
            r#"
locus Watch { params { n: Int = 0; } run() { while !self.draining { std::time::sleep(20ms); } } }
main locus App {
    params { w: Watch = Watch { }; server: std::http::Server = std::http::Server { port: 0, handler: Hello { } }; }
    placement { w: pinned; }
}
fn main() { App { }; }
"#,
        ),
        (
            "pinned",
            r#"
main locus App {
    params { server: std::http::Server = std::http::Server { port: 0, handler: Hello { } }; }
    placement { server: pinned; }
    run() { while !self.draining { std::time::sleep(20ms); } }
}
fn main() { App { }; }
"#,
        ),
        (
            "async_io",
            r#"
main locus App {
    params { server: std::http::Server = std::http::Server { port: 0, handler: Hello { } }; }
    placement { server: cooperative(pool = web) where async_io; }
    run() { while !self.draining { std::time::sleep(20ms); } }
}
fn main() { App { }; }
"#,
        ),
    ];
    for (shape, src) in shapes {
        let (c, _port, bin) = serve(&format!("serve_drain_{shape}"), src);
        let (status, took, err) = term_and_wait(c);
        let _ = std::fs::remove_file(&bin);
        assert!(status.success(), "{shape}: the server ended and the program with it: {status:?} (signal {:?})\n{err}", status.signal());
        assert!(took < Duration::from_millis(2000), "{shape}: ended {took:?} after the signal, not at the grace\n{err}");
        assert!(!err.contains("did not finish"), "{shape}: {err}");
    }
}

/// The drain stops accepting, never cuts: a request the server is
/// handling when the signal comes is answered, then the program ends.
#[test]
fn a_request_in_flight_is_answered_before_the_server_ends() {
    let (c, port, bin) = serve(
        "serve_drain_in_flight",
        r#"
main locus App {
    params { server: std::http::Server = std::http::Server { port: 0, handler: Hello { } }; }
    placement { server: pinned; }
    run() { while !self.draining { std::time::sleep(20ms); } }
}
fn main() { App { }; }
"#,
    );
    let mut conn = std::net::TcpStream::connect(("127.0.0.1", port)).expect("connect");
    conn.write_all(b"GET /slow HTTP/1.1\r\nHost: x\r\n\r\n").expect("send");
    std::thread::sleep(Duration::from_millis(100));
    let (status, took, err) = term_and_wait(c);
    let mut answer = String::new();
    let _ = conn.read_to_string(&mut answer);
    let _ = std::fs::remove_file(&bin);
    assert!(answer.starts_with("HTTP/1.1 200"), "the request in flight was answered: {answer:?}\n{err}");
    assert!(status.success(), "{status:?}\n{err}");
    assert!(took < Duration::from_millis(2000), "{took:?}\n{err}");
}

/// A pinned locus logs to a sink on main while main serves and no
/// request comes: the line is written within a bound of its publish.
#[test]
fn a_log_line_to_main_arrives_while_main_serves_idle() {
    let bin = harness::unique_bin("serve_main_log");
    let src = format!(
        "{HELLO}{}",
        r#"
locus Background {
    params { log: std::log::Logger = std::log::Logger { name: "bg" }; }
    run() {
        std::time::sleep(300ms);
        self.log.warn("background says hi");
        std::time::sleep(30s);
    }
}
main locus App {
    params {
        sink: std::log::StdoutSink = std::log::StdoutSink { };
        bg: Background = Background { };
        server: std::http::Server = std::http::Server { port: 0, handler: Hello { } };
    }
    placement { bg: pinned; }
}
fn main() { App { }; }
"#
    );
    build_opts::build_source(&src, &bin, &build_opts::options()).expect("build");
    let started = Instant::now();
    let mut c = Command::new(&bin).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().expect("spawn");
    let mut err = BufReader::new(c.stdout.take().unwrap());
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        while err.read_line(&mut line).unwrap_or(0) > 0 {
            if line.contains("background says hi") {
                let _ = tx.send(line.clone());
                return;
            }
            line.clear();
        }
    });
    let got = rx.recv_timeout(Duration::from_secs(3));
    let _ = c.kill();
    let _ = c.wait();
    let _ = std::fs::remove_file(&bin);
    let line = got.expect("the pinned locus's log line is written while main serves, with no request");
    assert!(line.contains("level=warn logger=bg msg=\"background says hi\""), "{line:?}");
    assert!(started.elapsed() < Duration::from_secs(3));
}

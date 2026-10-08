//! A compiled program that serves over `unix::Rpc`, and the clients that
//! drive its socket (GH #1417, R2b).
//!
//! A test builds a program (`build_source`), starts it with [`Server`] and
//! talks to it over real connections ([`Conn`]). The program ends when the
//! test says so, not when a clock does: it watches a trigger file and, when
//! the file appears, stops its handle and returns from `run()`. [`Server::finish`]
//! writes the file and waits for the exit, so a test also holds the
//! program's own teardown to account (a program that does not exit within
//! the wait is a hang, and fails the test).

#![allow(dead_code)]

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use super::harness;

/// What the program printed and how it ended.
pub struct Finished {
    pub status: std::process::ExitStatus,
    pub stdout: String,
    pub stderr: String,
}

/// A running program and the directory it was given (its socket and its
/// trigger file live there).
pub struct Server {
    child: Option<Child>,
    pub dir: PathBuf,
    pub sock: PathBuf,
    trigger: PathBuf,
}

impl Server {
    /// Run `bin` with `SOCK` (the path to listen on) and `TRIGGER` (the
    /// file whose appearance stops it) in its environment, plus `env`.
    pub fn start(bin: &Path, env: &[(&str, &str)]) -> Server {
        let dir = harness::unique_dir("unixrpc");
        let sock = dir.join("s.sock");
        let trigger = dir.join("stop");
        assert!(sock.as_os_str().len() < 100, "the socket path is too long for sockaddr_un: {}", sock.display());
        let mut cmd = Command::new(bin);
        cmd.env("SOCK", &sock).env("SOCK2", dir.join("s2.sock")).env("SOCK3", dir.join("s3.sock")).env("TRIGGER", &trigger).stdout(Stdio::piped()).stderr(Stdio::piped());
        for (k, v) in env {
            cmd.env(k, v);
        }
        let child = cmd.spawn().expect("start the program");
        Server { child: Some(child), dir, sock, trigger }
    }

    pub fn pid(&self) -> u32 {
        self.child.as_ref().expect("running").id()
    }

    /// A connection to the socket, retrying while the program is still
    /// binding it.
    pub fn connect(&self) -> Conn {
        self.connect_to(&self.sock)
    }

    /// A connection to the program's second (`SOCK2`) socket.
    pub fn connect2(&self) -> Conn {
        self.connect_to(&self.dir.join("s2.sock"))
    }

    /// A connection to the program's third (`SOCK3`) socket.
    pub fn connect3(&self) -> Conn {
        self.connect_to(&self.dir.join("s3.sock"))
    }

    /// Whether a socket file is still on disk.
    pub fn has_socket(&self, which: u8) -> bool {
        match which {
            1 => self.sock.exists(),
            2 => self.dir.join("s2.sock").exists(),
            _ => self.dir.join("s3.sock").exists(),
        }
    }

    fn connect_to(&self, sock: &Path) -> Conn {
        let start = Instant::now();
        loop {
            match UnixStream::connect(sock) {
                Ok(s) => {
                    s.set_read_timeout(Some(Duration::from_secs(10))).expect("read timeout");
                    return Conn { s, buf: Vec::new(), sent: Vec::new() };
                }
                Err(e) => {
                    if start.elapsed() > Duration::from_secs(20) {
                        panic!("could not connect to {}: {e}", sock.display());
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        }
    }

    /// Ask the program to stop (its `stop()` runs), without waiting.
    pub fn trigger(&self) {
        std::fs::write(&self.trigger, "stop").expect("write the trigger");
    }

    /// Whether the program has exited.
    pub fn exited(&mut self) -> bool {
        match self.child.as_mut() {
            Some(c) => matches!(c.try_wait(), Ok(Some(_))),
            None => true,
        }
    }

    /// Stop the program, wait for it to end, and hand back what it printed.
    pub fn finish(mut self) -> Finished {
        self.trigger();
        self.wait(Duration::from_secs(30))
    }

    /// Wait for the program to end on its own.
    pub fn wait(&mut self, limit: Duration) -> Finished {
        let mut child = self.child.take().expect("running");
        let start = Instant::now();
        loop {
            if let Some(status) = child.try_wait().expect("wait") {
                let mut stdout = String::new();
                let mut stderr = String::new();
                if let Some(mut o) = child.stdout.take() {
                    let _ = o.read_to_string(&mut stdout);
                }
                if let Some(mut e) = child.stderr.take() {
                    let _ = e.read_to_string(&mut stderr);
                }
                return Finished { status, stdout, stderr };
            }
            if start.elapsed() > limit {
                let _ = child.kill();
                let _ = child.wait();
                panic!("the program did not exit within {limit:?} of being asked to stop");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// What a read of a connection found.
#[derive(Debug, PartialEq)]
pub enum Recv {
    Line(String),
    /// the server closed the connection
    Eof,
    /// nothing within the wait
    Timeout,
}

/// One client connection: lines out, lines in.
pub struct Conn {
    sent: Vec<String>,
    s: UnixStream,
    buf: Vec<u8>,
}

impl Conn {
    pub fn send(&mut self, line: &str) {
        self.sent.push(line.to_string());
        self.s.write_all(line.as_bytes()).expect("write");
        self.s.write_all(b"\n").expect("write");
    }

    /// Send bytes as they are (no newline added).
    pub fn send_raw(&mut self, bytes: &[u8]) {
        self.s.write_all(bytes).expect("write");
    }

    /// The next line, waiting at most `ms`.
    pub fn recv_within(&mut self, ms: u64) -> Recv {
        let end = Instant::now() + Duration::from_millis(ms);
        loop {
            if let Some(at) = self.buf.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = self.buf.drain(..=at).collect();
                return Recv::Line(String::from_utf8_lossy(&line[..line.len() - 1]).into_owned());
            }
            let now = Instant::now();
            if now >= end {
                return Recv::Timeout;
            }
            self.s.set_read_timeout(Some(end - now)).expect("read timeout");
            let mut chunk = [0u8; 65536];
            match self.s.read(&mut chunk) {
                Ok(0) => return Recv::Eof,
                Ok(n) => self.buf.extend_from_slice(&chunk[..n]),
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                    return Recv::Timeout;
                }
                Err(_) => return Recv::Eof,
            }
        }
    }

    /// The next line, which must arrive.
    pub fn line(&mut self) -> String {
        match self.recv_within(10_000) {
            Recv::Line(l) => l,
            other => panic!("expected a reply line, got {other:?}; sent so far on this connection: {:#?}", self.sent),
        }
    }

    /// Send a line and take the next one.
    pub fn ask(&mut self, line: &str) -> String {
        self.send(line);
        self.line()
    }

    /// Whether the server has closed this connection (within `ms`).
    pub fn closed_within(&mut self, ms: u64) -> bool {
        matches!(self.recv_within(ms), Recv::Eof)
    }

    /// Close our end.
    pub fn close(self) {
        let _ = self.s.shutdown(std::net::Shutdown::Both);
    }
}

/// `"request_id":<n>` made `"request_id":#`, so two replies compare apart
/// from the number the exposure assigned.
pub fn mask_request_id(line: &str) -> String {
    let key = "\"request_id\":";
    match line.find(key) {
        Some(at) => {
            let start = at + key.len();
            let digits = line[start..].chars().take_while(|c| c.is_ascii_digit() || *c == '-').count();
            format!("{}#{}", &line[..start], &line[start + digits..])
        }
        None => line.to_string(),
    }
}

/// The `request_id` of a reply line.
pub fn request_id_of(line: &str) -> i64 {
    let key = "\"request_id\":";
    let start = line.find(key).expect("a reply carries a request_id") + key.len();
    let digits: String = line[start..].chars().take_while(|c| c.is_ascii_digit() || *c == '-').collect();
    digits.parse().expect("a request_id is an integer")
}

/// Whether `line` carries the member `"kind":"<k>"`.
pub fn has_kind(line: &str, k: &str) -> bool {
    line.contains(&format!("\"kind\":\"{k}\""))
}

/// The user, group and process the kernel will say this test is.
pub fn me() -> (u32, u32, u32) {
    // SAFETY: getuid and getgid cannot fail and touch nothing.
    unsafe { (libc::getuid(), libc::getgid(), std::process::id()) }
}

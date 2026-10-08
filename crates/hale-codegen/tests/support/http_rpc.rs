//! A compiled program that serves over `http::Rpc`, and the HTTP clients
//! that drive its listeners (GH #1417, R3).
//!
//! A test builds a program (`build_source`), starts it with [`Server`] and
//! sends requests with [`request`]: one connection per request, as the
//! transport takes them (`Connection: close`). The program ends when the
//! test says so: it watches a trigger file and, when the file appears,
//! stops its handles and returns from `run()`. [`Server::finish`] writes the
//! file and waits for the exit.
//!
//! The recorded exchanges of `tests/api-contract/wire/http/` are read here
//! as text (there is no JSON library in this crate's tests): a recording is
//! a request (method, path, headers, body) and a reply (status, content
//! type, body), the bodies compact.

#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use super::harness;
use super::ports;

/// What the program printed and how it ended.
pub struct Finished {
    pub status: std::process::ExitStatus,
    pub stdout: String,
    pub stderr: String,
}

/// A running program: the ports it was told to listen on (`BIND`,
/// `BIND2`), the directory with its trigger file.
pub struct Server {
    child: Option<Child>,
    pub dir: PathBuf,
    pub port: u16,
    pub port2: u16,
    trigger: PathBuf,
}

impl Server {
    /// Run `bin` with `BIND` and `BIND2` (`127.0.0.1:<free port>`), `SOCK`
    /// (a socket path) and `TRIGGER` (the file whose appearance stops it)
    /// in its environment, plus `env`.
    pub fn start(bin: &Path, env: &[(&str, &str)]) -> Server {
        let dir = harness::unique_dir("httprpc");
        let trigger = dir.join("stop");
        let sock = dir.join("s.sock");
        assert!(sock.as_os_str().len() < 100, "the socket path is too long for sockaddr_un: {}", sock.display());
        let (port, port2) = (ports::free_port(), ports::free_port());
        let mut cmd = Command::new(bin);
        cmd.env("BIND", format!("127.0.0.1:{port}"))
            .env("BIND2", format!("127.0.0.1:{port2}"))
            .env("SOCK", &sock)
            .env("TRIGGER", &trigger)
            .env("TRIGGER_PUBLIC", dir.join("stop_public"))
            .env("TRIGGER_PARTNER", dir.join("stop_partner"))
            .env("TRIGGER_ADMIN", dir.join("stop_admin"))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (k, v) in env {
            cmd.env(k, v);
        }
        let child = cmd.spawn().expect("start the program");
        Server { child: Some(child), dir, port, port2, trigger }
    }

    pub fn pid(&self) -> u32 {
        self.child.as_ref().expect("running").id()
    }

    pub fn sock(&self) -> PathBuf {
        self.dir.join("s.sock")
    }

    /// Ask the program to stop (its `stop()` runs), without waiting.
    pub fn trigger(&self) {
        std::fs::write(&self.trigger, "stop").expect("write the trigger");
    }

    /// Ask the program to stop one exposure (`public`, `partner` or
    /// `admin`) and go on serving the others.
    pub fn stop_one(&self, name: &str) {
        std::fs::write(self.dir.join(format!("stop_{name}")), "stop").expect("write the trigger");
    }

    /// Whether the program has exited.
    pub fn exited(&mut self) -> bool {
        match self.child.as_mut() {
            Some(c) => matches!(c.try_wait(), Ok(Some(_))),
            None => true,
        }
    }

    /// Wait until the first listener accepts connections.
    pub fn ready(&self) {
        wait_listening(self.port);
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
                let mut out = String::new();
                if let Some(mut o) = child.stdout.take() {
                    let _ = o.read_to_string(&mut out);
                }
                let mut err = String::new();
                if let Some(mut e) = child.stderr.take() {
                    let _ = e.read_to_string(&mut err);
                }
                panic!("the program did not exit within {limit:?} of being asked to stop\nstdout:\n{out}\nstderr:\n{err}");
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

/// Wait until something accepts on `port` (the connection is dropped).
pub fn wait_listening(port: u16) {
    let start = Instant::now();
    loop {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return;
        }
        if start.elapsed() > Duration::from_secs(20) {
            panic!("nothing listens on 127.0.0.1:{port}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Wait until a Unix socket exists and accepts a connection: the listener
/// binds its path at birth, on a loaded machine after the TCP listeners
/// answer, so a client that connects at once may find no path yet. Bounded;
/// the probe is a connection that sends nothing and closes.
pub fn wait_accepting_unix(path: &Path) {
    let start = Instant::now();
    loop {
        if std::os::unix::net::UnixStream::connect(path).is_ok() {
            return;
        }
        if start.elapsed() > Duration::from_secs(20) {
            panic!("nothing accepts on {}", path.display());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// One line out and one line back over a Unix socket: the unix transport's
/// wire, for the exposures a program serves over both.
pub fn unix_ask(sock: &Path, line: &str) -> String {
    use std::os::unix::net::UnixStream;
    let start = Instant::now();
    let mut s = loop {
        match UnixStream::connect(sock) {
            Ok(s) => break s,
            Err(e) => {
                if start.elapsed() > Duration::from_secs(20) {
                    panic!("could not connect to {}: {e}", sock.display());
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    };
    s.set_read_timeout(Some(Duration::from_secs(15))).expect("read timeout");
    s.write_all(format!("{line}\n").as_bytes()).expect("write");
    let mut out = Vec::new();
    let mut b = [0u8; 1];
    loop {
        match s.read(&mut b) {
            Ok(1) if b[0] != b'\n' => out.push(b[0]),
            _ => break,
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Whether nothing accepts on `port`.
pub fn closed(port: u16) -> bool {
    TcpStream::connect(("127.0.0.1", port)).is_err()
}

/// One HTTP response.
#[derive(Debug, Clone)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

/// An HTTP request as text: `headers` are `Name: value` pairs; a body
/// carries its `Content-Length`.
pub fn request_text(method: &str, path: &str, headers: &[(String, String)], body: Option<&str>) -> String {
    let mut s = format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n");
    for (k, v) in headers {
        s.push_str(&format!("{k}: {v}\r\n"));
    }
    if let Some(b) = body {
        s.push_str(&format!("Content-Length: {}\r\n", b.len()));
    }
    s.push_str("Connection: close\r\n\r\n");
    if let Some(b) = body {
        s.push_str(b);
    }
    s
}

/// A client connection that has sent a request and not yet read its
/// answer.
pub struct Pending {
    s: TcpStream,
}

impl Pending {
    /// The answer, waiting at most `ms`; `None` if the server closed the
    /// connection without one.
    pub fn response_within(mut self, ms: u64) -> Option<Response> {
        self.s.set_read_timeout(Some(Duration::from_millis(ms))).expect("read timeout");
        let mut raw = Vec::new();
        let mut chunk = [0u8; 65536];
        loop {
            match self.s.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => raw.extend_from_slice(&chunk[..n]),
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                    panic!("no end of response within {ms} ms; read so far: {:?}", String::from_utf8_lossy(&raw));
                }
                Err(_) => break,
            }
        }
        parse_response(&raw)
    }

    /// Go away: close the connection without reading.
    pub fn close(self) {
        let _ = self.s.shutdown(std::net::Shutdown::Both);
    }
}

/// Connect to `port` (retrying while the program is still binding it) and
/// send `text`.
pub fn send(port: u16, text: &str) -> Pending {
    let start = Instant::now();
    let mut s = loop {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(s) => break s,
            Err(e) => {
                if start.elapsed() > Duration::from_secs(20) {
                    panic!("could not connect to 127.0.0.1:{port}: {e}");
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    };
    s.write_all(text.as_bytes()).expect("write the request");
    Pending { s }
}

/// One request, one answer.
pub fn request(port: u16, method: &str, path: &str, headers: &[(String, String)], body: Option<&str>) -> Response {
    send(port, &request_text(method, path, headers, body))
        .response_within(15_000)
        .unwrap_or_else(|| panic!("{method} {path}: the connection closed without a response"))
}

fn parse_response(raw: &[u8]) -> Option<Response> {
    if raw.is_empty() {
        return None;
    }
    let text = String::from_utf8_lossy(raw).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or_else(|| panic!("no header block in {text:?}"));
    let mut lines = head.split("\r\n");
    let status_line = lines.next().expect("a status line");
    let status = status_line.split(' ').nth(1).and_then(|c| c.parse().ok()).unwrap_or_else(|| panic!("a status in {status_line:?}"));
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect::<Vec<_>>();
    let len = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("content-length")).and_then(|(_, v)| v.parse::<usize>().ok());
    if let Some(len) = len {
        assert_eq!(body.len(), len, "Content-Length is the body's length: {text:?}");
    }
    Some(Response { status, headers, body: body.to_string() })
}

// ---- the recorded exchanges ----

/// `text` with the whitespace outside strings dropped.
pub fn compact(text: &str) -> String {
    let mut out = String::new();
    let mut in_str = false;
    let mut esc = false;
    for c in text.chars() {
        if in_str {
            out.push(c);
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
        } else if c == '"' {
            in_str = true;
            out.push(c);
        } else if !c.is_whitespace() {
            out.push(c);
        }
    }
    out
}

/// The value of the string member `key` in `text`.
fn string_member(text: &str, key: &str) -> Option<String> {
    let at = text.find(&format!("\"{key}\":"))?;
    let rest = text[at + key.len() + 3..].trim_start();
    let rest = rest.strip_prefix('"')?;
    Some(rest[..rest.find('"')?].to_string())
}

/// The compact object or number after `"key":` in `text`.
fn value_after(text: &str, key: &str) -> String {
    let at = text.find(&format!("\"{key}\":")).unwrap_or_else(|| panic!("no `{key}` in the recording"));
    let rest = text[at + key.len() + 3..].trim_start();
    if !rest.starts_with('{') {
        return rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    }
    let mut depth = 0;
    let mut in_str = false;
    let mut esc = false;
    let mut out = String::new();
    for c in rest.chars() {
        if in_str {
            out.push(c);
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_str = true;
                out.push(c);
            }
            c if c.is_whitespace() => {}
            '{' => {
                depth += 1;
                out.push(c);
            }
            '}' => {
                depth -= 1;
                out.push(c);
                if depth == 0 {
                    return out;
                }
            }
            c => out.push(c),
        }
    }
    panic!("an unterminated object in the recording");
}

/// One recorded exchange.
pub struct Recording {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
    pub status: u16,
    pub content_type: String,
    pub reply: String,
}

pub fn contract_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/api-contract")
}

pub fn recording(name: &str) -> Recording {
    let path = contract_dir().join("wire/http").join(format!("{name}.json"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let at = text.find("\"request\"").expect("a request");
    let rat = text.find("\"reply\"").expect("a reply");
    let (req, rep) = (&text[at..rat], &text[rat..]);
    let mut headers = Vec::new();
    for h in ["Authorization", "Content-Type", "Hale-Surface-Digest"] {
        if let Some(v) = string_member(req, h) {
            headers.push((h.to_string(), v));
        }
    }
    Recording {
        method: string_member(req, "method").expect("a method"),
        path: string_member(req, "path").expect("a path"),
        headers,
        body: value_after(req, "body"),
        status: value_after(rep, "status").parse().expect("a status"),
        content_type: string_member(rep, "Content-Type").expect("a content type"),
        reply: value_after(rep, "body"),
    }
}

impl Recording {
    /// Send the recorded request to `port`.
    pub fn send(&self, port: u16) -> Response {
        request(port, &self.method, &self.path, &self.headers, Some(&self.body))
    }

    /// Hold `got` to the recorded reply: the status, the content type and
    /// the body, byte for byte.
    pub fn assert_replied(&self, name: &str, got: &Response) {
        assert_eq!(got.status, self.status, "{name}: the status\n{}", got.body);
        assert_eq!(got.header("Content-Type"), Some(self.content_type.as_str()), "{name}: the content type");
        assert_eq!(got.body, self.reply, "{name}: the body differs from the recorded exchange");
    }
}

/// A description fixture, compact.
pub fn description_fixture(file: &str) -> String {
    let path = contract_dir().join(file);
    compact(&std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display())))
}

/// A header pair.
pub fn h(k: &str, v: &str) -> (String, String) {
    (k.to_string(), v.to_string())
}

/// `Authorization: Bearer <token>`.
pub fn bearer(token: &str) -> (String, String) {
    h("Authorization", &format!("Bearer {token}"))
}

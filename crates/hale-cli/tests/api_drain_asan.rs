//! A served call abandoned mid-handler at the drain is released, and the
//! receivers a server locus holds live as long as its exposure (GH #1565,
//! law 3), under AddressSanitizer and LeakSanitizer.
//!
//! The program is built through the CLI with `LOTUS_ASAN=1` on the `hale
//! build` child (the build knob a verb reads), as a user would build it.
//! A server locus holds its receiver, its bearer source and its exposure
//! over `http::Rpc`, and the main locus places it on an `async_io` pool.
//! Calls carry heap strings into the receiver and out. Then a call that
//! sleeps 400 ms is executing, parked in its handler, when SIGTERM lands:
//! the drain shuts the receiver's pool down under it and abandons the
//! parked coroutine. Its delivery owned a per-delivery region (the
//! payload's deserialized strings), which nothing reclaimed until the
//! abandon path released it: LeakSanitizer reported the region's struct
//! and its first chunk (65800 bytes) on every such drain.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[path = "../../hale-codegen/tests/support/child.rs"]
mod child;

const PROGRAM: &str = r#"
type Note { text: String; ms: Int; }
type Echoed { text: String; count: Int; }

api Notes { rpc Keeper::keep; }

locus Tokens {
    fn principal(token: String) -> std::api::Principal {
        if token == "t" { return std::api::Principal { mode: "bearer", name: "ops" }; }
        return std::api::Principal { mode: "bearer", name: "" };
    }
    fn refused() -> String { return "no such token"; }
}

locus Keeper {
    params { last: String = ""; count: Int = 0; }
    fn keep(n: Note) -> Echoed {
        let mut i = 0;
        while i < n.ms { std::time::sleep(1ms); i = i + 1; }
        self.last = n.text + "/" + to_string(self.count);
        self.count = self.count + 1;
        return Echoed { text: self.last + "!", count: self.count };
    }
}

locus NoteServer {
    params {
        tokens: Tokens = Tokens { };
        keeper: Keeper = Keeper { };
        api: api::Exposure = api::serve(Notes, http::Rpc { bind: std::env::var("BIND"), codec: json, principals: self.tokens }, as: "notes", receivers: { Keeper: self.keeper }, bound: 8, on_full: refuse);
    }
}

main locus App {
    params { notes: NoteServer = NoteServer { }; }
    placement { notes: cooperative(pool = io) where async_io; }
}

fn main() { App { }; }
"#;

fn unique_dir(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!("hale_api_drain_asan_{}_{}_{tag}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

/// One HTTP call, `Connection: close`: the response's status line and body.
fn call(port: u16, text: &str, ms: u64) -> String {
    let body = format!("{{\"text\":\"{text}\",\"ms\":{ms}}}");
    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    s.set_read_timeout(Some(Duration::from_secs(20))).ok();
    let req = format!(
        "POST /call/Keeper::keep HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer t\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    s.write_all(req.as_bytes()).expect("write");
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    out
}

#[test]
fn a_call_abandoned_mid_handler_at_the_drain_leaks_nothing_under_asan() {
    let dir = unique_dir("build");
    let src = dir.join("main.hl");
    std::fs::write(&src, PROGRAM).expect("write the program");
    let bin = dir.join("server");
    let built = Command::new(env!("CARGO_BIN_EXE_hale"))
        .env("HALE_SKIP_STALE_CHECK", "1")
        .env("LOTUS_ASAN", "1")
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(&bin)
        .output()
        .expect("hale build");
    assert!(built.status.success(), "the ASan build: {}", String::from_utf8_lossy(&built.stderr));
    let image = std::fs::read(&bin).expect("the binary");
    assert!(image.windows(11).any(|w| w == b"__asan_init"), "the build carries AddressSanitizer");

    let mut server = Command::new(&bin)
        .env("BIND", "127.0.0.1:0")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start the server");
    let port = child::tcp_port_on(&mut server, Ipv4Addr::LOCALHOST);
    for i in 0..5 {
        let got = call(port, &format!("note-{i}"), 0);
        assert!(got.starts_with("HTTP/1.1 200"), "call {i}: {got}");
        assert!(got.contains(&format!("\"text\":\"note-{i}/{i}!\"")), "call {i}: {got}");
    }
    // a call parked in its handler when the drain begins
    let slow = std::thread::spawn(move || call(port, "slow", 400));
    std::thread::sleep(Duration::from_millis(150));
    let sent = Command::new("kill").args(["-TERM", &server.id().to_string()]).status().expect("kill");
    assert!(sent.success(), "sent SIGTERM");
    let start = Instant::now();
    let status = loop {
        if let Some(s) = server.try_wait().expect("wait") {
            break s;
        }
        if start.elapsed() > Duration::from_secs(60) {
            let _ = server.kill();
            panic!("the server did not end after SIGTERM");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut stderr = String::new();
    if let Some(mut e) = server.stderr.take() {
        let _ = e.read_to_string(&mut stderr);
    }
    let _ = slow.join();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(!stderr.contains("LeakSanitizer") && !stderr.contains("ERROR: AddressSanitizer"), "the sanitizers found something:\n{stderr}");
    assert!(status.success(), "the drain ends the program cleanly: {status:?}\n{stderr}");
}

//! GH #1417 (R7): `std::io::h2`, the HTTP/2 server on a parked socket.
//!
//! The program is a listener and a transport written in the test (a locus
//! that answers by the bus subjects `__api.h2.event` and `__api.h2.cmd`,
//! the way any transport the stdlib lacks is written), and the client is
//! the hand-written one of `support/h2_client.rs`. The tests assert what a
//! client can observe: the handshake and a SETTINGS/PING exchange the
//! library answers on its own, a request opened and answered with headers,
//! data and trailers, many streams on one connection, a body that crosses
//! the flow-control window in both directions, a stream reset, and the
//! failures that end one connection and no other (a protocol error, a
//! GOAWAY, a vanished client); and that `stop` says GOAWAY and releases the
//! listener.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

#[path = "support/build.rs"]
mod build_opts;
#[path = "support/h2_client.rs"]
mod h2_client;
#[path = "support/harness.rs"]
mod harness;
#[path = "support/http_rpc.rs"]
mod http_rpc;
#[path = "support/ports.rs"]
mod ports;

use h2_client::*;
use http_rpc::*;

/// `$BIND` is the listener, `$TRIGGER` stops it. The transport answers
/// `/hello` at once, `/upload` with the count of body bytes it was sent
/// when the stream ends, `/big` with 300000 bytes, and says what it saw.
const ECHO: &str = r#"
@unbounded
fn written(path: String) -> Bool {
    return (std::io::fs::read_file(path) or "") != "";
}

@unbounded
fn filler(n: Int) -> String {
    let mut s = "x";
    while len(s) < n { s = s + s; }
    return s[0..n];
}

locus Echo {
    params {
        seen: Int = 0;
        path: String = "";
    }
    bus {
        subscribe "__api.h2.event" as on_event of type std::io::h2::Event;
        subscribe "__api.h2.cmd" as on_cmd of type std::io::h2::Cmd;
        publish "__api.h2.cmd" of type std::io::h2::Cmd;
    }
    fn say(e: std::io::h2::Event, headers: String, body: Bytes, trailers: String) {
        "__api.h2.cmd" <- std::io::h2::Cmd { key: e.key * 4294967296 + e.conn, kind: 0, stream: e.stream, code: 0, headers: std::bytes::from_string(headers), body: body, trailers: std::bytes::from_string(trailers) };
    }
    fn on_cmd(c: std::io::h2::Cmd) {
        if c.kind == 4 && c.key == 1 {
            "__api.h2.cmd" <- std::io::h2::Cmd { key: 1, kind: 3, stream: 0, code: 0, headers: std::bytes::from_string(""), body: std::bytes::from_string(""), trailers: std::bytes::from_string("") };
        }
    }
    fn stop() {
        "__api.h2.cmd" <- std::io::h2::Cmd { key: 1, kind: 5, stream: 0, code: 0, headers: std::bytes::from_string(""), body: std::bytes::from_string(""), trailers: std::bytes::from_string("") };
    }
    fn on_event(e: std::io::h2::Event) {
        if e.kind == 1 {
            let text = std::str::from_bytes(e.bytes);
            let at = std::str::index_of(text, ":path: ");
            let rest = text[(at + 7)..len(text)];
            self.path = rest[0..std::str::index_of(rest, "\n")];
            self.seen = 0;
            println("open conn=" + to_string(e.conn) + " stream=" + to_string(e.stream) + " path=" + self.path);
            if self.path == "/hello" {
                self.say(e, ":status: 200\ncontent-type: text/plain\n", std::bytes::from_string("hello"), "x-done: yes\n");
            }
            if self.path == "/big" {
                self.say(e, ":status: 200\n", std::bytes::from_string(filler(300000)), "x-done: big\n");
            }
        }
        if e.kind == 2 { self.seen = self.seen + len(e.bytes); }
        if e.kind == 3 {
            println("end conn=" + to_string(e.conn) + " stream=" + to_string(e.stream) + " bytes=" + to_string(self.seen));
            if self.path == "/upload" {
                self.say(e, ":status: 200\n", std::bytes::from_string(to_string(self.seen)), "");
            }
        }
        if e.kind == 4 { println("reset conn=" + to_string(e.conn) + " stream=" + to_string(e.stream) + " code=" + to_string(e.code)); }
        if e.kind == 5 { println("goaway conn=" + to_string(e.conn) + " code=" + to_string(e.code)); }
        if e.kind == 6 { println("closed conn=" + to_string(e.conn) + " stream=" + to_string(e.stream)); }
        if e.kind == 7 { println("ended conn=" + to_string(e.conn)); }
    }
}

main locus App {
    params {
        echo: Echo = Echo { };
        server: __StdIoH2Listener = __StdIoH2Listener { tid: 1, bind: std::env::var("BIND"), what: "the h2 test" };
    }
    placement { server: cooperative(pool = h2) where async_io; }
    run() {
        let all = std::env::var("TRIGGER");
        while !self.draining && !written(all) { std::time::sleep(10ms); }
        self.echo.stop();
        std::time::sleep(300ms);
        println("stopped");
    }
}

fn main() { App { }; }
"#;

fn echo() -> PathBuf {
    static BUILT: OnceLock<PathBuf> = OnceLock::new();
    BUILT
        .get_or_init(|| {
            let bin = harness::unique_bin("io_h2_server");
            build_opts::build_source(ECHO, &bin, &build_opts::options()).expect("build the h2 echo program");
            bin
        })
        .clone()
}

fn start() -> Server {
    let server = Server::start(&echo(), &[]);
    server.ready();
    server
}

const WAIT: Duration = Duration::from_secs(10);

fn get(c: &mut Client, stream: u32, path: &str) {
    c.headers(stream, &[(":method", "GET"), (":scheme", "http"), (":path", path), (":authority", "t")], true);
}

#[test]
fn a_request_is_opened_and_answered_with_headers_data_and_trailers() {
    let server = start();
    let mut c = Client::connect(server.port);
    get(&mut c, 1, "/hello");
    let r = c.response(1, WAIT);
    assert_eq!(r.status(), 200, "{r:?}");
    assert_eq!(r.header("content-type"), Some("text/plain"));
    assert_eq!(r.data, b"hello");
    assert_eq!(r.header("x-done"), Some("yes"), "the trailers arrive after the data: {r:?}");
    assert!(r.trailers.iter().any(|(n, _)| n == "x-done") && !r.headers.iter().any(|(n, _)| n == "x-done"));
    assert!(r.ended && r.reset.is_none());
    let done = server.finish();
    assert!(done.status.success(), "{:?}\n{}\n{}", done.status, done.stdout, done.stderr);
    assert!(done.stdout.contains(" stream=1 path=/hello"), "{}", done.stdout);
    assert!(done.stdout.contains(" stream=1 bytes=0"), "{}", done.stdout);
    assert!(done.stdout.contains("closed conn="), "{}", done.stdout);
}

#[test]
fn settings_ping_and_streams_are_the_librarys_own() {
    let server = start();
    let mut c = Client::connect(server.port);
    // the server's SETTINGS named its limits; the library acknowledged ours
    assert!(c.settings.iter().any(|(id, v)| *id == 3 && *v == 128), "MAX_CONCURRENT_STREAMS: {:?}", c.settings);
    c.ping(*b"hale-r7!");
    c.idle(Duration::from_millis(300));
    assert_eq!(c.pongs, 1, "a PING is acknowledged without the transport");
    // many streams on one connection, all answered
    for s in [1u32, 3, 5, 7] {
        get(&mut c, s, "/hello");
    }
    for s in [1u32, 3, 5, 7] {
        let r = c.response(s, WAIT);
        assert_eq!((r.status(), r.data.as_slice()), (200, &b"hello"[..]), "stream {s}: {r:?}");
    }
    let done = server.finish();
    assert!(done.status.success(), "{:?}\n{}\n{}", done.status, done.stdout, done.stderr);
}

#[test]
fn a_body_crosses_the_flow_control_windows_both_ways() {
    let server = start();
    let mut c = Client::connect(server.port);
    // 600000 bytes up: far past the default 65535 window, so the server's
    // WINDOW_UPDATEs are what let the rest in
    c.headers(1, &[(":method", "POST"), (":scheme", "http"), (":path", "/upload"), (":authority", "t")], false);
    c.data(1, &vec![7u8; 600_000], true);
    let up = c.response(1, WAIT);
    assert_eq!((up.status(), up.data.as_slice()), (200, &b"600000"[..]), "{up:?}");
    // 300000 bytes down, to a client whose window is 16 MiB
    get(&mut c, 3, "/big");
    let down = c.response(3, WAIT);
    assert_eq!(down.status(), 200);
    assert_eq!(down.data.len(), 300_000);
    assert!(down.data.iter().all(|b| *b == b'x'));
    assert_eq!(down.header("x-done"), Some("big"));
    assert!(server.finish().status.success());
}

#[test]
fn a_reset_stream_is_raised_and_the_connection_goes_on() {
    let server = start();
    let mut c = Client::connect(server.port);
    c.headers(1, &[(":method", "POST"), (":scheme", "http"), (":path", "/upload"), (":authority", "t")], false);
    c.data(1, b"abc", false);
    c.reset(1, 8);
    c.idle(Duration::from_millis(300));
    get(&mut c, 3, "/hello");
    let r = c.response(3, WAIT);
    assert_eq!(r.status(), 200, "{r:?}");
    let done = server.finish();
    assert!(done.stdout.contains(" stream=1 code=8"), "{}", done.stdout);
    assert!(done.stdout.contains("closed conn="), "{}", done.stdout);
}

#[test]
fn a_connection_that_fails_fails_alone() {
    let server = start();
    let mut a = Client::connect(server.port);
    let mut b = Client::connect(server.port);
    // a: a frame the protocol forbids on stream 0 (DATA), answered with a GOAWAY and the end of a
    a.frame(DATA, 0, 0, b"nope");
    let go = a.goaway_within(WAIT).expect("a protocol error is answered with GOAWAY");
    assert_eq!(go.1, 1, "PROTOCOL_ERROR");
    assert!(a.closed(WAIT), "the failed connection ends");
    // b: untouched
    get(&mut b, 1, "/hello");
    assert_eq!(b.response(1, WAIT).status(), 200);
    // c: the peer's own GOAWAY ends that connection, and only it
    let mut c = Client::connect(server.port);
    c.frame(GOAWAY, 0, 0, &[0, 0, 0, 0, 0, 0, 0, 0]);
    assert!(c.closed(WAIT), "the connection ends once the peer says it is going away");
    get(&mut b, 3, "/hello");
    assert_eq!(b.response(3, WAIT).status(), 200);
    // d: a client that vanishes mid-request
    {
        let mut d = Client::connect(server.port);
        d.headers(1, &[(":method", "POST"), (":scheme", "http"), (":path", "/upload"), (":authority", "t")], false);
        d.data(1, b"half", false);
    }
    std::thread::sleep(Duration::from_millis(200));
    let mut e = Client::connect(server.port);
    get(&mut e, 1, "/hello");
    assert_eq!(e.response(1, WAIT).status(), 200);
    let done = server.finish();
    assert!(done.status.success(), "{:?}\n{}\n{}", done.status, done.stdout, done.stderr);
    assert!(done.stdout.contains("goaway conn=") && done.stdout.contains(" code=0"), "{}", done.stdout);
    assert!(done.stdout.matches("ended conn=").count() >= 3, "every ended connection was raised:\n{}", done.stdout);
}

#[test]
fn stop_says_goaway_and_closes_the_listener() {
    let server = start();
    let mut c = Client::connect(server.port);
    get(&mut c, 1, "/hello");
    assert_eq!(c.response(1, WAIT).status(), 200);
    server.trigger();
    let go = c.goaway_within(WAIT).expect("stop says GOAWAY");
    assert_eq!(go.1, 0, "NO_ERROR");
    assert!(c.closed(WAIT), "the connection ends with nothing in flight");
    let start = std::time::Instant::now();
    while !closed(server.port) {
        assert!(start.elapsed() < WAIT, "the listener closed");
        std::thread::sleep(Duration::from_millis(10));
    }
    let done = server.finish();
    assert!(done.status.success(), "{:?}\n{}\n{}", done.status, done.stdout, done.stderr);
}

fn rss_kb(pid: u32) -> u64 {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).expect("status");
    status.lines().find_map(|l| l.strip_prefix("VmRSS:")).and_then(|v| v.trim().trim_end_matches("kB").trim().parse().ok()).expect("VmRSS")
}

#[test]
fn a_connection_carries_many_calls_in_bounded_memory() {
    // The connection reads into one buffer for its whole life: a read that
    // allocated would cost the process a page a call, and in the end its
    // capped payload arena. (The program prints three lines a call, so the
    // count stays under what its stdout pipe holds.)
    let server = start();
    let mut c = Client::connect(server.port);
    let mut stream = 1u32;
    let mut run = |c: &mut Client, n: u32| {
        for _ in 0..n {
            get(c, stream, "/hello");
            let r = c.response(stream, WAIT);
            assert_eq!(r.status(), 200, "stream {stream}: {r:?} eof={}", c.eof);
            stream += 2;
        }
    };
    run(&mut c, 100);
    let a = rss_kb(server.pid());
    run(&mut c, 400);
    let b = rss_kb(server.pid());
    let per_call = b.saturating_sub(a) * 1024 / 400;
    assert!(per_call < 1024, "a call costs the process {per_call} bytes ({a} kB -> {b} kB over 400 calls)");
    assert!(server.finish().status.success());
}

#[test]
fn the_h2_server_runs_clean_under_asan() {
    let bin = harness::unique_bin("io_h2_server_asan");
    harness::build_source_asan(ECHO, &bin);
    let server = Server::start(&bin, &[("LOTUS_NO_CHUNK_POOL", "1")]);
    server.ready();
    let mut c = Client::connect(server.port);
    get(&mut c, 1, "/hello");
    assert_eq!(c.response(1, WAIT).status(), 200);
    c.headers(3, &[(":method", "POST"), (":scheme", "http"), (":path", "/upload"), (":authority", "t")], false);
    c.data(3, &vec![1u8; 200_000], true);
    assert_eq!(c.response(3, WAIT).data, b"200000");
    get(&mut c, 5, "/big");
    assert_eq!(c.response(5, WAIT).data.len(), 300_000);
    // a stream reset mid-request, a failed connection, a vanished client
    c.headers(7, &[(":method", "POST"), (":scheme", "http"), (":path", "/upload"), (":authority", "t")], false);
    c.data(7, b"abc", false);
    c.reset(7, 8);
    let mut bad = Client::connect(server.port);
    bad.frame(DATA, 0, 0, b"nope");
    assert!(bad.closed(WAIT));
    {
        let mut gone = Client::connect(server.port);
        gone.headers(1, &[(":method", "POST"), (":scheme", "http"), (":path", "/upload"), (":authority", "t")], false);
        gone.data(1, b"half", false);
    }
    c.idle(Duration::from_millis(300));
    let done = server.finish();
    for bad in ["AddressSanitizer", "LeakSanitizer", "heap-use-after-free", "SUMMARY:"] {
        assert!(!done.stderr.contains(bad), "{bad} in:\n{}", done.stderr);
    }
    assert!(done.status.success(), "{:?}\n{}\n{}", done.status, done.stdout, done.stderr);
}

//! UDP ingress (downstream handoff, 2026-11-06 and 2026-11-07): an empty
//! datagram is an ordinary frame, not the reader's stop; a frame shorter
//! than its type, or one whose announced length runs past its end, is
//! dropped and counted, never decoded from what the reader's buffer held
//! before it; and the reader's stop at the exit quiesce is an explicit
//! wake on every platform.
//!
//! The reader took recvfrom's n == 0 as its stop sentinel (what a
//! shutdown(SHUT_RDWR) of the socket produced on Linux), so a genuine
//! zero-byte datagram ended ingress for good: every later datagram went
//! unread, silently. And Darwin does not honour shutdown() on an
//! unconnected UDP socket, so an idle reader there never woke at the
//! quiesce ("ingress quiesce: reader(s) still draining after 500 ms") and
//! the process hung on shutdown. The reader now polls the socket and a
//! stop pipe, on both platforms; the quiesce and the teardown write the
//! pipe.
//!
//! Once the empty datagram was a frame, the generated deserializer
//! decoded it: it copied each fixed field with no check against the
//! frame's length, so an empty datagram was delivered as `n=0`, an empty
//! one after a valid frame replayed that frame from the stale buffer, and
//! a one-byte one delivered the stale frame with its first byte replaced.
//! A String's announced length was checked against the whole frame, not
//! the bytes after it. Every read now checks what remains, a frame
//! shorter than its type's minimum is refused before any read, and the
//! reader counts the refusal (`dropped_decode` in the counters dump).
//! Held here on Linux: the review's sequence (empty, a valid frame, empty,
//! one byte) delivers exactly the valid frame; a String payload whose
//! length lies is dropped; then SIGTERM ends the program promptly.

use std::io::{BufRead, BufReader, Read};
use std::net::UdpSocket;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;
#[path = "support/child.rs"]
mod child;

const SUBSCRIBER: &str = r#"
    type Ping { n: Int; }
    locus Sub {
        bus { subscribe "evt" as on_evt of type Ping; }
        fn on_evt(p: Ping) { println("got n=" + to_string(p.n)); }
    }
    main locus App {
        params { s: Sub = Sub { }; }
        run() { while !self.draining { std::time::sleep(20ms); } println("main run done"); }
    }
    fn main() { App { }; }
"#;

const PUBLISHER: &str = r#"
    type Ping { n: Int; }
    locus Pub {
        bus { publish "evt" of type Ping; }
        birth() { "evt" <- Ping { n: 4242 }; }
    }
    fn main() { Pub { }; }
"#;

const STRING_SUBSCRIBER: &str = r#"
    type Msg { tag: Int; name: String; }
    locus Sub {
        bus { subscribe "evt" as on_evt of type Msg; }
        fn on_evt(m: Msg) { println("got tag=" + to_string(m.tag) + " name=" + m.name); }
    }
    main locus App {
        params { s: Sub = Sub { }; }
        run() { while !self.draining { std::time::sleep(20ms); } println("main run done"); }
    }
    fn main() { App { }; }
"#;

const BYTES_SUBSCRIBER: &str = r#"
    type Msg { tag: Int; payload: Bytes; tail: Int; }
    locus Sub {
        bus { subscribe "evt" as on_evt of type Msg; }
        fn on_evt(m: Msg) { println("got tag=" + to_string(m.tag) + " size=" + to_string(len(m.payload)) + " tail=" + to_string(m.tail)); }
    }
    main locus App {
        params { s: Sub = Sub { }; }
        run() { while !self.draining { std::time::sleep(20ms); } println("main run done"); }
    }
    fn main() { App { }; }
"#;

struct Stopped(std::process::Child);
impl Drop for Stopped {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn wait_for_line(lines: &Arc<Mutex<Vec<String>>>, want: &str, limit: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < limit {
        if lines.lock().unwrap().iter().any(|l| l.trim() == want) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

/// A listening subscriber on a reserved address, its stdout collected.
struct Listener {
    sub: Stopped,
    lines: Arc<Mutex<Vec<String>>>,
    reader: Option<std::thread::JoinHandle<()>>,
    addr: child::Reserved,
    files: Vec<std::path::PathBuf>,
}

impl Listener {
    fn start(name: &str, source: &str) -> Listener {
        let sub_bin = harness::unique_bin(&format!("udp_ingress_{name}"));
        build_opts::build_source(source, &sub_bin, &build_opts::options()).expect("build subscriber");
        // an address the subscriber is told before it binds: held, not drawn
        let addr = child::Reserved::new();
        let sub_cfg = harness::unique_bin(&format!("udp_ingress_{name}.conf"));
        std::fs::write(&sub_cfg, format!("evt = udp://{}:{}:listen\n", addr.host(), addr.port())).expect("sub cfg");
        let mut sub = Command::new(&sub_bin)
            .env("LOTUS_BUS_CONFIG", &sub_cfg)
            .env("LOTUS_BUS_COUNTERS_DUMP", "1")
            .env("LOTUS_DRAIN_GRACE_MS", "5000")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn subscriber");
        let stdout = sub.stdout.take().unwrap();
        let lines: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&lines);
        let reader = std::thread::spawn(move || {
            for l in BufReader::new(stdout).lines().map_while(Result::ok) {
                sink.lock().unwrap().push(l);
            }
        });
        let mut sub = Stopped(sub);
        // the subscriber's reader has bound the reserved address
        let bound = child::udp_port_on(&mut sub.0, *addr.addr.ip());
        assert_eq!(bound, addr.port());
        Listener { sub, lines, reader: Some(reader), addr, files: vec![sub_bin, sub_cfg] }
    }

    fn send(&self, probe: &UdpSocket, frame: &[u8]) {
        probe.send_to(frame, self.addr.addr).expect("send the datagram");
        std::thread::sleep(Duration::from_millis(50));
    }

    /// SIGTERM, then the program's exit status, how long it took, its stderr
    /// and its stdout.
    fn stop(mut self) -> (std::process::ExitStatus, Duration, String, String) {
        let sent = Instant::now();
        Command::new("kill").args(["-TERM", &self.sub.0.id().to_string()]).status().expect("kill");
        let deadline = sent + Duration::from_secs(5);
        let status = loop {
            if let Some(st) = self.sub.0.try_wait().expect("try_wait") {
                break st;
            }
            assert!(Instant::now() < deadline, "the program did not end within 5 s of SIGTERM: the reader did not stop");
            std::thread::sleep(Duration::from_millis(10));
        };
        let took = sent.elapsed();
        let mut stderr = String::new();
        if let Some(mut e) = self.sub.0.stderr.take() {
            let _ = e.read_to_string(&mut stderr);
        }
        if let Some(r) = self.reader.take() {
            let _ = r.join();
        }
        for f in &self.files {
            let _ = std::fs::remove_file(f);
        }
        let out = self.lines.lock().unwrap().join("\n");
        (status, took, stderr, out)
    }
}

/// The `got ...` lines of a subscriber's stdout, in order.
fn got(out: &str) -> Vec<&str> {
    out.lines().filter(|l| l.starts_with("got ")).collect()
}

/// A counter of the binding's `[bus counters]` line.
fn counter(stderr: &str, name: &str) -> u64 {
    let line = stderr
        .lines()
        .find(|l| l.contains("[bus counters]") && l.contains("subject=evt"))
        .unwrap_or_else(|| panic!("no counters line for evt in:\n{stderr}"));
    line.split_whitespace()
        .find_map(|kv| kv.strip_prefix(&format!("{name}=")))
        .unwrap_or_else(|| panic!("no `{name}=` in: {line}"))
        .parse()
        .expect("a count")
}

#[test]
fn an_empty_or_short_datagram_is_dropped_and_counted_and_sigterm_ends_the_program() {
    let pub_bin = harness::unique_bin("udp_ingress_pub");
    build_opts::build_source(PUBLISHER, &pub_bin, &build_opts::options()).expect("build publisher");
    let l = Listener::start("sub", SUBSCRIBER);
    let pub_cfg = harness::unique_bin("udp_ingress_pub.conf");
    std::fs::write(&pub_cfg, format!("evt = udp://{}:{}:connect\n", l.addr.host(), l.addr.port())).expect("pub cfg");

    // a zero-byte datagram: an ordinary frame, too short for `Ping`
    let probe = UdpSocket::bind("127.0.0.1:0").expect("probe socket");
    l.send(&probe, &[]);

    // then a valid frame, from a publisher of the same type
    let pub_out = Command::new(&pub_bin)
        .env("LOTUS_BUS_CONFIG", &pub_cfg)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run publisher");
    assert!(pub_out.status.success(), "publisher: {:?}\n{}", pub_out.status, String::from_utf8_lossy(&pub_out.stderr));
    assert!(
        wait_for_line(&l.lines, "got n=4242", Duration::from_secs(10)),
        "the frame after the empty datagram was not delivered: the reader stopped on it\nstdout:\n{}",
        l.lines.lock().unwrap().join("\n")
    );

    // an empty datagram after a valid frame (it replayed the frame from
    // the stale buffer), then a one-byte one (it delivered the stale frame
    // with its first byte replaced: `n=4103`)
    l.send(&probe, &[]);
    l.send(&probe, &[0x07]);
    std::thread::sleep(Duration::from_millis(100));

    // SIGTERM: main's run() sees the drain, the quiesce stops the reader
    // (the stop pipe, not a shutdown() of the socket), the program ends
    let (status, took, stderr, out) = l.stop();
    for f in [&pub_bin, &pub_cfg] {
        let _ = std::fs::remove_file(f);
    }
    assert!(status.success(), "{status:?}\nstdout:\n{out}\nstderr:\n{stderr}");
    assert!(!stderr.contains("still draining"), "the quiesce waited out its budget:\n{stderr}");
    assert!(took < Duration::from_secs(2), "the program took {took:?} to end after SIGTERM:\n{stderr}");
    assert!(out.lines().any(|l| l == "main run done"), "stdout:\n{out}");
    // exactly the valid frame reached the handler: nothing invented from
    // the empty datagrams, nothing replayed from the stale buffer
    assert_eq!(got(&out), vec!["got n=4242"], "stdout:\n{out}\nstderr:\n{stderr}");
    assert_eq!(counter(&stderr, "delivered"), 1, "{stderr}");
    assert_eq!(counter(&stderr, "dropped_decode"), 3, "two empty datagrams and a one-byte one:\n{stderr}");
}

#[test]
fn a_string_payload_whose_announced_length_runs_past_the_frame_is_dropped() {
    let l = Listener::start("str", STRING_SUBSCRIBER);
    let probe = UdpSocket::bind("127.0.0.1:0").expect("probe socket");
    // the wire form of `Msg`: the tag's 8 bytes, the name's 8-byte length, its bytes
    let frame = |tag: i64, len: i64, bytes: &[u8]| {
        let mut f = Vec::new();
        f.extend_from_slice(&tag.to_le_bytes());
        f.extend_from_slice(&len.to_le_bytes());
        f.extend_from_slice(bytes);
        f
    };
    l.send(&probe, &frame(1, 2, b"hi"));
    assert!(wait_for_line(&l.lines, "got tag=1 name=hi", Duration::from_secs(10)), "stdout:\n{}", l.lines.lock().unwrap().join("\n"));
    // a length the frame holds from its start (16 <= 20) but not after the
    // prefix (8 + 8 + 16 > 20): the old check passed it and the handler
    // read past the datagram
    l.send(&probe, &frame(2, 16, b"abcd"));
    // a negative length: the sum wraps
    l.send(&probe, &frame(2, -1, b""));
    // shorter than the type's minimum (two prefixes, 16 bytes)
    l.send(&probe, &frame(3, 0, b"")[..15]);
    // a valid frame after them is delivered
    l.send(&probe, &frame(4, 2, b"ok"));
    assert!(wait_for_line(&l.lines, "got tag=4 name=ok", Duration::from_secs(10)), "stdout:\n{}", l.lines.lock().unwrap().join("\n"));
    let (status, _, stderr, out) = l.stop();
    assert!(status.success(), "{status:?}\nstdout:\n{out}\nstderr:\n{stderr}");
    assert_eq!(got(&out), vec!["got tag=1 name=hi", "got tag=4 name=ok"], "stdout:\n{out}\nstderr:\n{stderr}");
    assert_eq!(counter(&stderr, "delivered"), 2, "{stderr}");
    assert_eq!(counter(&stderr, "dropped_decode"), 3, "a lying length, a negative one, a short frame:\n{stderr}");
}

#[test]
fn a_bytes_length_that_wraps_its_header_sum_is_dropped() {
    // reviewer, 2026-11-07: the Bytes read summed `len + 8` before the
    // frame check, so a length of -8 made the sum zero: cursor + 0 fit,
    // a zero-byte Bytes object was allocated, and the tail was read from
    // the prefix (-8). The raw length is now held to the bytes after its
    // prefix before any sum.
    let l = Listener::start("bytes", BYTES_SUBSCRIBER);
    let probe = UdpSocket::bind("127.0.0.1:0").expect("probe socket");
    // the wire form of `Msg`: the tag's 8 bytes, the payload's 8-byte length, its bytes, the tail's 8
    let frame = |tag: i64, len: u64, bytes: &[u8], tail: i64| {
        let mut f = Vec::new();
        f.extend_from_slice(&tag.to_le_bytes());
        f.extend_from_slice(&len.to_le_bytes());
        f.extend_from_slice(bytes);
        f.extend_from_slice(&tail.to_le_bytes());
        f
    };
    l.send(&probe, &frame(1, 2, b"hi", 9));
    assert!(wait_for_line(&l.lines, "got tag=1 size=2 tail=9", Duration::from_secs(10)), "stdout:\n{}", l.lines.lock().unwrap().join("\n"));
    // the reviewer's frame: 24 bytes, tag 42, length 0xfffffffffffffff8, tail 73
    let f = frame(42, 0xffff_ffff_ffff_fff8, b"", 73);
    assert_eq!(f.len(), 24);
    l.send(&probe, &f);
    // a length of -1: the header's eight bytes would be short-copied
    l.send(&probe, &frame(43, 0xffff_ffff_ffff_ffff, b"", 74));
    // a valid frame after them is delivered
    l.send(&probe, &frame(4, 3, b"end", 5));
    assert!(wait_for_line(&l.lines, "got tag=4 size=3 tail=5", Duration::from_secs(10)), "stdout:\n{}", l.lines.lock().unwrap().join("\n"));
    let (status, _, stderr, out) = l.stop();
    assert!(status.success(), "{status:?}\nstdout:\n{out}\nstderr:\n{stderr}");
    assert_eq!(got(&out), vec!["got tag=1 size=2 tail=9", "got tag=4 size=3 tail=5"], "stdout:\n{out}\nstderr:\n{stderr}");
    assert_eq!(counter(&stderr, "delivered"), 2, "{stderr}");
    assert_eq!(counter(&stderr, "dropped_decode"), 2, "the wrapping length and the -1 length:\n{stderr}");
}

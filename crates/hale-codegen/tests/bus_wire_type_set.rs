//! Every `type` crosses the wire between processes (GH #1951 item 4).
//!
//! The bus wire format's carriage table
//! (`hale_types::wire_carriage::WIRE_TABLE`) has an encoding for every
//! value a payload record may hold: a `bounded` list of strings, bytes,
//! records or enums with payloads is an `i64` count and that many
//! elements, an array of strings or bytes its `N` elements, an enum with
//! payloads a `u32` tag and the variant's values, a tuple a record of its
//! elements, a `std::json::Value` its text. `tests/hale/
//! bus_wire_type_set_test.hl` holds the in-process round trip (the local
//! decoder); this file holds the transport readers' (the wire decoder):
//!
//!   - a publisher process and a subscriber process round-trip every new
//!     shape over `unix://` and over `udp://`;
//!   - hand-written frames against a subscriber built with AddressSanitizer:
//!     a list count past its capacity, a negative one, an enum tag past the
//!     last variant, a string inside a list whose length runs past the
//!     frame, and a valid frame cut short are each dropped and counted
//!     (`dropped_decode`), and the valid frames around them delivered.
//!
//! A `udp://` subscriber listens on a `child::Reserved` address (the
//! listener is told its address before it binds, and the reader takes no
//! port 0), confirmed bound before anything is sent; a `unix://` one on a
//! path of its own. No port is drawn.

use std::io::{BufRead, BufReader, Read};
use std::net::UdpSocket;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;
#[path = "support/child.rs"]
mod child;

/// The payload of the round trips, shared by both programs.
const TYPES: &str = r#"
    type Point { x: Int = 0; label: String = ""; }
    type Kind = enum { Small, Large };
    type Status = enum {
        Ok,
        Other(String),
        Pair(String, Int),
        Blob(Bytes),
        At(Point),
        Many(bounded[String; 3]),
        Both((Int, String)),
    };
    type Row { name: String = ""; status: Status = Status::Ok; }
    type Msg {
        seq: Int = 0;
        tags: bounded[String; 4];
        blobs: bounded[Bytes; 3];
        rows: bounded[Row; 3];
        states: bounded[Status; 6];
        docs: bounded[std::json::Value; 2];
        none: bounded[String; 2];
        names: [String; 3];
        chunks: [Bytes; 2];
        kinds: [Kind; 2];
        picks: [Status; 2];
        status: Status = Status::Ok;
        pair: (Int, String);
        deep: (String, Point);
        config: std::json::Value;
        tail: Int = 0;
    }

    fn bt(b: Bytes) -> String {
        let mut out = "";
        let mut i = 0;
        while i < len(b) {
            if i > 0 { out = out + "."; }
            out = out + to_string(std::bytes::read_u8(b, i) or -1);
            i = i + 1;
        }
        return out;
    }
    fn joined(xs: bounded[String; 3]) -> String {
        let mut out = "";
        for x in xs { out = out + x + ";"; }
        return out;
    }
    fn show(s: Status) -> String {
        return match s {
            Status::Ok -> "ok",
            Status::Other(t) -> "other:" + t,
            Status::Pair(t, n) -> "pair:" + t + "/" + to_string(n),
            Status::Blob(b) -> "blob:" + bt(b),
            Status::At(p) -> "at:" + p.label + "@" + to_string(p.x),
            Status::Many(xs) -> "many:" + joined(xs),
            Status::Both(t) -> "both:" + to_string(t.0) + "/" + t.1,
        };
    }
    fn kind(k: Kind) -> String { return match k { Kind::Small -> "s", Kind::Large -> "L" }; }
    fn render(m: Msg) -> String {
        let mut out = to_string(m.seq) + "|";
        for t in m.tags { out = out + t + ","; }
        out = out + "|";
        for b in m.blobs { out = out + bt(b) + ","; }
        out = out + "|";
        for r in m.rows { out = out + r.name + "=" + show(r.status) + ","; }
        out = out + "|";
        for s in m.states { out = out + show(s) + ","; }
        out = out + "|";
        for d in m.docs { out = out + d.text + ","; }
        out = out + "|" + to_string(count(m.none)) + "|";
        out = out + m.names[0] + "," + m.names[1] + "," + m.names[2] + "|";
        out = out + bt(m.chunks[0]) + "," + bt(m.chunks[1]) + "|";
        out = out + kind(m.kinds[0]) + kind(m.kinds[1]) + "|";
        out = out + show(m.picks[0]) + "," + show(m.picks[1]) + "|";
        out = out + show(m.status) + "|";
        out = out + to_string(m.pair.0) + "/" + m.pair.1 + "|";
        out = out + m.deep.0 + "/" + m.deep.1.label + "@" + to_string(m.deep.1.x) + "|";
        out = out + m.config.text + "|" + to_string(m.tail);
        return out;
    }
"#;

const SUB_BODY: &str = r#"
    locus Sub {
        bus { subscribe "evt" as on_evt of type Msg; }
        fn on_evt(m: Msg) { println("got " + render(m)); }
    }
    main locus App {
        params { s: Sub = Sub { }; }
        run() { while !self.draining { std::time::sleep(20ms); } println("main run done"); }
    }
    fn main() { App { }; }
"#;

const PUB_BODY: &str = r#"
    fn up(s: String) -> String { return std::str::upper(s); }
    fn build(k: Int) -> Msg {
        let n = to_string(k);
        let mut many: bounded[String; 3] = [];
        push(many, up("m") + n) or raise;
        let mut m = Msg {
            seq: k,
            names: [up("a") + n, "", up("c") + n],
            chunks: [std::bytes::from_string(up("x") + n), std::bytes::from_string("")],
            kinds: [Kind::Large, Kind::Small],
            picks: [Status::Other(up("p") + n), Status::Ok],
            status: Status::Pair(up("st") + n, k * 10),
            pair: (k + 1, up("pr") + n),
            deep: (up("d") + n, Point { x: k, label: up("pt") + n }),
            config: std::json::Value::parse("{\"k\": " + n + "}") or raise,
            tail: 7000 + k,
        };
        push(m.tags, up("t") + n) or raise;
        push(m.tags, up("u") + n) or raise;
        push(m.blobs, std::bytes::from_string(up("b") + n)) or raise;
        push(m.rows, Row { name: up("r") + n, status: Status::At(Point { x: k, label: up("l") + n }) }) or raise;
        push(m.states, Status::Other(up("o") + n)) or raise;
        push(m.states, Status::Ok) or raise;
        push(m.states, Status::Blob(std::bytes::from_string(up("z") + n))) or raise;
        push(m.states, Status::Many(many)) or raise;
        push(m.states, Status::Both((k, up("h") + n))) or raise;
        push(m.docs, std::json::Value::parse("[" + n + "]") or raise) or raise;
        return m;
    }
    locus Pub {
        bus { publish "evt" of type Msg; }
        birth() {
            "evt" <- build(1);
            "evt" <- build(2);
        }
    }
    fn main() { Pub { }; }
"#;

/// What the subscriber prints for `build(k)`.
fn expected(k: u32) -> String {
    format!(
        "got {k}|T{k},U{k},|66.{b},|R{k}=at:L{k}@{k},|other:O{k},ok,blob:90.{b},many:M{k};,both:{k}/H{k},|[{k}],|0|A{k},,C{k}|88.{b},|Ls|other:P{k},ok|pair:ST{k}/{t}|{p}/PR{k}|D{k}/PT{k}@{k}|{{\"k\": {k}}}|{tail}",
        b = 48 + k,
        t = k * 10,
        p = k + 1,
        tail = 7000 + k,
    )
}

struct Stopped(std::process::Child);
impl Drop for Stopped {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A subscriber running under a bus config, its stdout collected.
struct Running {
    sub: Stopped,
    lines: Arc<Mutex<Vec<String>>>,
    reader: Option<std::thread::JoinHandle<()>>,
    files: Vec<PathBuf>,
}

impl Running {
    fn start(bin: PathBuf, cfg_line: &str, tag: &str) -> Running {
        let cfg = harness::unique_bin(&format!("wire_set_{tag}.conf"));
        std::fs::write(&cfg, cfg_line).expect("sub cfg");
        let mut sub = Command::new(&bin)
            .env("LOTUS_BUS_CONFIG", &cfg)
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
        Running { sub: Stopped(sub), lines, reader: Some(reader), files: vec![bin, cfg] }
    }

    fn wait_for(&self, want: &str) -> bool {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(20) {
            if self.lines.lock().unwrap().iter().any(|l| l.trim() == want) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    fn out(&self) -> String {
        self.lines.lock().unwrap().join("\n")
    }

    /// SIGTERM, then the exit status, stderr and stdout.
    fn stop(mut self) -> (std::process::ExitStatus, String, String) {
        Command::new("kill").args(["-TERM", &self.sub.0.id().to_string()]).status().expect("kill");
        let deadline = Instant::now() + Duration::from_secs(10);
        let status = loop {
            if let Some(st) = self.sub.0.try_wait().expect("try_wait") {
                break st;
            }
            assert!(Instant::now() < deadline, "the subscriber did not end within 10 s of SIGTERM");
            std::thread::sleep(Duration::from_millis(10));
        };
        self.finish(status)
    }

    fn finish(mut self, status: std::process::ExitStatus) -> (std::process::ExitStatus, String, String) {
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
        (status, stderr, out)
    }
}

fn got(out: &str) -> Vec<&str> {
    out.lines().filter(|l| l.starts_with("got ")).collect()
}

fn counter(stderr: &str, name: &str) -> u64 {
    let line = stderr
        .lines()
        .find(|l| l.contains("[bus counters]") && l.contains("subject=evt "))
        .unwrap_or_else(|| panic!("no counters line for evt in:\n{stderr}"));
    line.split_whitespace()
        .find_map(|kv| kv.strip_prefix(&format!("{name}=")))
        .unwrap_or_else(|| panic!("no `{name}=` in: {line}"))
        .parse()
        .expect("a count")
}

fn build(tag: &str, src: &str, asan: bool) -> PathBuf {
    let bin = harness::unique_bin(&format!("wire_set_{tag}"));
    let opts = hale_codegen::BuildOptions { asan, ..build_opts::options() };
    build_opts::build_source(src, &bin, &opts).unwrap_or_else(|e| panic!("build {tag}: {e:?}"));
    bin
}

fn publish(cfg_line: &str, tag: &str) {
    let bin = build(&format!("{tag}_pub"), &format!("{TYPES}{PUB_BODY}"), false);
    let cfg = harness::unique_bin(&format!("wire_set_{tag}_pub.conf"));
    std::fs::write(&cfg, cfg_line).expect("pub cfg");
    let out = Command::new(&bin).env("LOTUS_BUS_CONFIG", &cfg).output().expect("run publisher");
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(&cfg);
    assert!(out.status.success(), "publisher: {:?}\n{}", out.status, String::from_utf8_lossy(&out.stderr));
}

/// `clean_exit`: the subscriber ends cleanly at SIGTERM. A `unix://`
/// listener that has served a peer does not on Darwin (its reader is not
/// woken at the quiesce, whatever the payload), so there the drain grace
/// ends it.
fn assert_round_trip(sub: Running, clean_exit: bool) {
    for k in [1, 2] {
        assert!(sub.wait_for(&expected(k)), "frame {k} did not arrive as sent\nwant: {}\nstdout:\n{}", expected(k), sub.out());
    }
    let (status, stderr, out) = sub.stop();
    if clean_exit {
        assert!(status.success(), "{status:?}\nstdout:\n{out}\nstderr:\n{stderr}");
    }
    assert_eq!(got(&out).len(), 2, "stdout:\n{out}\nstderr:\n{stderr}");
    assert_eq!(counter(&stderr, "dropped_decode"), 0, "{stderr}");
}

#[test]
fn every_new_shape_round_trips_between_processes_over_unix() {
    let sub_bin = build("unix_sub", &format!("{TYPES}{SUB_BODY}"), false);
    let sock = harness::unique_bin("wire_set_unix.sock");
    let sub = Running::start(sub_bin, &format!("evt = unix://{} : listen\n", sock.display()), "unix_sub");
    let start = Instant::now();
    while !sock.exists() {
        assert!(start.elapsed() < Duration::from_secs(10), "the subscriber never bound {}", sock.display());
        std::thread::sleep(Duration::from_millis(20));
    }
    publish(&format!("evt = unix://{} : connect\n", sock.display()), "unix");
    assert_round_trip(sub, cfg!(target_os = "linux"));
    let _ = std::fs::remove_file(&sock);
}

#[test]
fn every_new_shape_round_trips_between_processes_over_udp() {
    let sub_bin = build("udp_sub", &format!("{TYPES}{SUB_BODY}"), false);
    let addr = child::Reserved::new();
    let mut sub = Running::start(sub_bin, &format!("evt = udp://{}:{}:listen\n", addr.host(), addr.port()), "udp_sub");
    assert_eq!(child::udp_port_on(&mut sub.sub.0, *addr.addr.ip()), addr.port());
    publish(&format!("evt = udp://{}:{}:connect\n", addr.host(), addr.port()), "udp");
    assert_round_trip(sub, true);
}

/// A payload small enough to write frames for by hand.
const SMALL_SUB: &str = r#"
    type Status = enum { Ok, Other(String) };
    type Msg { tag: Int; names: bounded[String; 2]; status: Status; }
    locus Sub {
        bus { subscribe "evt" as on_evt of type Msg; }
        fn on_evt(m: Msg) {
            let mut out = "got tag=" + to_string(m.tag) + " names=";
            for n in m.names { out = out + n + ","; }
            out = out + " status=" + match m.status { Status::Ok -> "ok", Status::Other(s) -> "other:" + s };
            println(out);
        }
    }
    main locus App {
        params { s: Sub = Sub { }; }
        run() { while !self.draining { std::time::sleep(20ms); } println("main run done"); }
    }
    fn main() { App { }; }
"#;

/// `Msg`'s wire form: the tag's 8 bytes, the list's `i64` count and each
/// name's `i64` length and bytes, the status's `u32` tag and, for
/// `Other`, its string.
fn frame(tag: i64, count: i64, names: &[&[u8]], status: u32, other: Option<&[u8]>) -> Vec<u8> {
    let mut f = Vec::new();
    f.extend_from_slice(&tag.to_le_bytes());
    f.extend_from_slice(&count.to_le_bytes());
    for n in names {
        f.extend_from_slice(&(n.len() as i64).to_le_bytes());
        f.extend_from_slice(n);
    }
    f.extend_from_slice(&status.to_le_bytes());
    if let Some(s) = other {
        f.extend_from_slice(&(s.len() as i64).to_le_bytes());
        f.extend_from_slice(s);
    }
    f
}

#[test]
fn a_malformed_list_or_enum_frame_is_dropped_and_counted_under_asan() {
    // AddressSanitizer on every new arm of the wire decoder: a read past
    // the datagram, or a slot written past a list's capacity, is a report
    // and a non-zero exit
    let sub_bin = build("bad_sub", SMALL_SUB, true);
    let addr = child::Reserved::new();
    let mut sub = Running::start(sub_bin, &format!("evt = udp://{}:{}:listen\n", addr.host(), addr.port()), "bad_sub");
    assert_eq!(child::udp_port_on(&mut sub.sub.0, *addr.addr.ip()), addr.port());
    let probe = UdpSocket::bind("127.0.0.1:0").expect("probe socket");
    let send = |f: &[u8]| {
        probe.send_to(f, addr.addr).expect("send the datagram");
        std::thread::sleep(Duration::from_millis(50));
    };
    let good = frame(1, 2, &[b"a", b"bc"], 1, Some(b"x"));
    send(&good);
    assert!(sub.wait_for("got tag=1 names=a,bc, status=other:x"), "stdout:\n{}", sub.out());
    // a count past the list's capacity of 2, with three names behind it
    send(&frame(2, 3, &[b"a", b"b", b"c"], 0, None));
    // a negative count
    send(&frame(3, -1, &[], 0, None));
    // an enum tag past the last variant (`Other` is 1)
    send(&frame(4, 0, &[], 2, None));
    // a name inside the list whose length runs past the frame
    let mut lying = frame(5, 1, &[b"abc"], 0, None);
    lying[16..24].copy_from_slice(&1000i64.to_le_bytes());
    send(&lying);
    // the valid frame, cut short inside the status's string
    send(&good[..good.len() - 1]);
    // a valid frame after them, an empty list and a payload-free variant
    send(&frame(6, 0, &[], 0, None));
    assert!(sub.wait_for("got tag=6 names= status=ok"), "stdout:\n{}", sub.out());
    let (status, stderr, out) = sub.stop();
    assert!(status.success(), "{status:?}\nstdout:\n{out}\nstderr:\n{stderr}");
    assert!(!stderr.contains("AddressSanitizer"), "{stderr}");
    assert_eq!(
        got(&out),
        vec!["got tag=1 names=a,bc, status=other:x", "got tag=6 names= status=ok"],
        "stdout:\n{out}\nstderr:\n{stderr}"
    );
    assert_eq!(counter(&stderr, "delivered"), 2, "{stderr}");
    assert_eq!(
        counter(&stderr, "dropped_decode"),
        5,
        "a count past N, a negative count, a tag past the variants, a lying length, a short frame:\n{stderr}"
    );
}

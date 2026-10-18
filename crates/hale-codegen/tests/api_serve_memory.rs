//! A served program's memory stays flat under sustained calls: an echo
//! surface (`Echo::ping`, one record in and one out) served over each
//! transport, called N times, its resident set read from `/proc` before and
//! after. The unix transport's peer used to read each request inside its
//! `run()`, which lasts as long as the connection, so every request the
//! connection carried stayed in that method's scratch until it closed
//! (about 9 KB a call); the read is a method of its own now. The bench that
//! found it is `scripts/api-bench/`.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;
#[path = "support/child.rs"]
mod child;

const TYPES: &str = "\
type Ping { n: Int; tag: String; }
type Pong { n: Int; tag: String; }
locus Echo {
    fn ping(p: Ping) -> Pong { return Pong { n: p.n + 1, tag: p.tag }; }
}
api Bench { rpc Echo::ping; }
locus Open { fn holds(p: std::api::Principal, r: String) -> Bool { return true; } }
locus Tokens {
    fn principal(token: String) -> std::api::Principal { return std::api::Principal { mode: \"bearer\", name: \"x\" }; }
    fn refused() -> String { return \"no\"; }
}
";

fn serve(transport: &str, tag: &str) -> Child {
    let src = format!(
        "{TYPES}main locus App {{\n    params {{\n        echo: Echo = Echo {{ }};\n        h: api::Exposure = api::serve(Bench, {transport}, as: \"b\", receivers: {{ Echo: self.echo }}, bound: 64, on_full: refuse);\n    }}\n}}\nfn main() {{ App {{ }}; }}\n"
    );
    let bin = harness::unique_bin(tag);
    build_opts::build_source(&src, &bin, &build_opts::options()).expect("build the echo server");
    Command::new(&bin).stdout(Stdio::null()).stderr(Stdio::null()).spawn().expect("run the echo server")
}

fn rss_kb(pid: u32) -> u64 {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).expect("status");
    status.lines().find_map(|l| l.strip_prefix("VmRSS:")).and_then(|v| v.trim().trim_end_matches(" kB").trim().parse().ok()).expect("VmRSS")
}

/// Calls on one connection over the unix transport.
#[test]
fn a_unix_connection_stays_flat_under_sustained_calls() {
    let sock = std::env::temp_dir().join(format!("hale-api-memory-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&sock);
    let mut srv = serve(&format!("unix::Rpc {{ path: {:?}, roles: Open {{ }} }}", sock.display().to_string()), "api_memory_unix");
    let start = Instant::now();
    let conn = loop {
        if let Ok(c) = UnixStream::connect(&sock) {
            break c;
        }
        assert!(start.elapsed() < Duration::from_secs(30), "the server never listened");
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut reader = BufReader::new(conn.try_clone().expect("clone"));
    let mut writer = conn;
    let mut call = |i: u64| {
        writeln!(writer, "{{\"call\":\"Echo::ping\",\"payload\":{{\"n\":{i},\"tag\":\"abc\"}},\"id\":\"c\"}}").expect("send");
        let mut line = String::new();
        reader.read_line(&mut line).expect("reply");
        assert!(line.contains(&format!("\"n\":{}", i + 1)), "{line}");
    };
    for i in 0..500 {
        call(i);
    }
    let before = rss_kb(srv.id());
    for i in 0..20_000 {
        call(i);
    }
    let after = rss_kb(srv.id());
    let _ = srv.kill();
    let _ = srv.wait();
    let _ = std::fs::remove_file(&sock);
    // 20k calls held about 180 MB before the fix
    assert!(after < before + 8 * 1024, "the unix server grew from {before} kB to {after} kB over 20000 calls");
}

/// One call per connection over HTTP: a connection's peer subscribes to its
/// outcomes, and the bus registry keeps a dissolved subscriber's entries,
/// about 0.3 KB a connection. Open until the registry reuses them.
#[test]
#[ignore = "open: the bus registry keeps a dissolved subscriber's entries (about 0.3 KB a connection)"]
fn http_connections_stay_flat() {
    let mut srv = serve("http::Rpc { bind: \"127.0.0.1:0\", codec: json, principals: Tokens { }, roles: Open { } }", "api_memory_http");
    let port = child::tcp_port(&mut srv);
    let call = |i: u64| {
        let body = format!("{{\"n\":{i},\"tag\":\"abc\"}}");
        let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).expect("connect");
        write!(
            s,
            "POST /call/Echo::ping HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer x\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .expect("send");
        let mut reply = String::new();
        s.read_to_string(&mut reply).expect("reply");
        assert!(reply.contains(&format!("\"n\":{}", i + 1)), "{reply}");
    };
    for i in 0..500 {
        call(i);
    }
    let before = rss_kb(srv.id());
    for i in 0..20_000 {
        call(i);
    }
    let after = rss_kb(srv.id());
    let _ = srv.kill();
    let _ = srv.wait();
    assert!(after < before + 2 * 1024, "the http server grew from {before} kB to {after} kB over 20000 connections");
}

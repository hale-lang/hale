//! The TCP connection substrate (spec/stdlib.md § "The TCP connection
//! substrate"): a listener that only binds, accepts and relays each
//! connection over the bus, and consumers that serve what they are handed.
//!
//! Here a pinned `std::io::tcp::Acceptor` deals its connections over two
//! `std::io::tcp::Connections`, each on an async_io pool of its own and each
//! serving HTTP through `std::http::Responder` with a handler that sleeps a
//! second and answers with its replica's tag: the routing is the listener's
//! shards, the concurrency each consumer's placement. A connection on a
//! route no consumer takes is closed by its listener, never leaked; and the
//! program stops its listener across pools with `std::io::tcp::stop`.

#![cfg(target_os = "linux")]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::collections::BTreeSet;
use std::time::{Duration, Instant};

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;
#[path = "support/ports.rs"]
mod ports;

// Two replicas behind one listener of two shards.
const REPLICAS: &str = r#"
    locus Api {
        params { tag: String = ""; }
        fn handle(req: std::http::Request) -> std::http::Response {
            std::time::sleep(1000ms);
            return std::http::Response { status: 200, body: self.tag };
        }
    }
    main locus App {
        params {
            port: Int = std::str::parse_int(std::env::arg(1)) or 0;
            zero: std::io::tcp::Connections = std::io::tcp::Connections { listener: "web", shard: 0, handler: std::http::Responder { handler: Api { tag: "zero" } } };
            one: std::io::tcp::Connections = std::io::tcp::Connections { listener: "web", shard: 1, handler: std::http::Responder { handler: Api { tag: "one" } } };
            acceptor: std::io::tcp::Acceptor = std::io::tcp::Acceptor { port: self.port, name: "web", shards: 2, ready_signal: "ready" };
        }
        placement {
            zero: cooperative(pool = zero) where async_io;
            one: cooperative(pool = one) where async_io;
            acceptor: pinned;
        }
        run() {
            while !self.draining { std::time::sleep(20ms); }
            std::io::tcp::stop("web");
        }
    }
    fn main() { App { }; }
"#;

// The same listener with the shard-1 consumer missing: every second
// connection is on a route nobody takes.
const HALF: &str = r#"
    locus Api {
        fn handle(req: std::http::Request) -> std::http::Response {
            return std::http::Response { status: 200, body: "zero" };
        }
    }
    main locus App {
        params {
            port: Int = std::str::parse_int(std::env::arg(1)) or 0;
            zero: std::io::tcp::Connections = std::io::tcp::Connections { listener: "web", shard: 0, handler: std::http::Responder { handler: Api { } } };
            acceptor: std::io::tcp::Acceptor = std::io::tcp::Acceptor { port: self.port, name: "web", shards: 2, ready_signal: "ready" };
        }
        placement {
            zero: cooperative(pool = zero) where async_io;
            acceptor: pinned;
        }
        run() {
            while !self.draining { std::time::sleep(20ms); }
            std::io::tcp::stop("web");
        }
    }
    fn main() { App { }; }
"#;

struct Running {
    child: Child,
    port: u16,
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Running {
    fn start(bin: &Path) -> Running {
        let port = ports::free_port();
        let mut child = Command::new(bin)
            .arg(port.to_string())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn");
        let mut line = String::new();
        let _ = BufReader::new(child.stdout.take().unwrap()).read_line(&mut line);
        assert_eq!(line.trim(), "ready", "the listener did not come up on {port}");
        Running { child, port }
    }

    /// SIGTERM, and the program's exit and stderr.
    fn term(mut self) -> (std::process::ExitStatus, String) {
        let _ = Command::new("kill").args(["-TERM", &self.child.id().to_string()]).status();
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(status) = self.child.try_wait().expect("wait") {
                let mut err = String::new();
                if let Some(mut e) = self.child.stderr.take() {
                    let _ = e.read_to_string(&mut err);
                }
                return (status, err);
            }
            assert!(Instant::now() < deadline, "the program did not end on SIGTERM");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn open_fds(&self) -> usize {
        std::fs::read_dir(format!("/proc/{}/fd", self.child.id())).map(|d| d.count()).unwrap_or(0)
    }
}

/// One request: the status code and the body, or None when the connection
/// was closed without a response.
fn get(port: u16) -> Option<(u16, String)> {
    let mut s = TcpStream::connect(("127.0.0.1", port)).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
    s.write_all(b"GET / HTTP/1.0\r\nHost: x\r\n\r\n").ok()?;
    let mut buf = String::new();
    let _ = s.read_to_string(&mut buf);
    let code = buf.split_whitespace().nth(1)?.parse().ok()?;
    Some((code, buf.split("\r\n\r\n").nth(1).unwrap_or("").to_string()))
}

fn build(name: &str, source: &str) -> PathBuf {
    let bin = harness::unique_bin(name);
    build_opts::build_source(source, &bin, &build_opts::options()).expect("build");
    bin
}

#[test]
fn two_replicas_share_one_listener_and_serve_concurrently() {
    let bin = build("tcp_substrate_replicas", REPLICAS);
    let server = Running::start(&bin);
    let port = server.port;
    let t0 = Instant::now();
    let hs: Vec<_> = (0..10).map(|_| std::thread::spawn(move || get(port))).collect();
    let answers: Vec<_> = hs.into_iter().map(|h| h.join().unwrap()).collect();
    let took = t0.elapsed();
    let count = |tag: &str| answers.iter().filter(|a| a.as_ref().is_some_and(|(c, b)| *c == 200 && b == tag)).count();
    assert_eq!((count("zero"), count("one")), (5, 5), "the listener deals connections round-robin over its shards: {answers:?}");
    assert!(took < Duration::from_millis(2500), "ten 1 s requests over two async_io replicas took {took:?}");
    // the main locus stops the pinned listener across pools by its name
    let (status, stderr) = server.term();
    assert!(status.success(), "stop(\"web\") ends the accept and the program exits: {status:?}\n{stderr}");
    assert!(!stderr.contains("lotus_tcp_accept_one"), "a deliberate stop is silent: {stderr}");
    let _ = std::fs::remove_file(&bin);
}

#[test]
fn a_connection_no_consumer_takes_is_closed_by_its_listener() {
    let bin = build("tcp_substrate_unclaimed", HALF);
    let server = Running::start(&bin);
    let port = server.port;
    // a warm-up pair, then many: the shard-1 half is closed unanswered,
    // promptly, and the process's descriptors do not grow with it
    let _ = (get(port), get(port));
    let before = server.open_fds();
    let mut served = 0;
    let mut closed = 0;
    for _ in 0..40 {
        let t0 = Instant::now();
        match get(port) {
            Some((200, b)) if b == "zero" => served += 1,
            None => closed += 1,
            other => panic!("unexpected answer {other:?}"),
        }
        assert!(t0.elapsed() < Duration::from_secs(2), "an unclaimed connection is closed, not left hanging");
    }
    assert_eq!((served, closed), (20, 20), "the shard-0 half is served, the shard-1 half closed");
    std::thread::sleep(Duration::from_millis(200));
    let after = server.open_fds();
    assert!(after <= before + 2, "unclaimed connections leak no descriptor: {before} -> {after}");
    let (status, stderr) = server.term();
    assert!(status.success(), "{status:?}\n{stderr}");
    let _ = std::fs::remove_file(&bin);
}

#[test]
fn the_replicas_run_clean_under_asan() {
    // handles cross threads (pinned listener, two async pools) and each
    // connection's flow is reclaimed as it ends; with the chunk pool off a
    // reclaimed region read again would be reported
    let bin = harness::unique_bin("tcp_substrate_replicas_asan");
    harness::build_source_asan(REPLICAS, &bin);
    let port = ports::free_port();
    let mut child = Command::new(&bin)
        .arg(port.to_string())
        .env("LOTUS_NO_CHUNK_POOL", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    let mut line = String::new();
    let _ = BufReader::new(child.stdout.take().unwrap()).read_line(&mut line);
    assert_eq!(line.trim(), "ready");
    let server = Running { child, port };
    let hs: Vec<_> = (0..6).map(|_| std::thread::spawn(move || get(port))).collect();
    for h in hs {
        assert_eq!(h.join().unwrap().map(|(c, _)| c), Some(200));
    }
    let (status, stderr) = server.term();
    for bad in ["AddressSanitizer", "LeakSanitizer", "heap-use-after-free", "SUMMARY:"] {
        assert!(!stderr.contains(bad), "{bad} in:\n{stderr}");
    }
    assert!(status.success(), "{status:?}\n{stderr}");
    let _ = std::fs::remove_file(&bin);
}

// Two servers on ephemeral ports, pinned, with no name given (review of
// #1508): each default name is its bound address, so each consumer takes
// its own server's connections only. NAMES are the two `name` params, or
// none.
const TWO_SERVERS: &str = r#"
    locus Tag {
        params { tag: String = ""; hits: Int = 0; }
        fn handle(req: std::http::Request) -> std::http::Response {
            self.hits = self.hits + 1;
            return std::http::Response { status: 200, body: self.tag + " " + to_string(self.hits) };
        }
    }
    main locus App {
        params {
            a: std::http::Server = std::http::Server { port: 0, handler: Tag { tag: "a" }NAME_A };
            b: std::http::Server = std::http::Server { port: 0, handler: Tag { tag: "b" }NAME_B };
        }
        placement { a: pinned; b: pinned; }
        on_failure(s: std::http::Server, err: ClosureViolation) {
            println("refused " + err.closure);
        }
        run() {
            // the pinned servers bind in their births, on their own threads:
            // wait for both (or for two seconds, when one is refused)
            let mut waited = 0;
            while (self.a.listen_fd < 0 || self.b.listen_fd < 0) && waited < 200 {
                std::time::sleep(10ms);
                waited = waited + 1;
            }
            println("ports " + to_string(std::io::tcp::__local_port(self.a.listen_fd)) + " " + to_string(std::io::tcp::__local_port(self.b.listen_fd)));
            println("names " + self.a.listener + " " + self.b.listener);
            while !self.draining { std::time::sleep(20ms); }
        }
    }
    fn main() { App { }; }
"#;

struct Pair {
    child: Child,
    ports: (i64, i64),
    names: String,
    refused: Option<String>,
}

impl Drop for Pair {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn start_pair(tag: &str, names: (&str, &str), expect_refusal: bool) -> Pair {
    let src = TWO_SERVERS.replace("NAME_A", names.0).replace("NAME_B", names.1);
    let bin = build(tag, &src);
    let mut child = Command::new(&bin).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().expect("spawn");
    let out = child.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            let _ = tx.send(line);
        }
    });
    let (mut ports, mut names, mut refused) = ((-1, -1), None::<String>, None);
    let deadline = Instant::now() + Duration::from_secs(20);
    // the names line, and, when one is expected, the refusal (the
    // supervisor hears it on its own schedule)
    while Instant::now() < deadline && (names.is_none() || (expect_refusal && refused.is_none())) {
        let Ok(l) = rx.recv_timeout(Duration::from_millis(100)) else { continue };
        if let Some(r) = l.strip_prefix("refused ") {
            refused = Some(r.to_string());
        } else if let Some(p) = l.strip_prefix("ports ") {
            let v: Vec<i64> = p.split(' ').map(|x| x.parse().unwrap()).collect();
            ports = (v[0], v[1]);
        } else if let Some(n) = l.strip_prefix("names ") {
            names = Some(n.to_string());
        }
    }
    let _ = std::fs::remove_file(&bin);
    Pair { child, ports, names: names.expect("the program printed its names"), refused }
}

/// The process's listening sockets: its socket descriptors whose inode
/// `/proc/<pid>/net/tcp` lists in the LISTEN state (`0A`).
fn listening(pid: u32) -> usize {
    let inodes: BTreeSet<String> = std::fs::read_dir(format!("/proc/{pid}/fd"))
        .map(|d| {
            d.flatten()
                .filter_map(|e| std::fs::read_link(e.path()).ok())
                .filter_map(|t| t.to_string_lossy().strip_prefix("socket:[").and_then(|r| r.strip_suffix(']')).map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let table = std::fs::read_to_string(format!("/proc/{pid}/net/tcp")).unwrap_or_default();
    table
        .lines()
        .skip(1)
        .filter(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            f.len() > 9 && f[3] == "0A" && inodes.contains(f[9])
        })
        .count()
}

/// Every request to `port` is answered by the handler tagged `tag`, in
/// order: no other consumer takes a connection of this server.
fn answered_by(port: i64, tag: &str, n: usize) {
    for i in 1..=n {
        let got = get(port as u16).map(|(_, b)| b);
        assert_eq!(got.as_deref(), Some(format!("{tag} {i}").as_str()), "request {i} to {port}");
    }
}

#[test]
fn two_ephemeral_servers_have_their_own_names_and_their_own_connections() {
    let p = start_pair("tcp_substrate_two_ephemeral", ("", ""), false);
    assert!(p.refused.is_none(), "{:?}", p.refused);
    let (a, b) = p.ports;
    assert!(a > 0 && b > 0 && a != b, "two bound ports: {a} {b}");
    assert_eq!(p.names, format!("127.0.0.1:{a} 127.0.0.1:{b}"), "each default name is its bound address");
    // interleaved, so a shared route would hand one server's connection
    // to the other's handler
    for i in 1..=5 {
        assert_eq!(get(a as u16).map(|(_, b)| b), Some(format!("a {i}")));
        assert_eq!(get(b as u16).map(|(_, b)| b), Some(format!("b {i}")));
    }
    assert_eq!(listening(p.child.id()), 2, "two listening sockets");
}

#[test]
fn an_explicit_name_another_listener_holds_fails_at_birth_with_its_socket_closed() {
    let p = start_pair("tcp_substrate_dup_name", (", name: \"dup\"", ", name: \"dup\""), true);
    assert_eq!(p.refused.as_deref(), Some("name_taken"), "the second server's birth is the violation");
    // the refused server's socket is closed: of the two ports bound at
    // instantiation, one answers and the other refuses the connection
    let (a, b) = p.ports;
    let answers = |port: i64| TcpStream::connect(("127.0.0.1", port as u16)).is_ok();
    assert!(answers(a) != answers(b), "exactly one of {a} {b} listens");
    let (live, tag) = if answers(a) { (a, "a") } else { (b, "b") };
    answered_by(live, tag, 3);
    assert_eq!(listening(p.child.id()), 1, "the refused server's socket is closed");
}

#[test]
fn distinct_explicit_names_both_serve() {
    let p = start_pair("tcp_substrate_distinct_names", (", name: \"one\"", ", name: \"two\""), false);
    assert!(p.refused.is_none(), "{:?}", p.refused);
    assert_eq!(p.names, "one two");
    answered_by(p.ports.0, "a", 3);
    answered_by(p.ports.1, "b", 3);
    assert_eq!(listening(p.child.id()), 2);
}

// A live Acceptor named "web" with no consumer, and a Server on the same
// port, named NAME, whose bind fails and whose supervisor absorbs it
// (review of #1508, round 2). The failed server holds no subscription on
// any route, so the acceptor's catch-unmatched closes every connection:
// with NAME "web" its consumer used to sit on web#0 unarmed, shadow the
// fallback, and leave every accepted descriptor open.
const SHADOW: &str = r#"
    locus Tag {
        fn handle(req: std::http::Request) -> std::http::Response {
            return std::http::Response { status: 200, body: "served" };
        }
    }
    main locus App {
        params {
            port: Int = std::str::parse_int(std::env::arg(1)) or 0;
            acc: std::io::tcp::Acceptor = std::io::tcp::Acceptor { port: self.port, name: "web", ready_signal: "ready" };
            web: std::http::Server = std::http::Server { port: self.port, name: "NAME", handler: Tag { } };
        }
        placement { acc: pinned; web: pinned; }
        on_failure(s: std::http::Server, err: ClosureViolation) {
            println("refused " + err.closure);
        }
        run() {
            while !self.draining { std::time::sleep(20ms); }
            std::io::tcp::stop("web");
        }
    }
    fn main() { App { }; }
"#;

/// Read `child`'s stdout until `want` (or 20 s).
fn wait_line(lines: &std::sync::mpsc::Receiver<String>, want: &str) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if let Ok(l) = lines.recv_timeout(Duration::from_millis(100)) {
            if l.trim() == want {
                return;
            }
        }
    }
    panic!("never printed `{want}`");
}

fn spawn_lines(bin: &Path, port: u16) -> (Child, std::sync::mpsc::Receiver<String>) {
    let mut child = Command::new(bin).arg(port.to_string()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().expect("spawn");
    let out = child.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            let _ = tx.send(line);
        }
    });
    (child, rx)
}

fn shadow_case(name: &str) {
    let bin = build(&format!("tcp_substrate_shadow_{name}"), &SHADOW.replace("NAME", name));
    let port = ports::free_port();
    let (child, lines) = spawn_lines(&bin, port);
    let running = Running { child, port };
    wait_line(&lines, "ready");
    wait_line(&lines, "refused listen_failed");
    let fds = || std::fs::read_dir(format!("/proc/{}/fd", running.child.id())).map(|d| d.count()).unwrap_or(0);
    let before = fds();
    for _ in 0..40 {
        let t0 = Instant::now();
        assert_eq!(get(port), None, "no consumer: the acceptor closes the connection");
        assert!(t0.elapsed() < Duration::from_secs(2), "closed at once, not left open");
    }
    std::thread::sleep(Duration::from_millis(200));
    let after = fds();
    assert!(after <= before + 2, "{name}: unclaimed connections leak no descriptor: {before} -> {after}");
    let _ = std::fs::remove_file(&bin);
    let (status, stderr) = running.term();
    assert!(status.success(), "{name}: {status:?}\n{stderr}");
}

#[test]
fn a_server_whose_birth_failed_leaves_no_subscription_on_a_live_route() {
    // the shadowing case: the failed server would have taken the live
    // acceptor's route; and the control, on a route of its own
    shadow_case("web");
    shadow_case("other");
}

// A server or an acceptor whose bind fails on a bad host; the supervisor
// corrects the host and restarts it, and the restart binds afresh.
const RESTART_SERVER: &str = r#"
    locus Tag {
        fn handle(req: std::http::Request) -> std::http::Response {
            return std::http::Response { status: 200, body: "served" };
        }
    }
    main locus App {
        params {
            port: Int = std::str::parse_int(std::env::arg(1)) or 0;
            web: std::http::Server = std::http::Server { host: "256.0.0.1", port: self.port, name: "web", handler: Tag { }, ready_signal: "ready" };
        }
        placement { web: cooperative(pool = web) where async_io; }
        on_failure(s: std::http::Server, err: ClosureViolation) {
            println("refused " + err.closure);
            s.host = "127.0.0.1";
            restart(s) for 1;
        }
        run() {
            while !self.draining { std::time::sleep(20ms); }
            std::io::tcp::stop("web");
        }
    }
    fn main() { App { }; }
"#;

const RESTART_ACCEPTOR: &str = r#"
    locus Tag {
        fn handle(req: std::http::Request) -> std::http::Response {
            return std::http::Response { status: 200, body: "served" };
        }
    }
    main locus App {
        params {
            port: Int = std::str::parse_int(std::env::arg(1)) or 0;
            conns: std::io::tcp::Connections = std::io::tcp::Connections { listener: "acc", handler: std::http::Responder { handler: Tag { } } };
            acc: std::io::tcp::Acceptor = std::io::tcp::Acceptor { host: "256.0.0.1", port: self.port, name: "acc", ready_signal: "ready" };
        }
        placement { conns: cooperative(pool = c) where async_io; acc: cooperative(pool = a) where async_io; }
        on_failure(a: std::io::tcp::Acceptor, err: ClosureViolation) {
            println("refused " + err.closure);
            a.host = "127.0.0.1";
            restart(a) for 1;
        }
        run() {
            while !self.draining { std::time::sleep(20ms); }
            std::io::tcp::stop("acc");
        }
    }
    fn main() { App { }; }
"#;

fn restart_case(tag: &str, source: &str) {
    let bin = build(tag, source);
    let port = ports::free_port();
    let (child, lines) = spawn_lines(&bin, port);
    let running = Running { child, port };
    wait_line(&lines, "refused listen_failed");
    wait_line(&lines, "ready");
    for _ in 0..3 {
        assert_eq!(get(port), Some((200, "served".to_string())), "{tag}: the restart bound the corrected host and serves");
    }
    // Not a SIGTERM exit: the restart runs the listener's `run()` on the
    // supervisor's thread, so the main locus's `run()` (which would stop
    // it) does not start — the runtime's restart placement, not the
    // substrate's; the harness kills the process.
    drop(running);
    let _ = std::fs::remove_file(&bin);
}

#[test]
fn a_restarted_server_binds_afresh() {
    restart_case("tcp_substrate_restart_server", RESTART_SERVER);
}

#[test]
fn a_restarted_acceptor_binds_afresh() {
    restart_case("tcp_substrate_restart_acceptor", RESTART_ACCEPTOR);
}

// A server whose main locus blocks on stdin (review of #1508, round 3).
// The server builds its consumer in `birth()`; pinned, that consumer's
// subscription went to the global queue, which only main drains, so a
// request was accepted and never answered. It registers with the pinned
// thread's mailbox now, as a consumer built in the params did.
const BLOCKED_MAIN: &str = r#"
    locus Api {
        params { hits: Int = 0; }
        fn handle(req: std::http::Request) -> std::http::Response {
            self.hits = self.hits + 1;
            return std::http::Response { status: 200, body: "ok " + to_string(self.hits) };
        }
    }
    main locus App {
        params {
            port: Int = std::str::parse_int(std::env::arg(1)) or 0;
            web: std::http::Server = std::http::Server { port: self.port, name: "web", handler: Api { }, ready_signal: "ready" };
        }
        placement { web: PLACEMENT; }
        run() {
            let line = std::io::stdin::read_line();
            println("main got " + line);
            std::io::tcp::stop("web");
        }
    }
    fn main() { App { }; }
"#;

fn served_while_main_blocks(tag: &str, placement: &str) {
    let bin = build(tag, &BLOCKED_MAIN.replace("PLACEMENT", placement));
    let port = ports::free_port();
    let mut child = Command::new(&bin)
        .arg(port.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    let mut stdin = child.stdin.take().unwrap();
    let out = child.stdout.take().unwrap();
    let (tx, lines) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            let _ = tx.send(line);
        }
    });
    let running = Running { child, port };
    wait_line(&lines, "ready");
    // main is blocked in read_line: the server's own thread answers
    for i in 1..=3 {
        assert_eq!(get(port), Some((200, format!("ok {i}"))), "{tag}: answered while main blocks");
    }
    stdin.write_all(b"bye\n").unwrap();
    drop(stdin);
    wait_line(&lines, "main got bye");
    drop(running);
    let _ = std::fs::remove_file(&bin);
}

#[test]
fn a_pinned_server_answers_while_main_blocks() {
    served_while_main_blocks("tcp_substrate_blocked_main_pinned", "pinned");
}

#[test]
fn an_async_io_server_answers_while_main_blocks() {
    served_while_main_blocks("tcp_substrate_blocked_main_async", "cooperative(pool = web) where async_io");
}

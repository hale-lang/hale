//! UDP ingress (downstream handoff, 2026-11-06): an empty datagram is an
//! ordinary frame, not the reader's stop, and the reader's stop at the
//! exit quiesce is an explicit wake on every platform.
//!
//! The reader took recvfrom's n == 0 as its stop sentinel (what a
//! shutdown(SHUT_RDWR) of the socket produced on Linux), so a genuine
//! zero-byte datagram ended ingress for good: every later datagram went
//! unread, silently. And Darwin does not honour shutdown() on an
//! unconnected UDP socket, so an idle reader there never woke at the
//! quiesce ("ingress quiesce: reader(s) still draining after 500 ms") and
//! the process hung on shutdown. The reader now polls the socket and a
//! stop pipe, on both platforms; the quiesce and the teardown write the
//! pipe. Held here on Linux: a zero-byte datagram, then a valid frame,
//! which is delivered; then SIGTERM, which ends the program promptly.

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

#[test]
fn an_empty_datagram_does_not_stop_the_reader_and_sigterm_ends_the_program() {
    let sub_bin = harness::unique_bin("udp_ingress_sub");
    build_opts::build_source(SUBSCRIBER, &sub_bin, &build_opts::options()).expect("build subscriber");
    let pub_bin = harness::unique_bin("udp_ingress_pub");
    build_opts::build_source(PUBLISHER, &pub_bin, &build_opts::options()).expect("build publisher");

    // an address the subscriber is told before it binds: held, not drawn
    let addr = child::Reserved::new();
    let sub_cfg = harness::unique_bin("udp_ingress_sub.conf");
    let pub_cfg = harness::unique_bin("udp_ingress_pub.conf");
    std::fs::write(&sub_cfg, format!("evt = udp://{}:{}:listen\n", addr.host(), addr.port())).expect("sub cfg");
    std::fs::write(&pub_cfg, format!("evt = udp://{}:{}:connect\n", addr.host(), addr.port())).expect("pub cfg");

    let mut sub = Command::new(&sub_bin)
        .env("LOTUS_BUS_CONFIG", &sub_cfg)
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

    // a zero-byte datagram: an ordinary (short, rejected) frame
    let probe = UdpSocket::bind("127.0.0.1:0").expect("probe socket");
    probe.send_to(&[], addr.addr).expect("send the empty datagram");
    std::thread::sleep(Duration::from_millis(50));

    // then a valid frame, from a publisher of the same type
    let pub_out = Command::new(&pub_bin)
        .env("LOTUS_BUS_CONFIG", &pub_cfg)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run publisher");
    assert!(pub_out.status.success(), "publisher: {:?}\n{}", pub_out.status, String::from_utf8_lossy(&pub_out.stderr));
    assert!(
        wait_for_line(&lines, "got n=4242", Duration::from_secs(10)),
        "the frame after the empty datagram was not delivered: the reader stopped on it\nstdout:\n{}",
        lines.lock().unwrap().join("\n")
    );

    // SIGTERM: main's run() sees the drain, the quiesce stops the reader
    // (the stop pipe, not a shutdown() of the socket), the program ends
    let sent = Instant::now();
    Command::new("kill").args(["-TERM", &sub.0.id().to_string()]).status().expect("kill");
    let deadline = sent + Duration::from_secs(5);
    let status = loop {
        if let Some(st) = sub.0.try_wait().expect("try_wait") {
            break st;
        }
        assert!(Instant::now() < deadline, "the program did not end within 5 s of SIGTERM: the reader did not stop");
        std::thread::sleep(Duration::from_millis(10));
    };
    let took = sent.elapsed();
    let mut stderr = String::new();
    if let Some(mut e) = sub.0.stderr.take() {
        let _ = e.read_to_string(&mut stderr);
    }
    let _ = reader.join();
    drop(sub);
    for f in [&sub_bin, &pub_bin, &sub_cfg, &pub_cfg] {
        let _ = std::fs::remove_file(f);
    }
    let out = lines.lock().unwrap().join("\n");
    assert!(status.success(), "{status:?}\nstdout:\n{out}\nstderr:\n{stderr}");
    assert!(!stderr.contains("still draining"), "the quiesce waited out its budget:\n{stderr}");
    assert!(took < Duration::from_secs(2), "the program took {took:?} to end after SIGTERM:\n{stderr}");
    assert!(out.lines().any(|l| l == "main run done"), "stdout:\n{out}");
}

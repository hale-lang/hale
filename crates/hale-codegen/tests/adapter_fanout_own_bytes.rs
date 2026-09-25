//! GH #1058: a subject bound to an adapter and also routed over an
//! environment Unix route sends the bytes it was published with.
//!
//! The publisher serializes into its thread's TLS wire buffer and the
//! remote fanout hands that buffer to each route in turn. An adapter
//! entry runs user code in the middle of that loop — its `send` — and
//! a `send` that republishes the bytes onto a topic of its own (pond's
//! NatsAdapter shape) serialized into the same buffer. The Unix route
//! after it sent the adapter's `Out` record, which the listener decoded
//! as `Note { n: 10 }` — the length of "probe.ping" — for every
//! message. The fanout now works from its own copy of the bytes when an
//! adapter is bound to the subject.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

const SRC: &str = r#"
type Note { n: Int = 0; }
topic Ping { payload: Note; subject: "probe.ping"; }

// an adapter shaped like pond's NatsAdapter: send hands the bytes on
// through an internal topic, to a queue nothing here drains
type Out { subject: String = ""; data: Bytes = b""; }
topic Outbound { payload: Out; subject: "probe.outbound"; }
locus Drop {
    bus { publish Outbound; }
    fn send(subject: String, bytes: Bytes) { Outbound <- Out { subject: subject, data: bytes }; }
}

locus Listener {
    params { heard: Int = 0; }
    bus { subscribe Ping as on_ping; }
    fn on_ping(p: Note) { self.heard = self.heard + 1; println("heard n=", p.n); }
    run() {
        let mut i = 0;
        while i < 100 && self.heard < 3 { std::time::sleep(100ms); i = i + 1; }
        std::process::exit(if self.heard >= 3 { 0 } else { 4 });
    }
}

locus Sender {
    bus { publish Ping; }
    run() {
        let mut i = 0;
        while i < 3 { Ping <- Note { n: i }; i = i + 1; std::time::sleep(100ms); }
        std::time::sleep(300ms);
    }
}

main locus App {
    bindings { Ping: Drop { }; }
    bus { publish Ping; }
    run() { }
}

fn main() {
    let verb = std::env::arg_or(1, "");
    if verb == "listen" { Listener { }; return; }
    if verb == "send" { Sender { }; return; }
}
"#;

#[test]
fn an_adapter_beside_a_unix_route_leaves_the_published_bytes_alone() {
    let program = hale_syntax::parse_source(SRC).expect("parse");
    let bin = harness::unique_bin("hale_1058_adapter_fanout");
    build_executable(&program, &bin).expect("build");
    let sock = harness::unique_bin("hale_1058_sock").with_extension("sock");
    let listen_cfg = harness::unique_bin("hale_1058_listen").with_extension("conf");
    let send_cfg = harness::unique_bin("hale_1058_send").with_extension("conf");
    let _ = std::fs::remove_file(&sock);
    std::fs::write(&listen_cfg, format!("probe.ping = unix://{} : listen\n", sock.display()))
        .expect("write listen config");
    std::fs::write(&send_cfg, format!("probe.ping = unix://{} : connect\n", sock.display()))
        .expect("write send config");

    let mut listener = Command::new(&bin)
        .arg("listen")
        .env("LOTUS_BUS_CONFIG", &listen_cfg)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn listener");
    let t0 = Instant::now();
    while !sock.exists() && t0.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(20));
    }
    let sent = Command::new(&bin)
        .arg("send")
        .env("LOTUS_BUS_CONFIG", &send_cfg)
        .output()
        .expect("run sender");
    let status = listener.wait().expect("listener exits");
    let mut stdout = String::new();
    listener.stdout.take().expect("piped").read_to_string(&mut stdout).expect("read");
    let mut stderr = String::new();
    listener.stderr.take().expect("piped").read_to_string(&mut stderr).expect("read");
    for p in [&bin, &sock, &listen_cfg, &send_cfg] {
        let _ = std::fs::remove_file(p);
    }
    assert!(
        sent.status.success(),
        "sender: {:?}; stderr: {}",
        sent.status,
        String::from_utf8_lossy(&sent.stderr)
    );
    assert!(status.success(), "listener: {status:?}; stdout: {stdout}; stderr: {stderr}");
    assert_eq!(
        stdout.lines().collect::<Vec<_>>(),
        ["heard n=0", "heard n=1", "heard n=2"],
        "stderr: {stderr}"
    );
}

//! GH #1477 S1 — runtime probes. A main-locus param of type
//! `std::observe::Probes` serves `/healthz`, `/readyz` and `/capacity`
//! from a thread of the runtime's own, ready once the main locus's births
//! are done (spec/semantics.md § Probes). Each program binds port 0 and the test reads the port the
//! kernel gave back from `/proc` (`child::tcp_port`): no test hands a
//! child a port it drew.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;
#[path = "support/child.rs"]
mod child;

fn build(name: &str, source: &str) -> std::path::PathBuf {
    let bin = harness::unique_bin(&format!("probes_{name}"));
    build_opts::build_source(source, &bin, &build_opts::options()).expect("build");
    bin
}

fn spawn(bin: &std::path::Path, stall_ms: Option<&str>) -> (Child, u16) {
    let mut cmd = Command::new(bin);
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
    if let Some(ms) = stall_ms {
        cmd.env("LOTUS_PROBES_STALL_MS", ms);
    }
    let mut child = cmd.spawn().expect("run the program");
    let port = child::tcp_port(&mut child);
    (child, port)
}

/// One request; the status code and the body.
fn ask(port: u16, request: &str) -> (u16, String) {
    let mut c = TcpStream::connect(("127.0.0.1", port)).expect("connect to the probes");
    c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    c.write_all(request.as_bytes()).expect("send");
    let mut buf = Vec::new();
    let _ = c.read_to_end(&mut buf);
    let text = String::from_utf8_lossy(&buf).to_string();
    let status = text.split(' ').nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let body = text.split_once("\r\n\r\n").map(|(_, b)| b.to_string()).unwrap_or_default();
    (status, body)
}

fn get(port: u16, path: &str) -> (u16, String) {
    ask(port, &format!("GET {path} HTTP/1.1\r\nHost: probes\r\n\r\n"))
}

fn stop(mut child: Child) {
    let _ = child.kill();
    let _ = child.wait();
}

const SERVED: &str = r#"
main locus App {
    params { probes: std::observe::Probes = std::observe::Probes { port: 0 }; }
    run() { std::time::sleep(30s); }
}
fn main() { App { }; }
"#;

/// A declared probe set answers each path, from its own thread while
/// main sleeps in run(): live, ready, and the queues' capacity; anything
/// else is 404, and a probe answers GET (and HEAD) only.
#[test]
fn the_probes_answer_health_readiness_and_capacity() {
    let bin = build("served", SERVED);
    let (child, port) = spawn(&bin, None);
    assert_eq!(get(port, "/healthz"), (200, "ok\n".to_string()));
    assert_eq!(get(port, "/readyz"), (200, "ready\n".to_string()));
    let (status, body) = get(port, "/capacity");
    assert_eq!(status, 200, "{body}");
    assert!(body.starts_with("{\"ready\":true,\"draining\":false,\"queues\":[{\"name\":\"main\",\"pending\":0,"), "{body}");
    assert_eq!(get(port, "/nope").0, 404);
    assert_eq!(ask(port, "POST /healthz HTTP/1.1\r\nContent-Length: 0\r\n\r\n").0, 405);
    let (status, body) = ask(port, "HEAD /readyz HTTP/1.1\r\n\r\n");
    assert_eq!((status, body.as_str()), (200, ""));
    stop(child);
    let _ = std::fs::remove_file(&bin);
}

const DRAINS: &str = r#"
main locus App {
    params { probes: std::observe::Probes = std::observe::Probes { port: 0 }; }
    run() {
        while !self.draining { std::time::sleep(20ms); }
    }
    drain() { std::time::sleep(5s); }
}
fn main() { App { }; }
"#;

/// A draining process is not ready: after SIGTERM, while the main
/// locus's drain() runs, `/readyz` answers 503 `draining`, and the probes
/// still answer, from their own thread.
#[test]
fn a_draining_process_is_not_ready() {
    let bin = build("drains", DRAINS);
    let (child, port) = spawn(&bin, None);
    assert_eq!(get(port, "/readyz").0, 200);
    let _ = Command::new("kill").arg("-TERM").arg(child.id().to_string()).status();
    let until = Instant::now() + Duration::from_secs(3);
    let mut last = (0, String::new());
    while Instant::now() < until {
        last = get(port, "/readyz");
        if last.0 == 503 {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(last, (503, "draining\n".to_string()));
    assert!(get(port, "/capacity").1.contains("\"draining\":true"));
    stop(child);
    let _ = std::fs::remove_file(&bin);
}

const STALLS: &str = r#"
type Job { n: Int; }
locus Slow {
    bus { subscribe "jobs" as on_job of type Job; }
    fn on_job(j: Job) { std::time::sleep(20s); }
}
main locus App {
    params {
        probes: std::observe::Probes = std::observe::Probes { port: 0 };
        slow: Slow = Slow { };
    }
    placement { slow: cooperative(pool = slow); }
    bus { publish "jobs" of type Job; }
    run() {
        "jobs" <- Job { n: 1 };
        "jobs" <- Job { n: 2 };
        "jobs" <- Job { n: 3 };
        std::time::sleep(30s);
    }
}
fn main() { App { }; }
"#;

/// A pool whose worker holds one cell while more wait is stalled once its
/// heartbeat has stood still past the window: `/healthz` answers 503 and
/// names it, and `/capacity` shows what waits.
#[test]
fn a_stalled_pool_is_not_live() {
    let bin = build("stalls", STALLS);
    let (child, port) = spawn(&bin, Some("500"));
    let until = Instant::now() + Duration::from_secs(5);
    let mut last = (0, String::new());
    while Instant::now() < until {
        last = get(port, "/healthz");
        if last.0 == 503 {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(last, (503, "stalled: pool:slow\n".to_string()));
    let capacity = get(port, "/capacity").1;
    assert!(capacity.contains("{\"name\":\"pool:slow\",\"pending\":2,"), "{capacity}");
    stop(child);
    let _ = std::fs::remove_file(&bin);
}

const BOOTS: &str = r#"
locus Warm {
    birth() { std::time::sleep(2s); }
}
main locus App {
    params {
        probes: std::observe::Probes = std::observe::Probes { port: 0 };
        warm: Warm = Warm { };
    }
    run() { std::time::sleep(30s); }
}
fn main() { App { }; }
"#;

/// The probes' birth starts the listener; the process is ready only once
/// every birth of the main locus is done: while a later param is still
/// being born, `/readyz` answers 503 `starting` and `/healthz` 200.
#[test]
fn the_process_is_ready_once_its_births_are_done() {
    let bin = build("boots", BOOTS);
    let (child, port) = spawn(&bin, None);
    assert_eq!(get(port, "/readyz"), (503, "starting\n".to_string()));
    assert_eq!(get(port, "/healthz").0, 200);
    let until = Instant::now() + Duration::from_secs(5);
    let mut last = (0, String::new());
    while Instant::now() < until {
        last = get(port, "/readyz");
        if last.0 == 200 {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(last, (200, "ready\n".to_string()));
    stop(child);
    let _ = std::fs::remove_file(&bin);
}

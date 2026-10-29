//! An `http::Rest` exposure's `fallback:` is served as `std::http::Server`
//! serves its handler: on the listener's `where async_io` pool each
//! connection is a coroutine of its own, so N slow requests the fallback
//! answers (a second's `sleep`, or an event stream a second long) take
//! about a second, not N (spec/api.md § The REST transport, `fallback`).
//! A typed row is the exposure's dispatch, one call per receiver at a time
//! (§ The runtime boundaries), and stays so beside it: N calls of one
//! receiver's row that each sleep a second take N seconds.
//!
//! The fallback is never placed anywhere else: the listener's row must be a
//! `where async_io` pool and the handler's field the same pool (the
//! checker's refusals, `hale-types/tests/api_serve_check.rs`), so there is
//! no fallback served one connection at a time to compare against.
//!
//! The oracles are wall-clock bounds far from both shapes, and the answers
//! themselves.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

#[path = "support/build.rs"]
mod build_opts;
#[path = "support/harness.rs"]
mod harness;
#[path = "support/http_rpc.rs"]
mod http_rpc;

use http_rpc::*;

const PROGRAM: &str = r#"
type Health { status: String; }

api Shop {
    rpc Probe::live at "GET /healthz";
}

locus Tokens {
    fn principal(token: String) -> std::api::Principal {
        return std::api::Principal { mode: "bearer", name: "user" };
    }
    fn refused() -> String { return "no such token"; }
}

// one receiver whose one row takes a second
locus Probe {
    fn live() -> Health {
        std::time::sleep(1000ms);
        return Health { status: "live" };
    }
}

// the fallback: a second's sleep on `/slow`, a second's event stream on
// `/events`, at once elsewhere
locus Front {
    params { hits: Int = 0; }
    fn handle(req: std::http::Request) -> std::http::Response {
        if req.path == "/events" {
            let w = std::http::SseWriter { conn_fd: req.conn_fd };
            let mut i = 0;
            while i < 5 {
                std::time::sleep(200ms);
                w.event(std::http::SseEvent { event: "tick", data: to_string(i) }) or discard;
                i = i + 1;
            }
            return w.end();
        }
        if req.path == "/slow" {
            std::time::sleep(1000ms);
            self.hits = self.hits + 1;
            return std::http::Response { status: 200, body: "slow" };
        }
        return std::http::Response { status: 200, body: "fast" };
    }
}

main locus Svc {
    params {
        tokens: Tokens = Tokens { };
        probe: Probe = Probe { };
        front: Front = Front { };
        web: api::Exposure = api::serve(Shop, http::Rest { bind: std::env::var("BIND"), codec: json, principals: self.tokens, anonymous: "anon", fallback: self.front }, as: "web", bound: 64, on_full: refuse);
    }
    placement {
        web: cooperative(pool = edge) where async_io;
        front: cooperative(pool = edge) where async_io;
        probe: cooperative(pool = work) where async_io;
    }
    run() {
        let trigger = std::env::var("TRIGGER");
        while !self.draining && (std::io::fs::read_file(trigger) or "") == "" {
            std::time::sleep(10ms);
        }
        self.web.stop();
    }
}

fn main() { Svc { }; }
"#;

fn program() -> PathBuf {
    static BUILT: OnceLock<PathBuf> = OnceLock::new();
    BUILT
        .get_or_init(|| {
            let bin = harness::unique_bin("api_rest_conc");
            build_opts::build_source(PROGRAM, &bin, &build_opts::options()).expect("build the REST program");
            bin
        })
        .clone()
}

/// `n` concurrent GETs of `path`: every answer, and how long the last took.
fn burst(port: u16, path: &'static str, n: usize) -> (Vec<Response>, Duration) {
    let t0 = Instant::now();
    let handles: Vec<_> = (0..n).map(|_| std::thread::spawn(move || request(port, "GET", path, &[], None))).collect();
    let answers = handles.into_iter().map(|h| h.join().expect("a client thread")).collect();
    (answers, t0.elapsed())
}

#[test]
fn slow_fallback_requests_are_served_concurrently() {
    let server = Server::start(&program(), &[]);
    server.ready();
    for n in [8usize, 40] {
        let (answers, took) = burst(server.port, "/slow", n);
        for a in &answers {
            assert_eq!((a.status, a.body.as_str()), (200, "slow"), "every request answered");
        }
        // one at a time this is n seconds; concurrently, about one
        assert!(took < Duration::from_millis(2500), "{n} concurrent 1 s fallback requests took {took:?}");
    }
    let done = server.finish();
    assert!(done.status.success(), "the program ends cleanly: {:?}\n{}", done.status, done.stderr);
}

#[test]
fn fallback_event_streams_interleave() {
    let server = Server::start(&program(), &[]);
    server.ready();
    let (answers, took) = burst(server.port, "/events", 8);
    for a in &answers {
        assert_eq!(a.status, 200, "{a:?}");
        assert!(a.header("Content-Type").is_some_and(|t| t.contains("text/event-stream")), "{a:?}");
        for i in 0..5 {
            assert!(a.body.contains(&format!("event: tick\ndata: {i}\n\n")), "event {i} of the stream: {:?}", a.body);
        }
    }
    // each stream is a second of 200 ms parks; eight of them one after
    // another would be eight seconds
    assert!(took < Duration::from_millis(2500), "8 concurrent 1 s event streams took {took:?}");
    let done = server.finish();
    assert!(done.status.success(), "the program ends cleanly: {:?}\n{}", done.status, done.stderr);
}

#[test]
fn a_receivers_row_stays_one_call_at_a_time_beside_the_fallback() {
    let server = Server::start(&program(), &[]);
    server.ready();
    let port = server.port;
    let t0 = Instant::now();
    let rows: Vec<_> = (0..3).map(|_| std::thread::spawn(move || request(port, "GET", "/healthz", &[], None))).collect();
    // while the receiver works through them, the fallback answers at once
    std::thread::sleep(Duration::from_millis(150));
    let t1 = Instant::now();
    let fast = request(port, "GET", "/other", &[], None);
    assert_eq!((fast.status, fast.body.as_str()), (200, "fast"));
    assert!(t1.elapsed() < Duration::from_millis(500), "the fallback waited {:?} behind the rows", t1.elapsed());
    for h in rows {
        let r = h.join().expect("a client thread");
        assert_eq!((r.status, r.body.as_str()), (200, "{\"status\":\"live\"}"), "{r:?}");
    }
    // one receiver takes its calls one at a time: three seconds, not one
    let took = t0.elapsed();
    assert!(took >= Duration::from_millis(2800), "3 calls of one receiver's 1 s row took {took:?}: not one at a time");
    let done = server.finish();
    assert!(done.status.success(), "the program ends cleanly: {:?}\n{}", done.status, done.stderr);
}

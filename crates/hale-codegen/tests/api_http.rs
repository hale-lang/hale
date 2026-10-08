//! GH #1417 (R3): `http::Rpc`, the second socket transport of `api::serve`.
//!
//! The witness (`tests/api-contract/program.hl`) serves `Public` over HTTP.
//! These tests build that exposure as a program, serve it on a loopback port,
//! and drive it with real connections:
//!
//! * the recorded exchanges of `tests/api-contract/wire/http/` are replayed:
//!   each request is sent as recorded and each reply is held to the recorded
//!   one, status, content type and body, byte for byte (an HTTP reply carries
//!   no request id, so there is nothing to mask);
//! * the description fetched at `GET /.description` is, for `alice` and for
//!   `bob`, the fixture `public.<caller>.description.json`;
//! * a connection that fails ends that connection, and the listener goes on;
//! * a listener that cannot bind fails the boot.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;
#[path = "support/ports.rs"]
mod ports;
#[path = "support/http_rpc.rs"]
mod http_rpc;

use http_rpc::*;

/// The witness's `Public` half: the contract program's types, rows, handlers
/// and sources, served over `http::Rpc` on `$BIND`, `@BOUND@` requests at a
/// time. `Orders::place` holds the call for `$SLOW_MS`. The program stops its
/// handle when `$TRIGGER` appears.
pub const PUBLIC: &str = r#"
role trader;

unit cent;
type Money = quantity Int in cent;
type OrderId = distinct Int;

type PlaceOrder { symbol: String; qty: Int; limit: Money; }
type OrderReceipt { order: OrderId; notional: Money; }
type CancelOrder { order: OrderId; }
type Cancelled { order: OrderId; was_open: Bool; }
type OrderError { code: String; reason: String; }

api Public {
    rpc Orders::place;
    rpc Orders::cancel requires: [trader];
}

locus Tokens {
    fn principal(token: String) -> std::api::Principal {
        if token == "t-alice" { return std::api::Principal { mode: "bearer", name: "alice" }; }
        if token == "t-bob" { return std::api::Principal { mode: "bearer", name: "bob" }; }
        return std::api::Principal { mode: "bearer", name: "" };
    }
    fn refused() -> String { return "no such token"; }
}

locus Grants {
    params {
        trader: String = "";
    }
    fn holds(p: std::api::Principal, r: String) -> Bool {
        if r == "trader" { return len(self.trader) > 0 && p.name == self.trader; }
        return false;
    }
}

locus Orders {
    params {
        next: Int = 41;
        open: Int = 0;
    }
    closure position_limit { captures: open; epoch inline; }

    fn place(o: PlaceOrder) -> OrderReceipt fallible(ClosureViolation) {
        if o.qty > 10000 { violate position_limit; }
        let ms = std::str::parse_int(std::env::var("SLOW_MS")) or 0;
        let mut i = 0;
        while i < ms {
            std::time::sleep(1ms);
            i = i + 1;
        }
        let id = OrderId(self.next);
        self.next = self.next + 1;
        self.open = self.open + 1;
        return OrderReceipt { order: id, notional: o.limit * o.qty };
    }

    fn cancel(c: CancelOrder, ctx: std::api::Context) -> Cancelled fallible(OrderError) {
        let n = Int(c.order);
        if n < 41 || n >= self.next {
            fail OrderError { code: "unknown_order", reason: "no order " + to_string(n) };
        }
        self.open = self.open - 1;
        return Cancelled { order: c.order, was_open: true };
    }
}

main locus Desk {
    params {
        bearer: Tokens = Tokens { };
        public_roles: Grants = Grants { trader: "alice" };
        orders: Orders = Orders { };
    }
    placement {
        orders: cooperative(pool = desk) where async_io;
    }
    on_failure(l: Orders, err: ClosureViolation) { }
    run() {
        let public = api::serve(Public, http::Rpc { bind: std::env::var("BIND"), codec: json, principals: self.bearer, roles: self.public_roles }, as: "public", bound: @BOUND@, on_full: refuse);
        let trigger = std::env::var("TRIGGER");
        while !self.draining && (std::io::fs::read_file(trigger) or "") == "" {
            std::time::sleep(10ms);
        }
        public.stop();
        println("stopped");
    }
}

fn main() { Desk { }; }
"#;

/// The program with `bound` requests at a time, built once per process.
fn public(bound: u32) -> PathBuf {
    static BUILT: OnceLock<std::sync::Mutex<std::collections::BTreeMap<u32, PathBuf>>> = OnceLock::new();
    let map = BUILT.get_or_init(Default::default);
    let mut map = map.lock().unwrap();
    map.entry(bound)
        .or_insert_with(|| {
            let bin = harness::unique_bin(&format!("api_http_public{bound}"));
            let src = PUBLIC.replace("@BOUND@", &bound.to_string());
            build_opts::build_source(&src, &bin, &build_opts::options()).expect("build the Public program");
            bin
        })
        .clone()
}

const DIGEST: &str = "fnv1a64:a8930d6e7998e986";

fn call(port: u16, member: &str, token: &str, body: &str) -> Response {
    request(port, "POST", &format!("/call/{member}"), &[bearer(token), h("Content-Type", "application/json"), h("Hale-Surface-Digest", DIGEST)], Some(body))
}

#[test]
fn the_recorded_http_exchanges_replay_byte_for_byte() {
    let server = Server::start(&public(16), &[]);
    server.ready();
    // refused before anything runs: a token nobody holds, a caller without the
    // role, a payload of the wrong shape, a digest that is not the served one
    for name in ["refusal_unauthenticated", "refusal_unauthorized", "refusal_malformed", "refusal_digest_mismatch"] {
        let rec = recording(name);
        rec.assert_replied(name, &rec.send(server.port));
    }
    // a result, and the handler's own error (order 999 does not exist)
    for name in ["result", "handler_error"] {
        let rec = recording(name);
        rec.assert_replied(name, &rec.send(server.port));
    }
    // the server error of a violating handler, and the next call to that
    // handler's type: unavailable
    for name in ["server_error", "refusal_unavailable"] {
        let rec = recording(name);
        rec.assert_replied(name, &rec.send(server.port));
    }
    let done = server.finish();
    assert!(done.status.success(), "the program ended cleanly: {:?}\n{}\n{}", done.status, done.stdout, done.stderr);
    assert!(done.stdout.contains("stopped"), "stop() returned:\n{}", done.stdout);
}

#[test]
fn a_request_over_the_bound_is_refused_full_as_recorded() {
    // bound 1: a slow call holds the only place, and the next request is
    // refused while it runs
    let server = Server::start(&public(1), &[("SLOW_MS", "600")]);
    server.ready();
    let result = recording("result");
    let slow = send(server.port, &request_text(&result.method, &result.path, &result.headers, Some(&result.body)));
    std::thread::sleep(Duration::from_millis(200));
    let full = recording("refusal_full");
    full.assert_replied("refusal_full", &full.send(server.port));
    let first = slow.response_within(15_000).expect("the slow call is answered");
    result.assert_replied("result", &first);
    assert!(server.finish().status.success());
}

#[test]
fn a_request_after_stop_began_is_refused_shutting_down_as_recorded() {
    // a slow call is executing when stop() begins; a request that arrives
    // while stop() waits for it is refused, the executing call is answered,
    // and then the listener is closed
    let mut server = Server::start(&public(16), &[("SLOW_MS", "800")]);
    server.ready();
    let result = recording("result");
    let slow = send(server.port, &request_text(&result.method, &result.path, &result.headers, Some(&result.body)));
    std::thread::sleep(Duration::from_millis(200));
    server.trigger();
    std::thread::sleep(Duration::from_millis(200));
    let down = recording("refusal_shutting_down");
    down.assert_replied("refusal_shutting_down", &down.send(server.port));
    result.assert_replied("result", &slow.response_within(15_000).expect("the executing call was answered"));
    let done = server.wait(Duration::from_secs(30));
    assert!(done.status.success(), "{:?}\n{}", done.status, done.stderr);
    assert!(std::net::TcpStream::connect(("127.0.0.1", server.port)).is_err(), "the listener is closed once stop() returned");
}

#[test]
fn a_description_over_http_is_the_fixture_for_the_caller() {
    let server = Server::start(&public(16), &[]);
    server.ready();
    for (caller, token) in [("alice", "t-alice"), ("bob", "t-bob")] {
        let got = request(server.port, "GET", "/.description", &[bearer(token)], None);
        assert_eq!(got.status, 200, "{caller}: {}", got.body);
        assert_eq!(got.header("Content-Type"), Some("application/json"));
        let want = description_fixture(&format!("public.{caller}.description.json")).replace("127.0.0.1:8080", &format!("127.0.0.1:{}", server.port));
        assert_eq!(got.body, want, "{caller}: the description differs from the fixture");
    }
    // a caller nobody names is refused, not described
    let nobody = request(server.port, "GET", "/.description", &[bearer("t-mallory")], None);
    assert_eq!(nobody.status, 401, "{}", nobody.body);
    let bare = request(server.port, "GET", "/.description", &[], None);
    assert_eq!(bare.status, 401, "{}", bare.body);
    assert!(bare.body.contains("\"kind\":\"unauthenticated\""), "{}", bare.body);
    assert!(server.finish().status.success());
}

#[test]
fn a_request_that_is_not_a_call_is_malformed() {
    let server = Server::start(&public(16), &[]);
    server.ready();
    let auth = [bearer("t-alice")];
    for (method, path, body) in [("GET", "/call/Orders::place", None), ("POST", "/elsewhere", Some("{}")), ("DELETE", "/.description", None), ("POST", "/call/Orders::place", None)] {
        let got = request(server.port, method, path, &auth, body);
        assert_eq!(got.status, 400, "{method} {path}: {}", got.body);
        assert!(got.body.starts_with("{\"refusal\":{\"kind\":\"malformed\",\"reason\":\""), "{method} {path}: {}", got.body);
    }
    // a member no row names
    let unknown = call(server.port, "Orders::nothing", "t-alice", "{}");
    assert_eq!(unknown.status, 400, "{}", unknown.body);
    assert!(unknown.body.contains("unknown_member: Orders::nothing"), "{}", unknown.body);
    assert!(server.finish().status.success());
}

#[test]
fn a_connection_that_fails_ends_that_connection_and_the_listener_serves_on() {
    let server = Server::start(&public(16), &[]);
    server.ready();
    let place = |tag: &str| {
        let got = call(server.port, "Orders::place", "t-alice", "{\"symbol\":\"ACME\",\"qty\":10,\"limit\":12500}");
        assert_eq!(got.status, 200, "{tag}: {}", got.body);
    };
    place("first");

    // bytes that are not a request, then a close: nothing is answered, and the
    // listener does not mind
    send(server.port, "\u{1}\u{2}this is not http\r\n\r\n").close();
    // a request cut off in the middle of its head
    send(server.port, "POST /call/Orders::place HTTP/1.1\r\nHost: x\r\nAuth").close();
    // a body shorter than its Content-Length, then gone
    send(server.port, "POST /call/Orders::place HTTP/1.1\r\nContent-Length: 500\r\n\r\n{\"sym").close();
    // a client that vanishes right after a complete request
    let rec = recording("result");
    send(server.port, &request_text(&rec.method, &rec.path, &rec.headers, Some(&rec.body))).close();
    // a head over the limit
    let big = format!("GET /.description HTTP/1.1\r\nX-Pad: {}\r\n\r\n", "a".repeat(70000));
    let over = send(server.port, &big).response_within(15_000);
    assert!(over.as_ref().is_none_or(|r| r.status == 400), "a head over the limit is refused or dropped: {over:?}");

    place("after the failures");
    let done = server.finish();
    assert!(done.status.success(), "{:?}\n{}", done.status, done.stderr);
}

#[test]
fn a_connection_that_resets_does_not_fail_a_read_in_progress_on_another() {
    // Two connections of one listener share its pool, each parked in a read
    // of a request head. One is reset (RST), failing its read; the other,
    // half through its request, then finishes it and must be answered.
    let server = Server::start(&public(16), &[]);
    server.ready();
    let rec = recording("result");
    let text = request_text(&rec.method, &rec.path, &rec.headers, Some(&rec.body));
    let (head, tail) = text.split_at(text.len() / 2);
    for round in 0..6 {
        let mut waiting = send(server.port, head);
        let vanishing = send(server.port, "POST /call/Orders::place HTTP/1.1\r\nHost: x\r\nAuth");
        std::thread::sleep(Duration::from_millis(80));
        vanishing.reset();
        std::thread::sleep(Duration::from_millis(80));
        waiting.more(tail);
        let got = waiting.response_within(15_000).unwrap_or_else(|| panic!("round {round}: closed without a response"));
        assert_eq!(got.status, rec.status, "round {round}: the connection that was mid-read is served: {}", got.body);
    }
    let done = server.finish();
    assert!(done.status.success(), "{:?}\n{}", done.status, done.stderr);
}

#[test]
fn a_listener_that_cannot_bind_fails_the_boot() {
    // a port something holds
    let held = std::net::TcpListener::bind("127.0.0.1:0").expect("hold a port");
    let port = held.local_addr().unwrap().port();
    let bin = public(16);
    let mut server = Server::start(&bin, &[("BIND", &format!("127.0.0.1:{port}"))]);
    let done = server.wait(Duration::from_secs(30));
    assert_eq!(done.status.code(), Some(2), "the boot fails: {:?}\n{}\n{}", done.status, done.stdout, done.stderr);
    assert!(done.stderr.contains("api: http::Rpc could not listen on 127.0.0.1:"), "the diagnostic:\n{}", done.stderr);
    drop(held);

    // and an address that is not host:port
    let mut server = Server::start(&bin, &[("BIND", "not-an-address")]);
    let done = server.wait(Duration::from_secs(30));
    assert_eq!(done.status.code(), Some(2), "{:?}\n{}", done.status, done.stderr);
}

/// A serving locus whose receiver is built where the locus is constructed
/// (`Desk { echoer: Echoer { base: 100 } }`, not in the param's default),
/// and whose listener address is one of its own params.
const CONSTRUCTED: &str = r#"
type Ping { n: Int; }
type Pong { n: Int; }
api Echo { rpc Echoer::echo; }

locus Tokens {
    fn principal(token: String) -> std::api::Principal {
        if token == "t-alice" { return std::api::Principal { mode: "bearer", name: "alice" }; }
        return std::api::Principal { mode: "bearer", name: "" };
    }
    fn refused() -> String { return "no such token"; }
}

locus Nobody {
    fn holds(p: std::api::Principal, r: String) -> Bool { return false; }
}

locus Echoer {
    params { base: Int = 0; }
    fn echo(p: Ping) -> Pong { return Pong { n: self.base + p.n }; }
}

main locus Desk {
    params {
        bearer: Tokens = Tokens { };
        roles: Nobody = Nobody { };
        echoer: Echoer = Echoer { };
        bind: String = "";
    }
    run() {
        let h = api::serve(Echo, http::Rpc { bind: self.bind, codec: json, principals: self.bearer, roles: self.roles }, as: "echo", bound: 4, on_full: refuse);
        let trigger = std::env::var("TRIGGER");
        while !self.draining && (std::io::fs::read_file(trigger) or "") == "" {
            std::time::sleep(10ms);
        }
        h.stop();
    }
}

fn main() { Desk { echoer: Echoer { base: 100 }, bind: std::env::var("BIND") }; }
"#;

#[test]
fn a_receiver_built_where_the_serving_locus_is_constructed_serves_and_the_address_may_be_a_param() {
    let bin = harness::unique_bin("api_http_constructed");
    build_opts::build_source(CONSTRUCTED, &bin, &build_opts::options()).expect("build the program");
    let server = Server::start(&bin, &[]);
    server.ready();
    let got = request(server.port, "POST", "/call/Echoer::echo", &[bearer("t-alice")], Some("{\"n\":5}"));
    assert_eq!((got.status, got.body.as_str()), (200, "{\"n\":105}"), "the instance the literal at the construction site builds answers");
    assert!(server.finish().status.success());
}

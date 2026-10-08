//! GH #1417 (R7): `grpc::Rpc`, the fourth socket transport of `api::serve`.
//!
//! The witness's `Public` half is the program of `api_http.rs` with its
//! transport swapped for `grpc::Rpc`. These tests build it, serve it on a
//! loopback port and speak gRPC to it with the hand-written HTTP/2 client of
//! `support/h2_client.rs`, one unary call to a stream:
//!
//! * the recorded exchanges of `tests/api-contract/wire/http/` are replayed:
//!   each request becomes `POST /Public/<member>` with the recorded body as
//!   its one message, and each reply is held to the recorded one, byte for
//!   byte, in gRPC's terms (spec/api.md § Outcomes, gRPC): the recorded body
//!   is the response message, or the object in `grpc-status-details-bin`;
//! * the description is the reserved method `hale.api.Description/Describe`;
//! * many calls share one connection; a stream or a connection that breaks
//!   leaves the others and the listener alone;
//! * the lifecycle rows a stream has: the bound, the stop, a client that goes
//!   away after its call started;
//! * a listener that cannot bind fails the boot.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

#[path = "support/build.rs"]
mod build_opts;
#[path = "support/h2_client.rs"]
pub(crate) mod h2_client;
#[path = "support/harness.rs"]
mod harness;
#[path = "support/http_rpc.rs"]
mod http_rpc;
#[path = "support/ports.rs"]
mod ports;

use h2_client::*;
use http_rpc::*;

const DIGEST: &str = "fnv1a64:a8930d6e7998e986";
const WAIT: Duration = Duration::from_secs(15);

/// The `Public` program of `api_http.rs`, served over `grpc::Rpc`.
fn source(bound: u32) -> String {
    let grpc = super::api_http::PUBLIC.replace("http::Rpc { bind: std::env::var(\"BIND\"), codec: json,", "grpc::Rpc { bind: std::env::var(\"BIND\"), codec: json,");
    assert_ne!(grpc, super::api_http::PUBLIC, "the transport was swapped");
    grpc.replace("@BOUND@", &bound.to_string())
}

/// The program with `bound` requests at a time, built once per process.
fn public(bound: u32) -> PathBuf {
    static BUILT: OnceLock<std::sync::Mutex<std::collections::BTreeMap<u32, PathBuf>>> = OnceLock::new();
    let map = BUILT.get_or_init(Default::default);
    let mut map = map.lock().unwrap();
    map.entry(bound)
        .or_insert_with(|| {
            let bin = harness::unique_bin(&format!("api_grpc_public{bound}"));
            build_opts::build_source(&source(bound), &bin, &build_opts::options()).expect("build the Public program over grpc::Rpc");
            bin
        })
        .clone()
}

// ---- gRPC's wire, written out ----

fn b64(bytes: &[u8]) -> String {
    const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        out.push(A[(n >> 18) as usize & 63] as char);
        out.push(A[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { A[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { A[n as usize & 63] as char } else { '=' });
    }
    out
}

fn varint(mut n: usize) -> Vec<u8> {
    let mut out = Vec::new();
    while n >= 128 {
        out.push((n % 128) as u8 | 0x80);
        n /= 128;
    }
    out.push(n as u8);
    out
}

fn field(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    out.extend(varint(body.len()));
    out.extend_from_slice(body);
    out
}

/// `google.rpc.Status { code, message, details: [Any { type_url, value }] }`.
fn status_details(code: usize, message: &str, type_url: &str, value: &str) -> String {
    let mut any = field(0x0a, type_url.as_bytes());
    any.extend(field(0x12, value.as_bytes()));
    let mut status = vec![0x08];
    status.extend(varint(code));
    status.extend(field(0x12, message.as_bytes()));
    status.extend(field(0x1a, &any));
    b64(&status)
}

fn pct(s: &str) -> String {
    s.bytes().map(|b| if (32..=126).contains(&b) && b != b'%' { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// A gRPC message: the compression flag, the length, the bytes.
fn message(text: &str) -> Vec<u8> {
    let mut m = vec![0];
    m.extend((text.len() as u32).to_be_bytes());
    m.extend_from_slice(text.as_bytes());
    m
}

fn pair(k: &str, v: &str) -> (String, String) {
    (k.to_string(), v.to_string())
}

/// The status of a refusal kind, by the spec's table.
fn code_of(kind: &str) -> usize {
    match kind {
        "malformed" => 3,
        "digest_mismatch" => 9,
        "unauthenticated" => 16,
        "unauthorized" => 7,
        "full" => 8,
        "shutting_down" | "unavailable" => 14,
        _ => 13,
    }
}

/// What a recorded HTTP reply is as a gRPC response, written from the
/// spec's column and nothing else of the implementation: the headers, the
/// data, the trailers.
fn expected(rec: &Recording, ctype: &str) -> (Vec<(String, String)>, Vec<u8>, Vec<(String, String)>) {
    let head = vec![pair(":status", "200"), pair("content-type", ctype)];
    if rec.status == 200 {
        return (head, message(&rec.reply), vec![pair("grpc-status", "0")]);
    }
    let (code, text, type_url, value) = if rec.status == 422 {
        (9, "handler_error".to_string(), "type.hale.dev/hale.api.HandlerError", rec.reply.clone())
    } else {
        let inner = rec.reply.strip_prefix("{\"refusal\":").and_then(|r| r.strip_suffix('}')).expect("a refusal body").to_string();
        let kind = inner.split("\"kind\":\"").nth(1).and_then(|r| r.split('"').next()).expect("a kind").to_string();
        let reason = inner.split("\"reason\":\"").nth(1).and_then(|r| r.split("\",").next().or(r.split("\"}").next())).map(|r| r.trim_end_matches("\"}").to_string());
        let text = if kind == "server" { "server_error".to_string() } else { reason.expect("a reason") };
        (code_of(&kind), text, "type.hale.dev/hale.api.Refusal", inner)
    };
    let mut trailers_only = head;
    trailers_only.push(pair("grpc-status", &code.to_string()));
    trailers_only.push(pair("grpc-message", &pct(&text)));
    trailers_only.push(pair("grpc-status-details-bin", &status_details(code, &text, type_url, &value)));
    (trailers_only, Vec::new(), Vec::new())
}

/// The recorded request's member as a gRPC method: `Orders.place`.
fn method_of(rec: &Recording) -> String {
    rec.path.strip_prefix("/call/").expect("a call path").replace("::", ".")
}

fn token_of(rec: &Recording) -> Option<String> {
    rec.headers.iter().find(|(k, _)| k == "Authorization").and_then(|(_, v)| v.strip_prefix("Bearer ")).map(str::to_string)
}

fn digest_of(rec: &Recording) -> Option<String> {
    rec.headers.iter().find(|(k, _)| k == "Hale-Surface-Digest").map(|(_, v)| v.clone())
}

/// One unary call: HEADERS and one message, on `stream`.
fn call(c: &mut Client, stream: u32, path: &str, token: Option<&str>, digest: Option<&str>, ctype: &str, msg: &[u8]) {
    let auth = token.map(|t| format!("Bearer {t}"));
    let mut hs: Vec<(&str, &str)> = vec![(":method", "POST"), (":scheme", "http"), (":path", path), (":authority", "localhost"), ("content-type", ctype), ("te", "trailers")];
    if let Some(a) = auth.as_deref() {
        hs.push(("authorization", a));
    }
    if let Some(d) = digest {
        hs.push(("hale-surface-digest", d));
    }
    c.headers(stream, &hs, false);
    c.data(stream, msg, true);
}

const JSON: &str = "application/grpc+json";

/// Send the recorded request as a gRPC call.
fn replay(c: &mut Client, stream: u32, rec: &Recording) {
    call(c, stream, &format!("/Public/{}", method_of(rec)), token_of(rec).as_deref(), digest_of(rec).as_deref(), JSON, &message(&rec.body));
}

/// Hold the response on `stream` to the recorded exchange.
fn assert_replayed(c: &mut Client, stream: u32, name: &str, rec: &Recording) {
    let got = c.response(stream, WAIT);
    let (headers, data, trailers) = expected(rec, JSON);
    assert_eq!(got.headers, headers, "{name}: the headers\n{got:?}");
    assert_eq!(got.data, data, "{name}: the message differs from the recorded body\n{got:?}");
    assert_eq!(got.trailers, trailers, "{name}: the trailers\n{got:?}");
    assert!(got.ended && got.reset.is_none(), "{name}: {got:?}");
}

fn start(bound: u32, env: &[(&str, &str)]) -> Server {
    let server = Server::start(&public(bound), env);
    server.ready();
    server
}

#[test]
fn the_recorded_exchanges_replay_byte_for_byte_over_grpc() {
    let server = start(16, &[]);
    let mut c = Client::connect(server.port);
    // every outcome in order, each on its own stream of one connection: the
    // refusals before anything runs, a result, the handler's own error, the
    // server error of a violating handler and the unavailable that follows it
    let names = [
        "refusal_unauthenticated",
        "refusal_unauthorized",
        "refusal_malformed",
        "refusal_digest_mismatch",
        "result",
        "handler_error",
        "server_error",
        "refusal_unavailable",
    ];
    for (i, name) in names.iter().enumerate() {
        let rec = recording(name);
        let stream = 2 * i as u32 + 1;
        replay(&mut c, stream, &rec);
        assert_replayed(&mut c, stream, name, &rec);
    }
    let done = server.finish();
    assert!(done.status.success(), "the program ended cleanly: {:?}\n{}\n{}", done.status, done.stdout, done.stderr);
    assert!(done.stdout.contains("stopped"), "stop() returned:\n{}", done.stdout);
}

#[test]
fn a_request_over_the_bound_is_refused_full_as_recorded() {
    // bound 1: a slow call holds the only place, and the next request is
    // refused while it runs
    let server = start(1, &[("SLOW_MS", "600")]);
    let mut c = Client::connect(server.port);
    let result = recording("result");
    replay(&mut c, 1, &result);
    std::thread::sleep(Duration::from_millis(200));
    let full = recording("refusal_full");
    replay(&mut c, 3, &full);
    assert_replayed(&mut c, 3, "refusal_full", &full);
    assert_replayed(&mut c, 1, "result", &result);
    assert!(server.finish().status.success());
}

#[test]
fn a_request_after_stop_began_is_refused_shutting_down_and_the_connection_says_goaway() {
    // a slow call is executing when stop() begins; a request that arrives
    // while stop() waits for it is refused, the executing call is answered,
    // and then the connection says GOAWAY and the listener is closed
    let mut server = start(16, &[("SLOW_MS", "800")]);
    let mut c = Client::connect(server.port);
    let result = recording("result");
    replay(&mut c, 1, &result);
    std::thread::sleep(Duration::from_millis(200));
    server.trigger();
    std::thread::sleep(Duration::from_millis(200));
    let down = recording("refusal_shutting_down");
    let mut late = Client::connect(server.port);
    replay(&mut late, 1, &down);
    assert_replayed(&mut late, 1, "refusal_shutting_down", &down);
    assert_replayed(&mut c, 1, "result", &result);
    let go = c.goaway_within(WAIT).expect("stop says GOAWAY after the queued replies");
    assert_eq!(go.1, 0, "NO_ERROR");
    assert!(c.closed(WAIT), "the connection ends");
    let done = server.wait(Duration::from_secs(30));
    assert!(done.status.success(), "{:?}\n{}", done.status, done.stderr);
    assert!(std::net::TcpStream::connect(("127.0.0.1", server.port)).is_err(), "the listener is closed once stop() returned");
}

/// The outcome encoding a gRPC exposure's description states.
const GRPC_OUTCOMES: &str = "{\"transport\":\"grpc\",\"result\":{\"status\":\"OK\",\"body\":\"response\"},\"handler_error\":{\"status\":\"FAILED_PRECONDITION\",\"details\":\"error\"},\"refusal\":{\"details\":\"refusal\",\"status\":{\"malformed\":\"INVALID_ARGUMENT\",\"digest_mismatch\":\"FAILED_PRECONDITION\",\"unauthenticated\":\"UNAUTHENTICATED\",\"unauthorized\":\"PERMISSION_DENIED\",\"full\":\"RESOURCE_EXHAUSTED\",\"shutting_down\":\"UNAVAILABLE\",\"unavailable\":\"UNAVAILABLE\"}},\"server_error\":{\"status\":\"INTERNAL\",\"details\":\"refusal\"},\"transport_failure\":\"the transport's own: the stream is reset or the connection ends without a status\"}";

/// A description fixture of an HTTP exposure as the same exposure served over
/// gRPC at `port` states it: the listener and the outcome encoding are the
/// transport's; everything else is the fixture's.
pub(crate) fn description_over_grpc(fixture: &str, port: u16, name: &str) -> String {
    let doc = description_fixture(fixture);
    let http = "\"listener\":{\"transport\":\"http\",\"address\":\"127.0.0.1:8080\"}";
    assert!(doc.contains(http), "{fixture} states its HTTP listener");
    let doc = doc.replace(http, &format!("\"listener\":{{\"transport\":\"grpc\",\"address\":\"127.0.0.1:{port}\"}}"));
    let doc = doc.replace("/public\",\"name\":\"public\"", &format!("/{name}\",\"name\":\"{name}\""));
    let at = doc.find("\"outcomes\":").expect("outcomes") + "\"outcomes\":".len();
    let mut depth = 0;
    let mut end = at;
    for (i, c) in doc[at..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = at + i + 1;
                    break;
                }
            }
            _ => {}
        }
    }
    format!("{}{}{}", &doc[..at], GRPC_OUTCOMES, &doc[end..])
}

/// One unary call as a response: the call's headers and message on `stream`.
pub(crate) fn unary(c: &mut Client, stream: u32, path: &str, token: Option<&str>, msg: &str) -> h2_client::Response {
    call(c, stream, path, token, Some(DIGEST), JSON, &message(msg));
    c.response(stream, WAIT)
}

/// A response's message, without its prefix.
pub(crate) fn text_of(r: &h2_client::Response) -> String {
    assert!(r.data.len() >= 5, "a message: {r:?}");
    String::from_utf8(r.data[5..].to_vec()).expect("utf-8")
}

const PLACE: &str = "{\"symbol\":\"ACME\",\"qty\":10,\"limit\":12500}";

#[test]
fn the_description_is_a_reserved_method_and_is_the_fixture_for_the_caller() {
    let server = start(16, &[]);
    let mut c = Client::connect(server.port);
    for (i, (caller, token)) in [("alice", "t-alice"), ("bob", "t-bob")].iter().enumerate() {
        let stream = 2 * i as u32 + 1;
        call(&mut c, stream, "/hale.api.Description/Describe", Some(token), None, JSON, &message("{}"));
        let got = c.response(stream, WAIT);
        assert_eq!(got.status(), 200, "{caller}: {got:?}");
        assert_eq!(got.header("content-type"), Some(JSON));
        assert_eq!(got.header("grpc-status"), Some("0"), "{caller}: {got:?}");
        assert_eq!(text_of(&got), description_over_grpc(&format!("public.{caller}.description.json"), server.port, "public"), "{caller}: the description differs from the fixture");
    }
    // a caller nobody names is refused, not described
    for (stream, token) in [(5u32, Some("t-mallory")), (7, None)] {
        call(&mut c, stream, "/hale.api.Description/Describe", token, None, JSON, &message("{}"));
        let got = c.response(stream, WAIT);
        assert_eq!(got.header("grpc-status"), Some("16"), "{got:?}");
        assert!(got.data.is_empty(), "a refusal has no message: {got:?}");
    }
    assert!(server.finish().status.success());
}

#[test]
fn a_request_that_is_not_a_call_is_malformed() {
    let server = start(16, &[]);
    let mut c = Client::connect(server.port);
    let stream = std::cell::Cell::new(1u32);
    let malformed = |c: &mut Client, why: &str, path: &str, ctype: &str, msg: &[u8], extra: &[(&str, &str)], reason: &str| {
        let mut hs: Vec<(&str, &str)> = vec![(":method", "POST"), (":scheme", "http"), (":path", path), (":authority", "localhost"), ("content-type", ctype), ("authorization", "Bearer t-alice")];
        hs.extend_from_slice(extra);
        c.headers(stream.get(), &hs, false);
        c.data(stream.get(), msg, true);
        let got = c.response(stream.get(), WAIT);
        assert_eq!(got.status(), 200, "{why}: {got:?}");
        assert_eq!(got.header("grpc-status"), Some("3"), "{why}: INVALID_ARGUMENT: {got:?}");
        let said = got.header("grpc-message").unwrap_or("").to_string();
        assert!(said.contains(&pct(reason)), "{why}: the reason `{reason}` in `{said}`");
        assert!(got.data.is_empty() && got.ended, "{why}: {got:?}");
        stream.set(stream.get() + 2);
    };
    let ok = message(PLACE);
    malformed(&mut c, "a content type that is not gRPC", "/Public/Orders.place", "text/plain", &ok, &[], "a gRPC call has content-type application/grpc");
    malformed(&mut c, "protobuf is not served", "/Public/Orders.place", "application/grpc+proto", &ok, &[], "content-type application/grpc+proto is not served: the codec is json");
    malformed(&mut c, "compression", "/Public/Orders.place", JSON, &ok, &[("grpc-encoding", "gzip")], "compression is not supported");
    let mut flagged = ok.clone();
    flagged[0] = 1;
    malformed(&mut c, "a compressed message", "/Public/Orders.place", JSON, &flagged, &[], "compressed messages are not supported");
    let mut two = ok.clone();
    two.extend_from_slice(&ok);
    malformed(&mut c, "two messages", "/Public/Orders.place", JSON, &two, &[], "a unary call carries exactly one message");
    malformed(&mut c, "a short prefix", "/Public/Orders.place", JSON, &[0, 0, 0], &[], "a call carries a message with its five-byte prefix");
    malformed(&mut c, "an empty message", "/Public/Orders.place", JSON, &message(""), &[], "a call carries a message");
    malformed(&mut c, "a message that is not text", "/Public/Orders.place", JSON, &[0, 0, 0, 0, 3, b'{', 0, b'}'], &[], "the message is not json text");
    malformed(&mut c, "another service", "/Admin/Orders.place", JSON, &ok, &[], "no such service Admin: this exposure serves Public");
    malformed(&mut c, "no member", "/Public/Orders.nothing", JSON, &ok, &[], "unknown_member: Orders::nothing");
    malformed(&mut c, "no method", "/Public", JSON, &ok, &[], "a gRPC call is POST /<Surface>/<member>");
    malformed(&mut c, "a payload of the wrong shape", "/Public/Orders.place", JSON, &message("{\"symbol\":\"ACME\",\"limit\":12500}"), &[], "missing_field: qty");
    // a message past the limit, declared by its prefix
    let mut huge = vec![0];
    huge.extend(2_000_000u32.to_be_bytes());
    huge.extend(vec![b' '; 100]);
    malformed(&mut c, "a message past the limit", "/Public/Orders.place", JSON, &huge, &[], "a request message is at most 1048576 bytes");
    // ... and one actually sent past it, over many DATA frames
    let big = message(&format!("{{\"symbol\":\"{}\",\"qty\":1,\"limit\":1}}", "A".repeat(1_100_000)));
    malformed(&mut c, "a body past the limit", "/Public/Orders.place", JSON, &big, &[], "a request message is at most 1048576 bytes");
    // a GET is not a call
    c.headers(stream.get(), &[(":method", "GET"), (":scheme", "http"), (":path", "/Public/Orders.place"), (":authority", "localhost"), ("content-type", JSON), ("authorization", "Bearer t-alice")], true);
    let got = c.response(stream.get(), WAIT);
    assert_eq!(got.header("grpc-status"), Some("3"), "{got:?}");
    assert!(got.header("grpc-message").unwrap_or("").contains("a%20gRPC%20call%20is%20a%20POST") || got.header("grpc-message").unwrap_or("").contains("a gRPC call is a POST"), "{got:?}");
    stream.set(stream.get() + 2);
    // the connection served all of that and serves on; a member may be spelled with its `::`
    call(&mut c, stream.get(), "/Public/Orders::place", Some("t-alice"), Some(DIGEST), "application/grpc", &ok);
    let fine = c.response(stream.get(), WAIT);
    assert_eq!(fine.header("grpc-status"), Some("0"), "{fine:?}");
    assert_eq!(fine.header("content-type"), Some("application/grpc"), "a bare application/grpc is answered as it was asked");
    assert_eq!(text_of(&fine), "{\"order\":41,\"notional\":125000}");
    assert!(server.finish().status.success());
}

#[test]
fn many_calls_share_a_connection_and_a_broken_stream_leaves_the_others() {
    let server = start(64, &[]);
    let mut c = Client::connect(server.port);
    // eight calls in flight on one connection before any answer is read
    for i in 0..8u32 {
        call(&mut c, 2 * i + 1, "/Public/Orders.place", Some("t-alice"), Some(DIGEST), JSON, &message(PLACE));
    }
    let mut orders = Vec::new();
    for i in 0..8u32 {
        let r = c.response(2 * i + 1, WAIT);
        assert_eq!(r.header("grpc-status"), Some("0"), "stream {}: {r:?}", 2 * i + 1);
        orders.push(text_of(&r));
    }
    orders.sort();
    let want: Vec<String> = (41..49).map(|n| format!("{{\"order\":{n},\"notional\":125000}}")).collect();
    assert_eq!(orders, want, "eight calls, eight distinct orders");

    // a stream the client abandons half way (HEADERS, some DATA, then a reset)
    c.headers(17, &[(":method", "POST"), (":scheme", "http"), (":path", "/Public/Orders.place"), (":authority", "localhost"), ("content-type", JSON), ("authorization", "Bearer t-alice")], false);
    c.data(17, &message(PLACE)[..10], false);
    c.reset(17, 8);
    // a stream that sent its headers and nothing more, then went away
    c.headers(19, &[(":method", "POST"), (":scheme", "http"), (":path", "/Public/Orders.place"), (":authority", "localhost"), ("content-type", JSON)], false);
    c.reset(19, 8);
    // and a stream that is a call, beside them
    call(&mut c, 21, "/Public/Orders.place", Some("t-alice"), Some(DIGEST), JSON, &message(PLACE));
    let r = c.response(21, WAIT);
    assert_eq!(text_of(&r), "{\"order\":49,\"notional\":125000}", "{r:?}");

    // a connection that fails (a frame the protocol forbids) fails alone
    let mut bad = Client::connect(server.port);
    bad.frame(DATA, 0, 0, b"nope");
    assert_eq!(bad.goaway_within(WAIT).map(|g| g.1), Some(1), "PROTOCOL_ERROR");
    assert!(bad.closed(WAIT));
    // another that vanishes in the middle of a call
    {
        let mut gone = Client::connect(server.port);
        gone.headers(1, &[(":method", "POST"), (":scheme", "http"), (":path", "/Public/Orders.place"), (":authority", "localhost"), ("content-type", JSON), ("authorization", "Bearer t-alice")], false);
        gone.data(1, &message(PLACE)[..7], false);
    }
    std::thread::sleep(Duration::from_millis(150));
    call(&mut c, 23, "/Public/Orders.place", Some("t-alice"), Some(DIGEST), JSON, &message(PLACE));
    let r = c.response(23, WAIT);
    assert_eq!(text_of(&r), "{\"order\":50,\"notional\":125000}", "the first connection served through all of it: {r:?}");
    let mut fresh = Client::connect(server.port);
    call(&mut fresh, 1, "/Public/Orders.place", Some("t-alice"), Some(DIGEST), JSON, &message(PLACE));
    let r = fresh.response(1, WAIT);
    assert_eq!(text_of(&r), "{\"order\":51,\"notional\":125000}");

    // A peer that resets its connection fails a read on the pool it shares
    // with the idle first connection: that connection's next read must not
    // see the failure (the runtime keeps one `last_io_status` for the
    // pool's coroutines, and a read that asks it afterwards can get
    // another connection's answer; the connection reads raw).
    let mut stream = 25u32;
    for round in 0..6i64 {
        {
            // closes with the server's SETTINGS unread: the kernel resets
            let mut vanishing = Client::connect(server.port);
            vanishing.headers(1, &[(":method", "POST"), (":scheme", "http"), (":path", "/Public/Orders.place"), (":authority", "localhost"), ("content-type", JSON)], false);
        }
        std::thread::sleep(Duration::from_millis(60));
        call(&mut c, stream, "/Public/Orders.place", Some("t-alice"), Some(DIGEST), JSON, &message(PLACE));
        let r = c.response(stream, WAIT);
        assert_eq!(text_of(&r), format!("{{\"order\":{},\"notional\":125000}}", 52 + round), "round {round}: the idle connection serves: {r:?}");
        stream += 2;
    }
    assert!(server.finish().status.success());
}

fn rss_kb(pid: u32) -> u64 {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).expect("status");
    status.lines().find_map(|l| l.strip_prefix("VmRSS:")).and_then(|v| v.trim().trim_end_matches("kB").trim().parse().ok()).expect("VmRSS")
}

/// The `Public` program with a receipt that carries a megabyte: `memo`, `n`
/// bytes of the alphabet starting `id % 26` letters in, so each call's
/// answer says whose it is. (The position limit is out of the way.)
fn big_source() -> String {
    let mut s = source(64);
    for (from, to) in [
        ("type OrderReceipt { order: OrderId; notional: Money; }", "type OrderReceipt { order: OrderId; notional: Money; memo: String; }"),
        ("if o.qty > 10000 { violate position_limit; }", "if o.qty > 100000000 { violate position_limit; }"),
        ("return OrderReceipt { order: id, notional: o.limit * o.qty };", "return OrderReceipt { order: id, notional: o.limit * o.qty, memo: memo(Int(id), o.qty) };"),
        (
            "main locus Desk {",
            "@unbounded\nfn memo(id: Int, n: Int) -> String {\n    let r = id % 26;\n    let a = \"abcdefghijklmnopqrstuvwxyz\";\n    let mut s = a[r..26] + a[0..r];\n    while len(s) < n { s = s + s; }\n    return s[0..n];\n}\n\nmain locus Desk {",
        ),
    ] {
        assert!(s.contains(from), "{from}");
        s = s.replacen(from, to, 1);
    }
    s
}

fn big() -> PathBuf {
    static BUILT: OnceLock<PathBuf> = OnceLock::new();
    BUILT
        .get_or_init(|| {
            let bin = harness::unique_bin("api_grpc_big");
            build_opts::build_source(&big_source(), &bin, &build_opts::options()).expect("build the Public program with a large receipt");
            bin
        })
        .clone()
}

#[test]
fn large_answers_to_calls_in_flight_share_a_connection_without_interleaving() {
    // Eight calls at once, each answered with a megabyte (eight in all,
    // several times what the socket's buffers hold), to a client that reads
    // nothing for a while and then in small pieces, with PINGs while the
    // server's writes are parked. The connection has one writer: every
    // message arrives whole and its own.
    const MEMO: usize = 1_000_000;
    let server = Server::start(&big(), &[]);
    server.ready();
    let mut c = Client::connect(server.port);
    let streams: Vec<u32> = (0..8).map(|i| 2 * i + 1).collect();
    let big_place = "{\"symbol\":\"ACME\",\"qty\":1000000,\"limit\":1}";
    for s in &streams {
        call(&mut c, *s, "/Public/Orders.place", Some("t-alice"), None, JSON, &message(big_place));
    }
    std::thread::sleep(Duration::from_millis(500));
    c.ping(*b"parked!!");
    std::thread::sleep(Duration::from_millis(200));
    c.ping(*b"parked2!");
    c.slowly(&streams, Duration::from_secs(90), 8192, Duration::from_millis(1));
    let mut orders = Vec::new();
    for s in &streams {
        let r = c.streams.get(s).cloned().unwrap_or_default();
        assert_eq!(r.header("grpc-status"), Some("0"), "stream {s}: {} bytes of data, ended={}", r.data.len(), r.ended);
        let text = text_of(&r);
        let order: usize = text.strip_prefix("{\"order\":").and_then(|t| t.split(',').next()).and_then(|n| n.parse().ok()).unwrap_or_else(|| panic!("stream {s}: {}", &text[..text.len().min(80)]));
        let memo = text.split("\"memo\":\"").nth(1).and_then(|m| m.strip_suffix("\"}")).expect("a memo");
        assert_eq!(memo.len(), MEMO, "stream {s}");
        let whole = memo.bytes().enumerate().all(|(i, b)| b == b'a' + ((i + order) % 26) as u8);
        assert!(whole, "stream {s} (order {order}): a byte is not where its call put it");
        orders.push(order);
    }
    orders.sort();
    assert_eq!(orders, (41..49).collect::<Vec<_>>(), "eight calls, eight distinct orders");
    c.idle(Duration::from_millis(200));
    assert_eq!(c.pongs, 2, "both PINGs were acknowledged");
    assert!(c.goaway.is_none() && !c.eof, "the connection is whole");
    assert!(server.finish().status.success());
}

#[test]
fn a_half_close_while_the_writer_is_parked_ends_the_connection() {
    // Eight calls answered with a megabyte each, to a client that reads
    // nothing: the connection's writer parks in a send. The client half-closes
    // its write side, its read side open and unread; the server's connection
    // ends in bounded time and gives its descriptor back, and another
    // connection of the listener still answers.
    let server = Server::start(&big(), &[]);
    server.ready();
    let fds = |pid: u32| std::fs::read_dir(format!("/proc/{pid}/fd")).expect("fd dir").count();
    let mut b = Client::connect(server.port);
    let place = "{\"symbol\":\"ACME\",\"qty\":1,\"limit\":1}";
    call(&mut b, 1, "/Public/Orders.place", Some("t-alice"), None, JSON, &message(place));
    assert_eq!(b.response(1, Duration::from_secs(10)).header("grpc-status"), Some("0"));
    let held = fds(server.pid());
    let mut c = Client::connect(server.port);
    assert_eq!(fds(server.pid()), held + 1, "the connection holds a descriptor");
    let big_place = "{\"symbol\":\"ACME\",\"qty\":1000000,\"limit\":1}";
    for s in (0..8).map(|i| 2 * i + 1) {
        call(&mut c, s, "/Public/Orders.place", Some("t-alice"), None, JSON, &message(big_place));
    }
    std::thread::sleep(Duration::from_millis(700));
    c.half_close();
    let start = std::time::Instant::now();
    while fds(server.pid()) != held {
        assert!(start.elapsed() < Duration::from_secs(10), "the descriptor of the half-closed connection is released");
        std::thread::sleep(Duration::from_millis(20));
    }
    call(&mut b, 3, "/Public/Orders.place", Some("t-alice"), None, JSON, &message(place));
    assert_eq!(b.response(3, Duration::from_secs(10)).header("grpc-status"), Some("0"), "the other connection is unaffected");
    assert!(server.finish().status.success());
    drop(c);
}

#[test]
fn a_long_lived_connection_does_not_grow_with_the_calls_it_carries() {
    // One connection, call after call. Its read once allocated a blob out
    // of an arena that outlives the read (about 4 KiB of resident memory a
    // call, and the process's capped payload arena in the end); the
    // connection now reads into one buffer, and what a call costs the
    // process is what an HTTP call costs it (a few hundred bytes a call, the
    // runtime's own bookkeeping), not a page.
    let server = start(64, &[]);
    let mut c = Client::connect(server.port);
    let mut stream = 1u32;
    let mut run = |c: &mut Client, n: u32| {
        for _ in 0..n {
            call(c, stream, "/Public/Orders.cancel", Some("t-alice"), Some(DIGEST), JSON, &message("{\"order\":99999}"));
            let r = c.response(stream, WAIT);
            assert_eq!(r.header("grpc-status"), Some("9"), "{r:?}");
            stream += 2;
        }
    };
    run(&mut c, 100);
    let before = rss_kb(server.pid());
    run(&mut c, 400);
    let after = rss_kb(server.pid());
    let per_call = after.saturating_sub(before) * 1024 / 400;
    assert!(per_call < 2048, "a call costs the process {per_call} bytes ({before} kB -> {after} kB over 400 calls)");
    assert!(server.finish().status.success());
}

#[test]
fn a_client_that_goes_away_after_its_call_started_leaves_the_work_to_finish() {
    // bound 1: the call that was started holds the exposure's only place
    // until the handler returns, though nobody is waiting for it. Once by a
    // stream reset, once by a connection that closes.
    for by_reset in [true, false] {
        let server = start(1, &[("SLOW_MS", "700")]);
        let mut a = Client::connect(server.port);
        call(&mut a, 1, "/Public/Orders.place", Some("t-alice"), Some(DIGEST), JSON, &message(PLACE));
        std::thread::sleep(Duration::from_millis(250));
        if by_reset {
            a.reset(1, 8);
            a.idle(Duration::from_millis(100));
        } else {
            drop(a);
            std::thread::sleep(Duration::from_millis(100));
        }
        let mut b = Client::connect(server.port);
        call(&mut b, 1, "/Public/Orders.place", Some("t-alice"), Some(DIGEST), JSON, &message(PLACE));
        let full = b.response(1, WAIT);
        assert_eq!(full.header("grpc-status"), Some("8"), "RESOURCE_EXHAUSTED: the lost call still holds its place: {full:?}");
        // once the handler returns, the place is released, once
        let start = std::time::Instant::now();
        loop {
            let id = 3 + 2 * (start.elapsed().as_millis() as u32 / 50);
            call(&mut b, id, "/Public/Orders.place", Some("t-alice"), Some(DIGEST), JSON, &message(PLACE));
            let again = b.response(id, WAIT);
            if again.header("grpc-status") == Some("0") {
                // the lost call ran and took order 41; this one is the next
                assert_eq!(text_of(&again), "{\"order\":42,\"notional\":125000}", "the work ran once");
                break;
            }
            assert_eq!(again.header("grpc-status"), Some("8"), "{again:?}");
            assert!(start.elapsed() < Duration::from_secs(10), "the place was never released");
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(server.finish().status.success());
    }
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
    assert!(done.stderr.contains("api: grpc::Rpc could not listen on 127.0.0.1:"), "the diagnostic:\n{}", done.stderr);
    drop(held);

    // and an address that is not host:port
    let mut server = Server::start(&bin, &[("BIND", "not-an-address")]);
    let done = server.wait(Duration::from_secs(30));
    assert_eq!(done.status.code(), Some(2), "{:?}\n{}", done.status, done.stderr);
}

#[test]
fn the_program_runs_clean_under_asan() {
    // the same program under AddressSanitizer with the arena's chunk
    // recycling off: calls of every outcome, abandoned and broken streams, a
    // failed connection, a connection that vanishes mid-call, a listener
    // released at stop(): a use after free or a leak would be reported here
    let bin = harness::unique_bin("api_grpc_asan");
    harness::build_source_asan(&source(64), &bin);
    let server = Server::start(&bin, &[("LOTUS_NO_CHUNK_POOL", "1"), ("SLOW_MS", "200")]);
    server.ready();
    let mut c = Client::connect(server.port);
    for (i, name) in ["result", "handler_error", "refusal_unauthorized", "refusal_malformed", "server_error", "refusal_unavailable"].iter().enumerate() {
        let rec = recording(name);
        let stream = 2 * i as u32 + 1;
        replay(&mut c, stream, &rec);
        assert_replayed(&mut c, stream, name, &rec);
    }
    // a description, and a message that crosses many DATA frames
    call(&mut c, 13, "/hale.api.Description/Describe", Some("t-alice"), None, JSON, &message("{}"));
    assert_eq!(c.response(13, WAIT).header("grpc-status"), Some("0"));
    let big = message(&format!("{{\"symbol\":\"{}\",\"qty\":1,\"limit\":1}}", "A".repeat(500_000)));
    call(&mut c, 15, "/Public/Orders.place", Some("t-alice"), Some(DIGEST), JSON, &big);
    assert_eq!(c.response(15, WAIT).status(), 200);
    // abandoned and broken streams, a failed connection, a vanished client
    c.headers(17, &[(":method", "POST"), (":scheme", "http"), (":path", "/Public/Orders.place"), (":authority", "localhost"), ("content-type", JSON)], false);
    c.data(17, &message(PLACE)[..10], false);
    c.reset(17, 8);
    call(&mut c, 19, "/Public/Orders.place", Some("t-alice"), Some(DIGEST), JSON, &message(PLACE));
    c.reset(19, 8);
    let mut bad = Client::connect(server.port);
    bad.frame(DATA, 0, 0, b"nope");
    assert!(bad.closed(WAIT));
    {
        let mut gone = Client::connect(server.port);
        call(&mut gone, 1, "/Public/Orders.place", Some("t-alice"), Some(DIGEST), JSON, &message(PLACE));
    }
    // a call still executing when the program stops
    let mut slow = Client::connect(server.port);
    let place = recording("result");
    replay(&mut slow, 1, &place);
    std::thread::sleep(Duration::from_millis(50));
    let done = server.finish();
    drop(slow);
    for bad in ["AddressSanitizer", "LeakSanitizer", "heap-use-after-free", "SUMMARY:"] {
        assert!(!done.stderr.contains(bad), "{bad} in:\n{}", done.stderr);
    }
    assert!(done.status.success(), "{:?}\n{}\n{}", done.status, done.stdout, done.stderr);
}

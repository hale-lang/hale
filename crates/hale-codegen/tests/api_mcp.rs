//! GH #1417 (R6): `mcp::Rpc`, the third socket transport of `api::serve`.
//!
//! The witness's `Public` half is the program of `api_http.rs` with its
//! transport swapped for `mcp::Rpc`. These tests build it, serve it on a
//! loopback port and speak MCP to it (JSON-RPC 2.0 in `POST /mcp`):
//!
//! * the recorded exchanges of `tests/api-contract/wire/http/` are replayed
//!   as `tools/call`: each outcome arrives as the MCP column of
//!   spec/api.md § Outcomes says (a result, a JSON-RPC error carrying the
//!   handler's error, the refusal object, or the server error), with the
//!   recorded bodies as the payloads;
//! * `tools/list` is the tools of `Public.mcp.json` the caller may use;
//! * `initialize`, `ping` and notifications answer as MCP requires;
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

const DIGEST: &str = "fnv1a64:a8930d6e7998e986";

/// The `Public` program of `api_http.rs`, served over `mcp::Rpc`.
fn source(bound: u32) -> String {
    let mcp = super::api_http::PUBLIC.replace("http::Rpc { bind: std::env::var(\"BIND\"), codec: json,", "mcp::Rpc { bind: std::env::var(\"BIND\"),");
    assert_ne!(mcp, super::api_http::PUBLIC, "the transport was swapped");
    mcp.replace("@BOUND@", &bound.to_string())
}

/// The program with `bound` requests at a time, built once per process.
fn public(bound: u32) -> PathBuf {
    static BUILT: OnceLock<std::sync::Mutex<std::collections::BTreeMap<u32, PathBuf>>> = OnceLock::new();
    let map = BUILT.get_or_init(Default::default);
    let mut map = map.lock().unwrap();
    map.entry(bound)
        .or_insert_with(|| {
            let bin = harness::unique_bin(&format!("api_mcp_public{bound}"));
            build_opts::build_source(&source(bound), &bin, &build_opts::options()).expect("build the Public program over mcp::Rpc");
            bin
        })
        .clone()
}

/// A JSON string's content: `text` with `\` and `"` escaped.
fn esc(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

fn rpc(port: u16, token: Option<&str>, body: &str) -> Response {
    let mut headers = vec![h("Content-Type", "application/json"), h("Accept", "application/json, text/event-stream")];
    if let Some(t) = token {
        headers.push(bearer(t));
    }
    request(port, "POST", "/mcp", &headers, Some(body))
}

fn message(id: u32, method: &str, params: &str) -> String {
    format!("{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"{method}\",\"params\":{params}}}")
}

/// The recording's request as a `tools/call` with `id`.
fn tools_call(rec: &Recording, id: u32) -> String {
    let tool = rec.path.trim_start_matches("/call/").replace("::", "__");
    message(id, "tools/call", &format!("{{\"name\":\"{tool}\",\"arguments\":{}}}", rec.body))
}

fn send_call(port: u16, rec: &Recording, id: u32) -> Response {
    let token = rec.headers.iter().find(|(k, _)| k == "Authorization").map(|(_, v)| v.trim_start_matches("Bearer ").to_string());
    let digest = rec.headers.iter().find(|(k, _)| k == "Hale-Surface-Digest").map(|(_, v)| v.clone());
    let mut headers = vec![h("Content-Type", "application/json")];
    if let Some(t) = &token {
        headers.push(bearer(t));
    }
    if let Some(d) = digest {
        headers.push(h("Hale-Surface-Digest", &d));
    }
    request(port, "POST", "/mcp", &headers, Some(&tools_call(rec, id)))
}

fn error_body(id: u32, code: i32, message: &str, data: &str) -> String {
    format!("{{\"jsonrpc\":\"2.0\",\"id\":{id},\"error\":{{\"code\":{code},\"message\":\"{message}\",\"data\":{data}}}}}")
}

/// The refusal object of a recorded HTTP reply (`{"refusal": …}`).
fn refusal_of(reply: &str) -> String {
    reply.strip_prefix("{\"refusal\":").and_then(|r| r.strip_suffix('}')).unwrap_or_else(|| panic!("not a refusal body: {reply}")).to_string()
}

#[test]
fn the_recorded_outcomes_arrive_as_the_mcp_column_says() {
    let server = Server::start(&public(16), &[]);
    server.ready();
    let mut id = 10;
    let mut go = |name: &str| {
        id += 1;
        let rec = recording(name);
        let got = send_call(server.port, &rec, id);
        (rec, got, id)
    };
    // result: a JSON-RPC result whose content is the response
    let (rec, got, n) = go("result");
    assert_eq!(got.status, 200, "{}", got.body);
    assert_eq!(got.header("Content-Type"), Some("application/json"));
    assert_eq!(got.body, format!("{{\"jsonrpc\":\"2.0\",\"id\":{n},\"result\":{{\"content\":[{{\"type\":\"text\",\"text\":\"{}\"}}],\"structuredContent\":{},\"isError\":false}}}}", esc(&rec.reply), rec.reply));
    // handler error: an error object carrying the handler's E
    let (rec, got, n) = go("handler_error");
    assert_eq!((got.status, got.body), (200, error_body(n, -32001, "handler_error", &rec.reply)));
    // refusals: an error object carrying the refusal; the code by kind
    for (name, kind, code, status) in [
        ("refusal_unauthenticated", "unauthenticated", -32004, 401),
        ("refusal_unauthorized", "unauthorized", -32005, 200),
        ("refusal_malformed", "malformed", -32602, 200),
        ("refusal_digest_mismatch", "digest_mismatch", -32003, 200),
    ] {
        let (rec, got, n) = go(name);
        assert_eq!((got.status, got.body), (status, error_body(n, code, kind, &refusal_of(&rec.reply))), "{name}");
    }
    // the server error of a violating handler, then the next call to that
    // handler's type: unavailable
    let (rec, got, n) = go("server_error");
    assert_eq!((got.status, got.body), (200, error_body(n, -32603, "server_error", &refusal_of(&rec.reply))));
    let (rec, got, n) = go("refusal_unavailable");
    assert_eq!((got.status, got.body), (200, error_body(n, -32008, "unavailable", &refusal_of(&rec.reply))));
    let done = server.finish();
    assert!(done.status.success(), "the program ended cleanly: {:?}\n{}\n{}", done.status, done.stdout, done.stderr);
    assert!(done.stdout.contains("stopped"), "stop() returned:\n{}", done.stdout);
}

#[test]
fn a_request_over_the_bound_is_refused_full() {
    let server = Server::start(&public(1), &[("SLOW_MS", "600")]);
    server.ready();
    let result = recording("result");
    let token = "t-alice";
    let slow = send(
        server.port,
        &request_text("POST", "/mcp", &[bearer(token), h("Hale-Surface-Digest", DIGEST)], Some(&tools_call(&result, 1))),
    );
    std::thread::sleep(Duration::from_millis(200));
    let full = recording("refusal_full");
    let got = send_call(server.port, &full, 2);
    assert_eq!((got.status, got.body), (200, error_body(2, -32006, "full", &refusal_of(&full.reply))));
    let first = slow.response_within(15_000).expect("the slow call is answered");
    assert!(first.body.contains("\"id\":1,\"result\""), "{}", first.body);
    assert!(server.finish().status.success());
}

#[test]
fn a_request_after_stop_began_is_refused_shutting_down() {
    let mut server = Server::start(&public(16), &[("SLOW_MS", "800")]);
    server.ready();
    let result = recording("result");
    let slow = send(
        server.port,
        &request_text("POST", "/mcp", &[bearer("t-alice"), h("Hale-Surface-Digest", DIGEST)], Some(&tools_call(&result, 1))),
    );
    std::thread::sleep(Duration::from_millis(200));
    server.trigger();
    std::thread::sleep(Duration::from_millis(200));
    let down = recording("refusal_shutting_down");
    let got = send_call(server.port, &down, 2);
    assert_eq!((got.status, got.body), (200, error_body(2, -32007, "shutting_down", &refusal_of(&down.reply))));
    let first = slow.response_within(15_000).expect("the executing call was answered");
    assert!(first.body.contains("\"id\":1,\"result\""), "{}", first.body);
    let done = server.wait(Duration::from_secs(30));
    assert!(done.status.success(), "{:?}\n{}", done.status, done.stderr);
    assert!(std::net::TcpStream::connect(("127.0.0.1", server.port)).is_err(), "the listener is closed once stop() returned");
}

#[test]
fn tools_list_is_the_mcp_projection_filtered_by_the_caller() {
    let server = Server::start(&public(16), &[]);
    server.ready();
    let fixture = description_fixture("Public.mcp.json");
    let tools = {
        let at = fixture.find("\"tools\":[").expect("tools") + "\"tools\":".len();
        // the array, bracket to bracket
        let (mut depth, mut in_str, mut esc_next, mut end) = (0, false, false, at);
        for (i, c) in fixture[at..].char_indices() {
            if in_str {
                if esc_next {
                    esc_next = false;
                } else if c == '\\' {
                    esc_next = true;
                } else if c == '"' {
                    in_str = false;
                }
                continue;
            }
            match c {
                '"' => in_str = true,
                '[' | '{' => depth += 1,
                ']' | '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = at + i + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        fixture[at..end].to_string()
    };
    // alice holds `trader`: both tools, as the fixture lists them
    let alice = rpc(server.port, Some("t-alice"), &message(1, "tools/list", "{}"));
    assert_eq!(alice.status, 200, "{}", alice.body);
    assert_eq!(alice.body, format!("{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{{\"tools\":{tools}}}}}"));
    // bob does not: `Orders__cancel` is not listed
    let bob = rpc(server.port, Some("t-bob"), &message(2, "tools/list", "{}"));
    assert_eq!(bob.status, 200, "{}", bob.body);
    assert!(bob.body.contains("\"name\":\"Orders__place\""), "{}", bob.body);
    assert!(!bob.body.contains("Orders__cancel"), "a tool the caller may not use is not listed: {}", bob.body);
    // nobody is refused
    let nobody = rpc(server.port, None, &message(3, "tools/list", "{}"));
    assert_eq!(nobody.status, 401, "{}", nobody.body);
    assert!(nobody.body.contains("\"code\":-32004"), "{}", nobody.body);
    assert!(server.finish().status.success());
}

#[test]
fn initialize_ping_and_notifications_answer_as_mcp_requires() {
    let server = Server::start(&public(16), &[]);
    server.ready();
    let init = rpc(server.port, Some("t-alice"), &message(7, "initialize", "{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{},\"clientInfo\":{\"name\":\"t\",\"version\":\"1\"}}"));
    assert_eq!(init.status, 200, "{}", init.body);
    assert_eq!(
        init.body,
        format!("{{\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{{\"tools\":{{\"listChanged\":false}}}},\"serverInfo\":{{\"name\":\"Public\",\"version\":\"{DIGEST}\"}}}}}}")
    );
    // no version asked: the current one is answered; a string id is echoed verbatim
    let bare = rpc(server.port, Some("t-alice"), "{\"jsonrpc\":\"2.0\",\"id\":\"abc\",\"method\":\"initialize\",\"params\":{}}");
    assert!(bare.body.starts_with("{\"jsonrpc\":\"2.0\",\"id\":\"abc\",\"result\":{\"protocolVersion\":\"2025-06-18\""), "{}", bare.body);
    let ping = rpc(server.port, Some("t-alice"), &message(8, "ping", "{}"));
    assert_eq!(ping.body, "{\"jsonrpc\":\"2.0\",\"id\":8,\"result\":{}}");
    // a notification is accepted and answered with nothing
    let note = rpc(server.port, Some("t-alice"), "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}");
    assert_eq!((note.status, note.body.as_str()), (202, ""));
    // a method the server does not have
    let other = rpc(server.port, Some("t-alice"), &message(9, "resources/list", "{}"));
    assert_eq!(other.body, error_body(9, -32601, "malformed", "{\"kind\":\"malformed\",\"reason\":\"method not found: resources/list\"}"));
    assert!(server.finish().status.success());
}

#[test]
fn a_message_that_is_not_a_call_is_refused_and_the_connection_ends() {
    let server = Server::start(&public(16), &[]);
    server.ready();
    // a tool no row names
    let unknown = rpc(server.port, Some("t-alice"), &message(1, "tools/call", "{\"name\":\"Orders__nothing\",\"arguments\":{}}"));
    assert_eq!(unknown.status, 200, "{}", unknown.body);
    assert!(unknown.body.contains("\"code\":-32602") && unknown.body.contains("unknown_member: Orders__nothing"), "{}", unknown.body);
    // not JSON, a batch, a call without a name
    let junk = rpc(server.port, Some("t-alice"), "this is not json");
    assert!(junk.body.contains("\"code\":-32700") && junk.body.contains("\"id\":null"), "{}", junk.body);
    let batch = rpc(server.port, Some("t-alice"), "[{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}]");
    assert!(batch.body.contains("\"code\":-32600"), "{}", batch.body);
    let nameless = rpc(server.port, Some("t-alice"), &message(2, "tools/call", "{}"));
    assert!(nameless.body.contains("\"code\":-32602"), "{}", nameless.body);
    // the endpoint takes POST /mcp
    let get = request(server.port, "GET", "/mcp", &[bearer("t-alice")], None);
    assert_eq!(get.status, 405, "{}", get.body);
    let elsewhere = request(server.port, "POST", "/call/Orders::place", &[bearer("t-alice")], Some("{}"));
    assert!(elsewhere.body.contains("\"code\":-32600"), "{}", elsewhere.body);
    // bytes that are not a request, a request cut off, a client that vanishes
    send(server.port, "\u{1}\u{2}this is not http\r\n\r\n").close();
    send(server.port, "POST /mcp HTTP/1.1\r\nContent-Length: 500\r\n\r\n{\"jsonrpc").close();
    let rec = recording("result");
    send(server.port, &request_text("POST", "/mcp", &[bearer("t-alice")], Some(&tools_call(&rec, 3)))).close();
    // and the listener serves on
    let after = send_call(server.port, &rec, 4);
    assert_eq!(after.status, 200, "{}", after.body);
    assert!(after.body.contains("\"id\":4,\"result\""), "{}", after.body);
    let done = server.finish();
    assert!(done.status.success(), "{:?}\n{}", done.status, done.stderr);
}

/// A surface whose members hold `__`: `A::b__c` and `A__b::c` are one name
/// under a plain `::` → `__` spelling.
const UNDERSCORES: &str = r#"
type Q { n: Int; }
type R { n: Int; }

api S {
    rpc A::b_c;
    rpc A::b__c;
    rpc A__b::c;
}

locus Tok {
    fn principal(token: String) -> std::api::Principal { return std::api::Principal { mode: "bearer", name: "x" }; }
    fn refused() -> String { return ""; }
}

locus A {
    fn b_c(q: Q) -> R { return R { n: 1 }; }
    fn b__c(q: Q) -> R { return R { n: 3 }; }
}

locus A__b {
    fn c(q: Q) -> R { return R { n: 2 }; }
}

main locus D {
    params {
        t: Tok = Tok { };
        a: A = A { };
        ab: A__b = A__b { };
    }
    run() {
        let h = api::serve(S, mcp::Rpc { bind: std::env::var("BIND"), principals: self.t }, as: "s", bound: 4, on_full: refuse);
        let trigger = std::env::var("TRIGGER");
        while !self.draining && (std::io::fs::read_file(trigger) or "") == "" {
            std::time::sleep(10ms);
        }
        h.stop();
    }
}

fn main() { D { }; }
"#;

#[test]
fn a_member_with_underscores_is_called_by_its_own_tool_name() {
    let bin = harness::unique_bin("api_mcp_underscores");
    build_opts::build_source(UNDERSCORES, &bin, &build_opts::options()).expect("build the underscore surface over mcp::Rpc");
    let server = Server::start(&bin, &[]);
    server.ready();
    let list = rpc(server.port, Some("t"), &message(1, "tools/list", "{}"));
    let mut names: Vec<&str> = list.body.split("\"name\":\"").skip(1).filter_map(|t| t.split('"').next()).collect();
    names.sort();
    // three distinct tools; a `_` of an identifier that would run into the
    // joining `__` is written `_-`
    assert_eq!(names, ["A_-_-b__c", "A__b_-_-c", "A__b_c"], "{}", list.body);
    // each reaches its own member: the handlers answer 1, 3 and 2
    for (id, (tool, n)) in [("A__b_c", 1), ("A__b_-_-c", 3), ("A_-_-b__c", 2)].into_iter().enumerate() {
        let got = rpc(server.port, Some("t"), &message(id as u32 + 2, "tools/call", &format!("{{\"name\":\"{tool}\",\"arguments\":{{\"n\":0}}}}")));
        assert!(got.body.contains(&format!("\"structuredContent\":{{\"n\":{n}}}")), "{tool}: {}", got.body);
    }
    // the plain rewrite's name is no tool: it is refused, not guessed at
    let rewritten = rpc(server.port, Some("t"), &message(9, "tools/call", "{\"name\":\"A__b__c\",\"arguments\":{\"n\":0}}"));
    assert!(rewritten.body.contains("\"code\":-32602") && rewritten.body.contains("unknown_member: A__b__c"), "{}", rewritten.body);
    assert!(server.finish().status.success());
}

#[test]
fn a_listener_that_cannot_bind_fails_the_boot() {
    let held = std::net::TcpListener::bind("127.0.0.1:0").expect("hold a port");
    let port = held.local_addr().unwrap().port();
    let bin = public(16);
    let mut server = Server::start(&bin, &[("BIND", &format!("127.0.0.1:{port}"))]);
    let done = server.wait(Duration::from_secs(30));
    assert_eq!(done.status.code(), Some(2), "the boot fails: {:?}\n{}\n{}", done.status, done.stdout, done.stderr);
    assert!(done.stderr.contains("api: mcp::Rpc could not listen on 127.0.0.1:"), "the diagnostic:\n{}", done.stderr);
}

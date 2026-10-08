//! GH #1417 (R5 exit): the contract's § 3 program, whole.
//!
//! `tests/api-contract/program.hl` is built as it is written, with the
//! five addresses a test cannot fix (the two HTTP binds, the Unix path, the
//! hub's bind, the Unix operator) read from the environment, a fourth serve
//! site (`Public` over the hub, `feed`) and a run loop that stops one
//! exposure, or all of them, when told to. Every type, surface, role
//! source, receiver, placement, binding and `on_failure` is the contract's.
//!
//! One run of that program serves its rpc half over `http::Rpc` (twice),
//! `unix::Rpc`, `mcp::Rpc` (a fifth serve site, `agent`, of `Public`) and
//! `grpc::Rpc` (a sixth, `wire`, whose calls, trailers and GOAWAY at the stop
//! the test holds to the spec's gRPC column), and its stream half over `ws::Hub`, and the tests assert what a
//! caller of each can observe: the assertions of `api_http_witness`
//! (members reach their own receiver and pool, a shared handler meets each
//! surface's `requires`, a digest mismatch, every outcome, one exposure
//! stopped while the others serve, a connection that breaks) and of
//! `api_hub_streams` (admission and its refusals, events in order under a
//! `seq` that counts every event offered, the description for the caller,
//! `closed` at the stop and the address released), with the events the rpcs
//! publish on the stream and the hub carrying the surface on the
//! subscriber's connection. What needs a hook the contract's program does not
//! carry (a handler that holds its pool's thread, a bound of one, a role
//! revision, a credential that expires) stays in the tests that build their
//! own.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;
#[path = "support/ports.rs"]
mod ports;
#[path = "support/http_rpc.rs"]
mod http_rpc;
#[path = "support/unix_rpc.rs"]
mod unix_rpc;
#[path = "support/ws_client.rs"]
mod ws_client;

use http_rpc::*;
use ws_client::*;

/// The contract's program, as it is in the repository.
const CONTRACT: &str = include_str!("../../../tests/api-contract/program.hl");

/// `from` becomes `to` in the contract's text; the contract must have it.
fn swap(src: &str, from: &str, to: &str) -> String {
    assert!(src.contains(from), "tests/api-contract/program.hl no longer has `{from}`");
    src.replacen(from, to, 1)
}

/// The contract's program for a test: its addresses from `$BIND`, `$BIND2`,
/// `$SOCK` and `$OPERATOR`, the hub's at `hub` (a bind read from the
/// environment states no address in the description), `Public` also served
/// over the hub, and a
/// run loop that stops `public`, `partner` and `admin` on `$TRIGGER_PUBLIC`,
/// `$TRIGGER_PARTNER` and `$TRIGGER_ADMIN` and everything on `$TRIGGER`.
fn witness_whole(hub: u16, mcp: u16, grpc: u16) -> String {
    let s = CONTRACT;
    let s = swap(&s, "bind: \"127.0.0.1:8080\"", "bind: std::env::var(\"BIND\")");
    let s = swap(&s, "bind: \"127.0.0.1:8081\"", "bind: std::env::var(\"BIND2\")");
    let s = swap(&s, "path: \"/run/desk/admin.sock\"", "path: std::env::var(\"SOCK\")");
    let s = swap(&s, "bind: \"127.0.0.1:9000\"", &format!("bind: \"127.0.0.1:{hub}\""));
    let s = swap(&s, "operator: \"uid:1000\"", "operator: std::env::var(\"OPERATOR\")");
    let s = swap(
        &s,
        "as: \"admin\", receivers: { Orders: self.orders }, bound: 16, on_full: refuse);\n",
        "as: \"admin\", receivers: { Orders: self.orders }, bound: 16, on_full: refuse);\n        let feed = api::serve(Public, self.hub, as: \"feed\", receivers: { Orders: self.orders }, bound: 16, on_full: refuse);\n",
    );
    let s = swap(
        &s,
        "        let feed = api::serve(",
        &format!("        let agent = api::serve(Public, mcp::Rpc {{ bind: \"127.0.0.1:{mcp}\", principals: self.bearer, roles: self.public_roles }}, as: \"agent\", receivers: {{ Orders: self.orders }}, bound: 16, on_full: refuse);\n        let wire = api::serve(Public, grpc::Rpc {{ bind: \"127.0.0.1:{grpc}\", principals: self.bearer, roles: self.public_roles }}, as: \"wire\", receivers: {{ Orders: self.orders }}, bound: 16, on_full: refuse);\n        let feed = api::serve("),
    );
    let s = swap(
        &s,
        "        std::api::run_until_stopped(public);\n        public.stop();\n        partner.stop();\n        admin.stop();\n",
        concat!(
            "        let tp = std::env::var(\"TRIGGER_PUBLIC\");\n",
            "        let tq = std::env::var(\"TRIGGER_PARTNER\");\n",
            "        let ta = std::env::var(\"TRIGGER_ADMIN\");\n",
            "        let all = std::env::var(\"TRIGGER\");\n",
            "        let mut public_up = true;\n",
            "        let mut partner_up = true;\n",
            "        let mut admin_up = true;\n",
            "        while !self.draining && !written(all) {\n",
            "            if public_up && written(tp) { public.stop(); public_up = false; println(\"public stopped\"); }\n",
            "            if partner_up && written(tq) { partner.stop(); partner_up = false; println(\"partner stopped\"); }\n",
            "            if admin_up && written(ta) { admin.stop(); admin_up = false; println(\"admin stopped\"); }\n",
            "            std::time::sleep(10ms);\n",
            "        }\n",
            "        public.stop();\n",
            "        partner.stop();\n",
            "        admin.stop();\n",
            "        agent.stop();\n",
            "        wire.stop();\n",
            "        feed.stop();\n",
            "        self.hub.stop();\n",
            "        println(\"stopped\");\n",
            "        println(\"orders_failed=\" + to_string(self.orders_failed));\n",
            "        println(\"ledger_failed=\" + to_string(self.ledger_failed));\n",
        ),
    );
    swap(
        &s,
        "fn main() { Desk { }; }",
        "fn written(path: String) -> Bool {\n    return (std::io::fs::read_file(path) or \"\") != \"\";\n}\n\nfn main() { Desk { }; }",
    )
}

/// The program built once per process, and the ports its hub, its MCP
/// endpoint and its gRPC endpoint were built for.
fn build() -> (PathBuf, u16, u16, u16) {
    static BIN: OnceLock<(PathBuf, u16, u16, u16)> = OnceLock::new();
    BIN.get_or_init(|| {
        let bin = harness::unique_bin("api_witness_whole");
        let (hub, mcp, grpc) = (ports::free_port(), ports::free_port(), ports::free_port());
        build_opts::build_source(&witness_whole(hub, mcp, grpc), &bin, &build_opts::options()).expect("build the contract's program");
        (bin, hub, mcp, grpc)
    })
    .clone()
}

const PUBLIC_DIGEST: &str = "fnv1a64:a8930d6e7998e986";
const ADMIN_DIGEST: &str = "fnv1a64:40381db6685c9f75";

/// The program, running: the http server (`port`, `port2`, the socket), and
/// the ports of its hub and its MCP endpoint.
struct Whole {
    server: Server,
    hub: u16,
    mcp: u16,
    grpc: u16,
}

fn start(bin: &std::path::Path, hub: u16, mcp: u16, grpc: u16, env: &[(&str, &str)]) -> Whole {
    let operator = format!("uid:{}", unix_rpc::me().0);
    let mut all: Vec<(&str, &str)> = vec![("OPERATOR", &operator)];
    all.extend_from_slice(env);
    let server = Server::start(bin, &all);
    server.ready();
    wait_listening(server.port2);
    wait_accepting_unix(&server.sock());
    wait_listening(hub);
    wait_listening(mcp);
    wait_listening(grpc);
    Whole { server, hub, mcp, grpc }
}

fn place(port: u16, token: &str) -> Response {
    request(port, "POST", "/call/Orders::place", &[bearer(token), h("Hale-Surface-Digest", PUBLIC_DIGEST)], Some("{\"symbol\":\"ACME\",\"qty\":10,\"limit\":12500}"))
}

fn cancel(port: u16, token: &str, order: i64) -> Response {
    let body = format!("{{\"order\":{order}}}");
    request(port, "POST", "/call/Orders::cancel", &[bearer(token), h("Hale-Surface-Digest", PUBLIC_DIGEST)], Some(&body))
}

/// The order id in `{"order":N,…}`.
fn order_of(text: &str) -> i64 {
    let at = text.find("\"order\":").unwrap_or_else(|| panic!("no order in {text}")) + 8;
    text[at..].chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap()
}

fn unix(w: &Whole, line: &str) -> String {
    unix_ask(&w.server.sock(), line)
}

/// The event frame for order `order`, the `seq`th offered to the stream.
fn fill(seq: i64, order: i64) -> String {
    format!("{{\"type\":\"event\",\"topic\":\"Fills\",\"seq\":{seq},\"payload\":{{\"order\":{order},\"qty\":10,\"price\":12500}}}}")
}

fn fixture(name: &str) -> serde_json::Value {
    let path = contract_dir().join(name);
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))).expect("json")
}

fn wait_closed(port: u16, what: &str) {
    let start = Instant::now();
    while !closed(port) {
        assert!(start.elapsed() < Duration::from_secs(10), "{what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn the_contracts_program_serves_its_rpc_and_stream_halves_in_one_run() {
    let (bin, hub, mcp, grpc) = build();
    let w = start(&bin, hub, mcp, grpc, &[]);
    let (public, partner) = (w.server.port, w.server.port2);

    // ---- the stream half: admission, in the contract's words ----
    let mut dave = Ws::connect(w.hub, Some("t-dave"));
    dave.subscribe("Fills");
    assert_eq!(dave.text(), "{\"type\":\"subscribed\",\"topic\":\"Fills\"}");
    // bob is a bearer the sources name, and holds no `operator` under `hub_roles`
    let mut bob = Ws::connect(w.hub, Some("t-bob"));
    bob.subscribe("Fills");
    assert_eq!(
        bob.text(),
        "{\"type\":\"refusal\",\"topic\":\"Fills\",\"refusal\":{\"kind\":\"unauthorized\",\"reason\":\"Fills requires operator\",\"requires\":[\"operator\"]}}"
    );
    let mut anon = Ws::connect(w.hub, None);
    anon.subscribe("Fills");
    assert_eq!(
        anon.text(),
        "{\"type\":\"refusal\",\"topic\":\"Fills\",\"refusal\":{\"kind\":\"unauthenticated\",\"reason\":\"no credential was presented\"}}"
    );
    let mut ghost = Ws::connect(w.hub, Some("t-ghost"));
    ghost.subscribe("Fills");
    assert!(ghost.text().contains("\"kind\":\"unauthenticated\""));
    dave.subscribe("Nothing");
    assert_eq!(
        dave.text(),
        "{\"type\":\"refusal\",\"topic\":\"Nothing\",\"refusal\":{\"kind\":\"malformed\",\"reason\":\"unknown_topic: Nothing\"}}"
    );
    dave.send_ping(b"alive");
    assert_eq!(dave.recv_within(5000), Frame::Pong(b"alive".to_vec()));

    // ---- the description over the hub lists `Fills` for the operator only ----
    for (token, name) in [("t-dave", "fills.dave.description.json"), ("t-bob", "fills.bob.description.json")] {
        let (status, body) = get_description(w.hub, Some(token));
        assert_eq!(status, 200, "{body}");
        let got: serde_json::Value = serde_json::from_str(&body).unwrap_or_else(|e| panic!("{e}: {body}"));
        let mut want = fixture(name);
        want["listener"]["address"] = serde_json::Value::String(format!("127.0.0.1:{}", w.hub));
        assert_eq!(got, want, "the description for {token}");
        assert_eq!(body.contains("\"topic\":\"Fills\""), token == "t-dave", "{token}: {body}");
    }
    assert_eq!(get_description(w.hub, None).0, 401);
    assert_eq!(get_description(w.hub, Some("t-ghost")).0, 401);

    // ---- every member reaches its own receiver; the stream carries what they publish ----
    let a = place(public, "t-alice");
    assert_eq!(a.status, 200, "{}", a.body);
    assert_eq!(a.body, "{\"order\":41,\"notional\":125000}");
    assert_eq!(dave.text(), fill(1, 41));
    let c = place(partner, "t-carol");
    assert_eq!(c.status, 200, "{}", c.body);
    assert_eq!(c.body, "{\"order\":9001,\"notional\":125000}");
    assert_eq!(dave.text(), fill(2, 9001));
    // a refused subscriber has nothing queued, whatever was published
    bob.silence(100);
    let miss = cancel(public, "t-alice", 9001);
    assert_eq!(miss.status, 422, "public's Orders never placed 9001: {}", miss.body);
    assert_eq!(miss.body, "{\"code\":\"unknown_order\",\"reason\":\"no order 9001\"}");
    let hit = cancel(partner, "t-carol", 9001);
    assert_eq!(hit.status, 200, "partner's Orders placed 9001: {}", hit.body);
    // the admin exposure is bound to public's instance: it cancels the order public placed
    let line = unix(&w, &format!("{{\"call\":\"Orders::cancel\",\"payload\":{{\"order\":41}},\"id\":\"a-1\",\"digest\":\"{ADMIN_DIGEST}\"}}"));
    assert!(line.contains("\"id\":\"a-1\",\"ok\":true,\"value\":{\"order\":41,\"was_open\":true}"), "{line}");
    let line = unix(&w, &format!("{{\"call\":\"Orders::cancel\",\"payload\":{{\"order\":9001}},\"id\":\"a-2\",\"digest\":\"{ADMIN_DIGEST}\"}}"));
    assert!(line.contains("\"ok\":false,\"error\":{\"code\":\"unknown_order\""), "admin's Orders is public's, which never placed 9001: {line}");

    // ---- a shared handler meets each surface's `requires` ----
    let denied = cancel(partner, "t-alice", 41);
    assert_eq!(denied.status, 403, "{}", denied.body);
    assert_eq!(denied.body, "{\"refusal\":{\"kind\":\"unauthorized\",\"reason\":\"Orders::cancel requires trader\",\"requires\":[\"trader\"]}}");
    assert_eq!(cancel(public, "t-bob", 41).status, 403);
    let line = unix(&w, &format!("{{\"call\":\"Orders::cancel\",\"payload\":{{\"order\":999}},\"id\":\"a-3\",\"digest\":\"{ADMIN_DIGEST}\"}}"));
    assert!(line.contains("\"ok\":false,\"error\":{\"code\":\"unknown_order\""), "the operator is let through to the handler: {line}");
    // the descriptions differ with the exposure's source, and are the fixtures'
    for (port, token, name, bound) in [
        (public, "t-alice", "public.alice.description.json", "127.0.0.1:8080"),
        (public, "t-bob", "public.bob.description.json", "127.0.0.1:8080"),
        (partner, "t-carol", "partner.carol.description.json", "127.0.0.1:8081"),
        (partner, "t-alice", "partner.alice.description.json", "127.0.0.1:8081"),
    ] {
        let got = request(port, "GET", "/.description", &[bearer(token)], None);
        assert_eq!(got.status, 200, "{}", got.body);
        let want = description_fixture(name).replace(bound, &format!("127.0.0.1:{port}"));
        assert_eq!(got.body, want, "{token}: the description of :{port} differs from {name}");
    }
    let nobody = request(public, "GET", "/.description", &[bearer("t-mallory")], None);
    assert_eq!(nobody.status, 401, "{}", nobody.body);
    let line = unix(&w, "{\"describe\":true}");
    assert!(line.contains("\"ok\":true"), "{line}");

    // ---- a digest mismatch names the served digest, on each transport ----
    for port in [public, partner] {
        let got = request(port, "POST", "/call/Orders::place", &[bearer("t-alice"), h("Hale-Surface-Digest", "fnv1a64:0123456789abcdef")], Some("{\"symbol\":\"ACME\",\"qty\":10,\"limit\":12500}"));
        assert_eq!(got.status, 409, "{}", got.body);
        assert_eq!(
            got.body,
            format!("{{\"refusal\":{{\"kind\":\"digest_mismatch\",\"reason\":\"the request was generated against fnv1a64:0123456789abcdef\",\"served\":\"{PUBLIC_DIGEST}\"}}}}")
        );
    }
    let line = unix(&w, "{\"call\":\"Ledger::rebalance\",\"payload\":{\"book\":\"main\"},\"id\":\"m-1\",\"digest\":\"fnv1a64:0123456789abcdef\"}");
    assert!(line.contains(&format!("\"refusal\":{{\"kind\":\"digest_mismatch\",\"reason\":\"the request was generated against fnv1a64:0123456789abcdef\",\"served\":\"{ADMIN_DIGEST}\"}}")), "{line}");
    // the digest is optional: a request that sends none is not refused for it
    let none = request(public, "POST", "/call/Orders::place", &[bearer("t-alice")], Some("{\"symbol\":\"ACME\",\"qty\":10,\"limit\":12500}"));
    assert_eq!(none.status, 200, "{}", none.body);
    assert_eq!(order_of(&none.body), 42);
    assert_eq!(dave.text(), fill(3, 42));

    // ---- the hub carries the surface on the subscriber's connection ----
    dave.send_text("{\"type\":\"call\",\"id\":\"c1\",\"call\":\"Orders::place\",\"payload\":{\"symbol\":\"ACME\",\"qty\":10,\"limit\":12500}}");
    let (mut reply, mut event) = (None, None);
    while reply.is_none() || event.is_none() {
        let f = dave.text();
        if f.contains("\"type\":\"event\"") {
            event = Some(f);
        } else {
            reply = Some(f);
        }
    }
    let reply = reply.unwrap();
    assert!(reply.starts_with("{\"type\":\"reply\",\"request_id\":"), "{reply}");
    assert!(reply.contains("\"id\":\"c1\",\"ok\":true,\"value\":{\"order\":43,\"notional\":125000}"), "{reply}");
    // ... and what the call published is the next event offered, on the same connection
    assert_eq!(event.unwrap(), fill(4, 43));
    // dave holds `operator` under `hub_roles`, not `trader`: the shared handler's `requires` is the surface's
    dave.send_text("{\"type\":\"call\",\"id\":\"c2\",\"call\":\"Orders::cancel\",\"payload\":{\"order\":43}}");
    let refusal = dave.text();
    assert!(refusal.contains("\"id\":\"c2\",\"ok\":false,\"refusal\":{\"kind\":\"unauthorized\",\"reason\":\"Orders::cancel requires trader\""), "{refusal}");
    // a caller the sources name nobody for is refused before the handler runs
    anon.send_text("{\"type\":\"call\",\"id\":\"n1\",\"call\":\"Orders::place\",\"payload\":{\"symbol\":\"ACME\",\"qty\":10,\"limit\":12500}}");
    assert!(anon.text().contains("\"ok\":false,\"refusal\":{\"kind\":\"unauthenticated\""));
    bob.send_text("{\"type\":\"call\",\"id\":\"b1\",\"call\":\"Orders::nothing\",\"payload\":{}}");
    assert!(bob.text().contains("unknown_member: Orders::nothing"));
    dave.silence(100);

    // ---- every outcome on each transport ----
    let result = place(partner, "t-carol");
    assert_eq!((result.status, result.body.as_str()), (200, "{\"order\":9002,\"notional\":125000}"));
    assert_eq!(dave.text(), fill(5, 9002));
    let handler_error = cancel(partner, "t-carol", 99999);
    assert_eq!((handler_error.status, handler_error.body.as_str()), (422, "{\"code\":\"unknown_order\",\"reason\":\"no order 99999\"}"));
    let malformed = request(partner, "POST", "/call/Orders::place", &[bearer("t-carol")], Some("{\"symbol\":\"ACME\",\"limit\":12500}"));
    assert_eq!((malformed.status, malformed.body.as_str()), (400, "{\"refusal\":{\"kind\":\"malformed\",\"reason\":\"missing_field: qty\"}}"));
    let nobody = place(partner, "t-mallory");
    assert_eq!((nobody.status, nobody.body.as_str()), (401, "{\"refusal\":{\"kind\":\"unauthenticated\",\"reason\":\"no such token\"}}"));
    let violation = request(partner, "POST", "/call/Orders::place", &[bearer("t-carol")], Some("{\"symbol\":\"ACME\",\"qty\":50000,\"limit\":12500}"));
    assert_eq!((violation.status, violation.body.as_str()), (500, "{\"refusal\":{\"kind\":\"server\"}}"));
    let after = place(partner, "t-carol");
    assert_eq!(after.status, 503, "the receiver that violated is unavailable: {}", after.body);
    assert_eq!(after.body, "{\"refusal\":{\"kind\":\"unavailable\",\"reason\":\"the receiver of Orders::place is unavailable; the call did not run\"}}");
    // ... while public's instance is another receiver, untouched by it
    assert_eq!(place(public, "t-alice").status, 200);
    assert_eq!(dave.text(), fill(6, 44));
    let line = unix(&w, &format!("{{\"call\":\"Ledger::rebalance\",\"payload\":{{\"book\":\"main\"}},\"id\":\"u-1\",\"digest\":\"{ADMIN_DIGEST}\"}}"));
    assert!(line.contains("\"id\":\"u-1\",\"ok\":true,\"value\":{\"moved\":1500}"), "{line}");
    let line = unix(&w, "{\"call\":\"Orders::cancel\",\"payload\":{\"order\":999},\"id\":\"u-2\"}");
    assert!(line.contains("\"id\":\"u-2\",\"ok\":false,\"error\":{\"code\":\"unknown_order\",\"reason\":\"no order 999\"}"), "{line}");
    let line = unix(&w, "{\"call\":\"Ledger::rebalance\",\"payload\":{\"nope\":1},\"id\":\"u-3\"}");
    assert!(line.contains("\"id\":\"u-3\",\"ok\":false,\"refusal\":{\"kind\":\"malformed\""), "{line}");
    let line = unix(&w, "{\"call\":\"Ledger::rebalance\",\"payload\":{\"book\":\"\"},\"id\":\"u-4\"}");
    assert!(line.contains("\"id\":\"u-4\",\"ok\":false,\"refusal\":{\"kind\":\"server\"}"), "{line}");
    let line = unix(&w, "{\"call\":\"Ledger::rebalance\",\"payload\":{\"book\":\"main\"},\"id\":\"u-5\"}");
    assert!(line.contains("\"id\":\"u-5\",\"ok\":false,\"refusal\":{\"kind\":\"unavailable\""), "{line}");

    // ---- a connection that breaks is forgotten alone, on every listener ----
    send(public, "POST /call/Orders::place HTTP/1.1\r\nContent-Length: 100\r\n\r\n{\"sym").close();
    send(partner, "GET /.desc").close();
    {
        use std::io::Write;
        let mut s = std::os::unix::net::UnixStream::connect(w.server.sock()).expect("connect");
        s.write_all(b"{\"call\":\"Orders::cancel\",\"payl").expect("write");
    }
    let mut second = Ws::connect(w.hub, Some("t-dave"));
    second.subscribe("Fills");
    assert_eq!(second.text(), "{\"type\":\"subscribed\",\"topic\":\"Fills\"}");
    second.abandon();
    assert_eq!(place(public, "t-alice").status, 200);
    assert_eq!(dave.text(), fill(7, 45));

    // ---- stopping one exposure leaves the others serving ----
    w.server.stop_one("public");
    wait_closed(public, "public's listener closed after its stop()");
    // partner, admin and the hub go on, with their own state
    assert_eq!(request(partner, "GET", "/.description", &[bearer("t-carol")], None).status, 200);
    let line = unix(&w, &format!("{{\"call\":\"Orders::cancel\",\"payload\":{{\"order\":41}},\"id\":\"a-4\",\"digest\":\"{ADMIN_DIGEST}\"}}"));
    assert!(line.contains("\"ok\":true"), "admin still serves public's Orders: {line}");
    dave.send_text("{\"type\":\"call\",\"id\":\"c3\",\"call\":\"Orders::place\",\"payload\":{\"symbol\":\"ACME\",\"qty\":10,\"limit\":12500}}");
    let (mut got_reply, mut got_event) = (false, false);
    while !(got_reply && got_event) {
        let f = dave.text();
        if f == fill(8, 46) {
            got_event = true;
        } else {
            assert!(f.contains("\"id\":\"c3\",\"ok\":true,\"value\":{\"order\":46"), "{f}");
            got_reply = true;
        }
    }
    // ---- `Public` over MCP, beside the rest, in the same run ----
    let mcp_rpc = |token: Option<&str>, body: &str| {
        let mut headers = vec![h("Content-Type", "application/json")];
        if let Some(t) = token {
            headers.push(bearer(t));
        }
        request(w.mcp, "POST", "/mcp", &headers, Some(body))
    };
    let listed = mcp_rpc(Some("t-alice"), r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#);
    assert_eq!(listed.status, 200, "{}", listed.body);
    let listed: serde_json::Value = serde_json::from_str(&listed.body).expect("json");
    let want: serde_json::Value = serde_json::from_str(&description_fixture("Public.mcp.json")).expect("json");
    assert_eq!(listed["result"]["tools"], want["tools"], "alice's tools are Public.mcp.json's");
    let bob_listed = mcp_rpc(Some("t-bob"), r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#);
    assert!(bob_listed.body.contains("Orders__place") && !bob_listed.body.contains("Orders__cancel"), "{}", bob_listed.body);
    assert_eq!(mcp_rpc(None, r#"{"jsonrpc":"2.0","id":3,"method":"tools/list","params":{}}"#).status, 401);
    let call = |id: u32, tool: &str, args: &str| {
        let body = format!(
            r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/call","params":{{"name":"{tool}","arguments":{args},"_meta":{{"hale/digest":"{PUBLIC_DIGEST}"}}}}}}"#
        );
        mcp_rpc(Some("t-alice"), &body)
    };
    // a call is a call: the same receiver, the same handler, the same stream
    let placed = call(4, "Orders__place", r#"{"symbol":"ACME","qty":10,"limit":12500}"#);
    assert_eq!(placed.status, 200, "{}", placed.body);
    assert_eq!(
        placed.body,
        r#"{"jsonrpc":"2.0","id":4,"result":{"content":[{"type":"text","text":"{\"order\":47,\"notional\":125000}"}],"structuredContent":{"order":47,"notional":125000},"isError":false}}"#
    );
    assert_eq!(dave.text(), fill(9, 47));
    // the handler's error, the role's refusal and a missing field, as the MCP column says
    let missed = call(5, "Orders__cancel", r#"{"order":99999}"#);
    assert_eq!(missed.status, 200, "{}", missed.body);
    assert!(missed.body.contains(r#""id":5,"error":{"code":-32001,"message":"handler_error""#), "{}", missed.body);
    let forbidden = mcp_rpc(
        Some("t-bob"),
        r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"Orders__cancel","arguments":{"order":47}}}"#,
    );
    assert!(forbidden.body.contains(r#""id":6,"error":{"code":-32005,"message":"unauthorized""#), "{}", forbidden.body);
    let malformed = call(7, "Orders__place", r#"{"symbol":"ACME","limit":12500}"#);
    assert!(malformed.body.contains(r#""id":7,"error":{"code":-32602,"message":"malformed""#), "{}", malformed.body);
    let cancelled = call(8, "Orders__cancel", r#"{"order":47}"#);
    assert!(cancelled.body.contains(r#""id":8,"result":"#) && cancelled.body.contains(r#""isError":false"#), "{}", cancelled.body);
    dave.silence(100);

    // ---- `Public` over gRPC, beside the rest, in the same run ----
    // (the connection stays open: the final stop says GOAWAY on it)
    let mut g = super::api_grpc::h2_client::Client::connect(w.grpc);
    {
        use super::api_grpc::{description_over_grpc, text_of, unary};
        let described = unary(&mut g, 1, "/hale.api.Description/Describe", Some("t-alice"), "{}");
        assert_eq!(described.header("grpc-status"), Some("0"), "{described:?}");
        assert_eq!(text_of(&described), description_over_grpc("public.alice.description.json", w.grpc, "wire"), "alice's description over gRPC is the fixture's");
        // a call is a call: the same receiver, the same handler, the same stream
        let placed = unary(&mut g, 3, "/Public/Orders.place", Some("t-alice"), "{\"symbol\":\"ACME\",\"qty\":10,\"limit\":12500}");
        assert_eq!(placed.header("grpc-status"), Some("0"), "{placed:?}");
        assert_eq!(text_of(&placed), "{\"order\":48,\"notional\":125000}");
        assert_eq!(dave.text(), fill(10, 48));
        // the handler's error, the role's refusal, a missing field and a caller nobody names
        let missed = unary(&mut g, 5, "/Public/Orders.cancel", Some("t-alice"), "{\"order\":99999}");
        assert_eq!((missed.header("grpc-status"), missed.header("grpc-message")), (Some("9"), Some("handler_error")), "{missed:?}");
        let forbidden = unary(&mut g, 7, "/Public/Orders.cancel", Some("t-bob"), "{\"order\":48}");
        assert_eq!(forbidden.header("grpc-status"), Some("7"), "{forbidden:?}");
        let malformed = unary(&mut g, 9, "/Public/Orders.place", Some("t-alice"), "{\"symbol\":\"ACME\",\"limit\":12500}");
        assert_eq!((malformed.header("grpc-status"), malformed.header("grpc-message")), (Some("3"), Some("missing_field: qty")), "{malformed:?}");
        let nobody = unary(&mut g, 11, "/Public/Orders.place", Some("t-mallory"), "{\"symbol\":\"ACME\",\"qty\":10,\"limit\":12500}");
        assert_eq!((nobody.header("grpc-status"), nobody.header("grpc-message")), (Some("16"), Some("no such token")), "{nobody:?}");
        let cancelled = unary(&mut g, 13, "/Public/Orders.cancel", Some("t-alice"), "{\"order\":48}");
        assert_eq!(cancelled.header("grpc-status"), Some("0"), "{cancelled:?}");
        assert_eq!(text_of(&cancelled), "{\"order\":48,\"was_open\":true}");
    }
    // ---- ... and as protobuf (R8b), on the same connection: the messages of
    // the generated `.proto`, the same handlers, the same stream ----
    {
        use super::api_grpc_proto::{encode, framed, reflected_services, status_details, unary_pb, PROTO};
        let place = "{\"symbol\":\"ACME\",\"qty\":10,\"limit\":12500}";
        let placed = unary_pb(&mut g, 15, "/Public/Orders__place", Some("t-alice"), place, "PlaceOrder");
        assert_eq!(placed.header("grpc-status"), Some("0"), "{placed:?}");
        assert_eq!(placed.header("content-type"), Some(PROTO), "{placed:?}");
        assert_eq!(placed.data, framed(&encode("{\"order\":49,\"notional\":125000}", "OrderReceipt")), "the receipt as OrderReceipt");
        assert_eq!(dave.text(), fill(11, 49), "the fill reaches the operator's stream whichever codec placed the order");
        let missed = unary_pb(&mut g, 17, "/Public/Orders__cancel", Some("t-alice"), "{\"order\":99999}", "CancelOrder");
        let error = encode("{\"code\":\"unknown_order\",\"reason\":\"no order 99999\"}", "OrderError");
        assert_eq!(missed.header("grpc-status"), Some("9"), "{missed:?}");
        assert_eq!(
            missed.header("grpc-status-details-bin"),
            Some(status_details(9, "handler_error", "type.hale.dev/OrderError", &error).as_str()),
            "the handler's error is the Any of an OrderError"
        );
        let forbidden = unary_pb(&mut g, 19, "/Public/Orders__cancel", Some("t-bob"), "{\"order\":49}", "CancelOrder");
        assert_eq!(forbidden.header("grpc-status"), Some("7"), "{forbidden:?}");
        let malformed = unary_pb(&mut g, 21, "/Public/Orders__place", Some("t-alice"), "{\"symbol\":\"ACME\",\"limit\":12500}", "PlaceOrder");
        assert_eq!((malformed.header("grpc-status"), malformed.header("grpc-message")), (Some("3"), Some("missing_field: qty")), "{malformed:?}");
        let nobody = unary_pb(&mut g, 23, "/Public/Orders__place", Some("t-mallory"), place, "PlaceOrder");
        assert_eq!((nobody.header("grpc-status"), nobody.header("grpc-message")), (Some("16"), Some("no such token")), "{nobody:?}");
        let cancelled = unary_pb(&mut g, 25, "/Public/Orders__cancel", Some("t-alice"), "{\"order\":49}", "CancelOrder");
        assert_eq!(cancelled.header("grpc-status"), Some("0"), "{cancelled:?}");
        assert_eq!(cancelled.data, framed(&encode("{\"order\":49,\"was_open\":true}", "Cancelled")));
        // the surface is discoverable beside the rest
        assert_eq!(reflected_services(&mut g, 27, "t-alice"), ["Public", "hale.api.Description", "grpc.reflection.v1.ServerReflection"]);
    }
    dave.silence(100);

    w.server.stop_one("partner");
    wait_closed(partner, "partner's listener closed after its stop()");
    let line = unix(&w, "{\"call\":\"Ledger::rebalance\",\"payload\":{\"book\":\"main\"},\"id\":\"a-5\"}");
    assert!(line.contains("\"refusal\":{\"kind\":\"unavailable\""), "the ledger violated above, and admin says so: {line}");
    assert!(unix(&w, "{\"describe\":true}").contains("\"ok\":true"));
    w.server.stop_one("admin");
    let start = Instant::now();
    while w.server.sock().exists() {
        assert!(start.elapsed() < Duration::from_secs(10), "admin's socket removed after its stop()");
        std::thread::sleep(Duration::from_millis(10));
    }
    // the hub is a listener of its own: it served on through all three stops
    let (status, _) = get_description(w.hub, Some("t-dave"));
    assert_eq!(status, 200);

    // ---- the stop: `closed` after what was queued, then the close; every address released ----
    let (hub, mcp, grpc) = (w.hub, w.mcp, w.grpc);
    let done = w.server.finish();
    assert!(done.status.success(), "{:?}\n{}\n{}", done.status, done.stdout, done.stderr);
    assert!(std::net::TcpStream::connect(("127.0.0.1", mcp)).is_err(), "the MCP endpoint's address is released");
    assert!(std::net::TcpStream::connect(("127.0.0.1", grpc)).is_err(), "the gRPC endpoint's address is released");
    let go = g.goaway_within(Duration::from_secs(15)).expect("wire's stop says GOAWAY on the open connection");
    assert_eq!(go.1, 0, "NO_ERROR");
    for line in ["public stopped", "partner stopped", "admin stopped", "stopped"] {
        assert!(done.stdout.contains(line), "{line}:\n{}", done.stdout);
    }
    assert!(done.stdout.contains("orders_failed=1") && done.stdout.contains("ledger_failed=1"), "each violation reached its owner's on_failure once:\n{}", done.stdout);
    assert_eq!(dave.text(), "{\"type\":\"closed\",\"reason\":\"shutting_down\"}");
    assert!(matches!(dave.recv_within(5000), Frame::Close(_)));
    assert_eq!(dave.recv_within(5000), Frame::Eof);
    assert!(std::net::TcpStream::connect(("127.0.0.1", hub)).is_err(), "the hub's address is released");
}

#[test]
fn the_contracts_program_runs_clean_under_asan() {
    // the same program under AddressSanitizer with the arena's chunk recycling
    // off: both transports of the rpc half and the stream half in one process,
    // a connection that outlives its request, a subscriber that goes away, a
    // listener released at stop(), a peer locus per connection
    let bin = harness::unique_bin("api_witness_whole_asan");
    let (hub, mcp, grpc) = (ports::free_port(), ports::free_port(), ports::free_port());
    harness::build_source_asan(&witness_whole(hub, mcp, grpc), &bin);
    let w = start(&bin, hub, mcp, grpc, &[("LOTUS_NO_CHUNK_POOL", "1")]);
    let (public, partner) = (w.server.port, w.server.port2);
    let mut dave = Ws::connect(w.hub, Some("t-dave"));
    dave.subscribe("Fills");
    assert_eq!(dave.text(), "{\"type\":\"subscribed\",\"topic\":\"Fills\"}");
    let mut bob = Ws::connect(w.hub, Some("t-bob"));
    bob.subscribe("Fills");
    assert!(bob.text().contains("\"kind\":\"unauthorized\""));
    assert_eq!(place(public, "t-alice").status, 200);
    assert_eq!(dave.text(), fill(1, 41));
    assert_eq!(place(partner, "t-carol").status, 200);
    assert_eq!(dave.text(), fill(2, 9001));
    assert_eq!(request(public, "GET", "/.description", &[bearer("t-alice")], None).status, 200);
    assert_eq!(get_description(w.hub, Some("t-dave")).0, 200);
    assert_eq!(request(partner, "POST", "/call/Orders::place", &[bearer("t-carol")], Some("{\"symbol\":\"ACME\",\"qty\":50000,\"limit\":1}")).status, 500);
    dave.send_text("{\"type\":\"call\",\"id\":\"c1\",\"call\":\"Orders::place\",\"payload\":{\"symbol\":\"ACME\",\"qty\":10,\"limit\":12500}}");
    let (mut got_reply, mut got_event) = (false, false);
    while !(got_reply && got_event) {
        let f = dave.text();
        if f.contains("\"type\":\"event\"") {
            got_event = true;
        } else {
            assert!(f.contains("\"id\":\"c1\",\"ok\":true"), "{f}");
            got_reply = true;
        }
    }
    send(partner, "POST /call/Orders::place HTTP/1.1\r\nContent-Length: 100\r\n\r\n{\"sym").close();
    let line = unix(&w, "{\"describe\":true}");
    assert!(line.contains("\"ok\":true"), "{line}");
    let json = [h("Content-Type", "application/json"), bearer("t-alice")];
    let tools = request(w.mcp, "POST", "/mcp", &json, Some(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#));
    assert_eq!(tools.status, 200, "{}", tools.body);
    let placed = request(
        w.mcp,
        "POST",
        "/mcp",
        &json,
        Some(r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"Orders__place","arguments":{"symbol":"ACME","qty":10,"limit":12500}}}"#),
    );
    assert!(placed.body.contains(r#""isError":false"#), "{}", placed.body);
    {
        let mut g = super::api_grpc::h2_client::Client::connect(w.grpc);
        let r = super::api_grpc::unary(&mut g, 1, "/Public/Orders.place", Some("t-alice"), "{\"symbol\":\"ACME\",\"qty\":10,\"limit\":12500}");
        assert_eq!(r.header("grpc-status"), Some("0"), "{r:?}");
        let r = super::api_grpc::unary(&mut g, 3, "/Public/Orders.cancel", Some("t-bob"), "{\"order\":41}");
        assert_eq!(r.header("grpc-status"), Some("7"), "{r:?}");
    }
    send(w.mcp, "POST /mcp HTTP/1.1\r\nContent-Length: 100\r\n\r\n{\"json").close();
    let second = Ws::connect(w.hub, Some("t-dave"));
    second.abandon();
    let done = w.server.finish();
    for bad in ["AddressSanitizer", "LeakSanitizer", "heap-use-after-free", "SUMMARY:"] {
        assert!(!done.stderr.contains(bad), "{bad} in:\n{}", done.stderr);
    }
    assert!(done.status.success(), "{:?}\n{}\n{}", done.status, done.stdout, done.stderr);
}

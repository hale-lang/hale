//! GH #1530: `http::Rest`, the REST transport of `api::serve`.
//!
//! One program serves the route shape a hand-written router of an
//! OpenAI-compatible gateway has (`GET /healthz`, `GET /readyz`, `POST
//! /v1/chat/completions`, a capture under `/v1/models/`, a literal beside a
//! capture, a create answered `201`, a delete answered `204`) from one
//! surface, over `http::Rest`, and each status the transport owns is
//! driven by a real connection: 200, 201, 204, the handler error's class
//! (404, 409), 400 for a parameter that does not bind, 401, 403, 404 for a
//! path no route serves, 405 with `Allow`, `OPTIONS`, `HEAD` and CORS; the
//! body cap the transport sets (413), and the cross-origin write guard (415
//! for a body that is not JSON, 403 for a bodiless write from an origin
//! `cors` does not list).

use std::path::PathBuf;
use std::sync::OnceLock;

#[path = "support/build.rs"]
mod build_opts;
#[path = "support/harness.rs"]
mod harness;
#[path = "support/http_rpc.rs"]
mod http_rpc;

use http_rpc::*;

const PROGRAM: &str = r#"
role operator;

type Health { status: String; }
type ChatAsk { model: String; prompt: String; }
type ChatReply { model: String; text: String; }
type ModelAsk { model: String; }
type ModelInfo { id: String; }
type ModelList { count: Int; }
type ItemNew { name: String; }
type ItemKey { id: Int; }
type ItemPut { id: Int; name: String; }
type Item { id: Int; name: String; }
type Search { q: String; limit: Int; exact: Bool; }
type Found { q: String; limit: Int; exact: Bool; }
type Summary { total: Int; }
type DrainAsk { reason: String; }
type Drained { reason: String; }
type ItemError = enum { @not_found Missing, @conflict Taken, @invalid Bad };

api Gateway {
    rpc Probe::live at "GET /healthz";
    rpc Probe::ready at "GET /readyz";
    rpc Chat::complete at "POST /v1/chat/completions";
    rpc Chat::model at "GET /v1/models/{model}";
    rpc Chat::models at "GET /v1/models";
    rpc Items::create at "POST /v1/items" status: 201;
    rpc Items::summary at "GET /v1/items/summary";
    rpc Items::search at "GET /v1/search";
    rpc Items::get at "GET /v1/items/{id}";
    rpc Items::replace at "PUT /v1/items/{id}";
    rpc Items::remove at "DELETE /v1/items/{id}" status: 204;
    rpc Ops::retire at "POST /admin/retire" requires: [operator];
    rpc Ops::plain;
}

locus Tokens {
    fn principal(token: String) -> std::api::Principal {
        if token == "t-user" { return std::api::Principal { mode: "bearer", name: "user" }; }
        if token == "t-root" { return std::api::Principal { mode: "bearer", name: "root" }; }
        return std::api::Principal { mode: "bearer", name: "" };
    }
    fn refused() -> String { return "no such token"; }
}

locus Grants {
    fn holds(p: std::api::Principal, r: String) -> Bool {
        if r == "operator" { return p.name == "root"; }
        return false;
    }
}

locus Probe {
    fn live() -> Health { return Health { status: "live" }; }
    fn ready() -> Health { return Health { status: "ready" }; }
}

locus Chat {
    fn complete(a: ChatAsk) -> ChatReply { return ChatReply { model: a.model, text: "re: " + a.prompt }; }
    fn model(a: ModelAsk) -> ModelInfo { return ModelInfo { id: a.model }; }
    fn models() -> ModelList { return ModelList { count: 2 }; }
}

locus Items {
    fn create(n: ItemNew) -> Item fallible(ItemError) {
        if n.name == "dup" { fail ItemError::Taken; }
        if n.name == "" { fail ItemError::Bad; }
        return Item { id: 7, name: n.name };
    }
    fn search(s: Search) -> Found { return Found { q: s.q, limit: s.limit, exact: s.exact }; }
    fn summary() -> Summary { return Summary { total: 3 }; }
    fn get(k: ItemKey) -> Item fallible(ItemError) {
        if k.id == 404 { fail ItemError::Missing; }
        return Item { id: k.id, name: "item" };
    }
    fn replace(p: ItemPut) -> Item { return Item { id: p.id, name: p.name }; }
    fn remove(k: ItemKey) fallible(ItemError) {
        if k.id == 404 { fail ItemError::Missing; }
    }
}

locus Ops {
    fn retire(d: DrainAsk) -> Drained { return Drained { reason: d.reason }; }
    fn plain(d: DrainAsk) -> Drained { return Drained { reason: "plain " + d.reason }; }
}

main locus Svc {
    params {
        bearer: Tokens = Tokens { };
        grants: Grants = Grants { };
        probe: Probe = Probe { };
        chat: Chat = Chat { };
        items: Items = Items { };
        ops: Ops = Ops { };
        web: api::Exposure = api::serve(Gateway, http::Rest { bind: std::env::var("BIND"), codec: json, principals: self.bearer, roles: self.grants, anonymous: "public", cors: "https://app.example", max_request_bytes: 2097152 }, as: "web", bound: 16, on_full: refuse);
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
            let bin = harness::unique_bin("api_rest");
            build_opts::build_source(PROGRAM, &bin, &build_opts::options())
                .expect("build the REST program");
            bin
        })
        .clone()
}

fn get(port: u16, path: &str) -> Response {
    request(port, "GET", path, &[], None)
}

/// The methods of an `Allow` header, as a set.
fn allowed(r: &Response) -> std::collections::BTreeSet<String> {
    r.header("Allow")
        .unwrap_or("")
        .split(", ")
        .map(str::to_string)
        .collect()
}

fn set(methods: &[&str]) -> std::collections::BTreeSet<String> {
    methods.iter().map(|m| m.to_string()).collect()
}

/// What a `HEAD` is answered with, read to the end of the connection: the
/// shared client reads a `Content-Length` as the body's, which a HEAD's is not.
fn head_of(port: u16, path: &str) -> String {
    use std::io::{Read, Write};
    let mut c = std::net::TcpStream::connect(("127.0.0.1", port)).expect("connect");
    c.set_read_timeout(Some(std::time::Duration::from_secs(10)))
        .unwrap();
    c.write_all(format!("HEAD {path} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes())
        .unwrap();
    let mut out = Vec::new();
    c.read_to_end(&mut out).expect("read the reply");
    String::from_utf8_lossy(&out).into_owned()
}

fn as_user(port: u16, method: &str, path: &str, body: Option<&str>) -> Response {
    request(port, method, path, &[bearer("t-user")], body)
}

#[test]
fn the_hand_written_routers_routes_are_served_from_one_surface() {
    let server = Server::start(&program(), &[]);
    server.ready();
    let p = server.port;

    // the probes: no credential, the anonymous caller
    let live = get(p, "/healthz");
    assert_eq!(
        (live.status, live.body.as_str()),
        (200, "{\"status\":\"live\"}"),
        "{}",
        live.body
    );
    let ready = get(p, "/readyz");
    assert_eq!(
        (ready.status, ready.body.as_str()),
        (200, "{\"status\":\"ready\"}")
    );
    assert_eq!(ready.header("Content-Type"), Some("application/json"));

    // the body is the request
    let chat = as_user(
        p,
        "POST",
        "/v1/chat/completions",
        Some("{\"model\":\"m1\",\"prompt\":\"hi\"}"),
    );
    assert_eq!(
        (chat.status, chat.body.as_str()),
        (200, "{\"model\":\"m1\",\"text\":\"re: hi\"}"),
        "{}",
        chat.body
    );

    // a path parameter is the request's field, and is percent-decoded
    let model = as_user(p, "GET", "/v1/models/gpt%2D4", None);
    assert_eq!(
        (model.status, model.body.as_str()),
        (200, "{\"id\":\"gpt-4\"}"),
        "{}",
        model.body
    );
    // a request-less handler
    let models = as_user(p, "GET", "/v1/models", None);
    assert_eq!(
        (models.status, models.body.as_str()),
        (200, "{\"count\":2}")
    );
    // a literal wins over a capture in its position
    let summary = as_user(p, "GET", "/v1/items/summary", None);
    assert_eq!(
        (summary.status, summary.body.as_str()),
        (200, "{\"total\":3}"),
        "{}",
        summary.body
    );

    // a path parameter is an integer
    let item = as_user(p, "GET", "/v1/items/12?unknown=1", None);
    assert_eq!(
        (item.status, item.body.as_str()),
        (200, "{\"id\":12,\"name\":\"item\"}"),
        "{}",
        item.body
    );
    // the query binds the request's scalar fields by kind, percent-decoded
    let found = as_user(p, "GET", "/v1/search?q=a%20b+c&limit=5&exact=true", None);
    assert_eq!(
        (found.status, found.body.as_str()),
        (200, "{\"q\":\"a b c\",\"limit\":5,\"exact\":true}"),
        "{}",
        found.body
    );
    // a field the request needs and the query lacks is the codec's malformed
    let short = as_user(p, "GET", "/v1/search?q=x", None);
    assert_eq!(short.status, 400, "{}", short.body);
    assert!(short.body.contains("missing_field"), "{}", short.body);
    // path and body together
    let put = as_user(p, "PUT", "/v1/items/9", Some("{\"name\":\"nine\"}"));
    assert_eq!(
        (put.status, put.body.as_str()),
        (200, "{\"id\":9,\"name\":\"nine\"}"),
        "{}",
        put.body
    );

    // the row's success status
    let created = as_user(p, "POST", "/v1/items", Some("{\"name\":\"fresh\"}"));
    assert_eq!(created.status, 201, "{}", created.body);
    assert_eq!(created.body, "{\"id\":7,\"name\":\"fresh\"}");
    let removed = as_user(p, "DELETE", "/v1/items/5", None);
    assert_eq!(
        (removed.status, removed.body.as_str()),
        (204, ""),
        "a 204 has no body"
    );

    // the handler's error, by its class
    let missing = as_user(p, "GET", "/v1/items/404", None);
    assert_eq!(missing.status, 404, "{}", missing.body);
    assert_eq!(missing.header("Hale-Error-Class"), Some("not_found"));
    let taken = as_user(p, "POST", "/v1/items", Some("{\"name\":\"dup\"}"));
    assert_eq!(taken.status, 409, "{}", taken.body);
    let bad = as_user(p, "POST", "/v1/items", Some("{\"name\":\"\"}"));
    assert_eq!(bad.status, 422, "{}", bad.body);
    assert_eq!(as_user(p, "DELETE", "/v1/items/404", None).status, 404);

    // a parameter that does not bind is malformed, before the handler
    let notint = as_user(p, "GET", "/v1/items/abc", None);
    assert_eq!(notint.status, 400, "{}", notint.body);
    assert!(
        notint.body.contains("\"kind\":\"malformed\"")
            && notint.body.contains("path parameter `id` is not a integer"),
        "{}",
        notint.body
    );
    let notbool = as_user(p, "GET", "/v1/search?q=x&limit=1&exact=maybe", None);
    assert_eq!(notbool.status, 400, "{}", notbool.body);

    // a row with no route is the member's `POST /call/<member>`
    let plain = as_user(p, "POST", "/call/Ops::plain", Some("{\"reason\":\"r\"}"));
    assert_eq!(
        (plain.status, plain.body.as_str()),
        (200, "{\"reason\":\"plain r\"}"),
        "{}",
        plain.body
    );

    // the same admission as http::Rpc: a role, a bad token, no token on a
    // row that needs a caller of a role
    let drain = |token: Option<&str>| {
        let headers: Vec<(String, String)> = token.map(bearer).into_iter().collect();
        request(
            p,
            "POST",
            "/admin/retire",
            &headers,
            Some("{\"reason\":\"x\"}"),
        )
    };
    assert_eq!(drain(Some("t-root")).status, 200);
    let denied = drain(Some("t-user"));
    assert_eq!(denied.status, 403, "{}", denied.body);
    assert!(
        denied.body.contains("\"requires\":[\"operator\"]"),
        "{}",
        denied.body
    );
    assert_eq!(
        drain(None).status,
        403,
        "the anonymous caller holds no role"
    );
    let wrong = request(p, "GET", "/v1/models", &[bearer("t-nope")], None);
    assert_eq!(wrong.status, 401, "{}", wrong.body);
    assert!(wrong.body.contains("no such token"), "{}", wrong.body);
    // a digest the surface does not have is refused as ever
    let stale = request(
        p,
        "GET",
        "/healthz",
        &[h("Hale-Surface-Digest", "fnv1a64:0000000000000000")],
        None,
    );
    assert_eq!(stale.status, 409, "{}", stale.body);

    assert!(server.finish().status.success());
}

#[test]
fn no_route_is_404_a_route_of_another_method_is_405_with_allow() {
    let server = Server::start(&program(), &[]);
    server.ready();
    let p = server.port;

    let none = as_user(p, "GET", "/v1/nothing", None);
    assert_eq!(none.status, 404, "{}", none.body);
    assert!(
        none.body.contains("\"kind\":\"not_found\""),
        "{}",
        none.body
    );

    let wrong = as_user(p, "DELETE", "/v1/chat/completions", None);
    assert_eq!(wrong.status, 405, "{}", wrong.body);
    assert_eq!(allowed(&wrong), set(&["POST", "OPTIONS"]));
    let item = as_user(p, "PATCH", "/v1/items/3", None);
    assert_eq!(item.status, 405, "{}", item.body);
    assert_eq!(
        allowed(&item),
        set(&["GET", "HEAD", "PUT", "DELETE", "OPTIONS"])
    );
    // the Rpc path is not a route here
    assert_eq!(as_user(p, "GET", "/call/Ops::plain", None).status, 405);
    assert!(server.finish().status.success());
}

#[test]
fn options_and_head_are_answered_from_the_table() {
    let server = Server::start(&program(), &[]);
    server.ready();
    let p = server.port;

    // OPTIONS names what the path takes, needs no credential
    let opts = request(p, "OPTIONS", "/v1/items/3", &[], None);
    assert_eq!((opts.status, opts.body.as_str()), (204, ""));
    assert_eq!(
        allowed(&opts),
        set(&["GET", "HEAD", "PUT", "DELETE", "OPTIONS"])
    );
    assert_eq!(request(p, "OPTIONS", "/nowhere", &[], None).status, 404);

    // HEAD is the GET without its body, with the body's length
    let got = get(p, "/healthz");
    let head = head_of(p, "/healthz");
    assert!(head.starts_with("HTTP/1.1 200 OK\r\n"), "{head:?}");
    assert!(
        head.contains(&format!("Content-Length: {}\r\n", got.body.len())),
        "{head:?}"
    );
    assert!(
        head.ends_with("\r\n\r\n"),
        "a HEAD reply has no body: {head:?}"
    );
    // a path with no GET has no HEAD
    assert!(head_of(p, "/v1/chat/completions").starts_with("HTTP/1.1 405 "));
    assert!(server.finish().status.success());
}

#[test]
fn cors_is_answered_against_the_listed_origins() {
    let server = Server::start(&program(), &[]);
    server.ready();
    let p = server.port;
    let pre = |origin: &str| {
        request(
            p,
            "OPTIONS",
            "/v1/chat/completions",
            &[
                h("Origin", origin),
                h("Access-Control-Request-Method", "POST"),
            ],
            None,
        )
    };

    let ok = pre("https://app.example");
    assert_eq!(ok.status, 204, "{}", ok.body);
    assert_eq!(
        ok.header("Access-Control-Allow-Origin"),
        Some("https://app.example")
    );
    assert_eq!(
        ok.header("Access-Control-Allow-Methods"),
        Some("POST, OPTIONS"),
        "the methods of the route, not of the transport"
    );
    assert_eq!(
        ok.header("Access-Control-Allow-Headers"),
        Some("Authorization, Content-Type, Hale-Surface-Digest")
    );
    assert_eq!(ok.header("Vary"), Some("Origin"));

    let other = pre("https://evil.example");
    assert_eq!(other.status, 403, "{}", other.body);
    assert_eq!(other.header("Access-Control-Allow-Origin"), None);

    // every other answer to a listed origin carries the origin, refusals and 404 included
    let origin = h("Origin", "https://app.example");
    let live = request(p, "GET", "/healthz", &[origin.clone()], None);
    assert_eq!(live.status, 200);
    assert_eq!(
        live.header("Access-Control-Allow-Origin"),
        Some("https://app.example")
    );
    let lost = request(p, "GET", "/nothing", &[origin.clone()], None);
    assert_eq!(lost.status, 404);
    assert_eq!(
        lost.header("Access-Control-Allow-Origin"),
        Some("https://app.example")
    );
    assert!(server.finish().status.success());
}

#[test]
fn the_description_is_served_beside_the_routes() {
    let server = Server::start(&program(), &[]);
    server.ready();
    let d = request(server.port, "GET", "/.description", &[], None);
    assert_eq!(d.status, 200, "{}", d.body);
    assert!(
        d.body.contains("Probe::live") && d.body.contains("Items::remove"),
        "{}",
        d.body
    );
    assert!(server.finish().status.success());
}

/// `max_request_bytes` on the transport is the listener's cap: a body under it
/// past the 1 MiB default is served, and a declared length over it is refused
/// `413` before a byte of the body is sent.
#[test]
fn the_transport_sets_the_body_cap() {
    let server = Server::start(&program(), &[]);
    server.ready();
    let p = server.port;
    let prompt = "x".repeat(1_536 * 1024);
    let big = as_user(p, "POST", "/v1/chat/completions", Some(&format!("{{\"model\":\"m\",\"prompt\":\"{prompt}\"}}")));
    assert_eq!(big.status, 200, "1.5 MiB is under a 2 MiB cap: {}", &big.body[..big.body.len().min(200)]);
    assert!(big.body.ends_with("xxx\"}"), "the prompt came whole");

    // the head alone, declaring 3 MiB: the refusal comes without the body
    let over = send(p, "POST /v1/chat/completions HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nContent-Length: 3145728\r\n\r\n")
        .response_within(10_000)
        .expect("refused before the body is read");
    assert_eq!(over.status, 413, "{}", over.body);
    assert!(over.body.contains("\"kind\":\"too_large\""), "{}", over.body);
    assert!(over.body.contains("at most 2097152 bytes"), "{}", over.body);
    assert!(server.finish().status.success());
}

/// The cross-origin write guard: a cross-site form's POST (a `text/plain`
/// or form-encoded body, which a browser sends with no preflight) is refused
/// `415`; a write with no body from an origin `cors` does not list is `403`.
#[test]
fn a_cross_site_write_is_refused() {
    let server = Server::start(&program(), &[]);
    server.ready();
    let p = server.port;
    let body = "{\"model\":\"m1\",\"prompt\":\"hi\"}";
    for ct in ["text/plain", "application/x-www-form-urlencoded", "multipart/form-data; boundary=x"] {
        let form = request(
            p,
            "POST",
            "/v1/chat/completions",
            &[bearer("t-user"), h("Origin", "https://evil.example"), h("Content-Type", ct)],
            Some(body),
        );
        assert_eq!(form.status, 415, "{ct}: {}", form.body);
        assert!(form.body.contains("\"kind\":\"unsupported_media_type\""), "{}", form.body);
    }
    // a body with no type at all is refused the same way
    let untyped = send(p, &format!("POST /v1/chat/completions HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer t-user\r\nContent-Length: {}\r\n\r\n{body}", body.len()))
        .response_within(10_000)
        .expect("a reply");
    assert_eq!(untyped.status, 415, "{}", untyped.body);
    // the codec's type, its case and parameters aside, is served
    let typed = request(p, "POST", "/v1/chat/completions", &[bearer("t-user"), h("Content-Type", "Application/JSON; charset=utf-8")], Some(body));
    assert_eq!(typed.status, 200, "{}", typed.body);

    // a bodiless write: refused to an unlisted origin, served to a listed
    // one and to a caller that sends none (not a browser)
    let evil = request(p, "DELETE", "/v1/items/5", &[bearer("t-user"), h("Origin", "https://evil.example")], None);
    assert_eq!(evil.status, 403, "{}", evil.body);
    assert!(evil.body.contains("\"kind\":\"unauthorized\""), "{}", evil.body);
    assert_eq!(evil.header("Access-Control-Allow-Origin"), None);
    let listed = request(p, "DELETE", "/v1/items/5", &[bearer("t-user"), h("Origin", "https://app.example")], None);
    assert_eq!(listed.status, 204, "{}", listed.body);
    let none = request(p, "DELETE", "/v1/items/5", &[bearer("t-user")], None);
    assert_eq!(none.status, 204, "{}", none.body);
    // a read is never a write: a GET from any origin is served
    let read = request(p, "GET", "/healthz", &[h("Origin", "https://evil.example")], None);
    assert_eq!(read.status, 200, "{}", read.body);
    assert!(server.finish().status.success());
}

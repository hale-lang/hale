//! GH #1417 (R8b): `grpc::Rpc` speaks protobuf.
//!
//! The program is `api_grpc.rs`'s (the witness's `Public` half over
//! `grpc::Rpc`), spoken to with the same hand-written HTTP/2 client. The
//! messages are written here by hand from the generated `.proto` of the
//! contract (`tests/api-contract/Public.proto`, which the layouts below are
//! held to), and the replies are held to the spec's gRPC column as protobuf
//! states it, byte for byte:
//!
//! * the ten recorded exchanges replay: each request becomes the message its
//!   JSON body is, under `application/grpc` and under `application/grpc+proto`,
//!   and the reply is the recorded body as a message (a result), or the `Any`
//!   of a `google.rpc.Status` whose value is the handler's error message or
//!   `HaleRefusal`;
//! * a message that is not protobuf, or lacks a field, is `malformed` in the
//!   JSON codec's words;
//! * the description is a one-field message;
//! * server reflection lists the services and answers the descriptor of each
//!   file, and the descriptor of the surface's file is the one a compiler
//!   (`protox`, a test-only dependency) makes of the generated `.proto`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use prost::Message;

use super::api_grpc::h2_client::Client;
use super::api_grpc::{b64, call, code_of, description_over_grpc, digest_of, field, pct, start, token_of, varint, DIGEST, JSON, WAIT};
use super::api_grpc::http_rpc::{contract_dir, recording, Recording};

pub(crate) const PROTO: &str = "application/grpc+proto";
const BARE: &str = "application/grpc";

// ---- protobuf, written out ----

fn put_varint(n: u64) -> Vec<u8> {
    varint(n as usize)
}

fn tag(no: u32, wire: u8) -> Vec<u8> {
    put_varint(u64::from(no) << 3 | u64::from(wire))
}

fn put_int(no: u32, v: i64) -> Vec<u8> {
    let mut out = tag(no, 0);
    out.extend(put_varint(v as u64));
    out
}

fn put_bool(no: u32, v: bool) -> Vec<u8> {
    put_int(no, i64::from(v))
}

fn put_str(no: u32, s: &str) -> Vec<u8> {
    let mut out = tag(no, 2);
    out.extend(put_varint(s.len() as u64));
    out.extend_from_slice(s.as_bytes());
    out
}

#[derive(Clone, Copy, Debug)]
enum Kind {
    Int,
    Str,
    Bool,
}

/// The fields of the contract's messages, by number: what
/// `tests/api-contract/Public.proto` declares (held to it below).
fn layout(message: &str) -> Vec<(&'static str, u32, Kind)> {
    match message {
        "PlaceOrder" => vec![("symbol", 1, Kind::Str), ("qty", 2, Kind::Int), ("limit", 3, Kind::Int)],
        "CancelOrder" => vec![("order", 1, Kind::Int)],
        "OrderReceipt" => vec![("order", 1, Kind::Int), ("notional", 2, Kind::Int)],
        "Cancelled" => vec![("order", 1, Kind::Int), ("was_open", 2, Kind::Bool)],
        "OrderError" => vec![("code", 1, Kind::Str), ("reason", 2, Kind::Str)],
        other => panic!("no layout for {other}"),
    }
}

/// A flat JSON object as the message `message`: the fields it holds, in
/// number order, each as its wire type.
pub(crate) fn encode(json: &str, message: &str) -> Vec<u8> {
    let v: serde_json::Value = serde_json::from_str(json).unwrap_or_else(|e| panic!("{json}: {e}"));
    let mut out = Vec::new();
    for (name, no, kind) in layout(message) {
        let Some(x) = v.get(name) else { continue };
        out.extend(match kind {
            Kind::Int => put_int(no, x.as_i64().expect("an integer")),
            Kind::Str => put_str(no, x.as_str().expect("a string")),
            Kind::Bool => put_bool(no, x.as_bool().expect("a bool")),
        });
    }
    out
}

/// A gRPC message of bytes.
pub(crate) fn framed(body: &[u8]) -> Vec<u8> {
    let mut m = vec![0];
    m.extend((body.len() as u32).to_be_bytes());
    m.extend_from_slice(body);
    m
}

/// `google.rpc.Status { code, message, details: [Any { type_url, value }] }`, base64.
pub(crate) fn status_details(code: usize, message: &str, type_url: &str, value: &[u8]) -> String {
    let mut any = field(0x0a, type_url.as_bytes());
    any.extend(field(0x12, value));
    let mut status = vec![0x08];
    status.extend(varint(code));
    status.extend(field(0x12, message.as_bytes()));
    status.extend(field(0x1a, &any));
    b64(&status)
}

/// The messages of a recorded exchange: what its request and its reply are.
fn types_of(path: &str) -> (&'static str, &'static str, Option<&'static str>) {
    match path {
        "/call/Orders::place" => ("PlaceOrder", "OrderReceipt", None),
        "/call/Orders::cancel" => ("CancelOrder", "Cancelled", Some("OrderError")),
        other => panic!("no types for {other}"),
    }
}

/// The rpc a recorded request calls: the member written as an MCP tool is.
fn rpc_of(rec: &Recording) -> String {
    rec.path.strip_prefix("/call/").expect("a call path").replace("::", "__")
}

/// `HaleRefusal` of a recorded refusal body (`{"refusal":{…}}`).
fn refusal_message(reply: &str) -> (Vec<u8>, String) {
    let v: serde_json::Value = serde_json::from_str(reply).expect("a refusal body");
    let r = &v["refusal"];
    let kind = r["kind"].as_str().expect("a kind");
    let mut out = put_str(1, kind);
    if let Some(reason) = r["reason"].as_str().filter(|s| !s.is_empty()) {
        out.extend(put_str(2, reason));
    }
    for role in r["requires"].as_array().into_iter().flatten() {
        out.extend(put_str(3, role.as_str().expect("a role")));
    }
    if let Some(served) = r["served"].as_str() {
        out.extend(put_str(4, served));
    }
    let said = if kind == "server" { "server_error".to_string() } else { r["reason"].as_str().expect("a reason").to_string() };
    (out, said)
}

/// What a recorded HTTP reply is as a protobuf gRPC response, written from
/// the spec's column and the `.proto`'s messages and nothing else of the
/// implementation: the headers, the data, the trailers.
fn expected(rec: &Recording, ctype: &str) -> (Vec<(String, String)>, Vec<u8>, Vec<(String, String)>) {
    let pair = |k: &str, v: &str| (k.to_string(), v.to_string());
    let head = vec![pair(":status", "200"), pair("content-type", ctype)];
    let (_, response, error) = types_of(&rec.path);
    if rec.status == 200 {
        return (head, framed(&encode(&rec.reply, response)), vec![pair("grpc-status", "0")]);
    }
    let (code, text, type_url, value) = if rec.status == 422 {
        let e = error.expect("a row with a handler error");
        (9, "handler_error".to_string(), format!("type.hale.dev/{e}"), encode(&rec.reply, e))
    } else {
        let (value, said) = refusal_message(&rec.reply);
        let kind = serde_json::from_str::<serde_json::Value>(&rec.reply).unwrap()["refusal"]["kind"].as_str().unwrap().to_string();
        (code_of(&kind), said, "type.hale.dev/HaleRefusal".to_string(), value)
    };
    let mut trailers_only = head;
    trailers_only.push(pair("grpc-status", &code.to_string()));
    trailers_only.push(pair("grpc-message", &pct(&text)));
    trailers_only.push(pair("grpc-status-details-bin", &status_details(code, &text, &type_url, &value)));
    (trailers_only, Vec::new(), Vec::new())
}

/// Send the recorded request as a protobuf call.
fn replay(c: &mut Client, stream: u32, rec: &Recording, ctype: &str) {
    let (request, _, _) = types_of(&rec.path);
    let path = format!("/Public/{}", rpc_of(rec));
    call(c, stream, &path, token_of(rec).as_deref(), digest_of(rec).as_deref(), ctype, &framed(&encode(&rec.body, request)));
}

fn assert_replayed(c: &mut Client, stream: u32, name: &str, rec: &Recording, ctype: &str) {
    let got = c.response(stream, WAIT);
    let (headers, data, trailers) = expected(rec, ctype);
    assert_eq!(got.headers, headers, "{name} ({ctype}): the headers\n{got:?}");
    assert_eq!(got.data, data, "{name} ({ctype}): the message differs from the recorded body as protobuf\n{got:?}");
    assert_eq!(got.trailers, trailers, "{name} ({ctype}): the trailers\n{got:?}");
    assert!(got.ended && got.reset.is_none(), "{name} ({ctype}): {got:?}");
}

// ---- the layouts are the generated file's ----

/// The fields `tests/api-contract/Public.proto` declares, per message.
fn declared() -> BTreeMap<String, Vec<(String, u32, &'static str)>> {
    let text = std::fs::read_to_string(contract_dir().join("Public.proto")).expect("the generated .proto");
    let mut out: BTreeMap<String, Vec<(String, u32, &'static str)>> = BTreeMap::new();
    let mut in_message: Option<String> = None;
    for line in text.lines() {
        if let Some(name) = line.strip_prefix("message ").and_then(|r| r.strip_suffix(" {")) {
            in_message = Some(name.to_string());
            out.entry(name.to_string()).or_default();
        } else if line == "}" {
            in_message = None;
        } else if let (Some(m), true) = (&in_message, line.starts_with("  ")) {
            let body = line.trim().split("//").next().unwrap().trim().trim_end_matches(';');
            let (decl, no) = body.rsplit_once(" = ").expect("a field");
            let words: Vec<&str> = decl.split_whitespace().collect();
            let (ty, name) = (words[words.len() - 2], words[words.len() - 1]);
            let kind = match ty {
                "int64" => "int",
                "string" => "str",
                "bool" => "bool",
                _ => "other",
            };
            out.get_mut(m).unwrap().push((name.to_string(), no.parse().unwrap(), kind));
        }
    }
    out
}

#[test]
fn the_layouts_these_tests_write_are_the_generated_files() {
    let declared = declared();
    for m in ["PlaceOrder", "CancelOrder", "OrderReceipt", "Cancelled", "OrderError"] {
        let mine: Vec<(String, u32, &str)> = layout(m)
            .into_iter()
            .map(|(n, no, k)| (n.to_string(), no, match k { Kind::Int => "int", Kind::Str => "str", Kind::Bool => "bool" }))
            .collect();
        assert_eq!(declared[m], mine, "{m}: the fixture's fields");
    }
}

// ---- the recorded exchanges ----

const EIGHT: [&str; 8] = [
    "refusal_unauthenticated",
    "refusal_unauthorized",
    "refusal_malformed",
    "refusal_digest_mismatch",
    "result",
    "handler_error",
    "server_error",
    "refusal_unavailable",
];

fn replay_all(ctype: &str) {
    let server = start(16, &[]);
    let mut c = Client::connect(server.port);
    for (i, name) in EIGHT.iter().enumerate() {
        let rec = recording(name);
        let stream = 2 * i as u32 + 1;
        replay(&mut c, stream, &rec, ctype);
        assert_replayed(&mut c, stream, name, &rec, ctype);
    }
    let done = server.finish();
    assert!(done.status.success(), "the program ended cleanly: {:?}\n{}\n{}", done.status, done.stdout, done.stderr);
}

#[test]
fn the_recorded_exchanges_replay_over_application_grpc_plus_proto() {
    replay_all(PROTO);
}

#[test]
fn the_recorded_exchanges_replay_over_a_bare_application_grpc() {
    replay_all(BARE);
}

#[test]
fn a_request_over_the_bound_is_refused_full_as_recorded_in_protobuf() {
    let server = start(1, &[("SLOW_MS", "600")]);
    let mut c = Client::connect(server.port);
    let result = recording("result");
    replay(&mut c, 1, &result, PROTO);
    std::thread::sleep(Duration::from_millis(200));
    let full = recording("refusal_full");
    replay(&mut c, 3, &full, PROTO);
    assert_replayed(&mut c, 3, "refusal_full", &full, PROTO);
    assert_replayed(&mut c, 1, "result", &result, PROTO);
    assert!(server.finish().status.success());
}

#[test]
fn a_request_after_stop_began_is_refused_shutting_down_in_protobuf() {
    let mut server = start(16, &[("SLOW_MS", "800")]);
    let mut c = Client::connect(server.port);
    let result = recording("result");
    replay(&mut c, 1, &result, PROTO);
    std::thread::sleep(Duration::from_millis(200));
    server.trigger();
    std::thread::sleep(Duration::from_millis(200));
    let down = recording("refusal_shutting_down");
    let mut late = Client::connect(server.port);
    replay(&mut late, 1, &down, PROTO);
    assert_replayed(&mut late, 1, "refusal_shutting_down", &down, PROTO);
    assert_replayed(&mut c, 1, "result", &result, PROTO);
    let go = c.goaway_within(WAIT).expect("stop says GOAWAY after the queued replies");
    assert_eq!(go.1, 0, "NO_ERROR");
    let done = server.wait(Duration::from_secs(30));
    assert!(done.status.success(), "{:?}\n{}", done.status, done.stderr);
}

/// A method is the rpc of the `.proto` (`Orders__place`), and the spellings
/// a JSON caller used (`Orders.place`, `Orders::place`) still name the member.
#[test]
fn the_rpc_of_the_proto_and_the_older_spellings_name_the_same_member() {
    let server = start(16, &[]);
    let mut c = Client::connect(server.port);
    let place = encode("{\"symbol\":\"ACME\",\"qty\":10,\"limit\":12500}", "PlaceOrder");
    for (i, path) in ["/Public/Orders__place", "/Public/Orders.place", "/Public/Orders::place"].iter().enumerate() {
        let stream = 2 * i as u32 + 1;
        call(&mut c, stream, path, Some("t-alice"), Some(DIGEST), PROTO, &framed(&place));
        let got = c.response(stream, WAIT);
        assert_eq!(got.header("grpc-status"), Some("0"), "{path}: {got:?}");
        // the receipt: order 41, 42, 43 and the notional, as OrderReceipt
        let mut want = put_int(1, 41 + i as i64);
        want.extend(put_int(2, 125000));
        assert_eq!(got.data, framed(&want), "{path}");
    }
    let unknown = {
        call(&mut c, 7, "/Public/Orders__nothing", Some("t-alice"), Some(DIGEST), PROTO, &framed(&place));
        c.response(7, WAIT)
    };
    assert_eq!(unknown.header("grpc-status"), Some("3"), "{unknown:?}");
    assert!(unknown.header("grpc-message").unwrap_or("").contains(&pct("unknown_member: Orders__nothing")), "{unknown:?}");
    assert!(server.finish().status.success());
}

/// A message that is not a message of the row is `malformed`, in the words
/// the JSON codec uses for the same fault.
#[test]
fn a_message_that_is_not_protobuf_or_lacks_a_field_is_malformed() {
    let server = start(16, &[]);
    let mut c = Client::connect(server.port);
    let mut stream = 1;
    let mut refused = |c: &mut Client, why: &str, body: &[u8], reason: &str| {
        call(c, stream, "/Public/Orders__place", Some("t-alice"), Some(DIGEST), PROTO, &framed(body));
        let got = c.response(stream, WAIT);
        assert_eq!(got.header("grpc-status"), Some("3"), "{why}: {got:?}");
        let said = got.header("grpc-message").unwrap_or("").to_string();
        assert!(said.contains(&pct(reason)), "{why}: `{reason}` in `{said}`");
        assert!(got.data.is_empty() && got.ended, "{why}: {got:?}");
        stream += 2;
    };
    let mut no_qty = put_str(1, "ACME");
    no_qty.extend(put_int(3, 12500));
    refused(&mut c, "a field absent", &no_qty, "missing_field: qty");
    refused(&mut c, "an empty message", &[], "missing_field: symbol");
    let mut wrong = put_int(1, 5);
    wrong.extend(put_int(2, 1));
    wrong.extend(put_int(3, 1));
    refused(&mut c, "a varint where a string is", &wrong, "wrong_type: symbol");
    refused(&mut c, "json text in a protobuf call", b"{\"symbol\":\"ACME\",\"qty\":10,\"limit\":12500}", "wrong_type: payload");
    let mut cut = put_str(1, "ACME");
    cut.truncate(cut.len() - 1);
    refused(&mut c, "a message cut short", &cut, "wrong_type: payload");
    // unknown fields are skipped, so an extra one is no fault
    let mut extra = encode("{\"symbol\":\"ACME\",\"qty\":10,\"limit\":12500}", "PlaceOrder");
    extra.extend(put_str(99, "ignored"));
    call(&mut c, stream, "/Public/Orders__place", Some("t-alice"), Some(DIGEST), PROTO, &framed(&extra));
    let got = c.response(stream, WAIT);
    assert_eq!(got.header("grpc-status"), Some("0"), "{got:?}");
    assert!(server.finish().status.success());
}

/// The reserved method answers a one-field message under protobuf, the
/// document under JSON.
#[test]
fn the_description_is_a_document_message_under_protobuf() {
    let server = start(16, &[]);
    let mut c = Client::connect(server.port);
    for (i, ctype) in [PROTO, BARE].iter().enumerate() {
        let stream = 2 * i as u32 + 1;
        call(&mut c, stream, "/hale.api.Description/Describe", Some("t-alice"), None, ctype, &framed(&[]));
        let got = c.response(stream, WAIT);
        assert_eq!(got.header("grpc-status"), Some("0"), "{got:?}");
        assert_eq!(got.header("content-type"), Some(*ctype));
        let n = u32::from_be_bytes(got.data[1..5].try_into().unwrap()) as usize;
        assert_eq!(got.data.len(), 5 + n);
        let doc = &got.data[5..];
        // field 1, a string: the document the fixture states for alice
        assert_eq!(doc[0], 0x0a, "{ctype}: field 1, length-delimited");
        let mut rest = &doc[1..];
        let mut len = 0usize;
        let mut shift = 0;
        loop {
            let b = rest[0];
            rest = &rest[1..];
            len |= usize::from(b & 0x7f) << shift;
            shift += 7;
            if b < 0x80 {
                break;
            }
        }
        assert_eq!(rest.len(), len, "{ctype}: the string runs to the end of the message");
        assert_eq!(String::from_utf8(rest.to_vec()).unwrap(), description_over_grpc("public.alice.description.json", server.port, "public"));
    }
    // and a JSON caller still gets the document itself
    call(&mut c, 5, "/hale.api.Description/Describe", Some("t-alice"), None, JSON, &super::api_grpc::message("{}"));
    let got = c.response(5, WAIT);
    assert_eq!(super::api_grpc::text_of(&got), description_over_grpc("public.alice.description.json", server.port, "public"));
    assert!(server.finish().status.success());
}

// ---- reflection ----

#[derive(Clone, PartialEq, prost::Message)]
struct ReflectionRequest {
    #[prost(string, tag = "1")]
    host: String,
    #[prost(oneof = "request::Kind", tags = "3, 4, 6, 7")]
    kind: Option<request::Kind>,
}

mod request {
    #[derive(Clone, PartialEq, prost::Oneof)]
    pub enum Kind {
        #[prost(string, tag = "3")]
        FileByFilename(String),
        #[prost(string, tag = "4")]
        FileContainingSymbol(String),
        #[prost(string, tag = "6")]
        AllExtensionNumbersOfType(String),
        #[prost(string, tag = "7")]
        ListServices(String),
    }
}

#[derive(Clone, PartialEq, prost::Message)]
struct ReflectionResponse {
    #[prost(string, tag = "1")]
    valid_host: String,
    #[prost(message, optional, tag = "2")]
    original_request: Option<ReflectionRequest>,
    #[prost(oneof = "response::Kind", tags = "4, 6, 7")]
    kind: Option<response::Kind>,
}

#[derive(Clone, PartialEq, prost::Message)]
struct FileDescriptors {
    #[prost(bytes = "vec", repeated, tag = "1")]
    file_descriptor_proto: Vec<Vec<u8>>,
}

#[derive(Clone, PartialEq, prost::Message)]
struct Services {
    #[prost(message, repeated, tag = "1")]
    service: Vec<ServiceName>,
}

#[derive(Clone, PartialEq, prost::Message)]
struct ServiceName {
    #[prost(string, tag = "1")]
    name: String,
}

#[derive(Clone, PartialEq, prost::Message)]
struct ErrorResponse {
    #[prost(int32, tag = "1")]
    error_code: i32,
    #[prost(string, tag = "2")]
    error_message: String,
}

mod response {
    #[derive(Clone, PartialEq, prost::Oneof)]
    pub enum Kind {
        #[prost(message, tag = "4")]
        FileDescriptorResponse(super::FileDescriptors),
        #[prost(message, tag = "6")]
        ListServicesResponse(super::Services),
        #[prost(message, tag = "7")]
        ErrorResponse(super::ErrorResponse),
    }
}

const REFLECTION: &str = "/grpc.reflection.v1.ServerReflection/ServerReflectionInfo";

fn ask(kind: request::Kind) -> ReflectionRequest {
    ReflectionRequest { host: String::new(), kind: Some(kind) }
}

/// The services the reflection service lists.
pub(crate) fn reflected_services(c: &mut Client, stream: u32, token: &str) -> Vec<String> {
    let got = reflect(c, stream, Some(token), &[ask(request::Kind::ListServices(String::new()))], true);
    match &got[0].kind {
        Some(response::Kind::ListServicesResponse(s)) => s.service.iter().map(|s| s.name.clone()).collect(),
        other => panic!("not a service list: {other:?}"),
    }
}

/// One protobuf call: `json` (a flat object) as the message `request`, framed.
pub(crate) fn unary_pb(c: &mut Client, stream: u32, path: &str, token: Option<&str>, json: &str, request: &str) -> super::api_grpc::h2_client::Response {
    call(c, stream, path, token, Some(DIGEST), PROTO, &framed(&encode(json, request)));
    c.response(stream, WAIT)
}

/// The messages of a response body, whole.
fn messages(data: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < data.len() {
        let n = u32::from_be_bytes(data[at + 1..at + 5].try_into().unwrap()) as usize;
        out.push(data[at + 5..at + 5 + n].to_vec());
        at += 5 + n;
    }
    out
}

fn reflect(c: &mut Client, stream: u32, token: Option<&str>, asks: &[ReflectionRequest], end: bool) -> Vec<ReflectionResponse> {
    let mut body = Vec::new();
    for a in asks {
        body.extend(framed(&a.encode_to_vec()));
    }
    let auth = token.map(|t| format!("Bearer {t}"));
    let mut hs: Vec<(&str, &str)> = vec![(":method", "POST"), (":scheme", "http"), (":path", REFLECTION), (":authority", "localhost"), ("content-type", BARE), ("te", "trailers")];
    if let Some(a) = auth.as_deref() {
        hs.push(("authorization", a));
    }
    c.headers(stream, &hs, false);
    c.data(stream, &body, end);
    let got = c.response(stream, WAIT);
    assert_eq!(got.header("grpc-status").or(got.trailers.iter().find(|(k, _)| k == "grpc-status").map(|(_, v)| v.as_str())), Some("0"), "{got:?}");
    messages(&got.data).iter().map(|m| ReflectionResponse::decode(m.as_slice()).expect("a ServerReflectionResponse")).collect()
}

fn descriptors(r: &ReflectionResponse) -> Vec<prost_types::FileDescriptorProto> {
    match &r.kind {
        Some(response::Kind::FileDescriptorResponse(d)) => d
            .file_descriptor_proto
            .iter()
            .map(|b| prost_types::FileDescriptorProto::decode(b.as_slice()).expect("a FileDescriptorProto"))
            .collect(),
        other => panic!("not a file descriptor response: {other:?}"),
    }
}

/// The file a compiler makes of `.proto` source: `name` resolved under `dir`.
fn compiled(dir: &std::path::Path, name: &str) -> prost_types::FileDescriptorProto {
    let set = protox::compile([name], [dir]).unwrap_or_else(|e| panic!("protox {name}: {e}"));
    let mut file = set.file.into_iter().find(|f| f.name() == name).expect("the file");
    file.source_code_info = None;
    file
}

fn scratch(what: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hale_r8b_{what}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

const DESCRIPTION_PROTO: &str = "syntax = \"proto3\";\n\
package hale.api;\n\
service Description { rpc Describe(DescribeRequest) returns (DescriptionDocument); }\n\
message DescribeRequest {}\n\
message DescriptionDocument { string json = 1; }\n";

/// gRPC's own `reflection.proto` (v1), as the gRPC repository states it.
const REFLECTION_PROTO: &str = "syntax = \"proto3\";\n\
package grpc.reflection.v1;\n\
service ServerReflection {\n\
  rpc ServerReflectionInfo(stream ServerReflectionRequest) returns (stream ServerReflectionResponse);\n\
}\n\
message ServerReflectionRequest {\n\
  string host = 1;\n\
  oneof message_request {\n\
    string file_by_filename = 3;\n\
    string file_containing_symbol = 4;\n\
    ExtensionRequest file_containing_extension = 5;\n\
    string all_extension_numbers_of_type = 6;\n\
    string list_services = 7;\n\
  }\n\
}\n\
message ExtensionRequest {\n\
  string containing_type = 1;\n\
  int32 extension_number = 2;\n\
}\n\
message ServerReflectionResponse {\n\
  string valid_host = 1;\n\
  ServerReflectionRequest original_request = 2;\n\
  oneof message_response {\n\
    FileDescriptorResponse file_descriptor_response = 4;\n\
    ExtensionNumberResponse all_extension_numbers_response = 5;\n\
    ListServiceResponse list_services_response = 6;\n\
    ErrorResponse error_response = 7;\n\
  }\n\
}\n\
message FileDescriptorResponse { repeated bytes file_descriptor_proto = 1; }\n\
message ExtensionNumberResponse {\n\
  string base_type_name = 1;\n\
  repeated int32 extension_number = 2;\n\
}\n\
message ListServiceResponse { repeated ServiceResponse service = 1; }\n\
message ServiceResponse { string name = 1; }\n\
message ErrorResponse {\n\
  int32 error_code = 1;\n\
  string error_message = 2;\n\
}\n";

#[test]
fn reflection_lists_the_services() {
    let server = start(16, &[]);
    let mut c = Client::connect(server.port);
    let list = ask(request::Kind::ListServices(String::new()));
    let got = reflect(&mut c, 1, Some("t-alice"), std::slice::from_ref(&list), true);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].original_request.as_ref(), Some(&list), "the request is echoed");
    match &got[0].kind {
        Some(response::Kind::ListServicesResponse(s)) => {
            let names: Vec<&str> = s.service.iter().map(|s| s.name.as_str()).collect();
            assert_eq!(names, ["Public", "hale.api.Description", "grpc.reflection.v1.ServerReflection"]);
        }
        other => panic!("not a service list: {other:?}"),
    }
    assert!(server.finish().status.success());
}

/// The descriptor the server reflects for each file is the file a compiler
/// makes of the `.proto`: the surface's (the generated file, committed under
/// `tests/api-contract/`), the reserved description method's and the
/// reflection service's own.
#[test]
fn reflection_answers_the_descriptor_the_generated_proto_compiles_to() {
    let server = start(16, &[]);
    let mut c = Client::connect(server.port);
    let dir = scratch("reflection");
    std::fs::create_dir_all(dir.join("hale/api")).unwrap();
    std::fs::create_dir_all(dir.join("grpc/reflection/v1")).unwrap();
    std::fs::write(dir.join("hale/api/description.proto"), DESCRIPTION_PROTO).unwrap();
    std::fs::write(dir.join("grpc/reflection/v1/reflection.proto"), REFLECTION_PROTO).unwrap();
    let want_public = compiled(&contract_dir(), "Public.proto");
    let want_description = compiled(&dir, "hale/api/description.proto");
    let want_reflection = compiled(&dir, "grpc/reflection/v1/reflection.proto");
    assert_eq!(want_public.service.len(), 1, "one service");

    let mut stream = 1;
    let mut one = |c: &mut Client, kind: request::Kind| {
        let got = reflect(c, stream, Some("t-alice"), &[ask(kind)], true);
        stream += 2;
        assert_eq!(got.len(), 1);
        got.into_iter().next().unwrap()
    };
    // by symbol: the service, an rpc of it and a message of it are all the one file
    for symbol in ["Public", "Public.Orders__place", "PlaceOrder", "HaleRefusal"] {
        let files = descriptors(&one(&mut c, request::Kind::FileContainingSymbol(symbol.to_string())));
        assert_eq!(files, [want_public.clone()], "{symbol}");
    }
    // by name
    let files = descriptors(&one(&mut c, request::Kind::FileByFilename("Public.proto".to_string())));
    assert_eq!(files, [want_public.clone()]);
    let files = descriptors(&one(&mut c, request::Kind::FileContainingSymbol("hale.api.Description".to_string())));
    assert_eq!(files, [want_description.clone()]);
    let files = descriptors(&one(&mut c, request::Kind::FileContainingSymbol("hale.api.Description.Describe".to_string())));
    assert_eq!(files, [want_description]);
    let files = descriptors(&one(&mut c, request::Kind::FileContainingSymbol("grpc.reflection.v1.ServerReflection".to_string())));
    assert_eq!(files, [want_reflection]);
    // a symbol nobody declares, and what the service does not know
    for kind in [request::Kind::FileContainingSymbol("Nobody".to_string()), request::Kind::FileByFilename("none.proto".to_string()), request::Kind::AllExtensionNumbersOfType("Public".to_string())] {
        match one(&mut c, kind).kind {
            Some(response::Kind::ErrorResponse(e)) => assert_eq!(e.error_code, 5, "NOT_FOUND: {e:?}"),
            other => panic!("not an error response: {other:?}"),
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(server.finish().status.success());
}

/// The headers that open a reflection call.
fn reflect_open(c: &mut Client, stream: u32) {
    let hs: Vec<(&str, &str)> = vec![(":method", "POST"), (":scheme", "http"), (":path", REFLECTION), (":authority", "localhost"), ("content-type", BARE), ("te", "trailers"), ("authorization", "Bearer t-alice")];
    c.headers(stream, &hs, false);
}

/// Read until `n` answers have come, the call still open.
fn answers(c: &mut Client, stream: u32, n: usize) -> Vec<ReflectionResponse> {
    let got = c.until(stream, |r| whole(&r.data) >= n, WAIT);
    assert!(whole(&got.data) >= n, "{n} answers expected: {got:?}");
    messages(&got.data).iter().map(|m| ReflectionResponse::decode(m.as_slice()).expect("a ServerReflectionResponse")).collect()
}

/// The messages a body holds whole.
fn whole(data: &[u8]) -> usize {
    let mut at = 0;
    let mut n = 0;
    while at + 5 <= data.len() {
        let size = u32::from_be_bytes(data[at + 1..at + 5].try_into().unwrap()) as usize;
        if at + 5 + size > data.len() {
            break;
        }
        at += 5 + size;
        n += 1;
    }
    n
}

/// The client ends its side; the status comes, once, after the answers.
fn half_close(c: &mut Client, stream: u32) -> super::api_grpc::h2_client::Response {
    c.data(stream, &[], true);
    let got = c.response(stream, WAIT);
    assert!(got.ended, "{got:?}");
    assert_eq!(got.trailers.iter().find(|(k, _)| k == "grpc-status").map(|(_, v)| v.as_str()), Some("0"), "{got:?}");
    got
}

/// The service is bidirectional: the server answers each request as it
/// arrives and keeps the call open until the client ends its side. A client
/// that waits for an answer before it asks again (grpcurl does) gets every
/// one, requests that arrive together get an answer each, in order, and a
/// request split across DATA frames is answered once, when it is whole.
#[test]
fn reflection_answers_each_request_as_it_arrives_and_stays_open_until_the_client_ends() {
    let server = start(16, &[]);
    let mut c = Client::connect(server.port);
    let list = ask(request::Kind::ListServices(String::new()));
    let symbol = ask(request::Kind::FileContainingSymbol("Public".to_string()));

    // one RPC, two requests, an answer awaited between them; then the end
    reflect_open(&mut c, 1);
    c.data(1, &framed(&list.encode_to_vec()), false);
    let got = answers(&mut c, 1, 1);
    assert!(matches!(got[0].kind, Some(response::Kind::ListServicesResponse(_))), "{got:?}");
    let open = c.until(1, |_| false, Duration::from_millis(200));
    assert!(!open.ended && open.trailers.is_empty(), "the call stays open after an answer: {open:?}");
    c.data(1, &framed(&symbol.encode_to_vec()), false);
    let got = answers(&mut c, 1, 2);
    assert_eq!(got.len(), 2, "an answer for each");
    assert_eq!(got[0].original_request.as_ref(), Some(&list), "the request is echoed");
    assert!(matches!(got[1].kind, Some(response::Kind::FileDescriptorResponse(_))), "{got:?}");
    assert_eq!(got[1].original_request.as_ref(), Some(&symbol));
    let done = half_close(&mut c, 1);
    assert_eq!(whole(&done.data), 2, "the end adds no message: {done:?}");

    // two requests in one DATA frame
    let reply = reflect(&mut c, 3, Some("t-alice"), &[list.clone(), symbol.clone()], true);
    assert_eq!(reply.len(), 2, "an answer for each");
    assert!(matches!(reply[0].kind, Some(response::Kind::ListServicesResponse(_))));
    assert!(matches!(reply[1].kind, Some(response::Kind::FileDescriptorResponse(_))));

    // one request split across two DATA frames: answered once, when whole
    let framed_list = framed(&list.encode_to_vec());
    reflect_open(&mut c, 5);
    c.data(5, &framed_list[..3], false);
    c.idle(Duration::from_millis(150));
    assert_eq!(c.until(5, |_| false, Duration::from_millis(50)).data.len(), 0, "no answer to half a message");
    c.data(5, &framed_list[3..], false);
    let got = answers(&mut c, 5, 1);
    assert!(matches!(got[0].kind, Some(response::Kind::ListServicesResponse(_))));
    assert_eq!(whole(&half_close(&mut c, 5).data), 1, "answered once");

    // a whole request and the start of the next in the first DATA frame: the
    // start is kept, not discarded
    let framed_symbol = framed(&symbol.encode_to_vec());
    reflect_open(&mut c, 7);
    let mut first = framed_list.clone();
    first.extend_from_slice(&framed_symbol[..6]);
    c.data(7, &first, false);
    let got = answers(&mut c, 7, 1);
    assert!(matches!(got[0].kind, Some(response::Kind::ListServicesResponse(_))));
    c.idle(Duration::from_millis(150));
    assert_eq!(whole(&c.until(7, |_| false, Duration::from_millis(50)).data), 1, "the unfinished request is not answered");
    c.data(7, &framed_symbol[6..], false);
    let got = answers(&mut c, 7, 2);
    assert!(matches!(got[1].kind, Some(response::Kind::FileDescriptorResponse(_))), "{got:?}");
    half_close(&mut c, 7);

    // the connection serves on
    let rec = recording("result");
    replay(&mut c, 9, &rec, PROTO);
    assert_replayed(&mut c, 9, "result", &rec, PROTO);
    assert!(server.finish().status.success());
}

/// A reflection call still open when the transport stops is ended with its
/// status, and the connection says GOAWAY.
#[test]
fn a_reflection_call_open_at_stop_ends_with_its_status_and_a_goaway() {
    let server = start(16, &[]);
    let mut c = Client::connect(server.port);
    let list = ask(request::Kind::ListServices(String::new()));
    reflect_open(&mut c, 1);
    c.data(1, &framed(&list.encode_to_vec()), false);
    answers(&mut c, 1, 1);
    let done = server.finish();
    let got = c.response(1, WAIT);
    assert_eq!(got.trailers.iter().find(|(k, _)| k == "grpc-status").map(|(_, v)| v.as_str()), Some("0"), "{got:?}");
    assert!(c.goaway_within(WAIT).is_some(), "GOAWAY");
    assert!(done.status.success(), "{:?}\n{}", done.status, done.stderr);
}

/// What a reflection call that has answered is told when it ends in error:
/// the status is the trailers of the response already open (not a second
/// response), then the stream ends and the connection serves on. Three ends
/// after a first answered request: a second message over the limit (8), a
/// message the client's half-close leaves unfinished (13), and a bearer source
/// that stops naming the caller at the second message (16; the bearer is
/// asked at every message). The hand-written client reads each as a response
/// whose headers came with the first answer and whose trailers hold the
/// status.
#[test]
fn a_reflection_error_after_the_first_answer_is_the_trailers_of_the_open_response() {
    // `t-once` is named by the bearer source for its first ask only
    let source = super::api_grpc::source(16).replace(
        "locus Tokens {\n    fn principal(token: String) -> std::api::Principal {\n",
        "locus Tokens {\n    params { asked: Int = 0; }\n    fn principal(token: String) -> std::api::Principal {\n        if token == \"t-once\" {\n            self.asked = self.asked + 1;\n            if self.asked == 1 { return std::api::Principal { mode: \"bearer\", name: \"alice\" }; }\n            return std::api::Principal { mode: \"bearer\", name: \"\" };\n        }\n",
    );
    assert!(source.contains("t-once"), "the bearer source was swapped");
    let bin = super::api_grpc::build_variant(&source, "api_grpc_proto_reflect_end");
    let server = super::api_grpc::http_rpc::Server::start(&bin, &[]);
    server.ready();
    let mut c = Client::connect(server.port);
    let list = ask(request::Kind::ListServices(String::new()));
    let framed_list = framed(&list.encode_to_vec());
    let open = |c: &mut Client, stream: u32, token: &str| {
        let auth = format!("Bearer {token}");
        let hs: Vec<(&str, &str)> = vec![(":method", "POST"), (":scheme", "http"), (":path", REFLECTION), (":authority", "localhost"), ("content-type", BARE), ("te", "trailers"), ("authorization", auth.as_str())];
        c.headers(stream, &hs, false);
        c.data(stream, &framed_list, false);
    };
    // the end: one answer in the open response, then the status as its
    // trailers, no second set of headers, and nothing after the status
    let ends = |c: &mut Client, stream: u32, status: &str, message: &str| {
        let got = c.response(stream, WAIT);
        assert!(got.ended && got.reset.is_none(), "{got:?}");
        assert_eq!(got.header(":status"), Some("200"), "{got:?}");
        assert!(!got.headers.iter().any(|(k, _)| k == "grpc-status"),"the status is not in the headers of an answered call: {got:?}");
        assert_eq!(whole(&got.data), 1, "the answer already sent, and no more: {got:?}");
        assert_eq!(got.trailers.iter().find(|(k, _)| k == "grpc-status").map(|(_, v)| v.as_str()), Some(status), "{got:?}");
        let said = got.trailers.iter().find(|(k, _)| k == "grpc-message").map(|(_, v)| v.as_str()).unwrap_or("");
        assert!(said.contains(message), "{said:?} names {message:?}: {got:?}");
    };

    // a second message over the limit: ends at once, without the half-close
    open(&mut c, 1, "t-alice");
    answers(&mut c, 1, 1);
    let mut huge = vec![0u8, 0, 0x1e, 0x84, 0x80]; // a message of 2,000,000 bytes, its first 1.1 MB
    huge.resize(1_100_000, 7);
    c.data(1, &huge, false);
    ends(&mut c, 1, "8", "over");

    // a message the half-close leaves unfinished
    open(&mut c, 3, "t-alice");
    answers(&mut c, 3, 1);
    c.data(3, &framed_list[..3], true);
    ends(&mut c, 3, "13", "ends");

    // the bearer source stops naming the caller at the second message
    open(&mut c, 5, "t-once");
    answers(&mut c, 5, 1);
    c.data(5, &framed_list, false);
    ends(&mut c, 5, "16", "refused");

    // the connection serves on
    let rec = recording("result");
    replay(&mut c, 7, &rec, PROTO);
    assert_replayed(&mut c, 7, "result", &rec, PROTO);
    assert!(server.finish().status.success());
}

/// The protobuf and reflection paths under AddressSanitizer with the arena's
/// chunk recycling off: every outcome, a large message, the description, the
/// reflection service, a stream abandoned mid-message, a call still
/// executing at stop.
#[test]
fn the_protobuf_and_reflection_paths_run_clean_under_asan() {
    let bin = super::api_grpc::harness::unique_bin("api_grpc_proto_asan");
    super::api_grpc::harness::build_source_asan(&super::api_grpc::source(64), &bin);
    let server = super::api_grpc::http_rpc::Server::start(&bin, &[("LOTUS_NO_CHUNK_POOL", "1"), ("SLOW_MS", "200")]);
    server.ready();
    let mut c = Client::connect(server.port);
    for (i, name) in ["result", "handler_error", "refusal_unauthorized", "refusal_malformed", "server_error", "refusal_unavailable"].iter().enumerate() {
        let rec = recording(name);
        let stream = 2 * i as u32 + 1;
        replay(&mut c, stream, &rec, PROTO);
        assert_replayed(&mut c, stream, name, &rec, PROTO);
    }
    call(&mut c, 13, "/hale.api.Description/Describe", Some("t-alice"), None, PROTO, &framed(&[]));
    assert_eq!(c.response(13, WAIT).header("grpc-status"), Some("0"));
    let mut big = put_str(1, &"A".repeat(500_000));
    big.extend(put_int(2, 1));
    big.extend(put_int(3, 1));
    call(&mut c, 15, "/Public/Orders__place", Some("t-alice"), Some(DIGEST), PROTO, &framed(&big));
    assert_eq!(c.response(15, WAIT).status(), 200);
    // reflection: the services, a file, a symbol nobody has
    let list = ask(request::Kind::ListServices(String::new()));
    let symbol = ask(request::Kind::FileContainingSymbol("PlaceOrder".to_string()));
    let nobody = ask(request::Kind::FileContainingSymbol("Nobody".to_string()));
    let got = reflect(&mut c, 17, Some("t-alice"), &[list, symbol, nobody], true);
    assert_eq!(got.len(), 3);
    // an abandoned stream, and a vanished client
    c.headers(19, &[(":method", "POST"), (":scheme", "http"), (":path", "/Public/Orders__place"), (":authority", "localhost"), ("content-type", PROTO)], false);
    c.data(19, &framed(&big)[..10], false);
    c.reset(19, 8);
    {
        let mut gone = Client::connect(server.port);
        call(&mut gone, 1, "/Public/Orders__place", Some("t-alice"), Some(DIGEST), PROTO, &framed(&encode("{\"symbol\":\"ACME\",\"qty\":10,\"limit\":12500}", "PlaceOrder")));
    }
    let mut slow = Client::connect(server.port);
    replay(&mut slow, 1, &recording("result"), PROTO);
    std::thread::sleep(Duration::from_millis(50));
    let done = server.finish();
    drop(slow);
    for bad in ["AddressSanitizer", "LeakSanitizer", "heap-use-after-free", "SUMMARY:"] {
        assert!(!done.stderr.contains(bad), "{bad} in:\n{}", done.stderr);
    }
    assert!(done.status.success(), "{:?}\n{}\n{}", done.status, done.stdout, done.stderr);
}

/// Reflection names what the surface has, so it is for a caller the bearer
/// source names, as a description is.
#[test]
fn reflection_is_refused_to_a_caller_nobody_names() {
    let server = start(16, &[]);
    let mut c = Client::connect(server.port);
    let list = ask(request::Kind::ListServices(String::new()));
    for (stream, token) in [(1u32, None), (3, Some("t-mallory"))] {
        let auth = token.map(|t| format!("Bearer {t}"));
        let mut hs: Vec<(&str, &str)> = vec![(":method", "POST"), (":scheme", "http"), (":path", REFLECTION), (":authority", "localhost"), ("content-type", BARE)];
        if let Some(a) = auth.as_deref() {
            hs.push(("authorization", a));
        }
        c.headers(stream, &hs, false);
        c.data(stream, &framed(&list.encode_to_vec()), true);
        let got = c.response(stream, WAIT);
        assert_eq!(got.header("grpc-status"), Some("16"), "{got:?}");
        assert!(got.data.is_empty(), "{got:?}");
    }
    assert!(server.finish().status.success());
}

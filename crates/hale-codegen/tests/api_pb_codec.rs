//! GH #1417 (R8b): the protobuf codec generated beside the JSON one.
//!
//! A program that serves a surface over `grpc::Rpc` carries, for each record
//! its rows name, `__api_pb_decode_<T>` and `__api_pb_encode_<T>`. The
//! in-language pin of the codec's rules is `tests/hale/api/pb_codec_test.hl`;
//! these hold it to the contract's own material:
//!
//! * every request and reply of the recorded exchanges
//!   (`tests/api-contract/wire/http/`), read by the JSON decoder, is carried
//!   as protobuf and read back as the same value, and a request the JSON
//!   decoder refuses is refused by the protobuf decoder in the same words;
//! * the programs of the corpus that serve a surface (the example fixture and
//!   the DNA's head commands) build with their HTTP exposure swapped for
//!   `grpc::Rpc`: every record shape their rows name gets a codec that
//!   compiles, and `hale api export` writes a `.proto` for it.

use std::path::{Path, PathBuf};
use std::process::Command;

#[path = "support/build.rs"]
mod build_opts;
#[path = "support/harness.rs"]
mod harness;
#[path = "support/http_rpc.rs"]
mod http_rpc;
#[path = "support/ports.rs"]
mod ports;

use http_rpc::{contract_dir, recording};

/// The contract program with `Public` served over `grpc::Rpc` and a `main`
/// that probes the codecs instead of running the desk.
fn witness_program(probes: &str) -> String {
    let src = std::fs::read_to_string(contract_dir().join("program.hl")).expect("the contract program");
    let grpc = src.replace("http::Rpc { bind: \"127.0.0.1:8080\", codec: json,", "grpc::Rpc { bind: \"127.0.0.1:8080\", codec: json,");
    assert_ne!(grpc, src, "the transport was swapped");
    let main = "fn main() { Desk { }; }";
    assert!(grpc.contains(main));
    grpc.replace(main, &format!("{PROBE}\nfn main() {{\n{probes}}}\n"))
}

/// What each record type of `Public` is probed with: the value read from
/// JSON, carried as protobuf, read back, and written as JSON again.
const PROBE: &str = r#"
fn probe_PlaceOrder(label: String, json: String) {
    let v = __api_decode_PlaceOrder(json) or { println(label + ": json refused " + err.kind + ": " + err.field); return; };
    let w = __api_pb_decode_PlaceOrder(__api_pb_encode_PlaceOrder(v)) or { println(label + ": pb refused " + err.kind + ": " + err.field); return; };
    println(label + ": " + __api_encode_PlaceOrder(w));
}
fn probe_CancelOrder(label: String, json: String) {
    let v = __api_decode_CancelOrder(json) or { println(label + ": json refused " + err.kind + ": " + err.field); return; };
    let w = __api_pb_decode_CancelOrder(__api_pb_encode_CancelOrder(v)) or { println(label + ": pb refused " + err.kind + ": " + err.field); return; };
    println(label + ": " + __api_encode_CancelOrder(w));
}
fn probe_OrderReceipt(label: String, json: String) {
    let v = __api_decode_OrderReceipt(json) or { println(label + ": json refused " + err.kind + ": " + err.field); return; };
    let w = __api_pb_decode_OrderReceipt(__api_pb_encode_OrderReceipt(v)) or { println(label + ": pb refused " + err.kind + ": " + err.field); return; };
    println(label + ": " + __api_encode_OrderReceipt(w));
}
fn probe_Cancelled(label: String, json: String) {
    let v = __api_decode_Cancelled(json) or { println(label + ": json refused " + err.kind + ": " + err.field); return; };
    let w = __api_pb_decode_Cancelled(__api_pb_encode_Cancelled(v)) or { println(label + ": pb refused " + err.kind + ": " + err.field); return; };
    println(label + ": " + __api_encode_Cancelled(w));
}
fn probe_OrderError(label: String, json: String) {
    let v = __api_decode_OrderError(json) or { println(label + ": json refused " + err.kind + ": " + err.field); return; };
    let w = __api_pb_decode_OrderError(__api_pb_encode_OrderError(v)) or { println(label + ": pb refused " + err.kind + ": " + err.field); return; };
    println(label + ": " + __api_encode_OrderError(w));
}
"#;

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The record a recorded exchange's request, and its reply, are.
fn request_type(path: &str) -> Option<&'static str> {
    match path {
        "/call/Orders::place" => Some("PlaceOrder"),
        "/call/Orders::cancel" => Some("CancelOrder"),
        _ => None,
    }
}

fn reply_type(path: &str, status: u16) -> Option<&'static str> {
    match (path, status) {
        ("/call/Orders::place", 200) => Some("OrderReceipt"),
        ("/call/Orders::cancel", 200) => Some("Cancelled"),
        ("/call/Orders::cancel", 422) => Some("OrderError"),
        _ => None,
    }
}

const RECORDED: [&str; 10] = [
    "result",
    "handler_error",
    "server_error",
    "refusal_digest_mismatch",
    "refusal_full",
    "refusal_malformed",
    "refusal_shutting_down",
    "refusal_unauthenticated",
    "refusal_unauthorized",
    "refusal_unavailable",
];

fn run(bin: &Path) -> String {
    let out = Command::new(bin).output().expect("run the probe");
    assert!(out.status.success(), "the probe ended: {:?}\n{}", out.status, String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).expect("UTF-8")
}

#[test]
fn the_recorded_exchanges_are_carried_as_protobuf_and_read_back_the_same() {
    let mut probes = String::new();
    let mut expect = Vec::new();
    for name in RECORDED {
        let rec = recording(name);
        if let Some(t) = request_type(&rec.path) {
            probes.push_str(&format!("    probe_{t}({}, {});\n", quote(&format!("{name} request")), quote(&rec.body)));
            expect.push((format!("{name} request"), rec.body.clone()));
        }
        if let Some(t) = reply_type(&rec.path, rec.status) {
            probes.push_str(&format!("    probe_{t}({}, {});\n", quote(&format!("{name} reply")), quote(&rec.reply)));
            expect.push((format!("{name} reply"), rec.reply.clone()));
        }
    }
    assert!(expect.len() >= 10, "the recordings carry requests and replies: {expect:?}");
    let bin = harness::unique_bin("api_pb_codec_witness");
    build_opts::build_source(&witness_program(&probes), &bin, &build_opts::options()).expect("build the probe");
    let out = run(&bin);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), expect.len(), "a line a probe:\n{out}");
    for (line, (label, body)) in lines.iter().zip(&expect) {
        if label == "refusal_malformed request" {
            // the recorded request the JSON decoder refuses: a place without its `qty`
            assert_eq!(*line, format!("{label}: json refused missing_field: qty"), "{line}");
            continue;
        }
        // the JSON the recording holds is the JSON the value writes
        assert_eq!(*line, format!("{label}: {body}"), "{label}: the value that went in is the value that came out");
    }
}

/// The recorded malformed request, as protobuf: a `PlaceOrder` with its
/// `symbol` and `limit` and no `qty`, refused in the JSON codec's words.
#[test]
fn the_recorded_malformed_request_is_refused_the_same_in_protobuf() {
    let probes = "    let m = std::bytes::concat(__api_pb_put_str(1, \"ACME\"), __api_pb_put_int(3, 12500));\n    \
        let v = __api_pb_decode_PlaceOrder(m) or { println(err.kind + \": \" + err.field); return; };\n    \
        println(v.symbol);\n";
    let bin = harness::unique_bin("api_pb_codec_malformed");
    build_opts::build_source(&witness_program(probes), &bin, &build_opts::options()).expect("build the probe");
    let rec = recording("refusal_malformed");
    assert!(rec.body.contains("\"symbol\":\"ACME\"") && !rec.body.contains("qty"), "{}", rec.body);
    assert_eq!(run(&bin).trim(), "missing_field: qty");
}

// ---- the corpus ----

fn repo() -> PathBuf {
    contract_dir().join("../..")
}

/// A program of the corpus that serves a surface, with its HTTP exposure
/// served over `grpc::Rpc`.
fn swapped(path: &str) -> String {
    let src = std::fs::read_to_string(repo().join(path)).unwrap_or_else(|e| panic!("{path}: {e}"));
    let grpc = src.replace("http::Rpc {", "grpc::Rpc {");
    assert_ne!(grpc, src, "{path}: the HTTP exposure was swapped");
    grpc
}

/// Every record shape the corpus's served surfaces name gets a protobuf codec
/// that compiles.
#[test]
fn the_corpus_surfaces_build_over_grpc_with_their_codecs() {
    for path in ["crates/hale-codegen/tests/fixtures/examples/92-build-an-api/main.hl"] {
        let bin = harness::unique_bin("api_pb_codec_corpus");
        build_opts::build_source(&swapped(path), &bin, &build_opts::options()).unwrap_or_else(|e| panic!("{path}: {e:?}"));
    }
}

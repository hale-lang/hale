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

// ---- a message field sent twice is merged ----

#[derive(Clone, PartialEq, prost::Message)]
struct PPart {
    #[prost(int64, tag = "1")]
    a: i64,
    #[prost(int64, tag = "2")]
    b: i64,
}

#[derive(Clone, PartialEq, prost::Message)]
struct PWhole {
    #[prost(message, optional, tag = "1")]
    part: Option<PPart>,
}

#[derive(Clone, PartialEq, prost::Message)]
struct PTop {
    #[prost(message, optional, tag = "1")]
    whole: Option<PWhole>,
}

#[derive(Clone, PartialEq, prost::Message)]
struct PNeed {
    #[prost(int64, tag = "1")]
    x: i64,
    #[prost(int64, tag = "2")]
    y: i64,
}

#[derive(Clone, PartialEq, prost::Message)]
struct PNeeds {
    #[prost(message, optional, tag = "1")]
    need: Option<PNeed>,
}

/// The Hale expression for `bytes`: one byte at a time, concatenated.
fn bytes_expr(bytes: &[u8]) -> String {
    let mut e = "std::bytes::from_string(\"\")".to_string();
    for b in bytes {
        e = format!("std::bytes::concat({e}, std::bytes::from_int({b}))");
    }
    e
}

/// A `Part` occurrence: only the fields it names.
fn part(a: Option<u8>, b: Option<u8>) -> Vec<u8> {
    let mut out = Vec::new();
    if let Some(a) = a {
        out.extend([0x08, a]);
    }
    if let Some(b) = b {
        out.extend([0x10, b]);
    }
    out
}

/// `body` as an occurrence of field 1.
fn occurrence(body: Vec<u8>) -> Vec<u8> {
    let mut out = vec![0x0a, body.len() as u8];
    out.extend(body);
    out
}

/// The messages' fields are numbered by declaration, and `prost` merges what
/// it decodes the way the wire format says. The generated decoder is held to
/// it on the same bytes: the reviewer's `0a0208070a021009`, three occurrences
/// whose scalars overwrite, two levels, and fields with no default split
/// between occurrences.
#[test]
fn a_message_sent_twice_decodes_as_prost_merges_it() {
    use prost::Message;
    let reviewer = [occurrence(part(Some(7), None)), occurrence(part(None, Some(9)))].concat();
    assert_eq!(reviewer, [0x0a, 0x02, 0x08, 0x07, 0x0a, 0x02, 0x10, 0x09]);
    let overwrite = [occurrence(part(Some(1), Some(5))), occurrence(part(None, Some(9))), occurrence(part(Some(3), None))].concat();
    let deep = [occurrence(occurrence(part(Some(7), None))), occurrence(occurrence(part(None, Some(9))))].concat();
    let split = [occurrence(part(Some(4), None)), occurrence(part(None, Some(6)))].concat();

    let pair = |p: Option<PPart>| {
        let p = p.expect("the part");
        format!("{},{}", p.a, p.b)
    };
    let need = PNeeds::decode(split.as_slice()).unwrap().need.unwrap();
    let expect = [
        ("whole", pair(PWhole::decode(reviewer.as_slice()).unwrap().part)),
        ("overwrite", pair(PWhole::decode(overwrite.as_slice()).unwrap().part)),
        ("deep", pair(PTop::decode(deep.as_slice()).unwrap().whole.unwrap().part)),
        ("needs", format!("{},{}", need.x, need.y)),
    ];
    assert_eq!(expect.iter().map(|(_, v)| v.as_str()).collect::<Vec<_>>(), ["7,9", "3,9", "7,9", "4,6"], "what prost merges");

    let program = format!(
        r#"
type Part {{ a: Int = 0; b: Int = 0; }}
type Whole {{ part: Part; }}
type Top {{ whole: Whole; }}
type Need {{ x: Int; y: Int; }}
type Needs {{ need: Need; }}
api Pb {{ rpc Probe::whole; rpc Probe::top; rpc Probe::needs; }}
locus Probe {{
    fn whole(w: Whole) -> Whole {{ return w; }}
    fn top(t: Top) -> Top {{ return t; }}
    fn needs(n: Needs) -> Needs {{ return n; }}
}}
locus Tokens {{
    fn principal(token: String) -> std::api::Principal {{ return std::api::Principal {{ mode: "bearer", name: "" }}; }}
    fn refused() -> String {{ return "no"; }}
}}
main locus Desk {{
    params {{ probe: Probe = Probe {{ }}; bearer: Tokens = Tokens {{ }}; h: api::Exposure = api::serve(Pb, grpc::Rpc {{ bind: "127.0.0.1:0", codec: json, principals: self.bearer }}, as: "pb", receivers: {{ Probe: self.probe }}, bound: 4, on_full: refuse); }}
    run() {{
        let w = __api_pb_decode_Whole({reviewer}) or {{ println("whole refused " + err.kind); return; }};
        println("whole " + to_string(w.part.a) + "," + to_string(w.part.b));
        let o = __api_pb_decode_Whole({overwrite}) or {{ println("overwrite refused " + err.kind); return; }};
        println("overwrite " + to_string(o.part.a) + "," + to_string(o.part.b));
        let t = __api_pb_decode_Top({deep}) or {{ println("deep refused " + err.kind); return; }};
        println("deep " + to_string(t.whole.part.a) + "," + to_string(t.whole.part.b));
        let n = __api_pb_decode_Needs({split}) or {{ println("needs refused " + err.kind + " " + err.field); return; }};
        println("needs " + to_string(n.need.x) + "," + to_string(n.need.y));
    }}
}}
fn main() {{ Desk {{ }}; }}
"#,
        reviewer = bytes_expr(&reviewer),
        overwrite = bytes_expr(&overwrite),
        deep = bytes_expr(&deep),
        split = bytes_expr(&split),
    );
    let bin = harness::unique_bin("api_pb_codec_merge");
    build_opts::build_source(&program, &bin, &build_opts::options()).expect("build the probe");
    let got = run(&bin);
    let want: Vec<String> = expect.iter().map(|(k, v)| format!("{k} {v}")).collect();
    assert_eq!(got.lines().collect::<Vec<_>>(), want.iter().map(String::as_str).collect::<Vec<_>>(), "the generated decoder and prost agree");
}

// ---- the descriptor is the file a compiler makes of the text ----

/// The `.proto` generator's file for one member whose request is `Data`, a
/// record of `Int` fields named `fields`.
fn data_file(fields: &[&str]) -> Result<hale_types::proto_gen::ProtoFile, String> {
    use hale_types::surface_doc::{ClientError, ClientMember, ClientModel};
    use hale_types::surfaces::{FieldSchema, TypeSchema};
    let int = || FieldSchema::Scalar { json: "integer", hale_type: None, unit: None };
    let data = TypeSchema { properties: fields.iter().map(|f| (f.to_string(), int())).collect(), required: Vec::new() };
    hale_types::proto_gen::from_model(&ClientModel {
        surface: "S".to_string(),
        digest: "fnv1a64:0".to_string(),
        members: vec![ClientMember { name: "R::f".to_string(), request: Some(FieldSchema::Ref("Data".to_string())), response: None, error: ClientError::None, requires: Vec::new() }],
        types: [("Data".to_string(), data)].into_iter().collect(),
        streams: Vec::new(),
    })
}

/// What a compiler (`protox`) makes of the file's text, and the descriptor the
/// file serializes for reflection, as `FileDescriptorProto`s.
fn compiled_and_reflected(file: &hale_types::proto_gen::ProtoFile, what: &str) -> (prost_types::FileDescriptorProto, prost_types::FileDescriptorProto) {
    use prost::Message;
    let dir = std::env::temp_dir().join(format!("hale_r8b_{what}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("S.proto"), file.text()).unwrap();
    let set = protox::compile(["S.proto"], [&dir]).unwrap_or_else(|e| panic!("protox: {e}\n{}", file.text()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut compiled = set.file.into_iter().find(|f| f.name() == "S.proto").expect("the file");
    compiled.source_code_info = None;
    (compiled, prost_types::FileDescriptorProto::decode(file.descriptor().as_slice()).expect("a FileDescriptorProto"))
}

/// Where `protox` compiles the text, the descriptor reflection serves is what
/// it makes of it. (The synthetic oneof's name is protoc's own choice in the
/// text, so the descriptor's follows the same rule: pinned in `proto_gen`'s
/// unit tests, where a field `_a` beside `a` can be built, since a record with
/// both is refused by the JSON-name rule before it is written.)
#[test]
fn the_reflected_descriptor_is_the_file_a_compiler_makes_of_the_text() {
    for fields in [vec!["a", "b"], vec!["a", "X_a"], vec!["b", "_c"], vec!["_a_b", "c"]] {
        let (compiled, reflected) = compiled_and_reflected(&data_file(&fields).unwrap(), "descriptor");
        assert_eq!(reflected, compiled, "{fields:?}: the reflected descriptor is the compiled text");
    }
}

// ---- a list is a `repeated` field ----

#[derive(Clone, PartialEq, prost::Message)]
struct PShelf {
    #[prost(int64, repeated, tag = "1")]
    ids: Vec<i64>,
    #[prost(string, repeated, tag = "2")]
    tags: Vec<String>,
    #[prost(message, repeated, tag = "3")]
    parts: Vec<PPart>,
    #[prost(bool, repeated, tag = "4")]
    flags: Vec<bool>,
    #[prost(double, repeated, tag = "5")]
    ratios: Vec<f64>,
}

/// A list is a `repeated` field: what `prost` writes (a numeric list packed,
/// as proto3 writes it by default) the generated decoder reads, and what the
/// generated encoder writes (each element an occurrence of its own) `prost`
/// reads back as the same value. The `.proto`'s descriptor for it is the
/// one a compiler makes of the text.
#[test]
fn a_list_is_a_repeated_field_that_prost_reads_and_writes() {
    use prost::Message;
    let shelf = PShelf {
        ids: vec![41, 0, -7],
        tags: vec!["x".to_string(), "y z".to_string()],
        parts: vec![PPart { a: 1, b: 2 }, PPart { a: 0, b: 9 }],
        flags: vec![true, false],
        ratios: vec![0.5, -2.25],
    };
    let bytes = shelf.encode_to_vec();
    assert_eq!(&bytes[..2], &[0x0a, 0x0c], "prost packs the ids: one length-delimited field 1");
    let program = format!(
        r#"
type Part {{ a: Int = 0; b: Int = 0; }}
type Shelf {{ ids: bounded[Int; 4]; tags: bounded[String; 2]; parts: bounded[Part; 2]; flags: bounded[Bool; 2]; ratios: [Float; 2]; }}
api Pb {{ rpc Probe::shelf; }}
locus Probe {{
    fn shelf(s: Shelf) -> Shelf {{ return s; }}
}}
locus Tokens {{
    fn principal(token: String) -> std::api::Principal {{ return std::api::Principal {{ mode: "bearer", name: "" }}; }}
    fn refused() -> String {{ return "no"; }}
}}
main locus Desk {{
    params {{ probe: Probe = Probe {{ }}; bearer: Tokens = Tokens {{ }}; h: api::Exposure = api::serve(Pb, grpc::Rpc {{ bind: "127.0.0.1:0", codec: json, principals: self.bearer }}, as: "pb", receivers: {{ Probe: self.probe }}, bound: 4, on_full: refuse); }}
    run() {{
        let s = __api_pb_decode_Shelf({bytes}) or {{ println("refused " + err.kind + " " + err.field); return; }};
        println(__api_encode_Shelf(s));
        let out = __api_pb_encode_Shelf(s);
        let mut text = "";
        let mut i = 0;
        while i < len(out) {{
            if i > 0 {{ text = text + ","; }}
            text = text + to_string(std::bytes::read_u8(out, i) or 0);
            i = i + 1;
        }}
        println(text);
    }}
}}
fn main() {{ Desk {{ }}; }}
"#,
        bytes = bytes_expr(&bytes),
    );
    let bin = harness::unique_bin("api_pb_codec_repeated");
    build_opts::build_source(&program, &bin, &build_opts::options()).expect("build the probe");
    let got = run(&bin);
    let mut lines = got.lines();
    assert_eq!(
        lines.next(),
        Some(r#"{"ids":[41,0,-7],"tags":["x","y z"],"parts":[{"a":1,"b":2},{"a":0,"b":9}],"flags":[true,false],"ratios":[0.5,-2.25]}"#),
        "the generated decoder reads what prost wrote"
    );
    let written: Vec<u8> = lines.next().expect("the encoded bytes").split(',').map(|b| b.parse().expect("a byte")).collect();
    assert_eq!(PShelf::decode(written.as_slice()).expect("prost reads it"), shelf, "prost reads what the generated encoder wrote");

    // the descriptor of a repeated field is the compiled text's
    use hale_types::surface_doc::{ClientError, ClientMember, ClientModel};
    use hale_types::surfaces::{FieldSchema, TypeSchema};
    let list = |items: FieldSchema| FieldSchema::List { items: Box::new(items), max: 4, exact: false };
    let data = TypeSchema {
        properties: vec![
            ("ids".to_string(), list(FieldSchema::Scalar { json: "integer", hale_type: None, unit: None })),
            ("parts".to_string(), list(FieldSchema::Ref("Part".to_string()))),
        ],
        required: Vec::new(),
    };
    let part = TypeSchema { properties: vec![("a".to_string(), FieldSchema::Scalar { json: "integer", hale_type: None, unit: None })], required: Vec::new() };
    let file = hale_types::proto_gen::from_model(&ClientModel {
        surface: "S".to_string(),
        digest: "fnv1a64:0".to_string(),
        members: vec![ClientMember { name: "R::f".to_string(), request: Some(FieldSchema::Ref("Data".to_string())), response: None, error: ClientError::None, requires: Vec::new() }],
        types: [("Data".to_string(), data), ("Part".to_string(), part)].into_iter().collect(),
        streams: Vec::new(),
    })
    .expect("a file");
    let (compiled, reflected) = compiled_and_reflected(&file, "repeated");
    assert_eq!(reflected, compiled, "the reflected descriptor of a repeated field is the compiled text");
}

/// A list of `Decimal<S>` is carried by both codecs as a `Decimal<S>` field
/// is (#1532): in JSON each element a number at the element's scale, every
/// digit kept (written as `to_string` writes it, trailing zeros dropped), a nonzero digit past the scale `wrong_type`; in protobuf each
/// element its digits in a string occurrence (length-delimited, so there is
/// no packed form). An api surface still refuses a Decimal row (its schema has
/// none), so the codecs are reached here through the harness's build, which
/// lowers what the check refuses, and called as the transport's thunks would.
#[test]
fn a_list_of_decimals_crosses_both_codecs_at_its_scale() {
    let program = r#"
type Prices { amounts: bounded[Decimal<2>; 3]; exact: [Decimal<2>; 2]; }
api Pb { rpc Probe::prices; }
locus Probe {
    fn prices(p: Prices) -> Prices { return p; }
}
locus Tokens {
    fn principal(token: String) -> std::api::Principal { return std::api::Principal { mode: "bearer", name: "" }; }
    fn refused() -> String { return "no"; }
}
main locus Desk {
    params { probe: Probe = Probe { }; bearer: Tokens = Tokens { }; h: api::Exposure = api::serve(Pb, grpc::Rpc { bind: "127.0.0.1:0", codec: json, principals: self.bearer }, as: "pb", receivers: { Probe: self.probe }, bound: 4, on_full: refuse); }
    fn json(text: String) {
        let p = __api_decode_Prices(text) or { println("json refused " + err.kind + " " + err.field); return; };
        println("json " + __api_encode_Prices(p));
        let back = __api_pb_decode_Prices(__api_pb_encode_Prices(p)) or { println("pb refused " + err.kind + " " + err.field); return; };
        println("pb " + __api_encode_Prices(back));
    }
    fn pb(b: Bytes) {
        let p = __api_pb_decode_Prices(b) or { println("pb refused " + err.kind + " " + err.field); return; };
        println("pb " + __api_encode_Prices(p));
    }
    run() {
        self.json("{\"amounts\":[19.99,0.5,-3],\"exact\":[0.01,100]}");
        self.json("{\"exact\":[1,2]}");
        self.json("{\"amounts\":[1.234],\"exact\":[1,2]}");
        self.json("{\"amounts\":[1,2,3,4],\"exact\":[1,2]}");
        self.json("{\"amounts\":[\"1.5\"],\"exact\":[1,2]}");
        self.json("{\"exact\":[1]}");
        let exact = std::bytes::concat(__api_pb_put_str(2, "1"), __api_pb_put_str(2, "2"));
        self.pb(std::bytes::concat(std::bytes::concat(__api_pb_put_str(1, "7.25"), __api_pb_put_str(1, "-0.01")), exact));
        self.pb(std::bytes::concat(__api_pb_put_str(1, "1.234"), exact));
        self.pb(std::bytes::concat(__api_pb_put_int(1, 5), exact));
    }
}
fn main() { Desk { }; }
"#;
    let bin = harness::unique_bin("api_pb_codec_decimal_list");
    build_opts::build_source(program, &bin, &build_opts::options()).expect("build the probe");
    let got = run(&bin);
    assert_eq!(
        got.lines().collect::<Vec<_>>(),
        [
            r#"json {"amounts":[19.99,0.5,-3],"exact":[0.01,100]}"#,
            r#"pb {"amounts":[19.99,0.5,-3],"exact":[0.01,100]}"#,
            r#"json {"amounts":[],"exact":[1,2]}"#,
            r#"pb {"amounts":[],"exact":[1,2]}"#,
            "json refused wrong_type amounts",
            "json refused too_long amounts",
            "json refused wrong_type amounts",
            "json refused too_short exact",
            r#"pb {"amounts":[7.25,-0.01],"exact":[1,2]}"#,
            "pb refused wrong_type amounts",
            "pb refused wrong_type amounts",
        ]
    );
}

// ---- an enum: a proto3 enum, or a message of one oneof ----

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, prost::Enumeration)]
#[repr(i32)]
enum PStatus {
    Up = 0,
    Down = 1,
    Gone = 2,
}

#[derive(Clone, PartialEq, prost::Message)]
struct PRect {
    #[prost(int64, tag = "1")]
    v0: i64,
    #[prost(int64, tag = "2")]
    v1: i64,
}

#[derive(Clone, PartialEq, prost::Oneof)]
enum PShapeValue {
    #[prost(bool, tag = "1")]
    Empty(bool),
    #[prost(double, tag = "2")]
    Circle(f64),
    #[prost(string, tag = "3")]
    Tag(String),
    #[prost(message, tag = "4")]
    Rect(PRect),
    #[prost(enumeration = "PStatus", tag = "5")]
    Mood(i32),
}

#[derive(Clone, PartialEq, prost::Message)]
struct PShape {
    #[prost(oneof = "PShapeValue", tags = "1, 2, 3, 4, 5")]
    value: Option<PShapeValue>,
}

#[derive(Clone, PartialEq, prost::Message)]
struct PHolder {
    #[prost(enumeration = "PStatus", optional, tag = "1")]
    s: Option<i32>,
    #[prost(message, optional, tag = "2")]
    shape: Option<PShape>,
    #[prost(enumeration = "PStatus", repeated, tag = "3")]
    many: Vec<i32>,
}

/// An enum crosses protobuf as proto3 does (spec/api.md § Codecs): a
/// payload-free one an `enum`, its variants numbered from 0; one with payloads
/// a message of one `oneof`, its members numbered from 1, a variant of several
/// values a message of fields `v0`, `v1`. What `prost` writes the generated
/// decoder reads (a repeated enum packed, as proto3 writes it), what the
/// generated encoder writes `prost` reads back as the same value, and the
/// `.proto`'s descriptor is the one a compiler makes of its text.
#[test]
fn an_enum_is_a_proto3_enum_or_a_oneof_that_prost_reads_and_writes() {
    use prost::Message;
    let cases = [
        (
            PHolder { s: Some(PStatus::Gone as i32), shape: Some(PShape { value: Some(PShapeValue::Rect(PRect { v0: 2, v1: -3 })) }), many: vec![1, 0, 2] },
            r#"{"s":"Gone","shape":{"Rect":[2,-3]},"many":["Down","Up","Gone"]}"#,
        ),
        (
            PHolder { s: Some(PStatus::Up as i32), shape: Some(PShape { value: Some(PShapeValue::Empty(true)) }), many: vec![] },
            r#"{"s":"Up","shape":"Empty","many":[]}"#,
        ),
        (
            PHolder { s: Some(PStatus::Down as i32), shape: Some(PShape { value: Some(PShapeValue::Tag("x y".to_string())) }), many: vec![] },
            r#"{"s":"Down","shape":{"Tag":"x y"},"many":[]}"#,
        ),
        (
            PHolder { s: Some(PStatus::Down as i32), shape: Some(PShape { value: Some(PShapeValue::Mood(PStatus::Gone as i32)) }), many: vec![] },
            r#"{"s":"Down","shape":{"Mood":"Gone"},"many":[]}"#,
        ),
    ];
    let mut probes = String::new();
    for (holder, _) in &cases {
        probes.push_str(&format!("        self.probe_one({});\n", bytes_expr(&holder.encode_to_vec())));
    }
    let program = format!(
        r#"
type Status = enum {{ Up, Down, Gone }};
type Shape = enum {{ Empty, Circle(Float), Tag(String), Rect(Int, Int), Mood(Status) }};
type Holder {{ s: Status; shape: Shape; many: bounded[Status; 4]; }}
api Pb {{ rpc Probe::holder; }}
locus Probe {{
    fn holder(h: Holder) -> Holder {{ return h; }}
}}
locus Tokens {{
    fn principal(token: String) -> std::api::Principal {{ return std::api::Principal {{ mode: "bearer", name: "" }}; }}
    fn refused() -> String {{ return "no"; }}
}}
main locus Desk {{
    params {{ probe: Probe = Probe {{ }}; bearer: Tokens = Tokens {{ }}; h: api::Exposure = api::serve(Pb, grpc::Rpc {{ bind: "127.0.0.1:0", codec: json, principals: self.bearer }}, as: "pb", receivers: {{ Probe: self.probe }}, bound: 4, on_full: refuse); }}
    fn probe_one(b: Bytes) {{
        let h = __api_pb_decode_Holder(b) or {{ println("refused " + err.kind + " " + err.field); return; }};
        println(__api_encode_Holder(h));
        let out = __api_pb_encode_Holder(h);
        let mut text = "";
        let mut i = 0;
        while i < len(out) {{
            if i > 0 {{ text = text + ","; }}
            text = text + to_string(std::bytes::read_u8(out, i) or 0);
            i = i + 1;
        }}
        println(text);
    }}
    run() {{
{probes}    }}
}}
fn main() {{ Desk {{ }}; }}
"#
    );
    let bin = harness::unique_bin("api_pb_codec_enum");
    build_opts::build_source(&program, &bin, &build_opts::options()).expect("build the probe");
    let got = run(&bin);
    let mut lines = got.lines();
    for (holder, json) in &cases {
        assert_eq!(lines.next(), Some(*json), "the generated decoder reads what prost wrote");
        let written: Vec<u8> = lines.next().expect("the encoded bytes").split(',').map(|b| b.parse().expect("a byte")).collect();
        assert_eq!(&PHolder::decode(written.as_slice()).expect("prost reads it"), holder, "prost reads what the generated encoder wrote");
    }

    // the descriptor of an enum and of a oneof message is the compiled text's
    use hale_types::surface_doc::{ClientError, ClientMember, ClientModel};
    use hale_types::surfaces::{FieldSchema, TypeSchema};
    let status = FieldSchema::Enum { name: "Status".to_string(), variants: ["Up", "Down", "Gone"].iter().map(|v| (v.to_string(), Vec::new())).collect() };
    let int = || FieldSchema::Scalar { json: "integer", hale_type: None, unit: None };
    let shape = FieldSchema::Enum {
        name: "Shape".to_string(),
        variants: vec![
            ("Empty".to_string(), Vec::new()),
            ("Circle".to_string(), vec![FieldSchema::Scalar { json: "number", hale_type: None, unit: None }]),
            ("Rect".to_string(), vec![int(), int()]),
            ("Mood".to_string(), vec![status.clone()]),
        ],
    };
    let holder = TypeSchema {
        properties: vec![
            ("s".to_string(), status.clone()),
            ("shape".to_string(), shape),
            ("many".to_string(), FieldSchema::List { items: Box::new(status), max: 4, exact: false }),
        ],
        required: Vec::new(),
    };
    let file = hale_types::proto_gen::from_model(&ClientModel {
        surface: "S".to_string(),
        digest: "fnv1a64:0".to_string(),
        members: vec![ClientMember { name: "R::f".to_string(), request: Some(FieldSchema::Ref("Holder".to_string())), response: None, error: ClientError::None, requires: Vec::new() }],
        types: [("Holder".to_string(), holder)].into_iter().collect(),
        streams: Vec::new(),
    })
    .expect("a file");
    let (compiled, reflected) = compiled_and_reflected(&file, "enum");
    assert_eq!(reflected, compiled, "the reflected descriptor of an enum and a oneof is the compiled text");
}

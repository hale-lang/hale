//! GH #1417 (R8b): the `.proto` of a surface, and the `FileDescriptorProto`
//! it compiles to, both generated from the surface's rows.
//!
//! The wire speaks it: `grpc::Rpc` serves `application/grpc` and
//! `application/grpc+proto` with the messages declared here (spec/api.md
//! § gRPC), and answers gRPC server reflection with the descriptor
//! [`ProtoFile::descriptor`] builds. One model, [`ProtoFile`], is rendered
//! two ways, so the text a client compiles and the descriptor a server
//! reflects are one file.
//!
//! The rules (each is pinned by a test):
//!   - a record is a top-level message named by the schema document's name
//!     for it (`lib::Item` is `lib_Item`); a field that names another record
//!     is a field of that message type;
//!   - field numbers are the struct's declaration order, `1, 2, …`: the
//!     surface digest folds the fields in that order, so a reorder or an
//!     insertion moves the digest, and a number never moves while the digest
//!     holds;
//!   - every scalar field is `optional`: the codec tells an absent field
//!     (`missing_field`, or its default) from a zero one, and proto3 tells
//!     them apart only for a field with presence;
//!   - `Int` is `int64`, `Float` is `double`, `Bool` is `bool`, `String` is
//!     `string`; an identity or a quantity is its `Int`, a comment naming the
//!     Hale type;
//!   - a row whose request, response or error is not a record (a scalar) is
//!     carried in a one-field message `<rpc>Request`, `<rpc>Response` or
//!     `<rpc>Error` (`value = 1`); a row with none is `HaleEmpty`;
//!   - an rpc is named as an MCP tool is (`Orders::place` is `Orders__place`),
//!     because a method name cannot hold `::`.
//!
//! A shape the schema walker cannot express (an enum, an array, a name the
//! program does not declare) is not approximated: [`from_model`] refuses
//! with the row and the shape named.

use std::collections::BTreeMap;

use crate::surface_doc::{tool_name, ClientError, ClientModel};
use crate::surfaces::{FieldSchema, TypeSchema};

/// A scalar of proto3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scalar {
    Double,
    Int32,
    Int64,
    Bool,
    String,
    Bytes,
}

impl Scalar {
    fn keyword(self) -> &'static str {
        match self {
            Scalar::Double => "double",
            Scalar::Int32 => "int32",
            Scalar::Int64 => "int64",
            Scalar::Bool => "bool",
            Scalar::String => "string",
            Scalar::Bytes => "bytes",
        }
    }

    fn descriptor_type(self) -> u64 {
        match self {
            Scalar::Double => 1,
            Scalar::Int64 => 3,
            Scalar::Int32 => 5,
            Scalar::Bool => 8,
            Scalar::String => 9,
            Scalar::Bytes => 12,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ty {
    Scalar(Scalar),
    /// A message of the file (or of the package), by its name.
    Message(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Label {
    Plain,
    /// proto3 `optional`: the field has presence.
    Optional,
    Repeated,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PField {
    pub name: String,
    pub number: u32,
    pub ty: Ty,
    pub label: Label,
    /// The `oneof` (an index into the message's `oneofs`) the field is in.
    pub oneof: Option<usize>,
    /// What the field is in Hale, where the wire type is not all of it.
    pub comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PMessage {
    pub name: String,
    pub comment: Vec<String>,
    pub fields: Vec<PField>,
    pub oneofs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PMethod {
    pub name: String,
    pub input: String,
    pub output: String,
    pub client_stream: bool,
    pub server_stream: bool,
    pub comment: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PService {
    pub name: String,
    pub methods: Vec<PMethod>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtoFile {
    /// The file's name as a descriptor states it (`Public.proto`).
    pub name: String,
    /// `""` for none: the service is then named by its bare name.
    pub package: String,
    pub header: Vec<String>,
    pub messages: Vec<PMessage>,
    pub service: Option<PService>,
}

fn field(name: &str, number: u32, ty: Ty, label: Label) -> PField {
    PField { name: name.to_string(), number, ty, label, oneof: None, comment: None }
}

fn message(name: &str, fields: Vec<PField>) -> PMessage {
    PMessage { name: name.to_string(), comment: Vec::new(), fields, oneofs: Vec::new() }
}

/// The message a record has in the file: its document name with the `::` of
/// an imported type spelled `_`.
pub fn message_name(display: &str) -> String {
    display.replace("::", "_")
}

/// The rpc a member is: `Orders::place` is `Orders__place`.
pub fn rpc_name(member: &str) -> String {
    tool_name(member)
}

/// The one-field message that carries a scalar `what` (`Request`, `Response`,
/// `Error`) of an rpc.
pub fn wrapper_name(member: &str, what: &str) -> String {
    format!("{}{what}", rpc_name(member))
}

/// The refusal's message: `kind`, `reason`, and the `requires` or `served`
/// a refusal of that kind carries.
pub const REFUSAL: &str = "HaleRefusal";
/// The message of a row with no request or no response.
pub const EMPTY: &str = "HaleEmpty";

/// The type URL of a detail that carries `message` (the handler error or a
/// refusal): `grpc-status-details-bin`'s `Any`.
pub fn type_url(message: &str) -> String {
    format!("type.hale.dev/{message}")
}

fn is_ident(s: &str) -> bool {
    let mut cs = s.chars();
    matches!(cs.next(), Some(c) if c.is_ascii_alphabetic() || c == '_') && cs.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn scalar_of(json: &str) -> Option<Scalar> {
    match json {
        "integer" => Some(Scalar::Int64),
        "number" => Some(Scalar::Double),
        "boolean" => Some(Scalar::Bool),
        "string" => Some(Scalar::String),
        _ => None,
    }
}

/// A scalar field's comment: the Hale type it stands for.
fn hale_comment(hale_type: &Option<String>, unit: &Option<String>) -> Option<String> {
    match (hale_type, unit) {
        (Some(t), Some(u)) => Some(format!("Hale: {t}, {u}")),
        (Some(t), None) => Some(format!("Hale: {t}")),
        (None, Some(u)) => Some(format!("Hale: {u}")),
        (None, None) => None,
    }
}

/// What a row's request, response or error is on the wire: the message that
/// carries it, and a wrapper message if it is a scalar.
fn carried(
    member: &str,
    what: &str,
    f: &FieldSchema,
    wrappers: &mut Vec<PMessage>,
) -> Result<String, String> {
    match f {
        FieldSchema::Ref(name) => Ok(message_name(name)),
        FieldSchema::Scalar { json, hale_type, unit } => {
            let Some(s) = scalar_of(json) else {
                return Err(format!("{member}: the {what} is a `{json}`, which proto3 has no scalar for"));
            };
            let name = wrapper_name(member, what);
            let mut v = field("value", 1, Ty::Scalar(s), Label::Optional);
            v.comment = hale_comment(hale_type, unit);
            let mut m = message(&name, vec![v]);
            m.comment = vec![format!("The {} of `{member}`, a scalar.", what.to_lowercase())];
            wrappers.push(m);
            Ok(name)
        }
        FieldSchema::Unformed => Err(format!(
            "{member}: the {what} is a shape the codec does not carry (an enum, an array or a name the program does not declare), \
             so it has no proto3 encoding; no approximation is made"
        )),
    }
}

/// The `.proto` model of a surface.
pub fn from_model(model: &ClientModel) -> Result<ProtoFile, String> {
    let surface = &model.surface;
    if !is_ident(surface) {
        return Err(format!("the surface `{surface}` is not a proto3 service name"));
    }
    // the records the rows reach, in the document's order (a hub's payload
    // is no rpc's: the model holds it for the clients, not for this file)
    let mut reached: Vec<&str> = Vec::new();
    {
        let mut work: Vec<&str> = Vec::new();
        for m in &model.members {
            let error = match &m.error {
                ClientError::Type(f) => Some(f),
                _ => None,
            };
            for f in m.request.iter().chain(m.response.iter()).chain(error) {
                if let FieldSchema::Ref(n) = f {
                    work.push(n);
                }
            }
        }
        while let Some(n) = work.pop() {
            if reached.contains(&n) {
                continue;
            }
            reached.push(n);
            if let Some(t) = model.types.get(n) {
                for (_, f) in &t.properties {
                    if let FieldSchema::Ref(r) = f {
                        work.push(r);
                    }
                }
            }
        }
    }
    let mut messages: Vec<PMessage> = Vec::new();
    for (display, schema) in model.types.iter().filter(|(d, _)| reached.contains(&d.as_str())) {
        messages.push(record(display, schema)?);
    }
    // the rpcs, in the rows' order
    let mut wrappers: Vec<PMessage> = Vec::new();
    let mut methods = Vec::new();
    let mut uses_empty = false;
    for m in &model.members {
        let mut comment = vec![format!("rpc {}", m.name)];
        comment.push(if m.requires.is_empty() {
            "requires no role".to_string()
        } else {
            format!("requires {}", m.requires.join(", "))
        });
        let input = match &m.request {
            Some(f) => carried(&m.name, "Request", f, &mut wrappers)?,
            None => {
                uses_empty = true;
                EMPTY.to_string()
            }
        };
        let output = match &m.response {
            Some(f) => carried(&m.name, "Response", f, &mut wrappers)?,
            None => {
                uses_empty = true;
                EMPTY.to_string()
            }
        };
        match &m.error {
            ClientError::None => {}
            ClientError::Server => comment.push("a violation is the server error (ClosureViolation): status 13, no handler error".to_string()),
            ClientError::Type(f) => {
                let e = carried(&m.name, "Error", f, &mut wrappers)?;
                comment.push(format!("handler error: status 9, a detail of type {}", type_url(&e)));
            }
        }
        methods.push(PMethod {
            name: rpc_name(&m.name),
            input,
            output,
            client_stream: false,
            server_stream: false,
            comment,
        });
    }
    messages.extend(wrappers);
    if uses_empty {
        let mut e = message(EMPTY, Vec::new());
        e.comment = vec!["A row with no request, or no response.".to_string()];
        messages.push(e);
    }
    let mut refusal = message(
        REFUSAL,
        vec![
            field("kind", 1, Ty::Scalar(Scalar::String), Label::Plain),
            field("reason", 2, Ty::Scalar(Scalar::String), Label::Plain),
            field("requires", 3, Ty::Scalar(Scalar::String), Label::Repeated),
            field("served", 4, Ty::Scalar(Scalar::String), Label::Plain),
        ],
    );
    refusal.comment = vec![
        "A refusal, and the server error (kind \"server\"): the detail of its status.".to_string(),
        "`requires` is the roles of an unauthorized refusal, `served` the digest of a digest_mismatch one.".to_string(),
    ];
    messages.push(refusal);
    // names share one scope
    let mut seen: BTreeMap<&str, ()> = BTreeMap::new();
    seen.insert(surface.as_str(), ());
    for m in &messages {
        if seen.insert(m.name.as_str(), ()).is_some() {
            return Err(format!(
                "the message `{}` is named twice in the file of {surface} (a type of the program, the surface, or a name this file generates)",
                m.name
            ));
        }
    }
    let mut names: BTreeMap<&str, ()> = BTreeMap::new();
    for m in &methods {
        if names.insert(m.name.as_str(), ()).is_some() {
            return Err(format!("the rpc `{}` is named twice in {surface}", m.name));
        }
    }
    Ok(ProtoFile {
        name: format!("{surface}.proto"),
        package: String::new(),
        header: header(model, uses_empty),
        messages,
        service: Some(PService { name: surface.clone(), methods }),
    })
}

fn record(display: &str, schema: &TypeSchema) -> Result<PMessage, String> {
    let name = message_name(display);
    if !is_ident(&name) {
        return Err(format!("the type `{display}` is not a proto3 message name"));
    }
    let mut fields = Vec::new();
    for (i, (key, f)) in schema.properties.iter().enumerate() {
        if !is_ident(key) {
            return Err(format!("{display}: the field `{key}` is not a proto3 field name"));
        }
        let number = i as u32 + 1;
        fields.push(match f {
            FieldSchema::Scalar { json, hale_type, unit } => {
                let Some(s) = scalar_of(json) else {
                    return Err(format!("{display}.{key}: `{json}` has no proto3 scalar"));
                };
                let mut p = field(key, number, Ty::Scalar(s), Label::Optional);
                p.comment = hale_comment(hale_type, unit);
                p
            }
            FieldSchema::Ref(n) => field(key, number, Ty::Message(message_name(n)), Label::Plain),
            FieldSchema::Unformed => {
                return Err(format!(
                    "{display}.{key}: a shape the codec does not carry (an enum, an array or a name the program does not declare) \
                     has no proto3 encoding; no approximation is made"
                ))
            }
        });
    }
    let mut m = message(&name, fields);
    if display != name {
        m.comment = vec![format!("Hale: {display}")];
    }
    Ok(m)
}

fn header(model: &ClientModel, uses_empty: bool) -> Vec<String> {
    let s = &model.surface;
    let mut h = vec![
        format!("Generated by `hale api export` from the rows of the surface {s} ({}).", model.digest),
        "Do not edit it: regenerate it. `hale api export --check` refuses a copy that drifted.".to_string(),
        String::new(),
        format!("The wire (spec/api.md § gRPC): POST /{s}/<rpc>, one unary message each way, content-type"),
        "application/grpc or application/grpc+proto (application/grpc+json carries the row's JSON).".to_string(),
        "An rpc is named as an MCP tool is: Orders::place is Orders__place.".to_string(),
        String::new(),
        "Field numbers are the declaration order of the Hale struct's fields, 1, 2, …: the surface digest".to_string(),
        "folds the fields in that order, so a reorder or an insertion moves the digest and no number".to_string(),
        "moves while the digest holds. Every scalar field is `optional`: an absent field is not a zero one".to_string(),
        "(a field with no default that is absent is a `malformed` refusal, missing_field). An identity or a".to_string(),
        "quantity is its Int, int64, and the comment names its Hale type.".to_string(),
        String::new(),
        "The five outcomes of a call:".to_string(),
        "  result           status OK (0); the message is the rpc's response.".to_string(),
        "  handler error    status FAILED_PRECONDITION (9); grpc-status-details-bin is a google.rpc.Status".to_string(),
        "                   whose one detail is an Any, type URL type.hale.dev/<the rpc's error message>,".to_string(),
        "                   value that message. Only an rpc that declares an error type has one.".to_string(),
        "  refusal          malformed INVALID_ARGUMENT (3), digest_mismatch FAILED_PRECONDITION (9),".to_string(),
        "                   unauthenticated UNAUTHENTICATED (16), unauthorized PERMISSION_DENIED (7), full".to_string(),
        "                   RESOURCE_EXHAUSTED (8), shutting_down and unavailable UNAVAILABLE (14); the".to_string(),
        format!("                   detail is an Any, type URL {}, value {REFUSAL}.", type_url(REFUSAL)),
        format!("  server error     status INTERNAL (13); the detail is {REFUSAL} with kind \"server\"."),
        "  transport        the stream is reset or the connection ends without a status: the call may have run.".to_string(),
    ];
    if uses_empty {
        h.push(String::new());
        h.push(format!("{EMPTY} is the message of a row that takes, or returns, nothing."));
    }
    h
}

// ---- the text ----

impl ProtoFile {
    /// The `.proto` source.
    pub fn text(&self) -> String {
        let mut o = String::new();
        for l in &self.header {
            if l.is_empty() {
                o.push_str("//\n");
            } else {
                o.push_str(&format!("// {l}\n"));
            }
        }
        o.push_str("\nsyntax = \"proto3\";\n");
        if !self.package.is_empty() {
            o.push_str(&format!("\npackage {};\n", self.package));
        }
        if let Some(svc) = &self.service {
            o.push_str(&format!("\nservice {} {{\n", svc.name));
            for (i, m) in svc.methods.iter().enumerate() {
                if i > 0 {
                    o.push('\n');
                }
                for c in &m.comment {
                    o.push_str(&format!("  // {c}\n"));
                }
                let (cs, ss) = (if m.client_stream { "stream " } else { "" }, if m.server_stream { "stream " } else { "" });
                o.push_str(&format!("  rpc {}({cs}{}) returns ({ss}{});\n", m.name, m.input, m.output));
            }
            o.push_str("}\n");
        }
        for m in &self.messages {
            o.push('\n');
            for c in &m.comment {
                o.push_str(&format!("// {c}\n"));
            }
            if m.fields.is_empty() {
                o.push_str(&format!("message {} {{}}\n", m.name));
                continue;
            }
            o.push_str(&format!("message {} {{\n", m.name));
            for (k, name) in m.oneofs.iter().enumerate() {
                o.push_str(&format!("  oneof {name} {{\n"));
                for f in m.fields.iter().filter(|f| f.oneof == Some(k)) {
                    o.push_str(&format!("    {}\n", field_text(f)));
                }
                o.push_str("  }\n");
            }
            for f in m.fields.iter().filter(|f| f.oneof.is_none()) {
                o.push_str(&format!("  {}\n", field_text(f)));
            }
            o.push_str("}\n");
        }
        o
    }

    /// The fully qualified names a reflection client may ask for: the
    /// service, its methods and the messages.
    pub fn symbols(&self) -> Vec<String> {
        let q = |n: &str| if self.package.is_empty() { n.to_string() } else { format!("{}.{n}", self.package) };
        let mut out = Vec::new();
        if let Some(svc) = &self.service {
            out.push(q(&svc.name));
            for m in &svc.methods {
                out.push(format!("{}.{}", q(&svc.name), m.name));
            }
        }
        for m in &self.messages {
            out.push(q(&m.name));
        }
        out
    }
}

fn field_text(f: &PField) -> String {
    let ty = match &f.ty {
        Ty::Scalar(s) => s.keyword().to_string(),
        Ty::Message(m) => m.clone(),
    };
    let label = match f.label {
        Label::Plain => "",
        Label::Optional => "optional ",
        Label::Repeated => "repeated ",
    };
    let tail = f.comment.as_ref().map_or(String::new(), |c| format!(" // {c}"));
    format!("{label}{ty} {} = {};{tail}", f.name, f.number)
}

// ---- the descriptor ----

fn varint(mut n: u64, out: &mut Vec<u8>) {
    while n >= 0x80 {
        out.push((n & 0x7f) as u8 | 0x80);
        n >>= 7;
    }
    out.push(n as u8);
}

fn tag(number: u32, wire: u8, out: &mut Vec<u8>) {
    varint(u64::from(number) << 3 | u64::from(wire), out);
}

fn put_varint(number: u32, v: u64, out: &mut Vec<u8>) {
    tag(number, 0, out);
    varint(v, out);
}

fn put_bytes(number: u32, b: &[u8], out: &mut Vec<u8>) {
    tag(number, 2, out);
    varint(b.len() as u64, out);
    out.extend_from_slice(b);
}

fn put_str(number: u32, s: &str, out: &mut Vec<u8>) {
    put_bytes(number, s.as_bytes(), out);
}

/// protoc's `json_name` of a field: the underscores removed, the letter after
/// each capitalised.
fn json_name(name: &str) -> String {
    let mut o = String::new();
    let mut up = false;
    for c in name.chars() {
        if c == '_' {
            up = true;
        } else if up {
            o.push(c.to_ascii_uppercase());
            up = false;
        } else {
            o.push(c);
        }
    }
    o
}

impl ProtoFile {
    /// The `FileDescriptorProto` this file compiles to, serialized as protoc
    /// serializes it (no source info): the answer of server reflection.
    pub fn descriptor(&self) -> Vec<u8> {
        let q = |n: &str| if self.package.is_empty() { format!(".{n}") } else { format!(".{}.{n}", self.package) };
        let mut file = Vec::new();
        put_str(1, &self.name, &mut file);
        if !self.package.is_empty() {
            put_str(2, &self.package, &mut file);
        }
        for m in &self.messages {
            let mut msg = Vec::new();
            put_str(1, &m.name, &mut msg);
            // a proto3 `optional` field has a synthetic oneof of its own, after the real ones
            let mut synthetic = Vec::new();
            for f in &m.fields {
                let mut fd = Vec::new();
                put_str(1, &f.name, &mut fd);
                put_varint(3, u64::from(f.number), &mut fd);
                put_varint(4, if f.label == Label::Repeated { 3 } else { 1 }, &mut fd);
                match &f.ty {
                    Ty::Scalar(s) => put_varint(5, s.descriptor_type(), &mut fd),
                    Ty::Message(n) => {
                        put_varint(5, 11, &mut fd);
                        put_str(6, &q(n), &mut fd);
                    }
                }
                let index = match (f.oneof, f.label) {
                    (Some(k), _) => Some(k),
                    (None, Label::Optional) => {
                        synthetic.push(format!("_{}", f.name));
                        Some(m.oneofs.len() + synthetic.len() - 1)
                    }
                    _ => None,
                };
                if let Some(k) = index {
                    put_varint(9, k as u64, &mut fd);
                }
                put_str(10, &json_name(&f.name), &mut fd);
                if f.label == Label::Optional && f.oneof.is_none() {
                    put_varint(17, 1, &mut fd);
                }
                put_bytes(2, &fd, &mut msg);
            }
            for o in m.oneofs.iter().chain(synthetic.iter()) {
                let mut od = Vec::new();
                put_str(1, o, &mut od);
                put_bytes(8, &od, &mut msg);
            }
            put_bytes(4, &msg, &mut file);
        }
        if let Some(svc) = &self.service {
            let mut sd = Vec::new();
            put_str(1, &svc.name, &mut sd);
            for m in &svc.methods {
                let mut md = Vec::new();
                put_str(1, &m.name, &mut md);
                put_str(2, &q(&m.input), &mut md);
                put_str(3, &q(&m.output), &mut md);
                if m.client_stream {
                    put_varint(5, 1, &mut md);
                }
                if m.server_stream {
                    put_varint(6, 1, &mut md);
                }
                put_bytes(2, &md, &mut sd);
            }
            put_bytes(6, &sd, &mut file);
        }
        put_str(12, "proto3", &mut file);
        file
    }
}

// ---- the files every gRPC exposure serves beside the surface's ----

/// `hale.api.Description`: the reserved method that returns the description.
pub fn description_file() -> ProtoFile {
    let mut doc = message("DescriptionDocument", vec![field("json", 1, Ty::Scalar(Scalar::String), Label::Plain)]);
    doc.comment = vec!["The description document (spec/api.md § The description), as JSON text.".to_string()];
    ProtoFile {
        name: "hale/api/description.proto".to_string(),
        package: "hale.api".to_string(),
        header: vec!["The reserved method of every gRPC exposure: the description for the caller its bearer names.".to_string()],
        messages: vec![message("DescribeRequest", Vec::new()), doc],
        service: Some(PService {
            name: "Description".to_string(),
            methods: vec![PMethod {
                name: "Describe".to_string(),
                input: "DescribeRequest".to_string(),
                output: "DescriptionDocument".to_string(),
                client_stream: false,
                server_stream: false,
                comment: Vec::new(),
            }],
        }),
    }
}

/// `grpc.reflection.v1.ServerReflection`, as grpc's reflection.proto states it.
pub fn reflection_file() -> ProtoFile {
    let s = |n: &str, no: u32| field(n, no, Ty::Scalar(Scalar::String), Label::Plain);
    let m = |n: &str, no: u32, t: &str| field(n, no, Ty::Message(t.to_string()), Label::Plain);
    let in_oneof = |mut f: PField| {
        f.oneof = Some(0);
        f
    };
    let mut request = message("ServerReflectionRequest", Vec::new());
    // the oneof: a filename, a symbol, an extension, a type, or the list
    request.fields = vec![
        s("host", 1),
        in_oneof(s("file_by_filename", 3)),
        in_oneof(s("file_containing_symbol", 4)),
        in_oneof(m("file_containing_extension", 5, "ExtensionRequest")),
        in_oneof(s("all_extension_numbers_of_type", 6)),
        in_oneof(s("list_services", 7)),
    ];
    request.oneofs = vec!["message_request".to_string()];
    let ext = message(
        "ExtensionRequest",
        vec![s("containing_type", 1), field("extension_number", 2, Ty::Scalar(Scalar::Int32), Label::Plain)],
    );
    let mut response = message("ServerReflectionResponse", Vec::new());
    response.fields = vec![
        s("valid_host", 1),
        m("original_request", 2, "ServerReflectionRequest"),
        in_oneof(m("file_descriptor_response", 4, "FileDescriptorResponse")),
        in_oneof(m("all_extension_numbers_response", 5, "ExtensionNumberResponse")),
        in_oneof(m("list_services_response", 6, "ListServiceResponse")),
        in_oneof(m("error_response", 7, "ErrorResponse")),
    ];
    response.oneofs = vec!["message_response".to_string()];
    let fdr = message("FileDescriptorResponse", vec![field("file_descriptor_proto", 1, Ty::Scalar(Scalar::Bytes), Label::Repeated)]);
    let enr = message(
        "ExtensionNumberResponse",
        vec![s("base_type_name", 1), field("extension_number", 2, Ty::Scalar(Scalar::Int32), Label::Repeated)],
    );
    let lsr = message("ListServiceResponse", vec![field("service", 1, Ty::Message("ServiceResponse".to_string()), Label::Repeated)]);
    let sr = message("ServiceResponse", vec![s("name", 1)]);
    let er = message("ErrorResponse", vec![field("error_code", 1, Ty::Scalar(Scalar::Int32), Label::Plain), s("error_message", 2)]);
    ProtoFile {
        name: "grpc/reflection/v1/reflection.proto".to_string(),
        package: "grpc.reflection.v1".to_string(),
        header: vec!["gRPC server reflection (grpc/reflection/v1/reflection.proto).".to_string()],
        messages: vec![request, ext, response, fdr, enr, lsr, sr, er],
        service: Some(PService {
            name: "ServerReflection".to_string(),
            methods: vec![PMethod {
                name: "ServerReflectionInfo".to_string(),
                input: "ServerReflectionRequest".to_string(),
                output: "ServerReflectionResponse".to_string(),
                client_stream: true,
                server_stream: true,
                comment: Vec::new(),
            }],
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface_doc::ClientMember;

    fn model(request: FieldSchema, types: Vec<(&str, TypeSchema)>) -> ClientModel {
        ClientModel {
            surface: "S".to_string(),
            digest: "fnv1a64:0".to_string(),
            members: vec![ClientMember {
                name: "R::f".to_string(),
                request: Some(request),
                response: None,
                error: ClientError::None,
                requires: Vec::new(),
            }],
            types: types.into_iter().map(|(n, t)| (n.to_string(), t)).collect(),
            streams: Vec::new(),
        }
    }

    fn int() -> FieldSchema {
        FieldSchema::Scalar { json: "integer", hale_type: None, unit: None }
    }

    /// A shape the walker cannot express is refused with the row and the
    /// field named, never approximated.
    #[test]
    fn a_shape_the_walker_cannot_express_is_a_stop() {
        let t = TypeSchema { properties: vec![("xs".to_string(), FieldSchema::Unformed)], required: vec!["xs".to_string()] };
        let err = from_model(&model(FieldSchema::Ref("T".to_string()), vec![("T", t)])).unwrap_err();
        assert!(err.contains("T.xs") && err.contains("no proto3 encoding"), "{err}");
        let err = from_model(&model(FieldSchema::Unformed, Vec::new())).unwrap_err();
        assert!(err.contains("R::f") && err.contains("Request"), "{err}");
    }

    /// Two names that land on one message are refused, not renamed.
    #[test]
    fn a_name_that_is_taken_is_a_stop() {
        let t = TypeSchema { properties: vec![("a".to_string(), int())], required: vec!["a".to_string()] };
        let outer = TypeSchema { properties: vec![("t".to_string(), FieldSchema::Ref("a_T".to_string()))], required: Vec::new() };
        let err = from_model(&model(FieldSchema::Ref("a::T".to_string()), vec![("a::T", outer), ("a_T", t)])).unwrap_err();
        assert!(err.contains("`a_T` is named twice"), "{err}");
    }

    /// The text and the descriptor are one model: the descriptor names every
    /// message, field number and rpc the text does.
    #[test]
    fn the_descriptor_carries_what_the_text_declares() {
        let t = TypeSchema { properties: vec![("a".to_string(), int())], required: vec!["a".to_string()] };
        let file = from_model(&model(FieldSchema::Ref("T".to_string()), vec![("T", t)])).unwrap();
        let text = file.text();
        assert!(text.contains("rpc R__f(T) returns (HaleEmpty);"), "{text}");
        assert!(text.contains("optional int64 a = 1;"), "{text}");
        let d = file.descriptor();
        let has = |needle: &[u8]| d.windows(needle.len()).any(|w| w == needle);
        assert!(has(b"S.proto") && has(b"R__f") && has(b".HaleEmpty") && has(b"_a") && has(b"proto3"));
        assert_eq!(file.symbols(), ["S", "S.R__f", "T", "HaleEmpty", "HaleRefusal"]);
    }
}

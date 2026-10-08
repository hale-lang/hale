//! GH #1417 (R1): the documents `hale check --api` prints from the
//! surface rows (spec/api.md § The description; the format is
//! `spec/api-description.schema.json`), and the OpenAPI, JSON Schema and
//! MCP forms of one surface, projections of the same rows.
//!
//! Every document is rendered from `hale_types::surfaces::SurfaceRows`
//! (the snapshot's `surface` family) and the type declarations' schemas
//! (`Schemas`), never from a locus's structure. A document is a value
//! whose keys keep the schema's order, so it is built here as [`J`] and
//! printed pretty, two spaces an indent, the form the R0 fixtures under
//! `tests/api-contract/` are written in; `serde_json`'s map would sort
//! the keys.
//!
//! What R1 cannot know is said where it is asked: a caller's grants are
//! its role source's to decide when the program runs (`Grants::holds`
//! is program code), so a description for a caller takes the roles the
//! caller holds under that exposure's source as an input (`--holds`),
//! and the principal as the exposure would establish it (`--caller`).

use std::collections::{BTreeMap, BTreeSet};

use hale_model::surface::digest_text;
use hale_types::surfaces::{FieldSchema, Handled, Hub, Row, Schemas, Serve, Source, Stream, SurfaceRows, TypeSchema};

/// A JSON value whose objects keep their keys in the order written.
#[derive(Debug, Clone, PartialEq)]
pub enum J {
    Null,
    Bool(bool),
    Int(i64),
    Str(String),
    Arr(Vec<J>),
    Obj(Vec<(String, J)>),
}

fn s(v: &str) -> J {
    J::Str(v.to_string())
}

fn o(pairs: Vec<(&str, J)>) -> J {
    J::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

fn strs<'a>(v: impl IntoIterator<Item = &'a str>) -> J {
    J::Arr(v.into_iter().map(s).collect())
}

/// A string as JSON writes it, escaped as `serde_json` escapes.
fn quote(v: &str, out: &mut String) {
    out.push('"');
    for c in v.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

impl J {
    fn write(&self, out: &mut String, depth: usize) {
        let pad = |out: &mut String, d: usize| out.push_str(&"  ".repeat(d));
        match self {
            J::Null => out.push_str("null"),
            J::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            J::Int(n) => out.push_str(&n.to_string()),
            J::Str(v) => quote(v, out),
            J::Arr(items) if items.is_empty() => out.push_str("[]"),
            J::Arr(items) => {
                out.push_str("[\n");
                for (i, item) in items.iter().enumerate() {
                    pad(out, depth + 1);
                    item.write(out, depth + 1);
                    if i + 1 < items.len() {
                        out.push(',');
                    }
                    out.push('\n');
                }
                pad(out, depth);
                out.push(']');
            }
            J::Obj(pairs) if pairs.is_empty() => out.push_str("{}"),
            J::Obj(pairs) => {
                out.push_str("{\n");
                for (i, (k, v)) in pairs.iter().enumerate() {
                    pad(out, depth + 1);
                    quote(k, out);
                    out.push_str(": ");
                    v.write(out, depth + 1);
                    if i + 1 < pairs.len() {
                        out.push(',');
                    }
                    out.push('\n');
                }
                pad(out, depth);
                out.push('}');
            }
        }
    }

    /// The document pretty, ended by one LF.
    pub fn pretty(&self) -> String {
        let mut out = String::new();
        self.write(&mut out, 0);
        out.push('\n');
        out
    }

    /// A `serde_json` value, its objects' keys in the map's order.
    fn from_value(v: &serde_json::Value) -> J {
        match v {
            serde_json::Value::Null => J::Null,
            serde_json::Value::Bool(b) => J::Bool(*b),
            serde_json::Value::Number(n) => J::Int(n.as_i64().unwrap_or_default()),
            serde_json::Value::String(t) => s(t),
            serde_json::Value::Array(a) => J::Arr(a.iter().map(J::from_value).collect()),
            serde_json::Value::Object(m) => J::Obj(m.iter().map(|(k, v)| (k.clone(), J::from_value(v))).collect()),
        }
    }
}

// ---- the fixed texts of the contract ----

const NOTE_AUTHORIZATION: &str = "requires is checked against this exposure's role source at the serve site, before a request is queued: a boundary check, never a proof over the program's internal call paths";
const NOTE_LIFECYCLE: &str = "a request accepted runs to completion: a timeout or a lost connection does not mean it did not run, and ending the wait never rolls it back";
const NOTE_DISCOVERY: &str = "this document lists what this caller may use under this exposure's role source; the server still authorizes every request";
const NOTE_INVENTORY: &str = "every exposure with its listener, its sources and its receivers: a deployment inventory, never an authorization statement for any caller";
const HUB_AUTHORIZATION: &str = "requires is checked against this hub's role source when a subscription is admitted, and again at the credential's expiry and at every role-source revision: a boundary check, never a proof over the program's internal call paths";
const HUB_LIFECYCLE: &str = "a subscription invalidated by expiry or revocation receives one unauthorized frame and loses what it had buffered; stop() sends closed, and a reconnect replays only what a stream's replay says";
const HUB_DISCOVERY: &str = "this document lists the streams this caller may subscribe to under this hub's role source; the hub still authorizes every subscription";
const REFUSALS: [&str; 6] = ["malformed", "digest_mismatch", "unauthenticated", "unauthorized", "full", "shutting_down"];

/// A stream row's loss statement: the publish contract's delivery and
/// what its `on_full` sheds.
fn loss(on_full: &str) -> String {
    let sheds = match on_full {
        "drop_new" => "drop_new sheds the frame being published",
        _ => "drop_old sheds the oldest undelivered frame",
    };
    format!(
        "at most once per admitted subscriber, in publish order; {sheds}, and seq counts every event offered to the \
         subscription, so a gap in seq is the frames shed; a subscription invalidated by expiry or revocation loses \
         what it had buffered"
    )
}

/// The outcome encoding a transport states (spec/api.md § Outcomes).
fn outcomes(transport: &str) -> Result<J, String> {
    match transport {
        "http" => Ok(o(vec![
            ("transport", s("http")),
            ("result", o(vec![("status", J::Int(200)), ("body", s("response"))])),
            ("handler_error", o(vec![("status", J::Int(422)), ("body", s("error"))])),
            (
                "refusal",
                o(vec![
                    ("body", s("refusal")),
                    (
                        "status",
                        o(vec![
                            ("malformed", J::Int(400)),
                            ("digest_mismatch", J::Int(409)),
                            ("unauthenticated", J::Int(401)),
                            ("unauthorized", J::Int(403)),
                            ("full", J::Int(429)),
                            ("shutting_down", J::Int(503)),
                        ]),
                    ),
                ]),
            ),
            ("server_error", o(vec![("status", J::Int(500)), ("body", s("refusal"))])),
            ("transport_failure", s("the transport's own: the connection ends without a response")),
        ])),
        "unix" => Ok(o(vec![
            ("transport", s("unix")),
            ("result", o(vec![("ok", J::Bool(true)), ("field", s("value"))])),
            ("handler_error", o(vec![("ok", J::Bool(false)), ("field", s("error"))])),
            ("refusal", o(vec![("ok", J::Bool(false)), ("field", s("refusal")), ("kinds", strs(REFUSALS))])),
            ("server_error", o(vec![("ok", J::Bool(false)), ("field", s("refusal")), ("kind", s("server"))])),
            ("transport_failure", s("eof")),
        ])),
        "ws" => {
            let frame = |t: &str, fields: &[&str]| o(vec![("type", s(t)), ("fields", strs(fields.iter().copied()))]);
            Ok(o(vec![
                ("transport", s("ws")),
                ("subscribe", frame("subscribe", &["topic"])),
                ("subscribed", frame("subscribed", &["topic"])),
                (
                    "refusal",
                    o(vec![
                        ("type", s("refusal")),
                        ("fields", strs(["topic", "refusal"])),
                        ("kinds", strs(["malformed", "unauthenticated", "unauthorized", "shutting_down"])),
                    ]),
                ),
                ("event", frame("event", &["topic", "seq", "payload"])),
                (
                    "unauthorized",
                    o(vec![
                        ("type", s("unauthorized")),
                        ("fields", strs(["topic", "reason"])),
                        ("reasons", strs(["expired", "revoked"])),
                    ]),
                ),
                (
                    "closed",
                    o(vec![("type", s("closed")), ("fields", strs(["reason"])), ("reasons", strs(["shutting_down"]))]),
                ),
                ("transport_failure", s("the connection closes without a closed frame")),
            ]))
        }
        other => Err(format!(
            "the `{other}` transport has no outcome encoding in v1 (spec/api.md § Outcomes: http, unix and a hub's ws)"
        )),
    }
}

// ---- schemas ----

/// The schemas a document collects, by type name.
struct Book<'s, 'a> {
    schemas: &'s Schemas<'a>,
    types: BTreeMap<String, TypeSchema>,
}

impl<'s, 'a> Book<'s, 'a> {
    fn new(schemas: &'s Schemas<'a>) -> Self {
        Book { schemas, types: BTreeMap::new() }
    }

    fn type_ref(&mut self, te: &hale_syntax::ast::TypeExpr) -> FieldSchema {
        self.schemas.type_ref(te, &mut self.types)
    }

    /// The collected schemas as a document's `schemas` object, each `$ref`
    /// under `base`.
    fn render(&self, base: &str) -> J {
        J::Obj(self.types.iter().map(|(k, t)| (k.clone(), type_schema(t, base))).collect())
    }
}

fn field_schema(f: &FieldSchema, base: &str) -> J {
    match f {
        FieldSchema::Scalar { json, hale_type, unit } => {
            let mut pairs = vec![("type".to_string(), s(json))];
            if let Some(t) = hale_type {
                pairs.push(("x-hale-type".to_string(), s(t)));
            }
            if let Some(u) = unit {
                pairs.push(("x-hale-unit".to_string(), s(u)));
            }
            J::Obj(pairs)
        }
        FieldSchema::Ref(name) => o(vec![("$ref", s(&format!("{base}{name}")))]),
        FieldSchema::Unformed => J::Obj(Vec::new()),
    }
}

fn type_schema(t: &TypeSchema, base: &str) -> J {
    o(vec![
        ("type", s("object")),
        ("properties", J::Obj(t.properties.iter().map(|(k, f)| (k.clone(), field_schema(f, base))).collect())),
        ("required", strs(t.required.iter().map(String::as_str))),
    ])
}

const DOC_REFS: &str = "#/schemas/";

/// A member as a description and the inventory list it.
fn member(row: &Row, book: &mut Book<'_, '_>, base: &str) -> J {
    let Handled::Fn(h) = &row.handler else {
        return o(vec![("name", s(&row.member))]);
    };
    let mut ty = |t: Option<&hale_types::surfaces::RowTy>| match t {
        Some(t) => field_schema(&book.type_ref(&t.te), base),
        None => J::Null,
    };
    let request = ty(h.request.as_ref());
    let response = ty(h.response.as_ref());
    let error = if h.server_error { s("ClosureViolation") } else { ty(h.error.as_ref()) };
    o(vec![
        ("name", s(&row.member)),
        ("request", request),
        ("response", response),
        ("error", error),
        ("requires", strs(row.requires.iter().map(|(r, _)| r.as_str()))),
    ])
}

fn listener(transport: &str, address: Option<&str>) -> J {
    o(vec![("transport", s(transport)), ("address", s(address.unwrap_or("")))])
}

/// A source as the inventory names it; the Unix transport's principal is
/// its peer's kernel credentials.
fn source(src: Option<&Source>, kernel: bool) -> J {
    match src {
        Some(src) => {
            let mut pairs = vec![("source", s(&src.source))];
            if let Some(t) = &src.ty {
                pairs.push(("type", s(t)));
            }
            o(pairs)
        }
        None if kernel => o(vec![("source", s("kernel"))]),
        None => o(vec![("source", s("none"))]),
    }
}

fn stream(st: &Stream, book: &mut Book<'_, '_>, base: &str) -> J {
    let on_full = st.on_full.as_deref().unwrap_or("drop_old");
    o(vec![
        ("topic", s(&st.topic)),
        ("subject", s(&st.subject)),
        ("direction", s(st.direction)),
        ("payload", st.payload.as_ref().map_or(J::Null, |p| field_schema(&book.type_ref(&p.te), base))),
        ("codec", s(&st.codec)),
        ("bound", J::Int(st.bound.unwrap_or(0) as i64)),
        ("on_full", s(on_full)),
        ("loss", s(&loss(on_full))),
        ("replay", J::Bool(st.replay)),
        ("requires", strs(st.requires.iter().map(|(r, _)| r.as_str()))),
    ])
}

fn exposure_id(rows: &SurfaceRows, serve: &Serve) -> String {
    let surface = serve.surface.as_deref().unwrap_or("");
    let digest = rows.surface(surface).map_or(0, |s| s.digest);
    format!("{surface}@{}/{}", digest_text(digest), serve.name.as_deref().unwrap_or(""))
}

fn hub_id(hub: &Hub) -> String {
    format!("hub@{}/{}", digest_text(hub.digest()), hub.transport.name.as_deref().unwrap_or(""))
}

fn serves_by_name(rows: &SurfaceRows) -> Vec<&Serve> {
    let mut v: Vec<&Serve> = rows.serves.iter().collect();
    v.sort_by(|a, b| a.name.as_deref().unwrap_or("").as_bytes().cmp(b.name.as_deref().unwrap_or("").as_bytes()));
    v
}

fn hubs_by_name(rows: &SurfaceRows) -> Vec<&Hub> {
    let mut v: Vec<&Hub> = rows.hubs.iter().collect();
    v.sort_by(|a, b| {
        a.transport.name.as_deref().unwrap_or("").as_bytes().cmp(b.transport.name.as_deref().unwrap_or("").as_bytes())
    });
    v
}

fn serve_exposure(rows: &SurfaceRows, serve: &Serve) -> J {
    let t = serve.transport.as_ref();
    let kind = t.map_or("", |t| t.kind.as_str());
    let surface = serve.surface.as_deref().unwrap_or("");
    o(vec![
        ("exposure", s(&exposure_id(rows, serve))),
        ("name", s(serve.name.as_deref().unwrap_or(""))),
        ("surface", s(surface)),
        ("digest", s(&digest_text(rows.surface(surface).map_or(0, |s| s.digest)))),
        ("listener", listener(kind, t.and_then(|t| t.address.as_deref()))),
        ("codec", s(t.map_or("json", |t| t.codec.as_str()))),
        ("principals", source(t.and_then(|t| t.principals.as_ref()), kind == "unix")),
        ("roles", source(t.and_then(|t| t.roles.as_ref()), false)),
        (
            "receivers",
            J::Arr(
                serve
                    .receivers
                    .iter()
                    .map(|r| {
                        o(vec![
                            ("type", s(&r.ty)),
                            ("instance", s(&r.instance)),
                            ("binding", s(if r.explicit { "explicit" } else { "inferred" })),
                            ("pool", s(r.pool.as_deref().unwrap_or("?"))),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}

/// The program-wide inventory (spec/api.md § The description): every
/// surface with its digest and all its rows, every exposure with its
/// listener, sources and receivers, every hub with its stream rows, the
/// schemas of every type they name.
pub fn inventory(rows: &SurfaceRows, schemas: &Schemas<'_>) -> J {
    let mut book = Book::new(schemas);
    let surfaces = rows
        .surfaces
        .iter()
        .map(|sf| {
            let members: Vec<J> = rows.rows_of(&sf.name).map(|r| member(r, &mut book, DOC_REFS)).collect();
            o(vec![("name", s(&sf.name)), ("digest", s(&digest_text(sf.digest))), ("members", J::Arr(members))])
        })
        .collect();
    let exposures = serves_by_name(rows).into_iter().map(|sv| serve_exposure(rows, sv)).collect();
    let hubs = hubs_by_name(rows)
        .into_iter()
        .map(|h| {
            let t = &h.transport;
            let streams: Vec<J> = h.streams.iter().map(|st| stream(st, &mut book, DOC_REFS)).collect();
            o(vec![
                ("exposure", s(&hub_id(h))),
                ("name", s(t.name.as_deref().unwrap_or(""))),
                ("digest", s(&digest_text(h.digest()))),
                ("instance", s(&h.instance)),
                ("listener", listener(&t.kind, t.address.as_deref())),
                ("codec", s(&t.codec)),
                ("principals", source(t.principals.as_ref(), false)),
                ("roles", source(t.roles.as_ref(), false)),
                ("streams", J::Arr(streams)),
            ])
        })
        .collect();
    o(vec![
        ("inventory", J::Int(1)),
        ("app", s(rows.app.as_deref().unwrap_or(""))),
        ("surfaces", J::Arr(surfaces)),
        ("exposures", J::Arr(exposures)),
        ("hubs", J::Arr(hubs)),
        ("schemas", book.render(DOC_REFS)),
        (
            "notes",
            o(vec![
                ("inventory", s(NOTE_INVENTORY)),
                ("authorization", s(NOTE_AUTHORIZATION)),
                ("lifecycle", s(NOTE_LIFECYCLE)),
            ]),
        ),
    ])
}

/// The principal `--caller` names: a JSON object as the exposure would
/// establish it (`{"mode": "unix", "name": "uid:1000", "uid": 1000, …}`),
/// or a bare name, whose mode is the transport's (`unix` over the Unix
/// socket, `bearer` otherwise).
fn principal(caller: &str, transport: &str) -> Result<J, String> {
    if caller.trim_start().starts_with('{') {
        let v: serde_json::Value =
            serde_json::from_str(caller).map_err(|e| format!("--caller is not a JSON object: {e}"))?;
        let serde_json::Value::Object(m) = &v else { return Err("--caller is not a JSON object".to_string()) };
        let order = ["mode", "name", "uid", "gid", "pid"];
        if let Some(k) = m.keys().find(|k| !order.contains(&k.as_str())) {
            return Err(format!("--caller: a principal has `mode`, `name`, `uid`, `gid` and `pid`, not `{k}`"));
        }
        for k in ["mode", "name"] {
            if !m.contains_key(k) {
                return Err(format!("--caller: a principal names its `{k}`"));
            }
        }
        return Ok(J::Obj(
            order.iter().filter_map(|k| m.get(*k).map(|v| (k.to_string(), J::from_value(v)))).collect(),
        ));
    }
    let mode = if transport == "unix" { "unix" } else { "bearer" };
    Ok(o(vec![("mode", s(mode)), ("name", s(caller))]))
}

fn holds_all(holds: &BTreeSet<String>, requires: &[(String, hale_syntax::Span)]) -> bool {
    requires.iter().all(|(r, _)| holds.contains(r))
}

/// One exposure's description for one caller (spec/api.md § The
/// description): a serve site's surface over its transport, or a hub's
/// stream rows, filtered by the roles the caller holds under that
/// exposure's role source.
pub fn description(
    rows: &SurfaceRows,
    schemas: &Schemas<'_>,
    exposure: &str,
    caller: &str,
    holds: &BTreeSet<String>,
) -> Result<J, String> {
    let mut book = Book::new(schemas);
    if let Some(serve) = rows.serves.iter().find(|sv| sv.name.as_deref() == Some(exposure)) {
        let t = serve.transport.as_ref().ok_or_else(|| format!("exposure `{exposure}` names no transport instance"))?;
        let surface = serve.surface.as_deref().unwrap_or("");
        let digest = rows.surface(surface).map_or(0, |s| s.digest);
        // The hubs at this exposure's listener.
        let hubs: Vec<&Hub> = rows
            .hubs
            .iter()
            .filter(|h| h.transport.address.is_some() && h.transport.address == t.address)
            .collect();
        let required: BTreeSet<&str> = rows
            .rows_of(surface)
            .flat_map(|r| r.requires.iter())
            .chain(hubs.iter().flat_map(|h| h.streams.iter()).flat_map(|st| st.requires.iter()))
            .map(|(r, _)| r.as_str())
            .collect();
        let members: Vec<J> = rows
            .rows_of(surface)
            .filter(|r| holds_all(holds, &r.requires))
            .map(|r| member(r, &mut book, DOC_REFS))
            .collect();
        let streams: Vec<J> = hubs
            .iter()
            .flat_map(|h| h.streams.iter())
            .filter(|st| holds_all(holds, &st.requires))
            .map(|st| stream(st, &mut book, DOC_REFS))
            .collect();
        return Ok(o(vec![
            ("description", J::Int(1)),
            ("exposure", s(&exposure_id(rows, serve))),
            ("name", s(exposure)),
            ("surface", s(surface)),
            ("digest", s(&digest_text(digest))),
            ("listener", listener(&t.kind, t.address.as_deref())),
            ("codec", s(&t.codec)),
            (
                "caller",
                o(vec![
                    ("principal", principal(caller, &t.kind)?),
                    ("roles", strs(holds.iter().map(String::as_str).filter(|r| required.contains(r)))),
                ]),
            ),
            ("members", J::Arr(members)),
            ("streams", J::Arr(streams)),
            ("outcomes", outcomes(&t.kind)?),
            ("schemas", book.render(DOC_REFS)),
            (
                "notes",
                o(vec![
                    ("authorization", s(NOTE_AUTHORIZATION)),
                    ("lifecycle", s(NOTE_LIFECYCLE)),
                    ("discovery", s(NOTE_DISCOVERY)),
                ]),
            ),
        ]));
    }
    if let Some(hub) = rows.hubs.iter().find(|h| h.transport.name.as_deref() == Some(exposure)) {
        let t = &hub.transport;
        let required: BTreeSet<&str> =
            hub.streams.iter().flat_map(|st| st.requires.iter()).map(|(r, _)| r.as_str()).collect();
        let streams: Vec<J> = hub
            .streams
            .iter()
            .filter(|st| holds_all(holds, &st.requires))
            .map(|st| stream(st, &mut book, DOC_REFS))
            .collect();
        return Ok(o(vec![
            ("description", J::Int(1)),
            ("exposure", s(&hub_id(hub))),
            ("name", s(exposure)),
            ("surface", J::Null),
            ("digest", s(&digest_text(hub.digest()))),
            ("listener", listener(&t.kind, t.address.as_deref())),
            ("codec", s(&t.codec)),
            (
                "caller",
                o(vec![
                    ("principal", principal(caller, &t.kind)?),
                    ("roles", strs(holds.iter().map(String::as_str).filter(|r| required.contains(r)))),
                ]),
            ),
            ("members", J::Arr(Vec::new())),
            ("streams", J::Arr(streams)),
            ("outcomes", outcomes("ws")?),
            ("schemas", book.render(DOC_REFS)),
            (
                "notes",
                o(vec![
                    ("authorization", s(HUB_AUTHORIZATION)),
                    ("lifecycle", s(HUB_LIFECYCLE)),
                    ("discovery", s(HUB_DISCOVERY)),
                ]),
            ),
        ]));
    }
    let mut names: Vec<&str> = rows
        .serves
        .iter()
        .filter_map(|sv| sv.name.as_deref())
        .chain(rows.hubs.iter().filter_map(|h| h.transport.name.as_deref()))
        .collect();
    names.sort_unstable();
    Err(format!(
        "the program serves no exposure named `{exposure}`; its exposures: {}",
        if names.is_empty() { "none".to_string() } else { names.join(", ") }
    ))
}

// ---- the projections of one surface (GH #1107's forms, over the rows) ----

fn surface_members<'r>(rows: &'r SurfaceRows, surface: &'r str) -> Result<(u64, Vec<&'r Row>), String> {
    let sf = rows.surface(surface).ok_or_else(|| {
        let names: Vec<&str> = rows.surfaces.iter().map(|s| s.name.as_str()).collect();
        format!(
            "the program declares no surface `{surface}`; its surfaces: {}",
            if names.is_empty() { "none".to_string() } else { names.join(", ") }
        )
    })?;
    Ok((sf.digest, rows.rows_of(surface).collect()))
}

fn requires_text(row: &Row) -> String {
    let roles: Vec<&str> = row.requires.iter().map(|(r, _)| r.as_str()).collect();
    if roles.is_empty() {
        "Requires no role".to_string()
    } else {
        format!("Requires {} under the exposure's role source", roles.join(", "))
    }
}

/// The OpenAPI 3.1 form of a surface: `POST /call/<member>` per row, the
/// body its request, `200` its response, `422` its handler error, `500`
/// the server error, the refusals by their statuses; the digest in
/// `info` and as the `Hale-Surface-Digest` header a request may carry.
pub fn openapi(rows: &SurfaceRows, schemas: &Schemas<'_>, surface: &str) -> Result<J, String> {
    const REFS: &str = "#/components/schemas/";
    let (digest, members) = surface_members(rows, surface)?;
    let mut book = Book::new(schemas);
    let content = |schema: J| ("content".to_string(), o(vec![("application/json", o(vec![("schema", schema)]))]));
    let refusal = |what: &str| {
        J::Obj(vec![("description".to_string(), s(what)), content(o(vec![("$ref", s(&format!("{REFS}Refusal")))]))])
    };
    let mut paths = Vec::new();
    for row in members {
        let Handled::Fn(h) = &row.handler else { continue };
        let mut op = vec![
            ("operationId".to_string(), s(&row.member)),
            ("summary".to_string(), s(&format!("rpc {}", row.member))),
            ("description".to_string(), s(&format!("{}; the server still authorizes every request.", requires_text(row)))),
            (
                "parameters".to_string(),
                J::Arr(vec![o(vec![
                    ("name", s("Hale-Surface-Digest")),
                    ("in", s("header")),
                    ("required", J::Bool(false)),
                    ("schema", o(vec![("type", s("string")), ("const", s(&digest_text(digest)))])),
                ])]),
            ),
        ];
        if let Some(req) = &h.request {
            let schema = field_schema(&book.type_ref(&req.te), REFS);
            op.push(("requestBody".to_string(), J::Obj(vec![("required".to_string(), J::Bool(true)), content(schema)])));
        }
        let mut responses = Vec::new();
        let mut ok = vec![("description".to_string(), s("result: the handler returned"))];
        if let Some(t) = &h.response {
            ok.push(content(field_schema(&book.type_ref(&t.te), REFS)));
        }
        let ok = J::Obj(ok);
        responses.push(("200".to_string(), ok));
        for (status, what) in [
            ("400", "refusal: malformed"),
            ("401", "refusal: unauthenticated"),
            ("403", "refusal: unauthorized"),
            ("409", "refusal: digest_mismatch"),
        ] {
            responses.push((status.to_string(), refusal(what)));
        }
        if let (Some(e), false) = (&h.error, h.server_error) {
            let schema = field_schema(&book.type_ref(&e.te), REFS);
            responses.push((
                "422".to_string(),
                J::Obj(vec![("description".to_string(), s(&format!("handler error: {}", e.display))), content(schema)]),
            ));
        }
        responses.push(("429".to_string(), refusal("refusal: full")));
        if h.server_error {
            responses.push(("500".to_string(), refusal("server error: the handler violated (ClosureViolation)")));
        }
        responses.push(("503".to_string(), refusal("refusal: shutting_down")));
        op.push(("responses".to_string(), J::Obj(responses)));
        op.push(("security".to_string(), J::Arr(vec![o(vec![("bearer", J::Arr(Vec::new()))])])));
        op.push(("x-hale-requires".to_string(), strs(row.requires.iter().map(|(r, _)| r.as_str()))));
        paths.push((format!("/call/{}", row.member), o(vec![("post", J::Obj(op))])));
    }
    let mut component_schemas = match book.render(REFS) {
        J::Obj(p) => p,
        _ => Vec::new(),
    };
    // The body of a refusal and of the server error, as the HTTP
    // transport sends it: `{"refusal": {"kind": …, …}}`.
    component_schemas.push((
        "Refusal".to_string(),
        o(vec![
            ("type", s("object")),
            (
                "properties",
                o(vec![(
                    "refusal",
                    o(vec![
                        ("type", s("object")),
                        (
                            "properties",
                            o(vec![
                                ("kind", o(vec![("type", s("string"))])),
                                ("reason", o(vec![("type", s("string"))])),
                                ("served", o(vec![("type", s("string"))])),
                                ("requires", o(vec![("type", s("array")), ("items", o(vec![("type", s("string"))]))])),
                            ]),
                        ),
                        ("required", strs(["kind"])),
                    ]),
                )]),
            ),
            ("required", strs(["refusal"])),
        ]),
    ));
    Ok(o(vec![
        ("openapi", s("3.1.0")),
        (
            "info",
            o(vec![
                ("title", s(&format!("{surface} surface"))),
                ("version", s(&digest_text(digest))),
                (
                    "description",
                    s(&format!(
                        "The surface {surface} over HTTP (spec/api.md § Outcomes): POST /call/<member>, the body by \
                         the JSON codec. {NOTE_AUTHORIZATION}; {NOTE_LIFECYCLE}."
                    )),
                ),
                ("x-hale-surface", s(surface)),
                ("x-hale-digest", s(&digest_text(digest))),
            ]),
        ),
        ("paths", J::Obj(paths)),
        (
            "components",
            o(vec![
                ("schemas", J::Obj(component_schemas)),
                ("securitySchemes", o(vec![("bearer", o(vec![("type", s("http")), ("scheme", s("bearer"))]))])),
            ]),
        ),
    ]))
}

/// The JSON Schema form of a surface: every type its rows name under
/// `$defs`, and each member's request, response and error by reference.
pub fn json_schema(rows: &SurfaceRows, schemas: &Schemas<'_>, surface: &str) -> Result<J, String> {
    const REFS: &str = "#/$defs/";
    let (digest, members) = surface_members(rows, surface)?;
    let mut book = Book::new(schemas);
    let members: Vec<(String, J)> = members.into_iter().map(|r| (r.member.clone(), member(r, &mut book, REFS))).collect();
    Ok(o(vec![
        ("$schema", s("https://json-schema.org/draft/2020-12/schema")),
        ("$id", s(&format!("urn:hale:surface:{surface}@{}", digest_text(digest)))),
        ("title", s(&format!("{surface} surface"))),
        ("x-hale-surface", s(surface)),
        ("x-hale-digest", s(&digest_text(digest))),
        ("x-hale-members", J::Obj(members)),
        ("$defs", book.render(REFS)),
    ]))
}

/// A tool name: the member with `::` spelled `__`.
fn tool_name(member: &str) -> String {
    member.replace("::", "__")
}

/// The MCP form of a surface: every row a tool, its input schema the
/// request's, self-contained (the types it reaches under `$defs`).
pub fn mcp(rows: &SurfaceRows, schemas: &Schemas<'_>, surface: &str) -> Result<J, String> {
    const REFS: &str = "#/$defs/";
    let (digest, members) = surface_members(rows, surface)?;
    let mut tools = Vec::new();
    for row in members {
        let Handled::Fn(h) = &row.handler else { continue };
        let mut book = Book::new(schemas);
        let input = match &h.request {
            Some(req) => match book.type_ref(&req.te) {
                FieldSchema::Ref(name) => {
                    let mut root = match book.types.get(&name).map(|t| type_schema(t, REFS)) {
                        Some(J::Obj(p)) => p,
                        _ => Vec::new(),
                    };
                    let defs: Vec<(String, J)> = book
                        .types
                        .iter()
                        .filter(|(k, _)| **k != name)
                        .map(|(k, t)| (k.clone(), type_schema(t, REFS)))
                        .collect();
                    if !defs.is_empty() {
                        root.push(("$defs".to_string(), J::Obj(defs)));
                    }
                    J::Obj(root)
                }
                other => field_schema(&other, REFS),
            },
            None => o(vec![("type", s("object")), ("properties", J::Obj(Vec::new()))]),
        };
        let returns = h.response.as_ref().map_or("nothing".to_string(), |t| t.display.clone());
        let fails = match (&h.error, h.server_error) {
            (_, true) => "; a violation is the server error".to_string(),
            (Some(e), false) => format!("; its failure is the handler error {}", e.display),
            (None, false) => String::new(),
        };
        tools.push(o(vec![
            ("name", s(&tool_name(&row.member))),
            (
                "description",
                s(&format!(
                    "rpc {} of {surface}: returns {returns}{fails}. {}; the server still authorizes every request.",
                    row.member,
                    requires_text(row)
                )),
            ),
            ("inputSchema", input),
            ("x-hale-requires", strs(row.requires.iter().map(|(r, _)| r.as_str()))),
        ]));
    }
    Ok(o(vec![
        ("x-hale-surface", s(surface)),
        ("x-hale-digest", s(&digest_text(digest))),
        ("tools", J::Arr(tools)),
        ("notes", o(vec![("authorization", s(NOTE_AUTHORIZATION)), ("lifecycle", s(NOTE_LIFECYCLE))])),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The printer writes what `serde_json::to_string_pretty` writes for
    /// the same value, keys in the order given.
    #[test]
    fn the_printer_is_serde_jsons_pretty_form() {
        let doc = o(vec![
            ("a", J::Int(1)),
            ("b", strs(["x", "y\"z"])),
            ("c", J::Obj(Vec::new())),
            ("d", J::Arr(Vec::new())),
            ("e", o(vec![("f", J::Null), ("g", J::Bool(false))])),
            ("h", s("tab\there\u{1}")),
        ]);
        let expect = serde_json::json!({
            "a": 1, "b": ["x", "y\"z"], "c": {}, "d": [], "e": { "f": null, "g": false }, "h": "tab\there\u{1}"
        });
        assert_eq!(doc.pretty(), serde_json::to_string_pretty(&expect).unwrap() + "\n");
    }
}

//! `hale api client --lang ts`: one TypeScript module generated from a
//! surface's rows (GH #1417, R8a; spec/api.md § The clients).
//!
//! Types from the shapes, one async function per member returning the tagged
//! union of the outcomes (a connection that never answered is thrown), the
//! digest sent on every call, and a typed async-iterable subscription per
//! stream row of the program's hubs. It depends on `fetch` and `WebSocket`
//! and on nothing else; the wire is the preamble beside this file.

use std::collections::BTreeSet;

use hale_types::surface_doc::{ClientError, ClientMember, ClientModel, ClientStream};
use hale_types::surfaces::{FieldSchema, TypeSchema};

use super::{camel, json_quote, type_ident, unformed};

const PREAMBLE: &str = include_str!("ts_preamble.ts");

fn lower_camel(name: &str) -> String {
    let c = camel(name);
    let mut cs = c.chars();
    match cs.next() {
        Some(f) => f.to_ascii_lowercase().to_string() + cs.as_str(),
        None => String::new(),
    }
}

fn is_ts_ident(key: &str) -> bool {
    let mut cs = key.chars();
    matches!(cs.next(), Some(c) if c.is_ascii_alphabetic() || c == '_' || c == '$')
        && cs.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

/// A property key as TypeScript writes it.
fn prop(key: &str) -> String {
    if is_ts_ident(key) {
        key.to_string()
    } else {
        json_quote(key)
    }
}

/// A property read: `v.key` or `v["key"]`.
fn access(value: &str, key: &str) -> String {
    if is_ts_ident(key) {
        format!("{value}.{key}")
    } else {
        format!("{value}[{}]", json_quote(key))
    }
}

fn ts_type(f: &FieldSchema, what: &str) -> Result<String, String> {
    match f {
        FieldSchema::Scalar { json, .. } => Ok(match *json {
            "integer" | "number" => "number",
            "boolean" => "boolean",
            _ => "string",
        }
        .to_string()),
        FieldSchema::Ref(name) => Ok(type_ident(name)),
        FieldSchema::Unformed => Err(unformed(what)),
    }
}

/// The wire form of `value`, a TypeScript expression of the schema's type.
fn encode(f: &FieldSchema, value: &str, what: &str) -> Result<String, String> {
    match f {
        FieldSchema::Scalar { .. } => Ok(value.to_string()),
        FieldSchema::Ref(name) => Ok(format!("encode_{}({value})", type_ident(name))),
        FieldSchema::Unformed => Err(unformed(what)),
    }
}

/// The typed value of `json`, an `unknown` expression.
fn decode(f: &FieldSchema, json: &str, what: &str) -> Result<String, String> {
    match f {
        FieldSchema::Scalar { json: kind, .. } => Ok(match *kind {
            "integer" | "number" => format!("Number({json} ?? 0)"),
            "boolean" => format!("({json} ?? false) === true"),
            _ => format!("String({json} ?? \"\")"),
        }),
        FieldSchema::Ref(name) => Ok(format!("decode_{}({json})", type_ident(name))),
        FieldSchema::Unformed => Err(unformed(what)),
    }
}

fn struct_decl(name: &str, t: &TypeSchema) -> Result<String, String> {
    let ident = type_ident(name);
    let mut out = format!("export interface {ident} {{\n");
    for (key, f) in &t.properties {
        out.push_str(&format!("  {}: {};\n", prop(key), ts_type(f, &format!("{name}.{key}"))?));
    }
    out.push_str("}\n\n");
    out.push_str(&format!("export function encode_{ident}(v: {ident}): unknown {{\n  return {{\n"));
    for (key, f) in &t.properties {
        out.push_str(&format!("    {}: {},\n", prop(key), encode(f, &access("v", key), &format!("{name}.{key}"))?));
    }
    out.push_str("  };\n}\n\n");
    out.push_str(&format!("export function decode_{ident}(json: unknown): {ident} {{\n"));
    out.push_str("  const o = (json ?? {}) as __Record<string, unknown>;\n  return {\n");
    for (key, f) in &t.properties {
        out.push_str(&format!("    {}: {},\n", prop(key), decode(f, &format!("o[{}]", json_quote(key)), &format!("{name}.{key}"))?));
    }
    out.push_str("  };\n}\n\n");
    Ok(out)
}

fn member_fn(m: &ClientMember) -> Result<String, String> {
    let fn_name = lower_camel(&m.name);
    let outcome = format!("{}Outcome", camel(&m.name));
    let what = &m.name;
    let value = match &m.response {
        Some(r) => ts_type(r, &format!("{what} response"))?,
        None => "void".to_string(),
    };
    let error = match &m.error {
        ClientError::Type(e) => ts_type(e, &format!("{what} error"))?,
        _ => "never".to_string(),
    };
    let requires = if m.requires.is_empty() { String::new() } else { format!(", requires {}", m.requires.join(", ")) };
    let (param, payload) = match &m.request {
        Some(req) => (
            format!(", request: {}", ts_type(req, &format!("{what} request"))?),
            format!("JSON.stringify({})", encode(req, "request", &format!("{what} request"))?),
        ),
        None => (String::new(), "\"{}\"".to_string()),
    };
    let dec_value = match &m.response {
        Some(r) => format!("(json) => {}", decode(r, "json", &format!("{what} response"))?),
        None => "() => undefined".to_string(),
    };
    let dec_error = match &m.error {
        ClientError::Type(e) => format!("(json) => {}", decode(e, "json", &format!("{what} error"))?),
        _ => "() => { throw new TransportError(\"the member has no handler error, and one came\"); }".to_string(),
    };
    Ok(format!(
        "/** {name}{requires} */\nexport type {outcome} = Outcome<{value}, {error}>;\n\n\
         export function {fn_name}(opts: ClientOptions{param}): __Promise<{outcome}> {{\n  \
         return apiCall(opts, {member}, {payload}, {dec_value}, {dec_error});\n}}\n\n",
        name = m.name,
        member = json_quote(&m.name),
    ))
}

fn stream_decl(s: &ClientStream) -> Result<String, String> {
    let base = camel(&s.topic);
    let what = format!("stream {}", s.topic);
    let Some(p) = &s.payload else { return Err(format!("the {what} has no payload type, which a stream row always has")) };
    let payload = ts_type(p, &format!("{what} payload"))?;
    let decoder = decode(p, "json", &format!("{what} payload"))?;
    let roles = if s.requires.is_empty() { "none".to_string() } else { s.requires.join(", ") };
    Ok(format!(
        "/** The stream {topic} of the hub {hub} (stream digest {digest}), requires {roles}: `subscribed`, then\n \
         * each `event` with its `seq` (a gap is the frames shed) until `expired`, `revoked` or `closed`; a\n \
         * connection that ends with no closed frame throws a TransportError from the iterator. */\n\
         export type {base}Event = StreamEvent<{payload}>;\n\n\
         export function subscribe{base}(opts: StreamOptions): Subscription<{payload}> {{\n  \
         return apiSubscribe(opts, {topic_lit}, (json) => {decoder});\n}}\n\n",
        topic = s.topic,
        hub = s.hub,
        digest = s.hub_digest,
        topic_lit = json_quote(&s.topic),
    ))
}

pub fn generate(model: &ClientModel) -> Result<String, String> {
    let mut seen = BTreeSet::new();
    let mut claim = |name: String| -> Result<(), String> {
        if seen.insert(name.clone()) {
            Ok(())
        } else {
            Err(format!("two things of the client are named `{name}`: rename one of the surface's types or members"))
        }
    };
    for name in [
        "SURFACE", "SURFACE_DIGEST", "ClientOptions", "StreamOptions", "Refusal", "TransportError", "Outcome", "describe",
        "StreamEvent", "Subscription", "apiCall", "apiRefusal", "apiSubscribe", "__Record", "__Promise", "__Array", "__AsyncIterable",
        "__AsyncGenerator", "__Response", "__MessageEvent",
    ] {
        claim(name.to_string())?;
    }
    let mut out = format!(
        "// Generated by `hale api client --surface {surface} --lang ts` from the rows of the surface\n\
         // {surface} ({digest}). Do not edit: regenerate it, and `hale api client --check` refuses a\n\
         // copy whose digest is no longer the surface's. See spec/api.md § The clients.\n\
         //\n\
         // One async function per member, `<locus><Fn>(opts, request)`, answering the tagged union of\n\
         // `result`, `handler_error`, `refusal` and `server_error`; a connection that never answered is\n\
         // thrown as a TransportError. A quantity, an identity or a range crosses as the number it counts\n\
         // (an Int past 2^53 loses precision as a JSON number does in JavaScript).\n\n\
         export const SURFACE = {surface_lit};\n\
         export const SURFACE_DIGEST = {digest_lit};\n\n",
        surface = model.surface,
        digest = model.digest,
        surface_lit = json_quote(&model.surface),
        digest_lit = json_quote(&model.digest),
    );
    out.push_str(PREAMBLE);
    out.push('\n');
    for (name, t) in &model.types {
        claim(type_ident(name))?;
        claim(format!("encode_{}", type_ident(name)))?;
        claim(format!("decode_{}", type_ident(name)))?;
        out.push_str(&struct_decl(name, t)?);
    }
    for m in &model.members {
        claim(lower_camel(&m.name))?;
        claim(format!("{}Outcome", camel(&m.name)))?;
        out.push_str(&member_fn(m)?);
    }
    let mut topics = BTreeSet::new();
    for s in &model.streams {
        if !topics.insert(s.topic.clone()) {
            return Err(format!("the topic `{}` is a stream of two hubs; a client names a stream by its topic", s.topic));
        }
        claim(format!("{}Event", camel(&s.topic)))?;
        claim(format!("subscribe{}", camel(&s.topic)))?;
        out.push_str(&stream_decl(s)?);
    }
    Ok(out.trim_end().to_string() + "\n")
}

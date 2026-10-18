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

use super::{json_quote, type_ident, unformed};

const PREAMBLE: &str = include_str!("ts_preamble.ts");

/// The helpers that lift a wire map's keys out and put them back, carried
/// only by a client whose surface names a map.
const MAP_HELPERS: &str = include_str!("ts_map.ts");

pub(super) fn lower_camel(name: &str) -> String {
    hale_types::api_names::lower_camel(name)
}

fn is_ts_ident(key: &str) -> bool {
    let mut cs = key.chars();
    matches!(cs.next(), Some(c) if c.is_ascii_alphabetic() || c == '_' || c == '$')
        && cs.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

/// A property key as TypeScript writes it.
pub(super) fn prop(key: &str) -> String {
    if is_ts_ident(key) {
        key.to_string()
    } else {
        json_quote(key)
    }
}

/// A property read: `v.key` or `v["key"]`.
pub(super) fn access(value: &str, key: &str) -> String {
    if is_ts_ident(key) {
        format!("{value}.{key}")
    } else {
        format!("{value}[{}]", json_quote(key))
    }
}

fn ts_type(f: &FieldSchema, what: &str) -> Result<String, String> {
    type_in(f, what, false)
}

/// A schema's type as TypeScript writes it, or (`jsdoc`) as a JSDoc type of
/// the JavaScript client, which has no `__Array` alias to name.
pub(super) fn type_in(f: &FieldSchema, what: &str, jsdoc: bool) -> Result<String, String> {
    match f {
        FieldSchema::Scalar { json, .. } => Ok(match *json {
            "integer" | "number" => "number",
            "boolean" => "boolean",
            _ => "string",
        }
        .to_string()),
        FieldSchema::Ref(name) => Ok(type_ident(name)),
        // an instant stays its RFC 3339 text: a `Date` holds milliseconds,
        // and the wire's nanoseconds would not survive it
        FieldSchema::Time => Ok("string".to_string()),
        FieldSchema::List { items, .. } if jsdoc => Ok(format!("globalThis.Array<{}>", type_in(items, what, true)?)),
        FieldSchema::List { items, .. } => Ok(format!("__Array<{}>", ts_type(items, what)?)),
        FieldSchema::Enum { name, .. } => Ok(type_ident(name)),
        // a map is its records, as the server's declaration holds them (the
        // object keyed by their key field is the wire's)
        FieldSchema::Map { name, .. } if jsdoc => Ok(format!("globalThis.Array<{}>", type_ident(name))),
        FieldSchema::Map { name, .. } => Ok(format!("__Array<{}>", type_ident(name))),
        // an optional field is its value's type, the property optional
        FieldSchema::Nullable { inner, .. } => type_in(inner, what, jsdoc),
        FieldSchema::Presence { .. } => Err(unformed(what)),
        FieldSchema::Unformed => Err(unformed(what)),
    }
}

/// The wire form of `value`, a TypeScript (and JavaScript) expression of the schema's type.
pub(super) fn encode(f: &FieldSchema, value: &str, what: &str) -> Result<String, String> {
    match f {
        FieldSchema::Scalar { .. } | FieldSchema::Time => Ok(value.to_string()),
        FieldSchema::Ref(name) | FieldSchema::Enum { name, .. } => Ok(format!("encode_{}({value})", type_ident(name))),
        FieldSchema::Map { name, lifted, .. } => {
            Ok(format!("apiMapEncode({value}, {}, encode_{})", json_quote(lifted), type_ident(name)))
        }
        // a list of scalars is itself; a list of records each record's form
        FieldSchema::List { items, .. } => match items.as_ref() {
            FieldSchema::Scalar { .. } | FieldSchema::Time => Ok(value.to_string()),
            i => Ok(format!("({value}).map((e) => {})", encode(i, "e", what)?)),
        },
        // left out of the object when absent (`JSON.stringify` drops an
        // `undefined` property), so the server reads it as absent
        FieldSchema::Nullable { inner, .. } => Ok(format!("({value} == null ? undefined : {})", encode(inner, value, what)?)),
        FieldSchema::Presence { .. } => Err(unformed(what)),
        FieldSchema::Unformed => Err(unformed(what)),
    }
}

/// The typed value of `json`, an `unknown` expression.
fn decode(f: &FieldSchema, json: &str, what: &str) -> Result<String, String> {
    decode_in(f, json, what, false)
}

/// [`decode`] in TypeScript, or (`js`) in JavaScript, whose cast is a JSDoc one.
pub(super) fn decode_in(f: &FieldSchema, json: &str, what: &str, js: bool) -> Result<String, String> {
    match f {
        FieldSchema::Scalar { json: kind, .. } => Ok(match *kind {
            "integer" | "number" => format!("Number({json} ?? 0)"),
            "boolean" => format!("({json} ?? false) === true"),
            _ => format!("String({json} ?? \"\")"),
        }),
        FieldSchema::Ref(name) | FieldSchema::Enum { name, .. } => Ok(format!("decode_{}({json})", type_ident(name))),
        FieldSchema::Map { name, lifted, .. } => {
            Ok(format!("apiMapDecode({json}, {}, decode_{})", json_quote(lifted), type_ident(name)))
        }
        FieldSchema::Time => Ok(format!("String({json} ?? \"\")")),
        FieldSchema::List { items, .. } if js => Ok(format!(
            "(globalThis.Array.isArray({json}) ? /** @type {{unknown[]}} */ ({json}).map((e) => {}) : [])",
            decode_in(items, "e", what, true)?
        )),
        FieldSchema::List { items, .. } => Ok(format!(
            "(globalThis.Array.isArray({json}) ? ({json} as unknown[]).map((e) => {}) : [])",
            decode(items, "e", what)?
        )),
        FieldSchema::Nullable { inner, .. } => Ok(format!("({json} == null ? undefined : {})", decode_in(inner, json, what, js)?)),
        FieldSchema::Presence { .. } => Err(unformed(what)),
        FieldSchema::Unformed => Err(unformed(what)),
    }
}

fn struct_decl(name: &str, t: &TypeSchema) -> Result<String, String> {
    let ident = type_ident(name);
    let mut out = format!("export interface {ident} {{\n");
    // an optional field is an optional property; its flag is the property's presence
    for (key, f) in t.properties.iter().filter(|(_, f)| !matches!(f, FieldSchema::Presence { .. })) {
        let q = if matches!(f, FieldSchema::Nullable { .. }) { "?" } else { "" };
        out.push_str(&format!("  {}{q}: {};\n", prop(key), ts_type(f, &format!("{name}.{key}"))?));
    }
    out.push_str("}\n\n");
    out.push_str(&format!("export function encode_{ident}(v: {ident}): unknown {{\n  return {{\n"));
    for (key, f) in t.properties.iter().filter(|(_, f)| !matches!(f, FieldSchema::Presence { .. })) {
        out.push_str(&format!("    {}: {},\n", prop(key), encode(f, &access("v", key), &format!("{name}.{key}"))?));
    }
    out.push_str("  };\n}\n\n");
    out.push_str(&format!("export function decode_{ident}(json: unknown): {ident} {{\n"));
    out.push_str("  const o = (json ?? {}) as __Record<string, unknown>;\n  return {\n");
    for (key, f) in t.properties.iter().filter(|(_, f)| !matches!(f, FieldSchema::Presence { .. })) {
        out.push_str(&format!("    {}: {},\n", prop(key), decode(f, &format!("o[{}]", json_quote(key)), &format!("{name}.{key}"))?));
    }
    out.push_str("  };\n}\n\n");
    Ok(out)
}

/// An enum's type, and its codec's two functions: a payload-free variant is
/// its name, one with values an object keyed by the variant (one value
/// itself, several a tuple), as the wire is (spec/api.md § Codecs). In
/// JavaScript (`js`) the type is a JSDoc typedef and the casts JSDoc ones.
pub(super) fn enum_decl(name: &str, variants: &super::ClientEnum, js: bool) -> Result<String, String> {
    let ident = type_ident(name);
    let mut arms = Vec::new();
    for (v, values) in variants {
        let what = format!("{name}::{v}");
        arms.push(match values.as_slice() {
            [] => json_quote(v),
            [one] => format!("{{ {}: {} }}", prop(v), type_in(one, &what, js)?),
            many => format!(
                "{{ {}: [{}] }}",
                prop(v),
                many.iter().map(|f| type_in(f, &what, js)).collect::<Result<Vec<_>, _>>()?.join(", ")
            ),
        });
    }
    let union = arms.join(" | ");
    let cast = |ty: &str, e: &str| {
        let simple = e.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        match (js, simple) {
            (true, _) => format!("/** @type {{{ty}}} */ ({e})"),
            (false, true) => format!("({e} as {ty})"),
            (false, false) => format!("(({e}) as {ty})"),
        }
    };
    let mut out = if js {
        format!("/** @typedef {{{union}}} {ident} */

/**
 * @param {{{ident}}} v
 * @returns {{unknown}}
 */
export function encode_{ident}(v) {{
")
    } else {
        format!("export type {ident} = {union};

export function encode_{ident}(v: {ident}): unknown {{
")
    };
    let payloads = variants.iter().any(|(_, values)| !values.is_empty());
    if payloads {
        out.push_str("  if (typeof v === \"string\") return v;\n");
    }
    for (v, values) in variants.iter().filter(|(_, values)| !values.is_empty()) {
        let what = format!("{name}::{v}");
        let at = access("v", v);
        let value = match values.as_slice() {
            [one] => encode(one, &at, &what)?,
            many => format!(
                "[{}]",
                many.iter().enumerate().map(|(i, f)| encode(f, &format!("{at}[{i}]"), &what)).collect::<Result<Vec<_>, _>>()?.join(", ")
            ),
        };
        out.push_str(&format!("  if ({} in v) return {{ {}: {value} }};\n", json_quote(v), prop(v)));
    }
    out.push_str("  return v;\n}\n\n");
    out.push_str(&if js {
        format!("/**\n * @param {{unknown}} json\n * @returns {{{ident}}}\n */\nexport function decode_{ident}(json) {{\n")
    } else {
        format!("export function decode_{ident}(json: unknown): {ident} {{\n")
    });
    if payloads {
        out.push_str(&format!("  if (typeof json === \"string\") return {};\n", cast(&ident, "json")));
        out.push_str(&format!(
            "  const o = {};\n",
            cast(if js { "globalThis.Record<string, unknown>" } else { "__Record<string, unknown>" }, "json ?? {}")
        ));
    }
    for (v, values) in variants.iter().filter(|(_, values)| !values.is_empty()) {
        let what = format!("{name}::{v}");
        let at = format!("o[{}]", json_quote(v));
        let value = match values.as_slice() {
            [one] => decode_in(one, &at, &what, js)?,
            many => {
                let a = format!("(globalThis.Array.isArray({at}) ? {} : [])", cast("unknown[]", &at));
                format!(
                    "[{}]",
                    many.iter()
                        .enumerate()
                        .map(|(i, f)| decode_in(f, &format!("{a}[{i}]"), &what, js))
                        .collect::<Result<Vec<_>, _>>()?
                        .join(", ")
                )
            }
        };
        out.push_str(&format!("  if ({} in o) return {{ {}: {value} }};\n", json_quote(v), prop(v)));
    }
    out.push_str(&format!("  return {};\n}}\n\n", cast(&ident, "json")));
    Ok(out)
}

fn member_fn(m: &ClientMember) -> Result<String, String> {
    let fn_name = lower_camel(&m.name);
    let outcome = hale_types::api_names::outcome(&m.name);
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
    let event = hale_types::api_names::event(&s.topic);
    let subscribe = hale_types::api_names::subscribe_fn(&s.topic);
    let what = format!("stream {}", s.topic);
    let Some(p) = &s.payload else { return Err(format!("the {what} has no payload type, which a stream row always has")) };
    let payload = ts_type(p, &format!("{what} payload"))?;
    let decoder = decode(p, "json", &format!("{what} payload"))?;
    let roles = if s.requires.is_empty() { "none".to_string() } else { s.requires.join(", ") };
    Ok(format!(
        "/** The stream {topic} of the hub {hub} (stream digest {digest}), requires {roles}: `subscribed`, then\n \
         * each `event` with its `seq` (a gap is the frames shed) until `expired`, `revoked` or `closed`; a\n \
         * connection that ends with no closed frame throws a TransportError from the iterator. */\n\
         export type {event} = StreamEvent<{payload}>;\n\n\
         export function {subscribe}(opts: StreamOptions): Subscription<{payload}> {{\n  \
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
    for name in hale_types::api_names::TS_CLIENT_NAMES {
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
    if super::model_has_map(model) {
        out.push_str(MAP_HELPERS);
        out.push('\n');
    }
    for (name, variants) in &super::model_enums(model) {
        claim(type_ident(name))?;
        claim(format!("encode_{}", type_ident(name)))?;
        claim(format!("decode_{}", type_ident(name)))?;
        out.push_str(&enum_decl(name, variants, false)?);
    }
    for (name, t) in &model.types {
        claim(type_ident(name))?;
        claim(format!("encode_{}", type_ident(name)))?;
        claim(format!("decode_{}", type_ident(name)))?;
        out.push_str(&struct_decl(name, t)?);
    }
    for m in &model.members {
        claim(lower_camel(&m.name))?;
        claim(hale_types::api_names::outcome(&m.name))?;
        out.push_str(&member_fn(m)?);
    }
    let mut topics = BTreeSet::new();
    for s in &model.streams {
        if !topics.insert(s.topic.clone()) {
            return Err(format!("the topic `{}` is a stream of two hubs; a client names a stream by its topic", s.topic));
        }
        claim(hale_types::api_names::event(&s.topic))?;
        claim(hale_types::api_names::subscribe_fn(&s.topic))?;
        out.push_str(&stream_decl(s)?);
    }
    Ok(out.trim_end().to_string() + "\n")
}

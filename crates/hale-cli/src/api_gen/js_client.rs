//! `hale api client --lang js`: one JavaScript (ES module) client generated
//! from a surface's rows (spec/api.md § The clients).
//!
//! The TypeScript client's surface, to the name: the same exports, the same
//! outcomes, the same wire, and its types as JSDoc, so a page loads it with no
//! compile step and an editor or `tsc --checkJs` still holds a caller to the
//! rows. The wire is `js_preamble.js`, the TypeScript preamble with its types
//! moved into comments; the expressions are the TypeScript generator's.

use std::collections::BTreeSet;

use hale_types::surface_doc::{ClientError, ClientMember, ClientModel, ClientStream};
use hale_types::surfaces::{FieldSchema, TypeSchema};

use super::ts_client::{access, decode_in, encode, enum_decl, lower_camel, prop, type_in};
use super::{camel, json_quote, type_ident};

const PREAMBLE: &str = include_str!("js_preamble.js");

/// The helpers that lift a wire map's keys out and put them back, carried
/// only by a client whose surface names a map.
const MAP_HELPERS: &str = include_str!("js_map.js");

fn struct_decl(name: &str, t: &TypeSchema) -> Result<String, String> {
    let ident = type_ident(name);
    let mut out = String::from("/**\n * @typedef {{\n");
    // an optional field is an optional property; its flag is the property's presence
    for (key, f) in t.properties.iter().filter(|(_, f)| !matches!(f, FieldSchema::Presence { .. })) {
        let q = if matches!(f, FieldSchema::Nullable { .. }) { "?" } else { "" };
        out.push_str(&format!(" *   {}{q}: {},\n", prop(key), type_in(f, &format!("{name}.{key}"), true)?));
    }
    out.push_str(&format!(" * }}}} {ident}\n */\n\n"));
    out.push_str(&format!("/**\n * @param {{{ident}}} v\n * @returns {{unknown}}\n */\nexport function encode_{ident}(v) {{\n  return {{\n"));
    for (key, f) in t.properties.iter().filter(|(_, f)| !matches!(f, FieldSchema::Presence { .. })) {
        out.push_str(&format!("    {}: {},\n", prop(key), encode(f, &access("v", key), &format!("{name}.{key}"))?));
    }
    out.push_str("  };\n}\n\n");
    out.push_str(&format!("/**\n * @param {{unknown}} json\n * @returns {{{ident}}}\n */\nexport function decode_{ident}(json) {{\n"));
    out.push_str("  const o = /** @type {globalThis.Record<string, unknown>} */ (json ?? {});\n  return {\n");
    for (key, f) in t.properties.iter().filter(|(_, f)| !matches!(f, FieldSchema::Presence { .. })) {
        out.push_str(&format!(
            "    {}: {},\n",
            prop(key),
            decode_in(f, &format!("o[{}]", json_quote(key)), &format!("{name}.{key}"), true)?
        ));
    }
    out.push_str("  };\n}\n\n");
    Ok(out)
}

fn member_fn(m: &ClientMember) -> Result<String, String> {
    let fn_name = lower_camel(&m.name);
    let outcome = format!("{}Outcome", camel(&m.name));
    let what = &m.name;
    let value = match &m.response {
        Some(r) => type_in(r, &format!("{what} response"), true)?,
        None => "void".to_string(),
    };
    let error = match &m.error {
        ClientError::Type(e) => type_in(e, &format!("{what} error"), true)?,
        _ => "never".to_string(),
    };
    let requires = if m.requires.is_empty() { String::new() } else { format!(", requires {}", m.requires.join(", ")) };
    let (param_doc, param, payload) = match &m.request {
        Some(req) => (
            format!(" * @param {{{}}} request\n", type_in(req, &format!("{what} request"), true)?),
            ", request",
            format!("JSON.stringify({})", encode(req, "request", &format!("{what} request"))?),
        ),
        None => (String::new(), "", "\"{}\"".to_string()),
    };
    let dec_value = match &m.response {
        Some(r) => format!("(json) => {}", decode_in(r, "json", &format!("{what} response"), true)?),
        None => "() => undefined".to_string(),
    };
    let dec_error = match &m.error {
        ClientError::Type(e) => format!("(json) => {}", decode_in(e, "json", &format!("{what} error"), true)?),
        _ => "() => { throw new TransportError(\"the member has no handler error, and one came\"); }".to_string(),
    };
    Ok(format!(
        "/** @typedef {{Outcome<{value}, {error}>}} {outcome} */\n\n\
         /**\n * {name}{requires}\n * @param {{ClientOptions}} opts\n{param_doc} * @returns {{globalThis.Promise<{outcome}>}}\n */\n\
         export function {fn_name}(opts{param}) {{\n  \
         return apiCall(opts, {member}, {payload}, {dec_value}, {dec_error});\n}}\n\n",
        name = m.name,
        member = json_quote(&m.name),
    ))
}

fn stream_decl(s: &ClientStream) -> Result<String, String> {
    let base = camel(&s.topic);
    let what = format!("stream {}", s.topic);
    let Some(p) = &s.payload else { return Err(format!("the {what} has no payload type, which a stream row always has")) };
    let payload = type_in(p, &format!("{what} payload"), true)?;
    let decoder = decode_in(p, "json", &format!("{what} payload"), true)?;
    let roles = if s.requires.is_empty() { "none".to_string() } else { s.requires.join(", ") };
    Ok(format!(
        "/** @typedef {{StreamEvent<{payload}>}} {base}Event */\n\n\
         /**\n * The stream {topic} of the hub {hub} (stream digest {digest}), requires {roles}: `subscribed`, then\n \
         * each `event` with its `seq` (a gap is the frames shed) until `expired`, `revoked` or `closed`; a\n \
         * connection that ends with no closed frame throws a TransportError from the iterator.\n \
         * @param {{StreamOptions}} opts\n * @returns {{Subscription<{payload}>}}\n */\n\
         export function subscribe{base}(opts) {{\n  \
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
        "StreamEvent", "Subscription", "apiCall", "apiRefusal", "apiSubscribe", "apiMapEncode", "apiMapDecode",
    ] {
        claim(name.to_string())?;
    }
    let mut out = format!(
        "// Generated by `hale api client --surface {surface} --lang js` from the rows of the surface\n\
         // {surface} ({digest}). Do not edit: regenerate it, and `hale api client --check` refuses a\n\
         // copy whose digest is no longer the surface's. See spec/api.md § The clients.\n\
         //\n\
         // An ES module with no dependency beyond `fetch` and `WebSocket`, typed in JSDoc. One async\n\
         // function per member, `<locus><Fn>(opts, request)`, answering the tagged union of `result`,\n\
         // `handler_error`, `refusal` and `server_error`; a connection that never answered is thrown as a\n\
         // TransportError. A quantity, an identity or a range crosses as the number it counts (an Int\n\
         // past 2^53 loses precision as a JSON number does in JavaScript), a Time as its RFC 3339 text.\n\
         // @ts-check\n\n\
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
        out.push_str(&enum_decl(name, variants, true)?);
    }
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

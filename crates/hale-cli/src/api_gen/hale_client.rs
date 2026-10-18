//! `hale api client --lang hale`: a Hale module generated from a surface's
//! rows (GH #1417, R8a; spec/api.md § The clients).
//!
//! One fn per member, typed by the row's request and response shapes, whose
//! outcome is an enum of the five; one subscription locus per stream row of
//! the program's hubs. The wire is the preamble beside this file, the same
//! for every surface; everything else is a function of the rows.

use std::collections::BTreeSet;

use hale_types::surface_doc::{ClientError, ClientMember, ClientModel, ClientStream};
use hale_types::surfaces::{FieldSchema, TypeSchema};

use super::{camel, field_ident, hale_lit, json_quote, snake, type_ident, unformed};

const PREAMBLE: &str = include_str!("hale_preamble.hl");

/// The instant a client reads where a reply holds none it can parse.
const EPOCH: &str = "`1970-01-01T00:00:00Z`";

/// A Hale type as the client declares it: what the server's declaration
/// says, so the compiler's api codec (`T::api_encode` / `T::api_decode`,
/// spec/api.md § Codecs) reads and writes it exactly as the server does.
fn hale_type(f: &FieldSchema, what: &str) -> Result<String, String> {
    match f {
        FieldSchema::Scalar { json, .. } => Ok(match *json {
            "integer" => "Int",
            "number" => "Float",
            "boolean" => "Bool",
            _ => "String",
        }
        .to_string()),
        FieldSchema::Ref(name) => Ok(type_ident(name)),
        FieldSchema::Time => Ok("Time".to_string()),
        FieldSchema::List { items, max, exact: false } => Ok(format!("bounded[{}; {max}]", hale_type(items, what)?)),
        FieldSchema::List { items, max, exact: true } => Ok(format!("[{}; {max}]", hale_type(items, what)?)),
        FieldSchema::Enum { name, .. } => Ok(type_ident(name)),
        // a map is its list (`struct_decl` writes the `indexed_by` clause)
        FieldSchema::Map { name, max, .. } => Ok(format!("bounded[{}; {max}]", type_ident(name))),
        // an optional field is its value's type, its flag a `Bool` beside it
        FieldSchema::Nullable { inner, .. } => hale_type(inner, what),
        FieldSchema::Presence { .. } => Ok("Bool".to_string()),
        FieldSchema::Unformed => Err(unformed(what)),
    }
}

/// The JSON of `value`, a Hale expression of the schema's type.
fn encode(f: &FieldSchema, value: &str, what: &str) -> Result<String, String> {
    match f {
        FieldSchema::Scalar { json, .. } => Ok(match *json {
            "integer" | "number" => format!("to_string({value})"),
            "boolean" => format!("(if {value} {{ \"true\" }} else {{ \"false\" }})"),
            _ => format!("api_str({value})"),
        }),
        FieldSchema::Ref(name) | FieldSchema::Enum { name, .. } => Ok(format!("{}::api_encode({value})", type_ident(name))),
        FieldSchema::Time => Ok(format!("api_str(std::time::iso8601({value}))")),
        // a list, a map or an optional field is a record's, never a whole payload
        FieldSchema::List { .. }
        | FieldSchema::Map { .. }
        | FieldSchema::Nullable { .. }
        | FieldSchema::Presence { .. }
        | FieldSchema::Unformed => Err(unformed(what)),
    }
}

/// The statements that bind `var` to the value of the JSON document `json`
/// (a Hale expression of text), running `fail` (statements that end in a
/// `return`) when a record's codec refuses it.
fn decode_into(f: &FieldSchema, json: &str, var: &str, fail: &str, what: &str) -> Result<String, String> {
    Ok(match f {
        FieldSchema::Scalar { json: kind, .. } => format!(
            "let {var} = {};\n",
            match *kind {
                "integer" => format!("api_scalar_int({json})"),
                "number" => format!("api_scalar_float({json})"),
                "boolean" => format!("api_scalar_bool({json})"),
                _ => format!("api_scalar_str({json})"),
            }
        ),
        FieldSchema::Ref(name) | FieldSchema::Enum { name, .. } => {
            format!("let {var} = {}::api_decode({json}) or {{ {fail} }};\n", type_ident(name))
        }
        FieldSchema::Time => format!("let {var} = std::time::parse_time(api_scalar_str({json})) or {EPOCH};\n"),
        FieldSchema::List { .. }
        | FieldSchema::Map { .. }
        | FieldSchema::Nullable { .. }
        | FieldSchema::Presence { .. }
        | FieldSchema::Unformed => return Err(unformed(what)),
    })
}

/// The field identifier of each JSON key of a struct, in key order. `field_ident` alone is not
/// injective (`a-b` and `a_b` are both `a_b`), so the first key to want an identifier keeps it and a
/// later one gets `_2`, `_3`, ...; an optional field's flag is `has_` and its field's identifier,
/// which the codec pairs by name. Every identifier comes out of one allocator: an optional field and
/// its flag are placed first, as a pair (a field's identifier is bumped until its flag's is free too),
/// then the ordinary fields around them. A field whose identifier is not its key carries the key as
/// its `json:` tag.
fn field_idents(t: &TypeSchema) -> Vec<String> {
    let mut used = BTreeSet::new();
    let mut idents: Vec<Option<String>> = vec![None; t.properties.len()];
    // the flag of each optional field, by its declaration (its JSON key may be another)
    let partner = |j: usize| -> Option<usize> {
        let FieldSchema::Nullable { flag, .. } = &t.properties[j].1 else { return None };
        t.properties.iter().position(|(_, f)| matches!(f, FieldSchema::Presence { flag: name, .. } if name == flag))
    };
    for j in 0..t.properties.len() {
        let Some(i) = partner(j) else { continue };
        let natural = field_ident(&t.properties[j].0);
        let mut ident = natural.clone();
        let mut n = 2;
        while used.contains(&ident) || used.contains(&format!("has_{ident}")) {
            ident = format!("{natural}_{n}");
            n += 1;
        }
        used.insert(ident.clone());
        used.insert(format!("has_{ident}"));
        idents[i] = Some(format!("has_{ident}"));
        idents[j] = Some(ident);
    }
    for (k, (key, _)) in t.properties.iter().enumerate() {
        if idents[k].is_some() {
            continue;
        }
        let natural = field_ident(key);
        let mut ident = natural.clone();
        let mut n = 2;
        while !used.insert(ident.clone()) {
            ident = format!("{natural}_{n}");
            n += 1;
        }
        idents[k] = Some(ident);
    }
    idents.into_iter().map(|i| i.expect("every field named")).collect()
}

/// Whether a record of the model can be built from no fields, `T { }`:
/// every field of it is one a payload may leave out, and each such record
/// field's own type can.
fn buildable_empty(model: &ClientModel, name: &str, seen: &mut BTreeSet<String>) -> bool {
    let Some(t) = model.types.get(name) else { return false };
    if !seen.insert(name.to_string()) {
        return false;
    }
    t.required.is_empty()
        && t.properties.iter().all(|(_, f)| match f {
            FieldSchema::Ref(r) => buildable_empty(model, r, seen),
            FieldSchema::List { exact: true, .. } => false,
            _ => true,
        })
}

/// The default a field the server lets a payload leave out takes in the
/// client's declaration, so a value can be built naming only what it means
/// (`T { code: "x" }`): its type's zero, a record's empty literal where one
/// can be built; `None` where the field is required or has none.
fn default_of(model: &ClientModel, f: &FieldSchema) -> Option<String> {
    match f {
        FieldSchema::Scalar { json, .. } => Some(
            match *json {
                "integer" => "0",
                "number" => "0.0",
                "boolean" => "false",
                _ => "\"\"",
            }
            .to_string(),
        ),
        FieldSchema::Time => Some(EPOCH.to_string()),
        FieldSchema::Presence { .. } => Some("false".to_string()),
        FieldSchema::Ref(r) if buildable_empty(model, r, &mut BTreeSet::new()) => Some(format!("{} {{ }}", type_ident(r))),
        FieldSchema::Nullable { inner, .. } => default_of(model, inner),
        // an enum's zero is its first variant with its values' zeros, as the
        // codec's is (spec/api.md § Codecs)
        FieldSchema::Enum { name, variants } => {
            let (v, values) = variants.first()?;
            if values.is_empty() {
                return Some(format!("{}::{v}", type_ident(name)));
            }
            let zeros = values.iter().map(|f| default_of(model, f)).collect::<Option<Vec<_>>>()?;
            Some(format!("{}::{v}({})", type_ident(name), zeros.join(", ")))
        }
        _ => None,
    }
}

/// A record as the client declares it: its fields, each named for its key
/// (and tagged with the key where the name differs), so the compiler's api
/// codec carries it as the server's does, and a field a payload may leave
/// out given its zero.
fn struct_decl(model: &ClientModel, name: &str, t: &TypeSchema) -> Result<String, String> {
    let ident = type_ident(name);
    let idents = field_idents(t);
    let mut out = format!("type {ident} {{\n");
    for ((key, f), field) in t.properties.iter().zip(&idents) {
        let tag = if field != key && !matches!(f, FieldSchema::Presence { .. }) {
            format!(" `json:\"{}\"`", key.replace('\\', "\\\\").replace('"', "\\\""))
        } else {
            String::new()
        };
        let value = match f {
            FieldSchema::Nullable { inner, .. } => inner.as_ref(),
            f => f,
        };
        let default = if t.required.contains(key) || matches!(value, FieldSchema::List { .. } | FieldSchema::Map { .. }) {
            String::new()
        } else {
            default_of(model, f).map_or(String::new(), |d| format!(" = {d}"))
        };
        // a map keys its records by the client's name of the lifted field
        let keyed = match value {
            FieldSchema::Map { name: record, lifted, .. } => {
                let rt = model.types.get(record).ok_or_else(|| unformed(&format!("{name}.{key}")))?;
                let at = rt.properties.iter().position(|(k, _)| k == lifted).ok_or_else(|| unformed(&format!("{name}.{key}")))?;
                format!(" indexed_by {}", field_idents(rt)[at])
            }
            _ => String::new(),
        };
        out.push_str(&format!("    {field}: {}{keyed}{default}{tag};\n", hale_type(f, &format!("{name}.{key}"))?));
    }
    out.push_str("}\n\n");
    Ok(out)
}

/// An enum as the client declares it: the server's variants, each with its
/// values' types, so the api codec carries it as the server's does.
fn enum_decl(name: &str, variants: &super::ClientEnum) -> Result<String, String> {
    let vs = variants
        .iter()
        .map(|(v, values)| {
            if values.is_empty() {
                return Ok(v.clone());
            }
            let tys = values.iter().map(|f| hale_type(f, &format!("{name}::{v}"))).collect::<Result<Vec<_>, _>>()?;
            Ok(format!("{v}({})", tys.join(", ")))
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(format!("type {} = enum {{ {} }};\n\n", type_ident(name), vs.join(", ")))
}

fn member_fn(m: &ClientMember) -> Result<String, String> {
    let fn_name = snake(&m.name);
    let outcome = format!("{}Outcome", camel(&m.name));
    let what = &m.name;
    let mut variants = Vec::new();
    variants.push(match &m.response {
        Some(r) => format!("Result({})", hale_type(r, &format!("{what} response"))?),
        None => "Result".to_string(),
    });
    if let ClientError::Type(e) = &m.error {
        variants.push(format!("HandlerError({})", hale_type(e, &format!("{what} error"))?));
    }
    variants.push("Refusal(ApiRefusal)".to_string());
    variants.push("ServerError".to_string());
    variants.push("Lost(String)".to_string());
    let mut out = format!(
        "// {name}{requires}\ntype {outcome} = enum {{ {variants} }};\n\n",
        name = m.name,
        requires = if m.requires.is_empty() { String::new() } else { format!(", requires {}", m.requires.join(", ")) },
        variants = variants.join(", ")
    );
    let (params, payload) = match &m.request {
        Some(req) => (
            format!(", request: {}", hale_type(req, &format!("{what} request"))?),
            encode(req, "request", &format!("{what} request"))?,
        ),
        None => (String::new(), "\"{}\"".to_string()),
    };
    out.push_str(&format!("fn {fn_name}(endpoint: String, bearer: String{params}) -> {outcome} {{\n"));
    out.push_str(&format!("    let raw = api_call(endpoint, bearer, {}, {payload});\n", hale_lit(&m.name)));
    // a reply the codec refuses is a lost answer, named
    let lost = |part: &str| {
        format!("return {outcome}::Lost(\"the {part} did not decode: \" + err.kind + \": \" + err.field);")
    };
    match &m.response {
        Some(r) => {
            out.push_str("    if raw.outcome == 0 {\n");
            out.push_str(&format!("        {}", decode_into(r, "raw.body", "value", &lost("result"), &format!("{what} response"))?));
            out.push_str(&format!("        return {outcome}::Result(value);\n    }}\n"));
        }
        None => out.push_str(&format!("    if raw.outcome == 0 {{ return {outcome}::Result; }}\n")),
    }
    if let ClientError::Type(e) = &m.error {
        out.push_str("    if raw.outcome == 1 {\n");
        out.push_str(&format!("        {}", decode_into(e, "raw.body", "error", &lost("handler error"), &format!("{what} error"))?));
        out.push_str(&format!("        return {outcome}::HandlerError(error);\n    }}\n"));
    }
    out.push_str(&format!("    if raw.outcome == 2 {{ return {outcome}::Refusal(api_refusal(raw)); }}\n"));
    out.push_str(&format!("    if raw.outcome == 3 {{ return {outcome}::ServerError; }}\n"));
    if !matches!(m.error, ClientError::Type(_)) {
        out.push_str(&format!(
            "    if raw.outcome == 1 {{ return {outcome}::Lost(\"the member has no handler error, and one came: \" + raw.body); }}\n"
        ));
    }
    out.push_str(&format!("    return {outcome}::Lost(raw.why);\n}}\n\n"));
    Ok(out)
}

fn stream_decl(s: &ClientStream) -> Result<String, String> {
    let base = camel(&s.topic);
    let event = format!("{base}Event");
    let sub = format!("{base}Subscription");
    let what = format!("stream {}", s.topic);
    let payload = match &s.payload {
        Some(p) => hale_type(p, &format!("{what} payload"))?,
        None => return Err(format!("the {what} has no payload type, which a stream row always has")),
    };
    let decode = decode_into(
        s.payload.as_ref().unwrap_or(&FieldSchema::Unformed),
        "payload",
        "value",
        &format!(
            "self.ended = \"a payload did not decode: \" + err.kind + \": \" + err.field; return {event}::Lost(self.ended);"
        ),
        &format!("{what} payload"),
    )?;
    let roles = if s.requires.is_empty() { "none".to_string() } else { s.requires.join(", ") };
    Ok(format!(
        "// The stream {topic} of the hub {hub} (stream digest {digest}), requires {roles}.\n\
         // `start` upgrades and subscribes and answers `Subscribed` or `Refusal`; `next` then yields\n\
         // each `Event(seq, payload)` until `Expired`, `Revoked` or `Closed` ends it (the same again\n\
         // after that), or `Lost` when the connection ends with no closed frame; `next_for` answers\n\
         // `Idle` when nothing came within the wait.\n\
         type {event} = enum {{ Subscribed, Event(Int, {payload}), Refusal(ApiRefusal), Expired, Revoked, Closed, Idle, Lost(String) }};\n\n\
         locus {sub} {{\n\
         \x20   params {{\n\
         \x20       ws: ApiWs = ApiWs {{ }};\n\
         \x20       ended: String = \"\";\n\
         \x20   }}\n\
         \x20   fn start(endpoint: String, bearer: String) -> {event} {{\n\
         \x20       if !self.ws.connect(endpoint, bearer) {{\n\
         \x20           self.ended = \"lost\";\n\
         \x20           return {event}::Lost(self.ws.why);\n\
         \x20       }}\n\
         \x20       self.ws.send_text({subscribe});\n\
         \x20       return self.next_for(30000);\n\
         \x20   }}\n\
         \x20   @unbounded\n\
         \x20   fn next() -> {event} {{\n\
         \x20       let mut e = self.next_for(1000);\n\
         \x20       while self.waiting(e) {{\n\
         \x20           e = self.next_for(1000);\n\
         \x20       }}\n\
         \x20       return e;\n\
         \x20   }}\n\
         \x20   fn waiting(e: {event}) -> Bool {{\n\
         \x20       return match e {{\n\
         \x20           {event}::Idle -> true,\n\
         \x20           _ -> false,\n\
         \x20       }};\n\
         \x20   }}\n\
         \x20   fn next_for(wait_ms: Int) -> {event} {{\n\
         \x20       if self.ended == \"expired\" {{ return {event}::Expired; }}\n\
         \x20       if self.ended == \"revoked\" {{ return {event}::Revoked; }}\n\
         \x20       if self.ended == \"closed\" {{ return {event}::Closed; }}\n\
         \x20       if len(self.ended) > 0 {{ return {event}::Lost(self.ended); }}\n\
         \x20       let text = self.ws.read_text(wait_ms * 1000000);\n\
         \x20       if len(text) == 0 {{\n\
         \x20           if self.ws.open {{ return {event}::Idle; }}\n\
         \x20           self.ended = \"the hub ended the connection with no closed frame\";\n\
         \x20           return {event}::Lost(self.ended);\n\
         \x20       }}\n\
         \x20       let kind = std::json::find_string_field(text, \"type\");\n\
         \x20       if kind == \"subscribed\" {{ return {event}::Subscribed; }}\n\
         \x20       if kind == \"event\" {{\n\
         \x20           let payload = std::json::find_field_raw(text, \"payload\");\n\
         \x20           {decode}\
         \x20           return {event}::Event(std::json::find_int_field(text, \"seq\"), value);\n\
         \x20       }}\n\
         \x20       if kind == \"refusal\" {{\n\
         \x20           self.ended = \"the subscription was refused\";\n\
         \x20           return {event}::Refusal(api_refusal(api_refused(std::json::find_field_raw(text, \"refusal\"), 0)));\n\
         \x20       }}\n\
         \x20       if kind == \"unauthorized\" {{\n\
         \x20           if std::json::find_string_field(text, \"reason\") == \"expired\" {{\n\
         \x20               self.ended = \"expired\";\n\
         \x20               return {event}::Expired;\n\
         \x20           }}\n\
         \x20           self.ended = \"revoked\";\n\
         \x20           return {event}::Revoked;\n\
         \x20       }}\n\
         \x20       if kind == \"closed\" {{\n\
         \x20           self.ended = \"closed\";\n\
         \x20           return {event}::Closed;\n\
         \x20       }}\n\
         \x20       return {event}::Idle;\n\
         \x20   }}\n\
         }}\n\n",
        topic = s.topic,
        hub = s.hub,
        digest = s.hub_digest,
        subscribe = hale_lit(&format!("{{\"type\":\"subscribe\",\"topic\":{}}}", json_quote(&s.topic))),
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
    let mut out = String::new();
    out.push_str(&format!(
        "// Generated by `hale api client --surface {surface} --lang hale` from the rows of the surface\n\
         // {surface} ({digest}). Do not edit: regenerate it, and `hale api client --check` refuses a\n\
         // copy whose digest is no longer the surface's. See spec/api.md § The clients.\n\
         //\n\
         // One fn per member: `fn <locus>_<fn>(endpoint, bearer, request) -> <Member>Outcome`, the five\n\
         // outcomes of spec/api.md § Outcomes as one enum (`Result`, `HandlerError`, `Refusal`,\n\
         // `ServerError`, `Lost`), and one subscription locus per stream row of the program's hubs.\n\
         // A quantity, an identity or a range crosses as the integer it counts.\n\n\
         const SURFACE: String = {surface_lit};\n\
         const SURFACE_DIGEST: String = {digest_lit};\n\n",
        surface = model.surface,
        digest = model.digest,
        surface_lit = hale_lit(&model.surface),
        digest_lit = hale_lit(&model.digest),
    ));
    for name in ["SURFACE", "SURFACE_DIGEST"] {
        claim(name.to_string())?;
    }
    out.push_str(PREAMBLE);
    out.push('\n');
    for name in [
        "ApiRefusal", "ApiRaw", "ApiWs", "api_lost", "api_refusal", "api_str", "api_scalar_str", "api_scalar_int",
        "api_scalar_bool", "api_scalar_float", "api_roles", "api_refused", "api_http_send", "api_http",
        "api_unix_exchange", "api_unix", "api_call", "api_get", "api_describe", "api_lists", "API_WAIT_NS", "API_ID", "API_WS_KEY", "API_MASK",
    ] {
        claim(name.to_string())?;
    }
    for (name, variants) in &super::model_enums(model) {
        claim(type_ident(name))?;
        out.push_str(&enum_decl(name, variants)?);
    }
    for (name, t) in &model.types {
        claim(type_ident(name))?;
        out.push_str(&struct_decl(model, name, t)?);
    }
    for m in &model.members {
        claim(snake(&m.name))?;
        claim(format!("{}Outcome", camel(&m.name)))?;
        out.push_str(&member_fn(m)?);
    }
    let mut topics = BTreeSet::new();
    for s in &model.streams {
        if !topics.insert(s.topic.clone()) {
            return Err(format!("the topic `{}` is a stream of two hubs; a client names a stream by its topic", s.topic));
        }
        claim(format!("{}Event", camel(&s.topic)))?;
        claim(format!("{}Subscription", camel(&s.topic)))?;
        out.push_str(&stream_decl(s)?);
    }
    let formatted = hale_syntax::fmt::format_source(&out).map_err(|e| match e {
        hale_syntax::fmt::FmtError::Parse(diags) => format!(
            "the generated client does not parse (a generator bug): {}",
            diags.first().map_or(String::new(), |d| d.message.clone())
        ),
        hale_syntax::fmt::FmtError::Changed(_) => "the formatter altered the generated client (a generator bug)".to_string(),
    })?;
    Ok(formatted)
}

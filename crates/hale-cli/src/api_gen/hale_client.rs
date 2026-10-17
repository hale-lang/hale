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

use super::{camel, field_ident, hale_lit, json_quote, optional_unsupported, snake, type_ident, unformed};

const PREAMBLE: &str = include_str!("hale_preamble.hl");

/// The instant a client reads where a reply holds none it can parse.
const EPOCH: &str = "`1970-01-01T00:00:00Z`";

/// A Hale type as the client declares it.
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
        FieldSchema::Nullable { .. } | FieldSchema::Presence { .. } => Err(optional_unsupported(what)),
        FieldSchema::Unformed => Err(unformed(what)),
    }
}

/// The value a fixed array's slots hold before the payload's elements are
/// read into them: the zero of a scalar, a record read from `{}`.
fn zero(f: &FieldSchema, what: &str) -> Result<String, String> {
    match f {
        FieldSchema::Scalar { json, .. } => Ok(match *json {
            "integer" => "0",
            "number" => "0.0",
            "boolean" => "false",
            _ => "\"\"",
        }
        .to_string()),
        FieldSchema::Ref(name) => Ok(format!("api_dec_{}(\"{{}}\")", type_ident(name))),
        FieldSchema::Time => Ok(EPOCH.to_string()),
        _ => Err(unformed(what)),
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
        FieldSchema::Ref(name) => Ok(format!("api_enc_{}({value})", type_ident(name))),
        FieldSchema::Time => Ok(format!("api_str(std::time::iso8601({value}))")),
        // a list is written by its record's helper (`list_enc`), never inline
        FieldSchema::Nullable { .. } | FieldSchema::Presence { .. } => Err(optional_unsupported(what)),
        FieldSchema::List { .. } | FieldSchema::Unformed => Err(unformed(what)),
    }
}

/// The Hale value of the JSON document `json`, a Hale expression of text.
fn decode_whole(f: &FieldSchema, json: &str, what: &str) -> Result<String, String> {
    match f {
        FieldSchema::Scalar { json: kind, .. } => Ok(match *kind {
            "integer" => format!("api_scalar_int({json})"),
            "number" => format!("api_scalar_float({json})"),
            "boolean" => format!("api_scalar_bool({json})"),
            _ => format!("api_scalar_str({json})"),
        }),
        FieldSchema::Ref(name) => Ok(format!("api_dec_{}({json})", type_ident(name))),
        FieldSchema::Time => Ok(format!("(std::time::parse_time(api_scalar_str({json})) or {EPOCH})")),
        // a list is read by its record's decoder (`list_dec`), never whole
        FieldSchema::Nullable { .. } | FieldSchema::Presence { .. } => Err(optional_unsupported(what)),
        FieldSchema::List { .. } | FieldSchema::Unformed => Err(unformed(what)),
    }
}

/// The Hale value of the field `key` of the object `json`.
fn decode_field(f: &FieldSchema, json: &str, key: &str, what: &str) -> Result<String, String> {
    let q = hale_lit(key);
    match f {
        FieldSchema::Scalar { json: kind, .. } => Ok(match *kind {
            "integer" => format!("std::json::find_int_field({json}, {q})"),
            "number" => format!("api_scalar_float(std::json::find_field_raw({json}, {q}))"),
            "boolean" => format!("std::json::find_bool_field({json}, {q})"),
            _ => format!("std::json::find_string_field({json}, {q})"),
        }),
        FieldSchema::Ref(name) => Ok(format!("api_dec_{}(std::json::find_field_raw({json}, {q}))", type_ident(name))),
        FieldSchema::Time => Ok(format!("(std::time::parse_time(std::json::find_string_field({json}, {q})) or {EPOCH})")),
        FieldSchema::Nullable { .. } | FieldSchema::Presence { .. } => Err(optional_unsupported(what)),
        FieldSchema::List { .. } | FieldSchema::Unformed => Err(unformed(what)),
    }
}

/// The helper that writes the list field `field` of a `record` as a JSON array.
fn list_enc_name(record: &str, field: &str) -> String {
    format!("api_enc_{record}__{field}")
}

/// `api_enc_<T>__<field>(v: T) -> String`: the list `v.<field>` as a JSON array.
fn list_enc(record: &str, field: &str, items: &FieldSchema, what: &str) -> Result<String, String> {
    Ok(format!(
        "@bounded_by(v)\nfn {name}(v: {record}) -> String {{\n    let mut out = \"[\";\n    let mut n = 0;\n    for x in v.{field} {{\n        \
         if n > 0 {{ out = out + \",\"; }}\n        out = out + {elem};\n        n = n + 1;\n    }}\n    return out + \"]\";\n}}\n\n",
        name = list_enc_name(record, field),
        elem = encode(items, "x", what)?,
    ))
}

/// The statements of a record's decoder that read the list at the key `key` of
/// `json` into `target`: a bounded field by `push` (`target` the record's field),
/// a fixed array by its slots (`target` a local, `[zero; N]` until then), the
/// elements past what it holds left unread.
fn list_dec(target: &str, local: &str, key: &str, items: &FieldSchema, max: u64, exact: bool, what: &str) -> Result<String, String> {
    let text = format!("{local}_text");
    let it = format!("{local}_at");
    let elem = decode_whole(items, &format!("{text}[{it}.cur_start..{it}.cur_end]"), what)?;
    let mut out = format!("    let {text} = std::json::find_field_raw(json, {});\n", hale_lit(key));
    out.push_str(&format!("    let mut {it} = std::json::array_first_span({text});\n"));
    if exact {
        out.push_str(&format!("    let mut {local}_n = 0;\n"));
        out.push_str(&format!("    while !{it}.done && {local}_n < {max} {{\n"));
        out.push_str(&format!("        {target}[{local}_n] = {elem};\n"));
        out.push_str(&format!("        {local}_n = {local}_n + 1;\n"));
    } else {
        out.push_str(&format!("    while !{it}.done {{\n"));
        out.push_str(&format!("        push({target}, {elem}) or discard;\n"));
    }
    out.push_str(&format!("        {it} = std::json::array_next_span({it}, {text});\n    }}\n"));
    Ok(out)
}

/// The field identifier of each JSON key of a struct, in key order. `field_ident` alone is not
/// injective (`a-b` and `a_b` are both `a_b`), so the first key to want an identifier keeps it and a
/// later one gets `_2`, `_3`, ...; the declaration, the encoder and the decoder all read this one
/// mapping, and each wire key stays the one the row names.
fn field_idents(t: &TypeSchema) -> Vec<String> {
    let mut used = BTreeSet::new();
    t.properties
        .iter()
        .map(|(key, _)| {
            let natural = field_ident(key);
            let mut ident = natural.clone();
            let mut n = 2;
            while !used.insert(ident.clone()) {
                ident = format!("{natural}_{n}");
                n += 1;
            }
            ident
        })
        .collect()
}

fn struct_decl(name: &str, t: &TypeSchema) -> Result<String, String> {
    let ident = type_ident(name);
    let idents = field_idents(t);
    let mut out = format!("type {ident} {{\n");
    for ((key, f), field) in t.properties.iter().zip(&idents) {
        out.push_str(&format!("    {field}: {};\n", hale_type(f, &format!("{name}.{key}"))?));
    }
    out.push_str("}\n\n");
    if t.properties.iter().any(|(_, f)| matches!(f, FieldSchema::List { .. })) {
        return list_struct_fns(name, t, &ident, &idents, out);
    }
    // the encoder: one compact object, the fields in declaration order
    out.push_str(&format!("fn api_enc_{ident}(v: {ident}) -> String {{\n"));
    if t.properties.is_empty() {
        out.push_str("    return \"{}\";\n");
    } else {
        let mut parts = Vec::new();
        for (i, ((key, f), field)) in t.properties.iter().zip(&idents).enumerate() {
            let sep = if i == 0 { "{" } else { "," };
            let value = encode(f, &format!("v.{field}"), &format!("{name}.{key}"))?;
            parts.push(format!("{} + {value}", hale_lit(&format!("{sep}{}:", json_quote(key)))));
        }
        out.push_str(&format!("    return {} + \"}}\";\n", parts.join(" + ")));
    }
    out.push_str("}\n\n");
    // the decoder: every field read by its key, absent ones as the type's zero
    out.push_str(&format!("fn api_dec_{ident}(json: String) -> {ident} {{\n"));
    out.push_str(&format!("    return {ident} {{\n"));
    for ((key, f), field) in t.properties.iter().zip(&idents) {
        out.push_str(&format!("        {field}: {},\n", decode_field(f, "json", key, &format!("{name}.{key}"))?));
    }
    out.push_str("    };\n}\n\n");
    Ok(out)
}

/// The encoder and decoder of a record with a list field: as
/// `struct_decl`'s, with each list written by its helper and read by a loop.
fn list_struct_fns(name: &str, t: &TypeSchema, ident: &str, idents: &[String], mut out: String) -> Result<String, String> {
    let mut parts = Vec::new();
    for (i, ((key, f), field)) in t.properties.iter().zip(idents).enumerate() {
        let sep = if i == 0 { "{" } else { "," };
        let what = format!("{name}.{key}");
        let value = match f {
            FieldSchema::List { items, .. } => {
                out.push_str(&list_enc(ident, field, items, &what)?);
                format!("{}(v)", list_enc_name(ident, field))
            }
            f => encode(f, &format!("v.{field}"), &what)?,
        };
        parts.push(format!("{} + {value}", hale_lit(&format!("{sep}{}:", json_quote(key)))));
    }
    out.push_str(&format!("fn api_enc_{ident}(v: {ident}) -> String {{\n"));
    out.push_str(&format!("    return {} + \"}}\";\n}}\n\n", parts.join(" + ")));
    // the decoder: the fixed arrays read first, the record made, the bounded lists pushed into it
    out.push_str(&format!("@bounded_by(json)\nfn api_dec_{ident}(json: String) -> {ident} {{\n"));
    for ((key, f), field) in t.properties.iter().zip(idents) {
        if let FieldSchema::List { items, max, exact: true } = f {
            let what = format!("{name}.{key}");
            let local = format!("l_{field}");
            out.push_str(&format!("    let mut {local} = [{}; {max}];\n", zero(items, &what)?));
            out.push_str(&list_dec(&local, &local, key, items, *max, true, &what)?);
        }
    }
    let bounded = t.properties.iter().any(|(_, f)| matches!(f, FieldSchema::List { exact: false, .. }));
    out.push_str(&format!("    let {}r = {ident} {{\n", if bounded { "mut " } else { "" }));
    for ((key, f), field) in t.properties.iter().zip(idents) {
        match f {
            FieldSchema::List { exact: true, .. } => out.push_str(&format!("        {field}: l_{field},\n")),
            FieldSchema::List { exact: false, .. } => {}
            f => out.push_str(&format!("        {field}: {},\n", decode_field(f, "json", key, &format!("{name}.{key}"))?)),
        }
    }
    out.push_str("    };\n");
    for ((key, f), field) in t.properties.iter().zip(idents) {
        if let FieldSchema::List { items, max, exact: false } = f {
            out.push_str(&list_dec(&format!("r.{field}"), &format!("l_{field}"), key, items, *max, false, &format!("{name}.{key}"))?);
        }
    }
    out.push_str("    return r;\n}\n\n");
    Ok(out)
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
    let result = match &m.response {
        Some(r) => format!("{outcome}::Result({})", decode_whole(r, "raw.body", &format!("{what} response"))?),
        None => format!("{outcome}::Result"),
    };
    out.push_str(&format!("    if raw.outcome == 0 {{ return {result}; }}\n"));
    if let ClientError::Type(e) = &m.error {
        out.push_str(&format!(
            "    if raw.outcome == 1 {{ return {outcome}::HandlerError({}); }}\n",
            decode_whole(e, "raw.body", &format!("{what} error"))?
        ));
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
    let decode = decode_whole(s.payload.as_ref().unwrap_or(&FieldSchema::Unformed), "payload", &format!("{what} payload"))?;
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
         \x20           return {event}::Event(std::json::find_int_field(text, \"seq\"), {decode});\n\
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
    for (name, t) in &model.types {
        claim(type_ident(name))?;
        claim(format!("api_enc_{}", type_ident(name)))?;
        claim(format!("api_dec_{}", type_ident(name)))?;
        for ((_, f), field) in t.properties.iter().zip(field_idents(t)) {
            if matches!(f, FieldSchema::List { .. }) {
                claim(list_enc_name(&type_ident(name), &field))?;
            }
        }
        out.push_str(&struct_decl(name, t)?);
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

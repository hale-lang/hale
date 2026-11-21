//! `hale api describe` and `hale api call`: the operator verbs that drive any
//! served Hale program from the description it serves (spec/api.md §
//! Driving a served program). Nothing is generated and nothing lives in the
//! program: the verb reads the caller's description, types the payload by the
//! member's schema, and speaks the wire of § The `Rpc` interface.
//!
//! A typed flag (`--ship '{"city":"Oslo"}'`) is checked, record fields and list
//! items recursively, against the description's schemas before anything is sent.
//! `--json` is the unchecked path: the payload goes as written and the server's
//! answer is the contract.
//!
//! The endpoints are the two transports a verb can drive today:
//!
//! ```text
//! unix:/run/desk/admin.sock   unix::Rpc, one JSON object per line
//! http://127.0.0.1:8080       http::Rpc, GET /.description, POST /call/<member>
//! ```
//!
//! Exit codes (`describe` uses 0, 2, 4 and 5):
//!
//! ```text
//! 0 a result (or the description)      3 server error
//! 1 handler error                      4 transport failure (no reply, lost)
//! 2 refusal                            5 usage
//! ```

use std::io::Read;
use std::process::ExitCode;

use serde_json::{json, Map, Value};

use crate::api_client::{bearer, http_request, outcome_of_http, outcome_of_line, percent_encode, raw_field, unix_exchange, Kind};

const EXIT_OK: u8 = 0;
const EXIT_HANDLER: u8 = 1;
const EXIT_REFUSED: u8 = 2;
const EXIT_SERVER: u8 = 3;
const EXIT_TRANSPORT: u8 = 4;
const EXIT_USAGE: u8 = 5;

// ---- endpoints --------------------------------------------------------------------

/// Where an exposure listens, as far as these verbs drive it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Endpoint {
    Unix(String),
    /// `host:port`.
    Http(String),
}

const FORMS: &str = "`unix:<path>` (a unix::Rpc socket) and `http://host:port` (an http::Rpc listener)";

impl Endpoint {
    pub(crate) fn parse(s: &str) -> Result<Endpoint, String> {
        if let Some(p) = s.strip_prefix("unix:") {
            if p.is_empty() {
                return Err("`unix:` names no socket path".to_string());
            }
            return Ok(Endpoint::Unix(p.to_string()));
        }
        if let Some(rest) = s.strip_prefix("http://") {
            let hp = rest.trim_end_matches('/');
            if hp.is_empty() || hp.contains('/') || !hp.contains(':') {
                return Err(format!("`{s}` needs a host:port and no path"));
            }
            return Ok(Endpoint::Http(hp.to_string()));
        }
        if s.starts_with("grpc://") || s.starts_with("mcp://") {
            return Err(format!("`{s}`: these verbs drive {FORMS} today; a grpc or mcp listener is driven by its own protocol's clients"));
        }
        Err(format!("`{s}` is not an endpoint: these verbs drive {FORMS}"))
    }

    fn show(&self) -> String {
        match self {
            Endpoint::Unix(p) => format!("unix:{p}"),
            Endpoint::Http(h) => format!("http://{h}"),
        }
    }
}

// ---- the description ---------------------------------------------------------------

/// What asking for the description came to.
enum Fetch {
    /// The document, as the exposure wrote it.
    Doc(String),
    /// The exposure refused to describe: the wire outcome.
    Refused(String),
}

/// The caller's description. `Err` is a transport failure: nothing answered,
/// or what answered is not a Hale exposure.
fn fetch(ep: &Endpoint, token: Option<&str>) -> Result<Fetch, String> {
    let not_hale = |what: &str| format!("{} answered, but it is not a Hale exposure: {what}", ep.show());
    let check = |raw: String| -> Result<Fetch, String> {
        match serde_json::from_str::<Value>(&raw) {
            Ok(v) if v.get("description").map_or(false, Value::is_number) && v.get("members").map_or(false, Value::is_array) => Ok(Fetch::Doc(raw)),
            Ok(_) => Err(not_hale("the document is no description (no `description` version and `members`)")),
            Err(e) => Err(not_hale(&format!("the document is not JSON ({e})"))),
        }
    };
    match ep {
        Endpoint::Unix(path) => {
            let line = unix_exchange(path, &json!({ "describe": true }))?;
            let v: Value = serde_json::from_str(&line).map_err(|_| not_hale("it sent a line that is not JSON"))?;
            if v.get("ok") == Some(&json!(true)) {
                let raw = raw_field(&line, "value").ok_or_else(|| not_hale("the reply carries no value"))?;
                check(raw)
            } else if v.get("refusal").is_some() {
                Ok(Fetch::Refused(line))
            } else {
                Err(not_hale("the reply is no outcome of the wire"))
            }
        }
        Endpoint::Http(hp) => {
            let r = http_request(hp, "GET", "/.description", &bearer(token), None)?;
            let body = r.body.trim().to_string();
            if r.status == 200 {
                check(body)
            } else if raw_field(&body, "refusal").is_some() {
                Ok(Fetch::Refused(body))
            } else {
                Err(not_hale(&format!("HTTP {} with a body that is no refusal", r.status)))
            }
        }
    }
}

/// `--bearer T`, else `HALE_API_BEARER`.
fn bearer_of(given: Option<String>) -> Option<String> {
    given.or_else(|| std::env::var("HALE_API_BEARER").ok().filter(|t| !t.is_empty()))
}

// ---- schemas ----------------------------------------------------------------------

/// `schema` with its `$ref` followed (to a name in the document's `schemas`).
fn resolve<'a>(doc: &'a Value, schema: &'a Value) -> &'a Value {
    let mut cur = schema;
    for _ in 0..8 {
        let Some(r) = cur.get("$ref").and_then(Value::as_str) else { return cur };
        match r.strip_prefix("#/schemas/").and_then(|n| doc.pointer("/schemas").and_then(|s| s.get(n))) {
            Some(next) => cur = next,
            None => return cur,
        }
    }
    cur
}

/// The value schema of an optional field (`{"anyOf": [<schema>, {"type":
/// "null"}]}`, spec/api.md § Codecs), if `schema` is one.
fn nullable_inner(schema: &Value) -> Option<&Value> {
    let any = schema.get("anyOf").and_then(Value::as_array)?;
    let is_null = |s: &Value| s.get("type").and_then(Value::as_str) == Some("null");
    if !any.iter().any(is_null) {
        return None;
    }
    any.iter().find(|s| !is_null(s))
}

/// The name a `$ref` schema carries, if it is one.
fn ref_name(schema: &Value) -> Option<&str> {
    schema.get("$ref").and_then(Value::as_str).and_then(|r| r.rsplit('/').next())
}

/// A schema in the words of the language: `Int (OrderId)`, `String`,
/// `Int (Money, q(cent))`, `[Int]`, a record by its name.
fn type_label(doc: &Value, schema: &Value) -> String {
    if let Some(n) = ref_name(schema) {
        return n.to_string();
    }
    if let Some(inner) = nullable_inner(schema) {
        return format!("{} or null", type_label(doc, inner));
    }
    // an enum: by its name (spec/api.md § Codecs)
    if schema.get("enum").is_some() || schema.get("oneOf").is_some() {
        if let Some(t) = schema.get("x-hale-type").and_then(Value::as_str) {
            return t.to_string();
        }
    }
    let base = match schema.get("type").and_then(Value::as_str) {
        Some("integer") => "Int".to_string(),
        Some("number") => "Float".to_string(),
        Some("boolean") => "Bool".to_string(),
        Some("string") => "String".to_string(),
        Some("array") => {
            let item = schema.get("items").map_or("JSON".to_string(), |i| type_label(doc, i));
            match (schema.get("minItems").and_then(Value::as_u64), schema.get("maxItems").and_then(Value::as_u64)) {
                (Some(lo), Some(hi)) if lo == hi => format!("[{item}; {hi}]"),
                (_, Some(hi)) => format!("[{item}; at most {hi}]"),
                _ => format!("[{item}]"),
            }
        }
        Some("object") => match (schema.get("x-hale-key").and_then(Value::as_str), schema.get("x-hale-value").and_then(Value::as_str)) {
            (Some(k), Some(v)) => format!("{{map by {k} of {v}}}"),
            (Some(k), None) => format!("{{map by {k}}}"),
            (None, _) => "{record}".to_string(),
        },
        _ => "JSON".to_string(),
    };
    match (schema.get("x-hale-type").and_then(Value::as_str), schema.get("x-hale-unit").and_then(Value::as_str)) {
        (Some(t), Some(u)) => format!("{base} ({t}, {u})"),
        (Some(t), None) => format!("{base} ({t})"),
        (None, Some(u)) => format!("{base} ({u})"),
        (None, None) => base,
    }
}

/// A member's payload fields: name, schema, required. The required fields come
/// first in the schema's own order (`required` is an array, so it keeps the
/// order the record declares; a parsed `properties` object does not), then the
/// optional ones by name.
fn payload_fields<'a>(doc: &'a Value, member: &'a Value) -> Vec<(&'a str, &'a Value, bool)> {
    let Some(req) = member.get("request") else { return Vec::new() };
    let schema = resolve(doc, req);
    let Some(props) = schema.get("properties").and_then(Value::as_object) else { return Vec::new() };
    let required: Vec<&str> = schema.get("required").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    let mut out: Vec<(&str, &Value, bool)> = required.iter().filter_map(|n| props.get_key_value(*n).map(|(k, v)| (k.as_str(), v, true))).collect();
    out.extend(props.iter().filter(|(k, _)| !required.contains(&k.as_str())).map(|(k, v)| (k.as_str(), v, false)));
    out
}

/// A member whose request is one value and no record (`fn submit(id: Int)`):
/// the parameter's name, as the description gives it (`"param"`, GH #2109;
/// a description served before it gives none), and the request's schema.
/// `None` for a record request and for a member that takes nothing.
fn scalar_request<'a>(doc: &'a Value, member: &'a Value) -> Option<(Option<&'a str>, &'a Value)> {
    let req = member.get("request").filter(|r| !r.is_null())?;
    if resolve(doc, req).get("properties").is_some() {
        return None;
    }
    Some((member.get("param").and_then(Value::as_str), req))
}

/// What a flag's value is written as, for a usage line: `<int>`, `<Red|Green>`.
fn value_hint(doc: &Value, schema: &Value) -> String {
    let r = resolve(doc, schema);
    if let Some(names) = r.get("enum").and_then(Value::as_array) {
        return names.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("|");
    }
    if is_decimal(r) {
        return "decimal".to_string();
    }
    match (r.get("type").and_then(Value::as_str), r.get("format").and_then(Value::as_str)) {
        (Some("string"), Some("date-time")) => "time, e.g. 2026-10-12T03:04:05Z".to_string(),
        (Some("integer"), _) => match r.get("x-hale-unit").and_then(Value::as_str).and_then(|u| u.strip_prefix("q(")).and_then(|u| u.strip_suffix(')')) {
            Some(u) => format!("int, a count of {u}"),
            None => "int".to_string(),
        },
        (Some("number"), _) => "number".to_string(),
        (Some("boolean"), _) => "true|false".to_string(),
        (Some("string"), _) => "text".to_string(),
        _ => "json".to_string(),
    }
}

/// How a scalar request is given: `Svc::submit takes one Int, `id`: pass
/// `--id <int>``.
fn scalar_usage(doc: &Value, name: &str, param: Option<&str>, schema: &Value) -> String {
    let ty = type_label(doc, schema);
    match param {
        Some(p) => format!("{name} takes one {ty}, `{p}`: pass `--{} <{}>`, or its JSON with --json", p.replace('_', "-"), value_hint(doc, schema)),
        None => format!("{name} takes one {ty}, and its description names no parameter: pass its JSON with --json"),
    }
}

fn requires_of(member: &Value) -> Vec<&str> {
    member.get("requires").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default()
}

// ---- describe ---------------------------------------------------------------------

/// The readable rendering: the exposure's identity line, the caller, then a
/// row per member the caller may see.
fn render(doc: &Value) -> String {
    let s = |p: &str| doc.pointer(p).and_then(Value::as_str).unwrap_or("?").to_string();
    let mut out = format!("{}  [{} {}]\n", s("/exposure"), s("/listener/transport"), s("/listener/address"));
    let roles: Vec<&str> = doc.pointer("/caller/roles").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    out.push_str(&format!(
        "caller: {} {}; roles: {}\n",
        s("/caller/principal/mode"),
        s("/caller/principal/name"),
        if roles.is_empty() { "none".to_string() } else { roles.join(", ") }
    ));
    let members = doc.get("members").and_then(Value::as_array).cloned().unwrap_or_default();
    if members.is_empty() {
        out.push_str("members: none\n");
    }
    for m in &members {
        let name = m.get("name").and_then(Value::as_str).unwrap_or("?");
        let fields: Vec<String> = match scalar_request(doc, m) {
            Some((p, sc)) => vec![format!("{}: {}", p.unwrap_or("request"), type_label(doc, sc))],
            None => payload_fields(doc, m).iter().map(|(n, sc, req)| format!("{n}{}: {}", if *req { "" } else { "?" }, type_label(doc, sc))).collect(),
        };
        let result = m.get("response").map_or("?".to_string(), |r| type_label(doc, r));
        let error = match m.get("error") {
            Some(Value::String(e)) => format!(", error {e}"),
            Some(e @ Value::Object(_)) => format!(", error {}", type_label(doc, e)),
            _ => String::new(),
        };
        let req = requires_of(m);
        out.push_str(&format!("  {name}({}) -> {result}{error}  requires: {}\n", fields.join(", "), if req.is_empty() { "-".to_string() } else { req.join(", ") }));
        // GH #1674: the member's doc comment, then each documented field's
        if let Some(d) = m.get("description").and_then(Value::as_str) {
            for l in d.lines() {
                out.push_str(&if l.is_empty() { "\n".to_string() } else { format!("    {l}\n") });
            }
        }
        for (n, sc, _) in payload_fields(doc, m) {
            if let Some(d) = sc.get("description").and_then(Value::as_str) {
                out.push_str(&format!("    {n}: {}\n", d.lines().next().unwrap_or("")));
            }
        }
    }
    out
}

/// A refusal's wire text on stderr, then the exit code.
fn refused(verb: &str, wire: &str) -> ExitCode {
    eprintln!("hale api {verb}: refused: {wire}");
    ExitCode::from(EXIT_REFUSED)
}

fn usage_error(verb: &str, msg: &str) -> ExitCode {
    eprintln!("hale api {verb}: {msg}");
    ExitCode::from(EXIT_USAGE)
}

/// One argument of a verb: `--flag value`, `--flag=value` or a bare token.
#[derive(Debug, PartialEq)]
enum Arg {
    Flag(String, Option<String>),
    Pos(String),
}

/// Splits `rest` into flags and positionals. A flag takes the next token as
/// its value when it does not itself start with `--`; `switches` never do.
fn split_args(rest: &[String], switches: &[&str]) -> Vec<Arg> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < rest.len() {
        let a = &rest[i];
        i += 1;
        let Some(flag) = a.strip_prefix("--") else {
            out.push(Arg::Pos(a.clone()));
            continue;
        };
        if let Some((k, v)) = flag.split_once('=') {
            out.push(Arg::Flag(k.to_string(), Some(v.to_string())));
        } else if switches.contains(&flag) || i >= rest.len() || rest[i].starts_with("--") {
            out.push(Arg::Flag(flag.to_string(), None));
        } else {
            out.push(Arg::Flag(flag.to_string(), Some(rest[i].clone())));
            i += 1;
        }
    }
    out
}

/// The short forms `hale describe` and `hale call`: the `api` verbs with two conveniences. A
/// bare path (no `unix:` and no `scheme://`) is `unix:<path>`, and a bare positional payload after
/// the member is `--json <payload>`. Everything else goes through as written.
pub(crate) fn short_form(verb: &str, rest: &[String]) -> Vec<String> {
    let switches: &[&str] = if verb == "call" { &["raw"] } else { &["json"] };
    let mut out = Vec::new();
    let mut at = 0;
    for a in split_args(rest, switches) {
        match a {
            Arg::Flag(k, None) => out.push(format!("--{k}")),
            Arg::Flag(k, Some(v)) => out.push(format!("--{k}={v}")),
            Arg::Pos(p) => {
                match at {
                    0 if !p.starts_with("unix:") && !p.contains("://") => out.push(format!("unix:{p}")),
                    2 if verb == "call" => {
                        out.push("--json".to_string());
                        out.push(p);
                    }
                    _ => out.push(p),
                }
                at += 1;
            }
        }
    }
    out
}

/// `hale api describe <endpoint> [--json] [--bearer T]`.
pub(crate) fn run_describe(rest: &[String]) -> ExitCode {
    let mut json_out = false;
    let mut token = None;
    let mut pos = Vec::new();
    for a in split_args(rest, &["json"]) {
        match a {
            Arg::Flag(k, None) if k == "json" => json_out = true,
            Arg::Flag(k, Some(v)) if k == "bearer" => token = Some(v),
            Arg::Flag(k, _) => return usage_error("describe", &format!("unknown or incomplete flag `--{k}` (usage: hale api describe <endpoint> [--json] [--bearer T])")),
            Arg::Pos(p) => pos.push(p),
        }
    }
    let [target] = pos.as_slice() else {
        return usage_error("describe", "usage: hale api describe <endpoint> [--json] [--bearer T]\n  endpoint: unix:<path> or http://host:port");
    };
    let ep = match Endpoint::parse(target) {
        Ok(e) => e,
        Err(e) => return usage_error("describe", &e),
    };
    let token = bearer_of(token);
    match fetch(&ep, token.as_deref()) {
        Err(e) => {
            eprintln!("hale api describe: {e}");
            ExitCode::from(EXIT_TRANSPORT)
        }
        Ok(Fetch::Refused(wire)) => refused("describe", &wire),
        Ok(Fetch::Doc(raw)) => {
            if json_out {
                println!("{}", raw.trim());
            } else {
                match serde_json::from_str::<Value>(&raw) {
                    Ok(doc) => print!("{}", render(&doc)),
                    Err(e) => {
                        eprintln!("hale api describe: the description is not JSON: {e}");
                        return ExitCode::from(EXIT_TRANSPORT);
                    }
                }
            }
            ExitCode::from(EXIT_OK)
        }
    }
}

// ---- call -------------------------------------------------------------------------

/// The flags the verb owns; a payload field of the same name is given with `--json`.
/// The request id is `--request-id`, not `--id`: `id` is the commonest payload field
/// name there is, and an own flag of that name made it unsettable from a flag.
const OWN_FLAGS: [&str; 5] = ["json", "bearer", "request-id", "digest", "raw"];

const CALL_USAGE: &str = "usage: hale api call <endpoint> <member> [--json '<payload>' | --json - | --<field> <value> ...]\n  [--bearer T] [--request-id ID] [--digest D] [--raw]\n  endpoint: unix:<path> or http://host:port\n  member: Locus::fn, or 'METHOD /path' for a route of an http::Rest listener (the flags are its query)";

/// A schema in full, compact, for a message that names a field.
fn schema_text(schema: &Value) -> String {
    schema.to_string()
}

/// `text` as the value of a field with `schema`: Int, Float, Bool and String by
/// their text, a quantity by its count, anything else (a record, a list) as
/// JSON that `check_value` holds to the description's schemas, nested records
/// and lists included.
///
/// `param` is a scalar request's parameter (GH #2109), whose value is also
/// held to what the codec reads before it is sent: a payload-free enum's wire
/// names, an instant's RFC 3339 text in UTC, a `Decimal`'s digits, each kept.
/// A record's field is not: its text goes as before, and the server refuses
/// what it cannot read.
fn typed_value(doc: &Value, field: &str, param: bool, schema: &Value, text: Option<&str>) -> Result<Value, String> {
    let resolved = resolve(doc, schema);
    let kind = resolved.get("type").and_then(Value::as_str).unwrap_or("");
    let noun = if param { "parameter" } else { "field" };
    let refuse = |why: &str| format!("`--{field}`: {why}; the {noun} is {} {}", type_label(doc, schema), schema_text(resolved));
    if param {
        if let Some(v) = param_value(resolved, text).map(|r| r.map_err(|why| refuse(&why))) {
            return v;
        }
    }
    match (kind, text) {
        ("boolean", None) => Ok(json!(true)),
        (_, None) => Err(refuse("needs a value")),
        ("integer", Some(t)) => t.trim().parse::<i64>().map(|n| json!(n)).map_err(|_| refuse(&format!("`{t}` is not an integer"))),
        ("number", Some(t)) => match t.trim().parse::<f64>() {
            Ok(f) if f.is_finite() => Ok(json!(f)),
            _ => Err(refuse(&format!("`{t}` is not a number"))),
        },
        ("boolean", Some(t)) => match t.trim() {
            "true" => Ok(json!(true)),
            "false" => Ok(json!(false)),
            _ => Err(refuse(&format!("`{t}` is not true or false"))),
        },
        ("string", Some(t)) => Ok(json!(t)),
        (k, Some(t)) => {
            let v: Value = serde_json::from_str(t).map_err(|e| refuse(&format!("`{t}` is not JSON ({e})")))?;
            let shape_ok = match k {
                "array" => v.is_array(),
                "object" => v.is_object(),
                _ => true,
            };
            if !shape_ok {
                return Err(refuse(&format!("`{t}` is not a JSON {k}")));
            }
            check_value(doc, field, schema, &v).map_err(|why| refuse(&why))?;
            Ok(v)
        }
    }
}

/// The checks a scalar parameter's text gets beyond its JSON kind's
/// ([`typed_value`]): `None` where there are none and the kind's own apply.
fn param_value(schema: &Value, text: Option<&str>) -> Option<Result<Value, String>> {
    let t = text?;
    // a payload-free enum by a wire name its description lists
    if let Some(names) = schema.get("enum").and_then(Value::as_array) {
        let listed: Vec<&str> = names.iter().filter_map(Value::as_str).collect();
        return Some(if listed.contains(&t) { Ok(json!(t)) } else { Err(format!("`{t}` is not one of {}", listed.join(", "))) });
    }
    // a Decimal by its digits, every one kept (spec/api.md § Codecs)
    if is_decimal(schema) {
        return Some(decimal_value(t.trim(), schema));
    }
    if schema.get("format").and_then(Value::as_str) == Some("date-time") {
        return Some(if is_utc_instant(t) {
            Ok(json!(t))
        } else {
            Err(format!("`{t}` is not an RFC 3339 instant in UTC (`2026-10-12T03:04:05Z`, `2026-10-12T03:04:05.25Z`)"))
        });
    }
    None
}

/// A schema of a `Decimal<S>` (`"x-hale-type": "Decimal<2>"`, a number or, for
/// a field tagged `string`, a string).
fn is_decimal(schema: &Value) -> bool {
    schema.get("x-hale-type").and_then(Value::as_str).is_some_and(|t| t.starts_with("Decimal<"))
}

/// A JSON number's text (RFC 8259 § 6) as its sign, its digits with no
/// leading or trailing zero, and the power of ten of the last of them; `None`
/// for a text that is no JSON number. `0` is `(false, "", 0)`.
fn decimal_parts(t: &str) -> Option<(bool, String, i64)> {
    let (neg, rest) = match t.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, t),
    };
    let (mantissa, exp) = match rest.find(['e', 'E']) {
        Some(i) => {
            let e = &rest[i + 1..];
            let digits = e.strip_prefix(['+', '-']).unwrap_or(e);
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            (&rest[..i], e.parse::<i64>().ok()?)
        }
        None => (rest, 0),
    };
    let (int, frac) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let int_ok = int == "0" || (!int.is_empty() && !int.starts_with('0'));
    let frac_ok = !mantissa.contains('.') || !frac.is_empty();
    if !int_ok || !frac_ok || !int.bytes().chain(frac.bytes()).all(|b| b.is_ascii_digit()) {
        return None;
    }
    let all = format!("{int}{frac}");
    let trimmed = all.trim_end_matches('0');
    let power = exp.saturating_sub(frac.len() as i64).saturating_add((all.len() - trimmed.len()) as i64);
    let digits = trimmed.trim_start_matches('0').to_string();
    if digits.is_empty() {
        return Some((false, String::new(), 0));
    }
    Some((neg, digits, power))
}

/// A `Decimal<S>`'s flag: a JSON number (an exponent applied) with no nonzero
/// digit past the scale. It is sent as the JSON number it reads as when a double
/// carries it exactly (the wire `--json` of the same text writes), else as a
/// JSON string of its digits, which the decoder reads to the digit (spec/api.md
/// § Codecs), so no digit is lost on the way.
fn decimal_value(t: &str, schema: &Value) -> Result<Value, String> {
    let Some((neg, digits, power)) = decimal_parts(t) else {
        return Err(format!("`{t}` is not a decimal number"));
    };
    let scale = schema.get("x-hale-type").and_then(Value::as_str).and_then(|n| n.strip_prefix("Decimal<")).and_then(|n| n.strip_suffix('>')).and_then(|n| n.parse::<i64>().ok());
    if let Some(sc) = scale.filter(|sc| !digits.is_empty() && power < -sc) {
        return Err(format!("`{t}` has a digit past the scale of {sc}"));
    }
    let quoted = schema.get("type").and_then(Value::as_str) == Some("string");
    if !quoted {
        if let Ok(v) = serde_json::from_str::<Value>(t) {
            if let Some(f) = v.as_f64().filter(|f| f.is_finite()) {
                if decimal_parts(&format!("{f}")) == Some((neg, digits.clone(), power)) {
                    return Ok(v);
                }
            }
        }
    }
    Ok(json!(t))
}

/// `t` is an instant as the codec reads one: `YYYY-MM-DDTHH:MM:SS`, a fraction
/// of one to nine digits, and `Z` (spec/api.md § An instant and a duration).
/// Whether the date exists and lies in `Time`'s range is the server's to say.
fn is_utc_instant(t: &str) -> bool {
    let b = t.as_bytes();
    let digit = |i: usize| b.get(i).is_some_and(u8::is_ascii_digit);
    let at = |i: usize, c: u8| b.get(i) == Some(&c);
    let head = (0..4).all(digit) && at(4, b'-') && digit(5) && digit(6) && at(7, b'-') && digit(8) && digit(9) && at(10, b'T') && digit(11) && digit(12) && at(13, b':') && digit(14) && digit(15) && at(16, b':') && digit(17) && digit(18);
    if !head {
        return false;
    }
    let tail = &t[19..];
    let tail = match tail.strip_prefix('.') {
        Some(f) => {
            let n = f.bytes().take_while(u8::is_ascii_digit).count();
            if n == 0 || n > 9 {
                return false;
            }
            &f[n..]
        }
        None => tail,
    };
    tail == "Z"
}

/// `v` against `schema` (references followed through the description's `schemas`), or
/// the first place it does not fit, named by its path (`ship.city`, `items[0]`), the
/// type expected there and what was given. A record is an object with every required
/// field, no unknown field, each field fitting its type; a list is an array whose items
/// all fit the item type; a quantity or a distinct integer fits by its count. A schema
/// with no `type` constrains nothing.
fn check_value(doc: &Value, path: &str, schema: &Value, v: &Value) -> Result<(), String> {
    // an optional field takes `null` (its absence) or a value of its type
    if let Some(inner) = nullable_inner(schema) {
        return if v.is_null() { Ok(()) } else { check_value(doc, path, inner, v) };
    }
    // an enum's variant: a name it lists, or an object one of its forms admits
    if let Some(any) = schema.get("oneOf").and_then(Value::as_array) {
        return if any.iter().any(|f| check_value(doc, path, f, v).is_ok()) {
            Ok(())
        } else {
            Err(format!("`{path}` is `{v}`, no variant of {}", type_label(doc, schema)))
        };
    }
    if let Some(names) = schema.get("enum").and_then(Value::as_array) {
        return if names.iter().any(|n| n == v) {
            Ok(())
        } else {
            Err(format!("`{path}` is `{v}`, no variant of {}", type_label(doc, schema)))
        };
    }
    let resolved = resolve(doc, schema);
    let misfit = |what: &str| format!("`{path}` is `{v}`, {what}; expected {}", type_label(doc, schema));
    match resolved.get("type").and_then(Value::as_str) {
        Some("integer") if !(v.is_i64() || v.is_u64()) => Err(misfit("not an integer")),
        Some("number") if !v.is_number() => Err(misfit("not a number")),
        Some("boolean") if !v.is_boolean() => Err(misfit("not true or false")),
        Some("string") if !v.is_string() => Err(misfit("not a string")),
        Some("array") => {
            let Some(items) = v.as_array() else { return Err(misfit("not a list")) };
            let n = items.len() as u64;
            if let Some(hi) = resolved.get("maxItems").and_then(Value::as_u64).filter(|hi| n > *hi) {
                return Err(misfit(&format!("{n} items, more than the {hi} the list holds")));
            }
            if let Some(lo) = resolved.get("minItems").and_then(Value::as_u64).filter(|lo| n < *lo) {
                return Err(misfit(&format!("{n} items, fewer than the {lo} the list holds")));
            }
            match resolved.get("items") {
                Some(item) => items.iter().enumerate().try_for_each(|(i, x)| check_value(doc, &format!("{path}[{i}]"), item, x)),
                None => Ok(()),
            }
        }
        // a wire map: every value fits the value schema, no more than its bound
        Some("object") if resolved.get("additionalProperties").is_some_and(Value::is_object) => {
            let Some(obj) = v.as_object() else { return Err(misfit("not an object")) };
            if let Some(hi) = resolved.get("maxProperties").and_then(Value::as_u64).filter(|hi| obj.len() as u64 > *hi) {
                return Err(misfit(&format!("{} entries, more than the {hi} the map holds", obj.len())));
            }
            let value = &resolved["additionalProperties"];
            obj.iter().try_for_each(|(k, x)| check_value(doc, &format!("{path}.{k}"), value, x))
        }
        Some("object") => {
            let Some(obj) = v.as_object() else { return Err(misfit("not a record")) };
            let props = resolved.get("properties").and_then(Value::as_object);
            let required = resolved.get("required").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str);
            for r in required {
                if !obj.contains_key(r) {
                    return Err(format!("`{path}` is `{v}`, which lacks the required field `{r}`; expected {}", type_label(doc, schema)));
                }
            }
            for (k, x) in obj {
                match props.and_then(|p| p.get(k)) {
                    Some(fs) => check_value(doc, &format!("{path}.{k}"), fs, x)?,
                    None if props.is_some() => return Err(format!("`{path}` is `{v}`, which has the unknown field `{k}`; expected {}", type_label(doc, schema))),
                    None => {}
                }
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// The payload a member's typed flags build, in the schema's field order; or
/// why not, before anything is sent.
fn build_payload(doc: &Value, member: &Value, flags: &[(String, Option<String>)]) -> Result<Value, String> {
    let name = member.get("name").and_then(Value::as_str).unwrap_or("?");
    // GH #2109: a request that is one value is given by its parameter's name,
    // `--id 7`, and sent as that value alone (the wire of § The `Rpc` interface)
    if let Some((param, schema)) = scalar_request(doc, member) {
        let usage = scalar_usage(doc, name, param, schema);
        let other = flags.iter().find(|(f, _)| param != Some(f.replace('-', "_").as_str()));
        return match (other, flags) {
            (Some((flag, _)), _) => Err(format!("`--{flag}` is not a parameter of {name}; {usage}")),
            (None, []) => Err(usage),
            (None, [(flag, text)]) => typed_value(doc, flag, true, schema, text.as_deref()),
            (None, [(flag, _), ..]) => Err(format!("`--{flag}` is given twice; {usage}")),
        };
    }
    let fields = payload_fields(doc, member);
    let listed = || fields.iter().map(|(n, s, r)| format!("--{n} <{}>{}", type_label(doc, s), if *r { "" } else { " (optional)" })).collect::<Vec<_>>().join(" ");
    let mut given: Vec<(&str, Option<&str>)> = Vec::new();
    for (flag, text) in flags {
        let key = flag.replace('-', "_");
        let Some((n, _, _)) = fields.iter().find(|(n, _, _)| *n == key) else {
            return Err(format!("`--{flag}` is no field of {name}'s payload; its fields: {}", if fields.is_empty() { "none".to_string() } else { listed() }));
        };
        if given.iter().any(|(g, _)| g == n) {
            return Err(format!("`--{flag}` is given twice"));
        }
        given.push((n, text.as_deref()));
    }
    let mut out = Map::new();
    for (n, schema, required) in &fields {
        match given.iter().find(|(g, _)| g == n) {
            Some((_, text)) => {
                out.insert((*n).to_string(), typed_value(doc, n, false, schema, *text)?);
            }
            None if *required => {
                return Err(format!(
                    "the payload of {name} needs `--{n}`: {} {}; its fields: {}",
                    type_label(doc, schema),
                    schema_text(resolve(doc, schema)),
                    listed()
                ));
            }
            None => {}
        }
    }
    Ok(Value::Object(out))
}

fn is_object_schema(doc: &Value, member: &Value) -> bool {
    member.get("request").map_or(false, |r| resolve(doc, r).get("type").and_then(Value::as_str) == Some("object"))
}

/// The wire request and its answer, as an outcome of § Outcomes or a transport failure.
fn send(ep: &Endpoint, token: Option<&str>, member: &str, payload: &Value, id: &str, digest: Option<&str>) -> Result<crate::api_client::Reply, String> {
    match ep {
        Endpoint::Unix(path) => {
            let mut req = Map::new();
            req.insert("call".into(), json!(member));
            req.insert("payload".into(), payload.clone());
            req.insert("id".into(), json!(id));
            if let Some(d) = digest {
                req.insert("digest".into(), json!(d));
            }
            let line = unix_exchange(path, &Value::Object(req))?;
            let v: Value = serde_json::from_str(&line).map_err(|_| format!("the reply is not JSON: {line}"))?;
            if v.get("id") != Some(&json!(id)) {
                return Err(format!("the reply answers another request (id {}, sent {id:?}): {line}", v.get("id").map_or("none".to_string(), Value::to_string)));
            }
            outcome_of_line(&line)
        }
        Endpoint::Http(hp) => {
            let mut headers = bearer(token);
            if let Some(d) = digest {
                headers.push(("Hale-Surface-Digest", d.to_string()));
            }
            let path = format!("/call/{}", percent_encode(member));
            outcome_of_http(http_request(hp, "POST", &path, &headers, Some(&payload.to_string()))?)
        }
    }
}

fn members_listed(doc: &Value) -> String {
    let names: Vec<&str> = doc.get("members").and_then(Value::as_array).map(|a| a.iter().filter_map(|m| m.get("name").and_then(Value::as_str)).collect()).unwrap_or_default();
    if names.is_empty() {
        "none".to_string()
    } else {
        names.join(", ")
    }
}

/// `hale api call <endpoint> <member> [payload] [--bearer T] [--request-id ID] [--digest D] [--raw]`.
pub(crate) fn run_call(rest: &[String]) -> ExitCode {
    let mut json_arg: Option<String> = None;
    let (mut token, mut id, mut digest) = (None, None, None);
    let mut raw = false;
    let mut pos = Vec::new();
    let mut flags: Vec<(String, Option<String>)> = Vec::new();
    for a in split_args(rest, &["raw"]) {
        match a {
            Arg::Pos(p) => pos.push(p),
            Arg::Flag(k, v) if OWN_FLAGS.contains(&k.as_str()) => match (k.as_str(), v) {
                ("raw", None) => raw = true,
                ("json", Some(v)) => json_arg = Some(v),
                ("bearer", Some(v)) => token = Some(v),
                ("request-id", Some(v)) => id = Some(v),
                ("digest", Some(v)) => digest = Some(v),
                (k, _) => return usage_error("call", &format!("`--{k}` is incomplete\n{CALL_USAGE}")),
            },
            Arg::Flag(k, v) => flags.push((k, v)),
        }
    }
    let [target, member] = pos.as_slice() else {
        return usage_error("call", CALL_USAGE);
    };
    let ep = match Endpoint::parse(target) {
        Ok(e) => e,
        Err(e) => return usage_error("call", &e),
    };
    if json_arg.is_some() && !flags.is_empty() {
        return usage_error("call", "give the payload as --json or as typed flags, not both");
    }
    let given_payload = match json_arg.as_deref() {
        None => None,
        Some(text) => {
            let mut buf = String::new();
            let text = if text == "-" {
                if let Err(e) = std::io::stdin().read_to_string(&mut buf) {
                    return usage_error("call", &format!("could not read the payload from stdin: {e}"));
                }
                buf.as_str()
            } else {
                text
            };
            match serde_json::from_str::<Value>(text) {
                Ok(v) => Some(v),
                Err(e) => return usage_error("call", &format!("the --json payload is not JSON: {e}")),
            }
        }
    };
    let token = bearer_of(token);

    // A route of an `http::Rest` listener is called by its method and path
    // (`'GET /orders/5'`): no description is read, the flags are the query
    // and a `--json` payload the body of a method that takes one.
    if let Some((method, path)) = member.split_once(' ').filter(|(m, p)| matches!(*m, "GET" | "POST" | "PUT" | "PATCH" | "DELETE") && p.starts_with('/')) {
        let Endpoint::Http(hp) = &ep else {
            return usage_error("call", "a route (`METHOD /path`) is called on an http:// endpoint");
        };
        if given_payload.is_some() && matches!(method, "GET" | "DELETE") {
            return usage_error("call", &format!("{method} takes no body: give the request's fields as flags, they are the query"));
        }
        let mut target = path.to_string();
        for (i, (k, v)) in flags.iter().enumerate() {
            target.push(if i == 0 && !path.contains('?') { '?' } else { '&' });
            target.push_str(&percent_encode(&k.replace('-', "_")));
            target.push('=');
            target.push_str(&percent_encode(v.as_deref().unwrap_or("true")));
        }
        let mut headers = bearer(token.as_deref());
        if let Some(d) = &digest {
            headers.push(("Hale-Surface-Digest", d.to_string()));
        }
        let body = given_payload.as_ref().map(Value::to_string);
        let reply = match http_request(hp, method, &target, &headers, body.as_deref()).and_then(outcome_of_http) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("hale api call: transport failure: {e}");
                return ExitCode::from(EXIT_TRANSPORT);
            }
        };
        return report(&reply, raw);
    }

    // The description is read once, before the payload is built: it types the
    // flags and names the digest the call is held to.
    let doc = match fetch(&ep, token.as_deref()) {
        Err(e) => {
            eprintln!("hale api call: {e}");
            return ExitCode::from(EXIT_TRANSPORT);
        }
        Ok(Fetch::Refused(wire)) if given_payload.is_none() => return refused("call", &wire),
        // a payload given as JSON is sent as it is: the server decides, and says why
        Ok(Fetch::Refused(_)) => None,
        Ok(Fetch::Doc(raw)) => match serde_json::from_str::<Value>(&raw) {
            Ok(d) => Some(d),
            Err(e) => {
                eprintln!("hale api call: the description is not JSON: {e}");
                return ExitCode::from(EXIT_TRANSPORT);
            }
        },
    };
    let entry = doc.as_ref().and_then(|d| d.get("members").and_then(Value::as_array).and_then(|a| a.iter().find(|m| m.get("name").and_then(Value::as_str) == Some(member.as_str()))));
    let payload = match (given_payload, doc.as_ref(), entry) {
        (Some(p), _, _) => p,
        (None, Some(d), Some(m)) => {
            if flags.is_empty() && !is_object_schema(d, m) && scalar_request(d, m).is_none() {
                return usage_error("call", &format!("the payload of {member} is not a record: give it with --json"));
            }
            match build_payload(d, m, &flags) {
                Ok(p) => p,
                Err(e) => return usage_error("call", &e),
            }
        }
        (None, Some(d), None) => {
            let listed = members_listed(d);
            return usage_error("call", &format!("{member} is not a member this caller sees (members: {listed}); --json sends a payload as given, and the server decides"));
        }
        (None, None, _) => unreachable!("a refused description with no --json returned above"),
    };
    let digest = digest.or_else(|| doc.as_ref().and_then(|d| d.get("digest").and_then(Value::as_str).map(str::to_string)));
    let id = id.unwrap_or_else(|| format!("hale-{}", std::process::id()));
    let reply = match send(&ep, token.as_deref(), member, &payload, &id, digest.as_deref()) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("hale api call: transport failure: {e}");
            return ExitCode::from(EXIT_TRANSPORT);
        }
    };
    report(&reply, raw)
}

/// Print an outcome as the verb does and exit by it.
fn report(reply: &crate::api_client::Reply, raw: bool) -> ExitCode {
    if raw {
        println!("{}", reply.raw.trim());
    }
    match reply.kind {
        Kind::Result => {
            if !raw {
                println!("{}", reply.body.trim());
            }
            ExitCode::from(EXIT_OK)
        }
        k => {
            if !raw {
                // a classed handler error names its class (spec/api.md §
                // Error classes)
                match (&reply.class, k) {
                    (Some(c), Kind::HandlerError) => eprintln!("handler error ({c}): {}", reply.raw.trim()),
                    _ => eprintln!("{}", reply.raw.trim()),
                }
            }
            ExitCode::from(match k {
                Kind::HandlerError => EXIT_HANDLER,
                Kind::Refusal => EXIT_REFUSED,
                _ => EXIT_SERVER,
            })
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_are_the_two_forms() {
        assert_eq!(Endpoint::parse("unix:/tmp/a.sock"), Ok(Endpoint::Unix("/tmp/a.sock".into())));
        assert_eq!(Endpoint::parse("http://127.0.0.1:8080"), Ok(Endpoint::Http("127.0.0.1:8080".into())));
        for bad in ["grpc://h:1", "mcp://h:1", "/tmp/a.sock", "ws://h:1", "http://h", "unix:"] {
            assert!(Endpoint::parse(bad).is_err(), "{bad}");
        }
        assert!(Endpoint::parse("grpc://h:1").unwrap_err().contains("unix:<path>"));
    }

    #[test]
    fn args_split_into_flags_and_positionals() {
        let a = |v: &[&str]| split_args(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>(), &["json"]);
        assert_eq!(a(&["e", "--json", "m"]), vec![Arg::Pos("e".into()), Arg::Flag("json".into(), None), Arg::Pos("m".into())]);
        assert_eq!(a(&["--x=1", "--y", "2", "--z"]), vec![Arg::Flag("x".into(), Some("1".into())), Arg::Flag("y".into(), Some("2".into())), Arg::Flag("z".into(), None)]);
    }

    /// A description with a member per scalar kind, as a served one lists them
    /// (GH #2109), and a record whose field is named like a parameter.
    fn scalar_doc() -> Value {
        let m = |name: &str, param: Option<&str>, request: Value| {
            let mut m = json!({"name": name, "request": request, "response": {"type": "integer"}, "error": null, "requires": []});
            if let Some(p) = param {
                m["param"] = json!(p);
            }
            m
        };
        json!({
            "members": [
                m("Svc::submit", Some("id"), json!({"type": "integer"})),
                m("Svc::ratio", Some("x"), json!({"type": "number"})),
                m("Svc::flag", Some("on"), json!({"type": "boolean"})),
                m("Svc::echo", Some("text"), json!({"type": "string"})),
                m("Svc::price", Some("p"), json!({"type": "number", "x-hale-type": "Decimal<2>"})),
                m("Svc::quoted", Some("p"), json!({"type": "string", "x-hale-type": "Decimal<2>"})),
                m("Svc::stamp", Some("at"), json!({"type": "string", "format": "date-time", "x-hale-type": "Time"})),
                m("Svc::wait", Some("d"), json!({"type": "integer", "x-hale-type": "Duration", "x-hale-unit": "q(ns)"})),
                m("Svc::charge", Some("m"), json!({"type": "integer", "x-hale-type": "Money", "x-hale-unit": "q(cent)"})),
                m("Svc::paint", Some("c"), json!({"type": "string", "enum": ["Red", "Green", "Blue"], "x-hale-type": "Color"})),
                m("Svc::ship", Some("ship_to"), json!({"type": "string"})),
                m("Svc::old", None, json!({"type": "integer"})),
                m("Svc::pair", None, json!({"$ref": "#/schemas/Pair"})),
            ],
            "schemas": {"Pair": {"type": "object", "properties": {"id": {"type": "integer"}, "n": {"type": "integer"}}, "required": ["id", "n"]}},
        })
    }

    fn payload(member: &str, flags: &[(&str, Option<&str>)]) -> Result<Value, String> {
        let doc = scalar_doc();
        let m = doc["members"].as_array().unwrap().iter().find(|m| m["name"] == member).unwrap().clone();
        build_payload(&doc, &m, &flags.iter().map(|(k, v)| (k.to_string(), v.map(str::to_string))).collect::<Vec<_>>())
    }

    #[test]
    fn a_scalar_request_is_its_parameter_sent_as_the_value_alone() {
        assert_eq!(payload("Svc::submit", &[("id", Some("7"))]), Ok(json!(7)));
        assert_eq!(payload("Svc::ratio", &[("x", Some("1.25"))]), Ok(json!(1.25)));
        assert_eq!(payload("Svc::flag", &[("on", Some("false"))]), Ok(json!(false)));
        assert_eq!(payload("Svc::flag", &[("on", None)]), Ok(json!(true)));
        assert_eq!(payload("Svc::echo", &[("text", Some("hi there"))]), Ok(json!("hi there")));
        assert_eq!(payload("Svc::stamp", &[("at", Some("2026-10-12T03:04:05.123456789Z"))]), Ok(json!("2026-10-12T03:04:05.123456789Z")));
        assert_eq!(payload("Svc::wait", &[("d", Some("1500000000"))]), Ok(json!(1500000000)));
        assert_eq!(payload("Svc::charge", &[("m", Some("125"))]), Ok(json!(125)));
        assert_eq!(payload("Svc::paint", &[("c", Some("Green"))]), Ok(json!("Green")));
        // a parameter's `_` may be written `-`, as a field's may
        assert_eq!(payload("Svc::ship", &[("ship-to", Some("Oslo"))]), Ok(json!("Oslo")));
        // a record keeps its fields, one of them named like a parameter
        assert_eq!(payload("Svc::pair", &[("id", Some("3")), ("n", Some("4"))]), Ok(json!({"id": 3, "n": 4})));
    }

    #[test]
    fn a_decimal_parameter_keeps_every_digit() {
        // a double carries it exactly: the JSON number `--json` of the same text sends
        assert_eq!(payload("Svc::price", &[("p", Some("97000.50"))]), Ok(json!(97000.5)));
        assert_eq!(payload("Svc::price", &[("p", Some("-2"))]), Ok(json!(-2)));
        assert_eq!(payload("Svc::price", &[("p", Some("1.5e1"))]), Ok(json!(15.0)));
        // one no double carries: its digits in a string, which the decoder reads exactly
        assert_eq!(payload("Svc::price", &[("p", Some("12345678901234567.89"))]), Ok(json!("12345678901234567.89")));
        // a field tagged `string` is always its digits in a string
        assert_eq!(payload("Svc::quoted", &[("p", Some("2.5"))]), Ok(json!("2.5")));
        for (bad, why) in [("1.234", "past the scale of 2"), ("+1", "not a decimal"), (".5", "not a decimal"), ("01", "not a decimal"), ("1.", "not a decimal"), ("1e-3", "past the scale"), ("abc", "not a decimal")] {
            let e = payload("Svc::price", &[("p", Some(bad))]).unwrap_err();
            assert!(e.contains(why) && e.contains("the parameter is Float (Decimal<2>)"), "{bad}: {e}");
        }
        assert_eq!(payload("Svc::price", &[("p", Some("1.2300"))]), Ok(json!(1.23)));
        // an exponent no i64 holds is no number of the scale, never an overflow
        assert!(payload("Svc::price", &[("p", Some("1e-9223372036854775808"))]).unwrap_err().contains("past the scale"));
        assert_eq!(payload("Svc::price", &[("p", Some("1e9223372036854775807"))]), Ok(json!("1e9223372036854775807")));
    }

    #[test]
    fn a_bad_parameter_value_names_the_type_it_needs() {
        for (member, flag, bad, why) in [
            ("Svc::submit", "id", "x", "`x` is not an integer; the parameter is Int"),
            ("Svc::ratio", "x", "fast", "`fast` is not a number; the parameter is Float"),
            ("Svc::flag", "on", "yes", "`yes` is not true or false; the parameter is Bool"),
            ("Svc::wait", "d", "1.5s", "`1.5s` is not an integer; the parameter is Int (Duration, q(ns))"),
            ("Svc::paint", "c", "Pink", "`Pink` is not one of Red, Green, Blue; the parameter is Color"),
            ("Svc::stamp", "at", "2026-10-12", "is not an RFC 3339 instant in UTC"),
            ("Svc::stamp", "at", "2026-10-12T03:04:05+01:00", "is not an RFC 3339 instant in UTC"),
            ("Svc::stamp", "at", "2026-10-12T03:04:05.1234567891Z", "is not an RFC 3339 instant in UTC"),
        ] {
            let e = payload(member, &[(flag, Some(bad))]).unwrap_err();
            assert!(e.starts_with(&format!("`--{flag}`: ")) && e.contains(why), "{member} {bad}: {e}");
        }
    }

    #[test]
    fn a_scalar_request_given_wrong_says_how_to_give_it() {
        let usage = "Svc::submit takes one Int, `id`: pass `--id <int>`, or its JSON with --json";
        assert_eq!(payload("Svc::submit", &[]), Err(usage.to_string()));
        assert_eq!(payload("Svc::submit", &[("n", Some("7"))]), Err(format!("`--n` is not a parameter of Svc::submit; {usage}")));
        assert_eq!(payload("Svc::submit", &[("id", Some("1")), ("id", Some("2"))]), Err(format!("`--id` is given twice; {usage}")));
        assert_eq!(payload("Svc::submit", &[("id", Some("1")), ("n", Some("2"))]), Err(format!("`--n` is not a parameter of Svc::submit; {usage}")));
        assert_eq!(
            payload("Svc::paint", &[]),
            Err("Svc::paint takes one Color, `c`: pass `--c <Red|Green|Blue>`, or its JSON with --json".to_string())
        );
        assert_eq!(
            payload("Svc::wait", &[]),
            Err("Svc::wait takes one Int (Duration, q(ns)), `d`: pass `--d <int, a count of ns>`, or its JSON with --json".to_string())
        );
        assert_eq!(payload("Svc::ship", &[]), Err("Svc::ship takes one String, `ship_to`: pass `--ship-to <text>`, or its JSON with --json".to_string()));
        // a description served before the parameter was named
        assert_eq!(
            payload("Svc::old", &[("id", Some("7"))]),
            Err("`--id` is not a parameter of Svc::old; Svc::old takes one Int, and its description names no parameter: pass its JSON with --json".to_string())
        );
    }
}

//! `hale api describe` and `hale api call`: the operator verbs that drive any
//! served Hale program from the description it serves (spec/api.md §
//! Driving a served program). Nothing is generated and nothing lives in the
//! program: the verb reads the caller's description, types the payload by the
//! member's schema, and speaks the wire of § The `Rpc` interface.
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

use std::process::ExitCode;

use serde_json::{json, Value};

use crate::api_client::{bearer, http_request, raw_field, unix_exchange};

const EXIT_OK: u8 = 0;
const EXIT_REFUSED: u8 = 2;
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
    let base = match schema.get("type").and_then(Value::as_str) {
        Some("integer") => "Int".to_string(),
        Some("number") => "Float".to_string(),
        Some("boolean") => "Bool".to_string(),
        Some("string") => "String".to_string(),
        Some("array") => format!("[{}]", schema.get("items").map_or("JSON".to_string(), |i| type_label(doc, i))),
        Some("object") => "{record}".to_string(),
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
        let fields: Vec<String> = payload_fields(doc, m)
            .iter()
            .map(|(n, sc, req)| format!("{n}{}: {}", if *req { "" } else { "?" }, type_label(doc, sc)))
            .collect();
        let result = m.get("response").map_or("?".to_string(), |r| type_label(doc, r));
        let error = match m.get("error") {
            Some(Value::String(e)) => format!(", error {e}"),
            Some(e @ Value::Object(_)) => format!(", error {}", type_label(doc, e)),
            _ => String::new(),
        };
        let req = requires_of(m);
        out.push_str(&format!("  {name}({}) -> {result}{error}  requires: {}\n", fields.join(", "), if req.is_empty() { "-".to_string() } else { req.join(", ") }));
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
}

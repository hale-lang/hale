//! An OpenAPI document or an MCP tool list as an imported surface: rows,
//! records and enums, rendered as the Hale source `hale api import` writes
//! (spec/api.md § Imported surfaces).
//!
//! The mapping is the smallest one the api codec makes honest. A JSON
//! Schema object with properties is a record, a string enum whose values
//! are all names is an enum, `integer`, `number`, `boolean` and `string`
//! are `Int`, `Float`, `Bool` and `String`, an array with a `maxItems` is
//! a `bounded` list, and a property the schema does not require (or allows
//! to be null) is a presence pair. Everything else the codec cannot carry
//! (a union, a map, an array with no bound, a recursive shape, a schema
//! that declares no type) is `std::json::Value`, with a comment naming why.
//! The mapping is a pure function of the document: two runs over one
//! document write the same bytes, which is what the drift check compares.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

/// What kind of document a surface was read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    OpenApi,
    Mcp,
}

impl Kind {
    pub fn word(self) -> &'static str {
        match self {
            Kind::OpenApi => "openapi",
            Kind::Mcp => "mcp",
        }
    }
}

/// A type a row or a field names.
#[derive(Debug, Clone, PartialEq)]
enum Ty {
    Int,
    Float,
    Bool,
    Str,
    Named(String),
    Bounded(Box<Ty>, u64),
    /// `std::json::Value`, and why the codec could carry nothing narrower.
    Value(String),
}

impl Ty {
    fn text(&self) -> String {
        match self {
            Ty::Int => "Int".into(),
            Ty::Float => "Float".into(),
            Ty::Bool => "Bool".into(),
            Ty::Str => "String".into(),
            Ty::Named(n) => n.clone(),
            Ty::Bounded(t, n) => format!("bounded[{}; {n}]", t.text()),
            Ty::Value(_) => "std::json::Value".into(),
        }
    }

    /// The literal a field of the type defaults to when it is optional, for
    /// a scalar.
    fn zero(&self) -> Option<&'static str> {
        match self {
            Ty::Int => Some("0"),
            Ty::Float => Some("0.0"),
            Ty::Bool => Some("false"),
            Ty::Str => Some("\"\""),
            _ => None,
        }
    }

    fn why(&self) -> Option<&str> {
        match self {
            Ty::Value(w) => Some(w),
            Ty::Bounded(t, _) => t.why(),
            _ => None,
        }
    }
}

struct Field {
    name: String,
    key: String,
    ty: Ty,
    optional: bool,
}

enum Decl {
    Record { name: String, fields: Vec<Field> },
    Enum { name: String, variants: Vec<String> },
}

struct Row {
    name: String,
    wire: String,
    request: Option<Ty>,
    response: Option<Ty>,
}

/// An imported surface, mapped.
pub struct Surface {
    pub name: String,
    pub kind: Kind,
    pub location: String,
    /// What the document says it is (its title and version), for the
    /// header.
    pub about: String,
    rows: Vec<Row>,
    decls: Vec<Decl>,
}

// ---- names ----

/// The words of a name: split at anything that is no letter or digit, and
/// where a lower-case letter or digit meets an upper-case one
/// (`listPets`, `HTTPServer`).
fn words(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    for piece in s.split(|c: char| !c.is_ascii_alphanumeric()) {
        let cs: Vec<char> = piece.chars().collect();
        let mut cur = String::new();
        for (i, &c) in cs.iter().enumerate() {
            let boundary = i > 0
                && c.is_ascii_uppercase()
                && (cs[i - 1].is_ascii_lowercase()
                    || cs[i - 1].is_ascii_digit()
                    || (cs[i - 1].is_ascii_uppercase() && cs.get(i + 1).is_some_and(|n| n.is_ascii_lowercase())));
            if boundary && !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            cur.push(c);
        }
        if !cur.is_empty() {
            out.push(cur);
        }
    }
    out
}

/// Whether `s` reads as one identifier of the language, and is no word it
/// reserves.
fn is_name(s: &str) -> bool {
    let mut cs = s.chars();
    let Some(first) = cs.next() else { return false };
    if !(first.is_ascii_alphabetic()) || !cs.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return false;
    }
    if hale_syntax::keywords::HARD_KEYWORDS.contains(&s) || matches!(s, "true" | "false" | "Self") {
        return false;
    }
    matches!(hale_syntax::lex(s).as_deref(), Ok([t, ..]) if matches!(&t.kind, hale_syntax::TokenKind::Ident(n) if n == s))
}

/// `listPets` as a field or row name: `list_pets`.
fn snake(s: &str, fallback: &str) -> String {
    let mut n = words(s)
        .iter()
        .map(|w| w.to_ascii_lowercase())
        .collect::<Vec<_>>()
        .join("_");
    if n.is_empty() {
        n = fallback.to_string();
    }
    if n.starts_with(|c: char| c.is_ascii_digit()) {
        n = format!("n{n}");
    }
    if !is_name(&n) {
        n.push('_');
    }
    n
}

/// `pet-status` as a type name: `PetStatus`.
fn camel(s: &str) -> String {
    words(s)
        .iter()
        .map(|w| {
            let mut cs = w.chars();
            match cs.next() {
                Some(f) => f.to_ascii_uppercase().to_string() + &cs.as_str().to_ascii_lowercase(),
                None => String::new(),
            }
        })
        .collect()
}

/// `name`, or `name_2`, `name_3`, … : the first `taken` does not hold.
fn fresh(name: String, taken: &mut BTreeSet<String>, sep: &str) -> String {
    let mut n = name.clone();
    let mut i = 2;
    while !taken.insert(n.clone()) {
        n = format!("{name}{sep}{i}");
        i += 1;
    }
    n
}

// ---- schemas ----

struct Mapper<'d> {
    doc: &'d Value,
    prefix: String,
    decls: Vec<Decl>,
    type_names: BTreeSet<String>,
    /// A `$ref` already mapped, and its type.
    refs: BTreeMap<String, Ty>,
    /// The `$ref`s being mapped: one reached again is a recursive shape.
    open: BTreeSet<String>,
}

/// The largest `maxItems` mapped to a `bounded` list.
const MAX_BOUND: u64 = 1024;

impl<'d> Mapper<'d> {
    fn new(doc: &'d Value, prefix: &str) -> Self {
        Mapper {
            doc,
            prefix: camel(prefix),
            decls: Vec::new(),
            type_names: BTreeSet::new(),
            refs: BTreeMap::new(),
            open: BTreeSet::new(),
        }
    }

    fn type_name(&mut self, hint: &str) -> String {
        let base = format!("{}{}", self.prefix, camel(hint));
        fresh(base, &mut self.type_names, "")
    }

    /// A local `$ref`'s target.
    fn deref(&self, r: &str) -> Option<&'d Value> {
        let pointer = r.strip_prefix('#')?;
        self.doc.pointer(pointer)
    }

    /// A schema with any `$ref` at its top followed.
    fn resolved(&self, schema: &'d Value) -> &'d Value {
        let mut s = schema;
        for _ in 0..32 {
            match s.get("$ref").and_then(Value::as_str).and_then(|r| self.deref(r)) {
                Some(t) => s = t,
                None => break,
            }
        }
        s
    }

    /// Whether a schema admits `null`.
    fn nullable(&self, schema: &'d Value) -> bool {
        let s = self.resolved(schema);
        s.get("nullable").and_then(Value::as_bool) == Some(true)
            || s.get("type")
                .and_then(Value::as_array)
                .is_some_and(|ts| ts.iter().any(|t| t == "null"))
    }

    /// The type of `schema`, a record or an enum it declares named after
    /// `hint`.
    fn ty(&mut self, schema: &'d Value, hint: &str) -> Ty {
        if let Some(r) = schema.get("$ref").and_then(Value::as_str) {
            if let Some(t) = self.refs.get(r) {
                return t.clone();
            }
            let Some(target) = self.deref(r) else {
                return Ty::Value(format!("`{r}` is a reference outside the document"));
            };
            if self.open.contains(r) {
                return Ty::Value(format!("`{r}` is a recursive shape, and a record cannot hold itself"));
            }
            self.open.insert(r.to_string());
            let name = r.rsplit('/').next().unwrap_or(hint).to_string();
            let t = self.ty(target, &name);
            self.open.remove(r);
            self.refs.insert(r.to_string(), t.clone());
            return t;
        }
        if let Some(all) = schema.get("allOf").and_then(Value::as_array) {
            if let [one] = all.as_slice() {
                return self.ty(one, hint);
            }
            return Ty::Value("`allOf` of several schemas: the codec has no intersection of shapes".into());
        }
        for union in ["oneOf", "anyOf"] {
            if schema.get(union).is_some() {
                return Ty::Value(format!("`{union}`: the codec has no union of shapes"));
            }
        }
        let ty = match schema.get("type") {
            Some(Value::String(t)) => Some(t.as_str()),
            Some(Value::Array(ts)) => {
                let named: Vec<&str> = ts.iter().filter_map(Value::as_str).filter(|t| *t != "null").collect();
                match named.as_slice() {
                    [one] => Some(*one),
                    _ => return Ty::Value("a value of several types: the codec has no union of shapes".into()),
                }
            }
            _ if schema.get("properties").is_some() => Some("object"),
            _ => None,
        };
        match ty {
            Some("integer") => Ty::Int,
            Some("number") => Ty::Float,
            Some("boolean") => Ty::Bool,
            Some("string") => self.string(schema, hint),
            Some("array") => self.array(schema, hint),
            Some("object") => self.object(schema, hint),
            Some(other) => Ty::Value(format!("a schema of type `{other}`, which the codec does not carry")),
            None => Ty::Value("a schema that declares no type".into()),
        }
    }

    fn string(&mut self, schema: &'d Value, hint: &str) -> Ty {
        let Some(values) = schema.get("enum").and_then(Value::as_array) else {
            return Ty::Str;
        };
        let names: Vec<&str> = values.iter().filter_map(Value::as_str).collect();
        let mut seen = BTreeSet::new();
        if names.is_empty() || names.len() != values.len() || !names.iter().all(|n| is_name(n) && seen.insert(*n)) {
            return Ty::Str;
        }
        let name = self.type_name(hint);
        self.decls.push(Decl::Enum {
            name: name.clone(),
            variants: names.iter().map(|s| s.to_string()).collect(),
        });
        Ty::Named(name)
    }

    fn array(&mut self, schema: &'d Value, hint: &str) -> Ty {
        let items = schema.get("items").unwrap_or(&Value::Null);
        let elem = if items.is_object() {
            self.ty(items, &format!("{hint} item"))
        } else {
            Ty::Value("an array whose items declare no schema".into())
        };
        if matches!(elem, Ty::Bounded(..)) {
            return Ty::Value("a list of lists, which the codec does not carry".into());
        }
        match schema.get("maxItems").and_then(Value::as_u64) {
            Some(n @ 1..=MAX_BOUND) => Ty::Bounded(Box::new(elem), n),
            Some(n) if n > MAX_BOUND => Ty::Value(format!(
                "an array of up to {n} items, more than the {MAX_BOUND} a bounded list is imported as"
            )),
            _ => Ty::Value("an array with no `maxItems`, and a record holds a bounded list only".into()),
        }
    }

    fn object(&mut self, schema: &'d Value, hint: &str) -> Ty {
        let props = match schema.get("properties").and_then(Value::as_object) {
            Some(p) if !p.is_empty() => p,
            _ if schema
                .get("additionalProperties")
                .is_some_and(|a| a.is_object() || a == true) =>
            {
                return Ty::Value("a map with no declared properties".into());
            }
            _ => return Ty::Value("an object with no declared properties".into()),
        };
        let name = self.type_name(hint);
        let required: BTreeSet<&str> = schema
            .get("required")
            .and_then(Value::as_array)
            .map(|r| r.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let mut taken = BTreeSet::new();
        let mut fields = Vec::new();
        for (key, prop) in props {
            // a nested record is named after its holder and its key, the
            // surface's prefix written once
            let hint = format!("{} {key}", name.strip_prefix(self.prefix.as_str()).unwrap_or(&name));
            let ty = self.ty(prop, &hint);
            let optional = !required.contains(key.as_str()) || self.nullable(prop);
            fields.push((key.clone(), ty, optional));
        }
        let fields = name_fields(fields, &mut taken);
        self.decls.push(Decl::Record {
            name: name.clone(),
            fields,
        });
        Ty::Named(name)
    }

    /// A record of the named values `parts` (key, schema, required), named
    /// after `hint`: an operation's parameters.
    fn record_of(&mut self, parts: Vec<(String, &'d Value, bool)>, hint: &str, extra: Option<(String, Ty)>) -> Ty {
        let name = self.type_name(hint);
        let mut taken = BTreeSet::new();
        let mut fields = Vec::new();
        for (key, schema, required) in parts {
            let ty = self.ty(schema, &format!("{hint} {key}"));
            let optional = !required || self.nullable(schema);
            fields.push((key, ty, optional));
        }
        if let Some((key, ty)) = extra {
            fields.push((key, ty, false));
        }
        let fields = name_fields(fields, &mut taken);
        self.decls.push(Decl::Record {
            name: name.clone(),
            fields,
        });
        Ty::Named(name)
    }
}

/// Fields from their keys: each a name of its own, its `has_` flag's
/// name kept free beside an optional one.
fn name_fields(fields: Vec<(String, Ty, bool)>, taken: &mut BTreeSet<String>) -> Vec<Field> {
    // the plain names first, so a key keeps its own name before a flag takes it
    let mut out: Vec<Field> = Vec::new();
    for (key, ty, optional) in fields {
        let mut name = fresh(snake(&key, "field"), taken, "_");
        if optional {
            while !taken.insert(format!("has_{name}")) {
                taken.remove(&name);
                name = fresh(format!("{name}_"), taken, "_");
            }
        }
        out.push(Field {
            name,
            key,
            ty,
            optional,
        });
    }
    out
}

// ---- documents ----

/// A row's whole request or response: a list crosses as a record's field
/// only, so a whole one is free-form.
fn whole(t: Ty) -> Ty {
    match t {
        Ty::Bounded(..) => {
            Ty::Value("a list as the whole value, which the codec carries as a record's field only".into())
        }
        t => t,
    }
}

const METHODS: [&str; 8] = ["get", "put", "post", "delete", "options", "head", "patch", "trace"];

/// An OpenAPI 3 document as the surface `name`.
pub fn from_openapi(doc: &Value, name: &str, location: &str) -> Result<Surface, String> {
    if doc.get("swagger").is_some() {
        return Err("the document is Swagger 2.0, and the import reads OpenAPI 3".into());
    }
    let version = doc
        .get("openapi")
        .and_then(Value::as_str)
        .ok_or("the document is no OpenAPI document: it has no `openapi` version")?;
    if !version.starts_with('3') {
        return Err(format!(
            "the document is OpenAPI {version}, and the import reads OpenAPI 3"
        ));
    }
    let paths = doc
        .get("paths")
        .and_then(Value::as_object)
        .ok_or("the document has no `paths`")?;
    let mut m = Mapper::new(doc, name);
    let mut row_names = BTreeSet::new();
    let mut rows = Vec::new();
    for (path, item) in paths {
        let item = m.resolved(item);
        let shared: Vec<&Value> = item
            .get("parameters")
            .and_then(Value::as_array)
            .map(|a| a.iter().collect())
            .unwrap_or_default();
        for method in METHODS {
            let Some(op) = item.get(method).filter(|o| o.is_object()) else {
                continue;
            };
            let wire = format!("{} {path}", method.to_ascii_uppercase());
            let derived = format!("{method} {path}");
            let base = match op.get("operationId").and_then(Value::as_str) {
                Some(id) if !id.is_empty() => snake(id, "op"),
                _ => snake(&derived, "op"),
            };
            let row = fresh(base, &mut row_names, "_");
            // the parameters an operation's request carries: path and query,
            // the operation's own replacing the path item's of one name
            let mut params: Vec<(String, &Value, bool)> = Vec::new();
            let own: Vec<&Value> = op
                .get("parameters")
                .and_then(Value::as_array)
                .map(|a| a.iter().collect())
                .unwrap_or_default();
            let mut seen = BTreeSet::new();
            for p in own.iter().chain(shared.iter()) {
                let p = m.resolved(p);
                let (Some(pname), Some(place)) = (
                    p.get("name").and_then(Value::as_str),
                    p.get("in").and_then(Value::as_str),
                ) else {
                    continue;
                };
                if !matches!(place, "path" | "query") || !seen.insert((pname.to_string(), place.to_string())) {
                    continue;
                }
                let required = place == "path" || p.get("required").and_then(Value::as_bool) == Some(true);
                let schema = p.get("schema").unwrap_or(&Value::Null);
                params.push((pname.to_string(), schema, required));
            }
            params.sort_by(|a, b| a.0.cmp(&b.0));
            let body = op.get("requestBody").map(|b| json_content(m.resolved(b)));
            let hint = format!("{row} request");
            let request = match (params.is_empty(), body) {
                (true, None) => None,
                (true, Some(Some(schema))) => Some(m.ty(schema, &format!("{row} body"))),
                (true, Some(None)) => Some(Ty::Value("a body that is not `application/json`".into())),
                (false, None) => Some(m.record_of(params, &hint, None)),
                (false, Some(schema)) => {
                    let body_ty = match schema {
                        Some(s) => m.ty(s, &format!("{row} body")),
                        None => Ty::Value("a body that is not `application/json`".into()),
                    };
                    // the body rides in a field of its own beside the parameters
                    Some(m.record_of(params, &hint, Some(("body".to_string(), body_ty))))
                }
            };
            let response = success_schema(&m, op).map(|s| m.ty(s, &format!("{row} response")));
            rows.push(Row {
                name: row,
                wire,
                request: request.map(whole),
                response: response.map(whole),
            });
        }
    }
    let info = doc.get("info");
    let title = info.and_then(|i| i.get("title")).and_then(Value::as_str).unwrap_or("");
    let ver = info
        .and_then(|i| i.get("version"))
        .and_then(Value::as_str)
        .unwrap_or("");
    Ok(finish(
        m,
        name,
        Kind::OpenApi,
        location,
        format!("OpenAPI {version}: {title} {ver}"),
        rows,
    ))
}

/// The schema of a body's `application/json` content (or any JSON media
/// type), if it has one.
fn json_content(b: &Value) -> Option<&Value> {
    let content = b.get("content")?.as_object()?;
    content
        .iter()
        .find(|(k, _)| k.as_str() == "application/json")
        .or_else(|| {
            content
                .iter()
                .find(|(k, _)| k.ends_with("+json") || k.ends_with("/json"))
        })
        .and_then(|(_, v)| v.get("schema"))
}

/// The schema of an operation's lowest-numbered 2xx response with JSON
/// content.
fn success_schema<'d>(m: &Mapper<'d>, op: &'d Value) -> Option<&'d Value> {
    let responses = op.get("responses")?.as_object()?;
    let mut ok: Vec<(&String, &Value)> = responses.iter().filter(|(k, _)| k.starts_with('2')).collect();
    ok.sort_by(|a, b| a.0.cmp(b.0));
    ok.into_iter().find_map(|(_, r)| json_content(m.resolved(r)))
}

/// An MCP endpoint's tools as the surface `name`: a row per tool, its
/// request the tool's `inputSchema`, its response the `outputSchema` or
/// free-form.
pub fn from_mcp(tools: &[Value], server: &str, name: &str, location: &str) -> Result<Surface, String> {
    let doc = Value::Null;
    let mut m = Mapper::new(&doc, name);
    let mut sorted: Vec<&Value> = tools.iter().collect();
    sorted.sort_by_key(|t| t.get("name").and_then(Value::as_str).unwrap_or("").to_string());
    let mut row_names = BTreeSet::new();
    let mut rows = Vec::new();
    // each tool's schemas are their own documents: a `$ref` resolves in it
    let owned: Vec<(String, Value, Option<Value>)> = sorted
        .iter()
        .filter_map(|t| {
            let n = t.get("name").and_then(Value::as_str)?;
            Some((
                n.to_string(),
                t.get("inputSchema").cloned().unwrap_or(Value::Null),
                t.get("outputSchema").cloned(),
            ))
        })
        .collect();
    for (tool, input, output) in &owned {
        let row = fresh(snake(tool, "tool"), &mut row_names, "_");
        let mut tm = Mapper::new(input, name);
        tm.type_names = std::mem::take(&mut m.type_names);
        let declares = input
            .get("properties")
            .and_then(Value::as_object)
            .is_some_and(|p| !p.is_empty());
        let request = if declares {
            Some(tm.ty(input, &format!("{row} input")))
        } else if input
            .get("additionalProperties")
            .is_some_and(|a| a.is_object() || a == true)
        {
            Some(Ty::Value("a map with no declared properties".into()))
        } else {
            None
        };
        m.type_names = std::mem::take(&mut tm.type_names);
        m.decls.append(&mut tm.decls);
        let response = match output {
            Some(out) => {
                let mut om = Mapper::new(out, name);
                om.type_names = std::mem::take(&mut m.type_names);
                let t = om.ty(out, &format!("{row} output"));
                m.type_names = std::mem::take(&mut om.type_names);
                m.decls.append(&mut om.decls);
                t
            }
            None => Ty::Value("the tool declares no `outputSchema`".into()),
        };
        rows.push(Row {
            name: row,
            wire: tool.clone(),
            request: request.map(whole),
            response: Some(whole(response)),
        });
    }
    Ok(finish(
        m,
        name,
        Kind::Mcp,
        location,
        format!("MCP: {server}").trim_end_matches(": ").to_string(),
        rows,
    ))
}

fn finish(m: Mapper<'_>, name: &str, kind: Kind, location: &str, about: String, mut rows: Vec<Row>) -> Surface {
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    let mut decls = m.decls;
    decls.sort_by(|a, b| decl_name(a).cmp(decl_name(b)));
    Surface {
        name: name.to_string(),
        kind,
        location: location.to_string(),
        about: about.trim().to_string(),
        rows,
        decls,
    }
}

fn decl_name(d: &Decl) -> &str {
    match d {
        Decl::Record { name, .. } | Decl::Enum { name, .. } => name,
    }
}

// ---- the source ----

fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// One line of a comment: never more than one line.
fn comment(s: &str) -> String {
    s.replace(['\n', '\r'], " ")
}

impl Surface {
    /// The surface as the Hale source the import writes: the header, the
    /// `api` block, then its records and enums by name.
    pub fn render(&self) -> String {
        let mut o = String::new();
        o.push_str(&format!(
            "// Generated by `hale api import` from {} {}{}.\n",
            self.kind.word(),
            comment(&self.location),
            if self.about.is_empty() {
                String::new()
            } else {
                format!(" ({})", comment(&self.about))
            }
        ));
        o.push_str(
            "// Re-run the import to change it; `hale api drift` names each row the source has changed since.\n\n",
        );
        o.push_str(&format!(
            "api {} from {} {} {{\n",
            self.name,
            self.kind.word(),
            quote(&self.location)
        ));
        for r in &self.rows {
            for (slot, t) in [("request", &r.request), ("response", &r.response)] {
                if let Some(why) = t.as_ref().and_then(Ty::why).filter(|w| !w.is_empty()) {
                    o.push_str(&format!("    // {slot}: std::json::Value, {}\n", comment(why)));
                }
            }
            o.push_str(&format!(
                "    rpc {}({}){} wire: {};\n",
                r.name,
                r.request.as_ref().map(Ty::text).unwrap_or_default(),
                r.response
                    .as_ref()
                    .map(|t| format!(" -> {}", t.text()))
                    .unwrap_or_default(),
                quote(&r.wire)
            ));
        }
        o.push_str("}\n");
        for d in &self.decls {
            o.push('\n');
            match d {
                Decl::Enum { name, variants } => {
                    o.push_str(&format!("type {name} = enum {{ {} }};\n", variants.join(", ")));
                }
                Decl::Record { name, fields } => {
                    o.push_str(&format!("type {name} {{\n"));
                    for f in fields {
                        if let Some(why) = f.ty.why().filter(|w| !w.is_empty()) {
                            o.push_str(&format!("    // std::json::Value: {}\n", comment(why)));
                        }
                        let tag = if f.name != f.key {
                            format!(" `json:{}`", quote_tag(&f.key))
                        } else {
                            String::new()
                        };
                        if f.optional {
                            o.push_str(&format!("    has_{}: Bool = false;\n", f.name));
                            match f.ty.zero() {
                                Some(z) => o.push_str(&format!("    {}: {} = {z}{tag};\n", f.name, f.ty.text())),
                                None => o.push_str(&format!("    {}: {}{tag};\n", f.name, f.ty.text())),
                            }
                        } else {
                            o.push_str(&format!("    {}: {}{tag};\n", f.name, f.ty.text()));
                        }
                    }
                    o.push_str("}\n");
                }
            }
        }
        o
    }

    pub fn row_count(&self) -> usize {
        self.rows.len()
    }
}

/// A `json:"key"` tag's value: the key in quotes, a quote or backslash in
/// it escaped.
fn quote_tag(k: &str) -> String {
    format!("\"{}\"", k.replace('\\', "\\\\").replace('"', "\\\"").replace('`', ""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_words_of_the_language() {
        assert_eq!(snake("listPets", "x"), "list_pets");
        assert_eq!(snake("get /pets/{petId}", "x"), "get_pets_pet_id");
        assert_eq!(snake("HTTPServer", "x"), "http_server");
        assert_eq!(snake("type", "x"), "type_");
        assert_eq!(snake("2fa", "x"), "n2fa");
        assert_eq!(snake("---", "x"), "x");
        assert_eq!(camel("pet-status"), "PetStatus");
        assert!(!is_name("in"));
        assert!(is_name("status"));
    }
}

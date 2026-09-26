//! GH #1107: the generic clients of an api binding, and the forms
//! its description takes.
//!
//! A program bound with `api: unix(...)` (spec/semantics.md § "The
//! api binding") serves one JSON object per line on a Unix socket
//! and answers `{"describe": true}` with its description: commands,
//! reads and streams with their schemas. Everything here reads only
//! that document. `hale describe` prints it (or its OpenAPI 3.1 or
//! MCP form), `hale call` sends a command or a read, `hale watch`
//! attaches to a stream, `hale admin` serves a local page over it,
//! and `hale mcp --app` (mcp.rs) turns it into tools and resources.
//! None of them knows a topic name in advance: the description is the
//! whole contract, and a client that presents a gate as a proof or a
//! read as a live view is misreading the two notes it carries.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::process::ExitCode;

use serde_json::{json, Map, Value};

/// One connection to a binding, with the line protocol on top.
pub struct Client {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    next_id: u64,
}

impl Client {
    pub fn connect(sock: &str) -> Result<Client, String> {
        let stream = UnixStream::connect(sock)
            .map_err(|e| format!("could not connect to {}: {}", sock, e))?;
        let writer = stream
            .try_clone()
            .map_err(|e| format!("could not clone the socket: {}", e))?;
        Ok(Client {
            reader: BufReader::new(stream),
            writer,
            next_id: 1,
        })
    }

    /// Send one request and return the answer carrying its id; frames
    /// that arrive in between are handed to `on_frame`.
    pub fn request(
        &mut self,
        mut req: Map<String, Value>,
        mut on_frame: impl FnMut(&Value),
    ) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        req.insert("id".to_string(), json!(id));
        let line = Value::Object(req).to_string();
        writeln!(self.writer, "{}", line).map_err(|e| format!("write failed: {}", e))?;
        self.writer.flush().map_err(|e| format!("write failed: {}", e))?;
        loop {
            let v = self.next_line()?.ok_or_else(|| "the binding closed the connection".to_string())?;
            if v.get("stream").is_some() {
                on_frame(&v);
                continue;
            }
            if v.get("id") == Some(&json!(id)) {
                return Ok(v);
            }
        }
    }

    /// The next line from the binding, `None` at end of stream.
    pub fn next_line(&mut self) -> Result<Option<Value>, String> {
        let mut line = String::new();
        loop {
            line.clear();
            let n = self
                .reader
                .read_line(&mut line)
                .map_err(|e| format!("read failed: {}", e))?;
            if n == 0 {
                return Ok(None);
            }
            let t = line.trim();
            if t.is_empty() {
                continue;
            }
            return serde_json::from_str(t)
                .map(Some)
                .map_err(|e| format!("the binding sent a line that is not JSON: {} ({})", t, e));
        }
    }

    pub fn describe(&mut self) -> Result<Value, String> {
        let raw = self.describe_raw()?;
        serde_json::from_str(&raw).map_err(|e| format!("the description is not JSON: {}", e))
    }

    /// The description as the binding wrote it, byte for byte: the
    /// `value` of the answer line, not a re-serialization of it.
    pub fn describe_raw(&mut self) -> Result<String, String> {
        let mut req = Map::new();
        req.insert("describe".to_string(), json!(true));
        let (ans, line) = self.request_line(req)?;
        answer_value(&ans)?;
        raw_field(&line, "value").ok_or_else(|| "the answer carries no value".to_string())
    }

    /// `request`, also returning the answer's raw line.
    pub fn request_line(&mut self, mut req: Map<String, Value>) -> Result<(Value, String), String> {
        let id = self.next_id;
        self.next_id += 1;
        req.insert("id".to_string(), json!(id));
        let line = Value::Object(req).to_string();
        writeln!(self.writer, "{}", line).map_err(|e| format!("write failed: {}", e))?;
        self.writer.flush().map_err(|e| format!("write failed: {}", e))?;
        loop {
            let raw = self.next_raw_line()?.ok_or_else(|| "the binding closed the connection".to_string())?;
            let v: Value = serde_json::from_str(&raw)
                .map_err(|e| format!("the binding sent a line that is not JSON: {} ({})", raw, e))?;
            if v.get("stream").is_some() {
                continue;
            }
            if v.get("id") == Some(&json!(id)) {
                return Ok((v, raw));
            }
        }
    }

    fn next_raw_line(&mut self) -> Result<Option<String>, String> {
        let mut line = String::new();
        loop {
            line.clear();
            let n = self
                .reader
                .read_line(&mut line)
                .map_err(|e| format!("read failed: {}", e))?;
            if n == 0 {
                return Ok(None);
            }
            if !line.trim().is_empty() {
                return Ok(Some(line.trim().to_string()));
            }
        }
    }
}

/// The raw text of a top-level field of one JSON object line, with
/// its bytes untouched (a value re-serialized through `Value` would
/// come back with sorted keys, and the description's order is part
/// of what a binding promises to serve).
pub fn raw_field(line: &str, key: &str) -> Option<String> {
    let needle = format!("\"{}\":", key);
    let bytes = line.as_bytes();
    // Find the key at depth 1, outside strings.
    let mut depth = 0i32;
    let mut in_str = false;
    let mut esc = false;
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if in_str {
            if esc {
                esc = false;
            } else if c == b'\\' {
                esc = true;
            } else if c == b'"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        match c {
            b'"' => {
                if depth == 1 && line[i..].starts_with(&needle) {
                    let start = i + needle.len();
                    return Some(raw_value(&line[start..]));
                }
                in_str = true;
            }
            b'{' | b'[' => depth += 1,
            b'}' | b']' => depth -= 1,
            _ => {}
        }
        i += 1;
    }
    None
}

/// The first complete JSON value at the head of `s`.
fn raw_value(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    let mut in_str = false;
    let mut esc = false;
    for (i, &c) in bytes.iter().enumerate() {
        if in_str {
            if esc {
                esc = false;
            } else if c == b'\\' {
                esc = true;
            } else if c == b'"' {
                in_str = false;
                if depth == 0 {
                    return s[..=i].to_string();
                }
            }
            continue;
        }
        match c {
            b'"' => in_str = true,
            b'{' | b'[' => depth += 1,
            b'}' | b']' => {
                depth -= 1;
                if depth == 0 {
                    return s[..=i].to_string();
                }
            }
            b',' if depth == 0 => return s[..i].to_string(),
            _ => {}
        }
    }
    s.to_string()
}

/// The `value` of an `ok` answer, or the refusal as an error line.
pub fn answer_value(ans: &Value) -> Result<Value, String> {
    if ans.get("ok") == Some(&json!(true)) {
        if let Some(v) = ans.get("value") {
            return Ok(v.clone());
        }
        if ans.get("accepted").is_some() {
            return Ok(json!({ "accepted": true }));
        }
        if let Some(a) = ans.get("attached") {
            return Ok(json!({ "attached": a }));
        }
        return Ok(Value::Null);
    }
    let kind = ans
        .pointer("/refusal/kind")
        .and_then(Value::as_str)
        .unwrap_or("refused");
    let reason = ans
        .pointer("/refusal/reason")
        .and_then(Value::as_str)
        .unwrap_or("");
    Err(format!("refused: {}: {}", kind, reason))
}

// ---- the description's forms ------------------------------------------

fn names(desc: &Value, table: &str) -> Vec<String> {
    desc.get(table)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|c| c.get("name").and_then(Value::as_str).map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// `#/schemas/T` references rewritten for a document that keeps the
/// schemas under another path.
fn rewrite_refs(v: &Value, base: &str) -> Value {
    match v {
        Value::Object(m) => Value::Object(
            m.iter()
                .map(|(k, x)| {
                    if k == "$ref" {
                        if let Some(s) = x.as_str() {
                            if let Some(t) = s.strip_prefix("#/schemas/") {
                                return (k.clone(), json!(format!("{}{}", base, t)));
                            }
                        }
                    }
                    (k.clone(), rewrite_refs(x, base))
                })
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(|x| rewrite_refs(x, base)).collect()),
        other => other.clone(),
    }
}

/// A reference to a named type under `base`, or the inline schema of
/// a scalar (`Int`, `Float`, `Bool`, `String` have no component).
fn type_schema(ty: &str, base: &str) -> Value {
    match ty {
        "Int" => json!({ "type": "integer" }),
        "Float" => json!({ "type": "number" }),
        "Bool" => json!({ "type": "boolean" }),
        "String" => json!({ "type": "string" }),
        other => json!({ "$ref": format!("{}{}", base, other) }),
    }
}

/// An MCP tool name: the topic's name with `::` (a cross-seed topic)
/// spelled `__`, inside the character set tool names allow.
pub fn tool_name(topic: &str) -> String {
    topic.replace("::", "__")
}

/// The OpenAPI 3.1 form: a `post /call/<name>` per command, a `get
/// /read/<name>` per read, a `get /watch/<name>` per stream, the
/// schemas under `components`, the refusal and the two notes.
pub fn openapi(desc: &Value) -> Value {
    let app = desc.get("app").and_then(Value::as_str).unwrap_or("app");
    let gates = desc.pointer("/notes/gates").and_then(Value::as_str).unwrap_or("");
    let reads_note = desc.pointer("/notes/reads").and_then(Value::as_str).unwrap_or("");
    let mut paths = Map::new();
    let refusal = json!({
        "description": "refused: the receipt names the kind (malformed, unknown, not_a_command, not_a_stream, over_bound, unauthorized) and the reason",
        "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Refusal" } } }
    });
    for c in desc.get("commands").and_then(Value::as_array).into_iter().flatten() {
        let name = c.get("name").and_then(Value::as_str).unwrap_or("");
        let payload = c.get("payload").and_then(Value::as_str).unwrap_or("");
        let reply = c.get("reply").and_then(Value::as_str);
        let ok_schema = match reply {
            Some(r) => type_schema(r, "#/components/schemas/"),
            None => json!({ "$ref": "#/components/schemas/Accepted" }),
        };
        let mut op = json!({
            "operationId": format!("call.{}", name),
            "summary": format!("command {}: publish a {} on subject {}", name, payload,
                c.get("subject").and_then(Value::as_str).unwrap_or("")),
            "requestBody": { "required": true, "content": { "application/json": {
                "schema": type_schema(payload, "#/components/schemas/") } } },
            "responses": {
                "200": { "description": match reply {
                    Some(r) => format!("the value {} the handler returned", r),
                    None => "accepted: dispatched to every born subscriber".to_string() },
                    "content": { "application/json": { "schema": ok_schema } } },
                "4XX": refusal.clone()
            }
        });
        if let Some(role) = c.get("role").and_then(Value::as_str) {
            op["security"] = json!([{ "role": [role] }]);
        }
        paths.insert(format!("/call/{}", name), json!({ "post": op }));
    }
    for r in desc.get("reads").and_then(Value::as_array).into_iter().flatten() {
        let name = r.get("name").and_then(Value::as_str).unwrap_or("");
        let ty = r.get("type").and_then(Value::as_str).unwrap_or("");
        let mut op = json!({
            "operationId": format!("read.{}", name),
            "summary": format!("read {}: a snapshot of a {}, with its as_of digest", name, ty),
            "responses": {
                "200": { "description": "a snapshot; never a live view", "content": { "application/json": {
                    "schema": { "type": "object", "properties": {
                        "value": type_schema(ty, "#/components/schemas/"),
                        "as_of": { "type": "string", "description": "sha256 digest of the answered value" } },
                        "required": ["value", "as_of"] } } } },
                "4XX": refusal.clone()
            }
        });
        if let Some(role) = r.get("role").and_then(Value::as_str) {
            op["security"] = json!([{ "role": [role] }]);
        }
        paths.insert(format!("/read/{}", name), json!({ "get": op }));
    }
    for st in desc.get("streams").and_then(Value::as_array).into_iter().flatten() {
        let name = st.get("name").and_then(Value::as_str).unwrap_or("");
        let payload = st.get("payload").and_then(Value::as_str).unwrap_or("");
        let mut op = json!({
            "operationId": format!("watch.{}", name),
            "summary": format!("stream {}: every {} published on subject {}, one JSON object per line after attach", name, payload,
                st.get("subject").and_then(Value::as_str).unwrap_or("")),
            "responses": {
                "200": { "description": "frames, one per line, for the life of the connection", "content": { "application/jsonl": {
                    "schema": { "type": "object", "properties": {
                        "stream": { "type": "string" },
                        "value": type_schema(payload, "#/components/schemas/"),
                        "dropped": { "type": "integer", "description": "frames the watcher's queue shed since the last one" } },
                        "required": ["stream"] } } } },
                "4XX": refusal.clone()
            }
        });
        if let Some(role) = st.get("role").and_then(Value::as_str) {
            op["security"] = json!([{ "role": [role] }]);
        }
        paths.insert(format!("/watch/{}", name), json!({ "get": op }));
    }
    let mut schemas = Map::new();
    if let Some(Value::Object(m)) = desc.get("schemas") {
        for (k, v) in m {
            schemas.insert(k.clone(), rewrite_refs(v, "#/components/schemas/"));
        }
    }
    schemas.insert("Refusal".to_string(), json!({
        "type": "object",
        "properties": { "kind": { "type": "string" }, "reason": { "type": "string" } },
        "required": ["kind", "reason"]
    }));
    schemas.insert("Accepted".to_string(), json!({
        "type": "object",
        "properties": { "accepted": { "type": "boolean", "const": true } },
        "required": ["accepted"]
    }));
    json!({
        "openapi": "3.1.0",
        "info": {
            "title": format!("{} api", app),
            "version": format!("hale-api/{}", desc.get("hale_api").and_then(Value::as_i64).unwrap_or(1)),
            "description": format!("Served by the program's api binding as one JSON object per line on a Unix socket. Gates: {}. Reads: {}.", gates, reads_note),
            "x-hale-notes": { "gates": gates, "reads": reads_note }
        },
        "servers": [{ "url": "unix:{socket}", "description": "the api binding's socket, a deployment choice; paths name the request verbs of its line protocol",
            "variables": { "socket": { "default": "/run/app.sock", "description": "where this copy of the program listens" } } }],
        "paths": Value::Object(paths),
        "components": {
            "schemas": Value::Object(schemas),
            "securitySchemes": { "role": {
                "type": "http", "scheme": "bearer",
                "description": format!("The principal the binding established: the peer's credentials on the Unix socket, a bearer token on HTTP. An operation's scopes are the roles the principal must hold. {}", gates) } }
        }
    })
}

/// A JSON Schema for one type with every nested type it reaches
/// inlined under `$defs`, self-contained as an MCP input schema must be.
fn self_contained_schema(desc: &Value, ty: &str) -> Value {
    let schemas = desc.get("schemas").and_then(Value::as_object);
    let mut root = schemas
        .and_then(|m| m.get(ty))
        .map(|v| rewrite_refs(v, "#/$defs/"))
        .unwrap_or_else(|| type_schema(ty, "#/$defs/"));
    // Collect every reachable nested type.
    let mut defs = Map::new();
    let mut todo: Vec<String> = Vec::new();
    fn collect(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::Object(m) => {
                if let Some(s) = m.get("$ref").and_then(Value::as_str) {
                    if let Some(t) = s.strip_prefix("#/$defs/") {
                        out.push(t.to_string());
                    }
                }
                for x in m.values() {
                    collect(x, out);
                }
            }
            Value::Array(a) => a.iter().for_each(|x| collect(x, out)),
            _ => {}
        }
    }
    collect(&root, &mut todo);
    while let Some(t) = todo.pop() {
        if defs.contains_key(&t) {
            continue;
        }
        let Some(s) = schemas.and_then(|m| m.get(&t)) else { continue };
        let s = rewrite_refs(s, "#/$defs/");
        collect(&s, &mut todo);
        defs.insert(t, s);
    }
    if !defs.is_empty() {
        if let Value::Object(m) = &mut root {
            m.insert("$defs".to_string(), Value::Object(defs));
        }
    }
    root
}

/// The MCP form: every command a tool, every read a resource; streams
/// have no MCP shape and are listed for a host that wants to say so.
pub fn mcp(desc: &Value) -> Value {
    let gates = desc.pointer("/notes/gates").and_then(Value::as_str).unwrap_or("");
    let mut tools = Vec::new();
    for c in desc.get("commands").and_then(Value::as_array).into_iter().flatten() {
        let name = c.get("name").and_then(Value::as_str).unwrap_or("");
        let payload = c.get("payload").and_then(Value::as_str).unwrap_or("");
        let reply = c.get("reply").and_then(Value::as_str);
        let role = c.get("role").and_then(Value::as_str);
        let mut d = format!(
            "Command {}: sends a {} to the program{}.",
            name,
            payload,
            match reply {
                Some(r) => format!(" and returns the {} its handler answers with", r),
                None => "; the answer is `accepted` once it is dispatched".to_string(),
            }
        );
        if let Some(r) = role {
            d.push_str(&format!(" Needs the role {}; {}.", r, gates));
        }
        tools.push(json!({
            "name": tool_name(name),
            "description": d,
            "inputSchema": self_contained_schema(desc, payload)
        }));
    }
    let mut resources = Vec::new();
    for r in desc.get("reads").and_then(Value::as_array).into_iter().flatten() {
        let name = r.get("name").and_then(Value::as_str).unwrap_or("");
        let ty = r.get("type").and_then(Value::as_str).unwrap_or("");
        resources.push(json!({
            "uri": format!("hale://read/{}", name),
            "name": name,
            "description": format!("Read {}: a snapshot of a {} with its as_of digest, never a live view.", name, ty),
            "mimeType": "application/json"
        }));
    }
    json!({
        "tools": tools,
        "resources": resources,
        "streams": names(desc, "streams"),
        "notes": desc.get("notes").cloned().unwrap_or(Value::Null)
    })
}

// ---- the verbs ------------------------------------------------------------

fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_default()
}

fn is_socket(path: &str) -> bool {
    use std::os::unix::fs::FileTypeExt;
    std::fs::metadata(path)
        .map(|m| m.file_type().is_socket())
        .unwrap_or(false)
}

/// `hale describe <socket | file.hl | dir> [--openapi | --mcp] [-o <path>]`.
pub fn run_describe(rest: &[String]) -> ExitCode {
    let mut form = "native";
    let mut out: Option<String> = None;
    let mut target: Option<String> = None;
    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            "--openapi" => {
                form = "openapi";
                i += 1;
            }
            "--mcp" => {
                form = "mcp";
                i += 1;
            }
            "-o" | "--out" => match rest.get(i + 1) {
                Some(v) => {
                    out = Some(v.clone());
                    i += 2;
                }
                None => {
                    eprintln!("hale describe: {} requires a path", rest[i]);
                    return ExitCode::from(2);
                }
            },
            other if other.starts_with('-') => {
                eprintln!("hale describe: unknown flag {}", other);
                return ExitCode::from(2);
            }
            other => {
                target = Some(other.to_string());
                i += 1;
            }
        }
    }
    let Some(target) = target else {
        eprintln!("usage: hale describe <socket | file.hl | dir> [--openapi | --mcp] [-o <path>]");
        return ExitCode::from(2);
    };
    let raw = match description_raw_of(&target) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("hale describe: {}", e);
            return ExitCode::from(1);
        }
    };
    // The native form is the binding's own bytes, never re-serialized:
    // spec/model.md promises the served and the emitted document agree
    // byte for byte, and a `Value` round trip would sort the keys.
    let text = if form == "native" {
        raw.trim().to_string() + "\n"
    } else {
        let desc: Value = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("hale describe: the description is not JSON: {}", e);
                return ExitCode::from(1);
            }
        };
        let doc = if form == "openapi" { openapi(&desc) } else { mcp(&desc) };
        pretty(&doc) + "\n"
    };
    match out {
        Some(path) => {
            if let Err(e) = std::fs::write(&path, text) {
                eprintln!("hale describe: could not write {}: {}", path, e);
                return ExitCode::from(2);
            }
        }
        None => print!("{}", text),
    }
    ExitCode::SUCCESS
}

/// The description of a running binding (a socket path) or of a
/// program (`hale check --dump-api`, self-exec'd so the two spellings
/// cannot drift), as bytes.
pub fn description_raw_of(target: &str) -> Result<String, String> {
    if is_socket(target) {
        return Client::connect(target)?.describe_raw();
    }
    let me = std::env::current_exe().map_err(|e| format!("current exe: {}", e))?;
    let out = std::process::Command::new(me)
        .args(["check", target, "--dump-api"])
        .output()
        .map_err(|e| format!("could not run hale check: {}", e))?;
    if !out.status.success() {
        return Err(format!(
            "{} does not describe an api binding:\n{}",
            target,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    // The description is the first line; `hale check` reports its own
    // verdict after it.
    let text = String::from_utf8_lossy(&out.stdout);
    let t = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
    if t.is_empty() || !t.starts_with('{') {
        return Err(format!(
            "{} has no `api:` entry on its main locus (add one, or run it under `hale run --api`)",
            target
        ));
    }
    Ok(t.to_string())
}

/// `hale call <socket> <name> [<json>]`: a command (its payload, `{}`
/// when omitted) or a read, decided by the description.
pub fn run_call(rest: &[String]) -> ExitCode {
    let mut receipt = false;
    let rest: Vec<String> = rest
        .iter()
        .filter(|a| {
            if *a == "--receipt" {
                receipt = true;
                false
            } else {
                true
            }
        })
        .cloned()
        .collect();
    let (sock, name, body) = match rest.as_slice() {
        [s, n] => (s, n, "{}".to_string()),
        [s, n, b] => (s, n, b.clone()),
        _ => {
            eprintln!("usage: hale call <socket> <command-or-read> [<json payload>] [--receipt]");
            return ExitCode::from(2);
        }
    };
    let mut client = match Client::connect(sock) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("hale call: {}", e);
            return ExitCode::from(1);
        }
    };
    let desc = match client.describe() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("hale call: {}", e);
            return ExitCode::from(1);
        }
    };
    let commands = names(&desc, "commands");
    let reads = names(&desc, "reads");
    let mut req = Map::new();
    if commands.iter().any(|c| c == name) {
        let payload: Value = match serde_json::from_str(&body) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("hale call: the payload is not JSON: {}", e);
                return ExitCode::from(2);
            }
        };
        req.insert("call".to_string(), json!(name));
        req.insert("payload".to_string(), payload);
    } else if reads.iter().any(|r| r == name) {
        req.insert("read".to_string(), json!(name));
    } else {
        eprintln!(
            "hale call: `{}` is neither a command nor a read of this program\n  commands: {}\n  reads: {}\n  streams (use `hale watch`): {}",
            name,
            commands.join(", "),
            reads.join(", "),
            names(&desc, "streams").join(", ")
        );
        return ExitCode::from(1);
    }
    let (ans, line) = match client.request_line(req) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("hale call: {}", e);
            return ExitCode::from(1);
        }
    };
    if receipt {
        // The whole receipt, as the binding wrote it: request_id,
        // the echoed id, the value or refusal, as_of, and whatever
        // later pieces add (the caller, the authorizing role).
        println!("{}", line);
        return if ans.get("ok") == Some(&json!(true)) { ExitCode::SUCCESS } else { ExitCode::from(1) };
    }
    match answer_value(&ans) {
        Ok(v) => {
            let mut shown = v;
            if let Some(as_of) = ans.get("as_of") {
                shown = json!({ "value": shown, "as_of": as_of });
            }
            println!("{}", pretty(&shown));
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("hale call: {}", e);
            eprintln!("{}", line);
            ExitCode::from(1)
        }
    }
}

/// `hale watch <socket> <stream>`: attach and print frames, one per
/// line, until the binding closes the connection.
pub fn run_watch(rest: &[String]) -> ExitCode {
    let [sock, name] = rest else {
        eprintln!("usage: hale watch <socket> <stream>");
        return ExitCode::from(2);
    };
    let mut client = match Client::connect(sock) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("hale watch: {}", e);
            return ExitCode::from(1);
        }
    };
    let mut req = Map::new();
    req.insert("watch".to_string(), json!(name));
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let ans = match client.request(req, |f| {
        let _ = writeln!(out, "{}", f);
    }) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("hale watch: {}", e);
            return ExitCode::from(1);
        }
    };
    if let Err(e) = answer_value(&ans) {
        eprintln!("hale watch: {}", e);
        return ExitCode::from(1);
    }
    loop {
        match client.next_line() {
            Ok(Some(v)) => {
                let _ = writeln!(out, "{}", v);
                let _ = out.flush();
            }
            Ok(None) => return ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("hale watch: {}", e);
                return ExitCode::from(1);
            }
        }
    }
}

// ---- hale admin: a local page over the description ------------------------

const ADMIN_PAGE: &str = include_str!("admin.html");

/// `hale admin <socket> [--port N]`: serve a page on 127.0.0.1 that
/// lists the commands, reads and streams the description names and
/// calls through the socket; `/api/describe`, `/api/call/<name>`,
/// `/api/read/<name>` and `/api/watch/<name>` (server-sent events)
/// are its own endpoints, each one request to the binding.
pub fn run_admin(rest: &[String]) -> ExitCode {
    let mut port: u16 = 7473;
    let mut sock: Option<String> = None;
    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            "--port" => match rest.get(i + 1).and_then(|v| v.parse::<u16>().ok()) {
                Some(p) => {
                    port = p;
                    i += 2;
                }
                None => {
                    eprintln!("hale admin: --port requires a number");
                    return ExitCode::from(2);
                }
            },
            other if other.starts_with('-') => {
                eprintln!("hale admin: unknown flag {}", other);
                return ExitCode::from(2);
            }
            other => {
                sock = Some(other.to_string());
                i += 1;
            }
        }
    }
    let Some(sock) = sock else {
        eprintln!("usage: hale admin <socket> [--port N]");
        return ExitCode::from(2);
    };
    if let Err(e) = Client::connect(&sock).and_then(|mut c| c.describe()) {
        eprintln!("hale admin: {}", e);
        return ExitCode::from(1);
    }
    let listener = match std::net::TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("hale admin: could not listen on 127.0.0.1:{}: {}", port, e);
            return ExitCode::from(1);
        }
    };
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(port);
    // A per-launch token the page carries and every /api request
    // must present, so a page on another origin cannot drive the
    // binding through this process (and a Host other than this
    // listener's is refused outright, against DNS rebinding).
    let token = launch_token();
    println!("hale admin: http://127.0.0.1:{}/  (over {})", port, sock);
    let _ = std::io::stdout().flush();
    let shared = std::sync::Arc::new(AdminShared { sock, token, port });
    for conn in listener.incoming() {
        let Ok(conn) = conn else { continue };
        let shared = shared.clone();
        std::thread::spawn(move || serve_admin_conn(conn, &shared));
    }
    ExitCode::SUCCESS
}

struct AdminShared {
    sock: String,
    token: String,
    port: u16,
}

fn launch_token() -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    std::time::SystemTime::now().hash(&mut h);
    std::process::id().hash(&mut h);
    let a = h.finish();
    std::time::Instant::now().hash(&mut h);
    let b = h.finish();
    format!("{:016x}{:016x}", a, b)
}

/// The request's Host is this listener; its Origin, when it sends
/// one, is this listener's page. Anything else is another site
/// reaching for the binding through this process.
fn admin_origin_ok(shared: &AdminShared, host: &str, origin: Option<&str>) -> bool {
    let mine = [
        format!("127.0.0.1:{}", shared.port),
        format!("localhost:{}", shared.port),
    ];
    if !mine.iter().any(|m| m == host.trim()) {
        return false;
    }
    match origin {
        None => true,
        Some(o) => mine.iter().any(|m| o.trim() == format!("http://{}", m)),
    }
}

fn http_reply(conn: &mut std::net::TcpStream, status: &str, ctype: &str, body: &[u8]) {
    let _ = write!(
        conn,
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        status,
        ctype,
        body.len()
    );
    let _ = conn.write_all(body);
    let _ = conn.flush();
}

fn serve_admin_conn(mut conn: std::net::TcpStream, shared: &AdminShared) {
    let sock = shared.sock.as_str();
    let mut reader = BufReader::new(match conn.try_clone() {
        Ok(c) => c,
        Err(_) => return,
    });
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let full_path = parts.next().unwrap_or("/").to_string();
    let (path, query) = match full_path.split_once('?') {
        Some((p, q)) => (p.to_string(), q.to_string()),
        None => (full_path.clone(), String::new()),
    };
    let mut content_length = 0usize;
    let mut host = String::new();
    let mut origin: Option<String> = None;
    let mut content_type = String::new();
    let mut header_token = String::new();
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h).is_err() || h.trim().is_empty() {
            break;
        }
        let Some((k, v)) = h.split_once(':') else { continue };
        let v = v.trim().to_string();
        match k.trim().to_ascii_lowercase().as_str() {
            "content-length" => content_length = v.parse().unwrap_or(0),
            "host" => host = v,
            "origin" => origin = Some(v),
            "content-type" => content_type = v.to_ascii_lowercase(),
            "x-hale-admin" => header_token = v,
            _ => {}
        }
    }
    let mut body = vec![0u8; content_length];
    if content_length > 0 && reader.read_exact(&mut body).is_err() {
        return;
    }
    let body = String::from_utf8_lossy(&body).to_string();
    let json_err = |conn: &mut std::net::TcpStream, status: &str, e: String| {
        let v = json!({ "error": e });
        http_reply(conn, status, "application/json", v.to_string().as_bytes());
    };
    if !admin_origin_ok(shared, &host, origin.as_deref()) {
        json_err(&mut conn, "403 Forbidden", "this page serves 127.0.0.1 only: the request's Host or Origin is another site".to_string());
        return;
    }
    if path == "/" {
        let page = ADMIN_PAGE
            .replace("{{SOCKET}}", sock)
            .replace("{{TOKEN}}", &shared.token);
        http_reply(&mut conn, "200 OK", "text/html; charset=utf-8", page.as_bytes());
        return;
    }
    let query_token = query
        .split('&')
        .find_map(|kv| kv.strip_prefix("token="))
        .unwrap_or("");
    if header_token != shared.token && query_token != shared.token {
        json_err(&mut conn, "403 Forbidden", "missing or wrong admin token: open the page this process printed and act from it".to_string());
        return;
    }
    if path == "/api/describe" {
        match Client::connect(sock).and_then(|mut c| c.describe_raw()) {
            Ok(d) => http_reply(&mut conn, "200 OK", "application/json", d.as_bytes()),
            Err(e) => json_err(&mut conn, "502 Bad Gateway", e),
        };
        return;
    }
    if let Some(name) = path.strip_prefix("/api/call/") {
        if method != "POST" {
            http_reply(&mut conn, "405 Method Not Allowed", "text/plain", b"POST");
            return;
        }
        if !content_type.starts_with("application/json") {
            json_err(&mut conn, "415 Unsupported Media Type", "a call's body is JSON: send Content-Type: application/json".to_string());
            return;
        }
        let payload: Value = match serde_json::from_str(&body) {
            Ok(v) => v,
            Err(e) => {
                json_err(&mut conn, "400 Bad Request", format!("the payload is not JSON: {}", e));
                return;
            }
        };
        let mut req = Map::new();
        req.insert("call".to_string(), json!(name));
        req.insert("payload".to_string(), payload);
        match Client::connect(sock).and_then(|mut c| c.request_line(req)) {
            Ok((_, line)) => http_reply(&mut conn, "200 OK", "application/json", line.as_bytes()),
            Err(e) => json_err(&mut conn, "502 Bad Gateway", e),
        };
        return;
    }
    if let Some(name) = path.strip_prefix("/api/read/") {
        let mut req = Map::new();
        req.insert("read".to_string(), json!(name));
        match Client::connect(sock).and_then(|mut c| c.request_line(req)) {
            Ok((_, line)) => http_reply(&mut conn, "200 OK", "application/json", line.as_bytes()),
            Err(e) => json_err(&mut conn, "502 Bad Gateway", e),
        };
        return;
    }
    if let Some(name) = path.strip_prefix("/api/watch/") {
        let mut client = match Client::connect(sock) {
            Ok(c) => c,
            Err(e) => return json_err(&mut conn, "502 Bad Gateway", e),
        };
        let mut req = Map::new();
        req.insert("watch".to_string(), json!(name));
        let (ans, line) = match client.request_line(req) {
            Ok(a) => a,
            Err(e) => return json_err(&mut conn, "502 Bad Gateway", e),
        };
        if answer_value(&ans).is_err() {
            // A refused attach is an answer, not a stream: say so and close.
            http_reply(&mut conn, "403 Forbidden", "application/json", line.as_bytes());
            return;
        }
        let _ = write!(
            conn,
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n"
        );
        let _ = write!(conn, "data: {}\n\n", line);
        let _ = conn.flush();
        while let Ok(Some(v)) = client.next_raw_line() {
            if write!(conn, "data: {}\n\n", v).is_err() || conn.flush().is_err() {
                break;
            }
        }
        return;
    }
    http_reply(&mut conn, "404 Not Found", "text/plain", b"not found");
}

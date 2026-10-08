//! GH #1107, #1417 (R4 C): the generic clients of a served surface.
//!
//! A served exposure answers a description (spec/api.md § The
//! description): its identity and digest, its listener, the caller the
//! exposure established and the roles that caller holds, the members it
//! may call with their schemas, the streams of the hubs at that listener
//! it may subscribe to, and the transport's outcome encoding. Everything
//! here reads only that document. `hale describe` prints it, `hale call`
//! sends one member, `hale watch` subscribes to a stream, `hale admin`
//! serves a local page over it, and `hale mcp --app` (mcp.rs) turns it
//! into tools. None of them knows a member's name in advance: the
//! description is the whole contract, and a client that presents the
//! boundary check as a proof is misreading the note it carries.
//!
//! An endpoint is named by what the transport is:
//!
//! ```text
//! /run/desk/admin.sock      a Unix socket (unix::Rpc), also unix:/run/desk/admin.sock
//! http://127.0.0.1:8080     an http::Rpc listener, the caller named by --token
//! ws://127.0.0.1:9000       a hub: streams, and rpcs carried on its connection
//! mcp://127.0.0.1:8090      an mcp::Rpc listener (hale mcp --app only)
//! ```
//!
//! The wire is R0's: a line per request over the socket, `POST
//! /call/<member>` with `Hale-Surface-Digest` and `GET /.description`
//! over HTTP, the `ws` frames over a hub. A call carries the digest of
//! the description it read, so a program that changed under the client
//! refuses it (`digest_mismatch`) instead of running a different
//! contract. There is no other path: a program that serves no surface has
//! no endpoint, and `hale describe <file>` prints what `hale check --api`
//! prints from its rows.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::os::unix::net::UnixStream;
use std::process::ExitCode;
use std::time::Duration;

use serde_json::{json, Map, Value};

// ---- endpoints ---------------------------------------------------------------

/// Where an exposure listens, by transport.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Endpoint {
    Unix(String),
    /// `host:port` of an `http::Rpc` listener.
    Http(String),
    /// `host:port` of a hub.
    Ws(String),
    /// `host:port` of an `mcp::Rpc` listener.
    Mcp(String),
}

impl Endpoint {
    pub fn parse(s: &str) -> Result<Endpoint, String> {
        if let Some(p) = s.strip_prefix("unix:") {
            return Ok(Endpoint::Unix(p.to_string()));
        }
        for (scheme, make) in [
            ("http://", Endpoint::Http as fn(String) -> Endpoint),
            ("ws://", Endpoint::Ws),
            ("mcp://", Endpoint::Mcp),
        ] {
            if let Some(rest) = s.strip_prefix(scheme) {
                let hp = rest.split('/').next().unwrap_or("");
                if !hp.contains(':') {
                    return Err(format!("`{}` needs a host:port", s));
                }
                return Ok(make(hp.to_string()));
            }
        }
        if s.starts_with("https://") || s.starts_with("wss://") {
            return Err(format!("`{}`: these clients do not speak TLS; name the plain listener", s));
        }
        if s.contains("://") {
            return Err(format!("`{}`: an endpoint is a socket path, or http://, ws:// or mcp:// host:port", s));
        }
        Ok(Endpoint::Unix(s.to_string()))
    }

    pub fn show(&self) -> String {
        match self {
            Endpoint::Unix(p) => p.clone(),
            Endpoint::Http(h) => format!("http://{}", h),
            Endpoint::Ws(h) => format!("ws://{}", h),
            Endpoint::Mcp(h) => format!("mcp://{}", h),
        }
    }
}

/// `--token T`, else `HALE_API_TOKEN`: the bearer an HTTP or hub
/// exposure's `principals:` source is asked to name.
fn take_token(args: &mut Vec<String>) -> Result<Option<String>, String> {
    let given = take_flag(args, "--token")?;
    Ok(given.or_else(|| std::env::var("HALE_API_TOKEN").ok().filter(|t| !t.is_empty())))
}

/// Removes `--name value` from `args`.
fn take_flag(args: &mut Vec<String>, name: &str) -> Result<Option<String>, String> {
    let Some(i) = args.iter().position(|a| a == name) else { return Ok(None) };
    if i + 1 >= args.len() {
        return Err(format!("{} requires a value", name));
    }
    let v = args.remove(i + 1);
    args.remove(i);
    Ok(Some(v))
}

fn take_switch(args: &mut Vec<String>, name: &str) -> bool {
    let before = args.len();
    args.retain(|a| a != name);
    args.len() != before
}

// ---- the transports ------------------------------------------------------------

const READ_TIMEOUT: Duration = Duration::from_secs(120);

/// One HTTP response: the status and the body, read to the end.
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

/// One request on one connection (`Connection: close`, as the listener
/// answers one).
pub fn http_request(
    hp: &str,
    method: &str,
    path: &str,
    headers: &[(&str, String)],
    body: Option<&str>,
) -> Result<HttpResponse, String> {
    let mut s = TcpStream::connect(hp).map_err(|e| format!("could not connect to {}: {}", hp, e))?;
    let _ = s.set_read_timeout(Some(READ_TIMEOUT));
    let mut req = format!("{} {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n", method, path, hp);
    for (k, v) in headers {
        req.push_str(&format!("{}: {}\r\n", k, v));
    }
    if let Some(b) = body {
        req.push_str(&format!("Content-Type: application/json\r\nContent-Length: {}\r\n", b.len()));
    }
    req.push_str("\r\n");
    if let Some(b) = body {
        req.push_str(b);
    }
    s.write_all(req.as_bytes()).map_err(|e| format!("write failed: {}", e))?;
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).map_err(|e| format!("read failed: {}", e))?;
    let text = String::from_utf8_lossy(&raw).to_string();
    let Some((head, body)) = text.split_once("\r\n\r\n") else {
        return Err(format!("{} closed the connection without a response", hp));
    };
    let status = head
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|c| c.parse::<u16>().ok())
        .ok_or_else(|| format!("{} sent a response that is not HTTP", hp))?;
    Ok(HttpResponse { status, body: body.to_string() })
}

fn bearer(token: Option<&str>) -> Vec<(&'static str, String)> {
    token.map(|t| vec![("Authorization", format!("Bearer {}", t))]).unwrap_or_default()
}

/// Everything but the unreserved characters, so a member's `::` is one
/// path segment.
fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok());
            if let Some(v) = hex {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

/// One line out, one line back, on a connection of its own.
fn unix_exchange(path: &str, req: &Value) -> Result<String, String> {
    let mut s = UnixStream::connect(path).map_err(|e| format!("could not connect to {}: {}", path, e))?;
    let _ = s.set_read_timeout(Some(READ_TIMEOUT));
    writeln!(s, "{}", req).map_err(|e| format!("write failed: {}", e))?;
    s.flush().map_err(|e| format!("write failed: {}", e))?;
    let mut r = BufReader::new(s);
    loop {
        let mut line = String::new();
        let n = r.read_line(&mut line).map_err(|e| format!("read failed: {}", e))?;
        if n == 0 {
            return Err("the exposure closed the connection without a reply".to_string());
        }
        if !line.trim().is_empty() {
            return Ok(line.trim().to_string());
        }
    }
}

/// The raw text of a top-level field of one JSON object line, with
/// its bytes untouched (a value re-serialized through `Value` would
/// come back with sorted keys, and the description's order is part
/// of what an exposure promises to serve).
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
                    return Some(raw_value(line[start..].trim_start()));
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
            b',' if depth == 0 => return s[..i].trim_end().to_string(),
            _ => {}
        }
    }
    s.trim_end().to_string()
}

// ---- the description -------------------------------------------------------------

/// The caller's description, as the exposure wrote it, byte for byte: the
/// `value` of the Unix reply, the body of `GET /.description`. The
/// exposure serves what the caller's roles show.
pub fn fetch_description(ep: &Endpoint, token: Option<&str>) -> Result<String, String> {
    match ep {
        Endpoint::Unix(path) => {
            let line = unix_exchange(path, &json!({ "describe": true }))?;
            let v: Value = serde_json::from_str(&line)
                .map_err(|e| format!("{} sent a line that is not JSON: {} ({})", path, line, e))?;
            if v.get("ok") != Some(&json!(true)) {
                return Err(refusal_text(&raw_field(&line, "refusal").unwrap_or_else(|| line.clone())));
            }
            raw_field(&line, "value").ok_or_else(|| "the reply carries no description".to_string())
        }
        Endpoint::Http(hp) | Endpoint::Ws(hp) => {
            let r = http_request(hp, "GET", "/.description", &bearer(token), None)?;
            if r.status != 200 {
                let why = raw_field(&r.body, "refusal").map(|f| refusal_text(&f)).unwrap_or_else(|| r.body.trim().to_string());
                return Err(format!("{} refused the description (HTTP {}): {}", ep.show(), r.status, why));
            }
            Ok(r.body.trim().to_string())
        }
        Endpoint::Mcp(_) => Err(format!(
            "{} is an mcp::Rpc listener: its discovery is `tools/list` (hale mcp --app), and its description is the program's (`hale check --api --exposure NAME --caller P`)",
            ep.show()
        )),
    }
}

fn parse_doc(raw: &str) -> Result<Value, String> {
    serde_json::from_str(raw).map_err(|e| format!("the description is not JSON: {}", e))
}

/// The names of the members the description lists.
pub fn member_names(doc: &Value) -> Vec<String> {
    names_of(doc, "members", "name")
}

/// The topics of the streams the description lists.
pub fn stream_topics(doc: &Value) -> Vec<String> {
    names_of(doc, "streams", "topic")
}

fn names_of(doc: &Value, table: &str, key: &str) -> Vec<String> {
    doc.get(table)
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|c| c.get(key).and_then(Value::as_str).map(str::to_string)).collect())
        .unwrap_or_default()
}

/// A refusal object (`{"kind": …, "reason": …}`) as one line.
fn refusal_text(obj: &str) -> String {
    let Ok(v) = serde_json::from_str::<Value>(obj) else { return obj.to_string() };
    let kind = v.get("kind").and_then(Value::as_str).unwrap_or("refused");
    let reason = v.get("reason").and_then(Value::as_str).unwrap_or("");
    let mut out = format!("{}: {}", kind, reason);
    if let Some(r) = v.get("requires").and_then(Value::as_array) {
        let r: Vec<&str> = r.iter().filter_map(Value::as_str).collect();
        out.push_str(&format!(" (requires {})", r.join(", ")));
    }
    if let Some(s) = v.get("served").and_then(Value::as_str) {
        out.push_str(&format!(" (served {})", s));
    }
    out
}

// ---- a call ------------------------------------------------------------------------

/// The five outcomes of spec/api.md § Outcomes a reply can be (the fifth,
/// the transport's failure, is an `Err`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Result,
    HandlerError,
    Refusal,
    ServerError,
}

impl Kind {
    pub fn word(self) -> &'static str {
        match self {
            Kind::Result => "result",
            Kind::HandlerError => "handler_error",
            Kind::Refusal => "refusal",
            Kind::ServerError => "server_error",
        }
    }
}

/// An exposure's answer to one call: the outcome, the JSON that carries it
/// (the response, `E`, or the refusal object), the HTTP status when there
/// is one, and the line or body as the exposure wrote it.
pub struct Reply {
    pub kind: Kind,
    pub body: String,
    pub status: Option<u16>,
    pub raw: String,
}

impl Reply {
    /// The answer as one line for a human: the result's JSON, or why not.
    pub fn failure_text(&self) -> String {
        match self.kind {
            Kind::Result => String::new(),
            Kind::HandlerError => format!("handler error: {}", self.body),
            Kind::Refusal => format!("refused: {}", refusal_text(&self.body)),
            Kind::ServerError => "server error: the handler violated a structural limit".to_string(),
        }
    }
}

/// A Unix reply line (also a hub's `reply` frame) as an outcome.
fn outcome_of_line(line: &str) -> Result<Reply, String> {
    let v: Value = serde_json::from_str(line).map_err(|e| format!("the exposure sent a line that is not JSON: {} ({})", line, e))?;
    let field = |k: &str| raw_field(line, k).unwrap_or_else(|| "null".to_string());
    let (kind, body) = if v.get("ok") == Some(&json!(true)) {
        (Kind::Result, field("value"))
    } else if v.get("error").is_some() {
        (Kind::HandlerError, field("error"))
    } else if v.pointer("/refusal/kind") == Some(&json!("server")) {
        (Kind::ServerError, field("refusal"))
    } else if v.get("refusal").is_some() {
        (Kind::Refusal, field("refusal"))
    } else {
        return Err(format!("the exposure sent a reply that is no outcome: {}", line));
    };
    Ok(Reply { kind, body, status: None, raw: line.to_string() })
}

/// An HTTP response as an outcome, by § Outcomes' HTTP column.
fn outcome_of_http(r: HttpResponse) -> Result<Reply, String> {
    let body = r.body.trim().to_string();
    let (kind, shown) = match r.status {
        200 => (Kind::Result, body.clone()),
        422 => (Kind::HandlerError, body.clone()),
        _ => {
            let obj = raw_field(&body, "refusal").ok_or_else(|| format!("HTTP {} with a body that is no refusal: {}", r.status, body))?;
            if serde_json::from_str::<Value>(&obj).ok().and_then(|v| v.get("kind").cloned()) == Some(json!("server")) {
                (Kind::ServerError, obj)
            } else {
                (Kind::Refusal, obj)
            }
        }
    };
    Ok(Reply { kind, body: shown, status: Some(r.status), raw: body })
}

/// Calls `member` with `payload` (JSON text) on the exposure, naming the
/// digest of the description the caller read.
pub fn call(ep: &Endpoint, token: Option<&str>, member: &str, payload: &str, digest: Option<&str>) -> Result<Reply, String> {
    let payload: Value = serde_json::from_str(payload).map_err(|e| format!("the payload is not JSON: {}", e))?;
    match ep {
        Endpoint::Unix(path) => {
            let mut req = Map::new();
            req.insert("call".into(), json!(member));
            req.insert("payload".into(), payload);
            req.insert("id".into(), json!(1));
            if let Some(d) = digest {
                req.insert("digest".into(), json!(d));
            }
            outcome_of_line(&unix_exchange(path, &Value::Object(req))?)
        }
        Endpoint::Http(hp) => {
            let mut headers = bearer(token);
            if let Some(d) = digest {
                headers.push(("Hale-Surface-Digest", d.to_string()));
            }
            let path = format!("/call/{}", percent_encode(member));
            outcome_of_http(http_request(hp, "POST", &path, &headers, Some(&payload.to_string()))?)
        }
        Endpoint::Ws(hp) => {
            // Rpcs carried on a hub's connection (spec/api.md § Rpcs on a hub).
            let mut ws = Ws::connect(hp, token)?;
            let mut frame = Map::new();
            frame.insert("type".into(), json!("call"));
            frame.insert("id".into(), json!("1"));
            frame.insert("call".into(), json!(member));
            frame.insert("payload".into(), payload);
            if let Some(d) = digest {
                frame.insert("digest".into(), json!(d));
            }
            ws.send_text(&Value::Object(frame).to_string())?;
            loop {
                let Some(msg) = ws.recv()? else {
                    return Err("the hub closed the connection without a reply".to_string());
                };
                let v: Value = serde_json::from_str(&msg).unwrap_or(Value::Null);
                match v.get("type").and_then(Value::as_str) {
                    Some("reply") => return outcome_of_line(&msg),
                    Some("refusal") => {
                        return Ok(Reply { kind: Kind::Refusal, body: raw_field(&msg, "refusal").unwrap_or_default(), status: None, raw: msg });
                    }
                    _ => {}
                }
            }
        }
        Endpoint::Mcp(_) => Err("an mcp::Rpc listener is called by `tools/call` (hale mcp --app)".to_string()),
    }
}

// ---- a WebSocket client ---------------------------------------------------------------

const WS_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

fn random_bytes(n: usize) -> Result<Vec<u8>, String> {
    let mut bytes = vec![0u8; n];
    let mut f = std::fs::File::open("/dev/urandom").map_err(|e| format!("could not open /dev/urandom: {}", e))?;
    f.read_exact(&mut bytes).map_err(|e| format!("could not read /dev/urandom: {}", e))?;
    Ok(bytes)
}

/// A client of a hub: RFC 6455's upgrade, masked text frames out, the
/// hub's unmasked ones in. Only what the hub speaks: no fragments, no
/// extensions.
pub struct Ws {
    r: BufReader<TcpStream>,
    w: TcpStream,
}

impl Ws {
    pub fn connect(hp: &str, token: Option<&str>) -> Result<Ws, String> {
        let s = TcpStream::connect(hp).map_err(|e| format!("could not connect to {}: {}", hp, e))?;
        let _ = s.set_read_timeout(Some(Duration::from_secs(30)));
        let key = openssl::base64::encode_block(&random_bytes(16)?);
        let mut req = format!(
            "GET / HTTP/1.1\r\nHost: {}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {}\r\nSec-WebSocket-Version: 13\r\n",
            hp, key
        );
        if let Some(t) = token {
            req.push_str(&format!("Authorization: Bearer {}\r\n", t));
        }
        req.push_str("\r\n");
        let mut w = s.try_clone().map_err(|e| format!("could not clone the connection: {}", e))?;
        w.write_all(req.as_bytes()).map_err(|e| format!("write failed: {}", e))?;
        let mut r = BufReader::new(s);
        let mut status = String::new();
        r.read_line(&mut status).map_err(|e| format!("read failed: {}", e))?;
        let mut accept = None;
        loop {
            let mut h = String::new();
            let n = r.read_line(&mut h).map_err(|e| format!("read failed: {}", e))?;
            if n == 0 || h.trim().is_empty() {
                break;
            }
            if let Some((k, v)) = h.split_once(':') {
                if k.trim().eq_ignore_ascii_case("sec-websocket-accept") {
                    accept = Some(v.trim().to_string());
                }
            }
        }
        if !status.split_whitespace().nth(1).map_or(false, |c| c == "101") {
            return Err(format!("{} did not upgrade to a WebSocket: {}", hp, status.trim()));
        }
        let want = openssl::hash::hash(openssl::hash::MessageDigest::sha1(), format!("{}{}", key, WS_GUID).as_bytes())
            .map(|d| openssl::base64::encode_block(&d))
            .map_err(|e| format!("sha1: {}", e))?;
        if accept.as_deref() != Some(want.as_str()) {
            return Err(format!("{} answered the upgrade with the wrong accept key", hp));
        }
        let _ = r.get_ref().set_read_timeout(None);
        Ok(Ws { r, w })
    }

    fn write_frame(&mut self, opcode: u8, payload: &[u8]) -> Result<(), String> {
        let mut f = vec![0x80 | opcode];
        let n = payload.len();
        if n < 126 {
            f.push(0x80 | n as u8);
        } else if n <= 65535 {
            f.push(0x80 | 126);
            f.extend((n as u16).to_be_bytes());
        } else {
            f.push(0x80 | 127);
            f.extend((n as u64).to_be_bytes());
        }
        let mask = random_bytes(4)?;
        f.extend(&mask);
        f.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
        self.w.write_all(&f).map_err(|e| format!("write failed: {}", e))
    }

    pub fn send_text(&mut self, text: &str) -> Result<(), String> {
        self.write_frame(0x1, text.as_bytes())
    }

    /// The next text message; `None` when the hub closes the connection.
    pub fn recv(&mut self) -> Result<Option<String>, String> {
        loop {
            let mut h = [0u8; 2];
            if let Err(e) = self.r.read_exact(&mut h) {
                return if e.kind() == std::io::ErrorKind::UnexpectedEof { Ok(None) } else { Err(format!("read failed: {}", e)) };
            }
            let (fin, op, masked) = (h[0] & 0x80 != 0, h[0] & 0x0f, h[1] & 0x80 != 0);
            let mut len = (h[1] & 0x7f) as u64;
            let rd = |r: &mut BufReader<TcpStream>, n: usize| -> Result<Vec<u8>, String> {
                let mut b = vec![0u8; n];
                r.read_exact(&mut b).map_err(|e| format!("read failed: {}", e))?;
                Ok(b)
            };
            if len == 126 {
                len = u16::from_be_bytes(rd(&mut self.r, 2)?.try_into().unwrap()) as u64;
            } else if len == 127 {
                len = u64::from_be_bytes(rd(&mut self.r, 8)?.try_into().unwrap());
            }
            if len > (16 << 20) {
                return Err("the hub sent a frame over 16 MiB".to_string());
            }
            let mask = if masked { Some(rd(&mut self.r, 4)?) } else { None };
            let mut payload = rd(&mut self.r, len as usize)?;
            if let Some(m) = mask {
                for (i, b) in payload.iter_mut().enumerate() {
                    *b ^= m[i % 4];
                }
            }
            match op {
                0x1 if fin => return Ok(Some(String::from_utf8_lossy(&payload).to_string())),
                0x1 | 0x0 => return Err("the hub sent a fragmented message".to_string()),
                0x8 => {
                    let _ = self.write_frame(0x8, &[]);
                    return Ok(None);
                }
                0x9 => self.write_frame(0xA, &payload)?,
                0xA => {}
                other => return Err(format!("the hub sent a frame of opcode {}", other)),
            }
        }
    }
}

// ---- the verbs ------------------------------------------------------------------------------

fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_default()
}

fn is_program(target: &str) -> bool {
    let p = std::path::Path::new(target);
    (p.is_dir() || p.extension().map_or(false, |e| e == "hl")) && p.exists()
}

/// `hale describe <endpoint | file.hl | dir> [-o <path>] [--token T]`.
///
/// An endpoint answers its description for the caller it names. A
/// program has no caller: `hale check --api` prints it from the rows, and
/// the flags `--exposure/--caller/--holds` and the forms `--surface NAME
/// --openapi|--json-schema|--mcp` go through to it.
pub fn run_describe(rest: &[String]) -> ExitCode {
    let mut args: Vec<String> = rest.to_vec();
    let token = match take_token(&mut args) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("hale describe: {}", e);
            return ExitCode::from(2);
        }
    };
    let out = match take_flag(&mut args, "-o").and_then(|o| match o {
        Some(o) => Ok(Some(o)),
        None => take_flag(&mut args, "--out"),
    }) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("hale describe: {}", e);
            return ExitCode::from(2);
        }
    };
    let Some(target) = args.iter().find(|a| !a.starts_with('-')).cloned() else {
        eprintln!("usage: hale describe <endpoint | file.hl | dir> [--token T] [-o <path>]");
        eprintln!("       endpoint: a socket path, http://host:port or ws://host:port; a program takes hale check --api's flags");
        return ExitCode::from(2);
    };
    let text = if is_program(&target) {
        let me = match std::env::current_exe() {
            Ok(m) => m,
            Err(e) => {
                eprintln!("hale describe: current exe: {}", e);
                return ExitCode::from(1);
            }
        };
        let mut cmd = std::process::Command::new(me);
        cmd.arg("check").arg("--api");
        for a in &args {
            if *a != target {
                cmd.arg(a);
            }
        }
        cmd.arg(&target);
        match cmd.output() {
            Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
            Ok(o) => {
                eprint!("{}", String::from_utf8_lossy(&o.stderr));
                return ExitCode::from(1);
            }
            Err(e) => {
                eprintln!("hale describe: could not run hale check: {}", e);
                return ExitCode::from(1);
            }
        }
    } else {
        if let Some(f) = args.iter().find(|a| a.starts_with('-')) {
            eprintln!(
                "hale describe: {} is a flag of a program's description (`hale describe <file.hl>`); an endpoint answers its own exposure's document for the caller it names",
                f
            );
            return ExitCode::from(2);
        }
        let ep = match Endpoint::parse(&target) {
            Ok(e) => e,
            Err(e) => {
                eprintln!("hale describe: {}", e);
                return ExitCode::from(2);
            }
        };
        // The exposure's own bytes, never re-serialized: spec/model.md
        // promises the served and the emitted document agree byte for byte.
        match fetch_description(&ep, token.as_deref()) {
            Ok(raw) => raw.trim().to_string() + "\n",
            Err(e) => {
                eprintln!("hale describe: {}", e);
                return ExitCode::from(1);
            }
        }
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

/// The member list as a refusal message: what the exposure shows this caller.
fn not_a_member(verb: &str, what: &str, name: &str, doc: &Value) -> String {
    format!(
        "hale {}: `{}` is not {} (the exposure describes the slice the caller's roles show)\n  members: {}\n  streams (use `hale watch`): {}",
        verb,
        name,
        what,
        member_names(doc).join(", "),
        stream_topics(doc).join(", ")
    )
}

/// `hale call <endpoint> <member> [<json payload>] [--token T] [--receipt]`.
pub fn run_call(rest: &[String]) -> ExitCode {
    let mut args: Vec<String> = rest.to_vec();
    let receipt = take_switch(&mut args, "--receipt");
    let token = match take_token(&mut args) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("hale call: {}", e);
            return ExitCode::from(2);
        }
    };
    let (target, name, body) = match args.as_slice() {
        [s, n] => (s.clone(), n.clone(), "{}".to_string()),
        [s, n, b] => (s.clone(), n.clone(), b.clone()),
        _ => {
            eprintln!("usage: hale call <endpoint> <member> [<json payload>] [--token T] [--receipt]");
            return ExitCode::from(2);
        }
    };
    let ep = match Endpoint::parse(&target) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("hale call: {}", e);
            return ExitCode::from(2);
        }
    };
    let doc = match fetch_description(&ep, token.as_deref()).and_then(|raw| parse_doc(&raw)) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("hale call: {}", e);
            return ExitCode::from(1);
        }
    };
    // A hub's live description lists its streams only (spec/api.md § Open
    // points), so a member is checked where the document lists members.
    let on_hub = matches!(ep, Endpoint::Ws(_));
    if !(on_hub && member_names(&doc).is_empty()) && !member_names(&doc).iter().any(|m| *m == name) {
        eprintln!("{}", not_a_member("call", "a member this caller may call", &name, &doc));
        return ExitCode::from(1);
    }
    let digest = doc.get("digest").and_then(Value::as_str).map(str::to_string);
    let reply = match call(&ep, token.as_deref(), &name, &body, digest.as_deref()) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("hale call: {}", e);
            return ExitCode::from(1);
        }
    };
    if receipt {
        // The whole answer as the exposure wrote it: over the socket the
        // reply line (request_id, the echoed id, the outcome, the caller);
        // over HTTP the status and the body.
        match reply.status {
            Some(s) => println!("{}", json!({ "status": s, "body": serde_json::from_str::<Value>(&reply.raw).unwrap_or(Value::Null) })),
            None => println!("{}", reply.raw),
        }
        return if reply.kind == Kind::Result { ExitCode::SUCCESS } else { ExitCode::from(1) };
    }
    if reply.kind == Kind::Result {
        match serde_json::from_str::<Value>(&reply.body) {
            Ok(v) => println!("{}", pretty(&v)),
            Err(_) => println!("{}", reply.body),
        }
        return ExitCode::SUCCESS;
    }
    eprintln!("hale call: {}", reply.failure_text());
    ExitCode::from(1)
}

/// `hale watch <ws://hub> <topic> [--token T]`: subscribe and print frames,
/// one per line, until the hub closes the connection.
pub fn run_watch(rest: &[String]) -> ExitCode {
    let mut args: Vec<String> = rest.to_vec();
    let token = match take_token(&mut args) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("hale watch: {}", e);
            return ExitCode::from(2);
        }
    };
    let [target, topic] = args.as_slice() else {
        eprintln!("usage: hale watch <ws://host:port> <topic> [--token T]");
        return ExitCode::from(2);
    };
    let ep = match Endpoint::parse(target) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("hale watch: {}", e);
            return ExitCode::from(2);
        }
    };
    let Endpoint::Ws(hp) = &ep else {
        eprintln!("hale watch: a stream is served by a hub: name its address as ws://host:port (`{}` is not one)", target);
        return ExitCode::from(2);
    };
    let doc = match fetch_description(&ep, token.as_deref()).and_then(|raw| parse_doc(&raw)) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("hale watch: {}", e);
            return ExitCode::from(1);
        }
    };
    if !stream_topics(&doc).iter().any(|t| t == topic) {
        eprintln!("{}", not_a_member("watch", "a stream this caller may subscribe to", topic, &doc));
        return ExitCode::from(1);
    }
    let mut ws = match Ws::connect(hp, token.as_deref()) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("hale watch: {}", e);
            return ExitCode::from(1);
        }
    };
    if let Err(e) = ws.send_text(&json!({ "type": "subscribe", "topic": topic }).to_string()) {
        eprintln!("hale watch: {}", e);
        return ExitCode::from(1);
    }
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    loop {
        match ws.recv() {
            Ok(Some(frame)) => {
                if writeln!(out, "{}", frame).and_then(|_| out.flush()).is_err() {
                    return ExitCode::SUCCESS;
                }
                let v: Value = serde_json::from_str(&frame).unwrap_or(Value::Null);
                match v.get("type").and_then(Value::as_str) {
                    Some("refusal") => {
                        eprintln!("hale watch: refused: {}", refusal_text(&raw_field(&frame, "refusal").unwrap_or_default()));
                        return ExitCode::from(1);
                    }
                    Some("unauthorized") => {
                        eprintln!("hale watch: the subscription ended: {}", v.get("reason").and_then(Value::as_str).unwrap_or("unauthorized"));
                        return ExitCode::from(1);
                    }
                    _ => {}
                }
            }
            Ok(None) => return ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("hale watch: {}", e);
                return ExitCode::from(1);
            }
        }
    }
}

// ---- hale admin: a local page over the description ------------------------------------------

const ADMIN_PAGE: &str = include_str!("admin.html");

/// `hale admin <endpoint> [--port N] [--token T]`: serve a page on
/// 127.0.0.1 that lists the members and streams the description names and
/// calls through the endpoint; `/api/describe`, `/api/call/<member>` and
/// `/api/watch/<topic>` (server-sent events) are its own routes, each one
/// request to the exposure.
pub fn run_admin(rest: &[String]) -> ExitCode {
    let mut args: Vec<String> = rest.to_vec();
    let up_token = match take_token(&mut args) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("hale admin: {}", e);
            return ExitCode::from(2);
        }
    };
    let mut port: u16 = 7473;
    match take_flag(&mut args, "--port") {
        Ok(Some(p)) => match p.parse::<u16>() {
            Ok(p) => port = p,
            Err(_) => {
                eprintln!("hale admin: --port requires a number");
                return ExitCode::from(2);
            }
        },
        Ok(None) => {}
        Err(e) => {
            eprintln!("hale admin: {}", e);
            return ExitCode::from(2);
        }
    }
    if let Some(f) = args.iter().find(|a| a.starts_with('-')) {
        eprintln!("hale admin: unknown flag {}", f);
        return ExitCode::from(2);
    }
    let [target] = args.as_slice() else {
        eprintln!("usage: hale admin <endpoint> [--port N] [--token T]");
        return ExitCode::from(2);
    };
    let ep = match Endpoint::parse(target) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("hale admin: {}", e);
            return ExitCode::from(2);
        }
    };
    if let Err(e) = fetch_description(&ep, up_token.as_deref()) {
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
    // exposure through this process (and a Host other than this
    // listener's is refused outright, against DNS rebinding).
    let token = match launch_token() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("hale admin: {}", e);
            return ExitCode::from(1);
        }
    };
    // The URL carries the token, Jupyter style: any local process can
    // open loopback TCP, so the page itself is served only to whoever
    // has the token this process printed.
    println!("hale admin: http://127.0.0.1:{}/?token={}  (over {})", port, token, ep.show());
    let _ = std::io::stdout().flush();
    let shared = std::sync::Arc::new(AdminShared { ep, up_token, token, port });
    for conn in listener.incoming() {
        let Ok(conn) = conn else { continue };
        let shared = shared.clone();
        std::thread::spawn(move || serve_admin_conn(conn, &shared));
    }
    ExitCode::SUCCESS
}

struct AdminShared {
    ep: Endpoint,
    /// The bearer this process presents to the exposure.
    up_token: Option<String>,
    token: String,
    port: u16,
}

/// 128 bits from the kernel's entropy, as hex.
fn launch_token() -> Result<String, String> {
    let bytes = random_bytes(16).map_err(|e| format!("the admin token: {}", e))?;
    Ok(bytes.iter().map(|b| format!("{:02x}", b)).collect())
}

/// Compares in time independent of where the strings differ.
fn token_matches(given: &str, expected: &str) -> bool {
    let (a, b) = (given.as_bytes(), expected.as_bytes());
    let mut acc: u8 = (a.len() != b.len()) as u8;
    for i in 0..expected.len() {
        acc |= b[i] ^ a.get(i).copied().unwrap_or(0);
    }
    acc == 0
}

/// The request's Host is this listener; its Origin, when it sends
/// one, is this listener's page. Anything else is another site
/// reaching for the exposure through this process.
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
    let query_token = query
        .split('&')
        .find_map(|kv| kv.strip_prefix("token="))
        .unwrap_or("");
    let tokened = token_matches(&header_token, &shared.token) || token_matches(query_token, &shared.token);
    if !tokened {
        if path == "/" {
            http_reply(&mut conn, "403 Forbidden", "text/plain", b"hale admin: open the URL this process printed; it carries the token\n");
        } else {
            json_err(&mut conn, "403 Forbidden", "missing or wrong admin token: open the URL this process printed and act from it".to_string());
        }
        return;
    }
    let up = shared.up_token.as_deref();
    if path == "/" {
        let page = ADMIN_PAGE
            .replace("{{ENDPOINT}}", &Value::String(shared.ep.show()).to_string().replace('<', "\\u003c"))
            .replace("{{TOKEN}}", &shared.token);
        http_reply(&mut conn, "200 OK", "text/html; charset=utf-8", page.as_bytes());
        return;
    }
    if path == "/api/describe" {
        match fetch_description(&shared.ep, up) {
            Ok(d) => http_reply(&mut conn, "200 OK", "application/json", d.as_bytes()),
            Err(e) => json_err(&mut conn, "502 Bad Gateway", e),
        };
        return;
    }
    if let Some(member) = path.strip_prefix("/api/call/") {
        let member = percent_decode(member);
        if method != "POST" {
            http_reply(&mut conn, "405 Method Not Allowed", "text/plain", b"POST");
            return;
        }
        if !content_type.starts_with("application/json") {
            json_err(&mut conn, "415 Unsupported Media Type", "a call's body is JSON: send Content-Type: application/json".to_string());
            return;
        }
        if let Err(e) = serde_json::from_str::<Value>(&body) {
            json_err(&mut conn, "400 Bad Request", format!("the payload is not JSON: {}", e));
            return;
        }
        // The digest of the description read now: a program that moved
        // under the page refuses the call by name.
        let digest = fetch_description(&shared.ep, up)
            .and_then(|raw| parse_doc(&raw))
            .map(|d| d.get("digest").and_then(Value::as_str).map(str::to_string));
        let answer = digest.and_then(|d| call(&shared.ep, up, &member, &body, d.as_deref()));
        match answer {
            Ok(r) => {
                let out = format!(
                    "{{\"outcome\":\"{}\",\"status\":{},\"body\":{}}}",
                    r.kind.word(),
                    r.status.map_or("null".to_string(), |s| s.to_string()),
                    if r.body.is_empty() { "null" } else { r.body.as_str() }
                );
                http_reply(&mut conn, "200 OK", "application/json", out.as_bytes());
            }
            Err(e) => json_err(&mut conn, "502 Bad Gateway", e),
        };
        return;
    }
    if let Some(topic) = path.strip_prefix("/api/watch/") {
        let topic = percent_decode(topic);
        let Endpoint::Ws(hp) = &shared.ep else {
            json_err(&mut conn, "400 Bad Request", "streams are served by a hub: run hale admin over its ws:// address".to_string());
            return;
        };
        let mut ws = match Ws::connect(hp, up) {
            Ok(w) => w,
            Err(e) => return json_err(&mut conn, "502 Bad Gateway", e),
        };
        if let Err(e) = ws.send_text(&json!({ "type": "subscribe", "topic": topic }).to_string()) {
            return json_err(&mut conn, "502 Bad Gateway", e);
        }
        let first = match ws.recv() {
            Ok(Some(f)) => f,
            Ok(None) => return json_err(&mut conn, "502 Bad Gateway", "the hub closed the connection".to_string()),
            Err(e) => return json_err(&mut conn, "502 Bad Gateway", e),
        };
        if first.contains("\"type\":\"refusal\"") {
            // A refused subscription is an answer, not a stream: say so and close.
            http_reply(&mut conn, "403 Forbidden", "application/json", first.as_bytes());
            return;
        }
        let _ = write!(
            conn,
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n"
        );
        let _ = write!(conn, "data: {}\n\n", first);
        let _ = conn.flush();
        while let Ok(Some(v)) = ws.recv() {
            if write!(conn, "data: {}\n\n", v).is_err() || conn.flush().is_err() {
                break;
            }
        }
        return;
    }
    http_reply(&mut conn, "404 Not Found", "text/plain", b"not found");
}

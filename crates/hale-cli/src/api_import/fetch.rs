//! Reading an imported surface's source: a local file, or an `http(s)` URL
//! fetched with one request per connection (spec/api.md § Imported
//! surfaces). The client is small on purpose: HTTP/1.1, `Connection:
//! close`, a `Content-Length` or chunked body, TLS through the system
//! OpenSSL the toolchain already links, redirects followed for a GET. An
//! MCP endpoint is asked over the same requests (`mcp_tools`).

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use serde_json::{json, Value};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const READ_TIMEOUT: Duration = Duration::from_secs(30);
/// The largest document read: an OpenAPI document of a large API is a few
/// MiB.
const MAX_BODY: usize = 32 << 20;
const MAX_REDIRECTS: usize = 5;

/// One response.
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// A URL this client reads: its scheme, host, port and the path with its
/// query.
struct Url {
    tls: bool,
    host: String,
    port: u16,
    path: String,
}

fn parse_url(url: &str) -> Result<Url, String> {
    let (tls, rest) = if let Some(r) = url.strip_prefix("https://") {
        (true, r)
    } else if let Some(r) = url.strip_prefix("http://") {
        (false, r)
    } else {
        return Err(format!("`{url}` is no http:// or https:// URL"));
    };
    let (authority, path) = match rest.find(['/', '?']) {
        Some(i) if rest.as_bytes()[i] == b'/' => (&rest[..i], rest[i..].to_string()),
        Some(i) => (&rest[..i], format!("/{}", &rest[i..])),
        None => (rest, "/".to_string()),
    };
    let path = path.split('#').next().unwrap_or("/").to_string();
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    let (host, port) = if let Some(v6) = authority.strip_prefix('[') {
        let (h, after) = v6
            .split_once(']')
            .ok_or_else(|| format!("`{url}`: an unclosed `[` in its host"))?;
        (h.to_string(), after.strip_prefix(':'))
    } else {
        match authority.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), Some(p)),
            None => (authority.to_string(), None),
        }
    };
    let port = match port {
        Some(p) => p.parse::<u16>().map_err(|_| format!("`{url}`: `{p}` is no port"))?,
        None if tls => 443,
        None => 80,
    };
    if host.is_empty() {
        return Err(format!("`{url}` names no host"));
    }
    Ok(Url { tls, host, port, path })
}

trait Stream: Read + Write {}
impl<T: Read + Write> Stream for T {}

fn connect(u: &Url) -> Result<Box<dyn Stream>, String> {
    let addrs: Vec<_> = (u.host.as_str(), u.port)
        .to_socket_addrs()
        .map_err(|e| format!("could not resolve {}: {e}", u.host))?
        .collect();
    let mut last = format!("{} resolved to no address", u.host);
    for a in addrs {
        match TcpStream::connect_timeout(&a, CONNECT_TIMEOUT) {
            Ok(s) => {
                let _ = s.set_read_timeout(Some(READ_TIMEOUT));
                let _ = s.set_write_timeout(Some(READ_TIMEOUT));
                if !u.tls {
                    return Ok(Box::new(s));
                }
                let connector = openssl::ssl::SslConnector::builder(openssl::ssl::SslMethod::tls())
                    .map_err(|e| format!("TLS: {e}"))?
                    .build();
                let tls = connector
                    .connect(&u.host, s)
                    .map_err(|e| format!("TLS to {}: {e}", u.host))?;
                return Ok(Box::new(tls));
            }
            Err(e) => last = format!("could not connect to {}:{}: {e}", u.host, u.port),
        }
    }
    Err(last)
}

/// One request and its response. A GET follows redirects. `done` is asked,
/// as the body arrives, whether what has arrived is all the caller needs: an
/// MCP endpoint may keep an event stream open after it has answered.
pub fn request(
    method: &str,
    url: &str,
    headers: &[(&str, String)],
    body: Option<&str>,
    done: &dyn Fn(&[u8]) -> bool,
) -> Result<Response, String> {
    let mut url = url.to_string();
    for _ in 0..=MAX_REDIRECTS {
        let r = request_once(method, &url, headers, body, done)?;
        if method == "GET" && matches!(r.status, 301 | 302 | 303 | 307 | 308) {
            if let Some(loc) = r.header("location") {
                url = resolve(&url, loc);
                continue;
            }
        }
        return Ok(r);
    }
    Err(format!("{url}: more than {MAX_REDIRECTS} redirects"))
}

/// A redirect's `Location` against the URL that answered it.
fn resolve(base: &str, loc: &str) -> String {
    if loc.starts_with("http://") || loc.starts_with("https://") {
        return loc.to_string();
    }
    let scheme_end = base.find("://").map_or(0, |i| i + 3);
    let origin_end = base[scheme_end..].find('/').map_or(base.len(), |i| scheme_end + i);
    if loc.starts_with('/') {
        return format!("{}{loc}", &base[..origin_end]);
    }
    let dir_end = base
        .rfind('/')
        .filter(|i| *i >= origin_end)
        .map_or(base.len(), |i| i + 1);
    let mut dir = base[..dir_end].to_string();
    if !dir.ends_with('/') {
        dir.push('/');
    }
    format!("{dir}{loc}")
}

fn request_once(
    method: &str,
    url: &str,
    headers: &[(&str, String)],
    body: Option<&str>,
    done: &dyn Fn(&[u8]) -> bool,
) -> Result<Response, String> {
    let u = parse_url(url)?;
    let mut s = connect(&u)?;
    let host = if (u.tls && u.port == 443) || (!u.tls && u.port == 80) {
        u.host.clone()
    } else {
        format!("{}:{}", u.host, u.port)
    };
    let mut req = format!(
        "{method} {} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nUser-Agent: hale-api-import/{}\r\n",
        u.path,
        env!("CARGO_PKG_VERSION")
    );
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    if let Some(b) = body {
        req.push_str(&format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            b.len()
        ));
    }
    req.push_str("\r\n");
    if let Some(b) = body {
        req.push_str(b);
    }
    s.write_all(req.as_bytes())
        .map_err(|e| format!("{url}: write failed: {e}"))?;
    let mut raw: Vec<u8> = Vec::new();
    let mut buf = [0u8; 16384];
    let mut head: Option<(u16, Vec<(String, String)>, usize)> = None;
    loop {
        let n = match s.read(&mut buf) {
            Ok(n) => n,
            // a TLS peer that closes without close_notify has still sent its body
            Err(_) if head.is_some() => 0,
            Err(e) => return Err(format!("{url}: read failed: {e}")),
        };
        if n == 0 {
            break;
        }
        raw.extend_from_slice(&buf[..n]);
        if raw.len() > MAX_BODY {
            return Err(format!("{url}: the response is larger than {} MiB", MAX_BODY >> 20));
        }
        if head.is_none() {
            head = parse_head(&raw);
        }
        if let Some((_, hs, at)) = &head {
            let (body, complete) = decode_body(hs, &raw[*at..]);
            if complete || done(&body) {
                break;
            }
        }
    }
    let (status, headers, at) = head
        .or_else(|| parse_head(&raw))
        .ok_or_else(|| format!("{url}: the response is not HTTP"))?;
    let (body, _) = decode_body(&headers, &raw[at..]);
    Ok(Response { status, headers, body })
}

/// The status line and headers, and where the body starts, once all of the
/// head has arrived.
fn parse_head(raw: &[u8]) -> Option<(u16, Vec<(String, String)>, usize)> {
    let end = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
    let text = String::from_utf8_lossy(&raw[..end]);
    let mut lines = text.split("\r\n");
    let status = lines.next()?.split_whitespace().nth(1)?.parse::<u16>().ok()?;
    let headers = lines
        .filter_map(|l| {
            l.split_once(':')
                .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        })
        .collect();
    Some((status, headers, end + 4))
}

/// The body that has arrived, de-chunked, and whether it is all of it.
fn decode_body(headers: &[(String, String)], raw: &[u8]) -> (Vec<u8>, bool) {
    let header = |n: &str| {
        headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(n))
            .map(|(_, v)| v.as_str())
    };
    if header("transfer-encoding").is_some_and(|v| v.to_ascii_lowercase().contains("chunked")) {
        let mut out = Vec::new();
        let mut at = 0;
        loop {
            let Some(eol) = raw[at..].windows(2).position(|w| w == b"\r\n") else {
                return (out, false);
            };
            let line = String::from_utf8_lossy(&raw[at..at + eol]);
            let Ok(size) = usize::from_str_radix(line.split(';').next().unwrap_or("").trim(), 16) else {
                return (out, false);
            };
            at += eol + 2;
            if size == 0 {
                return (out, true);
            }
            if raw.len() < at + size {
                out.extend_from_slice(&raw[at..]);
                return (out, false);
            }
            out.extend_from_slice(&raw[at..at + size]);
            at += size + 2;
            if at > raw.len() {
                return (out, false);
            }
        }
    }
    if let Some(n) = header("content-length").and_then(|v| v.parse::<usize>().ok()) {
        let take = n.min(raw.len());
        return (raw[..take].to_vec(), raw.len() >= n);
    }
    (raw.to_vec(), false)
}

/// What a source names: a URL, or a path.
pub fn is_url(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://")
}

/// A document (JSON or YAML) at `url`, by GET.
pub fn get_json(url: &str) -> Result<Value, String> {
    let accept = "application/json, application/yaml, text/yaml;q=0.9";
    let r = request("GET", url, &[("Accept", accept.to_string())], None, &|_| {
        false
    })?;
    if !(200..300).contains(&r.status) {
        return Err(format!("{url} answered {}", r.status));
    }
    let yaml = r.header("content-type").is_some_and(|c| {
        let c = c.to_ascii_lowercase();
        c.contains("yaml")
    });
    parse_document_as(&String::from_utf8_lossy(&r.body), url, yaml)
}

/// A document's text as JSON, or why not: JSON when it opens with `{`,
/// YAML (the OpenAPI subset of `yaml.rs`) when it does not or when its name
/// ends `.yaml` or `.yml`.
pub fn parse_document(text: &str, from: &str) -> Result<Value, String> {
    parse_document_as(text, from, false)
}

/// `parse_document`, with `yaml` set when the source has said it serves YAML.
pub fn parse_document_as(text: &str, from: &str, yaml: bool) -> Result<Value, String> {
    let name = from
        .split(['?', '#'])
        .next()
        .unwrap_or(from)
        .to_ascii_lowercase();
    let yaml = yaml || name.ends_with(".yaml") || name.ends_with(".yml");
    let head = text.trim_start_matches('\u{feff}').trim_start();
    if !yaml && head.starts_with('{') {
        return serde_json::from_str(text).map_err(|e| format!("{from} is not JSON: {e}"));
    }
    super::yaml::parse(text).map_err(|e| format!("{from} is YAML that the import cannot read: {e}"))
}

/// The JSON-RPC messages of an MCP response: its body as JSON, or each
/// `data:` of an event stream.
fn rpc_messages(r: &Response) -> Vec<Value> {
    let text = String::from_utf8_lossy(&r.body);
    let sse = r
        .header("content-type")
        .is_some_and(|c| c.contains("text/event-stream"));
    if !sse {
        return match serde_json::from_str::<Value>(&text) {
            Ok(Value::Array(a)) => a,
            Ok(v) => vec![v],
            Err(_) => Vec::new(),
        };
    }
    let mut out = Vec::new();
    for event in text.split("\n\n").flat_map(|e| e.split("\r\n\r\n")) {
        let data: Vec<&str> = event
            .lines()
            .filter_map(|l| l.strip_prefix("data:"))
            .map(|d| d.strip_prefix(' ').unwrap_or(d))
            .collect();
        if let Ok(v) = serde_json::from_str::<Value>(&data.join("\n")) {
            out.push(v);
        }
    }
    out
}

/// One JSON-RPC call to an MCP endpoint over its Streamable HTTP transport,
/// and its result.
/// The protocol versions the import speaks, newest first: the one it asks
/// for, and the earlier ones a server may answer with instead.
const MCP_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

/// What `initialize` settled: the session the server handed out, if any,
/// and the protocol version it chose, which every later request names
/// (`MCP-Protocol-Version`, the 2025-06-18 lifecycle's version negotiation).
#[derive(Default)]
struct McpSession {
    id: Option<String>,
    version: Option<String>,
}

impl McpSession {
    fn headers(&self) -> Vec<(&'static str, String)> {
        let mut headers = vec![("Accept", "application/json, text/event-stream".to_string())];
        if let Some(s) = &self.id {
            headers.push(("Mcp-Session-Id", s.clone()));
        }
        if let Some(v) = &self.version {
            headers.push(("MCP-Protocol-Version", v.clone()));
        }
        headers
    }
}

fn mcp_call(url: &str, session: &mut McpSession, id: u64, method: &str, params: Value) -> Result<Value, String> {
    let body = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string();
    let headers = session.headers();
    let answered = move |b: &[u8]| {
        let text = String::from_utf8_lossy(b);
        text.lines().filter_map(|l| l.strip_prefix("data:")).any(|d| {
            serde_json::from_str::<Value>(d.trim()).is_ok_and(|v| v.get("id").and_then(Value::as_u64) == Some(id))
        })
    };
    let r = request("POST", url, &headers, Some(&body), &answered)?;
    if !(200..300).contains(&r.status) {
        return Err(format!("{url} answered `{method}` with {}", r.status));
    }
    if let Some(s) = r.header("mcp-session-id") {
        session.id = Some(s.to_string());
    }
    let reply = rpc_messages(&r)
        .into_iter()
        .find(|m| m.get("id").and_then(Value::as_u64) == Some(id))
        .ok_or_else(|| format!("{url} sent no answer to `{method}`: is it an MCP endpoint?"))?;
    if let Some(e) = reply.get("error") {
        let msg = e.get("message").and_then(Value::as_str).unwrap_or("an error");
        return Err(format!("{url} refused `{method}`: {msg}"));
    }
    reply
        .get("result")
        .cloned()
        .ok_or_else(|| format!("{url} answered `{method}` with no result"))
}

/// An MCP endpoint's tools, every page of `tools/list`, after the
/// `initialize` handshake; and the server's name and version as it gives
/// them.
pub fn mcp_tools(url: &str) -> Result<(Vec<Value>, String), String> {
    let mut session = McpSession::default();
    let init = mcp_call(
        url,
        &mut session,
        1,
        "initialize",
        json!({
            "protocolVersion": MCP_VERSIONS[0],
            "capabilities": {},
            "clientInfo": {"name": "hale-api-import", "version": env!("CARGO_PKG_VERSION")},
        }),
    )?;
    let server = init
        .get("serverInfo")
        .map(|s| {
            let name = s.get("name").and_then(Value::as_str).unwrap_or("");
            let version = s.get("version").and_then(Value::as_str).unwrap_or("");
            format!("{name} {version}").trim().to_string()
        })
        .unwrap_or_default();
    // the version the server chose, which must be one the import speaks
    let version = init
        .get("protocolVersion")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{url} answered `initialize` with no `protocolVersion`"))?;
    if !MCP_VERSIONS.contains(&version) {
        return Err(format!(
            "{url} chose MCP protocol version {version}, and the import speaks {}",
            MCP_VERSIONS.join(", ")
        ));
    }
    session.version = Some(version.to_string());
    // the notification has no answer to wait for
    let note = json!({"jsonrpc": "2.0", "method": "notifications/initialized"}).to_string();
    let _ = request("POST", url, &session.headers(), Some(&note), &|_| true);
    let mut tools = Vec::new();
    let mut cursor: Option<String> = None;
    for page in 0..1000u64 {
        let params = match &cursor {
            Some(c) => json!({"cursor": c}),
            None => json!({}),
        };
        let result = mcp_call(url, &mut session, 2 + page, "tools/list", params)?;
        let listed = result
            .get("tools")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("{url} answered `tools/list` with no `tools`"))?;
        tools.extend(listed.iter().cloned());
        match result.get("nextCursor").and_then(Value::as_str) {
            Some(c) if !c.is_empty() => cursor = Some(c.to_string()),
            _ => return Ok((tools, server)),
        }
    }
    Err(format!("{url}: `tools/list` did not end after 1000 pages"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_and_redirects_resolve() {
        let u = parse_url("https://example.test/a/openapi.json?x=1").unwrap();
        assert!(u.tls);
        assert_eq!(
            (u.host.as_str(), u.port, u.path.as_str()),
            ("example.test", 443, "/a/openapi.json?x=1")
        );
        let u = parse_url("http://127.0.0.1:8080").unwrap();
        assert_eq!((u.port, u.path.as_str()), (8080, "/"));
        assert_eq!(resolve("http://h:1/a/b.json", "/c.json"), "http://h:1/c.json");
        assert_eq!(resolve("http://h:1/a/b.json", "c.json"), "http://h:1/a/c.json");
        assert_eq!(resolve("http://h:1/a/b.json", "https://o/x"), "https://o/x");
    }

    #[test]
    fn a_chunked_body_is_decoded() {
        let hs = vec![("Transfer-Encoding".to_string(), "chunked".to_string())];
        let (b, done) = decode_body(&hs, b"3\r\nabc\r\n2\r\nde\r\n0\r\n\r\n");
        assert_eq!((b.as_slice(), done), (&b"abcde"[..], true));
        let (b, done) = decode_body(&hs, b"3\r\nabc\r\n2\r\nd");
        assert_eq!((b.as_slice(), done), (&b"abcd"[..], false));
    }
}

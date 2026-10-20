//! `hale api import` and `hale api drift` (spec/api.md § Imported surfaces):
//! an OpenAPI document (a file, or served over HTTP) and an MCP endpoint's
//! tool list (a stub speaking the Streamable HTTP transport, in this
//! process) become an `api` block the checker accepts, and the drift check
//! names each row the source has since added, removed or changed, in the
//! stable form a watcher parses. Nothing here reaches past the loopback.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Arc, Mutex};

fn hale() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_hale"));
    c.env("HALE_SKIP_STALE_CHECK", "1");
    c
}

fn unique_dir(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "hale_api_import_{}_{}_{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed),
        tag
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn run(dir: &Path, args: &[&str]) -> Output {
    hale().current_dir(dir).args(args).output().expect("run hale")
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).to_string()
}

const SHELF: &str = r##"{
  "openapi": "3.0.3",
  "info": {"title": "Shelf", "version": "1.0.0"},
  "paths": {
    "/items": {
      "get": {
        "operationId": "listItems",
        "parameters": [{"name": "limit", "in": "query", "schema": {"type": "integer"}}],
        "responses": {"200": {"description": "ok", "content": {"application/json": {"schema": {"type": "array", "items": {"$ref": "#/components/schemas/Item"}}}}}}
      },
      "post": {
        "operationId": "createItem",
        "requestBody": {"required": true, "content": {"application/json": {"schema": {"$ref": "#/components/schemas/NewItem"}}}},
        "responses": {"201": {"description": "made", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/Item"}}}}}
      }
    },
    "/items/{itemId}": {
      "parameters": [{"name": "itemId", "in": "path", "required": true, "schema": {"type": "integer"}}],
      "get": {
        "responses": {"200": {"description": "ok", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/Item"}}}}}
      },
      "delete": {"operationId": "deleteItem", "responses": {"204": {"description": "gone"}}}
    }
  },
  "components": {
    "schemas": {
      "Item": {
        "type": "object",
        "required": ["id", "name"],
        "properties": {
          "id": {"type": "integer"},
          "name": {"type": "string"},
          "state": {"type": "string", "enum": ["open", "held", "gone"]},
          "type": {"type": "string"},
          "labels": {"type": "array", "maxItems": 4, "items": {"type": "string"}},
          "extra": {"oneOf": [{"type": "string"}, {"type": "integer"}]},
          "parent": {"$ref": "#/components/schemas/Item"}
        }
      },
      "NewItem": {
        "type": "object",
        "required": ["name"],
        "properties": {"name": {"type": "string"}, "note": {"type": "string", "nullable": true}}
      }
    }
  }
}"##;

const MAIN: &str = "main locus App {\n    run { }\n}\n";

/// A seed holding the import of `doc` as `Shelf`, the document beside it.
fn imported_seed(tag: &str) -> PathBuf {
    let dir = unique_dir(tag);
    std::fs::write(dir.join("shelf.json"), SHELF).unwrap();
    std::fs::create_dir_all(dir.join("seed")).unwrap();
    std::fs::write(dir.join("seed/main.hl"), MAIN).unwrap();
    let out = run(
        &dir,
        &["api", "import", "shelf.json", "--as", "Shelf", "--out", "seed/shelf.hl"],
    );
    assert!(out.status.success(), "import: {}", text(&out.stderr));
    dir
}

#[test]
fn an_openapi_document_becomes_a_surface_the_checker_accepts() {
    let dir = imported_seed("openapi");
    let src = std::fs::read_to_string(dir.join("seed/shelf.hl")).unwrap();
    for want in [
        "api Shelf from openapi \"../shelf.json\" {",
        "rpc create_item(ShelfNewItem) -> ShelfItem wire: \"POST /items\";",
        "rpc delete_item(ShelfDeleteItemRequest) wire: \"DELETE /items/{itemId}\";",
        // no operationId: a name from the method and the path
        "rpc get_items_item_id(ShelfGetItemsItemIdRequest) -> ShelfItem wire: \"GET /items/{itemId}\";",
        // an unbounded array is no record's list
        "// response: std::json::Value, an array with no `maxItems`, and a record holds a bounded list only",
        "rpc list_items(ShelfListItemsRequest) -> std::json::Value wire: \"GET /items\";",
        "item_id: Int `json:\"itemId\"`;",
        // an optional parameter and an optional, nullable field are presence pairs
        "has_limit: Bool = false;\n    limit: Int = 0;",
        "has_note: Bool = false;\n    note: String = \"\";",
        "labels: bounded[String; 4];",
        "// std::json::Value: `oneOf`: the codec has no union of shapes",
        "// std::json::Value: `#/components/schemas/Item` is a recursive shape, and a record cannot hold itself",
        "type_: String = \"\" `json:\"type\"`;",
        "type ShelfItemState = enum { open, held, gone };",
    ] {
        assert!(src.contains(want), "missing `{want}` in:\n{src}");
    }
    // a second run writes the same bytes
    let again = run(
        &dir,
        &[
            "api",
            "import",
            "shelf.json",
            "--as",
            "Shelf",
            "--out",
            "again/shelf.hl",
        ],
    );
    assert!(again.status.success(), "{}", text(&again.stderr));
    assert_eq!(std::fs::read_to_string(dir.join("again/shelf.hl")).unwrap(), src);

    let check = run(&dir, &["check", "seed"]);
    assert!(check.status.success(), "check: {}", text(&check.stderr));
    let fmt = run(&dir, &["fmt", "--check", "seed"]);
    assert!(fmt.status.success(), "fmt: {}{}", text(&fmt.stdout), text(&fmt.stderr));
    // the description lists the consumed surface's rows like any other's
    let api = run(&dir, &["check", "--api", "seed"]);
    assert!(api.status.success(), "check --api: {}", text(&api.stderr));
    assert!(text(&api.stdout).contains("\"name\": \"create_item\""));
    assert!(
        text(&api.stderr).contains("Shelf: in step with ../shelf.json (4 rows)"),
        "{}",
        text(&api.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn drift_names_each_row_the_document_has_changed() {
    let dir = imported_seed("drift");
    let in_step = run(&dir, &["api", "drift", "seed"]);
    assert_eq!(in_step.status.code(), Some(0), "{}", text(&in_step.stderr));
    assert_eq!(text(&in_step.stdout), "Shelf: in step with ../shelf.json (4 rows)\n");

    // one row renamed, one removed, one shape widened
    let moved = SHELF
        .replace("\"createItem\"", "\"addItem\"")
        .replace(
            "\"delete\": {\"operationId\": \"deleteItem\", \"responses\": {\"204\": {\"description\": \"gone\"}}}",
            "\"put\": {\"responses\": {\"204\": {\"description\": \"kept\"}}}",
        )
        .replace("\"maxItems\": 4", "\"maxItems\": 8");
    assert_ne!(moved, SHELF);
    std::fs::write(dir.join("shelf.json"), moved).unwrap();
    let drift = run(&dir, &["api", "drift", "seed"]);
    assert_eq!(drift.status.code(), Some(1), "{}", text(&drift.stderr));
    assert_eq!(
        text(&drift.stdout),
        "drift Shelf added add_item\n\
         drift Shelf removed create_item\n\
         drift Shelf removed delete_item\n\
         drift Shelf changed get_items_item_id response\n\
         drift Shelf added put_items_item_id\n"
    );
    let api = run(&dir, &["check", "--api", "seed"]);
    assert_eq!(api.status.code(), Some(1));
    assert!(
        text(&api.stderr).contains("drift Shelf changed get_items_item_id response"),
        "{}",
        text(&api.stderr)
    );

    // a source that cannot be read is no drift
    std::fs::remove_file(dir.join("shelf.json")).unwrap();
    let gone = run(&dir, &["api", "drift", "seed"]);
    assert_eq!(gone.status.code(), Some(2));
    assert!(text(&gone.stderr).contains("could not read"), "{}", text(&gone.stderr));
    let _ = std::fs::remove_dir_all(&dir);
}

/// What the stub answers: a request's method, path, headers and body, to
/// the response's status, headers and body.
type Handler = dyn Fn(&str, &str, &str, &str) -> (u16, Vec<(String, String)>, String) + Send + Sync;

/// An HTTP server on the loopback answering `handler`, one request per
/// connection; its base URL. The listener is held by the serving thread
/// for the life of the process.
fn stub(handler: Arc<Handler>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut conn) = conn else { continue };
            let mut raw = Vec::new();
            let mut buf = [0u8; 8192];
            let (head, body) = loop {
                let n = conn.read(&mut buf).unwrap_or(0);
                if n == 0 {
                    break (String::new(), String::new());
                }
                raw.extend_from_slice(&buf[..n]);
                let t = String::from_utf8_lossy(&raw).to_string();
                if let Some((h, b)) = t.split_once("\r\n\r\n") {
                    let len = h
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().to_string())
                        })
                        .and_then(|v| v.parse::<usize>().ok())
                        .unwrap_or(0);
                    if b.len() >= len {
                        break (h.to_string(), b.to_string());
                    }
                }
            };
            let mut first = head.lines().next().unwrap_or("").split_whitespace();
            let (method, path) = (
                first.next().unwrap_or("").to_string(),
                first.next().unwrap_or("").to_string(),
            );
            let (status, headers, reply) = handler(&method, &path, &head, &body);
            let mut out = format!("HTTP/1.1 {status} X\r\nConnection: close\r\n");
            for (k, v) in &headers {
                out.push_str(&format!("{k}: {v}\r\n"));
            }
            // chunked, in two pieces, as a real server may send it
            out.push_str("Transfer-Encoding: chunked\r\n\r\n");
            let (a, b) = reply.split_at(reply.len() / 2);
            for part in [a, b] {
                if !part.is_empty() {
                    out.push_str(&format!("{:x}\r\n{part}\r\n", part.len()));
                }
            }
            out.push_str("0\r\n\r\n");
            let _ = conn.write_all(out.as_bytes());
        }
    });
    format!("http://{addr}")
}

#[test]
fn an_openapi_url_is_read_over_http() {
    let served = Arc::new(Mutex::new(SHELF.to_string()));
    let doc = served.clone();
    let base = stub(Arc::new(move |method, path, _, _| {
        if method == "GET" && path == "/openapi.json" {
            (
                200,
                vec![("Content-Type".into(), "application/json".into())],
                doc.lock().unwrap().clone(),
            )
        } else {
            (404, vec![], String::new())
        }
    }));
    let dir = unique_dir("url");
    std::fs::write(dir.join("main.hl"), MAIN).unwrap();
    let url = format!("{base}/openapi.json");
    let out = run(&dir, &["api", "import", &url, "--as", "Shelf", "--out", "shelf.hl"]);
    assert!(out.status.success(), "import: {}", text(&out.stderr));
    let src = std::fs::read_to_string(dir.join("shelf.hl")).unwrap();
    assert!(src.contains(&format!("api Shelf from openapi \"{url}\" {{")), "{src}");
    let check = run(&dir, &["check", "."]);
    assert!(check.status.success(), "check: {}", text(&check.stderr));
    *served.lock().unwrap() = SHELF.replace("\"listItems\"", "\"listAll\"");
    let drift = run(&dir, &["api", "drift", ".", "--surface", "Shelf"]);
    assert_eq!(drift.status.code(), Some(1));
    assert_eq!(
        text(&drift.stdout),
        "drift Shelf added list_all\ndrift Shelf removed list_items\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

const TOOLS_PAGE_1: &str = r#"[
  {"name": "get-weather", "description": "the weather", "inputSchema": {"type": "object", "properties": {"city": {"type": "string"}, "days": {"type": "integer"}}, "required": ["city"]}}
]"#;
const TOOLS_PAGE_2: &str = r#"[
  {"name": "ping", "inputSchema": {"type": "object"}},
  {"name": "sum", "inputSchema": {"type": "object", "properties": {"a": {"type": "number"}, "b": {"type": "number"}}, "required": ["a", "b"]},
   "outputSchema": {"type": "object", "properties": {"total": {"type": "number"}}, "required": ["total"]}}
]"#;

/// An MCP endpoint over the Streamable HTTP transport: `initialize` hands
/// out a session and settles an earlier protocol version than the one asked
/// for; the notification and `tools/list` refuse a request that does not
/// name both, and `tools/list` answers in two pages, the first as an event
/// stream.
fn mcp_stub(pages: Arc<Mutex<(String, String)>>) -> String {
    let noted = Arc::new(Mutex::new(false));
    stub(Arc::new(move |method, path, head, body| {
        if method != "POST" || path != "/mcp" {
            return (405, vec![], String::new());
        }
        let req: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
        let id = req.get("id").cloned().unwrap_or_default();
        let session = head.lines().any(|l| l.to_ascii_lowercase() == "mcp-session-id: s-1");
        // every request after `initialize` names the version it settled
        let versioned = head
            .lines()
            .any(|l| l.to_ascii_lowercase() == "mcp-protocol-version: 2025-03-26");
        let json =
            |result: serde_json::Value| serde_json::json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string();
        match req.get("method").and_then(|m| m.as_str()) {
            Some("initialize") => (
                200,
                vec![
                    ("Content-Type".into(), "application/json".into()),
                    ("Mcp-Session-Id".into(), "s-1".into()),
                ],
                json(
                    serde_json::json!({"protocolVersion": "2025-03-26", "capabilities": {"tools": {}}, "serverInfo": {"name": "stub", "version": "0.1"}}),
                ),
            ),
            Some("notifications/initialized") if !versioned => (400, vec![], String::new()),
            Some("notifications/initialized") => {
                *noted.lock().unwrap() = true;
                (202, vec![], String::new())
            }
            Some("tools/list") if !session || !versioned || !*noted.lock().unwrap() => (400, vec![], String::new()),
            Some("tools/list") => {
                let p = pages.lock().unwrap();
                let cursor = req.pointer("/params/cursor").and_then(|c| c.as_str());
                let (tools, next) = match cursor {
                    None => (p.0.clone(), Some("page-2")),
                    Some(_) => (p.1.clone(), None),
                };
                let tools: serde_json::Value = serde_json::from_str(&tools).unwrap();
                let mut result = serde_json::json!({"tools": tools});
                if let Some(n) = next {
                    result["nextCursor"] = serde_json::json!(n);
                    let event = format!("event: message\ndata: {}\n\n", json(result));
                    return (200, vec![("Content-Type".into(), "text/event-stream".into())], event);
                }
                (
                    200,
                    vec![("Content-Type".into(), "application/json".into())],
                    json(result),
                )
            }
            _ => (400, vec![], String::new()),
        }
    }))
}

#[test]
fn an_mcp_endpoint_is_read_through_its_tool_list() {
    let pages = Arc::new(Mutex::new((TOOLS_PAGE_1.to_string(), TOOLS_PAGE_2.to_string())));
    let url = format!("{}/mcp", mcp_stub(pages.clone()));
    let dir = unique_dir("mcp");
    std::fs::write(dir.join("main.hl"), MAIN).unwrap();
    let out = run(&dir, &["api", "import", &url, "--as", "Tools"]);
    assert!(out.status.success(), "import: {}", text(&out.stderr));
    let src = std::fs::read_to_string(dir.join("tools.hl")).unwrap();
    let head = format!("api Tools from mcp \"{url}\" {{");
    for want in [
        head.as_str(),
        "// response: std::json::Value, the tool declares no `outputSchema`",
        "rpc get_weather(ToolsGetWeatherInput) -> std::json::Value wire: \"get-weather\";",
        "rpc ping() -> std::json::Value wire: \"ping\";",
        "rpc sum(ToolsSumInput) -> ToolsSumOutput wire: \"sum\";",
        "type ToolsSumOutput {\n    total: Float;\n}",
        "has_days: Bool = false;",
    ] {
        assert!(src.contains(want), "missing `{want}` in:\n{src}");
    }
    let check = run(&dir, &["check", "."]);
    assert!(check.status.success(), "check: {}", text(&check.stderr));
    assert_eq!(run(&dir, &["api", "drift", "."]).status.code(), Some(0));

    // the endpoint drops a tool, adds one, and changes one's input
    *pages.lock().unwrap() = (
        TOOLS_PAGE_1.replace("\"days\": {\"type\": \"integer\"}", "\"days\": {\"type\": \"string\"}"),
        TOOLS_PAGE_2.replace("\"name\": \"ping\"", "\"name\": \"echo\""),
    );
    let drift = run(&dir, &["api", "drift", "."]);
    assert_eq!(drift.status.code(), Some(1), "{}", text(&drift.stderr));
    assert_eq!(
        text(&drift.stdout),
        "drift Tools added echo\ndrift Tools changed get_weather request\ndrift Tools removed ping\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_imported_surface_is_consumed_never_served() {
    let dir = unique_dir("laws");
    std::fs::write(
        dir.join("main.hl"),
        "type Note { text: String; }\n\
         api Notes from openapi \"notes.json\" {\n    rpc post_note(Note) -> Note wire: \"POST /notes\";\n    rpc Desk::post;\n}\n\
         api Mixed {\n    rpc shape(Note) -> Note;\n    rpc Desk::post;\n}\n\
         locus Desk {\n    fn post(n: Note) -> Note { return n; }\n}\n\
         main locus App {\n    params {\n        desk: Desk = Desk {};\n        \
         web: api::Exposure = api::serve(Notes, http::Rpc { bind: \"127.0.0.1:0\" });\n    }\n    run { }\n}\n",
    )
    .unwrap();
    let check = run(&dir, &["check", "main.hl"]);
    assert!(!check.status.success());
    let err = text(&check.stderr);
    for want in [
        "rpc `Desk::post`: `Notes` is an imported surface, and its rows name the operations' shapes, not handlers",
        "`Mixed` holds `shape`, a row that names a shape, and `Desk::post`, a row that names a handler",
        "serve of `Notes`: `Notes` is a surface the program consumes, whose rows name shapes and no handler",
    ] {
        assert!(err.contains(want), "missing `{want}` in:\n{err}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_property_spelled_like_a_presence_flag_stays_a_property() {
    let dir = unique_dir("flags");
    std::fs::write(
        dir.join("probe.json"),
        r#"{"openapi": "3.0.3", "info": {"title": "Probe", "version": "1"},
            "paths": {"/count": {"get": {"operationId": "count",
              "responses": {"200": {"description": "ok", "content": {"application/json": {"schema": {
                "type": "object", "required": ["count", "has_count", "clear_count"],
                "properties": {"count": {"type": "integer"}, "has_count": {"type": "boolean"},
                               "clear_count": {"type": "boolean"}}}}}}}}}}}"#,
    )
    .unwrap();
    std::fs::write(dir.join("main.hl"), MAIN).unwrap();
    let out = run(&dir, &["api", "import", "probe.json", "--as", "Probe"]);
    assert!(out.status.success(), "import: {}", text(&out.stderr));
    let src = std::fs::read_to_string(dir.join("probe.hl")).unwrap();
    assert!(src.contains("key_has_count: Bool `json:\"has_count\"`;"), "{src}");
    assert!(src.contains("key_clear_count: Bool `json:\"clear_count\"`;"), "{src}");
    // the description keeps all three keys, each required
    let schema = run(&dir, &["check", "--api", "--surface", "Probe", "--json-schema", "."]);
    assert!(schema.status.success(), "{}", text(&schema.stderr));
    let doc = text(&schema.stdout);
    let at = doc
        .find("\"ProbeCountResponse\"")
        .unwrap_or_else(|| panic!("no ProbeCountResponse in {doc}"));
    let record = &doc[at..];
    let required = &record[record.find("\"required\"").expect("required")..];
    let required = &required[..required.find(']').unwrap()];
    for key in ["\"count\"", "\"has_count\"", "\"clear_count\""] {
        assert!(required.contains(key), "{key} not required in {record}");
    }
    assert_eq!(run(&dir, &["api", "drift", "."]).status.code(), Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_api_holds_a_dependency_seeds_imported_surface_to_its_source() {
    let dir = unique_dir("dep");
    std::fs::write(dir.join("shelf.json"), SHELF).unwrap();
    std::fs::create_dir_all(dir.join("app")).unwrap();
    let out = run(
        &dir,
        &["api", "import", "shelf.json", "--as", "Shelf", "--out", "lib/api.hl"],
    );
    assert!(out.status.success(), "import: {}", text(&out.stderr));
    std::fs::write(dir.join("app/main.hl"), format!("import \"../lib\" as lib;\n\n{MAIN}")).unwrap();
    let in_step = run(&dir, &["check", "--api", "app"]);
    assert!(in_step.status.success(), "{}", text(&in_step.stderr));
    assert!(
        text(&in_step.stderr).contains("Shelf: in step with ../shelf.json (4 rows)"),
        "{}",
        text(&in_step.stderr)
    );
    std::fs::write(dir.join("shelf.json"), SHELF.replace("\"listItems\"", "\"listAll\"")).unwrap();
    let moved = run(&dir, &["check", "--api", "app"]);
    assert_eq!(moved.status.code(), Some(1), "{}", text(&moved.stderr));
    assert!(
        text(&moved.stderr).contains("drift Shelf added list_all\ndrift Shelf removed list_items\n"),
        "{}",
        text(&moved.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

const NOTES_YAML: &str = r##"# a document as a service publishes it
openapi: 3.0.3
info:
  title: Notes
  version: 2.1.0
  description: |
    Notes for a desk.

    Each note has a title and a body.
paths:
  /notes:
    get:
      operationId: listNotes
      summary: >-
        List the notes,
        newest first.
      parameters:
        - name: limit
          in: query
          required: false
          schema: {type: integer}
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema:
                type: array
                maxItems: 50
                items:
                  $ref: '#/components/schemas/Note'
    post:
      operationId: createNote
      requestBody:
        required: true
        content:
          application/json:
            schema:
              $ref: "#/components/schemas/NewNote"
      responses:
        201:
          description: made
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/Note'
components:
  schemas:
    Note:
      type: object
      required: [id, title]
      properties:
        id:
          type: integer
        title:
          type: string
        pinned:
          type: boolean
          default: false
        tags:
          type: array
          maxItems: 4
          items: {type: string}
    NewNote:
      type: object
      required: [title]
      properties:
        title: {type: string}
        body: {type: string, nullable: true}
"##;

const NOTES_JSON: &str = r##"{
  "openapi": "3.0.3",
  "info": {"title": "Notes", "version": "2.1.0", "description": "Notes for a desk.\n\nEach note has a title and a body.\n"},
  "paths": {
    "/notes": {
      "get": {
        "operationId": "listNotes",
        "summary": "List the notes, newest first.",
        "parameters": [{"name": "limit", "in": "query", "required": false, "schema": {"type": "integer"}}],
        "responses": {"200": {"description": "ok", "content": {"application/json": {"schema": {"type": "array", "maxItems": 50, "items": {"$ref": "#/components/schemas/Note"}}}}}}
      },
      "post": {
        "operationId": "createNote",
        "requestBody": {"required": true, "content": {"application/json": {"schema": {"$ref": "#/components/schemas/NewNote"}}}},
        "responses": {"201": {"description": "made", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/Note"}}}}}
      }
    }
  },
  "components": {"schemas": {
    "Note": {"type": "object", "required": ["id", "title"], "properties": {
      "id": {"type": "integer"}, "title": {"type": "string"}, "pinned": {"type": "boolean", "default": false},
      "tags": {"type": "array", "maxItems": 4, "items": {"type": "string"}}}},
    "NewNote": {"type": "object", "required": ["title"], "properties": {
      "title": {"type": "string"}, "body": {"type": "string", "nullable": true}}}
  }}
}"##;

#[test]
fn a_yaml_document_imports_as_its_json_does() {
    let dir = unique_dir("yaml");
    std::fs::write(dir.join("notes.json"), NOTES_JSON).unwrap();
    std::fs::write(dir.join("notes.yaml"), NOTES_YAML).unwrap();
    // a YAML document under a name that says nothing of it is told by content
    std::fs::write(dir.join("notes.doc"), NOTES_YAML).unwrap();
    let import = |file: &str| {
        let to = format!("{file}.hl");
        let out = run(
            &dir,
            &["api", "import", file, "--as", "Notes", "--out", &to],
        );
        assert!(out.status.success(), "import {file}: {}", text(&out.stderr));
        std::fs::read_to_string(dir.join(&to))
            .unwrap()
            .replace(file, "DOC")
    };
    let from_json = import("notes.json");
    assert!(
        from_json.contains("rpc create_note(NotesNewNote) -> NotesNote wire: \"POST /notes\";"),
        "{from_json}"
    );
    assert_eq!(import("notes.yaml"), from_json);
    assert_eq!(import("notes.doc"), from_json);
}

#[test]
fn a_yaml_document_is_held_to_its_import_by_drift() {
    let dir = unique_dir("yaml_drift");
    std::fs::write(dir.join("notes.yaml"), NOTES_YAML).unwrap();
    let out = run(
        &dir,
        &[
            "api",
            "import",
            "notes.yaml",
            "--as",
            "Notes",
            "--out",
            "notes.hl",
        ],
    );
    assert!(out.status.success(), "import: {}", text(&out.stderr));
    let clean = run(&dir, &["api", "drift", "notes.hl"]);
    assert!(
        clean.status.success(),
        "drift: {}{}",
        text(&clean.stdout),
        text(&clean.stderr)
    );
    std::fs::write(
        dir.join("notes.yaml"),
        NOTES_YAML.replace("operationId: createNote", "operationId: addNote"),
    )
    .unwrap();
    let drifted = run(&dir, &["api", "drift", "notes.hl"]);
    assert!(!drifted.status.success());
    assert!(
        text(&drifted.stdout).contains("add_note"),
        "{}",
        text(&drifted.stdout)
    );
}

#[test]
fn a_yaml_construct_the_reader_refuses_names_its_line() {
    let dir = unique_dir("yaml_refuse");
    std::fs::write(dir.join("a.yaml"), "openapi: 3.0.3\ninfo: &i\n  title: A\n").unwrap();
    let out = run(&dir, &["api", "import", "a.yaml", "--as", "A"]);
    assert!(!out.status.success());
    let err = text(&out.stderr);
    assert!(err.contains("line 2: anchors"), "{err}");
}

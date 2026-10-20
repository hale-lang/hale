//! The client binding (spec/api.md § Calling a surface): a param
//! `api::client(Surface, http::Client { … } | mcp::Client { … })` is a
//! client of the surface, a typed method per row. One program calls a Hale
//! surface over both transports, an imported OpenAPI surface over HTTP and an
//! imported MCP surface over MCP, all against one stub in this process that
//! answers as the served side does (the error classes' statuses and
//! `Hale-Error-Class`, the refusal bodies, MCP's error codes, an event-stream
//! answer and a session id); the test reads what each call answered and what
//! the stub was sent. Beside it: the effect a call carries, which a claim
//! names as `effects(client(Shelf))`; the refusals of a site the compiler
//! cannot build; and the tools a leg acting as some roles is given of a
//! surface. Nothing here reaches past the loopback.

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
        "hale_api_client_{}_{}_{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed),
        tag
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).to_string()
}

fn run(dir: &Path, args: &[&str]) -> Output {
    hale()
        .current_dir(dir)
        .args(args)
        .output()
        .expect("run hale")
}

/// A Hale surface (`Shop`, whose handlers are another program's), an
/// OpenAPI surface and an MCP surface the program imported, and a client of
/// each.
const PROGRAM: &str = r#"
role editor;
role lead includes editor;

type OrderError = enum { @not_found Missing, @conflict Taken(String), Closed };
type Ask { id: Int; }
type Order { id: Int; name: String; }

api Shop {
    rpc Orders::get;
    rpc Orders::put requires: [editor];
}

locus Orders {
    fn get(a: Ask) -> Order fallible(OrderError) {
        fail OrderError::Closed;
    }
    fn put(o: Order) -> Order {
        return o;
    }
}

api Shelf from openapi "./shelf.json" {
    rpc get_item(ShelfGetItemRequest) -> ShelfItem wire: "GET /items/{itemId}";
    rpc create_item(ShelfNewItem) -> ShelfItem wire: "POST /items" requires: [editor];
    rpc list_items() -> std::json::Value wire: "GET /items";
}

type ShelfGetItemRequest {
    item_id: Int `json:"itemId"`;
    has_tag: Bool = false;
    tag: String = "";
}

type ShelfItem {
    id: Int;
    name: String;
}

type ShelfNewItem {
    name: String;
}

api Tools from mcp "http://127.0.0.1:1/mcp" {
    rpc echo(ToolsEchoRequest) -> std::json::Value wire: "echo";
    rpc fail_hard(ToolsEchoRequest) -> std::json::Value wire: "fail-hard";
}

type ToolsEchoRequest {
    said: String;
}

fn order_line(o: ShopOrdersGetOutcome) -> String {
    return match o {
        ShopOrdersGetOutcome::Result(v) -> "result " + to_string(v.id) + " " + v.name,
        ShopOrdersGetOutcome::HandlerError(e) -> match e {
            OrderError::Missing -> "handler_error Missing",
            OrderError::Taken(who) -> "handler_error Taken " + who,
            OrderError::Closed -> "handler_error Closed",
        },
        ShopOrdersGetOutcome::Refusal(r) -> "refusal " + r.kind + " " + r.served,
        ShopOrdersGetOutcome::ServerError -> "server_error",
        ShopOrdersGetOutcome::Lost(why) -> "lost " + why,
    };
}

fn put_line(o: ShopOrdersPutOutcome) -> String {
    return match o {
        ShopOrdersPutOutcome::Result(v) -> "result " + to_string(v.id),
        ShopOrdersPutOutcome::Refusal(r) -> "refusal " + r.kind + " " + r.requires,
        ShopOrdersPutOutcome::ServerError -> "server_error",
        ShopOrdersPutOutcome::Lost(why) -> "lost " + why,
    };
}

fn item_line(o: ShelfGetItemOutcome) -> String {
    return match o {
        ShelfGetItemOutcome::Result(v) -> "result " + to_string(v.id) + " " + v.name,
        ShelfGetItemOutcome::HandlerError(f) -> "failure " + f.class + " " + to_string(f.status) + " " + f.body,
        ShelfGetItemOutcome::Refusal(r) -> "refusal " + r.kind,
        ShelfGetItemOutcome::ServerError -> "server_error",
        ShelfGetItemOutcome::Lost(why) -> "lost " + why,
    };
}

fn made_line(o: ShelfCreateItemOutcome) -> String {
    return match o {
        ShelfCreateItemOutcome::Result(v) -> "result " + to_string(v.id) + " " + v.name,
        ShelfCreateItemOutcome::HandlerError(f) -> "failure " + f.class,
        ShelfCreateItemOutcome::Refusal(r) -> "refusal " + r.kind,
        ShelfCreateItemOutcome::ServerError -> "server_error",
        ShelfCreateItemOutcome::Lost(why) -> "lost " + why,
    };
}

fn list_line(o: ShelfListItemsOutcome) -> String {
    return match o {
        ShelfListItemsOutcome::Result(v) -> "result " + v.text,
        ShelfListItemsOutcome::HandlerError(f) -> "failure " + f.class,
        ShelfListItemsOutcome::Refusal(r) -> "refusal " + r.kind,
        ShelfListItemsOutcome::ServerError -> "server_error",
        ShelfListItemsOutcome::Lost(why) -> "lost " + why,
    };
}

fn echo_line(o: ToolsEchoOutcome) -> String {
    return match o {
        ToolsEchoOutcome::Result(v) -> "result " + v.text,
        ToolsEchoOutcome::HandlerError(f) -> "failure " + f.body,
        ToolsEchoOutcome::Refusal(r) -> "refusal " + r.kind,
        ToolsEchoOutcome::ServerError -> "server_error",
        ToolsEchoOutcome::Lost(why) -> "lost " + why,
    };
}

fn fail_line(o: ToolsFailHardOutcome) -> String {
    return match o {
        ToolsFailHardOutcome::Result(v) -> "result " + v.text,
        ToolsFailHardOutcome::HandlerError(f) -> "failure " + f.body,
        ToolsFailHardOutcome::Refusal(r) -> "refusal " + r.kind,
        ToolsFailHardOutcome::ServerError -> "server_error",
        ToolsFailHardOutcome::Lost(why) -> "lost " + why,
    };
}

main locus App {
    params {
        creds: std::api::VaultPrincipals = std::api::VaultPrincipals { slots: "editor=t-editor, *=t-any" };
        shop: api::Client = api::client(Shop, http::Client { base: std::env::var("HTTP"), principals: self.creds });
        shop_mcp: api::Client = api::client(Shop, mcp::Client { url: std::env::var("HTTP") + "/mcp", principals: self.creds });
        shelf: api::Client = api::client(Shelf, http::Client { base: std::env::var("HTTP"), principals: self.creds });
        tools: api::Client = api::client(Tools, mcp::Client { url: std::env::var("HTTP") + "/mcp" });
    }
    run() {
        let mut id = 1;
        while id <= 6 {
            println("http get " + to_string(id) + ": " + order_line(self.shop.orders_get(Ask { id: id })));
            id = id + 1;
        }
        println("http put: " + put_line(self.shop.orders_put(Order { id: 3, name: "c" })));
        println("mcp get 1: " + order_line(self.shop_mcp.orders_get(Ask { id: 1 })));
        println("mcp get 4: " + order_line(self.shop_mcp.orders_get(Ask { id: 4 })));
        println("mcp put: " + put_line(self.shop_mcp.orders_put(Order { id: 3, name: "c" })));
        println("item 7: " + item_line(self.shelf.get_item(ShelfGetItemRequest { item_id: 7, has_tag: true, tag: "a b" })));
        println("item 8: " + item_line(self.shelf.get_item(ShelfGetItemRequest { item_id: 8 })));
        println("made: " + made_line(self.shelf.create_item(ShelfNewItem { name: "pen" })));
        println("list: " + list_line(self.shelf.list_items()));
        println("echo: " + echo_line(self.tools.echo(ToolsEchoRequest { said: "hi" })));
        println("fail: " + fail_line(self.tools.fail_hard(ToolsEchoRequest { said: "x" })));
    }
}

fn main() { App { }; }
"#;

/// A header's value in a request head, "-" when it has none.
fn header(head: &str, name: &str) -> String {
    let want = format!("{}:", name.to_ascii_lowercase());
    head.lines()
        .find_map(|l| {
            l.to_ascii_lowercase()
                .starts_with(&want)
                .then(|| l[want.len()..].trim().to_string())
        })
        .unwrap_or_else(|| "-".to_string())
}

/// What the stub answers, as the served side would: `Shop` over HTTP (§ The
/// HTTP transport) and over MCP at `/mcp` (§ The MCP transport, with a
/// session and an event-stream answer as another MCP server may send), and
/// the imported surfaces' operations and tools.
fn answer(method: &str, path: &str, body: &str) -> (u16, Vec<(String, String)>, String) {
    let h = |k: &str, v: &str| (k.to_string(), v.to_string());
    let json =
        |s: &str| serde_json::from_str::<serde_json::Value>(s).unwrap_or(serde_json::Value::Null);
    match (method, path) {
        ("POST", "/call/Orders::get") => match json(body)["id"].as_i64() {
            Some(1) => (404, vec![h("Hale-Error-Class", "not_found")], "\"Missing\"".into()),
            Some(2) => (409, vec![h("Hale-Error-Class", "conflict")], "{\"Taken\":\"bob\"}".into()),
            Some(3) => (422, vec![], "\"Closed\"".into()),
            Some(4) => (200, vec![], "{\"id\":4,\"name\":\"x\"}".into()),
            Some(5) => (
                409,
                vec![],
                "{\"refusal\":{\"kind\":\"digest_mismatch\",\"reason\":\"r\",\"served\":\"fnv1a64:0000000000000000\"}}".into(),
            ),
            _ => (500, vec![], "{\"refusal\":{\"kind\":\"server\"}}".into()),
        },
        ("POST", "/call/Orders::put") => {
            (403, vec![], "{\"refusal\":{\"kind\":\"unauthorized\",\"reason\":\"no\",\"requires\":[\"editor\"]}}".into())
        }
        ("GET", p) if p.starts_with("/items/7") => (200, vec![], "{\"id\":7,\"name\":\"lamp\"}".into()),
        ("GET", "/items") => (200, vec![], "[1,2]".into()),
        ("POST", "/items") => (201, vec![], "{\"id\":9,\"name\":\"pen\"}".into()),
        ("POST", "/mcp") => {
            let m = json(body);
            let id = m["id"].clone();
            let reply = |r: serde_json::Value| {
                let mut r = r;
                r["jsonrpc"] = "2.0".into();
                r["id"] = id.clone();
                r.to_string()
            };
            match m["method"].as_str() {
                Some("initialize") => {
                    let msg = reply(serde_json::json!({"result": {"protocolVersion": "2025-06-18", "capabilities": {"tools": {}}, "serverInfo": {"name": "stub", "version": "1"}}}));
                    (
                        200,
                        vec![h("Content-Type", "text/event-stream"), h("Mcp-Session-Id", "s-1")],
                        format!("event: message\ndata: {msg}\n\n"),
                    )
                }
                _ if m.get("id").is_none() => (202, vec![], String::new()),
                _ => {
                    let args = &m["params"]["arguments"];
                    let r = match m["params"]["name"].as_str() {
                        Some("echo") => serde_json::json!({"result": {"content": [{"type": "text", "text": serde_json::json!({"said": args["said"]}).to_string()}], "isError": false}}),
                        Some("fail-hard") => serde_json::json!({"result": {"content": [{"type": "text", "text": "it broke"}], "isError": true}}),
                        Some("Orders__get") if args["id"] == 1 => serde_json::json!({"error": {"code": -32001, "message": "handler_error", "data": "Missing"}}),
                        Some("Orders__get") => serde_json::json!({"result": {"content": [{"type": "text", "text": "{\"id\":4,\"name\":\"x\"}"}], "structuredContent": {"id": 4, "name": "x"}, "isError": false}}),
                        Some("Orders__put") => serde_json::json!({"error": {"code": -32005, "message": "unauthorized", "data": {"kind": "unauthorized", "reason": "no", "requires": ["editor"]}}}),
                        _ => serde_json::json!({"error": {"code": -32602, "message": "unknown tool"}}),
                    };
                    (200, vec![h("Content-Type", "application/json")], reply(r))
                }
            }
        }
        _ => (404, vec![], "gone".into()),
    }
}

#[test]
fn a_client_calls_each_row_and_reads_each_outcome_as_the_served_side_maps_it() {
    let dir = unique_dir("calls");
    std::fs::write(dir.join("main.hl"), PROGRAM).unwrap();
    // the vault the principals read: a slot per role
    let vault = dir.join("vault");
    std::fs::create_dir_all(&vault).unwrap();
    std::fs::write(vault.join("t-editor"), "tok-ed").unwrap();
    std::fs::write(vault.join("t-any"), "tok-any").unwrap();
    let sent: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let log = sent.clone();
    let base = super::api_import::stub(Arc::new(
        move |method: &str, path: &str, head: &str, body: &str| {
            log.lock().unwrap().push(format!(
                "{method} {path} auth={} digest={} session={} version={} body={body}",
                header(head, "Authorization"),
                header(head, "Hale-Surface-Digest"),
                header(head, "Mcp-Session-Id"),
                header(head, "MCP-Protocol-Version"),
            ));
            answer(method, path, body)
        },
    ));
    let built = run(&dir, &["build", "main.hl", "-o", "app"]);
    assert!(
        built.status.success(),
        "the program builds:\n{}",
        text(&built.stderr)
    );
    let out = Command::new(dir.join("app"))
        .env("HTTP", &base)
        .env("HALE_VAULT_DIR", &vault)
        .env_remove("HALE_VAULT_ADDR")
        .output()
        .expect("run the program");
    assert!(
        out.status.success(),
        "{}\n{}",
        text(&out.stdout),
        text(&out.stderr)
    );
    let lines: Vec<String> = text(&out.stdout).lines().map(str::to_string).collect();
    assert_eq!(
        lines,
        [
            // a Hale surface over HTTP: the error classes' statuses read back as
            // the row's own error type, a refusal, the server error
            "http get 1: handler_error Missing",
            "http get 2: handler_error Taken bob",
            "http get 3: handler_error Closed",
            "http get 4: result 4 x",
            "http get 5: refusal digest_mismatch fnv1a64:0000000000000000",
            "http get 6: server_error",
            "http put: refusal unauthorized editor",
            // the same surface over MCP
            "mcp get 1: handler_error Missing",
            "mcp get 4: result 4 x",
            "mcp put: refusal unauthorized editor",
            // an imported OpenAPI surface: the operation's path and query filled
            // from the request, a status read as its error class
            "item 7: result 7 lamp",
            "item 8: failure not_found 404 gone",
            "made: result 9 pen",
            "list: result [1,2]",
            // an imported MCP surface: a tool's text result, and its error
            "echo: result {\"said\":\"hi\"}",
            "fail: failure it broke",
        ]
    );
    let sent = sent.lock().unwrap().clone();
    let first = |prefix: &str| {
        sent.iter()
            .find(|l| l.starts_with(prefix))
            .cloned()
            .unwrap_or_else(|| panic!("no `{prefix}` in {sent:#?}"))
    };
    // the row's member digest, its role's vault slot as the bearer
    let get = first("POST /call/Orders::get ");
    assert!(get.contains("auth=Bearer tok-any digest=fnv1a64:"), "{get}");
    let put = first("POST /call/Orders::put ");
    assert!(put.contains("auth=Bearer tok-ed digest=fnv1a64:"), "{put}");
    // the path parameter and the query from the request's fields; a field
    // left out is no query parameter
    assert!(first("GET /items/7?tag=a%20b ").contains("auth=Bearer tok-any"));
    assert!(first("GET /items/8 ").starts_with("GET /items/8 auth="));
    // a row that requires `editor` presents its slot; the body is the request
    let made = first("POST /items ");
    assert!(
        made.contains("auth=Bearer tok-ed") && made.ends_with("body={\"name\":\"pen\"}"),
        "{made}"
    );
    // MCP: the session opened once, then named with the version on every call;
    // a Hale row's digest rides in `_meta` and the header
    let opened: Vec<&String> = sent
        .iter()
        .filter(|l| l.contains("\"method\":\"initialize\""))
        .collect();
    assert_eq!(opened.len(), 2, "one session per client: {sent:#?}");
    assert!(
        opened.iter().all(|l| l.contains("session=-")),
        "{opened:#?}"
    );
    let call = first("POST /mcp auth=Bearer tok-any digest=fnv1a64:");
    assert!(
        call.contains("session=s-1 version=2025-06-18")
            && call.contains("\"name\":\"Orders__get\",\"arguments\":{\"id\":1},\"_meta\":{\"hale/digest\":\"fnv1a64:"),
        "{call}"
    );
    let echo = sent
        .iter()
        .find(|l| l.contains("\"name\":\"echo\""))
        .expect("echo was called");
    assert!(
        echo.starts_with("POST /mcp auth=- digest=- session=s-1"),
        "no principals, no bearer: {echo}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A program whose desk calls the shelf: a claim over `effects(client(Shelf))`
/// sees each call.
const CLAIMED: &str = r#"
api Shelf from openapi "./shelf.json" {
    rpc get_item(ShelfGetItemRequest) -> ShelfItem wire: "GET /items/{itemId}";
}
type ShelfGetItemRequest { item_id: Int `json:"itemId"`; }
type ShelfItem { id: Int; name: String; }

locus Desk {
    params { shelf: api::Client = api::client(Shelf, http::Client { base: "http://127.0.0.1:1" }); }
    fn look() -> Int {
        return match self.shelf.get_item(ShelfGetItemRequest { item_id: 1 }) {
            ShelfGetItemOutcome::Result(i) -> i.id,
            ShelfGetItemOutcome::HandlerError(f) -> 0,
            ShelfGetItemOutcome::Refusal(r) -> 0,
            ShelfGetItemOutcome::ServerError -> 0,
            ShelfGetItemOutcome::Lost(w) -> 0,
        };
    }
}
locus Quiet { fn hi() -> Int { return 1; } }
group desks = { Desk };
group quiet = { Quiet };

main locus App {
    params { desk: Desk = Desk { }; q: Quiet = Quiet { }; }
    claims {
        desk_offline: forbid reaches(desks, effects(client(Shelf)));
        quiet_offline: forbid reaches(quiet, effects(client(Shelf)));
        no_client: forbid reaches(desks, effects(client(*)));
    }
    run() { println(to_string(self.desk.look() + self.q.hi())); }
}
fn main() { App { }; }
"#;

#[test]
fn a_claim_names_a_surfaces_calls_as_effects_of_its_client() {
    let dir = unique_dir("claims");
    std::fs::write(dir.join("main.hl"), CLAIMED).unwrap();
    let out = run(&dir, &["check", "main.hl"]);
    let err = text(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{err}");
    assert!(
        err.contains("claim `desk_offline` violated: `desks` reaches `effects(client(Shelf))` — witness: `Desk::look` -> `__ApiClient_Shelf::get_item`"),
        "{err}"
    );
    assert!(
        err.contains("claim `no_client` violated"),
        "the family's star names every surface's calls: {err}"
    );
    assert!(
        !err.contains("quiet_offline"),
        "a group that calls nothing holds: {err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_client_the_compiler_cannot_build_is_refused_naming_why() {
    let dir = unique_dir("refused");
    let src = r#"
api Shelf from openapi "./shelf.json" {
    rpc get_item(ShelfGetItemRequest) -> ShelfItem wire: "GET /items/{itemId}";
}
type ShelfGetItemRequest { item_id: Int `json:"itemId"`; }
type ShelfItem { id: Int; name: String; }

main locus App {
    params {
        a: api::Client = api::client(Shelff, http::Client { base: "http://127.0.0.1:1" });
        b: api::Client = api::client(Shelf, mcp::Client { url: "http://127.0.0.1:1/mcp" });
        c: api::Client = api::client(Shelf, std::http::Client { });
    }
    run() { }
}
fn main() { App { }; }
"#;
    std::fs::write(dir.join("main.hl"), src).unwrap();
    let out = run(&dir, &["check", "main.hl"]);
    let err = text(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{err}");
    for want in [
        "client of `Shelff`: no surface `Shelff` is declared (spec/api.md § Calling a surface)",
        "client of `Shelf` over `mcp::Client`: row `get_item` is the HTTP operation `GET /items/{itemId}`; a surface imported from OpenAPI is called over `http::Client`",
        "client of `Shelf`: the transport is a literal of `http::Client { base, principals }` or `mcp::Client { url, principals }`",
    ] {
        assert!(err.contains(want), "missing `{want}` in:\n{err}");
    }
    assert_eq!(
        err.matches("type error").count(),
        3,
        "one refusal per site and nothing else:\n{err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The tool names `hale api export --tools-for` lists.
fn tools_for(dir: &Path, surface: &str, roles: &str) -> Vec<String> {
    let out = run(
        dir,
        &[
            "api",
            "export",
            "--surface",
            surface,
            "--tools-for",
            roles,
            "main.hl",
        ],
    );
    assert!(out.status.success(), "{}", text(&out.stderr));
    let doc: serde_json::Value = serde_json::from_slice(&out.stdout).expect("the tools are JSON");
    assert_eq!(doc["x-hale-surface"], surface);
    doc["tools"]
        .as_array()
        .expect("a tools array")
        .iter()
        .map(|t| {
            assert!(
                t["inputSchema"]["type"] == "object",
                "every tool takes an object: {t}"
            );
            t["name"].as_str().unwrap_or("").to_string()
        })
        .collect()
}

#[test]
fn a_legs_tools_are_the_rows_its_roles_admit_namespaced_by_surface() {
    let dir = unique_dir("tools");
    std::fs::write(dir.join("main.hl"), PROGRAM).unwrap();
    assert_eq!(
        tools_for(&dir, "Shelf", ""),
        ["Shelf.get_item", "Shelf.list_items"]
    );
    assert_eq!(
        tools_for(&dir, "Shelf", "editor"),
        ["Shelf.create_item", "Shelf.get_item", "Shelf.list_items"]
    );
    // a role holds every role it includes
    assert_eq!(
        tools_for(&dir, "Shelf", "lead"),
        ["Shelf.create_item", "Shelf.get_item", "Shelf.list_items"]
    );
    assert_eq!(tools_for(&dir, "Shop", ""), ["Shop.orders_get"]);
    assert_eq!(
        tools_for(&dir, "Shop", "editor"),
        ["Shop.orders_get", "Shop.orders_put"]
    );
    let both = run(
        &dir,
        &[
            "api",
            "export",
            "--surface",
            "Shop",
            "--tools-for",
            "editor",
            "--out",
            "x",
            "main.hl",
        ],
    );
    assert_eq!(both.status.code(), Some(2), "{}", text(&both.stderr));
    let _ = std::fs::remove_dir_all(&dir);
}

// The codegen suite's: what a child listens on, read from `/proc`.
#[path = "../../hale-codegen/tests/support/child.rs"]
#[allow(dead_code)]
mod child;

/// The surface both programs declare: the server answers it, the client is
/// typed by it.
const ROUND_TRIP_SURFACE: &str = r#"
type OrderError = enum { @not_found Missing, @conflict Taken(String), @invalid Bad, @forbidden Nope, @unavailable Busy, Closed };
type Ask { id: Int; }
type Order { id: Int; name: String; }

api Shop {
    rpc Orders::get;
}

locus Orders {
    fn get(a: Ask) -> Order fallible(OrderError) {
        if a.id == 1 { fail OrderError::Missing; }
        if a.id == 2 { fail OrderError::Taken("bob"); }
        if a.id == 3 { fail OrderError::Bad; }
        if a.id == 4 { fail OrderError::Nope; }
        if a.id == 5 { fail OrderError::Busy; }
        if a.id == 6 { fail OrderError::Closed; }
        return Order { id: a.id, name: "ok" };
    }
}
"#;

const ROUND_TRIP_SERVER: &str = r#"
locus Tokens {
    fn principal(token: String) -> std::api::Principal { return std::api::Principal { mode: "bearer", name: token }; }
    fn refused() -> String { return "no such token"; }
}

main locus Server {
    params {
        bearer: Tokens = Tokens { };
        orders: Orders = Orders { };
        web: api::Exposure = api::serve(Shop, http::Rpc { bind: "127.0.0.1:0", codec: json, principals: self.bearer }, as: "web", receivers: { Orders: self.orders }, bound: 8, on_full: refuse);
    }
    run() {
        while !self.draining { std::time::sleep(10ms); }
        self.web.stop();
    }
}

fn main() { Server { }; }
"#;

const ROUND_TRIP_CLIENT: &str = r#"
locus Me {
    fn bearer(role: String) -> String { return "caller-1"; }
}

fn line(o: ShopOrdersGetOutcome) -> String {
    return match o {
        ShopOrdersGetOutcome::Result(v) -> "result " + to_string(v.id) + " " + v.name,
        ShopOrdersGetOutcome::HandlerError(e) -> match e {
            OrderError::Missing -> "Missing",
            OrderError::Taken(who) -> "Taken " + who,
            OrderError::Bad -> "Bad",
            OrderError::Nope -> "Nope",
            OrderError::Busy -> "Busy",
            OrderError::Closed -> "Closed",
        },
        ShopOrdersGetOutcome::Refusal(r) -> "refusal " + r.kind + " " + r.reason,
        ShopOrdersGetOutcome::ServerError -> "server_error",
        ShopOrdersGetOutcome::Lost(why) -> "lost " + why,
    };
}

main locus Caller {
    params {
        me: Me = Me { };
        shop: api::Client = api::client(Shop, http::Client { base: std::env::var("SHOP"), principals: self.me });
    }
    run() {
        let mut id = 1;
        while id <= 7 {
            println(to_string(id) + ": " + line(self.shop.orders_get(Ask { id: id })));
            id = id + 1;
        }
    }
}

fn main() { Caller { }; }
"#;

/// Two Hale programs: one serves `Shop` over `http::Rpc`, the other holds a
/// client of the same surface. Every error class the handler fails with
/// reaches the caller as the variant it failed with.
#[test]
fn a_client_reads_a_served_surfaces_outcomes_as_the_server_wrote_them() {
    let dir = unique_dir("round_trip");
    std::fs::write(
        dir.join("server.hl"),
        format!("{ROUND_TRIP_SURFACE}{ROUND_TRIP_SERVER}"),
    )
    .unwrap();
    std::fs::write(
        dir.join("caller.hl"),
        format!("{ROUND_TRIP_SURFACE}{ROUND_TRIP_CLIENT}"),
    )
    .unwrap();
    for (src, bin) in [("server.hl", "server"), ("caller.hl", "caller")] {
        let built = run(&dir, &["build", src, "-o", bin]);
        assert!(
            built.status.success(),
            "{src} builds:\n{}",
            text(&built.stderr)
        );
    }
    let mut server = Command::new(dir.join("server"))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("start the server");
    // the server binds port 0; its port is read back from the kernel
    let port = child::tcp_port(&mut server);
    let out = Command::new(dir.join("caller"))
        .env("SHOP", format!("http://127.0.0.1:{port}"))
        .output()
        .expect("run the caller");
    let _ = server.kill();
    let _ = server.wait();
    assert!(
        out.status.success(),
        "{}\n{}",
        text(&out.stdout),
        text(&out.stderr)
    );
    let lines: Vec<String> = text(&out.stdout).lines().map(str::to_string).collect();
    assert_eq!(
        lines,
        [
            "1: Missing",
            "2: Taken bob",
            "3: Bad",
            "4: Nope",
            "5: Busy",
            "6: Closed",
            "7: result 7 ok"
        ],
        "each class's status, read back as the handler's own variant"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

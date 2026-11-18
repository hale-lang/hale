//! GH #1674: `///` doc comments carried into every document a surface
//! generates (spec/api.md § Documenting a surface).
//!
//! The fixture program under `fixtures/api_doc_comments/` documents its
//! surface, rows, handlers, request parameter, types, fields and variants.
//! The committed files beside it are what `hale check --api` and `hale api
//! client` write for it, byte for byte: the inventory, one exposure's
//! description, the OpenAPI, JSON Schema, MCP and `.proto` forms and the
//! three clients (the Hale client as `orders.client.hl.txt`, so the corpus,
//! which reads every `.hl` under the fixtures, does not take it for a
//! program). Regenerate deliberately:
//!
//!     HALE_REGEN_API_DOC_COMMENTS=1 cargo test -p hale-cli --test tooling_services api_doc_comments::
//!
//! Prose is not shape: the same program with every doc comment removed has
//! the same surface digest and member digests, and only the prose differs
//! between the two documents. A doc comment in a position nothing reads is
//! a warning, and the check still passes.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

fn hale() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_hale"));
    c.env("HALE_SKIP_STALE_CHECK", "1");
    c
}

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/api_doc_comments")
}

fn run(args: &[&str], file: &Path) -> String {
    let out = hale().args(args).arg(file).output().expect("run hale");
    assert!(out.status.success(), "hale {args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).expect("utf8")
}

fn check_baseline(name: &str, args: &[&str]) -> String {
    let got = run(args, &fixture_dir().join("main.hl"));
    let path = fixture_dir().join(name);
    if std::env::var("HALE_REGEN_API_DOC_COMMENTS").is_ok() {
        std::fs::write(&path, &got).expect("write baseline");
        return got;
    }
    let want = std::fs::read_to_string(&path).expect("baseline present");
    assert!(
        got == want,
        "{name} moved from its committed baseline. If that is intended, regenerate with \
         HALE_REGEN_API_DOC_COMMENTS=1 cargo test -p hale-cli --test tooling_services api_doc_comments::\n--- got\n{got}"
    );
    got
}

fn json(text: &str) -> Value {
    serde_json::from_str(text).expect("json")
}

/// The fixture with every `///` line removed, in a scratch file of its own.
fn undocumented() -> PathBuf {
    let src = std::fs::read_to_string(fixture_dir().join("main.hl")).expect("fixture");
    let stripped: String = src.lines().filter(|l| !l.trim_start().starts_with("///")).map(|l| format!("{l}\n")).collect();
    assert_ne!(src, stripped, "the fixture documents something");
    let dir = std::env::temp_dir().join(format!("hale-api-doc-comments-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch");
    let path = dir.join("main.hl");
    std::fs::write(&path, stripped).expect("write");
    path
}

#[test]
fn the_inventory_and_a_description_carry_the_docs() {
    let inv = json(&check_baseline("inventory.json", &["check", "--api"]));
    let errs = super::api_contract_fixtures::errors_against_schema(&inv);
    assert!(errs.is_empty(), "the inventory does not conform: {errs:?}");
    let surface = &inv["surfaces"][0];
    assert_eq!(surface["description"], "The order desk's public surface.");
    let members = surface["members"].as_array().unwrap();
    let member = |n: &str| members.iter().find(|m| m["name"] == n).unwrap();
    // the row's doc replaces its handler's
    assert_eq!(member("Desk::place")["description"], "Place an order.\nIt is filled at the limit or better; a desk that is closed refuses it.");
    // a row with no doc of its own takes its handler's; one with neither has none
    assert_eq!(member("Desk::cancel")["description"], "Cancel an order.");
    assert!(member("Desk::count").get("description").is_none());
    // the request parameter's doc, on the request
    assert_eq!(member("Desk::place")["request"]["description"], "The order to place.");
    let place = &inv["schemas"]["PlaceOrder"];
    assert_eq!(place["description"], "A request to place an order.\n\nThe desk fills it at the limit or better.");
    assert_eq!(place["properties"]["symbol"]["description"], "The instrument's ticker symbol.");
    // a field with no doc of its own states its scalar's
    assert_eq!(place["properties"]["limit"]["description"], "An amount of money, counted in cents.");
    // an enum inline: the field's doc over the enum's, and its variants'
    assert_eq!(place["properties"]["origin"]["description"], "Where the order came from.");
    assert_eq!(place["properties"]["origin"]["x-hale-variant-descriptions"]["Desk"], "Typed by a person.");
    assert!(inv["schemas"]["Ticket"].get("description").is_none(), "an undocumented type has no description");

    let d = json(&check_baseline("exposure.json", &["check", "--api", "--exposure", "orders", "--caller", "uid:1000", "--holds", "clerk"]));
    let errs = super::api_contract_fixtures::errors_against_schema(&d);
    assert!(errs.is_empty(), "the description does not conform: {errs:?}");
    let error = &d["members"].as_array().unwrap().iter().find(|m| m["name"] == "Desk::place").unwrap()["error"];
    assert_eq!(error["description"], "Why an order was not placed.");
    assert_eq!(error["x-hale-variant-descriptions"]["Closed"], "The desk is closed for the day.");
}

#[test]
fn the_openapi_form_puts_the_first_line_in_summary_and_the_rest_in_description() {
    let d = json(&check_baseline("Orders.openapi.json", &["check", "--api", "--surface", "Orders", "--openapi"]));
    assert!(d["info"]["description"].as_str().unwrap().starts_with("The order desk's public surface.\n\n"));
    let place = &d["paths"]["/call/Desk::place"]["post"];
    assert_eq!(place["summary"], "Place an order.");
    let description = place["description"].as_str().unwrap();
    assert!(description.starts_with("It is filled at the limit or better"), "{description}");
    assert!(description.ends_with("the server still authorizes every request."), "{description}");
    assert_eq!(place["requestBody"]["description"], "The order to place.");
    // a one-line doc is the summary alone
    let cancel = &d["paths"]["/call/Desk::cancel"]["post"];
    assert_eq!(cancel["summary"], "Cancel an order.");
    assert!(cancel["description"].as_str().unwrap().starts_with("Requires no role"));
    // no doc: the summary is the row's name, as before
    assert_eq!(d["paths"]["/call/Desk::count"]["post"]["summary"], "rpc Desk::count");
    assert_eq!(d["components"]["schemas"]["Receipt"]["properties"]["order"]["description"], "The order's number on the desk.");
}

#[test]
fn the_json_schema_and_mcp_forms_carry_the_docs() {
    let d = json(&check_baseline("Orders.json-schema.json", &["check", "--api", "--surface", "Orders", "--json-schema"]));
    assert_eq!(d["description"], "The order desk's public surface.");
    assert_eq!(d["$defs"]["PlaceOrder"]["properties"]["qty"]["description"], "How many units to buy.");
    assert_eq!(d["x-hale-members"]["Desk::place"]["description"].as_str().unwrap().lines().next(), Some("Place an order."));

    let d = json(&check_baseline("Orders.mcp.json", &["check", "--api", "--surface", "Orders", "--mcp"]));
    let tools = d["tools"].as_array().unwrap();
    let tool = |n: &str| tools.iter().find(|t| t["name"] == n).unwrap();
    let place = tool("Desk__place");
    assert!(place["description"].as_str().unwrap().starts_with("Place an order.\nIt is filled"));
    assert!(place["description"].as_str().unwrap().contains("rpc Desk::place of Orders"));
    // the tool's input is the request: the parameter's doc, and each field's
    assert_eq!(place["inputSchema"]["description"], "The order to place.");
    assert_eq!(place["inputSchema"]["properties"]["symbol"]["description"], "The instrument's ticker symbol.");
    assert!(tool("Desk__count")["description"].as_str().unwrap().starts_with("rpc Desk::count"));
}

#[test]
fn the_proto_and_the_clients_carry_the_docs() {
    let proto = check_baseline("Orders.proto", &["check", "--api", "--surface", "Orders", "--proto"]);
    assert!(proto.contains("// A request to place an order.\n//\n// The desk fills it at the limit or better.\nmessage PlaceOrder {"), "{proto}");
    assert!(proto.contains("  // The instrument's ticker symbol.\n  optional string symbol = 1;"), "{proto}");
    assert!(proto.contains("  // Typed by a person.\n  Origin_Desk = 0;"), "{proto}");
    assert!(proto.contains("  // Place an order.\n"), "{proto}");

    let ts = check_baseline("orders.client.ts", &["api", "client", "--surface", "Orders", "--lang", "ts"]);
    assert!(ts.contains("export interface PlaceOrder {\n  /** The instrument's ticker symbol. */\n  symbol: string;"), "{ts}");
    assert!(ts.contains("/** Cancel an order. */\nexport function deskCancel("), "{ts}");
    let js = check_baseline("orders.client.js", &["api", "client", "--surface", "Orders", "--lang", "js"]);
    assert!(js.contains(" * `symbol`: The instrument's ticker symbol.\n"), "{js}");
    let hl = check_baseline("orders.client.hl.txt", &["api", "client", "--surface", "Orders", "--lang", "hale"]);
    assert!(hl.contains("    /// The instrument's ticker symbol.\n    symbol: String;"), "{hl}");
    assert!(hl.contains("/// Cancel an order.\nfn desk_cancel("), "{hl}");
}

#[test]
fn doc_comments_move_no_digest() {
    let plain = undocumented();
    let with = json(&run(&["check", "--api"], &fixture_dir().join("main.hl")));
    let without = json(&run(&["check", "--api"], &plain));
    assert_eq!(with["surfaces"][0]["digest"], without["surfaces"][0]["digest"], "the surface digest is the shapes'");
    let digests = |inv: &Value| -> Vec<(String, String)> {
        inv["surfaces"][0]["members"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| (m["name"].as_str().unwrap().to_string(), m["digest"].as_str().unwrap().to_string()))
            .collect()
    };
    assert_eq!(digests(&with), digests(&without), "a member's digest is its shapes'");
    // and everything else is the same document, the prose aside
    fn strip(v: &Value) -> Value {
        match v {
            Value::Object(m) => Value::Object(
                m.iter()
                    .filter(|(k, _)| *k != "description" && *k != "x-hale-variant-descriptions")
                    .map(|(k, x)| (k.clone(), strip(x)))
                    .collect(),
            ),
            Value::Array(a) => Value::Array(a.iter().map(strip).collect()),
            other => other.clone(),
        }
    }
    assert_eq!(strip(&with), strip(&without));
    // the committed bundle's drift check still sees a doc change: the
    // generated files differ while the digest holds
    let a = run(&["check", "--api", "--surface", "Orders", "--openapi"], &fixture_dir().join("main.hl"));
    let b = run(&["check", "--api", "--surface", "Orders", "--openapi"], &plain);
    assert_ne!(a, b);
    let _ = std::fs::remove_dir_all(plain.parent().unwrap());
}

#[test]
fn a_doc_comment_nothing_reads_is_a_warning() {
    let dir = std::env::temp_dir().join(format!("hale-stray-doc-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch");
    let path = dir.join("main.hl");
    std::fs::write(
        &path,
        "/// The answer.\nfn answer() -> Int {\n    /// a statement is not documented\n    let x = 42;\n    return x; /// nor is a trailing note\n}\n\nfn main() { println(answer()); }\n",
    )
    .expect("write");
    let out = hale().arg("check").arg(&path).output().expect("run hale");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "a stray doc comment is no error: {stderr}");
    assert!(stderr.contains("this doc comment documents nothing"), "{stderr}");
    assert!(stderr.contains("main.hl:3:"), "the warning is located at the stray comment: {stderr}");
    assert!(stderr.contains("this doc comment follows code on its line"), "{stderr}");
    assert_eq!(stderr.matches("doc comment").count(), 2, "the fn's own doc is read, not warned at: {stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

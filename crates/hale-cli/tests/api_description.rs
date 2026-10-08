//! GH #1107, rows in GH #1417 (R1, R4): the description of a served
//! surface, the model's first wire form (spec/model.md § "The
//! description", spec/api.md § The description).
//!
//! The fixture program under `fixtures/api_description/` describes itself
//! in the same bytes every time, as the program-wide inventory, as one
//! exposure's description for a caller, and in the OpenAPI 3.1 and MCP
//! forms derived from a surface's rows; the committed files beside it are
//! the baseline. Regenerate deliberately:
//!
//!     HALE_REGEN_API_DESCRIPTION=1 cargo test -p hale-cli --test tooling_services api_description::
//!
//! What this file held for the structural binding (the `--dump-api`
//! document, a running socket's served description, `hale mcp --app`,
//! `hale admin`, `hale call`) is gone with the binding; the generic
//! clients are held beside the clients (R4 C).

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

fn hale() -> Command {
    Command::new(env!("CARGO_BIN_EXE_hale"))
}

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/api_description")
}

/// `hale check --api <flags> main.hl`: stdout, which is the document alone.
fn describe(flags: &[&str]) -> String {
    let out = hale()
        .arg("check")
        .arg("--api")
        .args(flags)
        .arg(fixture_dir().join("main.hl"))
        .output()
        .expect("hale check --api");
    assert!(
        out.status.success(),
        "hale check --api {:?} failed: {}",
        flags,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("utf8")
}

fn check_baseline(name: &str, flags: &[&str]) {
    let got = describe(flags);
    let path = fixture_dir().join(name);
    if std::env::var("HALE_REGEN_API_DESCRIPTION").is_ok() {
        std::fs::write(&path, &got).expect("write baseline");
        return;
    }
    let want = std::fs::read_to_string(&path).expect("baseline present");
    assert!(
        got == want,
        "{} moved from its committed baseline. If that is intended, regenerate with \
         HALE_REGEN_API_DESCRIPTION=1 cargo test -p hale-cli --test tooling_services api_description::\n--- got\n{}",
        name,
        got
    );
    // Byte-reproducible: a second run is the same bytes.
    assert_eq!(describe(flags), got, "{} is not reproducible", name);
}

#[test]
fn the_inventory_matches_its_baseline() {
    check_baseline("inventory.json", &[]);
    let d: Value = serde_json::from_str(&describe(&[])).expect("json");
    assert_eq!(d["app"], "Shop");
    let members: Vec<&str> = d["surfaces"][0]["members"].as_array().unwrap().iter().map(|m| m["name"].as_str().unwrap()).collect();
    assert_eq!(members, ["Billing::on_audit", "Billing::on_count", "Billing::on_refund", "Billing::read_ledger"]);
    // A scalar reply is inlined, never a $ref to a schema that does not exist.
    assert_eq!(d["surfaces"][0]["members"][1]["response"], serde_json::json!({ "type": "integer" }));
    assert_eq!(d["surfaces"][0]["members"][0]["response"], Value::Null);
    assert_eq!(d["surfaces"][0]["members"][0]["requires"], serde_json::json!(["auditor"]));
}

#[test]
fn an_exposures_description_matches_its_baseline_for_a_caller() {
    check_baseline("exposure.json", &["--exposure", "books", "--caller", "uid:1000", "--holds", "auditor"]);
    let d: Value = serde_json::from_str(&describe(&["--exposure", "books", "--caller", "uid:1000", "--holds", "auditor"])).expect("json");
    assert_eq!(d["caller"]["roles"], serde_json::json!(["auditor"]));
    assert_eq!(d["members"].as_array().unwrap().len(), 4, "a holder sees every row");
    // The same program for a caller who holds nothing: the gated rows are not listed.
    let d: Value = serde_json::from_str(&describe(&["--exposure", "books", "--caller", "uid:1001"])).expect("json");
    let names: Vec<&str> = d["members"].as_array().unwrap().iter().map(|m| m["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Billing::on_count", "Billing::on_refund"]);
}

#[test]
fn the_openapi_form_matches_its_baseline_and_validates() {
    check_baseline("openapi.json", &["--surface", "Books", "--openapi"]);
    let d: Value = serde_json::from_str(&describe(&["--surface", "Books", "--openapi"])).expect("json");
    assert_eq!(d["openapi"], "3.1.0");
    let paths = d["paths"].as_object().unwrap();
    assert_eq!(paths.len(), 4);
    // A scalar reply is inlined, never a $ref to a component that does not exist.
    assert_eq!(
        paths["/call/Billing::on_count"]["post"]["responses"]["200"]["content"]["application/json"]["schema"],
        serde_json::json!({ "type": "integer" })
    );
    for m in ["on_audit", "on_count", "on_refund", "read_ledger"] {
        let op = &paths[&format!("/call/Billing::{}", m)]["post"];
        assert_eq!(op["operationId"], format!("Billing::{}", m));
        assert!(op["requestBody"]["content"]["application/json"]["schema"]["$ref"].is_string());
        assert!(op["responses"]["200"].is_object() && op["responses"]["400"].is_object() && op["responses"]["403"].is_object());
    }
    // Every $ref resolves under components.schemas.
    let schemas = d["components"]["schemas"].as_object().unwrap();
    fn refs(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::Object(m) => {
                if let Some(s) = m.get("$ref").and_then(Value::as_str) {
                    out.push(s.to_string());
                }
                m.values().for_each(|x| refs(x, out));
            }
            Value::Array(a) => a.iter().for_each(|x| refs(x, out)),
            _ => {}
        }
    }
    let mut all = Vec::new();
    refs(&d, &mut all);
    assert!(!all.is_empty());
    for r in all {
        let name = r.strip_prefix("#/components/schemas/").expect(&r);
        assert!(schemas.contains_key(name), "unresolved {}", r);
    }
    assert!(schemas.contains_key("hale.Refusal"));
    assert!(d["info"]["description"].as_str().unwrap().contains("boundary check"));
}

#[test]
fn the_mcp_form_matches_its_baseline() {
    check_baseline("mcp.json", &["--surface", "Books", "--mcp"]);
    let d: Value = serde_json::from_str(&describe(&["--surface", "Books", "--mcp"])).expect("json");
    let tools: Vec<&str> = d["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(tools, ["Billing__on_audit", "Billing__on_count", "Billing__on_refund", "Billing__read_ledger"]);
    // A tool's input schema is self-contained: the nested type rides in $defs.
    let refund = &d["tools"][2]["inputSchema"];
    assert_eq!(refund["properties"]["money"]["$ref"], "#/$defs/Money");
    assert!(refund["$defs"]["Money"].is_object());
    assert_eq!(d["tools"][0]["x-hale-requires"], serde_json::json!(["auditor"]));
}

#[test]
fn a_program_that_serves_nothing_describes_an_empty_inventory() {
    let dir = std::env::temp_dir().join(format!("hale_api_desc_none_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("plain.hl");
    std::fs::write(&f, "fn main() { println(\"hi\"); }\n").unwrap();
    let out = hale().arg("check").arg("--api").arg(&f).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let d: Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(d["surfaces"], serde_json::json!([]));
    assert_eq!(d["exposures"], serde_json::json!([]));
    let _ = std::fs::remove_dir_all(&dir);
}

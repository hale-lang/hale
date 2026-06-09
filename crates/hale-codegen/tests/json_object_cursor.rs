//! Single-pass JSON object member cursor (2026-06-09) — the JSON Tier 2
//! substrate. `std::json::object_first` / `object_next` walk `{...}`
//! members once, exposing each key/value as a source range (no substring
//! alloc), so a schema-specialized parser dispatches on the key and reads
//! the value in one pass instead of an O(N) `find_*_field` rescan per
//! field. Validates: out-of-order keys, whitespace, negatives, missing
//! fields, and nested objects/arrays skipped by the depth scan.

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use hale_codegen::build_executable;

fn build_and_run(name: &str, src: &str) -> (String, std::process::ExitStatus) {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let mut bin = std::env::temp_dir();
    bin.push(format!("lt-json-obj-{}-{}-{}.bin", name, std::process::id(), nanos));
    let program = hale_syntax::parse_source(src).expect("parse");
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (String::from_utf8_lossy(&out.stdout).to_string(), out.status)
}

const PARSER: &str = r#"
    fn parse(body: String) {
        let mut id = 0;
        let mut px = 0;
        let mut active = false;
        let mut side = "";
        let mut it = std::json::object_first(body);
        while !it.done {
            if std::json::obj_key_eq(it, body, "id") {
                id = std::json::obj_value_int(it, body);
            } else if std::json::obj_key_eq(it, body, "price") {
                px = std::json::obj_value_int(it, body);
            } else if std::json::obj_key_eq(it, body, "active") {
                active = std::json::obj_value_bool(it, body);
            } else if std::json::obj_key_eq(it, body, "side") {
                side = std::json::obj_value_string(it, body);
            }
            it = std::json::object_next(it, body);
        }
        println("id=", to_string(id), " px=", to_string(px),
                " active=", to_string(active), " side=", side);
    }
"#;

#[test]
fn object_cursor_single_pass_extracts_fields() {
    let src = format!(
        r#"{PARSER}
        fn main() {{
            // nested object on an unmatched key must be skipped whole
            parse("{{\"id\": 7, \"price\": 250, \"active\": true, \"side\": \"buy\", \"meta\": {{\"a\": 1, \"b\": [1,2,3]}}}}");
            // out of order, whitespace, negative, missing `active`
            parse("{{ \"side\":\"sell\" , \"price\" : -3 , \"id\":42 }}");
        }}
    "#
    );
    let (out, status) = build_and_run("basic", &src);
    assert!(status.success(), "run failed: {}", out);
    assert!(out.contains("id=7 px=250 active=true side=buy"), "record 1 wrong:\n{}", out);
    assert!(out.contains("id=42 px=-3 active=false side=sell"), "record 2 wrong:\n{}", out);
}

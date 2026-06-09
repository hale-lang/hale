//! JSON Tier 2 (2026-06-09): compiler-generated `Type::from_json`. A struct
//! with `json:` field tags gets a single-pass schema-specialized parser
//! generated from the tags (driving the object cursor). End-to-end: all
//! scalar types, key remapping, missing-required raises, declared default
//! fills a missing field, nested objects skipped.

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use hale_codegen::build_executable;

fn build_and_run(name: &str, src: &str) -> (String, std::process::ExitStatus) {
    let n = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let mut bin = std::env::temp_dir();
    bin.push(format!("lt-fromjson-{}-{}-{}.bin", name, std::process::id(), n));
    let program = hale_syntax::parse_source(src).expect("parse");
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (String::from_utf8_lossy(&out.stdout).to_string(), out.status)
}

const TYPE: &str = r#"
    type Order {
        id: Int      `json:"id"`;
        price: Int   `json:"px"`;
        qty: Float   `json:"sz"`;
        active: Bool `json:"on"`;
        side: String `json:"side"`;
        note: String = "none";
    }
"#;

#[test]
fn from_json_parses_all_scalar_types_and_remaps_keys() {
    let src = format!(
        r#"{TYPE}
        fn main() {{
            let o = Order::from_json("{{\"id\": 7, \"px\": 250, \"sz\": 1.5, \"on\": true, \"side\": \"buy\", \"meta\": {{\"x\": 1}}}}") or raise;
            println("id=", to_string(o.id), " px=", to_string(o.price), " sz=", to_string(o.qty),
                    " on=", to_string(o.active), " side=", o.side, " note=", o.note);
        }}
    "#
    );
    let (out, status) = build_and_run("ok", &src);
    assert!(status.success(), "run failed: {}", out);
    assert!(out.contains("id=7 px=250 sz=1.5 on=true side=buy note=none"),
        "wrong parse (note should default):\n{}", out);
}

#[test]
fn from_json_raises_on_missing_required_and_defaults_optional() {
    // `px` required + absent -> raise -> fallback branch. `note` has a
    // declared default so its absence is fine.
    let src = format!(
        r#"{TYPE}
        fn main() {{
            let bad = Order::from_json("{{\"id\": 1, \"sz\": 0.0, \"on\": false, \"side\": \"x\"}}")
                or Order {{ id: -1, price: -1, qty: 0.0, active: false, side: "ERR", note: "ERR" }};
            println("side=", bad.side);
        }}
    "#
    );
    let (out, status) = build_and_run("missing", &src);
    assert!(status.success(), "run failed: {}", out);
    assert!(out.contains("side=ERR"), "missing required px should have raised:\n{}", out);
}

#[test]
fn from_json_recurses_into_nested_json_structs() {
    // `home: Addr` where Addr is itself a generated JSON struct: the outer
    // parser hands the nested object's raw text to Addr's parser, and a
    // missing nested field propagates the error out.
    let src = r#"
        type Addr { city: String `json:"city"`; zip: Int `json:"zip"`; }
        type Person {
            name: String `json:"name"`;
            home: Addr   `json:"home"`;
        }
        fn main() {
            let p = Person::from_json("{\"name\": \"Ada\", \"home\": {\"city\": \"London\", \"zip\": 1234}}") or raise;
            println("name=", p.name, " city=", p.home.city, " zip=", to_string(p.home.zip));
            let bad = Person::from_json("{\"name\": \"X\", \"home\": {\"city\": \"NoZip\"}}")
                or Person { name: "ERR", home: Addr { city: "ERR", zip: -1 } };
            println("bad=", bad.name);
        }
    "#;
    let (out, status) = build_and_run("nested", src);
    assert!(status.success(), "run failed: {}", out);
    assert!(out.contains("name=Ada city=London zip=1234"), "nested parse wrong:\n{}", out);
    assert!(out.contains("bad=ERR"), "missing nested field should propagate:\n{}", out);
}

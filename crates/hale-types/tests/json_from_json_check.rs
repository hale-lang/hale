//! JSON Tier 2 typecheck (2026-06-09): after the generator runs,
//! `Type::from_json` resolves to a `fallible(JsonError)` function, so an
//! unaddressed call is flagged by the two-channel rule (and an addressed
//! one is clean). Mirrors the CLI pipeline: generate parsers, then check.

use hale_syntax::json_gen::generate_json_parsers;
use hale_syntax::parse_source;

fn check(src: &str) -> Vec<String> {
    let mut prog = parse_source(src).expect("parse failed");
    generate_json_parsers(&mut prog);
    hale_types::check_program(&prog).into_iter().map(|d| d.message).collect()
}

#[test]
fn unaddressed_from_json_is_flagged() {
    let msgs = check(
        r#"
        type Order { id: Int `json:"id"`; }
        fn f() { let o = Order::from_json("{}"); let _ = o.id; }
    "#,
    );
    assert!(
        msgs.iter().any(|m| m.contains("error not addressed")),
        "unaddressed from_json must be flagged; got: {:?}",
        msgs
    );
}

#[test]
fn addressed_from_json_is_clean() {
    let msgs = check(
        r#"
        type Order { id: Int `json:"id"`; }
        fn f() -> Int { let o = Order::from_json("{}") or raise; return o.id; }
    "#,
    );
    assert!(
        !msgs.iter().any(|m| m.contains("error not addressed")),
        "addressed from_json must be clean; got: {:?}",
        msgs
    );
}

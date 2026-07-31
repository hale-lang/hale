//! `LOTUS_LTO` mode selection (#322 follow-on, 2026-07-31).
//!
//! `thin` selects ThinLTO, `1`/`full` monolithic LTO, anything else
//! off. The modes must be mutually exclusive and must never engage
//! under a sanitizer or the wasm target, where the LTO link either
//! conflicts with the sanitizer runtime or is meaningless.

use hale_codegen::build_executable;
use hale_syntax::parse_source;

#[path = "support/harness.rs"]
mod harness;

const SRC: &str = r#"
    fn main() {
        let b = std::bytes::from_string("hello");
        println(std::bytes::at(b, 0));
    }
"#;

fn builds_under(var: Option<&str>, name: &str) -> bool {
    let program = parse_source(SRC).expect("parse");
    let bin = harness::unique_bin(name);
    match var {
        Some(v) => std::env::set_var("LOTUS_LTO", v),
        None => std::env::remove_var("LOTUS_LTO"),
    }
    let ok = build_executable(&program, &bin).is_ok();
    std::env::remove_var("LOTUS_LTO");
    let _ = std::fs::remove_file(&bin);
    ok
}

/// Each accepted spelling produces a working binary. ThinLTO is the
/// recommended flavor: measured at least as good as full LTO on
/// runtime (json_parse -10.9% vs -6.0%, median of 15) at a similar
/// link cost.
#[test]
fn every_lto_spelling_builds() {
    for v in [None, Some("thin"), Some("1"), Some("full")] {
        assert!(
            builds_under(v, "lto_modes"),
            "LOTUS_LTO={:?} must produce a working build",
            v
        );
    }
}

/// An unrecognized value is OFF, not an error and not a silent
/// upgrade to some LTO flavor.
#[test]
fn unknown_lto_value_is_off_not_an_error() {
    assert!(
        builds_under(Some("yes-please"), "lto_modes_unknown"),
        "an unrecognized LOTUS_LTO must fall back to a normal build"
    );
}

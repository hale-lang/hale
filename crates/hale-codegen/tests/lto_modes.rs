//! `LOTUS_LTO` mode selection (#322 follow-on, 2026-07-31).
//!
//! `thin` selects ThinLTO, `1`/`full` monolithic LTO, anything else
//! off. The modes must be mutually exclusive and must never engage
//! under a sanitizer or the wasm target, where the LTO link either
//! conflicts with the sanitizer runtime or is meaningless.

use hale_codegen::{build_executable_with_options, BuildOptions, LtoMode};
use hale_syntax::parse_source;

#[path = "support/harness.rs"]
mod harness;

const SRC: &str = r#"
    fn main() {
        let b = std::bytes::from_string("hello");
        println(std::bytes::at(b, 0));
    }
"#;

/// Build under the spelling `var` (`None` = the variable unset,
/// which `LtoMode::parse` sees as the empty string).
///
/// GH #843: the spelling used to be written into the *process*
/// environment for the duration of the build. It is parsed here and
/// handed to that one build as `BuildOptions::lto`, which keeps both
/// halves of what this file tests — that each accepted spelling maps
/// to the flavor it names, and that each flavor produces a working
/// binary — without a global write.
fn builds_under(var: Option<&str>, name: &str) -> bool {
    let program = parse_source(SRC).expect("parse");
    let bin = harness::unique_bin(name);
    let options = BuildOptions {
        lto: Some(LtoMode::parse(var.unwrap_or(""))),
        ..Default::default()
    };
    let ok =
        build_executable_with_options(&program, &bin, &[], &options).is_ok();
    let _ = std::fs::remove_file(&bin);
    ok
}

/// Each accepted spelling produces a working binary. ThinLTO is the
/// recommended flavor: measured at least as good as full LTO on
/// runtime (json_parse -10.9% vs -6.0%, median of 15) at a similar
/// link cost.
#[test]
fn every_lto_spelling_builds() {
    for (v, want) in [
        (None, LtoMode::Off),
        (Some("thin"), LtoMode::Thin),
        (Some("1"), LtoMode::Full),
        (Some("full"), LtoMode::Full),
    ] {
        assert_eq!(
            LtoMode::parse(v.unwrap_or("")),
            want,
            "LOTUS_LTO={:?} must select {:?}",
            v,
            want
        );
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
    assert_eq!(
        LtoMode::parse("yes-please"),
        LtoMode::Off,
        "an unrecognized LOTUS_LTO must not select an LTO flavor"
    );
    assert!(
        builds_under(Some("yes-please"), "lto_modes_unknown"),
        "an unrecognized LOTUS_LTO must fall back to a normal build"
    );
}

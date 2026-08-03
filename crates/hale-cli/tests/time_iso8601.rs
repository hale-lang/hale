//! `std::time::parse_iso8601` (#353 item 3).
//!
//! The survey called this "time formatting", which turned out to be
//! wrong in a useful way: `lotus_time_from_unix` ALREADY returns
//! ISO-8601 text — Hale's `Time` is the formatted string, which is why
//! `println(t)` on a Time renders a date. Formatting was never
//! missing.
//!
//! PARSING had no counterpart at all. A service could emit a timestamp
//! into a log or a config and had no way to read one back, so every
//! application grew its own parser and they disagreed.
//!
//! UTC only, deliberately. A timezone database is megabytes and the
//! wasm target carries whatever ships; local time additionally reads
//! `TZ`, which makes it an `env` effect rather than a pure
//! computation. A trailing offset is REJECTED rather than ignored, so
//! a local-time string is never silently read as UTC.

use std::process::Command;

fn run(src: &str, tag: &str) -> String {
    let dir = std::env::temp_dir()
        .join(format!("hale-iso-{}-{}", std::process::id(), tag));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let f = dir.join("main.hl");
    std::fs::write(&f, src).expect("write");
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("run")
        .arg(&f)
        .output()
        .expect("run");
    let _ = std::fs::remove_dir_all(&dir);
    String::from_utf8_lossy(&out.stdout).to_string()
}

#[test]
fn a_timestamp_round_trips() {
    let out = run(
        "fn main() {\n\
             let s = std::time::parse_iso8601(\"2023-11-14T22:13:20Z\") or { -1 };\n\
             println(s);\n\
             println(std::time::time_from_unix(s));\n\
         }",
        "round",
    );
    let got: Vec<&str> = out.lines().collect();
    assert_eq!(
        got,
        vec!["1700000000", "2023-11-14T22:13:20Z"],
        "parse must be the exact inverse of time_from_unix: {:?}",
        got
    );
}

#[test]
fn the_epoch_is_exact() {
    let out = run(
        "fn main() {\n\
             println(std::time::parse_iso8601(\"1970-01-01T00:00:00Z\") or { -1 });\n\
         }",
        "epoch",
    );
    assert_eq!(out.trim(), "0", "epoch must be 0: {}", out);
}

/// Malformed input is a `fallible`, not a sentinel the caller has to
/// know to test for — the same shape as `str::parse_int`.
#[test]
fn malformed_input_takes_the_or_branch() {
    let out = run(
        "fn main() {\n\
             println(std::time::parse_iso8601(\"not a date\") or { -1 });\n\
             println(std::time::parse_iso8601(\"2023-13-99T00:00:00Z\") or { -1 });\n\
             println(std::time::parse_iso8601(\"2023-11-14\") or { -1 });\n\
         }",
        "bad",
    );
    let got: Vec<&str> = out.lines().collect();
    assert_eq!(
        got,
        vec!["-1", "-1", "-1"],
        "garbage, an impossible date, and a truncated one must all \
         fail: {:?}",
        got
    );
}

/// A trailing offset must be REJECTED, not ignored. Silently reading
/// `+01:00` as UTC would be an hour-wrong timestamp that never
/// announces itself.
#[test]
fn a_non_utc_offset_is_rejected() {
    let out = run(
        "fn main() {\n\
             println(std::time::parse_iso8601(\"2023-11-14T22:13:20+01:00\") or { -1 });\n\
         }",
        "offset",
    );
    assert_eq!(
        out.trim(),
        "-1",
        "an offset must not be silently treated as UTC: {}",
        out
    );
}

/// Pure — no clock read, no TZ read — so it is usable from a
/// `@deterministic` fn.
#[test]
fn parsing_is_pure() {
    let out = run(
        "@deterministic @no_syscall\n\
         fn at(s: String) -> Int { return std::time::parse_iso8601(s) or { -1 }; }\n\
         fn main() { println(at(\"1970-01-01T00:00:01Z\")); }",
        "pure",
    );
    assert_eq!(out.trim(), "1", "must certify as pure: {}", out);
}

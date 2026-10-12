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

/// The whole run: exit status, stdout and stderr, for the refusals
/// below that end the program or never let it build.
fn run_full(verb: &str, src: &str, tag: &str) -> std::process::Output {
    let dir = std::env::temp_dir()
        .join(format!("hale-iso-{}-{}", std::process::id(), tag));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let f = dir.join("main.hl");
    std::fs::write(&f, src).expect("write");
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg(verb)
        .arg(&f)
        .output()
        .expect("run");
    let _ = std::fs::remove_dir_all(&dir);
    out
}

/// A `Time` is i64 nanoseconds, 1677-09-21 through 2262-04-11. A
/// literal past either end is the author's error at the literal, named
/// as a range and not as a malformed date. It used to build and wrap:
/// `2999-01-01` was an instant in 1829.
#[test]
fn a_time_literal_past_the_range_is_refused_at_check() {
    let out = run_full(
        "check",
        "fn main() { let t = `2999-01-01T00:00:00Z`; println(t); }",
        "lit-range",
    );
    let err = String::from_utf8_lossy(&out.stderr).to_string()
        + &String::from_utf8_lossy(&out.stdout);
    assert!(!out.status.success(), "the literal must not check: {err}");
    assert!(
        err.contains("time literal `2999-01-01T00:00:00Z` is outside `Time`'s range")
            && err.contains("2262-04-11T23:47:16.854775807Z"),
        "the diagnostic names the range: {err}"
    );
}

/// `time_from_unix(n)` is `n × 10⁹`, and `Time ± Duration` is an
/// instant; neither has a caller frame for an error, so a result no
/// i64 holds is a violation where it would be made, never a wrapped
/// instant in another century.
#[test]
fn a_time_past_the_range_is_a_violation_not_a_wrap() {
    for (tag, body, what) in [
        (
            "from-unix",
            "let n = std::str::parse_int(\"32503680000\") or 0;\n\
             println(std::time::time_from_unix(n));",
            "time_from_unix(32503680000)",
        ),
        (
            "plus",
            "let t = `2262-04-11T00:00:00Z`;\n\
             let d = 1day;\n\
             println(t + d);",
            "+ 86400000000000ns",
        ),
        (
            "minus",
            "let t = `1677-09-22T00:00:00Z`;\n\
             let d = 2day;\n\
             println(t - d);",
            "- 172800000000000ns",
        ),
    ] {
        let out = run_full("run", &format!("fn main() {{\n{body}\n}}"), tag);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(1), "{tag}: exits 1: {stdout}{stderr}");
        assert!(stdout.trim().is_empty(), "{tag}: no wrapped instant printed: {stdout}");
        assert!(
            stderr.contains("violation: ")
                && stderr.contains(what)
                && stderr.contains("is outside Time's range"),
            "{tag}: names the operation and the range: {stderr}"
        );
    }
}

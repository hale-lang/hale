//! 2026-05-26 — range-bearing JSON iter_find variants +
//! std::str::range_* helpers. The a downstream app team identified
//! `iter_find_string_field` returning an owned String per
//! field lookup as the dominant arena-pressure source on
//! large JSON-walk workloads (a 5 MB market-data level2 frame
//! with 100k+ elements). The range variants return (start,
//! end_exclusive) byte positions inside the source json
//! String instead — paired with std::str::range_eq /
//! range_parse_int / range_parse_decimal, the full walk
//! runs allocation-free.
//!
//! Tests exercise the headline shape: walk an order-book
//! snapshot, compare a string field to a literal, parse
//! a Decimal field. Plus the missing-field and malformed-
//! input paths.
//!
//! Earlier zero-element-copy work (json_span_iter.rs) cut
//! per-iter cost from O(element_size) to O(value_size) by
//! avoiding the per-element substring copy. These tests
//! complete the picture by avoiding the per-VALUE substring
//! copy too.

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use hale_codegen::build_executable;

fn unique_path(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "lt-json-range-{}-{}-{}.bin",
        tag,
        std::process::id(),
        nanos,
    ));
    p
}

fn build_and_run(name: &str, src: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = unique_path(name);
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status,
    )
}

#[test]
fn range_eq_matches_substring() {
    // Sanity: std::str::range_eq compares (json, start, end)
    // against an expected literal, byte-for-byte.
    let src = r#"
        fn main() {
            let s = "hello world";
            let h = std::str::range_eq(s, 0, 5, "hello");
            let w = std::str::range_eq(s, 6, 11, "world");
            let m = std::str::range_eq(s, 0, 5, "world");
            let l = std::str::range_eq(s, 0, 4, "hello");  // length mismatch
            println("h=", h, " w=", w, " m=", m, " l=", l);
        }
    "#;
    let (stdout, status) = build_and_run("range_eq", src);
    assert!(status.success(), "non-zero exit: {:?}\nstdout: {}", status, stdout);
    assert!(stdout.contains("h=true"), "got: {:?}", stdout);
    assert!(stdout.contains("w=true"), "got: {:?}", stdout);
    assert!(stdout.contains("m=false"), "byte mismatch must report false; got: {:?}", stdout);
    assert!(stdout.contains("l=false"), "length mismatch must report false; got: {:?}", stdout);
}

#[test]
fn range_parse_int_strict() {
    let src = r#"
        fn main() {
            let s = "[42][-7][bad]";
            let a = std::str::range_parse_int(s, 1, 3) or raise;
            let b = std::str::range_parse_int(s, 5, 7) or raise;
            println("a=", a, " b=", b);
            // Malformed sub-range reports ParseError.
            let _c = std::str::range_parse_int(s, 9, 12) or fallback();
        }

        fn fallback() -> Int { println("caught_parse_error"); return -1; }
    "#;
    let (stdout, status) = build_and_run("range_parse_int", src);
    assert!(status.success(), "non-zero exit: {:?}\nstdout: {}", status, stdout);
    assert!(stdout.contains("a=42"), "got: {:?}", stdout);
    assert!(stdout.contains("b=-7"), "got: {:?}", stdout);
    assert!(stdout.contains("caught_parse_error"), "malformed input must surface ParseError; got: {:?}", stdout);
}

#[test]
fn range_parse_decimal_strict() {
    let src = r#"
        fn main() {
            let s = "[100.5][nope][-0.001]";
            let a = std::str::range_parse_decimal(s, 1, 6) or raise;
            let c = std::str::range_parse_decimal(s, 14, 20) or raise;
            println("a=", a);
            println("c=", c);
            let _b = std::str::range_parse_decimal(s, 8, 12) or fallback();
        }

        fn fallback() -> Decimal { println("caught_parse_error"); return 0.0d; }
    "#;
    let (stdout, status) = build_and_run("range_parse_decimal", src);
    assert!(status.success(), "non-zero exit: {:?}\nstdout: {}", status, stdout);
    assert!(stdout.contains("a=100.5"), "got: {:?}", stdout);
    assert!(stdout.contains("c=-0.001"), "got: {:?}", stdout);
    assert!(stdout.contains("caught_parse_error"), "malformed input must surface ParseError; got: {:?}", stdout);
}

#[test]
fn iter_find_field_range_walks_array() {
    // The a downstream app headline shape: walk an L2 snapshot array,
    // compare side, parse price + size as Decimal. Whole loop
    // runs allocation-free per element (after the source body
    // is in arena).
    let src = r#"
        fn main() {
            let body = "[{\"side\":\"bid\",\"price\":\"100.5\",\"size\":\"1.25\"},{\"side\":\"offer\",\"price\":\"200\",\"size\":\"0.5\"}]";
            let mut it = std::json::array_first_span(body);
            let mut bid_count = 0;
            let mut ask_count = 0;
            let mut total_size = 0.0d;
            while !it.done {
                let side_r = std::json::iter_find_string_field_range(it, body, "side");
                if std::str::range_eq(body, side_r.start, side_r.end_pos, "bid") {
                    bid_count = bid_count + 1;
                } else if std::str::range_eq(body, side_r.start, side_r.end_pos, "offer") {
                    ask_count = ask_count + 1;
                }
                let size_r = std::json::iter_find_string_field_range(it, body, "size");
                let sz = std::str::range_parse_decimal(
                    body, size_r.start, size_r.end_pos
                ) or raise;
                total_size = total_size + sz;
                it = std::json::array_next_span(it, body);
            }
            println("bids=", bid_count, " asks=", ask_count);
            println("total_size=", total_size);
        }
    "#;
    let (stdout, status) = build_and_run("walk", src);
    assert!(status.success(), "non-zero exit: {:?}\nstdout: {}", status, stdout);
    assert!(stdout.contains("bids=1"), "got: {:?}", stdout);
    assert!(stdout.contains("asks=1"), "got: {:?}", stdout);
    // 1.25 + 0.5 = 1.75
    assert!(stdout.contains("total_size=1.75"), "got: {:?}", stdout);
}

#[test]
fn iter_find_field_range_missing_field_reports_not_ok() {
    let src = r#"
        fn main() {
            let body = "[{\"side\":\"bid\"},{\"side\":\"offer\"}]";
            let mut it = std::json::array_first_span(body);
            while !it.done {
                let price_r = std::json::iter_find_field_range(it, body, "price");
                if price_r.ok {
                    println("found_price");
                } else {
                    println("missing_price");
                }
                it = std::json::array_next_span(it, body);
            }
        }
    "#;
    let (stdout, status) = build_and_run("missing", src);
    assert!(status.success(), "non-zero exit: {:?}\nstdout: {}", status, stdout);
    // Both elements lack "price"; expect 2 missing_price lines.
    let n = stdout.matches("missing_price").count();
    assert_eq!(n, 2, "expected 2 missing_price; got: {:?}", stdout);
    assert!(!stdout.contains("found_price"), "no element has the field; got: {:?}", stdout);
}

#[test]
fn high_volume_walk_bounded_rss() {
    // 2026-05-26 regression guard. The original range_* impl had a
    // hidden `std::bytes::from_string(json)` inside each scan loop
    // (and inside the iter_find_string_field_range quote check),
    // which allocated a fresh Bytes copy of the entire source JSON
    // on every call. For a downstream app's market-data L2 workload (~5 MB
    // snapshot × 100k elements × ~5 stdlib calls per iter) that
    // pushed peak RSS to 13+ GB on a single snapshot. The fix
    // routed scan loops through std::str::byte_at_unchecked, which
    // takes the String pointer directly with no allocation.
    //
    // Bound: a 50k-element walk on a 2 MB synthesized array should
    // peak under 100 MB. (Pre-fix on the same input: GB-range and
    // climbs linearly with iteration count.)
    let src = r#"
        fn build_input() -> String {
            let mut b = std::str::builder_new();
            std::str::builder_append(b, "[");
            let mut i = 0;
            while i < 50000 {
                if i > 0 { std::str::builder_append(b, ","); }
                std::str::builder_append(b, "{\"side\":\"bid\",\"price\":\"100.5\",\"size\":\"1.25\"}");
                i = i + 1;
            }
            std::str::builder_append(b, "]");
            return std::str::builder_finish(b);
        }

        fn walk(json: String) -> Int {
            let mut it = std::json::array_first_span(json);
            let mut n = 0;
            while !it.done {
                let side_r = std::json::iter_find_string_field_range(it, json, "side");
                if std::str::range_eq(json, side_r.start, side_r.end_pos, "bid") {
                    n = n + 1;
                }
                let size_r = std::json::iter_find_string_field_range(it, json, "size");
                let _v = std::str::range_parse_decimal(
                    json, size_r.start, size_r.end_pos
                ) or 0.0d;
                it = std::json::array_next_span(it, json);
            }
            return n;
        }

        fn main() {
            let json = build_input();
            let _n = walk(json);
            // Print peak-equivalent RSS at end-of-walk. Test asserts
            // a bound on this value; the regression manifests as
            // multi-GB rather than single/double-digit MB.
            print("final_rss_mb=");
            println(std::process::rss_bytes() / 1048576);
        }
    "#;
    let (stdout, status) = build_and_run("high_volume", src);
    assert!(
        status.success(),
        "high-volume walk crashed (probable memory leak): {:?}\nstdout: {}",
        status, stdout,
    );
    // Parse out the final RSS line and bound it.
    let rss_line = stdout
        .lines()
        .find(|l| l.starts_with("final_rss_mb="))
        .expect(&format!("missing final_rss_mb in stdout: {:?}", stdout));
    let rss: i64 = rss_line
        .trim_start_matches("final_rss_mb=")
        .trim()
        .parse()
        .expect(&format!("can't parse rss line: {:?}", rss_line));
    // Pre-fix: GB-range, scales linearly with iter count. Post-fix:
    // single-digit MB on this size. 100 MB is a generous bound that
    // leaves headroom for cold-start / allocator-rounding noise.
    assert!(
        rss < 100,
        "50k-iter range walk exceeded 100 MB RSS ({}MB) — likely \
         a bytes_from_string regression in a scan helper. Pre-fix \
         this OOM'd on a downstream app at 13+ GB.",
        rss
    );
}

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

#[path = "support/harness.rs"]
mod harness;

fn unique_path(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let p = harness::unique_bin(&format!(
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
fn high_volume_walk_cost_per_element_is_document_size_independent() {
    // 2026-05-26 regression guard. The original range_* impl had a
    // hidden `std::bytes::from_string(json)` inside each scan loop
    // (and inside the iter_find_string_field_range quote check),
    // which allocated a fresh Bytes copy of the entire source JSON
    // on every call. For a downstream app's market-data L2 workload
    // (~5 MB snapshot × 100k elements × ~5 stdlib calls per iter)
    // that pushed peak RSS to 13+ GB on a single snapshot. The fix
    // routed scan loops through std::str::byte_at_unchecked, which
    // takes the String pointer directly with no allocation.
    //
    // What that regression IS, stated as a property: the walk's
    // memory cost per ELEMENT became proportional to the DOCUMENT.
    // So that is what this test measures. It walks the same 20k
    // elements twice — once over a ~90 KB document, once over a
    // ~900 KB one — and compares bytes of RSS growth per element.
    // Post-fix the two agree (measured 467 vs 480 B/element, stable
    // across runs and across machine load); the regression makes the
    // second ten times the first, because each of the ~5 stdlib
    // calls per element copies the whole document.
    //
    // Reshaped 2026-09-20 (GH #772). The bound this replaces —
    // `std::process::rss_bytes() / 1048576 < 100` after a 50k-element
    // walk — could not work, for two independent reasons:
    //
    //   1. `rss_bytes()` is `getrusage(RUSAGE_SELF).ru_maxrss`, which
    //      a spawned program inherits from its parent through
    //      fork+exec (see `harness::statm_resident_bytes`). Under
    //      `cargo test` the parent is a libtest process running
    //      several in-process LLVM builds, so the assertion was
    //      reading the harness's memory: 137-145 MB observed
    //      against a 100 MB cap on a machine where this binary's
    //      own peak is 30 MB.
    //   2. Even measured correctly the walk is not RSS-flat: a plain
    //      `fn`'s per-iteration temporaries accumulate in the caller's
    //      arena until it returns, ~470 B per element here, so "RSS
    //      after 10k iterations ≈ after 1k" is false by construction
    //      and 50k elements × 470 B put the true reading within a few
    //      MB of the 100 MB line anyway.
    //
    // Both windows run inside one process, and every number the
    // assertions use is a difference between two of that process's
    // own /proc/self/statm reads, so neither machine load nor the
    // harness's footprint can move them.
    let src = r#"
        fn build_input(elements: Int) -> String {
            let mut b = std::str::builder_new();
            std::str::builder_append(b, "[");
            let mut i = 0;
            while i < elements {
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

        // `passes` full walks, bracketed by the program's own
        // residency. One unmeasured warm pass first, so first-touch
        // page-in and the arena's initial chunk growth land outside
        // the window.
        fn measured_walks(json: String, passes: Int, tag: String) {
            let _warm = walk(json);
            print(tag);
            print("_before_statm=");
            println(std::io::fs::read_file("/proc/self/statm") or "");
            let mut p = 0;
            while p < passes {
                let _n = walk(json);
                p = p + 1;
            }
            print(tag);
            print("_after_statm=");
            println(std::io::fs::read_file("/proc/self/statm") or "");
        }

        fn main() {
            let small = build_input(2000);
            let big = build_input(20000);
            print("small_len="); println(len(small));
            print("big_len="); println(len(big));
            // 10 passes x 2k elements and 1 pass x 20k elements:
            // the same 20k elements walked over a 10x longer document.
            measured_walks(small, 10, "small");
            measured_walks(big, 1, "big");
        }
    "#;
    let (stdout, status) = build_and_run("high_volume", src);
    assert!(
        status.success(),
        "high-volume walk crashed (probable memory leak): {:?}\nstdout: {}",
        status, stdout,
    );

    let line = |key: &str| -> &str {
        stdout
            .lines()
            .find_map(|l| l.strip_prefix(key))
            .unwrap_or_else(|| panic!("missing {} in stdout: {:?}", key, stdout))
    };
    let num = |key: &str| -> i64 {
        line(key)
            .trim()
            .parse()
            .unwrap_or_else(|e| panic!("can't parse {}: {} ({:?})", key, e, stdout))
    };
    // Bytes of resident growth per element walked. RSS is CURRENT
    // residency, so a kernel that reclaimed pages under memory
    // pressure can hand back a smaller second reading; that is not a
    // leak, so the window floors at zero.
    let cost_per_element = |tag: &str, elements: i64| -> i64 {
        let before =
            harness::statm_resident_bytes(line(&format!("{tag}_before_statm=")));
        let after =
            harness::statm_resident_bytes(line(&format!("{tag}_after_statm=")));
        (after - before).max(0) / elements
    };
    let small_len = num("small_len=");
    let big_len = num("big_len=");
    let small_cost = cost_per_element("small", 20_000);
    let big_cost = cost_per_element("big", 20_000);

    // The shape. Same element count, 10x the document: per-element
    // cost must not follow the document. The 4 KiB slack covers
    // page granularity on a ~10 MB window; the regression is a 10x
    // gap, not a 2x one.
    assert!(
        big_cost <= small_cost * 2 + 4096,
        "the same element count over a {}x larger document ({} B vs \
         {} B) cost {} B per element vs {} B — the walk's per-element \
         memory is following the document size, which is the \
         bytes_from_string-in-a-scan-helper regression (pre-fix this \
         OOM'd a downstream handoff's L2 workload at 13+ GB).",
        big_len / small_len,
        big_len,
        small_len,
        big_cost,
        small_cost,
    );

    // ...and the same property against the document rather than
    // against the other window, so a regression that inflated BOTH
    // costs equally cannot pass by symmetry. Pre-fix each element
    // cost roughly five document copies; post-fix it is ~470 B
    // against a 90 KB document.
    assert!(
        small_cost * 8 <= small_len,
        "each walked element grew RSS by {} B on a {} B document — \
         the per-element cost must be a small constant, not a \
         fraction of the document (a scan helper is copying the \
         source JSON again).",
        small_cost,
        small_len,
    );
}

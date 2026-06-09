//! the zero-element-copy walker friction — zero-element-copy JSON array walker.
//!
//! The `*_span` variants of `array_first` / `array_next` track
//! only positions in the source json (no per-element substring
//! allocation). `iter_find_*` helpers scan bounded by the
//! current element's range, so each iteration allocates only
//! the looked-up value rather than the entire element.

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
        "lt-json-span-{}-{}-{}.bin",
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
fn array_first_span_skips_element_copy() {
    let src = r#"
        fn main() {
            let body = "[{\"side\":\"bid\",\"price\":\"100.5\"},{\"side\":\"offer\",\"price\":\"200\"}]";
            let mut it = std::json::array_first_span(body);
            let mut n = 0;
            while !it.done {
                let side = std::json::iter_find_string_field(it, body, "side");
                let price = std::json::iter_find_field_raw(it, body, "price");
                println(n, ": side=", side, " price=", price);
                n = n + 1;
                it = std::json::array_next_span(it, body);
            }
            println("total=", n);
        }
    "#;
    let (stdout, status) = build_and_run("walk", src);
    assert!(status.success(), "binary exited non-zero: {:?}\nstdout: {}", status, stdout);
    assert!(stdout.contains("0: side=bid price=\"100.5\""), "got: {:?}", stdout);
    assert!(stdout.contains("1: side=offer price=\"200\""), "got: {:?}", stdout);
    assert!(stdout.contains("total=2"), "got: {:?}", stdout);
}

#[test]
fn iter_find_int_field_in_array() {
    let src = r#"
        fn main() {
            let body = "[{\"n\":1},{\"n\":42},{\"n\":-3}]";
            let mut it = std::json::array_first_span(body);
            let mut sum = 0;
            while !it.done {
                let v = std::json::iter_find_int_field(it, body, "n");
                sum = sum + v;
                it = std::json::array_next_span(it, body);
            }
            println("sum=", sum);
        }
    "#;
    let (stdout, status) = build_and_run("ints", src);
    assert!(status.success(), "binary exited non-zero: {:?}\nstdout: {}", status, stdout);
    assert!(stdout.contains("sum=40"), "got: {:?}", stdout);
}

#[test]
fn iter_substring_extracts_element_on_demand() {
    let src = r#"
        fn main() {
            let body = "[{\"a\":1},{\"a\":2}]";
            let it = std::json::array_first_span(body);
            let elem = std::json::iter_substring(it, body);
            println("first=", elem);
        }
    "#;
    let (stdout, status) = build_and_run("substr", src);
    assert!(status.success(), "binary exited non-zero: {:?}\nstdout: {}", status, stdout);
    assert!(stdout.contains("first={\"a\":1}"), "got: {:?}", stdout);
}

#[test]
fn iter_find_field_raw_in_isolates_to_current_element() {
    // The same field name `"price"` appears in two elements with
    // different values; iter_find_field_raw must return the
    // value for the CURRENT iter's element, not the first match
    // in the whole json. This is the load-bearing property — the
    // unbounded `find_field_raw(it.element, name)` worked because
    // element was already isolated; here we get the same isolation
    // via bounded scan.
    let src = r#"
        fn main() {
            let body = "[{\"price\":\"AAA\"},{\"price\":\"BBB\"},{\"price\":\"CCC\"}]";
            let mut it = std::json::array_first_span(body);
            it = std::json::array_next_span(it, body);
            // Now on the second element.
            let p = std::json::iter_find_field_raw(it, body, "price");
            println("second price=", p);
        }
    "#;
    let (stdout, status) = build_and_run("isolate", src);
    assert!(status.success(), "binary exited non-zero: {:?}\nstdout: {}", status, stdout);
    assert!(stdout.contains("second price=\"BBB\""), "got: {:?}", stdout);
}

#[test]
fn empty_array_done_immediately() {
    let src = r#"
        fn main() {
            let it = std::json::array_first_span("[]");
            if it.done {
                println("empty");
            } else {
                println("not empty");
            }
        }
    "#;
    let (stdout, status) = build_and_run("empty", src);
    assert!(status.success(), "binary exited non-zero: {:?}\nstdout: {}", status, stdout);
    assert!(stdout.contains("empty"), "got: {:?}", stdout);
}

#[test]
fn find_field_raw_in_with_explicit_bounds() {
    // Caller computes bounds themselves and calls the bounded
    // primitive directly.
    let src = r#"
        fn main() {
            let body = "{\"a\":1,\"b\":2,\"c\":3}";
            // Bound to the slice covering "b":2 only — exercises
            // the upper-bound clamp.
            let v = std::json::find_field_raw_in(body, "a", 0, 7);
            println("a=", v);
            // Searching past end_exclusive must return "":
            let v2 = std::json::find_field_raw_in(body, "c", 0, 7);
            println("c=", v2);
        }
    "#;
    let (stdout, status) = build_and_run("bounds", src);
    assert!(status.success(), "binary exited non-zero: {:?}\nstdout: {}", status, stdout);
    assert!(stdout.contains("a=1"), "got: {:?}", stdout);
    assert!(stdout.contains("c="), "got: {:?}", stdout);
    // The c= line should have nothing after the =:
    let c_line = stdout.lines().find(|l| l.starts_with("c=")).unwrap();
    assert_eq!(c_line, "c=", "expected c=<empty>, got: {:?}", c_line);
}

#[test]
fn array_of_records_walk_through_simd_cursor() {
    // Market-data shape: an array of objects. Walk the array (array
    // cursor) and parse each element (object cursor) — both SIMD now.
    let src = r#"
        type Level { px: Float `json:"px"`; sz: Int `json:"sz"`; }
        fn main() {
            let book = "[ {\"px\": 1.5, \"sz\": 10}, {\"px\": 2.25, \"sz\": 20},
                          {\"px\": 3.0, \"sz\": 30, \"note\": \"x\\\"y\"} ]";
            let mut it = std::json::array_first_span(book);
            let mut total = 0;
            while !it.done {
                let lvl = Level::from_json(std::json::iter_substring(it, book)) or raise;
                total = total + lvl.sz;
                it = std::json::array_next_span(it, book);
            }
            println("total=", to_string(total));
        }
    "#;
    let (out, status) = build_and_run("arr_rec", src);
    assert!(status.success(), "run failed: {}", out);
    assert!(out.contains("total=60"), "array-of-records walk wrong:\n{}", out);
}

//! std::json::Builder — small JSON-shape helpers.

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn build_and_run(name: &str, source: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(source).expect("parse");
    let bin = harness::unique_bin(&format!("hale_test_stdlib_json_{}", name));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (String::from_utf8_lossy(&output.stdout).to_string(), output.status)
}

#[test]
fn append_entry_inserts_comma_space_between_non_empty_acc() {
    let src = r#"
        fn main() {
            let b = std::json::Builder { };
            let a = b.append_entry("", "first");
            let bb = b.append_entry(a, "second");
            let cc = b.append_entry(bb, "third");
            println("inner=", cc);
        }
    "#;
    let (stdout, status) = build_and_run("append", src);
    assert!(status.success());
    assert!(stdout.contains("inner=first, second, third"), "got: {:?}", stdout);
}

#[test]
fn quote_wraps_in_double_quotes() {
    let src = r#"
        fn main() {
            let b = std::json::Builder { };
            println("q=", b.quote("net/http"));
            println("e=", b.quote(""));
        }
    "#;
    let (stdout, status) = build_and_run("quote", src);
    assert!(status.success());
    assert!(stdout.contains(r#"q="net/http""#), "got: {:?}", stdout);
    assert!(stdout.contains(r#"e="""#),         "got: {:?}", stdout);
}

#[test]
fn wrap_array_and_wrap_object_brace_correctly() {
    let src = r#"
        fn main() {
            let b = std::json::Builder { };
            println("arr=", b.wrap_array("\"a\", \"b\""));
            println("obj=", b.wrap_object("\"k\": \"v\""));
            println("empty=", b.wrap_array(""));
        }
    "#;
    let (stdout, status) = build_and_run("wrap", src);
    assert!(status.success());
    assert!(stdout.contains(r#"arr=["a", "b"]"#),  "got: {:?}", stdout);
    assert!(stdout.contains(r#"obj={"k": "v"}"#),  "got: {:?}", stdout);
    assert!(stdout.contains("empty=[]"),            "got: {:?}", stdout);
}

#[test]
fn build_quoted_array_handles_newline_separated_input() {
    let src = r#"
        fn main() {
            let b = std::json::Builder { };
            // Standard case with trailing newline.
            println("a=", b.build_quoted_array("log\nnet/http\nos\n"));
            // No trailing newline.
            println("b=", b.build_quoted_array("log\nnet/http\nos"));
            // Empty.
            println("c=", b.build_quoted_array(""));
            // Blank lines must be skipped.
            println("d=", b.build_quoted_array("log\n\nos\n"));
        }
    "#;
    let (stdout, status) = build_and_run("qa", src);
    assert!(status.success());
    assert!(stdout.contains(r#"a=["log", "net/http", "os"]"#), "trailing nl; got: {:?}", stdout);
    assert!(stdout.contains(r#"b=["log", "net/http", "os"]"#), "no trailing nl; got: {:?}", stdout);
    assert!(stdout.contains(r#"c=[]"#),                          "empty; got: {:?}", stdout);
    assert!(stdout.contains(r#"d=["log", "os"]"#),               "blank lines; got: {:?}", stdout);
}

#[test]
fn build_array_passes_raw_entries_through() {
    // build_array doesn't quote — the caller supplies pre-built entries.
    let src = r#"
        fn main() {
            let b = std::json::Builder { };
            println("a=", b.build_array("{\"k\": 1}\n{\"k\": 2}\n"));
        }
    "#;
    let (stdout, status) = build_and_run("raw", src);
    assert!(status.success());
    assert!(stdout.contains(r#"a=[{"k": 1}, {"k": 2}]"#), "got: {:?}", stdout);
}

#[test]
fn find_field_raw_returns_value_token_verbatim() {
    // 2026-05-20 — find_field_raw exposes the substring of a
    // field's value token. Numeric / bool / string forms all
    // return the raw bytes (incl. surrounding quotes for strings).
    let src = r#"
        fn main() {
            let s = "{\"name\":\"alice\",\"age\":30,\"active\":true}";
            let v_name   = std::json::find_field_raw(s, "name");
            let v_age    = std::json::find_field_raw(s, "age");
            let v_active = std::json::find_field_raw(s, "active");
            let v_miss   = std::json::find_field_raw(s, "missing");
            println("name=", v_name);
            println("age=", v_age);
            println("active=", v_active);
            println("miss=[", v_miss, "]");
        }
    "#;
    let (stdout, status) = build_and_run("find_field_raw", src);
    assert!(status.success());
    assert!(stdout.contains("name=\"alice\""), "got: {:?}", stdout);
    assert!(stdout.contains("age=30"), "got: {:?}", stdout);
    assert!(stdout.contains("active=true"), "got: {:?}", stdout);
    assert!(stdout.contains("miss=[]"), "got: {:?}", stdout);
}

#[test]
fn find_field_raw_enables_nested_object_descent() {
    // The point of exposing find_field_raw — wrapped-JSON
    // wrapped payloads where the real fields live inside a
    // nested object. Two-step extract: find_field_raw to get
    // the inner object's substring, then find_string_field for
    // the leaf scalars.
    let src = r#"
        fn main() {
            let s = "{\"result\":{\"channel\":\"data\",\"symbol\":\"ABC-123\"}}";
            let inner = std::json::find_field_raw(s, "result");
            let ch = std::json::find_string_field(inner, "channel");
            let sy = std::json::find_string_field(inner, "symbol");
            println("ch=", ch);
            println("sy=", sy);
        }
    "#;
    let (stdout, status) = build_and_run("find_field_raw_nested", src);
    assert!(status.success());
    assert!(stdout.contains("ch=data"), "got: {:?}", stdout);
    assert!(stdout.contains("sy=ABC-123"), "got: {:?}", stdout);
}

/// GH #708 — escaping, unescaping and `Builder` are linear in the
/// bytes they produce, and the proof is a hard ceiling rather than a
/// stopwatch.
///
/// Until 2026-09-19 all three grew their output with `out = out +
/// piece`. A String is immutable, so every write copied the whole
/// accumulated prefix into a fresh arena allocation and left the old
/// one behind — quadratic time AND quadratic retained memory, since
/// the arena does not reclaim until the call returns. A downstream
/// handoff hit it on a generated document with a 600,000-byte shared
/// value and 512 references: 23-24 GiB of scratch, swap exhausted.
///
/// The shape of the proof matters. A wall-clock timeout does not
/// bound scratch allocation, and a check on the finished document's
/// size sees only what survived. The address-space cap is what the
/// intermediate prefixes have to fit inside. 512 MiB against ~2 MB of
/// real output is ~250x of headroom for the linear form; the
/// quadratic form needs ~1 TB for the escape pass alone and dies in
/// under two seconds.
///
/// Linux only: `ulimit -v` is the cap that makes this test a test.
#[cfg(target_os = "linux")]
#[test]
fn escape_and_builder_stay_within_a_hard_memory_cap() {
    // 1011-byte chunk, three of whose bytes escape, repeated to
    // ~2 MB. Then a ~2 MB document: 1000 fields of a 2 KB value.
    let src = r#"
        fn main() {
            let chunk =
                std::str::repeat("abcdefghijklmnopqrstuvwxyz0123456789", 28)
                + "\"\\\n";
            let big = std::str::repeat(chunk, 2000);
            println("in=", len(big));
            let esc = std::json::escape_string(big);
            println("esc=", len(esc));
            let back = std::json::unescape_string(esc);
            println("back=", len(back));
            println("same=", back == big);

            let value = std::str::repeat("v", 2000);
            let b = std::json::Builder { };
            b.begin_object();
            let mut i = 0;
            while i < 1000 {
                b.string_field("field" + to_string(i), value);
                i = i + 1;
            }
            b.end_object();
            let doc = b.result();
            println("doc=", len(doc));
            println("head=", doc[0..20]);
        }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin("hale_test_stdlib_json_scaling");
    build_executable(&program, &bin).expect("build");
    // sh -c 'ulimit -v 524288; exec <bin>' under a 120 s wall bound.
    // The pre-fix binary segfaults here in ~1.6 s once the arena
    // cannot grow; the fixed one finishes in ~0.1 s.
    let output = Command::new("timeout")
        .arg("120")
        .arg("sh")
        .arg("-c")
        .arg(format!("ulimit -v 524288; exec {}", bin.display()))
        .output()
        .expect("run under a 512 MiB address-space cap");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "std::json did not stay inside 512 MiB of address space \
         (124 = the 120 s wall bound).\nexit: {:?}\nstdout: {}\nstderr: {}",
        output.status.code(),
        stdout,
        stderr,
    );
    // Exact lengths, not a substring of the shape: a truncated or
    // doubled output would otherwise pass.
    assert!(stdout.contains("in=2022000"), "input length; got: {:?}", stdout);
    assert!(stdout.contains("esc=2028000"), "escaped length; got: {:?}", stdout);
    assert!(stdout.contains("back=2022000"), "decoded length; got: {:?}", stdout);
    assert!(stdout.contains("same=true"), "round trip; got: {:?}", stdout);
    assert!(stdout.contains("doc=2015890"), "document length; got: {:?}", stdout);
    assert!(
        stdout.contains(r#"head={"field0": "vvvvvvvv"#),
        "document prefix; got: {:?}",
        stdout
    );
}

/// GH #754 — `std::json::valid` is LINEAR in the input, and the proof
/// is a wall bound over ~1 MiB.
///
/// The validators this replaces were written with per-character
/// checked slicing (`text[p..p + 1]`), which allocates a String per
/// byte and re-runs `len()` (a native strlen) per step: a downstream
/// handoff measured 10.6 s over 1 MiB that way, and 17 ms after the
/// rewrite to a cached length plus `byte_at_unchecked` (GH #720).
/// Seconds-versus-milliseconds is exactly what a wall bound can tell
/// apart, so a quadratic regression here fails the build rather than
/// slowly costing a caller their request budget.
///
/// Three shapes, since they exercise different loops: a flat array of
/// numbers (the scalar scanner), a 64-field object of long string
/// values (`valid_object` at its documented width bound, where its
/// per-field walk over the earlier keys is longest), and the same
/// array behind 64 levels of nesting (the depth mask).
///
/// The fourth measurement is the one that is not about time at all.
/// Both fns are documented to allocate nothing, and a free fn's
/// temporaries live in the CALLER's arena until the caller returns —
/// so a per-call scratch buffer does not show up as a leak, it shows
/// up as a loop that grows without bound. The first cut of
/// `valid_object` kept seen keys in a fixed `[0; 1024]` table, which
/// lowers to an arena allocation: 200,000 calls grew RSS by 1.69 GB.
/// Here the same loop must not move RSS at all.
#[test]
fn valid_scans_a_megabyte_within_a_wall_bound() {
    let src = r#"
        fn main() {
            let arr = std::bytes::BytesBuilder { initial_cap: 1200000 };
            arr.append_str("[0");
            let mut i = 0;
            while i < 150000 {
                arr.append_str(",-12.5e3");
                i = i + 1;
            }
            arr.append_str("]");
            let doc = std::str::clone(arr.text_view());
            println("arr_len=", len(doc));
            println("arr_valid=", std::json::valid(doc));

            let obj = std::bytes::BytesBuilder { initial_cap: 1200000 };
            obj.append_str("{");
            let value = std::str::repeat("v", 16000);
            let mut f = 0;
            while f < 64 {
                if f > 0 { obj.append_str(","); }
                obj.append_str("\"field");
                obj.append_str(to_string(f));
                obj.append_str("\": \"");
                obj.append_str(value);
                obj.append_str("\"");
                f = f + 1;
            }
            obj.append_str("}");
            let wide = std::str::clone(obj.text_view());
            println("obj_len=", len(wide));
            println("obj_valid=", std::json::valid_object(wide));

            let nest = std::bytes::BytesBuilder { initial_cap: 1200000 };
            let mut d = 0;
            while d < 63 {
                nest.append_str("[");
                d = d + 1;
            }
            nest.append_str(doc);
            while d > 0 {
                nest.append_str("]");
                d = d - 1;
            }
            let deep = std::str::clone(nest.text_view());
            println("deep_len=", len(deep));
            println("deep_valid=", std::json::valid(deep));

            // Allocation: a row-shaped gate, 200,000 times.
            let row = "{\"seq\":1,\"kind\":\"k\",\"body\":\"b\"}";
            print("before_statm=");
            println(std::io::fs::read_file("/proc/self/statm") or "");
            let mut r = 0;
            let mut gated = 0;
            while r < 200000 {
                if std::json::valid_object(row) { gated = gated + 1; }
                if std::json::valid(row) { gated = gated + 1; }
                r = r + 1;
            }
            println("gated=", gated);
            print("after_statm=");
            println(std::io::fs::read_file("/proc/self/statm") or "");
        }
    "#;
    // The growth is a difference between the program's own
    // /proc/self/statm reads, not `std::process::rss_bytes()`: that
    // one is `getrusage(RUSAGE_SELF).ru_maxrss`, which a spawned
    // program inherits from its parent through fork+exec, so under
    // the test harness both readings clamped to the harness's own
    // footprint and the difference was always 0 (GH #772).
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin("hale_test_stdlib_json_valid_scaling");
    build_executable(&program, &bin).expect("build");
    // ~10 ms per megabyte measured; 10 s is 1000x of headroom for the
    // linear form and far under what a per-character-slice scan needs.
    let output = Command::new("timeout")
        .arg("10")
        .arg(&bin)
        .output()
        .expect("run the validator under a 10 s wall bound");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "validating ~1 MiB did not finish inside 10 s \
         (124 = the wall bound).\nexit: {:?}\nstdout: {}\nstderr: {}",
        output.status.code(),
        stdout,
        stderr,
    );
    // Exact lengths: a truncated document would otherwise validate
    // quickly and pass.
    assert!(stdout.contains("arr_len=1200003"), "array length; got: {:?}", stdout);
    assert!(stdout.contains("arr_valid=true"), "array verdict; got: {:?}", stdout);
    assert!(stdout.contains("obj_len=1024887"), "object length; got: {:?}", stdout);
    assert!(stdout.contains("obj_valid=true"), "object verdict; got: {:?}", stdout);
    assert!(stdout.contains("deep_len=1200129"), "nested length; got: {:?}", stdout);
    assert!(stdout.contains("deep_valid=true"), "nested verdict; got: {:?}", stdout);
    assert!(stdout.contains("gated=400000"), "gate verdicts; got: {:?}", stdout);
    let read = |key: &str| -> i64 {
        let line = stdout
            .lines()
            .find_map(|l| l.strip_prefix(key))
            .unwrap_or_else(|| panic!("missing {} in stdout: {:?}", key, stdout));
        harness::statm_resident_bytes(line)
    };
    let growth = read("after_statm=") - read("before_statm=");
    assert!(
        growth < 4 * 1024 * 1024,
        "400,000 validator calls grew RSS by {} bytes — both fns are \
         documented to allocate nothing, and a free fn's scratch lands \
         in the caller's arena until the caller returns (measured \
         2026-09-20: 68 KB)",
        growth,
    );
}

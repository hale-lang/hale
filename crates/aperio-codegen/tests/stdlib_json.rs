//! std::json::Builder — small JSON-shape helpers.

use std::process::Command;

use aperio_codegen::build_executable;

fn build_and_run(name: &str, source: &str) -> (String, std::process::ExitStatus) {
    let program = aperio_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("aperio_test_stdlib_json_{}", name));
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

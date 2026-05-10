//! std::tagged::Accumulator — tagged-line accumulator parsing.
//!
//! Validates the surface against the byte-shape that
//! apps/onboard + apps/tower-join produce today. After this
//! ships, those apps drop their hand-rolled __count_tag /
//! __first_tag_body / __collect_tag_* helpers.

use std::process::Command;

use aperio_codegen::build_executable;

fn build_and_run(name: &str, source: &str) -> (String, std::process::ExitStatus) {
    let program = aperio_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("aperio_test_stdlib_tagged_{}", name));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (String::from_utf8_lossy(&output.stdout).to_string(), output.status)
}

#[test]
fn count_tallies_lines_matching_tag() {
    let src = r#"
        fn main() {
            let acc = "IMPORT:log\nIMPORT:net/http\nTYPE:Server\nIMPORT:os\n";
            let r = std::tagged::Accumulator { };
            println("imports=", r.count(acc, "IMPORT"));
            println("types=", r.count(acc, "TYPE"));
            println("absent=", r.count(acc, "MISSING"));
        }
    "#;
    let (stdout, status) = build_and_run("count", src);
    assert!(status.success());
    assert!(stdout.contains("imports=3"), "got: {:?}", stdout);
    assert!(stdout.contains("types=1"),   "got: {:?}", stdout);
    assert!(stdout.contains("absent=0"),  "got: {:?}", stdout);
}

#[test]
fn first_body_returns_first_matching_payload() {
    let src = r#"
        fn main() {
            let acc = "PKG:main\nIMPORT:log\nIMPORT:net/http\n";
            let r = std::tagged::Accumulator { };
            println("pkg=", r.first_body(acc, "PKG"));
            println("imp=", r.first_body(acc, "IMPORT"));
            println("miss=", r.first_body(acc, "MISSING"));
        }
    "#;
    let (stdout, status) = build_and_run("first", src);
    assert!(status.success());
    assert!(stdout.contains("pkg=main"), "got: {:?}", stdout);
    assert!(stdout.contains("imp=log"),  "got: {:?}", stdout);
    assert!(stdout.contains("miss="),    "missing tag → empty; got: {:?}", stdout);
    assert!(!stdout.contains("miss=l"),  "missing tag must not leak match; got: {:?}", stdout);
}

#[test]
fn collect_csv_joins_bodies_with_comma_space() {
    let src = r#"
        fn main() {
            let acc = "IMPORT:log\nIMPORT:net/http\nIMPORT:os\n";
            let r = std::tagged::Accumulator { };
            println("csv=", r.collect_csv(acc, "IMPORT"));
            println("empty=", r.collect_csv(acc, "MISSING"));
        }
    "#;
    let (stdout, status) = build_and_run("csv", src);
    assert!(status.success());
    assert!(stdout.contains("csv=log, net/http, os"), "got: {:?}", stdout);
    assert!(stdout.contains("empty="),                "got: {:?}", stdout);
}

#[test]
fn collect_array_emits_json_array_of_quoted_strings() {
    let src = r#"
        fn main() {
            let acc = "IMPORT:log\nIMPORT:net/http\n";
            let r = std::tagged::Accumulator { };
            println("arr=", r.collect_array(acc, "IMPORT"));
            println("empty=", r.collect_array(acc, "MISSING"));
        }
    "#;
    let (stdout, status) = build_and_run("array", src);
    assert!(status.success());
    assert!(stdout.contains(r#"arr=["log", "net/http"]"#), "got: {:?}", stdout);
    assert!(stdout.contains("empty=[]"),                    "got: {:?}", stdout);
}

#[test]
fn each_body_returns_newline_joined_matching_bodies() {
    let src = r#"
        fn main() {
            let acc = "IMPORT:log\nTYPE:Server\nIMPORT:os\n";
            let r = std::tagged::Accumulator { };
            let imports = r.each_body(acc, "IMPORT");
            // Re-iterate via std::iter::Lines.
            let it = std::iter::Lines { };
            let mut from = 0;
            while from >= 0 {
                let line = it.line_at(imports, from);
                from = it.next_idx(imports, from);
                if it.is_skippable(line) { continue; }
                println("E=", line);
            }
        }
    "#;
    let (stdout, status) = build_and_run("each", src);
    assert!(status.success());
    assert!(stdout.contains("E=log"), "got: {:?}", stdout);
    assert!(stdout.contains("E=os"),  "got: {:?}", stdout);
    assert!(!stdout.contains("E=Server"), "non-IMPORT must not leak; got: {:?}", stdout);
}

#[test]
fn body_preserves_colons_after_first() {
    // Bodies that themselves contain ":" should split only on
    // the FIRST ":" — the rest stays in the body. Confirms the
    // line_matches / body_of split logic.
    let src = r#"
        fn main() {
            let acc = "WIRE:handler|/api:v1\n";
            let r = std::tagged::Accumulator { };
            println("wire=", r.first_body(acc, "WIRE"));
        }
    "#;
    let (stdout, status) = build_and_run("colons", src);
    assert!(status.success());
    assert!(stdout.contains("wire=handler|/api:v1"), "got: {:?}", stdout);
}

//! std::source::Walk — directory-of-source-files iteration.
//!
//! Validates the seed contract: a Walk configured with flavor
//! + ext + on_file iterates files in directory order, parses
//! each via Lang, and concatenates the callback's String
//! fragments. Each test isolates one behavior of the seed.

use std::process::Command;

use aperio_codegen::build_executable;

fn build_aperio(name: &str, source: &str) -> std::path::PathBuf {
    let program = aperio_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("aperio_test_stdlib_source_{}", name));
    build_executable(&program, &bin).expect("build");
    bin
}

fn make_fixture(test_name: &str, files: &[(&str, &str)]) -> std::path::PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!("aperio_test_source_walk_{}", test_name));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    for (name, body) in files {
        std::fs::write(dir.join(name), body).expect("write file");
    }
    dir
}

#[test]
fn walk_invokes_callback_per_matched_file() {
    let fixture = make_fixture(
        "callback_per_file",
        &[
            ("alpha.go", "package alpha\n"),
            ("beta.go",  "package beta\n"),
            ("gamma.go", "package gamma\n"),
            ("README.md", "ignore me\n"),
        ],
    );
    let dir = fixture.to_string_lossy().to_string();
    let src = format!(r#"
        fn __record(lang: std::lang::Lang, name: String, root: Int) -> String {{
            return "F:" + name + "\n";
        }}

        fn main() {{
            let w = std::source::Walk {{
                flavor: "go",
                ext:    ".go",
                on_file: __record,
            }};
            let body = w.each_file("{dir}");
            print(body);
        }}
    "#);
    let bin = build_aperio("callback", &src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_dir_all(&fixture);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("F:alpha.go"), "got: {:?}", stdout);
    assert!(stdout.contains("F:beta.go"),  "got: {:?}", stdout);
    assert!(stdout.contains("F:gamma.go"), "got: {:?}", stdout);
    // README.md is filtered out by ext.
    assert!(!stdout.contains("F:README.md"), "got: {:?}", stdout);
}

#[test]
fn walk_empty_dir_returns_empty_string() {
    let fixture = make_fixture("empty", &[]);
    let dir = fixture.to_string_lossy().to_string();
    let src = format!(r#"
        fn __record(lang: std::lang::Lang, name: String, root: Int) -> String {{
            return "should-not-fire\n";
        }}

        fn main() {{
            let w = std::source::Walk {{ flavor: "go", ext: ".go",
                                         on_file: __record }};
            let body = w.each_file("{dir}");
            println("len=", len(body));
        }}
    "#);
    let bin = build_aperio("empty_dir", &src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_dir_all(&fixture);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("len=0"), "expected empty body; got: {:?}", stdout);
    assert!(!stdout.contains("should-not-fire"), "got: {:?}", stdout);
}

#[test]
fn walk_ext_filter_skips_non_matching_files() {
    let fixture = make_fixture(
        "ext_filter",
        &[
            ("keep.go",   "package keep\n"),
            ("skip.rs",   "fn main() {}\n"),
            ("skip.txt",  "plain text\n"),
            ("skip.json", "{}\n"),
        ],
    );
    let dir = fixture.to_string_lossy().to_string();
    let src = format!(r#"
        fn __record(lang: std::lang::Lang, name: String, root: Int) -> String {{
            return name + "\n";
        }}

        fn main() {{
            let w = std::source::Walk {{ flavor: "go", ext: ".go",
                                         on_file: __record }};
            print(w.each_file("{dir}"));
        }}
    "#);
    let bin = build_aperio("ext_filter", &src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_dir_all(&fixture);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("keep.go"), "got: {:?}", stdout);
    assert!(!stdout.contains("skip.rs"), "got: {:?}", stdout);
    assert!(!stdout.contains("skip.txt"), "got: {:?}", stdout);
    assert!(!stdout.contains("skip.json"), "got: {:?}", stdout);
}

#[test]
fn walk_callback_can_query_parsed_root() {
    // Confirms the root int handed to on_file is a usable
    // tree-sitter node — the callback queries its named-child
    // count to prove it really parsed.
    let fixture = make_fixture(
        "parsed_root",
        &[
            ("simple.go", "package simple\nfunc main() {}\n"),
        ],
    );
    let dir = fixture.to_string_lossy().to_string();
    let src = format!(r#"
        fn __probe(lang: std::lang::Lang, name: String, root: Int) -> String {{
            let n = std::ts::node_named_child_count(root);
            return name + " children=" + to_string(n) + "\n";
        }}

        fn main() {{
            let w = std::source::Walk {{ flavor: "go", ext: ".go",
                                         on_file: __probe }};
            print(w.each_file("{dir}"));
        }}
    "#);
    let bin = build_aperio("parsed_root", &src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_dir_all(&fixture);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    // simple.go has package_clause + function_declaration = 2 named children.
    assert!(stdout.contains("simple.go children=2"), "got: {:?}", stdout);
}

#[test]
fn walk_concatenates_fragments_in_order() {
    let fixture = make_fixture(
        "concat",
        &[
            ("a.go", "package a\n"),
            ("b.go", "package b\n"),
            ("c.go", "package c\n"),
        ],
    );
    let dir = fixture.to_string_lossy().to_string();
    let src = format!(r#"
        fn __tag(lang: std::lang::Lang, name: String, root: Int) -> String {{
            return "[" + name + "]";
        }}

        fn main() {{
            let w = std::source::Walk {{ flavor: "go", ext: ".go",
                                         on_file: __tag }};
            println("out=", w.each_file("{dir}"));
        }}
    "#);
    let bin = build_aperio("concat", &src);
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_dir_all(&fixture);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    // All three fragments present, no separator between them.
    assert!(stdout.contains("[a.go]"), "got: {:?}", stdout);
    assert!(stdout.contains("[b.go]"), "got: {:?}", stdout);
    assert!(stdout.contains("[c.go]"), "got: {:?}", stdout);
}

//! std::yaml::Builder + std::yaml::Reader round-trip smoke tests.

use std::process::Command;

use aperio_codegen::build_executable;

fn build_and_run(name: &str, source: &str) -> (String, std::process::ExitStatus) {
    let program = aperio_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("aperio_test_stdlib_yaml_{}", name));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (String::from_utf8_lossy(&output.stdout).to_string(), output.status)
}

#[test]
fn builder_emits_block_style_scalars_with_indent() {
    let src = r#"
        fn main() {
            let b = std::yaml::Builder { };
            print(b.field(0, "schema_version", "1"));
            print(b.field_q(0, "module", "demo"));
            print(b.field_i(0, "binaries_count", 3));
            print(b.field_b(0, "ready", true));
            print(b.field_null(0, "missing"));
        }
    "#;
    let (stdout, status) = build_and_run("scalars", src);
    assert!(status.success(), "stdout: {:?}", stdout);
    assert!(stdout.contains("schema_version: 1\n"),       "got: {:?}", stdout);
    assert!(stdout.contains("module: \"demo\"\n"),         "got: {:?}", stdout);
    assert!(stdout.contains("binaries_count: 3\n"),        "got: {:?}", stdout);
    assert!(stdout.contains("ready: true\n"),              "got: {:?}", stdout);
    assert!(stdout.contains("missing: null\n"),            "got: {:?}", stdout);
}

#[test]
fn builder_indents_nested_and_list_items_at_correct_columns() {
    let src = r#"
        fn main() {
            let b = std::yaml::Builder { };
            let mut s = "";
            s = s + b.nested_open(0, "binaries");
            s = s + b.list_item_first_q(2, "name", "echo");
            s = s + b.field_q(2, "rel", "cmd/echo");
            s = s + b.nested_open(2, "files");
            s = s + b.list_item_first_q(4, "file", "main.go");
            s = s + b.field_q(4, "package", "main");
            print(s);
        }
    "#;
    let (stdout, status) = build_and_run("nesting", src);
    assert!(status.success(), "stdout: {:?}", stdout);
    assert!(stdout.contains("binaries:\n"),                "got: {:?}", stdout);
    assert!(stdout.contains("  - name: \"echo\"\n"),       "got: {:?}", stdout);
    assert!(stdout.contains("    rel: \"cmd/echo\"\n"),    "got: {:?}", stdout);
    assert!(stdout.contains("    files:\n"),               "got: {:?}", stdout);
    assert!(stdout.contains("      - file: \"main.go\"\n"),"got: {:?}", stdout);
    assert!(stdout.contains("        package: \"main\"\n"),"got: {:?}", stdout);
}

#[test]
fn reader_scalar_returns_unquoted_value() {
    let src = r#"
        fn main() {
            let yaml = "schema_version: 1\nmodule: \"demo\"\nflavor: go\n";
            let r = std::yaml::Reader { text: yaml };
            println("schema=", r.scalar("schema_version"));
            println("module=", r.scalar("module"));
            println("flavor=", r.scalar("flavor"));
            println("missing=", r.scalar("nope"));
        }
    "#;
    let (stdout, status) = build_and_run("scalar", src);
    assert!(status.success(), "stdout: {:?}", stdout);
    assert!(stdout.contains("schema=1\n"),    "got: {:?}", stdout);
    assert!(stdout.contains("module=demo\n"), "got: {:?}", stdout);
    assert!(stdout.contains("flavor=go\n"),   "got: {:?}", stdout);
    assert!(stdout.contains("missing=\n"),    "got: {:?}", stdout);
}

#[test]
fn reader_has_distinguishes_present_from_absent() {
    let src = r#"
        fn main() {
            let yaml = "module: demo\nflavor: go\n";
            let r = std::yaml::Reader { text: yaml };
            let mut t = "false";
            if r.has("module") { t = "true"; }
            println("has_module=", t);
            let mut u = "false";
            if r.has("nope") { u = "true"; }
            println("has_nope=", u);
        }
    "#;
    let (stdout, status) = build_and_run("has", src);
    assert!(status.success(), "stdout: {:?}", stdout);
    assert!(stdout.contains("has_module=true\n"), "got: {:?}", stdout);
    assert!(stdout.contains("has_nope=false\n"),  "got: {:?}", stdout);
}

#[test]
fn reader_list_count_counts_items_under_header() {
    let src = r#"
        fn main() {
            let yaml = "binaries:\n  - name: a\n  - name: b\n  - name: c\nflavor: go\n";
            let r = std::yaml::Reader { text: yaml };
            println("n=", to_string(r.list_count("binaries")));
            println("none=", to_string(r.list_count("missing")));
        }
    "#;
    let (stdout, status) = build_and_run("list_count", src);
    assert!(status.success(), "stdout: {:?}", stdout);
    assert!(stdout.contains("n=3\n"),    "got: {:?}", stdout);
    assert!(stdout.contains("none=0\n"), "got: {:?}", stdout);
}

#[test]
fn reader_list_item_strips_marker_and_indent_for_recursion() {
    let src = r#"
        fn main() {
            let yaml = "binaries:\n  - name: alpha\n    rel: cmd/alpha\n  - name: beta\n    rel: cmd/beta\n";
            let r = std::yaml::Reader { text: yaml };
            let first = r.list_item("binaries", 0);
            let r1 = std::yaml::Reader { text: first };
            println("first.name=", r1.scalar("name"));
            println("first.rel=", r1.scalar("rel"));
            let second = r.list_item("binaries", 1);
            let r2 = std::yaml::Reader { text: second };
            println("second.name=", r2.scalar("name"));
            println("second.rel=", r2.scalar("rel"));
        }
    "#;
    let (stdout, status) = build_and_run("list_item", src);
    assert!(status.success(), "stdout: {:?}", stdout);
    assert!(stdout.contains("first.name=alpha\n"),     "got: {:?}", stdout);
    assert!(stdout.contains("first.rel=cmd/alpha\n"),  "got: {:?}", stdout);
    assert!(stdout.contains("second.name=beta\n"),     "got: {:?}", stdout);
    assert!(stdout.contains("second.rel=cmd/beta\n"),  "got: {:?}", stdout);
}

#[test]
fn reader_nested_strips_one_indent_for_sub_object() {
    let src = r#"
        fn main() {
            let yaml = "codebase:\n  root: /repo\n  flavor: go\nbinaries:\n  - name: a\n";
            let r = std::yaml::Reader { text: yaml };
            let body = r.nested("codebase");
            let sub = std::yaml::Reader { text: body };
            println("root=", sub.scalar("root"));
            println("flavor=", sub.scalar("flavor"));
        }
    "#;
    let (stdout, status) = build_and_run("nested", src);
    assert!(status.success(), "stdout: {:?}", stdout);
    assert!(stdout.contains("root=/repo\n"), "got: {:?}", stdout);
    assert!(stdout.contains("flavor=go\n"),  "got: {:?}", stdout);
}

#[test]
fn reader_handles_deeply_nested_lists_in_lists() {
    // binaries -> list of binaries; each binary has files list.
    let src = r#"
        fn main() {
            let mut yaml = "";
            yaml = yaml + "binaries:\n";
            yaml = yaml + "  - name: alpha\n";
            yaml = yaml + "    files:\n";
            yaml = yaml + "      - file: main.go\n";
            yaml = yaml + "        package: main\n";
            yaml = yaml + "      - file: util.go\n";
            yaml = yaml + "        package: alpha\n";
            yaml = yaml + "  - name: beta\n";
            yaml = yaml + "    files:\n";
            yaml = yaml + "      - file: main.go\n";
            yaml = yaml + "        package: main\n";

            let r = std::yaml::Reader { text: yaml };
            let bin0 = r.list_item("binaries", 0);
            let b0 = std::yaml::Reader { text: bin0 };
            println("b0.name=", b0.scalar("name"));
            println("b0.files.count=", to_string(b0.list_count("files")));

            let f0 = b0.list_item("files", 0);
            let r0 = std::yaml::Reader { text: f0 };
            println("b0.f0.file=", r0.scalar("file"));
            println("b0.f0.package=", r0.scalar("package"));

            let f1 = b0.list_item("files", 1);
            let r1 = std::yaml::Reader { text: f1 };
            println("b0.f1.file=", r1.scalar("file"));

            let bin1 = r.list_item("binaries", 1);
            let b1 = std::yaml::Reader { text: bin1 };
            println("b1.name=", b1.scalar("name"));
            println("b1.files.count=", to_string(b1.list_count("files")));
        }
    "#;
    let (stdout, status) = build_and_run("deep", src);
    assert!(status.success(), "stdout: {:?}", stdout);
    assert!(stdout.contains("b0.name=alpha\n"),       "got: {:?}", stdout);
    assert!(stdout.contains("b0.files.count=2\n"),    "got: {:?}", stdout);
    assert!(stdout.contains("b0.f0.file=main.go\n"),  "got: {:?}", stdout);
    assert!(stdout.contains("b0.f0.package=main\n"),  "got: {:?}", stdout);
    assert!(stdout.contains("b0.f1.file=util.go\n"),  "got: {:?}", stdout);
    assert!(stdout.contains("b1.name=beta\n"),        "got: {:?}", stdout);
    assert!(stdout.contains("b1.files.count=1\n"),    "got: {:?}", stdout);
}

#[test]
fn round_trip_builder_to_reader_preserves_structure() {
    // Emit via Builder, parse back via Reader. Exercises the
    // shared indent convention end-to-end.
    let src = r#"
        fn main() {
            let b = std::yaml::Builder { };
            let mut y = "";
            y = y + b.field(0, "schema_version", "1");
            y = y + b.nested_open(0, "binaries");
            y = y + b.list_item_first_q(2, "name", "echo");
            y = y + b.field_q(2, "rel", "cmd/echo");
            y = y + b.nested_open(2, "files");
            y = y + b.list_item_first_q(4, "file", "main.go");
            y = y + b.field_q(4, "package", "main");

            let r = std::yaml::Reader { text: y };
            println("schema=", r.scalar("schema_version"));
            println("n_bins=", to_string(r.list_count("binaries")));

            let bin = r.list_item("binaries", 0);
            let br = std::yaml::Reader { text: bin };
            println("bin.name=", br.scalar("name"));
            println("bin.rel=", br.scalar("rel"));
            println("bin.n_files=", to_string(br.list_count("files")));

            let f = br.list_item("files", 0);
            let fr = std::yaml::Reader { text: f };
            println("file.file=", fr.scalar("file"));
            println("file.package=", fr.scalar("package"));
        }
    "#;
    let (stdout, status) = build_and_run("roundtrip", src);
    assert!(status.success(), "stdout: {:?}", stdout);
    assert!(stdout.contains("schema=1\n"),               "got: {:?}", stdout);
    assert!(stdout.contains("n_bins=1\n"),               "got: {:?}", stdout);
    assert!(stdout.contains("bin.name=echo\n"),          "got: {:?}", stdout);
    assert!(stdout.contains("bin.rel=cmd/echo\n"),       "got: {:?}", stdout);
    assert!(stdout.contains("bin.n_files=1\n"),          "got: {:?}", stdout);
    assert!(stdout.contains("file.file=main.go\n"),      "got: {:?}", stdout);
    assert!(stdout.contains("file.package=main\n"),      "got: {:?}", stdout);
}

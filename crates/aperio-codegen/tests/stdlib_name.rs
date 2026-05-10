//! std::name::Convention — orthography helpers.

use std::process::Command;

use aperio_codegen::build_executable;

fn build_and_run(name: &str, source: &str) -> (String, std::process::ExitStatus) {
    let program = aperio_syntax::parse_source(source).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("aperio_test_stdlib_name_{}", name));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (String::from_utf8_lossy(&output.stdout).to_string(), output.status)
}

#[test]
fn propose_locus_name_matches_app_helper_output() {
    // Mirrors the byte shape that apps/onboard + apps/tower-join
    // emit via their hand-rolled __propose_locus_name: file stem
    // with the source extension stripped, snake segments
    // capitalized, "L" suffix appended.
    let src = r#"
        fn main() {
            let nc = std::name::Convention { strip: ".go" };
            println("a=", nc.propose_locus_name("main.go"));
            println("b=", nc.propose_locus_name("request_cache.go"));
            println("c=", nc.propose_locus_name("http_server_v2.go"));
            println("d=", nc.propose_locus_name("AlreadyCamel.go"));
            println("e=", nc.propose_locus_name(".go"));
        }
    "#;
    let (stdout, status) = build_and_run("propose", src);
    assert!(status.success());
    assert!(stdout.contains("a=MainL"),          "got: {:?}", stdout);
    assert!(stdout.contains("b=RequestCacheL"),  "got: {:?}", stdout);
    assert!(stdout.contains("c=HttpServerV2L"),  "got: {:?}", stdout);
    assert!(stdout.contains("d=AlreadyCamelL"),  "got: {:?}", stdout);
    assert!(stdout.contains("e=?L"),             "empty stem → ?L; got: {:?}", stdout);
}

#[test]
fn strip_extension_passes_through_when_extension_missing() {
    let src = r#"
        fn main() {
            let nc = std::name::Convention { strip: ".go" };
            println("a=", nc.strip_extension("main.go"));
            println("b=", nc.strip_extension("Makefile"));
            println("c=", nc.strip_extension("main.rs"));
            println("d=", nc.strip_extension(""));
        }
    "#;
    let (stdout, status) = build_and_run("strip", src);
    assert!(status.success());
    assert!(stdout.contains("a=main"),     "got: {:?}", stdout);
    assert!(stdout.contains("b=Makefile"), "missing ext → unchanged; got: {:?}", stdout);
    assert!(stdout.contains("c=main.rs"),  "wrong ext → unchanged; got: {:?}", stdout);
    assert!(stdout.contains("d="),         "empty → empty; got: {:?}", stdout);
}

#[test]
fn snake_to_camel_capitalizes_segments() {
    let src = r#"
        fn main() {
            let nc = std::name::Convention { };
            println("a=", nc.snake_to_camel("request_cache"));
            println("b=", nc.snake_to_camel("a"));
            println("c=", nc.snake_to_camel(""));
            println("d=", nc.snake_to_camel("__leading"));
            println("e=", nc.snake_to_camel("trailing_"));
        }
    "#;
    let (stdout, status) = build_and_run("s2c", src);
    assert!(status.success());
    assert!(stdout.contains("a=RequestCache"), "got: {:?}", stdout);
    assert!(stdout.contains("b=A"),            "got: {:?}", stdout);
    assert!(stdout.contains("c="),             "got: {:?}", stdout);
    // Leading underscores trigger next_upper; first letter still capitalized.
    assert!(stdout.contains("d=Leading"),      "got: {:?}", stdout);
    assert!(stdout.contains("e=Trailing"),     "got: {:?}", stdout);
}

#[test]
fn camel_to_snake_splits_before_uppercase() {
    let src = r#"
        fn main() {
            let nc = std::name::Convention { };
            println("a=", nc.camel_to_snake("RequestCache"));
            println("b=", nc.camel_to_snake("HTTP"));
            println("c=", nc.camel_to_snake("Single"));
            println("d=", nc.camel_to_snake(""));
        }
    "#;
    let (stdout, status) = build_and_run("c2s", src);
    assert!(status.success());
    assert!(stdout.contains("a=request_cache"), "got: {:?}", stdout);
    // No special acronym handling: HTTP → h_t_t_p (matches the
    // open-coded behavior; refinements come later if needed).
    assert!(stdout.contains("b=h_t_t_p"),       "got: {:?}", stdout);
    assert!(stdout.contains("c=single"),        "got: {:?}", stdout);
    assert!(stdout.contains("d="),              "got: {:?}", stdout);
}

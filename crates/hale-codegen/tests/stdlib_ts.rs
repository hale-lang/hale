//! m96: Hale surface for the tree-sitter substrate.
//!
//! `std::ts::*` exposes parse + walk via i64 handles (1-based,
//! 0 = absent / parse failure). These tests build small Hale
//! programs, run them, and assert on stdout — exercising the
//! full chain: parse → bundle-merge → lower → link with the
//! hale-ts-shim staticlib → run, with the tree-sitter Go
//! grammar bundled.
//!
//! The shim staticlib must already be built (workspace
//! `cargo build -p hale-ts-shim`); the codegen link step
//! locates `libhale_ts_shim.a` under the workspace target
//! dir. CI builds the workspace before running tests, so the
//! ordering is implicit there. For local `cargo test -p
//! hale-codegen` runs without a prior workspace build, the
//! `hale-ts-shim` direct dependency in this crate's Cargo
//! ensures cargo materializes it before these tests run.

use std::process::Command;


use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn build_and_run(name: &str, source: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(source).expect("parse");
    let bin = harness::unique_bin(name);
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        output.status,
    )
}

#[test]
fn parse_go_returns_nonzero_handle_and_root_kind_is_source_file() {
    let source = r#"
        fn main() {
            let src = "package main\n";
            let t = std::ts::parse_go(src);
            if t == 0 { println("FAIL parse"); return; }
            let r = std::ts::root_node(t);
            println("kind=", std::ts::node_kind(r));
        }
    "#;
    let (stdout, status) = build_and_run("root_kind", source);
    assert!(status.success(), "exited non-zero: {:?}", status);
    assert!(
        stdout.contains("kind=source_file"),
        "expected root kind to be source_file; stdout: {:?}",
        stdout
    );
}

#[test]
fn parse_go_named_child_count_matches_grammar() {
    // A program with one package clause + one function decl has
    // exactly two top-level named children at the source_file
    // level. Validates the named-child count + walk primitives.
    let source = r#"
        fn main() {
            let src = "package main\nfunc foo() {}\n";
            let t = std::ts::parse_go(src);
            let r = std::ts::root_node(t);
            println("n=", std::ts::node_named_child_count(r));
            let c0 = std::ts::node_named_child(r, 0);
            let c1 = std::ts::node_named_child(r, 1);
            println("c0=", std::ts::node_kind(c0));
            println("c1=", std::ts::node_kind(c1));
        }
    "#;
    let (stdout, status) = build_and_run("named_child_count", source);
    assert!(status.success(), "exited non-zero: {:?}", status);
    assert!(stdout.contains("n=2"), "stdout: {:?}", stdout);
    assert!(stdout.contains("c0=package_clause"), "stdout: {:?}", stdout);
    assert!(
        stdout.contains("c1=function_declaration"),
        "stdout: {:?}",
        stdout
    );
}

#[test]
fn node_text_returns_source_slice() {
    // Exercise node_text + start/end byte. The package clause
    // spans bytes 0..12 of "package main\n..." ("package main").
    let source = r#"
        fn main() {
            let src = "package main\nfunc foo() {}\n";
            let t = std::ts::parse_go(src);
            let r = std::ts::root_node(t);
            let pkg = std::ts::node_named_child(r, 0);
            println("start=", std::ts::node_start_byte(pkg));
            println("end=", std::ts::node_end_byte(pkg));
            println("text=", std::ts::node_text(pkg));
        }
    "#;
    let (stdout, status) = build_and_run("node_text", source);
    assert!(status.success(), "exited non-zero: {:?}", status);
    assert!(stdout.contains("start=0"), "stdout: {:?}", stdout);
    assert!(stdout.contains("end=12"), "stdout: {:?}", stdout);
    assert!(stdout.contains("text=package main"), "stdout: {:?}", stdout);
}

#[test]
fn invalid_handles_return_sentinels() {
    // Probing past the tree's child count returns 0 (no node)
    // rather than a crash; querying kind on the sentinel returns
    // the empty string (empty kind line in stdout).
    let source = r#"
        fn main() {
            let src = "package main\n";
            let t = std::ts::parse_go(src);
            let r = std::ts::root_node(t);
            let nope = std::ts::node_named_child(r, 999);
            println("oob=", nope);
            println("kind=[", std::ts::node_kind(nope), "]");
        }
    "#;
    let (stdout, status) = build_and_run("oob", source);
    assert!(status.success(), "exited non-zero: {:?}", status);
    assert!(stdout.contains("oob=0"), "stdout: {:?}", stdout);
    assert!(stdout.contains("kind=[]"), "stdout: {:?}", stdout);
}

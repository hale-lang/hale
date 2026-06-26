//! Two `or { block }` disposer codegen gaps (found writing a real HTTP
//! service in Hale — `std::process::run` + `std::io::fs` handlers).
//!
//! Bug A — a diverging `or { … }` block (always `return`/`fail`) produces no
//! substitute value, so it must type-check against ANY success type. v0
//! synthesized a placeholder `(undef, Int)` for the block's unreachable
//! fall-through and ran it through the substitute-type check, so the block
//! only compiled when the fallible's success type happened to be `Int`.
//! Fixed by detecting the terminated err branch (like `or raise`) and
//! skipping the substitute-type check.
//!
//! Bug B — a `{ block }` disposer on a Unit-success fallible (e.g.
//! `write_file(…) or { println(…); }`) was lowered as `Stmt::Expr(Block)`,
//! which `lower_stmt` has no arm for → "expression statement other than
//! locus literal or builtin call". Fixed by lowering a block disposer as a
//! block, not a wrapped statement.

use std::process::Command;

use hale_codegen::build_executable;

fn build_and_run(name: &str, src: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut bin = std::env::temp_dir();
    bin.push(format!("hale_or_block_{}_{}", name, std::process::id()));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (String::from_utf8_lossy(&out.stdout).to_string(), out.status)
}

// --- Bug A: diverging block against non-Int success types ----------------

#[test]
fn diverging_or_block_compiles_for_string_success() {
    // read_file's success type is String; the diverging block must not be
    // forced to type as Int.
    let src = r#"
        fn b() -> String {
            let s = std::io::fs::read_file("/nope") or { return "fallback"; };
            return s;
        }
        fn main() { println(b()); }
    "#;
    let (out, status) = build_and_run("string_success", src);
    assert!(status.success());
    assert_eq!(out.trim(), "fallback");
}

#[test]
fn diverging_or_block_compiles_for_bytes_success() {
    // read_bytes' success type is Bytes.
    let src = r#"
        fn b() -> Int {
            let data = std::io::fs::read_bytes("/nope") or { return -1; };
            return len(data);
        }
        fn main() { println(b()); }
    "#;
    let (out, status) = build_and_run("bytes_success", src);
    assert!(status.success());
    assert_eq!(out.trim(), "-1");
}

#[test]
fn diverging_or_block_compiles_for_struct_success() {
    // process::run's success type is the ProcessOutput struct.
    let src = r#"
        fn r() -> String {
            let out = std::process::run("false") or { return "spawn-failed"; };
            return "ran";
        }
        fn main() { println(r()); }
    "#;
    let (out, status) = build_and_run("struct_success", src);
    assert!(status.success());
    assert_eq!(out.trim(), "ran");
}

#[test]
fn or_int_block_still_compiles() {
    // Regression guard: the success type that *did* work (Int) still does.
    let src = r#"
        fn a() -> String {
            let v = std::str::parse_int("x") or { return "fallback"; };
            return to_string(v);
        }
        fn main() { println(a()); }
    "#;
    let (out, status) = build_and_run("int_success", src);
    assert!(status.success());
    assert_eq!(out.trim(), "fallback");
}

// --- Bug B: `{ block }` disposer on a Unit-success fallible ----------------

#[test]
fn statement_position_or_block_disposer() {
    // write_file is `() fallible(IoError)`; an effectful block disposer in
    // statement position must run (not hit the expression-statement reject).
    let src = r#"
        fn main() {
            std::io::fs::write_file("/nope", "x") or { println("write failed"); };
            println("after");
        }
    "#;
    let (out, status) = build_and_run("stmt_block", src);
    assert!(status.success());
    assert_eq!(out.trim(), "write failed\nafter");
}

#[test]
fn diverging_block_in_statement_position() {
    // A diverging block disposer in statement position bails out of the fn.
    let src = r#"
        fn go() {
            std::io::fs::write_file("/nope", "x") or { println("bailing"); return; };
            println("after");
        }
        fn main() { go(); }
    "#;
    let (out, status) = build_and_run("stmt_block_diverge", src);
    assert!(status.success());
    assert_eq!(out.trim(), "bailing");
}

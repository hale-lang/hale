//! GH #808: what a build does when `libhale_ts_shim.a` is absent.
//!
//! The tree-sitter shim is a `crate-type = ["staticlib"]` sibling
//! crate that nothing declares a Cargo dependency on, so
//! `cargo build -p hale-cli` (the spot-check invocation CLAUDE.md
//! documents) produces a `hale` that cannot find it. Two things
//! used to go wrong in that state, and both are asserted here:
//!
//!   1. `-lm` rode along INSIDE the shim's link arm, so every
//!      `std::math::*` program failed with
//!      `ld.lld: error: undefined symbol: tanh` — a message about
//!      the user's program, caused by the toolchain. libm is now
//!      linked unconditionally.
//!   2. A program that really does need the shim hit the same
//!      undefined-symbol wall (`lotus_ts_parse_go`). It now gets a
//!      `CodegenError::MissingTsShim` carrying the `std::ts::*`
//!      call site and the command that builds the staticlib.
//!
//! `HALE_NO_TS_SHIM=1` forces the lookup to miss, so these run in
//! a fully-built workspace. It is process-global, so both tests
//! take one mutex and set the same value; nothing here ever unsets
//! it.

use std::process::Command;
use std::sync::Mutex;

use hale_codegen::{build_executable, CodegenError};

#[path = "support/harness.rs"]
mod harness;

static SHIM_OFF: Mutex<()> = Mutex::new(());

/// Run `f` with the shim lookup forced to `None`. Serialized so
/// the `set_var` never races a concurrent read in the sibling test.
fn without_ts_shim<R>(f: impl FnOnce() -> R) -> R {
    let _guard = SHIM_OFF.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("HALE_NO_TS_SHIM", "1");
    f()
}

#[test]
fn math_program_links_and_runs_without_the_ts_shim() {
    // The exact reproducer from the issue. `tanh` resolves out of
    // libm; nothing here touches `std::ts`.
    let src = r#"
fn main() {
    println("tanh0=", std::math::tanh(0.0));
}
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin("libm_no_ts_shim");
    let out = without_ts_shim(|| {
        build_executable(&program, &bin)
            .expect("a std::math program must link without the ts shim");
        Command::new(&bin).output().expect("run")
    });
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "exited non-zero: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("tanh0=0"),
        "expected tanh(0.0) == 0; stdout: {:?}",
        stdout
    );
}

#[test]
fn std_ts_program_gets_a_located_refusal_not_a_link_failure() {
    let src = r#"
fn main() {
    let t = std::ts::parse_go("package main\n");
    println("t=", t);
}
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin("std_ts_no_shim");
    let err = without_ts_shim(|| {
        build_executable(&program, &bin)
            .expect_err("a std::ts program must be refused without the shim")
    });
    let _ = std::fs::remove_file(&bin);
    let (msg, span) = match err {
        CodegenError::MissingTsShim(msg, span) => (msg, span),
        // Notably NOT CodegenError::Link — that is the regression.
        other => panic!("expected MissingTsShim, got {:?}", other),
    };
    assert!(
        msg.contains("libhale_ts_shim.a"),
        "message must name the missing artifact: {:?}",
        msg
    );
    assert!(
        msg.contains("cargo build --release"),
        "message must name the command that produces it: {:?}",
        msg
    );
    let span = span.expect("refusal must be located at the std::ts call");
    let cited = &src[span.start.as_usize()..span.end.as_usize()];
    assert!(
        cited.contains("std::ts::parse_go"),
        "span must cite the std::ts call site; cited {:?}",
        cited
    );
}

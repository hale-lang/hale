//! Runs the repo's own Hale-language test suite (`tests/hale/`)
//! through `hale test`.
//!
//! ## Why the compiler should test itself in its own language
//!
//! `hale test` ships in the same binary as `hale build` and is
//! documented in `spec/testing.md` — and until now the repo
//! contained exactly four `*_test.hl` files, all of them fixtures
//! for testing the runner. The compiler did not use its own test
//! framework on itself.
//!
//! Two things change when a behaviour test moves from Rust into Hale:
//!
//!   * **The expectation stops being transcribed.** The Rust form is
//!     "compile a program that prints, then substring-match the
//!     output from another language". The Hale form is an assertion
//!     next to the code. There is no second copy to drift, and
//!     `assert_eq_int(n, 42)` is strictly stronger than
//!     `stdout.contains("a=42")` — which also passes on `a=421`.
//!     The suite leans heavily on substring matching: 2,553
//!     `.contains(` against 256 `assert_eq!`.
//!
//!   * **The program gets typechecked.** `build_executable` — what
//!     the codegen tests call — parses and lowers but never runs
//!     `check_program`. So those programs are compiled and executed
//!     without ever being checked. Measured across the corpus, 8.5%
//!     of the programs embedded in codegen tests do not pass `hale
//!     check`. The first conversion here hit that immediately: the
//!     `err.kind` shape, which `docs/src/everyday/http.md` and
//!     `spec/decisions.md` both show, failed to typecheck — a live
//!     bug no Rust test could see.
//!
//! ## What does NOT belong here
//!
//! Tests that assert on *compiler output* rather than program
//! behaviour — diagnostics, LLVM IR shape, leak counts, obs-segment
//! records. Those stay in Rust, and `stdlib_str.rs` keeps exactly
//! its two diagnostic tests for that reason.

use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p
}

#[test]
fn hale_native_suite_passes() {
    let dir = repo_root().join("tests/hale");
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("test")
        .arg(&dir)
        .output()
        .expect("invoke hale test tests/hale");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "the Hale-language test suite failed.\nstdout:\n{}\nstderr:\n{}",
        stdout,
        stderr
    );
    assert!(
        stdout.contains(", 0 failed"),
        "expected a passing summary, got:\n{}",
        stdout
    );
}

/// A suite that quietly emptied would pass the check above by
/// running nothing.
#[test]
fn hale_native_suite_is_not_empty() {
    let dir = repo_root().join("tests/hale");
    let n = std::fs::read_dir(&dir)
        .expect("tests/hale exists")
        .flatten()
        .filter(|e| {
            e.path()
                .file_name()
                .map(|f| f.to_string_lossy().ends_with("_test.hl"))
                .unwrap_or(false)
        })
        .count();
    assert!(
        n >= 2,
        "expected the Hale-native suite to hold tests, found {}",
        n
    );
}

//! `hale verify` over loops in a long-lived `run()` that make a fresh
//! Bytes per iteration (`hale_types::iteration_reclaim`).
//!
//! A loop body that assigns no binding declared before it (beyond
//! scalars and locus references) frees what each iteration made when the
//! iteration ends, so the natural downloader loop (`let c =
//! body.chunk() or { break; }; h.update(c); out.write_bytes(c)`) and a
//! builder snapshot per iteration, with a `break` handler or a fallback
//! literal, verify clean. A loop that keeps each iteration's value in an
//! outer binding still grows, and the survey names the allocating call,
//! not a literal near it: before, the call was invisible to it (a
//! literal was its only site), so the shape that grew passed and the
//! `or b""` spelling was named at the `b""`.

use std::path::PathBuf;
use std::process::Command;

fn fixture(name: &str) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.extend(["tests", "fixtures", "run-loop-reclaim", name]);
    p
}

fn verify(name: &str) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("verify")
        .arg(fixture(name))
        .output()
        .expect("invoke hale verify");
    (out.status.success(), String::from_utf8_lossy(&out.stderr).to_string())
}

#[test]
fn loops_reclaimed_per_iteration_verify_clean() {
    let (ok, stderr) = verify("reclaimed.hl");
    assert!(ok, "hale verify refused loops that are reclaimed per iteration:\n{stderr}");
    assert!(!stderr.contains("unbounded allocation"), "the survey named a reclaimed loop:\n{stderr}");
}

#[test]
fn a_loop_that_keeps_its_iterations_is_named_at_the_call() {
    let (ok, stderr) = verify("still_grows.hl");
    assert!(!ok, "hale verify passed a loop that keeps every snapshot:\n{stderr}");
    let findings: Vec<&str> = stderr.lines().filter(|l| l.contains("unbounded allocation")).collect();
    assert_eq!(findings.len(), 1, "one finding, at the call:\n{stderr}");
    assert!(
        findings[0].contains("still_grows.hl:14:21:") && findings[0].contains("fresh value `snapshot(..)` returns"),
        "the finding names `buf.snapshot()` (line 14, column 21):\n{stderr}"
    );
}

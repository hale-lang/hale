//! The runtime and the model must agree on which subjects a locus
//! may publish to.
//!
//! `lotus_wildcard_match` (runtime/lotus_arena.c) enforces a
//! computed-subject publish against the locus's declared patterns.
//! `hale_types::wildcard_match` is what the checker, the bus graph,
//! the judgments and the model use to answer the same question. Two
//! implementations of one predicate drift; when these two drift, a
//! publish the model proved impossible becomes possible at runtime —
//! the exact defect the enforcement closes.
//!
//! So both run over ONE case table here.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

#[path = "support/harness.rs"]
mod harness;

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn build_driver() -> PathBuf {
    let mut runtime_c = manifest_dir();
    runtime_c.push("runtime");
    runtime_c.push("lotus_arena.c");
    let mut driver_c = manifest_dir();
    driver_c.push("tests");
    driver_c.push("wildcard_driver.c");
    let bin = harness::unique_bin("hale_wildcard_parity_driver");
    let status = Command::new("clang")
        .arg(driver_c)
        .arg(runtime_c)
        .arg("-O2")
        .arg("-lpthread")
        .arg("-o")
        .arg(&bin)
        .status()
        .expect("clang invocation");
    assert!(status.success(), "clang failed building wildcard driver");
    bin
}

/// (pattern, subject) pairs. Deliberately includes the edges where a
/// hand port is most likely to disagree: the pattern root with no
/// trailing dot, a bare `**`, a `**` that is not dot-anchored, an
/// interior `**`, empty strings, and near-miss prefixes.
fn cases() -> Vec<(&'static str, &'static str)> {
    let pats = [
        "io.tcp.**",
        "**",
        "log.**",
        "a**",
        "io.**.tcp",
        "io.tcp",
        "",
        "app.order",
        ".**",
    ];
    let subjects = [
        "io.tcp",
        "io.tcp.venue",
        "io.tcp.venue.deep",
        "io.tcpX",
        "io.tcp.",
        "app.order",
        "log",
        "log.a",
        "",
        "a",
        "ab",
        "io",
        ".x",
    ];
    let mut out = Vec::new();
    for p in pats {
        for s in subjects {
            out.push((p, s));
        }
    }
    out
}

#[test]
fn runtime_and_model_wildcard_match_agree() {
    let cases = cases();
    let bin = build_driver();
    let mut child = Command::new(&bin)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn wildcard driver");
    {
        let stdin = child.stdin.as_mut().expect("stdin");
        for (p, s) in &cases {
            writeln!(stdin, "{}\t{}", p, s).expect("write case");
        }
    }
    let out = child.wait_with_output().expect("driver output");
    assert!(out.status.success(), "driver exited non-zero");
    let text = String::from_utf8_lossy(&out.stdout);
    let c_results: Vec<bool> = text
        .lines()
        .map(|l| l.trim() == "1")
        .collect();
    assert_eq!(
        c_results.len(),
        cases.len(),
        "driver returned {} rows for {} cases",
        c_results.len(),
        cases.len()
    );
    let mut disagreements = Vec::new();
    for ((p, s), c) in cases.iter().zip(c_results.iter()) {
        let rust = hale_types::wildcard_match(p, s);
        if rust != *c {
            disagreements.push(format!(
                "pattern `{}` subject `{}`: rust={} c={}",
                p, s, rust, c
            ));
        }
    }
    let _ = std::fs::remove_file(&bin);
    assert!(
        disagreements.is_empty(),
        "runtime and model disagree on {} case(s):\n{}",
        disagreements.len(),
        disagreements.join("\n")
    );
}

/// The property the enforcement actually rests on: a locus declaring
/// `io.tcp.**` must never be able to publish an unrelated
/// application subject. Stated separately from the parity table so
/// it survives someone rewriting the table.
#[test]
fn a_declared_pattern_does_not_authorize_a_foreign_subject() {
    assert!(!hale_types::wildcard_match("io.tcp.**", "app.order"));
    assert!(!hale_types::wildcard_match("io.tcp.**", "io.tcpX"));
    assert!(hale_types::wildcard_match("io.tcp.**", "io.tcp"));
    assert!(hale_types::wildcard_match("io.tcp.**", "io.tcp.venue"));
}

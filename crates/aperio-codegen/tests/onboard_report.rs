//! The polished codebase recognition report — apps/onboard.
//!
//! End-to-end test of the agent-friendly middle-step product.
//! Asserts on the structured text output (header, per-locus
//! boxes, unknowns section, narrative). Substring matching
//! rather than line-exact comparison so cosmetic tweaks to the
//! report don't churn the test suite.

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use aperio_codegen::build_executable;

fn workspace_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p
}

fn build() -> PathBuf {
    let src_path = workspace_root()
        .join("apps")
        .join("onboard")
        .join("main.ap");
    let src = std::fs::read_to_string(&src_path).expect("read main.ap");
    let program = aperio_syntax::parse_source(&src).expect("parse main.ap");
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut bin = std::env::temp_dir();
    bin.push(format!(
        "aperio_onboard_{}_{}",
        std::process::id(),
        nanos
    ));
    build_executable(&program, &bin).expect("build onboard");
    bin
}

fn run_against(fixture_subdir: &str) -> String {
    let bin = build();
    let fixture = workspace_root().join("apps").join(fixture_subdir).join("fixture");
    let out = Command::new(&bin)
        .arg(fixture)
        .output()
        .expect("run onboard");
    let _ = std::fs::remove_file(&bin);
    assert!(
        out.status.success(),
        "onboard exited non-zero: {:?}; stderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

#[test]
fn header_announces_recognition_and_flavor() {
    let report = run_against("operational-graph");
    assert!(
        report.contains("Aperio recognition:"),
        "missing recognition header; output:\n{}",
        report
    );
    assert!(
        report.contains("flavor: go"),
        "missing flavor line; output:\n{}",
        report
    );
}

#[test]
fn each_locus_gets_a_box_with_verdict_marker() {
    let report = run_against("operational-graph");
    // Every file has agreement>=2 → all four box headers carry [locus].
    for name in ["MainL", "HandlersL", "WorkerL", "StoreL"] {
        let box_header = format!("+-- {}", name);
        assert!(
            report.contains(&box_header),
            "missing box header for {}; output:\n{}",
            name,
            report
        );
    }
    let locus_count = report.matches("[locus]").count();
    assert_eq!(
        locus_count, 4,
        "expected 4 [locus] verdicts; got {}; output:\n{}",
        locus_count, report
    );
}

#[test]
fn aperio_shape_interpretation_appears_per_locus() {
    let report = run_against("operational-graph");
    // main.go gets the "root locus" shape.
    assert!(
        report.contains("root locus"),
        "missing root locus interpretation; output:\n{}",
        report
    );
    // handlers.go gets the bus-subscriber shape.
    assert!(
        report.contains("bus-subscriber locus"),
        "missing bus-subscriber interpretation; output:\n{}",
        report
    );
    // worker.go gets the long-running shape.
    assert!(
        report.contains("long-running"),
        "missing long-running interpretation; output:\n{}",
        report
    );
    // store.go gets the state-holding shape.
    assert!(
        report.contains("state-holding"),
        "missing state-holding interpretation; output:\n{}",
        report
    );
}

#[test]
fn unknowns_section_lists_each_unknown_with_file_and_action() {
    // store.go has 3 type names with morphemes the seed lookup
    // doesn't cover (Request, Session, Audit). All should
    // surface in the unknowns section.
    let report = run_against("operational-graph");
    assert!(
        report.contains("Unknowns flagged for agent review"),
        "missing unknowns section; output:\n{}",
        report
    );
    for tname in ["RequestCache", "SessionManager", "AuditLogger"] {
        let needle = format!("store.go : {}", tname);
        assert!(
            report.contains(&needle),
            "missing unknown entry for {}; output:\n{}",
            tname,
            report
        );
    }
    // Each unknown carries an "action: open ..." prompt.
    let action_count = report.matches("action: open store.go").count();
    assert_eq!(
        action_count, 3,
        "expected 3 action prompts; got {}; output:\n{}",
        action_count, report
    );
}

#[test]
fn narrative_summarizes_recognition_outcome() {
    let report = run_against("operational-graph");
    assert!(
        report.contains("Recognition\n-----------"),
        "missing recognition section; output:\n{}",
        report
    );
    assert!(
        report.contains("4 file(s) exhibit lotus shape"),
        "missing per-file count narrative; output:\n{}",
        report
    );
    // Operational fixture has 3 unknowns → narrative mentions
    // the agent-review prompt.
    assert!(
        report.contains("3 unknown morpheme(s) flagged"),
        "missing unknown count in narrative; output:\n{}",
        report
    );
}

#[test]
fn import_graph_fixture_renders_mixed_verdicts() {
    // import-graph fixture has 1 locus, 2 type_or_fn, 1 structural.
    let report = run_against("import-graph");
    assert_eq!(
        report.matches("[locus]").count(),
        1,
        "expected 1 [locus] verdict; output:\n{}",
        report
    );
    assert_eq!(
        report.matches("[type_or_fn]").count(),
        2,
        "expected 2 [type_or_fn] verdicts; output:\n{}",
        report
    );
    assert_eq!(
        report.matches("[structural]").count(),
        1,
        "expected 1 [structural] verdict; output:\n{}",
        report
    );
}

#[test]
fn no_fabricated_motion_forms_in_report() {
    // Negative: the fixtures must produce <unknown:X> markers
    // for short morphemes (Order, User, Audit, Request,
    // Session) — never fabricated motion-forms.
    let report = run_against("operational-graph");
    for fab in ["ording", "using", "auditing-", "requesting-", "sessioning-"] {
        assert!(
            !report.contains(fab),
            "regression: fabricated motion {} detected; output:\n{}",
            fab, report
        );
    }
}

#[test]
fn shape_rules_doc_referenced_for_agents() {
    // The unknowns section must point the agent at the canonical
    // rules doc so the recognition is reproducible across
    // sessions.
    let report = run_against("operational-graph");
    assert!(
        report.contains("notes/onboarding-shape-rules.md"),
        "missing shape-rules doc reference; output:\n{}",
        report
    );
}

//! Runs the DNA Phase 0 domain proof (`dna/tests/`, GH #526) through
//! `hale test`, and `hale verify` over its core, so a compiler change
//! that breaks the domain shapes fails the build here rather than in
//! a friction log. `dna/core` is a library seed (imported by the
//! tests); every fixture is an ordinary Hale program that exits 0.

use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p
}

#[test]
fn dna_fixtures_pass() {
    let dir = repo_root().join("dna/tests");
    // The editing fixture runs the toolchain (`hale fmt`, `hale check`)
    // inside its worktree: hand it this build, not whatever is on PATH.
    // GH #583 K1: the knowledge store's Postgres half runs when a DSN is
    // in the environment (CI's service container; a developer's compose)
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_hale"));
    cmd.arg("test").arg(&dir).env("HALE_BIN", env!("CARGO_BIN_EXE_hale"));
    // Fixtures run from a directory of their own, never from inside this
    // repository: an organism's defaults are relative to where it runs
    // (a gateway's worktrees, a file store's receipts), and a fixture
    // that trips one must not write into the checkout — a worktree of the
    // whole repository under crates/hale-cli moved the corpus baseline
    // for a parallel test.
    cmd.current_dir(std::env::temp_dir());
    if let Ok(dsn) = std::env::var("HALE_DNA_KNOWLEDGE_DSN") {
        cmd.env("HALE_DNA_KNOWLEDGE_DSN", dsn);
    }
    let out = cmd.output().expect("invoke hale test dna/tests");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "the DNA fixtures failed.\nstdout:\n{}\nstderr:\n{}",
        stdout,
        stderr
    );
    assert!(stdout.contains(", 0 failed"), "expected a passing summary, got:\n{}", stdout);
}

#[test]
fn dna_core_verifies_clean() {
    let dir = repo_root().join("dna/core");
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("verify")
        .arg(&dir)
        .output()
        .expect("invoke hale verify dna/core");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "hale verify dna/core must report zero findings.\nstdout:\n{}\nstderr:\n{}",
        stdout,
        stderr
    );
}

/// A suite that quietly emptied would pass the check above by
/// running nothing. #526 names eight programs; seven are Hale-native,
/// #528 adds the hosted-model adapter's, #529 the gateways' and
/// the source-editing Attempt's, #583 the budget's and the Anthropic
/// adapter's.
#[test]
fn dna_fixture_set_is_complete() {
    let dir = repo_root().join("dna/tests");
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .expect("dna/tests exists")
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            n.ends_with("_test.hl").then_some(n)
        })
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            "acceptance_binding_test.hl",
            "anthropic_messages_test.hl",
            "apply_test.hl",
            "assembly_test.hl",
            "body_lease_blocked_test.hl",
            "body_lease_start_test.hl",
            "body_provision_script_test.hl",
            "budget_test.hl",
            "deployment_test.hl",
            "editing_test.hl",
            "effect_outcomes_test.hl",
            "extensions_test.hl",
            "fanout_join_test.hl",
            "grant_layering_test.hl",
            "grant_resources_test.hl",
            "handed_task_test.hl",
            "handoff_test.hl",
            "harness_test.hl",
            "journal_contention_test.hl",
            "journal_test.hl",
            "knowledge_context_test.hl",
            "knowledge_events_test.hl",
            "knowledge_store_test.hl",
            "knowledge_test.hl",
            "mutation_review_test.hl",
            "openai_chat_test.hl",
            "optimize_test.hl",
            "org_test.hl",
            "performers_test.hl",
            "plan_routing_test.hl",
            "practice_test.hl",
            "principal_oidc_test.hl",
            "principal_test.hl",
            "prompt_receipt_test.hl",
            "receipt_retention_test.hl",
            "receipt_vault_test.hl",
            "recorded_model_test.hl",
            "recursion_settlement_test.hl",
            "rehydrate_work_test.hl",
            "retired_admission_test.hl",
            "review_authority_test.hl",
            "schedule_cli_test.hl",
            "schedule_test.hl",
            "supersession_test.hl",
            "task_decide_test.hl",
            "task_evidence_test.hl",
            "verification_test.hl",
            "workspace_test.hl",
        ]
    );
}

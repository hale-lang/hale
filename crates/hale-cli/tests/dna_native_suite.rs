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
    // the books slice copies its application from dna/acceptance
    cmd.env("HALE_DNA_SOURCE", repo_root());
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
    // #637: every fixture's scratch is reaped when the fixture ends
    // (`dna::reap_on_exit`); nothing a fixture started outlives the run
    std::thread::sleep(std::time::Duration::from_secs(5));
    let ps = Command::new("ps").args(["-eo", "pid=,args="]).output().expect("ps");
    let me = std::process::id().to_string();
    let left: Vec<String> = String::from_utf8_lossy(&ps.stdout)
        .lines()
        .filter(|l| l.contains("/tmp/dna-") && l.split_whitespace().next() != Some(me.as_str()))
        .map(str::to_string)
        .collect();
    assert!(left.is_empty(), "processes a DNA fixture started are still running:\n{}", left.join("\n"));
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
            "b1_team_test.hl",
            "body_lease_blocked_test.hl",
            "body_lease_start_test.hl",
            "body_provision_script_test.hl",
            "body_scan_test.hl",
            "books_slice_test.hl",
            "budget_test.hl",
            "concern_identity_test.hl",
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
            "lease_epoch_test.hl",
            "ledger_test.hl",
            "membrane_loss_test.hl",
            "mutation_review_test.hl",
            "openai_chat_test.hl",
            "optimize_test.hl",
            "org_test.hl",
            "owners_test.hl",
            "ownership_test.hl",
            "performers_test.hl",
            "plan_routing_test.hl",
            "practice_test.hl",
            "principal_oidc_test.hl",
            "principal_test.hl",
            "prompt_receipt_test.hl",
            "receipt_retention_test.hl",
            "receipt_vault_test.hl",
            "record_test.hl",
            "recorded_model_test.hl",
            "recovery_association_test.hl",
            "recovery_two_memories_test.hl",
            "recursion_settlement_test.hl",
            "rehydrate_work_test.hl",
            "relay_repeated_request_test.hl",
            "retired_admission_test.hl",
            "review_authority_test.hl",
            "routed_attempt_identity_test.hl",
            "routing_test.hl",
            "schedule_cli_test.hl",
            "schedule_test.hl",
            "supersession_test.hl",
            "task_decide_test.hl",
            "task_evidence_test.hl",
            "two_heads_test.hl",
            "two_owners_test.hl",
            "verification_test.hl",
            "workspace_test.hl",
        ]
    );
}

/// GH #646 stage 0 (#649): the record has an API of its own, and only its
/// implementation spells the tool. In the core and the host, `git` is
/// invoked from exactly the record's implementation and the genome's own
/// files (the Structure): a new call site anywhere else is a boundary
/// crossed, and this fails the build until it goes behind `dna::Record`
/// (or `genome.hl` in the host).
#[test]
fn only_the_record_and_the_genome_spell_git() {
    let root = repo_root();
    let allowed = ["dna/core/record.hl", "dna/core/workspace.hl", "dna/core/verification.hl", "dna/core/org.hl", "dna/host/genome.hl"];
    let mut offenders = Vec::new();
    for dir in ["dna/core", "dna/host"] {
        for e in std::fs::read_dir(root.join(dir)).expect("dna dir").flatten() {
            let p = e.path();
            if p.extension().map(|x| x != "hl").unwrap_or(true) {
                continue;
            }
            let rel = format!("{dir}/{}", p.file_name().unwrap().to_string_lossy());
            let text = std::fs::read_to_string(&p).unwrap();
            let spells = text.lines().enumerate().filter(|(_, l)| {
                let l = l.trim_start();
                !l.starts_with("//") && (l.contains("run_tool(\"git") || l.contains("\"git\\n") || l.contains("\\ngit\\n") || l.contains("process::run(\"git"))
            });
            for (i, l) in spells {
                if !allowed.contains(&rel.as_str()) {
                    offenders.push(format!("{rel}:{}: {}", i + 1, l.trim()));
                }
            }
        }
    }
    assert!(offenders.is_empty(), "git is spelled outside the record's implementation and the genome's files:\n{}", offenders.join("\n"));
}

/// GH #647: a body's infrastructure and transport are implementations
/// behind the core's interfaces. In the host, `ssh`, `systemctl`,
/// `journalctl` and `docker compose` are invoked from exactly one file.
#[test]
fn only_the_reference_infrastructure_spells_its_tools() {
    let root = repo_root();
    let mut offenders = Vec::new();
    for e in std::fs::read_dir(root.join("dna/host")).expect("dna/host").flatten() {
        let p = e.path();
        if p.extension().map(|x| x != "hl").unwrap_or(true) || p.file_name().unwrap() == "infra.hl" {
            continue;
        }
        let text = std::fs::read_to_string(&p).unwrap();
        for (i, l) in text.lines().enumerate() {
            let t = l.trim_start();
            if t.starts_with("//") {
                continue;
            }
            if t.contains("ssh -o") || t.contains("systemctl --user") || t.contains("journalctl") || t.contains("docker\\ncompose") {
                offenders.push(format!("dna/host/{}:{}: {}", p.file_name().unwrap().to_string_lossy(), i + 1, t));
            }
        }
    }
    assert!(offenders.is_empty(), "a tool of the body's infrastructure is spelled outside dna/host/infra.hl:\n{}", offenders.join("\n"));
}

/// GH #648: the code-review host is an implementation behind `Forge`;
/// `gh` is invoked from exactly one file in the host.
#[test]
fn only_the_github_forge_spells_gh() {
    let root = repo_root();
    let mut offenders = Vec::new();
    for dir in ["dna/core", "dna/host"] {
        for e in std::fs::read_dir(root.join(dir)).expect("dna dir").flatten() {
            let p = e.path();
            if p.extension().map(|x| x != "hl").unwrap_or(true) || p.file_name().unwrap() == "forge_github.hl" {
                continue;
            }
            let text = std::fs::read_to_string(&p).unwrap();
            for (i, l) in text.lines().enumerate() {
                let t = l.trim_start();
                if !t.starts_with("//") && (t.contains("run_tool(\"gh") || t.contains("\"gh\\n")) {
                    offenders.push(format!("{dir}/{}:{}: {}", p.file_name().unwrap().to_string_lossy(), i + 1, t));
                }
            }
        }
    }
    assert!(offenders.is_empty(), "gh is spelled outside dna/host/forge_github.hl:\n{}", offenders.join("\n"));
}

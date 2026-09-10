//! #526 fixture 8: DNA law. Assertions about COMPILER OUTPUT —
//! refused builds and their witnesses — stay in Rust (CLAUDE.md), so
//! each fixture under `dna/tests/law/<name>/` is checked here and its
//! diagnostics matched. `*_pass` fixtures must check clean; `*_fail`
//! fixtures must be refused naming the claim and the witness.

use std::path::PathBuf;
use std::process::Command;

fn law_dir(name: &str) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p.join("dna/tests/law").join(name)
}

fn check(name: &str) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("check")
        .arg(".")
        .current_dir(law_dir(name))
        .output()
        .expect("invoke hale check");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), text)
}

fn passes(name: &str) {
    let (ok, text) = check(name);
    assert!(ok, "{name} must check clean:\n{text}");
}

fn refused(name: &str, needles: &[&str]) {
    let (ok, text) = check(name);
    assert!(!ok, "{name} must be refused:\n{text}");
    for n in needles {
        assert!(text.contains(n), "{name}: expected `{n}` in:\n{text}");
    }
}

/// Customer-side loci wired to the private router never reach an
/// external model: a structural fact, not a runtime `allows()` check.
#[test]
fn customer_data_confined_to_local_model() {
    passes("confine_pass");
}

/// The same locus wired to the full router: the witness names the
/// path into `effects(external_model)`, whatever runtime policy says.
#[test]
fn customer_data_reaching_external_model_is_refused_with_a_witness() {
    refused(
        "confine_fail",
        &[
            "claim `no_leak` violated",
            "effects(external_model)",
            "CustomerTriage::triage",
            "dna::ModelRouter::ask",
        ],
    );
}

/// Every apply path passes the Dna gate; constructing a permissive
/// review policy does not weaken the adopted constitution.
#[test]
fn apply_gated_through_the_assembly_holds() {
    passes("apply_gate_pass");
}

/// A concrete bypass of the gate is refused naming the carrier.
#[test]
fn apply_bypassing_the_gate_is_refused() {
    refused(
        "apply_gate_fail",
        &[
            "claim `apply_gated` violated",
            "effects(genome_apply)",
            "dna::LocalApplyDeployment::apply",
        ],
    );
}

#[test]
fn credential_sources_sealed_holds() {
    passes("sealed_pass");
}

#[test]
fn unsealed_credential_holder_is_refused_by_name() {
    refused("sealed_fail", &["claim `confined` violated", "LeakyCreds"]);
}

// ---- GH #529 D2: the source-editing Attempt is confined by construction

#[test]
fn editor_confined_to_its_worktree_holds() {
    passes("editor_confined_pass");
}

#[test]
fn editor_holding_a_repository_is_refused_with_a_witness() {
    refused(
        "editor_confined_fail",
        &[
            "claim `editors_never_commit` violated",
            "Committer::go",
            "dna::LocalGit::commit_all",
        ],
    );
}


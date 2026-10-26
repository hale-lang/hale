//! Compiler-produced reports bind both finite certificates and Lean statements
//! to the checked seed. Export preserves the ordinary check's failure status.
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[allow(dead_code)]
#[path = "../../hale-codegen/tests/support/harness.rs"]
mod harness;

const SOURCE: &str = r#"
fn sink(v: Int) -> Int { return v; }
fn safe(v: Int) -> Int { return v + 1; }
locus Worker { fn calculate(v: Int) -> Int { return safe(v); } }
group workers = { Worker };
group sinks = { sink };
main locus App { claims { isolated: forbid reaches(workers, sinks); } }
fn main() { App { }; println(Worker { }.calculate(1)); sink(1); }
"#;

fn seed(source: &str) -> PathBuf {
    let path = harness::unique_bin("proof_seed").with_extension("hl");
    std::fs::write(&path, source).unwrap();
    path
}

fn check(source: &Path, args: &[String]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("check")
        .arg(source)
        .args(args)
        .output()
        .unwrap()
}

fn assert_code(output: &Output, code: i32) {
    assert_eq!(
        output.status.code(),
        Some(code),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn source_bound_report_and_lean_document_replay_and_reject_tampering() {
    let source = seed(SOURCE);
    let report = harness::unique_bin("proof_report");
    let lean = harness::unique_bin("proof_lean");
    // Required paths work before the target and with both operand spellings.
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["check", "--dump-reachability-proofs"])
        .arg(&report)
        .arg(&source)
        .arg(format!("--dump-reachability-lean={}", lean.display()))
        .output()
        .unwrap();
    assert_code(&out, 0);
    let json = std::fs::read_to_string(&report).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["scope"], "selected_forbid_reaches_only");
    assert_eq!(value["rows"][0]["proof"]["certificate"]["kind"], "closed");
    assert_eq!(value["all_selected_forbid_laws_have_certificates"], true);
    let lean_source = std::fs::read_to_string(&lean).unwrap();
    assert!(lean_source.contains("theorem absence"));
    assert_code(
        &check(
            &source,
            &[
                format!("--check-reachability-proofs={}", report.display()),
                format!("--check-reachability-lean={}", lean.display()),
            ],
        ),
        0,
    );
    std::fs::write(
        &report,
        json.replace("\"targets\":", "\"targets\": [], \"targets\":"),
    )
    .unwrap();
    assert_code(
        &check(
            &source,
            &[format!("--check-reachability-proofs={}", report.display())],
        ),
        1,
    );
    std::fs::write(&report, &json).unwrap();
    std::fs::write(
        &lean,
        lean_source.replace("theorem absence", "theorem fabricated"),
    )
    .unwrap();
    assert_code(
        &check(
            &source,
            &[format!("--check-reachability-lean={}", lean.display())],
        ),
        1,
    );
    // Even a source comment changes source identity while leaving topology intact.
    std::fs::write(&source, format!("{SOURCE}\n// changed source\n")).unwrap();
    assert_code(
        &check(
            &source,
            &[format!("--check-reachability-proofs={}", report.display())],
        ),
        1,
    );
}

#[test]
fn concrete_violation_exports_a_path_and_retains_failure_status() {
    let source = seed(&SOURCE.replace("return safe(v)", "return sink(v)"));
    let report = harness::unique_bin("violation_report");
    let lean = harness::unique_bin("violation_lean");
    assert_code(
        &check(
            &source,
            &[
                format!("--dump-reachability-proofs={}", report.display()),
                format!("--dump-reachability-lean={}", lean.display()),
            ],
        ),
        1,
    );
    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&report).unwrap()).unwrap();
    assert_eq!(value["rows"][0]["verdict"], "violated");
    assert_eq!(
        value["rows"][0]["diagnostics"][0]["location"]["space"],
        "bundle"
    );
    assert_eq!(
        value["rows"][0]["diagnostics"][0]["location"]["span"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(value["rows"][0]["proof"]["certificate"]["kind"], "path");
    assert!(std::fs::read_to_string(&lean)
        .unwrap()
        .contains("theorem violation"));
    assert_code(
        &check(
            &source,
            &[format!("--check-reachability-proofs={}", report.display())],
        ),
        1,
    );
}

#[test]
fn invalid_claims_are_reported_and_cannot_emit_a_lean_theorem() {
    let source = seed(&SOURCE.replace("reaches(workers, sinks)", "reaches(missing, sinks)"));
    let report = harness::unique_bin("invalid_report");
    let lean = harness::unique_bin("invalid_lean");
    assert_code(
        &check(
            &source,
            &[format!("--dump-reachability-proofs={}", report.display())],
        ),
        1,
    );
    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&report).unwrap()).unwrap();
    assert_eq!(value["all_selected_forbid_laws_have_certificates"], false);
    assert_eq!(value["rows"][0]["verdict"], "invalid");
    assert!(value["rows"][0]["proof"].is_null());
    assert_code(
        &check(
            &source,
            &[format!("--dump-reachability-lean={}", lean.display())],
        ),
        1,
    );
    assert!(!lean.exists());
    assert_code(
        &check(
            &source,
            &[format!("--check-reachability-proofs={}", report.display())],
        ),
        1,
    );
}

#[test]
fn proof_flags_need_paths_refuse_source_replacement_and_illtyped_models() {
    let source = seed(SOURCE);
    for flag in [
        "--dump-reachability-proofs",
        "--dump-reachability-lean",
        "--check-reachability-proofs",
        "--check-reachability-lean",
    ] {
        assert_code(&check(&source, &[flag.into()]), 2);
        assert_code(&check(&source, &[format!("{flag}=")]), 2);
    }
    assert_code(
        &check(
            &source,
            &[format!("--dump-reachability-proofs={}", source.display())],
        ),
        2,
    );
    assert_eq!(std::fs::read_to_string(&source).unwrap(), SOURCE);
    let linked = harness::unique_bin("linked_proof_destination");
    std::fs::hard_link(&source, &linked).unwrap();
    assert_code(
        &check(
            &source,
            &[format!("--dump-reachability-proofs={}", linked.display())],
        ),
        0,
    );
    assert_eq!(std::fs::read_to_string(&source).unwrap(), SOURCE);
    let report: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&linked).unwrap()).unwrap();
    assert_eq!(report["scope"], "selected_forbid_reaches_only");
    let collision = harness::unique_bin("same_proof_destination");
    assert_code(
        &check(
            &source,
            &[
                format!("--dump-reachability-proofs={}", collision.display()),
                format!("--dump-reachability-lean={}", collision.display()),
            ],
        ),
        2,
    );
    assert!(!collision.exists());
    std::fs::write(
        &source,
        SOURCE.replace("return safe(v)", "return unbound(v)"),
    )
    .unwrap();
    let report = harness::unique_bin("illtyped_report");
    assert_code(
        &check(
            &source,
            &[format!("--dump-reachability-proofs={}", report.display())],
        ),
        1,
    );
    assert!(!report.exists());
}

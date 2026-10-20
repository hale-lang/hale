//! GH #1477 — `hale deploy render`: a program's placement as compose and
//! Kubernetes, pinned against goldens (`tests/fixtures/deploy/goldens`).
//! A pure function of the checked program: the same bytes every run, and
//! nothing rendered for what the program does not declare.

use std::path::{Path, PathBuf};
use std::process::Command;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/deploy")
}

fn voice_heart() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dna/tests/onboarding/voice-heart")
}

fn render(cwd: &Path, args: &[&str]) -> (bool, String, String) {
    let o = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("deploy")
        .arg("render")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("hale");
    (
        o.status.success(),
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    )
}

fn golden(name: &str) -> String {
    std::fs::read_to_string(fixtures().join("goldens").join(name)).expect("golden")
}

#[test]
fn a_seed_with_probes_and_scrape_renders_compose() {
    let (ok, out, err) = render(&fixtures(), &["svc", "--backend", "compose"]);
    assert!(ok, "{err}");
    assert_eq!(out, golden("svc.compose.yaml"));
    assert!(out.contains("/healthz") && out.contains("127.0.0.1:9100:9100"));
    // a unix binding is a path, not a port: said, not published
    assert!(err.contains("local paths, not ports"), "{err}");
}

#[test]
fn a_seed_with_probes_and_scrape_renders_kubernetes() {
    let (ok, out, err) = render(&fixtures(), &["svc", "--backend", "kubernetes"]);
    assert!(ok, "{err}");
    assert_eq!(out, golden("svc.kubernetes.yaml"));
    assert!(
        out.contains("livenessProbe")
            && out.contains("readinessProbe")
            && out.contains("prometheus.io/scrape")
    );
    assert!(
        !out.contains("HorizontalPodAutoscaler\n"),
        "no scale policy is declared, so none is rendered"
    );
}

#[test]
fn a_program_without_probes_renders_no_probe_section() {
    for backend in ["compose", "kubernetes"] {
        let (ok, out, err) = render(&fixtures(), &["bare", "--backend", backend]);
        assert!(ok, "{err}");
        for banned in [
            "healthcheck",
            "livenessProbe",
            "readinessProbe",
            "/healthz",
            "/readyz",
            "prometheus",
            "ports:",
        ] {
            assert!(
                !out.contains(banned),
                "{backend}: `{banned}` invented:\n{out}"
            );
        }
        assert!(err.contains("holds no std::observe::Probes"), "{err}");
    }
}

#[test]
fn a_fleet_plan_renders_its_instances_routes_and_environment() {
    let (ok, out, err) = render(&fixtures(), &["fleet.plan.json", "--env", "prod"]);
    assert!(ok, "{err}");
    assert_eq!(out, golden("fleet.prod.compose.yaml"));
    // the subscriber waits for its publishers, which have a healthcheck to wait on
    assert!(out.contains("condition: service_healthy"));
    let (ok, out, err) = render(&fixtures(), &["fleet.plan.json", "--env", "staging"]);
    assert!(ok, "{err}");
    assert_eq!(out, golden("fleet.staging.kubernetes.yaml"));
    assert!(out.contains("replicas: 2"));
}

#[test]
fn the_output_is_the_same_every_time_and_out_writes_a_file() {
    let a = render(&fixtures(), &["fleet.plan.json", "--env", "prod"]).1;
    let b = render(&fixtures(), &["fleet.plan.json", "--env", "prod"]).1;
    assert_eq!(a, b);
    let d = std::env::temp_dir().join(format!("hale_deploy_out_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    let (ok, out, err) = render(
        &fixtures(),
        &[
            "svc",
            "--backend",
            "kubernetes",
            "--out",
            &d.to_string_lossy(),
        ],
    );
    assert!(ok && out.is_empty(), "{err}");
    assert_eq!(
        std::fs::read_to_string(d.join("kubernetes.yaml")).unwrap(),
        golden("svc.kubernetes.yaml")
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn voices_api_program_renders_its_compose() {
    // voice-heart is three programs (api, brain, node) under one manifest;
    // its api program is the one rendered
    let (ok, out, err) = render(&voice_heart(), &["api", "--backend", "compose"]);
    assert!(ok, "{err}");
    assert_eq!(out, golden("voice-api.compose.yaml"));
    // the repository itself is no program, and is not rendered
    let (ok, _, err) = render(&voice_heart(), &[".", "--backend", "compose"]);
    assert!(!ok && err.contains("no .hl source here"), "{err}");
}

#[test]
fn the_backend_is_required_and_known() {
    let (ok, _, err) = render(&fixtures(), &["svc"]);
    assert!(!ok && err.contains("no backend"), "{err}");
    let (ok, _, err) = render(&fixtures(), &["svc", "--backend", "nomad"]);
    assert!(!ok && err.contains("unknown backend"), "{err}");
}

#[test]
fn a_relative_mount_is_a_bind_not_a_named_volume() {
    let (_, out, _) = render(&fixtures(), &["svc", "--backend", "compose"]);
    assert!(out.contains("\"./svc:/app:ro\""), "{out}");
}

#[test]
fn the_selected_environment_gates_the_seed_check() {
    // the seed holds without an environment and violates `strict`'s constitution
    let (ok, _, err) = render(&fixtures(), &["served", "--backend", "compose"]);
    assert!(ok, "{err}");
    let (ok, _, err) = render(&fixtures(), &["served", "--env", "strict"]);
    assert!(!ok && err.contains("logs_structured"), "{err}");
}

#[test]
fn instances_of_one_seed_keep_their_own_binaries() {
    let (ok, out, err) = render(&fixtures(), &["env.plan.json", "--backend", "compose"]);
    assert!(ok, "{err}");
    assert!(
        out.contains("[\"/app/out/a\"]") && out.contains("[\"/app/out/b\"]"),
        "{out}"
    );
    let (ok, _, err) = render(&fixtures(), &["env.plan.json", "--backend", "kubernetes"]);
    assert!(
        !ok && err.contains("`svc-0` and `svc-1`") && err.contains("different binaries"),
        "{err}"
    );
}

#[test]
fn names_that_normalize_alike_are_refused_with_both_ids() {
    let (ok, _, err) = render(&fixtures(), &["clash.plan.json", "--backend", "kubernetes"]);
    assert!(
        !ok && err.contains("`worker_a`") && err.contains("`worker-a`"),
        "{err}"
    );
}

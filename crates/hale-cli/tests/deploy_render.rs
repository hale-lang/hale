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

/// The fixtures copied out of the tree: a fleet's render cuts each
/// instance's artifact beside the fleet seed.
fn workspace(tag: &str) -> PathBuf {
    fn copy(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for e in std::fs::read_dir(from).unwrap() {
            let p = e.unwrap().path();
            let dest = to.join(p.file_name().unwrap());
            if p.is_dir() {
                copy(&p, &dest);
            } else {
                std::fs::copy(&p, &dest).unwrap();
            }
        }
    }
    let d = std::env::temp_dir().join(format!("hale_deploy_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    copy(&fixtures(), &d);
    d
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
fn a_fleet_seed_renders_its_instances_routes_and_environment() {
    let d = workspace("fleet");
    let (ok, out, err) = render(&d, &["demo", "--env", "prod"]);
    assert!(ok, "{err}");
    assert_eq!(out, golden("fleet.prod.compose.yaml"));
    // the subscriber waits for its publishers, which have a healthcheck to wait on
    assert!(out.contains("condition: service_healthy"));
    // no host in the source and none allocated: one project, said so
    assert!(err.contains("rendered for the host `local`"), "{err}");
    let (ok, out, err) = render(&d, &["demo", "--env", "staging"]);
    assert!(ok, "{err}");
    assert_eq!(out, golden("fleet.staging.kubernetes.yaml"));
    assert!(out.contains("replicas: 2"));
    let _ = std::fs::remove_dir_all(&d);
}

/// The habitat's allocation rows place each instance on a host; the
/// render is one compose project per host, and a route between hosts is
/// declared in both, by `instance@host` endpoints, never wired by
/// compose.
#[test]
fn a_two_host_fleet_renders_a_project_per_host() {
    let d = workspace("hosts");
    std::fs::write(
        d.join("alloc.json"),
        r#"[{"instance": "bare-0", "host": "box-a", "provider": "compose", "role": "instance"},
            {"instance": "svc-0", "host": "box-a"},
            {"instance": "svc-1", "host": "box-b"}]"#,
    )
    .unwrap();
    let (ok, _, err) = render(&d, &["demo", "--backend", "compose", "--placement", "alloc.json", "--out", "out"]);
    assert!(ok, "{err}");
    let a = std::fs::read_to_string(d.join("out/box-a/compose.yaml")).expect("box-a's project");
    let b = std::fs::read_to_string(d.join("out/box-b/compose.yaml")).expect("box-b's project");
    assert!(a.contains("name: \"box-a\"") && b.contains("name: \"box-b\""), "{a}\n{b}");
    assert!(a.contains("  bare-0:") && a.contains("  svc-0:") && !a.contains("  svc-1:"), "{a}");
    assert!(b.contains("  svc-1:") && !b.contains("  bare-0:"), "{b}");
    // in-host order is a dependency; the cross-host publisher is not
    assert!(a.contains("depends_on:\n      svc-0:"), "{a}");
    assert!(!a.contains("      svc-1:\n        condition"), "{a}");
    for project in [&a, &b] {
        assert!(
            project.contains("x-hale-routes:\n  \"s::Tick\":")
                && project.contains("publishers: [\"svc-0@box-a\", \"svc-1@box-b\"]")
                && project.contains("subscribers: [\"bare-0@box-a\"]"),
            "the route is declared in each project it touches:\n{project}"
        );
    }
    // the source's default and the allocation agree on one host: one project
    std::fs::write(d.join("one.json"), r#"[{"instance": "svc-1", "host": "local"}]"#).unwrap();
    let (ok, out, err) = render(&d, &["demo", "--backend", "kubernetes", "--placement", "one.json"]);
    assert!(ok && out.matches("# host: ").count() == 1, "{err}{out}");
    // an allocation of nothing the fleet deploys is refused
    std::fs::write(d.join("ghost.json"), r#"[{"instance": "ghost", "host": "box-a"}]"#).unwrap();
    let (ok, _, err) = render(&d, &["demo", "--backend", "compose", "--placement", "ghost.json"]);
    assert!(!ok && err.contains("allocates `ghost`, which fleet `Demo` does not deploy"), "{err}");
    // a host is one name, never a path: nothing renders outside --out
    for bad in ["../escaped", "/abs", ".hidden", "a/b"] {
        std::fs::write(d.join("bad.json"), format!(r#"[{{"instance": "svc-1", "host": "{bad}"}}]"#)).unwrap();
        let (ok, _, err) = render(&d, &["demo", "--backend", "compose", "--placement", "bad.json", "--out", "intended"]);
        assert!(!ok && err.contains("names no host"), "{bad}: {err}");
    }
    assert!(!d.join("escaped").exists() && !d.join("intended").exists(), "nothing was written");
    // nor in the source's placement
    let src = std::fs::read_to_string(d.join("demo/fleet.hl")).unwrap();
    std::fs::create_dir_all(d.join("pathy")).unwrap();
    std::fs::write(d.join("pathy/fleet.hl"), src.replace("svc: replicas(2);", "svc: host(\"../escaped\");")).unwrap();
    let (ok, _, err) = render(&d, &["pathy", "--backend", "compose", "--out", "intended"]);
    assert!(!ok && err.contains("names no host"), "{err}");
    assert!(!d.join("escaped").exists(), "nothing was written");
    // a placement is a fleet's
    let (ok, _, err) = render(&d, &["svc", "--backend", "compose", "--placement", "one.json"]);
    assert!(!ok && err.contains("no fleet seed"), "{err}");
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
fn names_that_normalize_alike_are_refused_with_both_ids() {
    let d = workspace("clash");
    std::fs::create_dir_all(d.join("clash")).unwrap();
    std::fs::write(
        d.join("clash/fleet.hl"),
        "import \"../svc\" as s;\nimport \"../bare\" as b;\n\nfleet locus Clash {\n    params {\n        worker: s::App;\n        Worker: b::App;\n    }\n}\n",
    )
    .unwrap();
    let (ok, _, err) = render(&d, &["clash", "--backend", "kubernetes"]);
    let _ = std::fs::remove_dir_all(&d);
    assert!(!ok && err.contains("`worker-0`") && err.contains("`Worker-0`"), "{err}");
}

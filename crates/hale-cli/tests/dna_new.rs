//! GH #528 — `hale dna new <name>` (Track C, PR 24): a greenfield
//! application with its DNA. The issue's acceptance: it passes
//! `hale check --matrix`, builds, runs, and its tests pass; the
//! app-wide law is ACTIVE because a fresh app has no holes.

use std::path::PathBuf;
use std::process::Command;

fn hale(args: &[&str], cwd: &std::path::Path) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale")).args(args).current_dir(cwd).output().expect("hale");
    (
        out.status.success(),
        format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)),
    )
}

fn workdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("hale_dna_new_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn new_makes_a_governed_application_that_checks_builds_runs_and_tests() {
    let d = workdir("demo");
    let (ok, out) = hale(&["dna", "new", "demo-app"], &d);
    assert!(ok, "{out}");
    let app = d.join("demo-app");
    for f in ["main.hl", "tests/main_test.hl", "hale.toml", "hale.lock", ".gitignore", "dna/org/main.hl", "dna/org/law.hl", "dna/org/purpose.hl", "vendor/dna/topics.hl", ".hale/dna/baseline.topology"] {
        assert!(app.join(f).exists(), "{f}: {out}");
    }
    let main = std::fs::read_to_string(app.join("main.hl")).unwrap();
    assert!(main.contains("main locus DemoApp"), "{main}");
    // the application carries no DNA (GH #566 F2); the organization's law is its own
    assert!(!main.contains("dna::") && !main.contains("genome"), "the application is not grafted: {main}");
    assert!(!app.join("dna_constitution.hl").exists() && !app.join("dna/assembly.hl").exists(), "no law or assembly in the application");
    let law = std::fs::read_to_string(app.join("dna/org/law.hl")).unwrap();
    assert!(law.contains("apply_only_through_the_substrate: forbid reaches(positions, effects(genome_apply)) avoiding substrate;"), "{law}");
    assert!(law.contains("editors_never_commit: forbid reaches(editors, effects(repo_write));"), "{law}");
    assert!(law.contains("editors_never_learn: forbid reaches(editors, knowledge);"), "{law}");
    let org = std::fs::read_to_string(app.join("dna/org/main.hl")).unwrap();
    assert!(org.contains("leader: dna::Leader") && org.contains("membrane: dna::Board") && org.contains("adopt Org;"), "{org}");
    let manifest = std::fs::read_to_string(app.join("hale.toml")).unwrap();
    assert!(manifest.contains("no_base = true") && manifest.contains("[environments.org]") && manifest.contains("entrypoints = [\"dna/org\"]"), "{manifest}");
    let (ok, out) = hale(&["check", "--matrix", "."], &app);
    assert!(ok, "matrix: {out}");
    let (ok, out) = hale(&["build", "."], &app);
    assert!(ok, "build: {out}");
    let run = Command::new(app.join("demo-app")).current_dir(&app).env("HALE_DNA_ONESHOT", "1").output().unwrap();
    assert!(run.status.success(), "run: {}", String::from_utf8_lossy(&run.stderr));
    assert!(String::from_utf8_lossy(&run.stdout).contains("1 ping(s) echoed"));
    let (ok, out) = hale(&["test", "."], &app);
    assert!(ok, "test: {out}");
    // a second `new` into the same directory is refused
    let (ok, out) = hale(&["dna", "new", "demo-app"], &d);
    assert!(!ok && out.contains("not empty"), "{out}");
    let _ = std::fs::remove_dir_all(&d);
}

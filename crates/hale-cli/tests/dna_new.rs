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
    for f in ["main.hl", "tests/main_test.hl", "hale.toml", "hale.lock", ".gitignore", "dna/assembly.hl", "dna_constitution.hl", "dna/purpose.hl", "vendor/dna/topics.hl", ".hale/dna/journal.jsonl"] {
        assert!(app.join(f).exists(), "{f}: {out}");
    }
    let main = std::fs::read_to_string(app.join("main.hl")).unwrap();
    assert!(main.contains("main locus DemoApp"), "{main}");
    let law = std::fs::read_to_string(app.join("dna_constitution.hl")).unwrap();
    assert!(law.contains("\n    organism_gated: forbid reaches(organism, effects(genome_apply)) avoiding dna_gate;"), "a fresh app has no holes, so the app-wide clause is active: {law}");
    let (ok, out) = hale(&["check", "--matrix", "."], &app);
    assert!(ok, "matrix: {out}");
    let (ok, out) = hale(&["build", "."], &app);
    assert!(ok, "build: {out}");
    let run = Command::new(app.join("demo-app")).current_dir(&app).output().unwrap();
    assert!(run.status.success(), "run: {}", String::from_utf8_lossy(&run.stderr));
    assert!(String::from_utf8_lossy(&run.stdout).contains("1 ping(s) echoed"));
    let (ok, out) = hale(&["test", "."], &app);
    assert!(ok, "test: {out}");
    // a second `new` into the same directory is refused
    let (ok, out) = hale(&["dna", "new", "demo-app"], &d);
    assert!(!ok && out.contains("not empty"), "{out}");
    let _ = std::fs::remove_dir_all(&d);
}

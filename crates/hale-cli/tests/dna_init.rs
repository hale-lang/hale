//! GH #528 — `hale dna init` / `upgrade` (Track C, PR 23).
//!
//! What these hold the command to (the issue's acceptance for the
//! existing-application path):
//!   1. The application's locus graph is untouched and it still
//!      passes its previous checks — plus `hale check --matrix` for
//!      the environment init declares, and it still builds and runs.
//!   2. `vendor/dna` is toolchain-owned and pinned in hale.lock;
//!      `dna/` is project-owned; `upgrade` re-materializes the former
//!      without touching the latter.
//!   3. The Journal is seeded from the compiler's model with
//!      `observed` and `inferred` provenance kept distinct, and the
//!      baseline review is requested before any work runs.
//!   4. Re-running is non-destructive: every file is kept.
//!   5. A law the application cannot certify (it has an unresolvable
//!      edge) is deferred with its reason, never silently dropped
//!      and never made to fail the application.

use std::path::{Path, PathBuf};
use std::process::Command;

fn hale(args: &[&str], cwd: &Path) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale")).args(args).current_dir(cwd).output().expect("hale");
    (
        out.status.success(),
        format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)),
    )
}

fn workdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("hale_dna_init_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// The pinned renderer fixture as an existing application: two
/// loci, two topics, a group, two claims, and one indirect call.
fn pipeline_app(tag: &str) -> PathBuf {
    let d = workdir(tag).join("pipeline");
    std::fs::create_dir_all(&d).unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/topology/pipeline.hl");
    std::fs::copy(fixture, d.join("main.hl")).unwrap();
    std::fs::write(d.join("hale.toml"), "[deps]\n").unwrap();
    d
}

fn journal_rows(root: &Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(root.join(".hale/dna/journal.jsonl"))
        .unwrap()
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| serde_json::from_str(l).expect("journal line is JSON"))
        .collect()
}

#[test]
fn init_attaches_the_dna_and_the_application_still_checks_builds_and_runs() {
    let app = pipeline_app("attach");
    let before = std::fs::read_to_string(app.join("main.hl")).unwrap();
    let (ok, out) = hale(&["dna", "init", "."], &app);
    assert!(ok, "{out}");
    for f in [
        "vendor/dna/assembly.hl",
        "vendor/dna/topics.hl",
        "dna/assembly.hl",
        "dna_constitution.hl",
        "dna/purpose.hl",
        ".hale/dna/journal.jsonl",
        ".hale/dna/baseline.topology",
        "hale.lock",
    ] {
        assert!(app.join(f).exists(), "init creates {f}: {out}");
    }
    let lock = std::fs::read_to_string(app.join("hale.lock")).unwrap();
    assert!(lock.contains("[dna]") && lock.contains("toolchain = "), "hale.lock pins the toolchain: {lock}");
    let manifest = std::fs::read_to_string(app.join("hale.toml")).unwrap();
    assert!(manifest.contains("base = \"Project\"") && manifest.contains("[environments.local]"), "{manifest}");

    // The locus graph is untouched: every original line survives.
    let after = std::fs::read_to_string(app.join("main.hl")).unwrap();
    for line in before.lines().filter(|l| !l.trim().is_empty()) {
        // the one-line params block gains the genome param in place
        let probe = line.trim().trim_end_matches('}').trim_end();
        assert!(after.contains(probe), "original line kept: {line}");
    }
    assert!(after.contains("genome: genome::Genome = genome::Genome { }"), "{after}");
    assert!(after.contains("adopt Project;"), "{after}");
    assert!(after.contains("dna::ReviewVerdict: unix(\".hale/dna/review.verdict.sock\", role: listen)"), "{after}");
    let law = std::fs::read_to_string(app.join("dna_constitution.hl")).unwrap();
    assert!(law.contains("group organism = { App };"), "{law}");

    // …and it still passes its previous checks, plus the matrix, and builds.
    let (ok, out) = hale(&["check", "."], &app);
    assert!(ok, "check: {out}");
    let (ok, out) = hale(&["check", "--matrix", "."], &app);
    assert!(ok, "matrix: {out}");
    let (ok, out) = hale(&["build", "."], &app);
    assert!(ok, "build: {out}");
    let run = Command::new(app.join("pipeline")).current_dir(&app).output().unwrap();
    assert!(run.status.success(), "run: {}", String::from_utf8_lossy(&run.stderr));

    // Re-running keeps everything.
    let (ok, out2) = hale(&["dna", "init", "."], &app);
    assert!(ok, "{out2}");
    assert!(out2.contains("kept") && !out2.contains("created "), "second init is non-destructive:\n{out2}");
    assert_eq!(after, std::fs::read_to_string(app.join("main.hl")).unwrap(), "main.hl untouched on re-run");
    let _ = std::fs::remove_dir_all(app.parent().unwrap());
}

#[test]
fn the_journal_is_seeded_from_the_model_with_provenance_kept_distinct() {
    let app = pipeline_app("journal");
    let (ok, out) = hale(&["dna", "init", "."], &app);
    assert!(ok, "{out}");
    let rows = journal_rows(&app);
    let kinds: Vec<&str> = rows.iter().map(|r| r["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds[0], "application.attached");
    assert_eq!(kinds.last().copied(), Some("review.requested"), "the baseline review is requested last: {kinds:?}");
    let body = |r: &serde_json::Value| -> serde_json::Value { serde_json::from_str(r["body"].as_str().unwrap()).unwrap() };
    // observed structure: every locus, topic and claim of the fixture
    let observed: Vec<String> = rows
        .iter()
        .filter(|r| r["kind"] == "structure.observed")
        .map(|r| r["entity"].as_str().unwrap().to_string())
        .collect();
    for e in ["locus:App", "locus:Worker", "locus:Store", "topic:Readings", "topic:Cmds", "claim:apart", "claim:one_writer"] {
        assert!(observed.iter().any(|x| x == e), "{e} observed: {observed:?}");
    }
    for r in rows.iter().filter(|r| r["kind"] == "structure.observed") {
        assert_eq!(body(r)["provenance"], "observed");
    }
    // inferred responsibilities: guesses, never ratified
    let proposed: Vec<&serde_json::Value> = rows.iter().filter(|r| r["kind"] == "responsibility.proposed").collect();
    assert_eq!(proposed.len(), 3, "one guess per locus");
    for r in &proposed {
        let b = body(r);
        assert_eq!(b["provenance"], "inferred");
        assert_eq!(b["ratified"], false);
    }
    let worker = proposed.iter().find(|r| r["entity"] == "locus:Worker").unwrap();
    let guess = body(worker)["responsibility"].as_str().unwrap().to_string();
    assert!(guess.contains("Readings") && guess.contains("Cmds"), "the guess is from structure: {guess}");
    // the baseline review names the purpose digest and the artifact
    let review = body(rows.last().unwrap());
    assert_eq!(review["provenance"], "declared");
    assert!(review["subject_digest"].as_str().unwrap().starts_with("sha256:"));
    let purpose = std::fs::read_to_string(app.join("dna/assembly.hl")).unwrap();
    assert!(purpose.contains(review["subject_digest"].as_str().unwrap()), "the Review in the assembly pins the same digest");
    // the chain is intact
    for (i, r) in rows.iter().enumerate() {
        assert_eq!(r["seq"].as_u64().unwrap() as usize, i);
        if i > 0 {
            assert_eq!(r["prev"], rows[i - 1]["digest"], "row {i} chains to its predecessor");
        }
    }
    let _ = std::fs::remove_dir_all(app.parent().unwrap());
}

#[test]
fn a_law_the_application_cannot_certify_is_deferred_with_its_reason() {
    let app = pipeline_app("deferred");
    let (ok, out) = hale(&["dna", "init", "."], &app);
    assert!(ok, "{out}");
    assert!(out.contains("`organism_gated` is deferred") && out.contains("call_it"), "{out}");
    let law = std::fs::read_to_string(app.join("dna_constitution.hl")).unwrap();
    assert!(law.contains("// organism_gated: forbid reaches(organism, effects(genome_apply)) avoiding dna_gate;"), "{law}");
    assert!(law.contains("apply_gated: forbid reaches(genome, effects(genome_apply)) avoiding dna_gate;"), "the assembly-scoped clause stays active: {law}");
    let deferred: Vec<serde_json::Value> = journal_rows(&app).into_iter().filter(|r| r["kind"] == "law.deferred").collect();
    assert_eq!(deferred.len(), 1, "one deferred clause, journaled");
    let b: serde_json::Value = serde_json::from_str(deferred[0]["body"].as_str().unwrap()).unwrap();
    assert_eq!(b["provenance"], "inferred");
    assert!(b["reason"].as_str().unwrap().contains("call_it"));
    let _ = std::fs::remove_dir_all(app.parent().unwrap());
}

#[test]
fn upgrade_rematerializes_vendor_without_touching_the_project_seed() {
    let app = pipeline_app("upgrade");
    let (ok, out) = hale(&["dna", "init", "."], &app);
    assert!(ok, "{out}");
    let assembly = app.join("dna/assembly.hl");
    std::fs::write(&assembly, "// mine\n").unwrap();
    let vendored = app.join("vendor/dna/topics.hl");
    std::fs::write(&vendored, "// tampered\n").unwrap();
    std::fs::remove_file(app.join("vendor/dna/review.hl")).unwrap();
    let (ok, out) = hale(&["dna", "upgrade", "."], &app);
    assert!(ok, "{out}");
    assert!(out.contains("2 file(s) rewritten"), "{out}");
    assert!(std::fs::read_to_string(&vendored).unwrap().contains("topic ReviewVerdict"), "vendor restored");
    assert!(app.join("vendor/dna/review.hl").exists());
    assert_eq!(std::fs::read_to_string(&assembly).unwrap(), "// mine\n", "dna/ is the project's");
    let _ = std::fs::remove_dir_all(app.parent().unwrap());
}

#[test]
fn init_refuses_a_seed_without_a_main_locus() {
    let d = workdir("nomain");
    std::fs::write(d.join("main.hl"), "fn main() { println(\"hi\"); }\n").unwrap();
    std::fs::write(d.join("hale.toml"), "[deps]\n").unwrap();
    let (ok, out) = hale(&["dna", "init", "."], &d);
    assert!(!ok);
    assert!(out.contains("declares no `main locus`"), "{out}");
    let _ = std::fs::remove_dir_all(&d);
}

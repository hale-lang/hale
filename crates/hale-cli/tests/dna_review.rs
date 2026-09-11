//! GH #529 D4 — the blocking human Review over a real Mutation, from
//! the CLI. A governed application under git; a driver program (the
//! same `dna::Dna` the Genome assembles, with scripted models) runs
//! one Mutation up to its Review; `hale dna review` renders the source
//! diff, the semantic diff and the evidence table OFFLINE from the
//! Journal and the receipts; the organism, started afterwards, re-births
//! the pending Review from the Journal; a verdict naming another
//! digest is refused by the Review; the maintainer's verdict on the
//! exact candidate settles it.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn hale(args: &[&str], cwd: &Path) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(args)
        .current_dir(cwd)
        .env("HALE_BIN", env!("CARGO_BIN_EXE_hale"))
        .env("XDG_CACHE_HOME", std::env::temp_dir().join("hale-tests-iris-cache"))
        .output()
        .expect("hale");
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn git(args: &[&str], cwd: &Path) {
    let st = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@local"])
        .args(args)
        .current_dir(cwd)
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .status()
        .expect("git");
    assert!(st.success(), "git {args:?}");
}

const DRIVER: &str = r#"// the Genome's own assembly, with scripted models, driving ONE mutation
import "vendor/dna" as dna;

fn main() {
    let cur = std::io::fs::read_file("main.hl") or "";
    let core = dna::Dna {
        journal: dna::FileJournal { path: ".hale/dna/journal.jsonl" },
        gateway: dna::MutationGateway {
            workspaces: dna::IsolatedWorktrees { repo: ".", root: ".hale/dna/worktrees" },
            repo: dna::LocalGit { repo: "." }
        },
        verification: dna::HaleVerification { evidence_dir: ".hale/dna/evidence", repo: ".", seed: "." },
        editor: dna::SourceEditor {
            name: "editor",
            models: dna::ModelRouter {
                quick: dna::FakeModel { name: "quick", answer: cur + "// tuned: the entrypoint is documented here\n" },
                deep: dna::FakeModel { name: "deep", answer: "docs_coverage +" }
            }
        },
        boundary: dna::AutonomyBoundary {
            child: "orgrev",
            grant: dna::Grant { child: "orgrev", classes: "docs refactor", max_magnitude: 4, review: "pre" }
        }
    };
    println(core.mutate("t0", "docs", "document the entrypoint", "main.hl", 1));
}
"#;

#[test]
fn a_mutation_is_rendered_offline_and_decided_through_the_organism() {
    let d = std::env::temp_dir().join(format!("hale_dna_review_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let (ok, out) = hale(&["dna", "new", "orgrev"], &d);
    assert!(ok, "{out}");
    let app: PathBuf = d.join("orgrev");
    git(&["init", "-q", "-b", "main"], &app);
    git(&["add", "-A"], &app);
    git(&["commit", "-q", "-m", "genome"], &app);

    // 1. one Mutation, up to its Review, by the assembly itself
    std::fs::create_dir_all(app.join("mutate")).unwrap();
    std::fs::write(app.join("mutate/main.hl"), DRIVER).unwrap();
    let (ok, out) = hale(&["run", "mutate"], &app);
    assert!(ok && out.contains("m1: review"), "driver:\n{out}");

    // 2. rendered offline: the pending list, then the Review itself
    let (ok, list) = hale(&["dna", "review"], &app);
    assert!(ok && list.contains("2 pending review(s) of 2") && list.contains("m1 needs maintainer") && list.contains("docs · candidate"), "{list}");
    let (ok, view) = hale(&["dna", "review", "m1"], &app);
    assert!(ok, "{view}");
    for needle in [
        "review m1 [pending]: apply m1 (docs): document the entrypoint?",
        "disposition under the grant: stage",
        "source diff (git",
        "+// tuned: the entrypoint is documented here",
        "semantic diff (hale model diff",
        "classification: source-only",
        "evidence (fmt=0 check=0 verify=0 test=0 diff=0):",
        "base       yes    0",
        "check      yes    0",
        "test       yes    0",
        "decide: hale dna review m1 approve|revise|reject|abstain",
    ] {
        assert!(view.contains(needle), "missing {needle:?} in:\n{view}");
    }
    let (ok, st) = hale(&["dna", "status"], &app);
    assert!(ok && st.contains("mutations:  1") && st.contains("m1 [stage] docs: document the entrypoint"), "{st}");

    // 3. the organism, started AFTER the request, re-births the Review
    let cache = std::env::temp_dir().join("hale-tests-iris-cache");
    let mut host = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["dna", "run", ".", "--no-iris"])
        .current_dir(&app)
        .env("XDG_CACHE_HOME", &cache)
        .env("HALE_BIN", env!("CARGO_BIN_EXE_hale"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("hale dna run");
    let dl = Instant::now() + Duration::from_secs(90);
    while Instant::now() < dl && !(app.join(".hale/dna/hale-dna.intent.offered.sock").exists() && app.join(".hale/dna/hale-dna.review.verdict.sock").exists()) {
        std::thread::sleep(Duration::from_millis(200));
    }
    let finish = |host: &mut std::process::Child| {
        let _ = Command::new("pkill").args(["-x", "orgrev"]).status();
        let _ = host.wait();
    };
    if !app.join(".hale/dna/hale-dna.review.verdict.sock").exists() {
        finish(&mut host);
        panic!("the membrane did not come up");
    }
    std::thread::sleep(Duration::from_millis(500));
    // a verdict naming another digest is refused BY THE REVIEW
    let (ok1, out1) = hale(&["dna", "review", "m1", "approve", "--as", "riley", "--digest", "0000000000000000000000000000000000000000"], &app);
    // the maintainer's verdict on the exact candidate settles it
    let (ok2, out2) = hale(&["dna", "review", "m1", "approve", "--as", "riley", "--comment", "fine"], &app);
    let (ok3, out3) = hale(&["dna", "status"], &app);
    let (ok4, out4) = hale(&["dna", "history", "m1"], &app);
    finish(&mut host);
    assert!(ok1 && out1.contains("refused the verdict: digest mismatch"), "wrong digest:\n{out1}");
    assert!(ok2 && out2.contains("review m1 settled: approve by riley"), "settle:\n{out2}");
    assert!(ok3 && out3.contains("1 pending of 2") && out3.contains("m1 [reviewed]"), "status:\n{out3}");
    assert!(ok4 && out4.contains("evidence.check") && out4.contains("review.requested") && out4.contains("review.settled") && out4.contains("mutation.candidate"), "history:\n{out4}");
    let _ = std::fs::remove_dir_all(&d);
}

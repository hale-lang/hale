//! GH #529 D5 + D6 — approval applies exactly the reviewed candidate,
//! and the host rebuilds, restarts and watches the window. A governed
//! application under git; one Mutation up to its Review (the driver
//! from `dna_review`); `hale dna run --observe 2` holds the organism;
//! the maintainer's verdict makes the organism apply the pinned commit
//! and ask for a restart; the host rebuilds, restarts the organism
//! (which journals `expression.restarted`), watches it stay up for the
//! window, and reports `healthy` on the membrane; the organism retains
//! the Mutation and dissolves its worktree. `git log` gains exactly the
//! candidate; the organism is still up afterwards.

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

fn git(args: &[&str], cwd: &Path) -> String {
    let out = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@local"])
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("git");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

const DRIVER: &str = r#"import "vendor/dna" as dna;

fn main() {
    let cur = std::io::fs::read_file("main.hl") or "";
    let core = dna::Dna {
        journal: dna::GitJournal { repo: "." },
        gateway: dna::MutationGateway {
            leases: dna::GitLeases { repo: "." },
            workspaces: dna::IsolatedWorktrees { repo: ".", root: ".hale/dna/worktrees" },
            repo: dna::LocalGit { repo: "." }
        },
        verification: dna::HaleVerification { receipts: dna::GitReceipts { repo: "." }, scratch: ".hale/dna/scratch", repo: ".", seed: "." },
        editor: dna::SourceEditor {
            name: "editor",
            models: dna::ModelRouter {
                quick: dna::FakeModel { name: "quick", answer: cur + "// tuned: the entrypoint is documented here\n" },
                deep: dna::FakeModel { name: "deep", answer: "docs_coverage +" }
            }
        },
        review_policy: dna::OrgPolicy { },
        membrane: dna::Board { who: "board" },
        boundary: dna::AutonomyBoundary {
            child: "orgapply",
            grant: dna::Grant { child: "orgapply", classes: "docs refactor", max_magnitude: 4, review: "pre" }
        }
    };
    println(core.mutate("t0", "docs", "document the entrypoint", "main.hl", 1));
}
"#;

/// The Journal as (kind, entity, body) rows. Two writers spell the
/// JSON differently (the host's serde, the organism's Builder), so
/// the file is parsed, never string-matched.
fn journal(app: &Path) -> Vec<(String, String, String)> {
    let text = Command::new("git").args(["-C", &app.to_string_lossy(), "show", "refs/dna/journal:journal.jsonl"]).output().map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
    text.lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .map(|v| {
            let s = |k: &str| v[k].as_str().unwrap_or("").to_string();
            (s("kind"), s("entity"), s("body"))
        })
        .collect()
}

fn has(rows: &[(String, String, String)], kind: &str, entity: &str) -> bool {
    rows.iter().any(|(k, e, _)| k == kind && e == entity)
}

#[test]
fn approval_applies_the_pinned_candidate_and_the_host_restarts_and_observes() {
    let d = std::env::temp_dir().join(format!("hale_dna_apply_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let (ok, out) = hale(&["dna", "new", "orgapply"], &d);
    assert!(ok, "{out}");
    let app: PathBuf = d.join("orgapply");
    git(&["init", "-q", "-b", "main"], &app);
    git(&["add", "-A"], &app);
    git(&["commit", "-q", "-m", "genome"], &app);
    let base = git(&["rev-parse", "HEAD"], &app);
    std::fs::create_dir_all(app.join("mutate")).unwrap();
    std::fs::write(app.join("mutate/main.hl"), DRIVER).unwrap();
    let (ok, out) = hale(&["run", "mutate"], &app);
    assert!(ok && out.contains("m1: review"), "driver:\n{out}");
    assert_eq!(git(&["rev-parse", "HEAD"], &app), base, "nothing applied before the verdict");

    let cache = std::env::temp_dir().join("hale-tests-iris-cache");
    let mut host = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["dna", "dev", ".", "--no-iris", "--observe", "2"])
        .current_dir(&app)
        .env("XDG_CACHE_HOME", &cache)
        .env("HALE_BIN", env!("CARGO_BIN_EXE_hale"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("hale dna run");
    let up = |app: &Path| app.join(".hale/dna/hale-dna.review.verdict.sock").exists() && app.join(".hale/dna/hale-dna.intent.offered.sock").exists();
    let dl = Instant::now() + Duration::from_secs(90);
    while Instant::now() < dl && !up(&app) {
        std::thread::sleep(Duration::from_millis(200));
    }
    // the host, then the processes it started (their pids are in .hale/dna)
    let finish = |host: &mut std::process::Child| {
        let _ = host.kill();
        let _ = host.wait();
        for f in ["org.pid", "app.pid"] {
            if let Ok(pid) = std::fs::read_to_string(app.join(".hale/dna").join(f)) {
                let _ = Command::new("kill").args(["-9", pid.trim()]).status();
            }
        }
    };
    if !up(&app) {
        finish(&mut host);
        panic!("the membrane did not come up");
    }
    std::thread::sleep(Duration::from_millis(500));
    let (ok, verdict) = hale(&["dna", "review", "m1", "approve", "--as", "riley"], &app);
    if !(ok && verdict.contains("settled: approve by riley")) {
        finish(&mut host);
        panic!("verdict:\n{verdict}");
    }
    // the organism applies, the host restarts, the window passes, the report lands
    let dl = Instant::now() + Duration::from_secs(120);
    while Instant::now() < dl && !has(&journal(&app), "mutation.retained", "m1") {
        std::thread::sleep(Duration::from_millis(300));
    }
    let rows = journal(&app);
    let retained = has(&rows, "mutation.retained", "m1");
    let (ok3, status) = hale(&["dna", "status"], &app);
    let (ok4, history) = hale(&["dna", "history", "m1"], &app);
    let head = git(&["rev-parse", "HEAD"], &app);
    let count = git(&["rev-list", "--count", "HEAD"], &app);
    let still_up = up(&app) && host.try_wait().ok().flatten().is_none();
    finish(&mut host);
    let kinds: Vec<String> = rows.iter().map(|(k, e, b)| format!("{k} {e} {}", &b[..b.len().min(60)])).collect();
    assert!(retained, "the Mutation was not retained within 120s:\n{}", kinds.join("\n"));
    assert!(has(&rows, "mutation.applied", "m1"), "{}", kinds.join("\n"));
    assert!(has(&rows, "expression.restart_requested", "m1"), "{}", kinds.join("\n"));
    let restarted = rows.iter().find(|(k, e, _)| k == "expression.restarted" && e == "m1").expect("the restarted expression journaled itself");
    assert!(restarted.2.contains("build "), "expression named (by the host, GH #566 F2): {}", restarted.2);
    let observed = rows.iter().find(|(k, e, _)| k == "expression.observed" && e == "m1").expect("the host reported on the membrane");
    assert!(observed.2.starts_with("healthy "), "{}", observed.2);
    assert_eq!(count, "2", "exactly the candidate was applied");
    assert_ne!(head, base);
    assert!(rows.iter().any(|(k, e, b)| k == "mutation.applied" && e == "m1" && *b == head), "HEAD is the applied candidate: {head}\n{}", kinds.join("\n"));
    assert!(rows.iter().any(|(k, e, b)| k == "mutation.worktree" && e == "m1" && b == "removed"), "worktree dissolved");
    assert!(ok3 && status.contains("m1 [retained]"), "status:\n{status}");
    assert!(ok4 && history.contains("expression.restarted") && history.contains("mutation.retained"), "history:\n{history}");
    assert!(still_up, "the restarted organism is up with its membrane");
    assert!(!app.join(".hale/dna/worktrees/m1").exists(), "the worktree is dissolved");
    let _ = std::fs::remove_dir_all(&d);
}

//! GH #529 D4 — the blocking human Review over a real Mutation, from
//! the CLI. A governed application under git; a driver program (the
//! same `dna::Dna` the Genome assembles, with scripted models) runs
//! one Mutation up to its Review; `hale dna review` renders the source
//! diff, the semantic diff and the evidence table OFFLINE from the
//! Journal and the receipts; the organism, started afterwards, re-births
//! the pending Review from the Journal; a verdict naming another
//! digest is refused by the Review; the maintainer's verdict on the
//! exact candidate settles it.

#[path = "support/reap.rs"]
mod reap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn hale(args: &[&str], cwd: &Path) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(args)
        .current_dir(cwd)
        .env("HALE_BIN", env!("CARGO_BIN_EXE_hale"))
        .env("HALE_DNA_DISCOVER", "off")
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
            child: "orgrev",
            grant: dna::Grant { child: "orgrev", classes: "docs refactor", max_magnitude: 4, review: "pre" }
        }
    };
    println(core.mutate("t0", "docs", "document the entrypoint", "main.hl", 1));
}
"#;

#[test]
fn a_mutation_is_rendered_offline_and_decided_through_the_organism() {
    let Some(_nats_owner) = std::env::var("HALE_DNA_NATS_URL_OWNER").ok().filter(|d| !d.is_empty()) else {
        eprintln!("dna_review: no HALE_DNA_NATS_URL_OWNER; a verdict cannot reach the organism, so nothing was exercised");
        return;
    };
    let d = std::env::temp_dir().join(format!("hale_dna_review_{}", std::process::id()));
    let _reap = reap::ReapOnDrop(d.clone());
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
    assert!(ok && list.contains("16 pending review(s) of 16") && list.contains("m1 needs leader") && list.contains("docs · candidate"), "{list}");
    let (ok, view) = hale(&["dna", "review", "m1"], &app);
    assert!(ok, "{view}");
    for needle in [
        "review m1 [pending]: apply m1 (docs): document the entrypoint?",
        "disposition under the grant: stage",
        "source diff (git",
        "+// tuned: the entrypoint is documented here",
        "semantic diff (hale model diff",
        "classification: source-only",
        "evidence (fmt=0 check=0 verify=0 test=0 diff=0 rollback=0 fleet=0):",
        "rollback   yes    0",
        "base       yes    0",
        "check      yes    0",
        "test       yes    0",
        "decide: hale dna review m1 approve|revise|reject|abstain",
    ] {
        assert!(view.contains(needle), "missing {needle:?} in:\n{view}");
    }
    let (ok, st) = hale(&["dna", "status"], &app);
    assert!(ok && st.contains("mutations:  1") && st.contains("m1 [stage] docs: document the entrypoint (main.hl) · task t0"), "{st}");

    // 3. the organism, started AFTER the request, re-births the Review
    let cache = std::env::temp_dir().join("hale-tests-iris-cache");
    let (ok, migrated) = hale(&["dna", "nerves", "migrate"], &app);
    assert!(ok, "{migrated}");
    let nats_spine = migrated.lines().find_map(|l| l.strip_prefix("HALE_DNA_NATS_URL_SPINE=")).expect("the spine's URL").to_string();
    let nats_org = migrated.lines().find_map(|l| l.strip_prefix("HALE_DNA_NATS_ORG=")).expect("the organization's token").to_string();
    let log = d.join("run.stderr");
    let mut host = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["dna", "run", ".", "--no-iris"])
        .current_dir(&app)
        .env("XDG_CACHE_HOME", &cache)
        .env("HALE_BIN", env!("CARGO_BIN_EXE_hale"))
        .env("HALE_DNA_DISCOVER", "off")
        .env("HALE_DNA_NATS_URL_SPINE", &nats_spine)
        .env("HALE_DNA_NATS_ORG", &nats_org)
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(&log).unwrap())
        .spawn()
        .expect("hale dna run");
    let nerves_up = || std::fs::read_to_string(&log).unwrap_or_default().contains("the organization reads its facts from the nerves");
    let dl = Instant::now() + Duration::from_secs(180);
    while Instant::now() < dl && !nerves_up() {
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
    if !nerves_up() {
        finish(&mut host);
        panic!("the organism never read its facts from the nerves:\n{}", std::fs::read_to_string(&log).unwrap_or_default());
    }
    std::thread::sleep(Duration::from_millis(500));
    // a verdict naming another digest is refused BY THE REVIEW
    let (ok1, out1) = hale(&["dna", "review", "m1", "approve", "--as", "riley", "--digest", "0000000000000000000000000000000000000000"], &app);
    // the maintainer's verdict on the exact candidate settles it
    let (ok2, out2) = hale(&["dna", "review", "m1", "approve", "--as", "riley", "--comment", "fine"], &app);
    // the settle answers first; the apply follows in the organization's
    // own handler, a moment later on a loaded shard
    let mut ok3 = false;
    let mut out3 = String::new();
    let dl = Instant::now() + Duration::from_secs(60);
    while Instant::now() < dl {
        let (o, s) = hale(&["dna", "status"], &app);
        ok3 = o;
        out3 = s;
        if out3.contains("m1 [applied]") || out3.contains("m1 [retained]") {
            break;
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    let (ok4, out4) = hale(&["dna", "history", "m1"], &app);
    finish(&mut host);
    // #649: the candidate is kept under the mutation's name whatever
    // happens to its worktree — listed, shown as a diff, and dropped by a row
    let kept = Command::new("git").args(["rev-parse", "-q", "--verify", "refs/dna/candidates/m1"]).current_dir(&app).output().unwrap();
    assert!(kept.status.success(), "the candidate is pointed at under refs/dna/candidates/m1");
    let (ok5, out5) = hale(&["dna", "candidates"], &app);
    assert!(ok5 && out5.contains("m1") && out5.contains("review m1 settled"), "candidates:\n{out5}");
    let (ok6, out6) = hale(&["dna", "candidates", "m1"], &app);
    assert!(ok6 && out6.contains("candidate m1 at") && out6.contains("diff ("), "one candidate as a diff:\n{out6}");
    let (ok7, out7) = hale(&["dna", "candidates", "drop", "m1", "--why", "reviewed and applied; the diff is in the genome", "--as", "riley"], &app);
    assert!(ok7 && out7.contains("candidate m1 dropped by riley"), "drop:\n{out7}");
    let gone = Command::new("git").args(["rev-parse", "-q", "--verify", "refs/dna/candidates/m1"]).current_dir(&app).output().unwrap();
    assert!(!gone.status.success(), "dropped: no longer pointed at");
    let (ok8, out8) = hale(&["dna", "history", "m1"], &app);
    assert!(ok8 && out8.contains("candidate.dropped"), "the drop is a row:\n{out8}");
    assert!(ok1 && out1.contains("refused the verdict: digest mismatch"), "wrong digest:\n{out1}");
    assert!(ok2 && out2.contains("review m1 settled: approve by riley"), "settle:\n{out2}");
    // approval applies (D5): the mutation is past `reviewed` by the time the status is read
    assert!(ok3 && out3.contains("15 pending of 16") && (out3.contains("m1 [applied]") || out3.contains("m1 [retained]")), "status:\n{out3}");
    assert!(ok4 && out4.contains("evidence.check") && out4.contains("review.requested") && out4.contains("review.settled") && out4.contains("mutation.candidate") && out4.contains("mutation.applied"), "history:\n{out4}");
    let _ = std::fs::remove_dir_all(&d);
}

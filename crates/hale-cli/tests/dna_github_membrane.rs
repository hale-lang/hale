//! GH #566 F4 — GitHub as a membrane: a pending Review becomes a pull
//! request with the three views as its body; a GitHub review becomes a
//! `review.verdict` row in the reviewer's login with the authority the
//! project grants that login; the settlement goes back as a comment
//! and the genome is pushed. `gh` is a fake on PATH here: it logs what
//! it was asked and answers from files the test writes, so the mapping
//! runs without the network.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn git(args: &[&str], cwd: &Path) -> String {
    let out = Command::new("git").args(["-c", "user.name=riley", "-c", "user.email=r@l"]).args(args).current_dir(cwd).output().expect("git");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn journal(app: &Path) -> Vec<(String, String, String)> {
    let out = Command::new("git").args(["-C", &app.to_string_lossy(), "show", "refs/dna/journal:journal.jsonl"]).output().unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .map(|v| {
            let s = |k: &str| v[k].as_str().unwrap_or("").to_string();
            (s("kind"), s("entity"), s("body"))
        })
        .collect()
}

fn wait_for(app: &Path, secs: u64, kind: &str, entity: &str) -> bool {
    let dl = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < dl {
        if journal(app).iter().any(|(k, e, _)| k == kind && e == entity) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    false
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
                quick: dna::FakeModel { name: "quick", answer: cur + "// documented for the pull request\n" },
                deep: dna::FakeModel { name: "deep", answer: "docs_coverage +" }
            }
        },
        review_policy: dna::OrgPolicy { },
        membrane: dna::Board { who: "board" },
        boundary: dna::AutonomyBoundary { child: "orggh", grant: dna::Grant { child: "orggh", classes: "docs refactor", max_magnitude: 4, review: "pre" } }
    };
    println(core.mutate("t0", "docs", "document the entrypoint in main.hl", "main.hl", 1));
}
"#;

/// The fake `gh`: logs every invocation; `pr create` prints a URL;
/// `pr view N` prints `<dir>/pr<N>.json`; `pr comment` logs.
const FAKE_GH: &str = r#"#!/bin/sh
dir="$(dirname "$0")"
echo "$@" >> "$dir/gh.log"
case "$1 $2" in
  "pr create") echo "https://github.com/o/r/pull/7" ;;
  "pr view") cat "$dir/pr$3.json" 2>/dev/null || echo '{"state":"OPEN","headRefOid":"","reviews":[]}' ;;
  "pr comment") echo "commented" ;;
  *) echo "fake gh: $*" >&2; exit 1 ;;
esac
"#;

#[test]
fn a_pending_review_becomes_a_pull_request_and_its_review_becomes_the_verdict() {
    let d = std::env::temp_dir().join(format!("hale_dna_gh_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    // the fake gh on PATH
    let bin = d.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(bin.join("gh"), FAKE_GH).unwrap();
    let _ = Command::new("chmod").args(["+x", &bin.join("gh").to_string_lossy()]).status();
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default());
    let hale = |args: &[&str], cwd: &Path| -> (bool, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_hale"))
            .args(args)
            .current_dir(cwd)
            .env("PATH", &path)
            .env("HALE_BIN", env!("CARGO_BIN_EXE_hale"))
            .env("XDG_CACHE_HOME", std::env::temp_dir().join("hale-tests-iris-cache"))
            .output()
            .expect("hale");
        (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
    };
    let (ok, out) = hale(&["dna", "new", "orggh"], &d);
    assert!(ok, "{out}");
    let app: PathBuf = d.join("orggh");
    let bare = d.join("origin.git");
    git(&["init", "-q", "--bare", "-b", "main", &bare.to_string_lossy()], &d);
    git(&["config", "user.name", "riley"], &app);
    git(&["config", "user.email", "r@l"], &app);
    git(&["config", "dna.github", "o/r"], &app);
    git(&["config", "dna.github.board", "octocat,riley"], &app);
    git(&["add", "-A"], &app);
    git(&["commit", "-q", "-m", "the app"], &app);
    git(&["remote", "add", "origin", &bare.to_string_lossy()], &app);
    git(&["push", "-q", "origin", "main", "refs/dna/*:refs/dna/*"], &app);
    let base = git(&["rev-parse", "HEAD"], &app);
    // one Mutation up to its Review, offline
    std::fs::create_dir_all(app.join("mutate")).unwrap();
    std::fs::write(app.join("mutate/main.hl"), DRIVER).unwrap();
    let (ok, out) = hale(&["run", "mutate"], &app);
    assert!(ok && out.contains("m1: review"), "driver:\n{out}");
    let cand = journal(&app).iter().find(|(k, e, _)| k == "mutation.candidate" && e == "m1").map(|r| r.2.clone()).expect("candidate");

    // the host mirrors it out, and reads GitHub's review back in
    let mut host = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["dna", "run", ".", "--no-iris"])
        .current_dir(&app)
        .env("PATH", &path)
        .env("XDG_CACHE_HOME", std::env::temp_dir().join("hale-tests-iris-cache"))
        .env("HALE_BIN", env!("CARGO_BIN_EXE_hale"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("hale dna run");
    let stop = |host: &mut std::process::Child| {
        let _ = host.kill();
        let _ = host.wait();
        for f in ["org.pid", "app.pid"] {
            if let Ok(pid) = std::fs::read_to_string(app.join(".hale/dna").join(f)) {
                let _ = Command::new("kill").args(["-9", pid.trim()]).status();
            }
        }
    };
    let opened = wait_for(&app, 90, "github.pr", "m1");
    if !opened {
        stop(&mut host);
        panic!("no pull request was opened for m1:\n{}", std::fs::read_to_string(bin.join("gh.log")).unwrap_or_default());
    }
    // octocat approves on GitHub, at the candidate's head
    std::fs::write(
        bin.join("pr7.json"),
        format!(r#"{{"state":"OPEN","headRefOid":"{cand}","reviews":[{{"author":{{"login":"octocat"}},"state":"APPROVED","commit":{{"oid":"{cand}"}}}}]}}"#),
    )
    .unwrap();
    let settled = wait_for(&app, 90, "review.settled", "m1");
    let told = wait_for(&app, 60, "github.commented", "m1");
    std::thread::sleep(Duration::from_secs(1));
    let rows = journal(&app);
    let log = std::fs::read_to_string(bin.join("gh.log")).unwrap_or_default();
    let origin_main = git(&["rev-parse", "main"], &bare);
    stop(&mut host);
    let dump: Vec<String> = rows.iter().map(|(k, e, b)| format!("{k} {e} {}", b.chars().take(90).collect::<String>())).collect();
    let dump = dump.join("\n");
    assert!(log.contains("pr create --repo o/r --base main --head dna/m1 --title"), "the pull request was opened:\n{log}");
    assert!(log.contains("source diff (git") && log.contains("evidence ("), "the body is the review's three views:\n{log}");
    let pr = rows.iter().find(|(k, e, _)| k == "github.pr" && e == "m1").unwrap();
    assert!(pr.2.contains("\"number\":7") && pr.2.contains("dna/m1"), "{}", pr.2);
    assert_eq!(git(&["rev-parse", "refs/heads/dna/m1"], &bare), cand, "the candidate is on origin as dna/m1");
    assert!(settled, "the GitHub review became the verdict and settled:\n{dump}");
    let verdict = rows.iter().find(|(k, e, _)| k == "review.verdict" && e == "m1").expect("a verdict row");
    assert!(verdict.2.contains("\"reviewer\":\"octocat\"") && verdict.2.contains("\"authority\":\"board\"") && verdict.2.contains(&format!("\"github\":\"octocat@{cand}@APPROVED\"")), "{}", verdict.2);
    assert!(rows.iter().any(|(k, e, b)| k == "review.settled" && e == "m1" && b == "approve by octocat"), "{dump}");
    assert!(rows.iter().filter(|(k, e, _)| k == "review.verdict" && e == "m1").count() == 1, "the same GitHub review is not read twice:\n{dump}");
    assert!(told && log.contains("pr comment 7 --repo o/r --body dna: review m1 settled: approve by octocat"), "the settlement went back:\n{log}");
    assert_eq!(origin_main, cand, "an approval pushed the genome, so GitHub sees the merge");
    assert_ne!(origin_main, base);
    let _ = std::fs::remove_dir_all(&d);
}

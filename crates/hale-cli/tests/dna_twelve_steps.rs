//! GH #529 D7 — the twelve steps of #521, end to end, on the in-repo
//! acceptance application (`dna/acceptance/chat-server`, the site's
//! chat server). Every step leaves its evidence in the Journal, which
//! `hale dna history` walks. The embedded iris view (step 2) is the
//! same status projection `hale dna status --json` prints; CI runs
//! the organism without a browser.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn hale(args: &[&str], cwd: &Path) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(args)
        .current_dir(cwd)
        .env("HALE_BIN", env!("CARGO_BIN_EXE_hale"))
        .env("HALE_DNA_ONESHOT", "1")
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

fn wait_for(app: &Path, secs: u64, kind: &str, entity: &str) -> bool {
    let dl = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < dl {
        if has(&journal(app), kind, entity) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    false
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dst = to.join(e.file_name());
        if e.path().is_dir() {
            copy_dir(&e.path(), &dst);
        } else {
            std::fs::copy(e.path(), &dst).unwrap();
        }
    }
}

#[test]
fn the_twelve_steps_run_on_the_acceptance_application() {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let d = std::env::temp_dir().join(format!("hale_dna_twelve_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    let app = d.join("orgtwelve");
    copy_dir(&repo.join("dna/acceptance/chat-server"), &app);
    git(&["init", "-q", "-b", "main"], &app);
    git(&["add", "-A"], &app);
    git(&["commit", "-q", "-m", "the chat server"], &app);

    // 1. an existing application adds DNA through ordinary machinery
    //    and still passes its previous checks
    let (ok, out) = hale(&["check", "."], &app);
    assert!(ok, "before: {out}");
    let (ok, out) = hale(&["test", "."], &app);
    assert!(ok && out.contains("1 passed, 0 failed"), "before: {out}");
    let (ok, out) = hale(&["dna", "init", "."], &app);
    assert!(ok, "init: {out}");
    let (ok, out) = hale(&["check", "--matrix", "."], &app);
    assert!(ok, "after init, the law: {out}");
    let (ok, out) = hale(&["test", "."], &app);
    assert!(ok && out.contains("1 passed, 0 failed"), "after init, the app's own tests: {out}");
    // the organism's editor runs on scripted models in CI: the Genome
    // is project-owned, so the test edits it like a maintainer would
    let assembly_path = app.join("dna/org/main.hl");
    let assembly = std::fs::read_to_string(&assembly_path).unwrap();
    let start = assembly.find("editor: dna::SourceEditor {").expect("the organization wires an editor");
    let end = assembly[start..].find("genome_seed:").expect("genome_seed follows the editor") + start;
    let scripted = "editor: dna::SourceEditor {\n                name: \"editor\",\n                models: dna::ModelRouter {\n                    quick: dna::FakeModel { name: \"quick\", answer_file: \".hale/dna/scripted-edit.hl\", answer_role: \"edit\" },\n                    deep: dna::FakeModel { name: \"deep\", answer: \"guests_greeted +\" }\n                }\n            },\n            ";
    std::fs::write(&assembly_path, format!("{}{}{}", &assembly[..start], scripted, &assembly[end..])).unwrap();
    let main = std::fs::read_to_string(app.join("main.hl")).unwrap();
    std::fs::write(app.join(".hale/dna/scripted-edit.hl"), format!("{main}// documented by the organism: the rooms are the only way to the signer\n")).unwrap();
    let (ok, out) = hale(&["check", "--matrix", "."], &app);
    assert!(ok, "the scripted Genome still checks: {out}");
    git(&["add", "-A"], &app);
    git(&["commit", "-q", "-m", "attach the DNA"], &app);
    let base = git(&["rev-parse", "HEAD"], &app);

    // 2. a local governed session (iris reads the same status projection)
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
    let dl = Instant::now() + Duration::from_secs(120);
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
    // the verdict flags are read fresh per call; the organism must not see ONESHOT
    let run = |args: &[&str]| -> (bool, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_hale"))
            .args(args)
            .current_dir(&app)
            .env("HALE_BIN", env!("CARGO_BIN_EXE_hale"))
            .env("XDG_CACHE_HOME", &cache)
            .output()
            .unwrap();
        (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
    };

    // 3 + 4. a human asks; the intent births a durable Task
    let (ok, ask) = run(&["dna", "ask", "document", "the", "chat", "server", "in", "main.hl"]);
    let asked = ok && ask.contains("task t1 born");
    // 5–8. Workflow, Step, the editing Attempt under its grant, the
    //      worktree, the evidence, the boundary's Review
    let requested = wait_for(&app, 120, "review.requested", "review:m1");
    // 9. the same pending Review in the CLI and in the projection iris reads
    let (ok1, list) = run(&["dna", "review"]);
    let (ok2, view) = run(&["dna", "review", "m1"]);
    let (ok3, st) = run(&["dna", "status", "--json"]);
    // 10. approval applies the exact reviewed candidate
    let (ok4, verdict) = run(&["dna", "review", "m1", "approve", "--as", "riley", "--comment", "the rooms stay the only way"]);
    // 11 + 12. expressed, observed, the pressure re-measured, retained
    let retained = wait_for(&app, 120, "mutation.retained", "m1");
    // rejection leaves genome and expression intact
    let (ok5, ask2) = run(&["dna", "ask", "document", "the", "chat", "server", "in", "main.hl"]);
    let requested2 = wait_for(&app, 120, "review.requested", "review:m2");
    let (ok6, verdict2) = run(&["dna", "review", "m2", "reject", "--as", "riley", "--comment", "once is enough"]);
    std::thread::sleep(Duration::from_millis(800));
    let (ok7, status) = run(&["dna", "status"]);
    let (ok8, hist_t1) = run(&["dna", "history", "t1"]);
    let (ok9, hist_m1) = run(&["dna", "history", "m1"]);
    let rows = journal(&app);
    let head = git(&["rev-parse", "HEAD"], &app);
    let count = git(&["rev-list", "--count", "HEAD"], &app);
    let still_up = up(&app) && host.try_wait().ok().flatten().is_none();
    finish(&mut host);
    let (ok10, after) = hale(&["check", "."], &app);
    let (ok11, tests_after) = hale(&["test", "."], &app);

    let kinds: Vec<String> = rows.iter().map(|(k, e, b)| format!("{k} {e} {}", &b[..b.len().min(70)])).collect();
    let dump = kinds.join("\n");
    assert!(asked, "3: {ask}");
    assert!(has(&rows, "intent.offered", "i1") || rows.iter().any(|(k, _, _)| k == "intent.offered"), "4: {dump}");
    assert!(has(&rows, "task.born", "t1"), "4: {dump}");
    assert!(requested, "5–8: no review.requested for m1 within 120s:\n{dump}");
    let proposed = rows.iter().find(|(k, e, _)| k == "mutation.proposed" && e == "m1").expect("6: proposed");
    assert!(proposed.2.starts_with("task t1 application:"), "6: the Mutation names its Task: {}", proposed.2);
    assert!(rows.iter().any(|(k, e, b)| k == "mutation.located" && e == "m1" && b.contains("main.hl under read edit fmt check @")), "5: the Attempt inspects under its grant:\n{dump}");
    assert!(has(&rows, "mutation.worktree", "m1"), "6: {dump}");
    let cand = rows.iter().find(|(k, e, _)| k == "mutation.candidate" && e == "m1").map(|r| r.2.clone()).expect("6: candidate");
    for step in ["base", "fmt", "check", "verify", "test", "rollback", "diff", "magnitude"] {
        assert!(has(&rows, &format!("evidence.{step}"), &cand), "7: evidence.{step} on the candidate:\n{dump}");
    }
    assert!(rows.iter().any(|(k, e, b)| k == "model.called" && e.starts_with("m1/a0") && b.contains("read edit fmt check @")), "5: the model calls carry the tool grant:\n{dump}");
    let req = rows.iter().find(|(k, e, _)| k == "review.requested" && e == "review:m1").unwrap();
    assert!(req.2.contains("\"disposition\": \"escalate\"") || req.2.contains("\"disposition\": \"review\"") || req.2.contains("\"disposition\": \"stage\""), "8: a blocking Review: {}", req.2);
    assert!(ok1 && list.contains("m1 needs board"), "9 cli list: {list}");
    assert!(ok2 && view.contains("source diff (git") && view.contains("+// documented by the organism") && view.contains("semantic diff (hale model diff") && view.contains("evidence (") && view.contains("check      yes"), "9 cli render: {view}");
    let st: serde_json::Value = serde_json::from_str(&st).expect("status --json");
    assert!(ok3 && st["reviews"].as_array().unwrap().iter().any(|r| r["mutation_id"] == "m1" && r["state"] == "pending" && r["evidence"].as_str().unwrap_or("").contains("check=0")), "9 the projection iris reads: {st}");
    assert!(ok4 && verdict.contains("review m1 settled: approve by riley"), "10: {verdict}");
    assert!(rows.iter().any(|(k, e, b)| k == "mutation.applied" && e == "m1" && *b == cand), "10: {dump}");
    assert!(retained, "11–12: not retained within 120s:\n{dump}");
    assert!(has(&rows, "expression.restart_requested", "m1") && has(&rows, "expression.restarted", "m1"), "11: {dump}");
    assert!(rows.iter().any(|(k, e, b)| k == "expression.observed" && e == "m1" && b.starts_with("healthy")), "11: {dump}");
    assert!(rows.iter().any(|(k, e, b)| k == "pressure.remeasured" && e == "m1" && b.contains("fitness guests_greeted +: healthy")), "11: the originating pressure is re-measured:\n{dump}");
    assert!(has(&rows, "mutation.retained", "m1"), "12: {dump}");
    // behind the organism's off-thread bus the Task settled `pending` in
    // its live pass and the assembly settled it in the Journal (F.15)
    assert!(rows.iter().any(|(k, e, b)| k == "task.pending" && e == "t1" && b.contains("routed:in-flight")), "4/5: the Task's live pass, pending:\n{dump}");
    assert!(rows.iter().any(|(k, e, b)| k == "task.done" && e == "t1" && b.contains("by editor: m1: review")), "the Task settled through the Journal:\n{dump}");
    // rejection: intact
    assert!(ok5 && ask2.contains("task t2 born"), "{ask2}");
    assert!(requested2, "m2 requested:\n{dump}");
    assert!(ok6 && verdict2.contains("review m2 settled: reject by riley"), "{verdict2}");
    assert!(has(&rows, "mutation.rejected", "m2"), "10 rejection journaled:\n{dump}");
    assert_eq!(head, cand, "HEAD is m1's candidate; m2 changed nothing");
    assert_ne!(head, base);
    assert_eq!(count, "3", "the chat server, the DNA, one applied Mutation");
    assert!(ok7 && status.contains("m1 [retained]") && status.contains("m2 [rejected]") && status.contains("t1 [done]"), "status:\n{status}");
    assert!(ok8 && hist_t1.contains("intent.offered") && hist_t1.contains("task.born") && hist_t1.contains("mutation.proposed") && hist_t1.contains("task.done"), "history t1:\n{hist_t1}");
    for needle in ["mutation.worktree", "evidence.check", "review.requested", "review.settled", "mutation.applied", "expression.restart_requested", "expression.restarted", "expression.observed", "pressure.remeasured", "mutation.retained"] {
        assert!(ok9 && hist_m1.contains(needle), "history m1 lacks {needle}:\n{hist_m1}");
    }
    assert!(still_up, "the restarted organism is up");
    assert!(ok10, "the applied genome still checks: {after}");
    // the application binary carries none of the organization (GH #566 F2)
    let bin = std::fs::read(app.join("orgtwelve")).expect("the application binary");
    let needle = b"vendor_dna";
    assert!(!bin.windows(needle.len()).any(|w| w == needle), "the application binary contains DNA symbols");
    assert!(ok11 && tests_after.contains("1 passed, 0 failed"), "and its tests pass: {tests_after}");
    let _ = std::fs::remove_dir_all(&d);
}

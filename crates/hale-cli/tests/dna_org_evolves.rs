//! GH #566 F3 — the organization evolves through the same loop as the
//! code. Persistent pressure on one part of the application becomes a
//! growth Mutation of the organization's own seed (`dna/org`), verified
//! like any change, the Board's to decide (`hale dna board` lists it),
//! rendered with the new position in its semantic diff; approval applies
//! it, the host restarts the organization itself, the window judges it,
//! and the new position is live. `hale dna report` files what happened.

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

#[test]
fn persistent_pressure_grows_the_organization_through_the_board() {
    let d = std::env::temp_dir().join(format!("hale_dna_grow_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let (ok, out) = hale(&["dna", "new", "orggrow"], &d);
    assert!(ok, "{out}");
    let app: PathBuf = d.join("orggrow");
    // scripted models: the editor answers with a prepared org main; the Leader abstains
    let org_path = app.join("dna/org/main.hl");
    let mut org = std::fs::read_to_string(&org_path).unwrap();
    let start = org.find("editor: dna::SourceEditor {").expect("the organization wires an editor");
    let end = org[start..].find("genome_seed:").expect("genome_seed follows the editor") + start;
    let scripted_editor = "editor: dna::SourceEditor {\n                name: \"editor\",\n                models: dna::ModelRouter {\n                    quick: dna::FakeModel { name: \"quick\", answer_file: \".hale/dna/scripted-org.hl\", answer_role: \"edit\" },\n                    deep: dna::FakeModel { name: \"deep\", answer: \"support_latency -\" }\n                }\n            },\n            ";
    org = format!("{}{}{}", &org[..start], scripted_editor, &org[end..]);
    let ls = org.find("leader: dna::Leader = dna::Leader {").expect("the Leader");
    let le = org[ls..].find("};").expect("the Leader's end") + ls + 2;
    let scripted_leader = "leader: dna::Leader = dna::Leader {\n            name: \"leader\",\n            models: dna::ModelRouter { quick: dna::FakeModel { name: \"quick\", answer: \"Abstain\" }, deep: dna::FakeModel { name: \"deep\", answer: \"Abstain\" } },\n            receipts: dna::GitReceipts { repo: \".\" },\n            source: dna::SourceReader { repo: \".\" }\n        };";
    org = format!("{}{}{}", &org[..ls], scripted_leader, &org[le..]);
    std::fs::write(&org_path, &org).unwrap();
    let (ok, out) = hale(&["check", "--matrix", "."], &app);
    assert!(ok, "the scripted organization checks: {out}");
    git(&["add", "-A"], &app);
    git(&["commit", "-q", "-m", "the app and its organization"], &app);
    let base = git(&["rev-parse", "HEAD"], &app);
    // what the growth proposes: a new position for the pressured part
    let grown = org.replace(
        "        purpose: dna::Review = dna::Review {",
        "        // grown under pressure from billing: a supervisor for that wing\n        billing: dna::Leader = dna::Leader { name: \"billing-supervisor\", receipts: dna::GitReceipts { repo: \".\" }, source: dna::SourceReader { repo: \".\" } };\n        purpose: dna::Review = dna::Review {",
    );
    assert_ne!(grown, org);
    std::fs::write(app.join(".hale/dna/scripted-org.hl"), &grown).unwrap();

    let cache = std::env::temp_dir().join("hale-tests-iris-cache");
    let mut host = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["dna", "run", ".", "--no-iris", "--observe", "2"])
        .current_dir(&app)
        .env("XDG_CACHE_HOME", &cache)
        .env("HALE_BIN", env!("CARGO_BIN_EXE_hale"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("hale dna run");
    let up = |app: &Path| app.join(".hale/dna/hale-dna.pressure.raised.sock").exists() && app.join(".hale/dna/hale-dna.review.verdict.sock").exists();
    let dl = Instant::now() + Duration::from_secs(120);
    while Instant::now() < dl && !up(&app) {
        std::thread::sleep(Duration::from_millis(200));
    }
    let stop = |host: &mut std::process::Child| {
        let _ = host.kill();
        let _ = host.wait();
        for f in ["org.pid", "app.pid"] {
            if let Ok(pid) = std::fs::read_to_string(app.join(".hale/dna").join(f)) {
                let _ = Command::new("kill").args(["-9", pid.trim()]).status();
            }
        }
    };
    if !up(&app) {
        stop(&mut host);
        panic!("the membrane did not come up");
    }
    std::thread::sleep(Duration::from_millis(500));
    let org_pid_before = std::fs::read_to_string(app.join(".hale/dna/org.pid")).unwrap_or_default();

    // pressure, three times from one source: the third proposes, and proposes as a candidate
    for _ in 0..3 {
        let (ok, out) = hale(&["dna", "pressure", "raise", "billing", "invoices", "queue", "behind", "support"], &app);
        assert!(ok, "{out}");
        std::thread::sleep(Duration::from_millis(400));
    }
    let requested = wait_for(&app, 120, "review.requested", "review:m1");
    let (ok1, board) = hale(&["dna", "board"], &app);
    let (ok2, view) = hale(&["dna", "review", "m1"], &app);
    let (ok3, verdict) = hale(&["dna", "review", "m1", "approve", "--as", "riley", "--comment", "grow it"], &app);
    let retained = wait_for(&app, 120, "mutation.retained", "m1");
    let org_pid_after = std::fs::read_to_string(app.join(".hale/dna/org.pid")).unwrap_or_default();
    let (ok4, report) = hale(&["dna", "report"], &app);
    let (ok5, pressure) = hale(&["dna", "pressure"], &app);
    let (ok6, status) = hale(&["dna", "status"], &app);
    let rows = journal(&app);
    let head = git(&["rev-parse", "HEAD"], &app);
    let live = git(&["show", "HEAD:dna/org/main.hl"], &app);
    stop(&mut host);
    let dump: Vec<String> = rows.iter().map(|(k, e, b)| format!("{k} {e} {}", b.chars().take(80).collect::<String>())).collect();
    let dump = dump.join("\n");
    assert!(requested, "the growth candidate reached a Review:\n{dump}");
    assert!(has(&rows, "appendage.proposed", "billing"), "{dump}");
    assert!(rows.iter().any(|(k, e, b)| k == "appendage.candidate" && e == "billing" && b == "m1: review"), "the proposal is a candidate:\n{dump}");
    let req = rows.iter().find(|(k, e, _)| k == "review.requested" && e == "review:m1").unwrap();
    assert!(req.2.contains("\"change_class\": \"organization\"") && req.2.contains("\"seed\": \"dna/org\"") && req.2.contains("\"required_authority\": \"board\""), "an organization change, the Board's: {}", req.2);
    assert!(ok1 && board.contains("2 review(s) need your verdict") && board.contains("organization · dna/org") && board.contains("proposals: 1"), "board:\n{board}");
    assert!(ok2 && view.contains("billing") && view.contains("+billing: Leader"), "the review shows the new position in the semantic diff:\n{view}");
    assert!(ok3 && verdict.contains("review m1 settled: approve by riley"), "{verdict}");
    assert!(retained, "the grown organization was not retained within 120s:\n{dump}");
    assert!(rows.iter().any(|(k, e, b)| k == "expression.restart_requested" && e == "m1" && b.contains("seed dna/org")), "{dump}");
    assert!(has(&rows, "expression.restarted", "m1"), "the restarted organization recorded itself:\n{dump}");
    assert_ne!(org_pid_before, org_pid_after, "the organization was restarted");
    assert_ne!(head, base);
    assert!(live.contains("billing-supervisor"), "the position is in the genome:\n{live}");
    assert!(ok4 && report.contains("report r") && report.contains("applied 1") && report.contains("retained 1") && report.contains("pressure 3") && report.contains("proposals 1"), "{report}");
    assert!(has(&rows, "report.filed", &rows.iter().find(|(k, _, _)| k == "report.filed").map(|r| r.1.clone()).unwrap_or_default()) || journal(&app).iter().any(|(k, _, _)| k == "report.filed"), "the report is in the record");
    assert!(ok5 && pressure.contains("3 signal(s) raised") && pressure.contains("appendage.candidate"), "{pressure}");
    assert!(ok6 && status.contains("m1 [retained] organization:"), "{status}");
    let _ = std::fs::remove_dir_all(&d);
}

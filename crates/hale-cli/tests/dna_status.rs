//! GH #528 — `hale dna status / ask / history / review` over the
//! membrane (Track C, PR 26). The host publishes and reads; the
//! organism decides; the Journal is the record both consult.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn hale(args: &[&str], cwd: &Path) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale")).args(args).current_dir(cwd).output().expect("hale");
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

#[test]
fn status_ask_review_and_history_read_the_organism_through_the_journal() {
    let d = std::env::temp_dir().join(format!("hale_dna_status_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let (ok, out) = hale(&["dna", "new", "orgstat"], &d);
    assert!(ok, "{out}");
    let app: PathBuf = d.join("orgstat");
    let cache = std::env::temp_dir().join("hale-tests-iris-cache");

    // offline: the Journal answers, and says the organism is not running
    let (ok, out) = hale(&["dna", "status"], &app);
    assert!(ok, "{out}");
    assert!(out.contains("not running") && out.contains("chain verified") && out.contains("1 pending of 1") && out.contains("needs maintainer"), "{out}");
    let (ok, out) = hale(&["dna", "ask", "anything"], &app);
    assert!(!ok && out.contains("not running"), "{out}");
    let (ok, out) = hale(&["dna", "history"], &app);
    assert!(ok && out.contains("application.attached") && out.contains("review.requested"), "{out}");

    // the organism, unobserved
    let mut host = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["dna", "run", ".", "--no-iris"])
        .current_dir(&app)
        .env("XDG_CACHE_HOME", &cache)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("hale dna run");
    let dl = Instant::now() + Duration::from_secs(60);
    while Instant::now() < dl && !(app.join(".hale/dna/hale-dna.intent.offered.sock").exists() && app.join(".hale/dna/hale-dna.review.verdict.sock").exists()) {
        std::thread::sleep(Duration::from_millis(200));
    }
    let finish = |host: &mut std::process::Child| {
        let _ = Command::new("pkill").args(["-x", "orgstat"]).status();
        let _ = host.wait();
    };
    if !app.join(".hale/dna/hale-dna.intent.offered.sock").exists() {
        finish(&mut host);
        panic!("the membrane did not come up");
    }
    let env_cache = |c: &mut Command| { c.env("XDG_CACHE_HOME", &cache); };
    let run = |args: &[&str]| -> (bool, String) {
        let mut c = Command::new(env!("CARGO_BIN_EXE_hale"));
        c.args(args).current_dir(&app);
        env_cache(&mut c);
        let out = c.output().unwrap();
        (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
    };
    // ask: a Task is born, and the answer comes from the Journal
    let (ok, out) = run(&["dna", "ask", "write", "the", "changelog"]);
    let ask_out = out.clone();
    let asked = ok && out.contains("task t1 born");
    // review: the wrong authority is refused BY THE REVIEW, the right one settles it
    let (ok1, out1) = run(&["dna", "review", "purpose", "approve", "--authority", "agent", "--as", "bot"]);
    let refused = ok1 && out1.contains("refused the verdict") && out1.contains("authority agent does not satisfy maintainer");
    let (ok2, out2) = run(&["dna", "review", "purpose", "approve", "--as", "riley", "--comment", "ratified"]);
    let settled = ok2 && out2.contains("settled: approve by riley");
    let (ok3, out3) = run(&["dna", "status"]);
    let (ok4, out4) = run(&["dna", "status", "--json"]);
    let (ok5, out5) = run(&["dna", "history", "t1"]);
    finish(&mut host);
    assert!(asked, "ask: {ask_out}");
    assert!(refused, "review (wrong authority): {out1}");
    assert!(settled, "review (maintainer): {out2}");
    assert!(ok3 && out3.contains("running (membrane bound)") && out3.contains("0 pending of 1") && out3.contains("settled approve by riley") && out3.contains("(1 verdict(s) refused)"), "status:\n{out3}");
    assert!(ok4, "{out4}");
    let st: serde_json::Value = serde_json::from_str(&out4).expect("status --json is JSON");
    assert_eq!(st["journal"]["chain"], "verified");
    assert_eq!(st["intents"]["offered"], 1);
    assert_eq!(st["tasks"][0]["id"], "t1");
    assert_eq!(st["reviews"][0]["state"], "settled");
    assert!(ok5 && out5.contains("history of t1") && out5.contains("task.born") && out5.contains("intent.offered"), "history:\n{out5}");
    let _ = std::fs::remove_dir_all(&d);
}

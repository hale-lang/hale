//! GH #596 W — work that is not a software change. The leader's plan
//! calls an ask a person's job; the organism hands the Task on and the
//! record keeps it HANDED; the person reports it done with
//! `hale dna task done <id> --as <who>`, a row in their name and nothing
//! else. A Task the organism is working, or has settled, is not a
//! person's to close.

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

fn journal(app: &Path) -> Vec<(String, String, String, String)> {
    let out = Command::new("git").args(["show", "refs/dna/journal:journal.jsonl"]).current_dir(app).output().unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).unwrap();
            let s = |k: &str| v[k].as_str().unwrap_or("").to_string();
            (s("kind"), s("entity"), s("body"), s("author"))
        })
        .collect()
}

/// A catalog whose leader calls every ask a person's job.
const PERSON_CATALOG: &str = r#"// a catalog for the task-done test
import "vendor/dna" as dna;

fn frontier() -> dna::FakeModel {
    return dna::FakeModel { name: "deep", model: "deep-1", answer: "kind: person\nclass: docs\ntarget: \ncount: 1\nThis is a call to make, not a change to the software.", price_micros: 50 };
}
fn fast() -> dna::FakeModel {
    return dna::FakeModel { name: "quick", model: "quick-1", answer: "kind: person\nclass: docs\ntarget: \ncount: 1\nThis is a call to make, not a change to the software.", price_micros: 5 };
}
fn desk() -> dna::OpenAiChat {
    return dna::OpenAiChat { name: "private", model: "gpt-4o", credential: dna::HostedCredential { env_var: "HALE_DNA_NO_SUCH_KEY_9c1e" } };
}
fn leader_models() -> dna::ModelRouter { return dna::ModelRouter { quick: fast(), deep: frontier() }; }
fn editor_models() -> dna::ModelRouter { return dna::ModelRouter { quick: fast(), deep: frontier() }; }
fn agent_models() -> dna::ModelRouter { return dna::ModelRouter { quick: fast(), deep: frontier() }; }
fn org_budget() -> dna::BudgetPolicy { return dna::BudgetPolicy { window: "none", allowance_micros: 100000 }; }
fn probe_catalog() -> String {
    return dna::probe("frontier", frontier()) + dna::probe("fast", fast()) + dna::probe("desk", desk());
}
"#;

#[test]
fn a_persons_job_is_handed_and_reported_done_in_their_name() {
    let d = std::env::temp_dir().join(format!("hale_dna_task_done_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let (ok, out) = hale(&["dna", "new", "handed"], &d);
    assert!(ok, "{out}");
    let app: PathBuf = d.join("handed");
    std::fs::write(app.join("dna/org/models.hl"), PERSON_CATALOG).unwrap();
    Command::new("git").args(["-c", "user.name=t", "-c", "user.email=t@l", "add", "-A"]).current_dir(&app).output().unwrap();
    Command::new("git").args(["-c", "user.name=t", "-c", "user.email=t@l", "commit", "-q", "-m", "genome"]).current_dir(&app).output().unwrap();
    // nothing to report before anything was asked
    let (ok, out) = hale(&["dna", "task", "done", "t1", "--as", "riley"], &app);
    assert!(!ok && out.contains("no Task `t1`"), "{out}");
    let (ok, out) = hale(&["dna", "task"], &app);
    assert!(!ok && out.contains("hale dna task done <id>"), "{out}");

    let mut host = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["dna", "run", ".", "--no-iris"])
        .current_dir(&app)
        .env("HALE_BIN", env!("CARGO_BIN_EXE_hale"))
        .env("HALE_DNA_DISCOVER", "off")
        .env("XDG_CACHE_HOME", std::env::temp_dir().join("hale-tests-iris-cache"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("hale dna run");
    let dl = Instant::now() + Duration::from_secs(120);
    while Instant::now() < dl && !app.join(".hale/dna/hale-dna.intent.offered.sock").exists() {
        std::thread::sleep(Duration::from_millis(200));
    }
    let finish = |host: &mut std::process::Child| {
        if let Ok(pid) = std::fs::read_to_string(app.join(".hale/dna/org.pid")) {
            let _ = Command::new("kill").args(["-9", pid.trim()]).status();
        }
        let _ = host.kill();
        let _ = host.wait();
    };
    let (ok, out) = hale(&["dna", "ask", "--no-wait", "call", "the", "supplier", "about", "the", "delayed", "pallets"], &app);
    assert!(ok, "{out}");
    // the leader planned it a person's job, and the organism handed it on
    let dl = Instant::now() + Duration::from_secs(60);
    let mut handed = false;
    while Instant::now() < dl && !handed {
        handed = journal(&app).iter().any(|(k, e, _, _)| k == "task.handed" && e == "t1");
        std::thread::sleep(Duration::from_millis(250));
    }
    if !handed {
        finish(&mut host);
        let dump: Vec<String> = journal(&app).iter().map(|(k, e, b, _)| format!("{k} {e} {}", b.chars().take(80).collect::<String>())).collect();
        panic!("the ask was not handed on:\n{}", dump.join("\n"));
    }
    let rows = journal(&app);
    assert!(rows.iter().any(|(k, e, b, _)| k == "task.planned" && e == "t1" && b.contains("\"kind\": \"person\"")), "planned as a person's job");
    assert!(!rows.iter().any(|(k, e, _, _)| k == "mutation.proposed" && e == "m1"), "nothing was mutated");
    // the person reports it done, in their name
    let (ok, out) = hale(&["dna", "task", "done", "t1", "--as", "riley", "--note", "called them; pallets land Thursday"], &app);
    assert!(ok && out.contains("task t1 done by riley: called them; pallets land Thursday"), "{out}");
    let rows = journal(&app);
    let done = rows.iter().find(|(k, e, _, _)| k == "task.done" && e == "t1").expect("task.done");
    assert_eq!(done.3, "riley", "in the person's name");
    assert!(done.2.contains("pallets land Thursday"), "{}", done.2);
    // and not twice, and not for a Task that is not handed
    let (ok, out) = hale(&["dna", "task", "done", "t1", "--as", "riley"], &app);
    assert!(!ok && out.contains("is done, not handed"), "{out}");
    finish(&mut host);
    let (ok, st) = hale(&["dna", "status"], &app);
    assert!(ok && st.contains("t1") && st.contains("done"), "{st}");
    let _ = std::fs::remove_dir_all(&d);
}

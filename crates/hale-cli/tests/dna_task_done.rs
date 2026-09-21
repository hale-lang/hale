//! GH #596 W — work that is not a software change. The leader's plan
//! calls an ask a person's job; the organism hands the Task on and the
//! record keeps it HANDED; the person reports it done with
//! `hale dna task done <id> --as <who>`, a row in their name and nothing
//! else. A Task the organism is working, or has settled, is not a
//! person's to close.

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
    return dna::FakeModel { name: "deep", model: "deep-1", answer: "kind: person\nclass: docs\ntarget: \ncount: 1\nassignee: noor\nThis is a call to make, not a change to the software.", price_micros: 50 };
}
fn fast() -> dna::FakeModel {
    return dna::FakeModel { name: "quick", model: "quick-1", answer: "kind: person\nclass: docs\ntarget: \ncount: 1\nassignee: noor\nThis is a call to make, not a change to the software.", price_micros: 5 };
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
    let _reap = reap::ReapOnDrop(d.clone());
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
        handed = journal(&app).iter().any(|(k, e, _, _)| k == "task.handed" && e == "t1.s0.p");
        std::thread::sleep(Duration::from_millis(250));
    }
    if !handed {
        finish(&mut host);
        let dump: Vec<String> = journal(&app).iter().map(|(k, e, b, _)| format!("{k} {e} {}", b.chars().take(80).collect::<String>())).collect();
        panic!("the ask was not handed on:\n{}", dump.join("\n"));
    }
    let rows = journal(&app);
    // card 18: the leader's word is bound into the admission; the case is the ask's handed Task
    assert!(rows.iter().any(|(k, e, b, _)| k == "workflow.admitted" && e == "t1" && b.contains("\"definition\": \"ask-person\"")), "planned as a person's job");
    assert!(!rows.iter().any(|(k, e, _, _)| k == "mutation.proposed" && e == "m1"), "nothing was mutated");
    // GH #604 rule 4: the row names the assignee, and only they may close it
    let handed_row = rows.iter().find(|(k, e, _, _)| k == "task.handed" && e == "t1.s0.p").expect("task.handed");
    assert!(handed_row.2.contains("\"assignee\": \"noor\""), "the handed row names noor: {}", handed_row.2);
    let (ok, out) = hale(&["dna", "task", "done", "t1.s0.p", "--as", "riley", "--note", "called them"], &app);
    assert!(!ok && out.contains("handed to noor, not to riley"), "riley cannot close noor's task: {out}");
    // GH #604 rule 5: reassignment is a row naming both; the Task stays handed
    let (ok, out) = hale(&["dna", "task", "reassign", "t1.s0.p", "--to", "dev", "--as", "noor"], &app);
    assert!(ok && out.contains("task t1.s0.p reassigned from noor to dev by noor"), "{out}");
    let (ok, out) = hale(&["dna", "task", "done", "t1.s0.p", "--as", "noor"], &app);
    assert!(!ok && out.contains("handed to dev, not to noor"), "after reassignment noor cannot close it: {out}");
    let (ok, out) = hale(&["dna", "task", "done", "t1.s0.p", "--as", "dev", "--note", "called them; pallets land Thursday"], &app);
    assert!(ok && out.contains("task t1.s0.p done by dev: called them; pallets land Thursday"), "{out}");
    let rows = journal(&app);
    let done = rows.iter().find(|(k, e, _, _)| k == "task.done" && e == "t1.s0.p").expect("task.done");
    assert_eq!(done.3, "dev", "in the assignee's name");
    assert!(done.2.contains("pallets land Thursday"), "{}", done.2);
    // and not twice, and not for a Task that is not handed
    let (ok, out) = hale(&["dna", "task", "done", "t1.s0.p", "--as", "dev"], &app);
    assert!(!ok && out.contains("is done, not handed"), "{out}");
    // retirement: a second job handed to noor moves to dev when noor retires, as rows
    let (ok, out) = hale(&["dna", "ask", "--no-wait", "call", "the", "supplier", "again", "next", "week"], &app);
    assert!(ok, "{out}");
    let dl = Instant::now() + Duration::from_secs(60);
    let mut handed2 = false;
    while Instant::now() < dl && !handed2 {
        handed2 = journal(&app).iter().any(|(k, e, _, _)| k == "task.handed" && e == "t2.s0.p");
        std::thread::sleep(Duration::from_millis(250));
    }
    assert!(handed2, "the second ask was handed on");
    let (ok, out) = hale(&["dna", "retire", "noor", "--as", "riley"], &app);
    assert!(!ok && out.contains("noor holds 1 handed task(s): t2.s0.p"), "retirement refused while work is held and no successor named: {out}");
    let (ok, out) = hale(&["dna", "retire", "noor", "--to", "dev", "--as", "riley"], &app);
    assert!(ok && out.contains("noor retired by riley; 1 task(s) transferred to dev"), "{out}");
    let rows = journal(&app);
    assert!(rows.iter().any(|(k, e, b, _)| k == "task.reassigned" && e == "t2.s0.p" && b.contains("\"to\": \"dev\"")), "the transfer is a row");
    assert!(rows.iter().any(|(k, e, _, _)| k == "person.retired" && e == "noor"), "the retirement is a row");
    let (ok, out) = hale(&["dna", "task", "done", "t2.s0.p", "--as", "dev", "--note", "done after the handover"], &app);
    assert!(ok, "the successor closes it: {out}");
    finish(&mut host);
    let (ok, st) = hale(&["dna", "status"], &app);
    assert!(ok && st.contains("t1") && st.contains("done"), "{st}");
    let _ = std::fs::remove_dir_all(&d);
}

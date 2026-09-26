//! GH #528 — `hale dna status / ask / history / review` over the
//! nerves (Track C, PR 26; GH #986). The host relays and reads; the
//! organism decides; the Journal is the record both consult.

#[path = "support/reap.rs"]
mod reap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn hale(args: &[&str], cwd: &Path) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale")).args(args).current_dir(cwd).output().expect("hale");
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

#[test]
fn status_ask_review_and_history_read_the_organism_through_the_journal() {
    let Some(_nats_owner) = std::env::var("HALE_DNA_NATS_URL_OWNER").ok().filter(|d| !d.is_empty()) else {
        eprintln!("dna_status: no HALE_DNA_NATS_URL_OWNER; a verdict cannot reach the organism, so nothing was exercised");
        return;
    };
    let d = std::env::temp_dir().join(format!("hale_dna_status_{}", std::process::id()));
    let _reap = reap::ReapOnDrop(d.clone());
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let (ok, out) = hale(&["dna", "new", "orgstat"], &d);
    assert!(ok, "{out}");
    let app: PathBuf = d.join("orgstat");
    let cache = std::env::temp_dir().join("hale-tests-iris-cache");

    // offline: the Journal answers, and says the organism is not running
    let (ok, out) = hale(&["dna", "status"], &app);
    assert!(ok, "{out}");
    assert!(out.contains("not running") && out.contains("chain verified") && out.contains("15 pending of 15") && out.contains("needs board"), "{out}");
    // GH #726: and which DNA source the toolchain it ran carries —
    // `vendor/dna` is that source, not the working tree's
    assert!(
        out.contains(&format!("embedded dna: {} (hale {})", &hale_dna::EMBEDDED_DIGEST[..16], env!("CARGO_PKG_VERSION"))),
        "status opens with the embedded source's digest:\n{out}"
    );
    // a tree materialized by another build is stale, and says so
    let prov = app.join(".hale/dna/embedded.digest");
    let real = std::fs::read_to_string(&prov).unwrap();
    std::fs::write(&prov, "hale 0.0.1\nembedded dna: 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\n").unwrap();
    let (ok, stale) = hale(&["dna", "status"], &app);
    assert!(ok && stale.contains("vendor/dna was materialized from 0123456789abcdef — run `hale dna upgrade`"), "a stale vendor tree is named:\n{stale}");
    std::fs::write(&prov, real).unwrap();
    let (ok, out) = hale(&["dna", "task", "create", "anything"], &app);
    assert!(!ok && out.contains("not running"), "{out}");
    let (ok, out) = hale(&["dna", "history"], &app);
    assert!(ok && out.contains("application.attached") && out.contains("review.requested"), "{out}");

    // the nerves (GH #986): a verdict or a task only reaches the
    // organism over them — the owner migrates the stream, the organism
    // and the host run under the spine's role and the organism's token
    let (ok, migrated) = hale(&["dna", "nerves", "migrate"], &app);
    assert!(ok, "{migrated}");
    let nats_spine = migrated.lines().find_map(|l| l.strip_prefix("HALE_DNA_NATS_URL_SPINE=")).expect("the spine's URL").to_string();
    let nats_org = migrated.lines().find_map(|l| l.strip_prefix("HALE_DNA_NATS_ORG=")).expect("the organization's token").to_string();
    // the organism, unobserved
    let log = d.join("run.stderr");
    let mut host = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["dna", "run", ".", "--no-iris"])
        .current_dir(&app)
        .env("XDG_CACHE_HOME", &cache)
        .env("HALE_DNA_NATS_URL_SPINE", &nats_spine)
        .env("HALE_DNA_NATS_ORG", &nats_org)
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(&log).unwrap())
        .spawn()
        .expect("hale dna run");
    let nerves_up = || std::fs::read_to_string(&log).unwrap_or_default().contains("the organization reads its facts from the nerves");
    // a loaded runner builds the organization beside tests that run for
    // minutes: the deadline is the runner's, not the organism's
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
    let env_cache = |c: &mut Command| { c.env("XDG_CACHE_HOME", &cache); };
    let run = |args: &[&str]| -> (bool, String) {
        let mut c = Command::new(env!("CARGO_BIN_EXE_hale"));
        c.args(args).current_dir(&app);
        env_cache(&mut c);
        let out = c.output().unwrap();
        (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
    };
    // ask: a Task is born, and the answer comes from the Journal
    let (ok, out) = run(&["dna", "task", "create", "write", "the", "changelog"]);
    let ask_out = out.clone();
    let asked = ok && out.contains("task t1 born");
    // review: the wrong authority is refused BY THE REVIEW, the right one settles it
    let (ok1, out1) = run(&["dna", "review", "purpose", "approve", "--authority", "agent", "--as", "bot"]);
    let refused = ok1 && out1.contains("refused the verdict") && out1.contains("authority agent does not satisfy board");
    let (ok2, out2) = run(&["dna", "review", "purpose", "approve", "--as", "riley", "--comment", "ratified"]);
    let settled = ok2 && out2.contains("settled: approve by riley");
    let (ok3, out3) = run(&["dna", "status"]);
    let (ok4, out4) = run(&["dna", "status", "--json"]);
    let (ok5, out5) = run(&["dna", "history", "t1"]);
    // GH #946: an ask for a judgment is admitted as one judgment leaf, an
    // agent's — the legs' relay answers pending for a leg to claim
    let (ok6, out6) = run(&["dna", "task", "create", "--judgment", "assess", "whether", "the", "queue", "is", "bounded"]);
    let (ok7, out7) = run(&["dna", "history", "t2"]);
    finish(&mut host);
    assert!(asked, "ask: {ask_out}");
    assert!(refused, "review (wrong authority): {out1}");
    assert!(settled, "review (maintainer): {out2}");
    assert!(ok3 && out3.contains("running (this clone's body holds the lease)") && out3.contains("14 pending of 15") && out3.contains("settled approve by riley") && out3.contains("(1 verdict(s) refused)"), "status:\n{out3}");
    assert!(ok4, "{out4}");
    let st: serde_json::Value = serde_json::from_str(&out4).expect("status --json is JSON");
    assert_eq!(st["journal"]["chain"], "verified");
    assert_eq!(st["intents"]["offered"], 1);
    assert_eq!(st["tasks"][0]["id"], "t1");
    // the purpose review, not the first row: the seeded design's Reviews sit beside it (GH #596 C)
    let purpose = st["reviews"].as_array().unwrap().iter().find(|r| r["id"] == "purpose").expect("the purpose review in the projection");
    assert_eq!(purpose["state"], "settled");
    assert!(ok5 && out5.contains("history of t1") && out5.contains("task.born") && out5.contains("intent.offered"), "history:\n{out5}");
    assert!(ok6 && out6.contains("task t2 born"), "a judgment asked: {out6}");
    assert!(ok7 && out7.contains("\"definition\": \"ask-judge\"") && out7.contains("attempt t2/wf1/s0/j/a0 by agent"), "the judgment is one leaf for an agent, pending for a leg:\n{out7}");
    let _ = std::fs::remove_dir_all(&d);
}

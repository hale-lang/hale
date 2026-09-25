//! The shakeout's finding 6 — a project whose path is ordinary but not
//! short. A Unix socket address holds 108 bytes, path and all, and the
//! membrane's five sockets used to live at `<root>/.hale/dna/<name>.sock`
//! (39 bytes of suffix, so a root of 70 characters was already too long)
//! bound RELATIVE to the root the organization ran in, while the client
//! wrote absolute routes — so `hale dna task create` could not reach
//! sockets that were there and listening, and the manual shakeout had
//! to move the checkout to `/tmp/…` to proceed. The membrane is gone
//! (GH #986: an ask crosses the nerves, a NATS URL, not a filesystem
//! path), so the 108-byte ceiling no longer applies; this keeps the
//! regression that a long, ordinary project path still works end to
//! end — the organization still starts and hears an ask from it.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn hale(args: &[&str], cwd: &std::path::Path) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(args)
        .current_dir(cwd)
        .env("XDG_CACHE_HOME", std::env::temp_dir().join("hale-tests-iris-cache"))
        .output()
        .expect("hale");
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

#[test]
fn an_intent_reaches_the_organization_from_a_long_project_path() {
    let Some(_nats_owner) = std::env::var("HALE_DNA_NATS_URL_OWNER").ok().filter(|d| !d.is_empty()) else {
        eprintln!("dna_long_path: no HALE_DNA_NATS_URL_OWNER; an ask cannot reach the organization, so nothing was exercised");
        return;
    };
    let d = std::env::temp_dir()
        .join(format!("hale_dna_len_{}", std::process::id()))
        .join("a-fairly-ordinary-checkout-directory")
        .join("nested-inside-another-one");
    let _ = std::fs::remove_dir_all(std::env::temp_dir().join(format!("hale_dna_len_{}", std::process::id())));
    std::fs::create_dir_all(&d).unwrap();
    let (ok, out) = hale(&["dna", "new", "orglong"], &d);
    assert!(ok, "{out}");
    let app: PathBuf = d.join("orglong");
    // the shakeout's finding 6 was a 108-byte Unix socket address the
    // membrane's client and server disagreed on relative vs absolute;
    // the membrane is gone (GH #986), and nothing an organization binds
    // any longer names a path of its own, so there is no ceiling left
    // to cross — this keeps only the regression that an ordinary,
    // not-short project path still works end to end
    let (ok, migrated) = hale(&["dna", "nerves", "migrate"], &app);
    assert!(ok, "{migrated}");
    let nats_spine = migrated.lines().find_map(|l| l.strip_prefix("HALE_DNA_NATS_URL_SPINE=")).expect("the spine's URL").to_string();
    let nats_org = migrated.lines().find_map(|l| l.strip_prefix("HALE_DNA_NATS_ORG=")).expect("the organization's token").to_string();
    let cache = std::env::temp_dir().join("hale-tests-iris-cache");
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
        .expect("spawn hale dna run");
    let up = || std::fs::read_to_string(&log).unwrap_or_default().contains("the organization reads its facts from the nerves");
    let deadline = Instant::now() + Duration::from_secs(120);
    while Instant::now() < deadline && !up() {
        if let Ok(Some(st)) = host.try_wait() {
            panic!("hale dna run exited early: {st}");
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let finish = |host: &mut std::process::Child| {
        if let Ok(pid) = std::fs::read_to_string(app.join(".hale/dna/org.pid")) {
            let _ = Command::new("kill").args(["-9", pid.trim()]).status();
        }
        let _ = host.kill();
        let _ = host.wait();
    };
    if !up() {
        finish(&mut host);
        panic!("the organization never read its facts from the nerves at this path:\n{}", std::fs::read_to_string(&log).unwrap_or_default());
    }
    let (ok, ask) = hale(&["dna", "task", "create", "write", "the", "changelog"], &app);
    let record = || -> String {
        Command::new("git")
            .args(["-C", &app.to_string_lossy(), "show", "refs/dna/journal:journal.jsonl"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_default()
    };
    let born = {
        let dl = Instant::now() + Duration::from_secs(20);
        loop {
            if record().contains("\"task.born\"") {
                break true;
            }
            if Instant::now() > dl {
                break false;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    };
    finish(&mut host);
    assert!(ok, "the ask reached the organization over the nerves: {ask}");
    assert!(born, "the organization heard it and bore a Task: {ask}");
    let _ = std::fs::remove_dir_all(std::env::temp_dir().join(format!("hale_dna_len_{}", std::process::id())));
}

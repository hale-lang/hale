//! The shakeout's finding 6 — a project whose path is ordinary but not
//! short. A Unix socket address holds 108 bytes, path and all, and the
//! membrane's five sockets live at `<root>/.hale/dna/<name>.sock`: 39
//! bytes of suffix, so a root of 70 characters is already too long.
//! The organization binds these names RELATIVE to the root it runs in
//! and was never the long side; the client wrote absolute routes, so
//! `hale dna ask` could not reach sockets that were there and
//! listening, and the manual shakeout had to move the checkout to
//! `/tmp/…` to proceed. Routes are relative on both sides now.

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
fn an_intent_reaches_the_membrane_from_a_long_project_path() {
    let d = std::env::temp_dir()
        .join(format!("hale_dna_len_{}", std::process::id()))
        .join("a-fairly-ordinary-checkout-directory")
        .join("nested-inside-another-one");
    let _ = std::fs::remove_dir_all(std::env::temp_dir().join(format!("hale_dna_len_{}", std::process::id())));
    std::fs::create_dir_all(&d).unwrap();
    let (ok, out) = hale(&["dna", "new", "orglong"], &d);
    assert!(ok, "{out}");
    let app: PathBuf = d.join("orglong");
    let verdict = app.join(".hale/dna/hale-dna.review.verdict.sock");
    assert!(
        verdict.to_string_lossy().len() > 108,
        "this test is only a test while the absolute address does not fit: {} bytes",
        verdict.to_string_lossy().len()
    );

    let cache = std::env::temp_dir().join("hale-tests-iris-cache");
    let mut host = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["dna", "run", ".", "--no-iris"])
        .current_dir(&app)
        .env("XDG_CACHE_HOME", &cache)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn hale dna run");
    let up = || verdict.exists() && app.join(".hale/dna/hale-dna.intent.offered.sock").exists();
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
        panic!("the organization never bound its membrane at this path");
    }
    let (ok, ask) = hale(&["dna", "ask", "write", "the", "changelog"], &app);
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
    assert!(ok, "the ask crossed the membrane: {ask}");
    assert!(born, "the organization heard it and bore a Task: {ask}");
    let _ = std::fs::remove_dir_all(std::env::temp_dir().join(format!("hale_dna_len_{}", std::process::id())));
}

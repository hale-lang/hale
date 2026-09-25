//! GH #617 — one body per record. Two hosts on one remote: the second
//! refuses while the first's lease is live; a forced claim from a head
//! stops the first at its next assertion and is a row in the forcer's
//! name; a partitioned host fails its assertion and stops before
//! relaying anything, once its lease has expired; `profile` reflects a
//! hand-edited remote without any stored label.

#[path = "support/reap.rs"]
mod reap;
#[path = "support/trace.rs"]
mod trace;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn hale_in(args: &[&str], cwd: &Path) -> (bool, i32, String) {
    let _s = trace::Span::new("hale", args.join(" "));
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(args)
        .current_dir(cwd)
        .env("HALE_BIN", env!("CARGO_BIN_EXE_hale"))
        .env("HALE_DNA_DISCOVER", "off")
        .env("XDG_CACHE_HOME", std::env::temp_dir().join("hale-tests-iris-cache"))
        .output()
        .expect("hale");
    (out.status.success(), out.status.code().unwrap_or(-1), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn git(args: &[&str], cwd: &Path) -> String {
    let _s = trace::Span::new("git", args[0].to_string());
    let out = Command::new("git").args(args).current_dir(cwd).output().expect("git");
    assert!(out.status.success(), "git {args:?} in {}: {}", cwd.display(), String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn record(cwd: &Path) -> Vec<serde_json::Value> {
    let out = Command::new("git").args(["show", "refs/dna/journal:journal.jsonl"]).current_dir(cwd).output().unwrap();
    String::from_utf8_lossy(&out.stdout).lines().filter(|l| !l.trim().is_empty()).map(|l| serde_json::from_str(l).unwrap()).collect()
}

/// `hale dna run` in the background, its stderr to a log; the log's path.
fn run_host(app: &Path, log: &Path) -> std::process::Child {
    let _s = trace::Span::new("spawn", "hale dna run");
    Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["dna", "run", ".", "--no-iris"])
        .current_dir(app)
        .env("HALE_BIN", env!("CARGO_BIN_EXE_hale"))
        .env("HALE_DNA_DISCOVER", "off")
        .env("XDG_CACHE_HOME", std::env::temp_dir().join("hale-tests-iris-cache"))
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(log).unwrap())
        .spawn()
        .expect("spawn hale dna run")
}

fn wait_log(log: &Path, needle: &str, secs: u64, host: &mut std::process::Child) -> bool {
    let has = |log: &Path| std::fs::read_to_string(log).unwrap_or_default().contains(needle);
    let mut at_exit: Option<bool> = None;
    let held = trace::wait_until(format!("log: {needle}"), Duration::from_secs(secs), Duration::from_millis(250), || {
        if has(log) {
            return true;
        }
        // the host exited: the log will not grow, so stop waiting on it
        if matches!(host.try_wait(), Ok(Some(_))) {
            at_exit = Some(has(log));
            return true;
        }
        false
    });
    at_exit.unwrap_or(held)
}

/// Whether the organization's process (`.hale/dna/org.pid`) is up: this
/// fixture is about the body lease, not about the organization
/// answering anything, so — unlike a fixture that needs a verdict to
/// cross the nerves — it waits for the process alone (GH #986).
fn wait_org_up(app: &Path, secs: u64, host: &mut std::process::Child) -> bool {
    let up = |app: &Path| -> bool {
        let pid = std::fs::read_to_string(app.join(".hale/dna/org.pid")).unwrap_or_default();
        let pid = pid.trim();
        !pid.is_empty() && Command::new("kill").args(["-0", pid]).status().map(|s| s.success()).unwrap_or(false)
    };
    let mut at_exit: Option<bool> = None;
    let held = trace::wait_until("org.pid written and the organization is up".to_string(), Duration::from_secs(secs), Duration::from_millis(250), || {
        if up(app) {
            return true;
        }
        if matches!(host.try_wait(), Ok(Some(_))) {
            at_exit = Some(up(app));
            return true;
        }
        false
    });
    at_exit.unwrap_or(held)
}

/// Whether a bare remote's record already carries a row of `kind`: used
/// to wait for a host's own body-lease claim to be PUSHED, not merely
/// appended locally — `org.pid` (GH #986's readiness signal here, since
/// this fixture is about the lease, not an answer over the nerves)
/// exists well before the host's first tick pushes anything, so a
/// caller that goes straight on to another clone's `body claim` can
/// race the push.
fn remote_has(bare: &Path, kind: &str) -> bool {
    Command::new("git")
        .args(["--git-dir", &bare.to_string_lossy(), "show", "refs/dna/journal:journal.jsonl"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains(&format!("\"kind\": \"{kind}\"")))
        .unwrap_or(false)
}

fn kill_org(app: &Path) {
    if let Ok(pid) = std::fs::read_to_string(app.join(".hale/dna/org.pid")) {
        let _ = Command::new("kill").args(["-9", pid.trim()]).status();
    }
}

#[test]
fn a_record_admits_one_body_and_a_partitioned_body_stops() {
    let _t = trace::test("dna_body_lease");
    let d = std::env::temp_dir().join(format!("hale_dna_body_{}", std::process::id()));
    let _reap = reap::ReapOnDrop(d.clone());
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let bare = d.join("origin.git");
    git(&["init", "-q", "--bare", "-b", "main", &bare.to_string_lossy()], &d);
    // clone A, made with the remote-body profile: the pieces are set, nothing is labelled
    let (ok, _, out) = hale_in(&["dna", "new", "bodied", "--profile", "remote-body", "--remote", &bare.to_string_lossy(), "--body", "riley@srv"], &d);
    assert!(ok && out.contains("remote origin = ") && out.contains("dna.body = riley@srv"), "{out}");
    let a: PathBuf = d.join("bodied");
    assert_eq!(git(&["remote", "get-url", "origin"], &a), bare.to_string_lossy());
    assert_eq!(git(&["config", "dna.body"], &a), "riley@srv");
    let (_, code, out) = hale_in(&["dna", "new", "labelled", "--profile", "cloud"], &d);
    assert!(code != 0 && out.contains("unknown profile `cloud`"), "a profile is not a label: {out}");
    git(&["config", "user.name", "organism-host"], &a);
    git(&["config", "user.email", "host@dna"], &a);
    git(&["add", "-A"], &a);
    git(&["commit", "-q", "-m", "the app"], &a);
    git(&["push", "-q", "origin", "main", "refs/dna/*:refs/dna/*"], &a);
    // clone B: a head, elsewhere, with its own copy of the toolchain-owned vendor
    let b = d.join("head");
    git(&["clone", "-q", "-b", "main", &bare.to_string_lossy(), &b.to_string_lossy()], &d);
    git(&["config", "user.name", "riley"], &b);
    git(&["config", "user.email", "riley@l"], &b);
    let (ok, _, out) = hale_in(&["dna", "upgrade", "."], &b);
    assert!(ok, "{out}");
    let (ok, _, out) = hale_in(&["dna", "sync"], &b);
    assert!(ok && out.contains("pulled the record"), "{out}");
    // before any body: the profile is detected from the pieces
    let (ok, _, prof) = hale_in(&["dna", "profile", "."], &b);
    assert!(ok && prof.contains("profile:     shared record, no body (detected)") && prof.contains("body:        none"), "{prof}");
    assert!(prof.contains(&format!("record:      shared over origin ({})", bare.to_string_lossy())), "{prof}");
    // host A takes the lease and runs
    let log_a = d.join("a.log");
    let mut host_a = run_host(&a, &log_a);
    assert!(wait_org_up(&a, 180, &mut host_a), "host A did not come up:\n{}", std::fs::read_to_string(&log_a).unwrap_or_default());
    let la = std::fs::read_to_string(&log_a).unwrap();
    assert!(la.contains("body lease taken as ") && la.contains(":") && la.contains("(token 1)"), "{la}");
    // the claim is appended locally at once, but reaches the remote only
    // on the host's first tick (HOST_TICK, 1s) — wait for it before a
    // second clone's own claim races it there
    assert!(
        trace::wait_until("A's claim reached the remote".to_string(), Duration::from_secs(15), Duration::from_millis(200), || remote_has(&bare, "body.claimed")),
        "A's claim never reached the remote"
    );
    // a second body on the same record is refused by the lease at the remote
    let (_, code, out) = hale_in(&["dna", "run", ".", "--no-iris"], &b);
    assert_eq!(code, 3, "{out}");
    assert!(out.contains("a body for this record is live on ") && out.contains("a record admits one body"), "{out}");
    assert!(!out.contains("organization (pid"), "nothing was built or run under a lease that is not ours: {out}");
    let (ok, _, body) = hale_in(&["dna", "body"], &b);
    assert!(ok && body.contains("body: live on ") && body.contains("/bodied, ticked "), "{body}");
    let (ok, _, prof) = hale_in(&["dna", "profile", "."], &b);
    assert!(ok && prof.contains("profile:     local body, shared record (detected)"), "the body runs on this machine: {prof}");
    // a hand-edited remote is reflected at once: no stored label to go stale
    let alias = d.join("alias.git");
    std::os::unix::fs::symlink(&bare, &alias).unwrap();
    git(&["remote", "set-url", "origin", &alias.to_string_lossy()], &b);
    let (ok, _, prof) = hale_in(&["dna", "profile", "."], &b);
    assert!(ok && prof.contains(&format!("shared over origin ({})", alias.to_string_lossy())), "{prof}");
    git(&["remote", "set-url", "origin", &bare.to_string_lossy()], &b);
    // the status projection carries the same two lines
    let (ok, _, st) = hale_in(&["dna", "status"], &b);
    assert!(ok && st.contains("\nprofile:    local body, shared record (detected)\nbody:       live on "), "{st}");
    // a claim without --force is refused while the body is live
    let (_, code, out) = hale_in(&["dna", "body", "claim"], &b);
    assert!(code != 0 && out.contains("a record admits one body") && out.contains("--force"), "{out}");
    // forced: the lease is released as a row in the forcer's name, and A stops at its next assertion
    let (ok, _, out) = hale_in(&["dna", "body", "claim", "--force", "--as", "riley"], &b);
    assert!(ok && out.contains("was taken by riley (body.claimed, forced)"), "{out}");
    assert!(wait_log(&log_a, "the body lease is no longer mine: released", 45, &mut host_a), "host A did not stop:\n{}", std::fs::read_to_string(&log_a).unwrap_or_default());
    let st = host_a.wait().unwrap();
    assert_eq!(st.code(), Some(3), "a body whose lease is gone exits 3");
    assert!(std::fs::read_to_string(a.join(".hale/dna/org.pid")).is_err(), "the organization went with it");
    let (ok, _, out) = hale_in(&["dna", "sync"], &b);
    assert!(ok, "{out}");
    let rows = record(&b);
    let claimed: Vec<&serde_json::Value> = rows.iter().filter(|r| r["kind"] == "body.claimed").collect();
    assert!(claimed.iter().any(|r| r["author"] == "host" && r["body"].as_str().unwrap().contains("\"forced\": false")), "A's own claim is a row: {claimed:?}");
    let forced = claimed.iter().find(|r| r["body"].as_str().unwrap().contains("\"forced\": true")).expect("the forced claim");
    assert!(forced["author"] == "riley" && forced["body"].as_str().unwrap().contains("\"by\": \"riley\"") && forced["entity"].as_str().unwrap().ends_with("/bodied"), "{forced}");
    // A again: the same clone is the same holder and takes the lease straight back
    let mut host_a = run_host(&a, &log_a);
    assert!(wait_org_up(&a, 180, &mut host_a), "{}", std::fs::read_to_string(&log_a).unwrap_or_default());
    assert!(std::fs::read_to_string(&log_a).unwrap().contains("(token 2)"), "the token fences: {}", std::fs::read_to_string(&log_a).unwrap());
    // a partition: the remote goes away; A keeps its lease until it expires, then stops
    let hidden = d.join("origin.hidden");
    std::fs::rename(&bare, &hidden).unwrap();
    let t0 = Instant::now();
    assert!(wait_log(&log_a, "a body that cannot prove its lease executes nothing", 75, &mut host_a), "a partitioned host did not stop:\n{}", std::fs::read_to_string(&log_a).unwrap_or_default());
    assert!(t0.elapsed() >= Duration::from_secs(5), "it kept its lease until the lease expired, not before");
    let st = host_a.wait().unwrap();
    assert_eq!(st.code(), Some(3));
    std::fs::rename(&hidden, &bare).unwrap();
    // the fence stopped the body before its lease expired; the lease goes stale once it has
    let mut stale = (false, String::new());
    for _ in 0..40 {
        let (ok, _, body) = hale_in(&["dna", "body"], &b);
        stale = (ok && body.contains("body: stale: ") && body.contains("its lease has expired"), body);
        if stale.0 {
            break;
        }
        trace::sleep("polling for the lease to go stale", Duration::from_millis(500));
    }
    assert!(stale.0, "{}", stale.1);
    kill_org(&a);
    let _ = std::fs::remove_dir_all(&d);
}

//! GH #617, #1590 — one body per record. Two clones of one genome reach
//! one record in memory: a host in the second refuses while the first's
//! lease is live; a forced claim from a head stops the first at its next
//! assertion and is a row in the forcer's name; a host whose lease memory
//! will not let it renew (its row held locked) stops once the lease it last
//! proved has expired; `profile` reflects a hand-edited genome remote
//! without any stored label.

#[path = "support/vault.rs"]
mod vault;
#[path = "support/record.rs"]
mod record;
#[path = "support/reap.rs"]
mod reap;
#[path = "support/trace.rs"]
mod trace;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn hale_in(args: &[&str], cwd: &Path) -> (bool, i32, String) {
    let _s = trace::Span::new("hale", args.join(" "));
    let out = vault::hale()
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

/// The Ledger's rows (a body's claims are the day's work, GH #1590).
fn ledger(cwd: &Path) -> Vec<serde_json::Value> {
    let (ok, _, out) = hale_in(&["dna", "ledger", "rows"], cwd);
    assert!(ok, "the ledger's rows: {out}");
    out.lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

/// `hale dna run` in the background on the spine's DSN, its stderr to a log.
fn run_host(app: &Path, log: &Path, spine: &str) -> std::process::Child {
    let _s = trace::Span::new("spawn", "hale dna run");
    vault::hale()
        .args(["dna", "run", ".", "--no-iris"])
        .current_dir(app)
        .env("HALE_BIN", env!("CARGO_BIN_EXE_hale"))
        .env("HALE_DNA_DISCOVER", "off")
        .env("HALE_DNA_MEMORY_DSN_SPINE", spine)
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

fn kill_org(app: &Path) {
    if let Ok(pid) = std::fs::read_to_string(app.join(".hale/dna/org.pid")) {
        let _ = Command::new("kill").args(["-9", pid.trim()]).status();
    }
}

#[test]
fn a_record_admits_one_body_and_a_partitioned_body_stops() {
    let _t = trace::test("dna_body_lease");
    let Some(owner) = record::owner_dsn() else {
        eprintln!("dna_body_lease: no HALE_DNA_MEMORY_DSN_OWNER; the body lease is memory's, so nothing was exercised");
        return;
    };
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
    git(&["push", "-q", "origin", "main"], &a);
    // clone B: a head, elsewhere, with its own copy of the toolchain-owned vendor
    let b = d.join("head");
    git(&["clone", "-q", "-b", "main", &bare.to_string_lossy(), &b.to_string_lossy()], &d);
    git(&["config", "user.name", "riley"], &b);
    git(&["config", "user.email", "riley@l"], &b);
    // B reaches the record through memory: `upgrade` migrates it with the
    // owner's DSN and keeps the head's in the clone
    let (ok, _, out) = hale_in(&["dna", "upgrade", "."], &b);
    assert!(ok, "{out}");
    let (ok, _, migrated) = hale_in(&["dna", "memory", "migrate"], &a);
    assert!(ok, "{migrated}");
    let spine = migrated.lines().find_map(|l| l.strip_prefix("HALE_DNA_MEMORY_DSN_SPINE=")).expect("the spine's DSN").to_string();
    // before any body: the profile is detected from the pieces
    let (ok, _, prof) = hale_in(&["dna", "profile", "."], &b);
    assert!(ok && prof.contains("profile:     no body (detected)") && prof.contains("body:        none"), "{prof}");
    assert!(prof.contains(", in memory") && prof.contains(&format!("genome:      shared over origin ({})", bare.to_string_lossy())), "{prof}");
    // host A takes the lease and runs
    let log_a = d.join("a.log");
    let mut host_a = run_host(&a, &log_a, &spine);
    assert!(wait_org_up(&a, 180, &mut host_a), "host A did not come up:\n{}", std::fs::read_to_string(&log_a).unwrap_or_default());
    let la = std::fs::read_to_string(&log_a).unwrap();
    assert!(la.contains("body lease taken as ") && la.contains(":") && la.contains("(token 1)"), "{la}");
    // a second body on the same record is refused by the lease in memory
    let (_, code, out) = {
        let o = vault::hale()
            .args(["dna", "run", ".", "--no-iris"])
            .current_dir(&b)
            .env("HALE_BIN", env!("CARGO_BIN_EXE_hale"))
            .env("HALE_DNA_DISCOVER", "off")
            .env("HALE_DNA_MEMORY_DSN_SPINE", &spine)
            .env("XDG_CACHE_HOME", std::env::temp_dir().join("hale-tests-iris-cache"))
            .output()
            .expect("hale");
        (o.status.success(), o.status.code().unwrap_or(-1), format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)))
    };
    assert_eq!(code, 3, "{out}");
    assert!(out.contains("a body for this record is live on ") && out.contains("a record admits one body"), "{out}");
    assert!(!out.contains("organization (pid"), "nothing was built or run under a lease that is not ours: {out}");
    let (ok, _, body) = hale_in(&["dna", "body"], &b);
    assert!(ok && body.contains("body: live on ") && body.contains("/bodied, ticked "), "{body}");
    let (ok, _, prof) = hale_in(&["dna", "profile", "."], &b);
    assert!(ok && prof.contains("profile:     local body (detected)"), "the body runs on this machine: {prof}");
    // a hand-edited remote is reflected at once: no stored label to go stale
    let alias = d.join("alias.git");
    std::os::unix::fs::symlink(&bare, &alias).unwrap();
    git(&["remote", "set-url", "origin", &alias.to_string_lossy()], &b);
    let (ok, _, prof) = hale_in(&["dna", "profile", "."], &b);
    assert!(ok && prof.contains(&format!("shared over origin ({})", alias.to_string_lossy())), "{prof}");
    git(&["remote", "set-url", "origin", &bare.to_string_lossy()], &b);
    // the status projection carries the same two lines
    let (ok, _, st) = hale_in(&["dna", "status"], &b);
    assert!(ok && st.contains("\nprofile:    local body (detected)\nbody:       live on "), "{st}");
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
    let rows = ledger(&b);
    let claimed: Vec<&serde_json::Value> = rows.iter().filter(|r| r["kind"] == "body.claimed").collect();
    assert!(claimed.iter().any(|r| r["author"] == "host" && r["body"].as_str().unwrap().contains("\"forced\": false")), "A's own claim is a row: {claimed:?}");
    let forced = claimed.iter().find(|r| r["body"].as_str().unwrap().contains("\"forced\": true")).expect("the forced claim");
    assert!(forced["author"] == "riley" && forced["body"].as_str().unwrap().contains("\"by\": \"riley\"") && forced["entity"].as_str().unwrap().ends_with("/bodied"), "{forced}");
    // A again: the same clone is the same holder and takes the lease straight back
    let mut host_a = run_host(&a, &log_a, &spine);
    assert!(wait_org_up(&a, 180, &mut host_a), "{}", std::fs::read_to_string(&log_a).unwrap_or_default());
    assert!(std::fs::read_to_string(&log_a).unwrap().contains("(token 2)"), "the token fences: {}", std::fs::read_to_string(&log_a).unwrap());
    // a partition: memory holds the lease's row locked, so no renewal lands;
    // A keeps the lease it last proved until it expires, then stops
    let sch = record::schema(&a);
    let locker_name = format!("dna-body-lease-{}", std::process::id());
    let mut locker = Command::new("psql")
        .args([owner.as_str(), "-q", "-v", "ON_ERROR_STOP=1", "-c", "BEGIN", "-c", &format!("SELECT 1 FROM {sch}.claims WHERE key = 'body' FOR UPDATE"), "-c", "SELECT pg_sleep(600)"])
        .env("PGAPPNAME", &locker_name)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("psql holds the lease's row");
    let t0 = Instant::now();
    assert!(wait_log(&log_a, "a body that cannot prove its lease executes nothing", 75, &mut host_a), "a partitioned host did not stop:\n{}", std::fs::read_to_string(&log_a).unwrap_or_default());
    assert!(t0.elapsed() >= Duration::from_secs(5), "it kept its lease until the lease expired, not before");
    let st = host_a.wait().unwrap();
    assert_eq!(st.code(), Some(3));
    let _ = locker.kill();
    let _ = locker.wait();
    // the server's side of the lock ends with its backend, not with psql: a
    // killed client leaves `pg_sleep` holding the row for its ten minutes
    let _ = Command::new("psql")
        .args([owner.as_str(), "-q", "-c", &format!("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE application_name = '{locker_name}'")])
        .output();
    // the fence stopped the body before its lease expired; the lease goes stale once it has,
    // within the lease's life (30 s) of its last renewal
    let mut stale = (false, String::new());
    for _ in 0..80 {
        let (ok, _, body) = hale_in(&["dna", "body"], &b);
        stale = (ok && body.contains("body: stale: ") && body.contains("its lease has expired"), body);
        if stale.0 {
            break;
        }
        trace::sleep("polling for the lease to go stale", Duration::from_millis(500));
    }
    assert!(stale.0, "{}", stale.1);
    kill_org(&a);
    record::unmigrate(&a, &owner);
    let _ = std::fs::remove_dir_all(&d);
}

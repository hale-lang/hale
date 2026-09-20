//! GH #617 asks 3 and 4 — the body on a server, and its secrets. A
//! provision against an unreachable host, a record with no remote, or a
//! remote local to this machine writes nothing; the dry run shows the
//! exact plan (the toolchain the lock pins, the clone, the unit that
//! supervises the host). A secret is read from stdin, never argv, lands
//! in a 600 file, and the record carries `secret.rotated <NAME>` and no
//! value. A host with no credential for its model says so on the board
//! at once; one with it hands it to the organization.

#[path = "support/reap.rs"]
mod reap;
#[path = "support/trace.rs"]
mod trace;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

fn hale_env(args: &[&str], cwd: &Path, home: &Path, stdin: Option<&str>) -> (bool, String) {
    let _s = trace::Span::new("hale", args.join(" "));
    let mut c = Command::new(env!("CARGO_BIN_EXE_hale"));
    c.args(args)
        .current_dir(cwd)
        .env("HALE_BIN", env!("CARGO_BIN_EXE_hale"))
        .env("HALE_DNA_DISCOVER", "off")
        .env("XDG_CACHE_HOME", std::env::temp_dir().join("hale-tests-iris-cache"))
        .env("HOME", home)
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("OPENAI_API_KEY")
        .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() });
    let mut child = c.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().expect("hale");
    if let Some(text) = stdin {
        use std::io::Write;
        child.stdin.take().unwrap().write_all(text.as_bytes()).unwrap();
    }
    let out = child.wait_with_output().unwrap();
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

fn git(args: &[&str], cwd: &Path) -> String {
    let _s = trace::Span::new("git", args[0].to_string());
    let out = Command::new("git").args(args).current_dir(cwd).output().expect("git");
    assert!(out.status.success(), "git {args:?} in {}: {}", cwd.display(), String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn journal(cwd: &Path) -> String {
    let _s = trace::Span::new("git", "show refs/dna/journal:journal.jsonl");
    let out = Command::new("git").args(["show", "refs/dna/journal:journal.jsonl"]).current_dir(cwd).output().unwrap();
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn run_host(app: &Path, home: &Path, log: &Path) -> std::process::Child {
    let _s = trace::Span::new("spawn", "hale dna run");
    Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["dna", "run", ".", "--no-iris"])
        .current_dir(app)
        .env("HALE_BIN", env!("CARGO_BIN_EXE_hale"))
        .env("HALE_DNA_DISCOVER", "off")
        .env("XDG_CACHE_HOME", std::env::temp_dir().join("hale-tests-iris-cache"))
        .env("HOME", home)
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("OPENAI_API_KEY")
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

fn stop_host(app: &Path, host: &mut std::process::Child) {
    if let Ok(pid) = std::fs::read_to_string(app.join(".hale/dna/org.pid")) {
        let _ = Command::new("kill").args(["-9", pid.trim()]).status();
    }
    let _ = host.wait();
}

#[test]
fn a_body_is_provisioned_only_where_it_can_be_and_secrets_never_reach_the_record() {
    let _t = trace::test("dna_body_provision");
    let d = std::env::temp_dir().join(format!("hale_dna_prov_{}", std::process::id()));
    let _reap = reap::ReapOnDrop(d.clone());
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let home_empty = d.join("home-empty");
    let home = d.join("home");
    std::fs::create_dir_all(&home_empty).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    let (ok, out) = hale_env(&["dna", "new", "prov"], &d, &home, None);
    assert!(ok, "{out}");
    let app: PathBuf = d.join("prov");
    git(&["config", "user.name", "riley"], &app);
    git(&["config", "user.email", "riley@l"], &app);
    git(&["add", "-A"], &app);
    git(&["commit", "-q", "-m", "the app"], &app);
    // no remote: the body has nothing to clone
    let (ok, out) = hale_env(&["dna", "body", "provision", "riley@srv"], &app, &home, None);
    assert!(!ok && out.contains("has no remote") && out.contains("nothing was written"), "{out}");
    // a remote local to this machine: the body could not reach it
    let bare = d.join("origin.git");
    git(&["init", "-q", "--bare", "-b", "main", &bare.to_string_lossy()], &d);
    git(&["remote", "add", "origin", &bare.to_string_lossy()], &app);
    let (ok, out) = hale_env(&["dna", "body", "provision", "riley@srv"], &app, &home, None);
    assert!(!ok && out.contains("local to this machine") && out.contains("Nothing was written"), "{out}");
    // an unreachable host: stop before writing anything
    git(&["remote", "set-url", "origin", "git@example.invalid:riley/prov.git"], &app);
    let (ok, out) = hale_env(&["dna", "body", "provision", "nobody@localhost.invalid", "--dsn", "postgres://dna:pw@db:5432/dna"], &app, &home, None);
    assert!(!ok && out.contains("ssh nobody@localhost.invalid is not reachable") && out.contains("nothing was written"), "{out}");
    let cfg = Command::new("git").args(["config", "dna.body"]).current_dir(&app).output().unwrap();
    assert!(!cfg.status.success(), "no body was recorded: {}", String::from_utf8_lossy(&cfg.stdout));
    assert!(!journal(&app).contains("body.provisioned"), "no row either");
    // the dry run is the plan, exactly: the pinned toolchain, the clone, the unit
    let lock = std::fs::read_to_string(app.join("hale.lock")).unwrap();
    let pin = lock.split("toolchain = \"").nth(1).unwrap().split('"').next().unwrap().to_string();
    assert!(!pin.is_empty());
    let (ok, plan) = hale_env(&["dna", "body", "provision", "riley@srv", "--dsn", "postgres://dna:pw@db:5432/dna", "--dry-run"], &app, &home, None);
    assert!(ok, "{plan}");
    // a body's name on a machine carries its record's identity (#635)
    let genesis = git(&["rev-list", "--max-parents=0", "refs/dna/journal"], &app);
    let key = format!("prov-{}", &genesis.trim()[..12]);
    for needle in [
        "nothing was written",
        &format!("PIN='{pin}'"),
        "REMOTE='git@example.invalid:riley/prov.git'",
        "DSN='postgres://dna:pw@db:5432/dna'",
        "HALE_VERSION=\"v$PIN\" sh -c \"$(curl -fsSL https://hale-lang.org/install.sh)\"",
        "git clone -q \"$REMOTE\" \"$DIR\"",
        "hale dna upgrade .",
        "HALE_DNA_KNOWLEDGE_DSN=$DSN",
        "hale-dna-$NAME.service",
        "WorkingDirectory=%h/dna/prov",
        &format!("EnvironmentFile=-%h/.config/hale-dna/{key}.env"),
        "ExecStart=%h/.hale/bin/hale dna dev . --no-iris",
        "Restart=on-failure",
        "systemctl --user enable --now",
        "then here: git config dna.body riley@srv",
    ] {
        assert!(plan.contains(needle), "the plan names {needle}:\n{plan}");
    }
    let cfg = Command::new("git").args(["config", "dna.body"]).current_dir(&app).output().unwrap();
    assert!(!cfg.status.success(), "a dry run writes nothing");
    // without the DSN the plan brings Postgres up from compose
    let (ok, plan) = hale_env(&["dna", "body", "provision", "riley@srv", "--dry-run"], &app, &home, None);
    assert!(ok && plan.contains("DSN=''") && plan.contains("docker compose version"), "{plan}");
    // body start/stop/logs need a body
    let (ok, out) = hale_env(&["dna", "body", "logs"], &app, &home, None);
    assert!(!ok && out.contains("no body is known here"), "{out}");
    git(&["remote", "set-url", "origin", &bare.to_string_lossy()], &app);
    git(&["push", "-q", "origin", "main", "refs/dna/*:refs/dna/*"], &app);

    // ---- secrets ----
    let catalog = std::fs::read_to_string(app.join("dna/org/models.hl")).unwrap();
    let cred = catalog.split("env_var: \"").nth(1).expect("the catalog names a credential").split('"').next().unwrap().to_string();
    let (ok, out) = hale_env(&["dna", "secret", "set", &format!("{cred}=sk-on-argv")], &app, &home, None);
    assert!(!ok && out.contains("never goes on the command line"), "{out}");
    let (ok, out) = hale_env(&["dna", "secret", "set", "not a name"], &app, &home, Some("x\n"));
    assert!(!ok && out.contains("is not an environment variable name"), "{out}");
    let (ok, out) = hale_env(&["dna", "secret", "set", &cred], &app, &home, Some(""));
    assert!(!ok && out.contains("no value was given on stdin"), "{out}");
    let (ok, out) = hale_env(&["dna", "secret", "rotate", &cred], &app, &home, Some("x\n"));
    assert!(!ok && out.contains("was never set"), "{out}");
    let (ok, out) = hale_env(&["dna", "secret", "set", &cred], &app, &home, Some("sk-test-123\n"));
    assert!(ok && out.contains(&format!("secret set: {cred} is in ")) && out.contains("the value is nowhere in the record"), "{out}");
    let env_file = home.join(format!(".config/hale-dna/{key}.env"));
    assert_eq!(std::fs::read_to_string(&env_file).unwrap(), format!("{cred}=sk-test-123\n"));
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&env_file).unwrap().permissions().mode() & 0o777, 0o600, "only this user reads it");
    }
    let (ok, out) = hale_env(&["dna", "secret", "rotate", &cred], &app, &home, Some("sk-test-456\n"));
    assert!(ok && out.contains(&format!("secret rotate: {cred} is in ")), "{out}");
    assert_eq!(std::fs::read_to_string(&env_file).unwrap(), format!("{cred}=sk-test-456\n"), "one line per name, the newest");
    let j = journal(&app);
    assert_eq!(j.matches(&format!("\"kind\": \"secret.rotated\", \"entity\": \"{cred}\"")).count(), 2, "{j}");
    assert!(!j.contains("sk-test"), "the value is nowhere in the record: {j}");
    assert!(std::fs::read_dir(app.join(".hale/dna")).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().starts_with("secret.")), "no temp file is left behind");

    // ---- the host: a missing credential surfaces at once ----
    let log = d.join("host.log");
    let mut host = run_host(&app, &home_empty, &log);
    assert!(wait_log(&log, "membrane bound", 180, &mut host), "{}", std::fs::read_to_string(&log).unwrap_or_default());
    let l = std::fs::read_to_string(&log).unwrap();
    assert!(l.contains(&format!("no credential for the model: none of {cred} is set here or in ~/.config/hale-dna/{key}.env")), "{l}");
    let (ok, st) = hale_env(&["dna", "status"], &app, &home_empty, None);
    assert!(ok && st.contains(&format!("; no credential for the model (none of {cred} is set where the body runs)")), "{st}");
    let (ok, board) = hale_env(&["dna", "board"], &app, &home_empty, None);
    assert!(ok && board.contains(&format!("body: no credential for the model (none of {cred} is set where the body runs)")), "{board}");
    stop_host(&app, &mut host);
    assert!(journal(&app).contains("\"kind\": \"body.credential_missing\", \"entity\": \"model\""));
    // with the secret on this machine: present, handed to the organization, and the board is clear
    let mut host = run_host(&app, &home, &log);
    assert!(wait_log(&log, "membrane bound", 180, &mut host), "{}", std::fs::read_to_string(&log).unwrap_or_default());
    assert!(std::fs::read_to_string(&log).unwrap().contains("the model's credential is present now"), "{}", std::fs::read_to_string(&log).unwrap());
    let org_pid = std::fs::read_to_string(app.join(".hale/dna/org.pid")).unwrap().trim().to_string();
    let environ = std::fs::read(format!("/proc/{org_pid}/environ")).unwrap_or_default();
    assert!(String::from_utf8_lossy(&environ).contains(&format!("{cred}=sk-test-456")), "the organization has the credential in its environment");
    let (ok, st) = hale_env(&["dna", "status"], &app, &home, None);
    assert!(ok && !st.contains("no credential"), "{st}");
    stop_host(&app, &mut host);
    assert!(journal(&app).contains("\"kind\": \"body.credential_present\", \"entity\": \"model\""));
    let _ = std::fs::remove_dir_all(&d);
}

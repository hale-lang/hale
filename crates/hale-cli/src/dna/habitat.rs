//! `hale habitat` — the acts of the machine's habitat (GH #1653).
//!
//! The local habitat is long-lived: one compose project per machine,
//! named for the habitat, holding a service per allocation — an
//! organism's memory, nerves and senses' store, named for the organism's
//! record identity and the role. Organisms are its tenants, reused from
//! run to run, and no command that works on an organism (`dev`, `run`,
//! `head`, `show`, `work`, the host's verbs) provisions or migrates
//! anything as a side effect: each reports what is missing and names the
//! act that supplies it. This module holds those acts. `migrate` brings
//! the organism's allocations up and moves memory's schema and the
//! nerves' stream to this toolchain's; `up`, `status` and `down` follow
//! with the habitat's genome.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use super::{host_run, local_habitat_dir, local_habitat_project, manifest_pin, memory_migrate, nerves_migrate, project, MemoryPlan, NervesPlan};

/// The verb's help: stdout and exit 0 for `--help`, stderr and exit 2
/// for a usage error (GH #817).
pub(crate) fn usage_text() -> &'static str {
    r#"usage: hale habitat up [--foreground] [--profile local]
       hale habitat status
       hale habitat down
       hale habitat migrate [organism]
       hale habitat sweep <root>

The local habitat holds every organism's infrastructure on this machine:
one docker compose project, named for the habitat, with a service per
allocation (memory-<id>, nerves-<id>, senses-<id>, <id> the organism's
record identity), under ${XDG_DATA_HOME:-~/.local/share}/hale/habitat/local.
It is long-lived: nothing tears it down when work ends, and two checkouts
of one organism are the same tenant.

up                   birth the habitat the first time, from the genome the
                     toolchain ships (its positions the nursery's `habitat`
                     pack, its law the `standard` pack and its document's
                     `law` items), the one organism born without a parent;
                     then bring every recorded allocation up, migrating any
                     whose recorded schema version is behind this
                     toolchain's, and start the habitat's body: a systemd
                     user unit named for the habitat, or, with
                     --foreground, in this process. Idempotent. The hosted
                     profile (backends the platform runs) is not built.
status               the habitat, its genome and body, and each
                     allocation: its organism, whether each role's service
                     runs, and the schema version it records against this
                     toolchain's.
down                 stop the habitat's body and every service of its
                     project; their data is kept, and `up` brings them back.
migrate [organism]   bring the organism's allocations up and migrate them:
                     memory's schema and the nerves' stream move to this
                     toolchain's, and the head's DSN is kept in the
                     checkout's .hale/dna/memory.env. With
                     HALE_DNA_MEMORY_DSN_OWNER or HALE_DNA_NATS_URL_OWNER
                     set, that server of your own is migrated instead.
                     Idempotent. The organism defaults to the current
                     directory. Each role's version is recorded in the
                     allocation's allocation.json.

sweep <root>         take down every compose project run from under <root>,
                     a test's habitat: its containers, networks and
                     volumes, confirmed gone by the project's label. The
                     projects are named in <root>/.dna-compose-projects
                     until they are, so a teardown that did not finish is
                     retried. Never the developer's habitat (under $HOME),
                     nor the one XDG_DATA_HOME names outside <root>. Exit 1
                     names what survived; nothing is done without docker.

Nothing else migrates or provisions: `hale dna dev`, `run`, `head`, `show`
and `work` run on what is up and migrated, and name this command when it
is not.
"#
}

pub(crate) fn run(args: &[String]) -> ExitCode {
    match args.first().map(String::as_str) {
        Some("--help" | "-h") => {
            print!("{}", usage_text());
            ExitCode::SUCCESS
        }
        Some("migrate") => migrate(args.get(1).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))),
        Some("up") => up(&args[1..]),
        Some("status") if args.len() == 1 => status(),
        Some("down") if args.len() == 1 => down(),
        Some("sweep") if args.len() == 2 => match down_under(Path::new(&args[1])) {
            Ok(()) => ExitCode::SUCCESS,
            Err(why) => {
                eprintln!("hale habitat sweep: {why}");
                ExitCode::from(1)
            }
        },
        _ => {
            eprint!("{}", usage_text());
            ExitCode::from(2)
        }
    }
}

fn migrate(dir: PathBuf) -> ExitCode {
    let root = match project(&dir) {
        Ok((root, _)) => root,
        Err(e) => {
            eprintln!("hale habitat migrate: {e}");
            return ExitCode::from(2);
        }
    };
    match migrate_lines(&root) {
        Ok(lines) => {
            for l in lines {
                println!("{l}");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("hale habitat migrate: {e}");
            ExitCode::from(1)
        }
    }
}

/// The organism at `root`'s allocations brought up and migrated: what was
/// done, a line each, or why it could not be.
fn migrate_lines(root: &Path) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    match memory_migrate(root) {
        Ok(MemoryPlan::Roles { .. }) => out.push(format!("memory  migrated; the head's DSN is kept in {}", root.join(".hale/dna/memory.env").display())),
        Ok(MemoryPlan::NoDatabase(why)) => return Err(format!("memory: {why}")),
        Err(e) => return Err(format!("memory: {e}")),
    }
    match nerves_migrate(root) {
        Ok(NervesPlan::Roles(_)) => out.push("nerves  migrated; the organization's stream is in place".to_string()),
        Ok(NervesPlan::NoServer(why)) => return Err(format!("nerves: {why}")),
        Err(e) => return Err(format!("nerves: {e}")),
    }
    // the senses' store is the habitat's when memory and the nerves are:
    // an environment that names servers of its own names its own store.
    // Nothing reads it but the reflexes, so one that cannot come up is said
    if !own_servers() {
        match host_run("senses-up", root, &[]) {
            Ok(o) if !o.starts_with("none: ") => out.push(format!("senses  up ({})", o.trim().trim_start_matches("HALE_DNA_SENSES_URL="))),
            Ok(o) => out.push(format!("senses  not up: {}", o.trim().strip_prefix("none: ").unwrap_or(o.trim()))),
            Err(e) => out.push(format!("senses  not up: {e}")),
        }
    }
    Ok(out)
}

/// Whether this environment names memory or nerves of its own.
fn own_servers() -> bool {
    ["HALE_DNA_MEMORY_DSN_OWNER", "HALE_DNA_NATS_URL_OWNER"].iter().any(|k| std::env::var(k).is_ok_and(|v| !v.is_empty()))
}

// ---------------------------------------------------------------
// the allocation record (GH #1653)
// ---------------------------------------------------------------

/// The directory of the organism at `root`'s allocations, `None` when it
/// pins no record identity.
fn allocation_dir(root: &Path) -> Option<PathBuf> {
    let id = manifest_pin(&root.join("hale.toml")).ok().flatten()?;
    (id.len() >= 12).then(|| PathBuf::from(local_habitat_dir()).join("allocations").join(&id[..12]))
}

/// GH #1653: an allocation is a row in the habitat's own record, as every
/// provisioning act is: `habitat.allocated <id>/<role>` the first time an
/// organism's role is provisioned (its record identity, the role, the
/// service that is its tenant in the habitat's project, the vault slots it
/// holds by name, never a value; never a checkout's path: an allocation is
/// the organism's, whichever of its checkouts migrated it) and
/// `habitat.migrated <id>/<role>` each time its schema version moves. The
/// rows are written into the habitat's record as soon as it exists: an
/// allocation made before the habitat is born (its own, at bootstrap, or
/// an organism's migrated first) waits in the cache and is written at
/// birth. `allocations/<id>/allocation.json` is that cache, derived from
/// the rows; `status` reads the rows when the habitat's memory is up.
/// Never for an operator's own server.
pub(super) fn record_allocation(root: &Path, role: &str, version: u64, slots: &str) -> Result<(), String> {
    let Some(dir) = allocation_dir(root) else { return Ok(()) };
    let id = manifest_pin(&root.join("hale.toml")).ok().flatten().unwrap_or_default();
    let path = dir.join("allocation.json");
    let mut row: serde_json::Value = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_else(|| serde_json::json!({}));
    row["organism"] = serde_json::json!(id);
    row["root"] = serde_json::json!(root.display().to_string());
    if !row["roles"].is_object() {
        row["roles"] = serde_json::json!({});
    }
    next_rows(&mut row, &id, role, version, slots, &local_habitat_project());
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    write_row(&path, &row)?;
    // into the habitat's record now, when it exists; else at its birth
    let _ = flush_pending();
    Ok(())
}

/// The cache `row` after the organism `id`'s `role` was migrated to
/// `version`: the rows the habitat's record is owed added to its pending
/// ones — `habitat.allocated` the first time, `habitat.migrated` whenever
/// the version moved — keyed by the organism and the role alone.
fn next_rows(row: &mut serde_json::Value, id: &str, role: &str, version: u64, slots: &str, project: &str) {
    let had = row["roles"][role]["schema_version"].as_u64();
    let recorded = row["roles"][role]["recorded"].as_bool().unwrap_or(false);
    let mut pending: Vec<serde_json::Value> = row["pending"].as_array().cloned().unwrap_or_default();
    let service = format!("{role}-{}", &id[..12]);
    let entity = format!("{}/{role}", &id[..12]);
    let slots: Vec<&str> = slots.split_whitespace().collect();
    if had.is_none() && !recorded {
        pending.push(serde_json::json!({ "kind": "habitat.allocated", "entity": entity, "body": serde_json::json!({ "organism": id, "role": role, "service": service, "project": project, "vault": slots }).to_string() }));
    }
    if had != Some(version) {
        pending.push(serde_json::json!({ "kind": "habitat.migrated", "entity": entity, "body": serde_json::json!({ "organism": id, "role": role, "from": had, "schema_version": version }).to_string() }));
    }
    row["roles"][role] = serde_json::json!({ "service": service, "schema_version": version, "vault": slots, "recorded": recorded || had.is_some() });
    row["pending"] = serde_json::json!(pending);
}

fn write_row(path: &Path, row: &serde_json::Value) -> Result<(), String> {
    let text = serde_json::to_string_pretty(row).map_err(|e| e.to_string())? + "\n";
    std::fs::write(path, text).map_err(|e| format!("write {}: {e}", path.display()))
}

/// The allocation rows every cache holds that the habitat's record does
/// not yet, written into it: what the habitat's birth does for the
/// allocations made before it (its own, at bootstrap), and every later
/// migration for its own. Nothing while the habitat is not born.
fn flush_pending() -> Result<usize, String> {
    let genome = genome_dir();
    if manifest_pin(&genome.join("hale.toml")).ok().flatten().is_none() || !genome.join(".hale/dna/memory.env").is_file() || !super::record_exists(&genome).unwrap_or(false) {
        return Ok(0);
    }
    let mut lines = String::new();
    let mut touched = Vec::new();
    for (key, row) in allocations() {
        let Some(p) = row["pending"].as_array() else { continue };
        if p.is_empty() {
            continue;
        }
        for r in p {
            lines.push_str(&r.to_string());
            lines.push('\n');
        }
        touched.push(key);
    }
    if lines.is_empty() {
        return Ok(0);
    }
    let file = PathBuf::from(local_habitat_dir()).join(".allocation-rows.jsonl");
    std::fs::write(&file, &lines).map_err(|e| format!("write {}: {e}", file.display()))?;
    let landed = host_run("record-seed", &genome, &[file.display().to_string(), String::new()]);
    let _ = std::fs::remove_file(&file);
    landed?;
    let base = PathBuf::from(local_habitat_dir()).join("allocations");
    for key in &touched {
        let path = base.join(key).join("allocation.json");
        let Some(mut row) = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) else { continue };
        row["pending"] = serde_json::json!([]);
        if let Some(roles) = row["roles"].as_object_mut() {
            for (_, r) in roles.iter_mut() {
                r["recorded"] = serde_json::json!(true);
            }
        }
        write_row(&path, &row)?;
    }
    Ok(lines.lines().count())
}

/// The allocations as the habitat's record states them: per `<id>/<role>`,
/// the organism, the service and the latest schema version, with the
/// checkout this machine last migrated it from beside it from the local
/// cache (never the record's);
/// `None` when the record cannot be read (the habitat not born, or its
/// memory not up), when the cache is all there is.
fn recorded_allocations() -> Option<Vec<(String, serde_json::Value)>> {
    let genome = genome_dir();
    manifest_pin(&genome.join("hale.toml")).ok().flatten()?;
    let rows = host_run("record-rows", &genome, &[]).ok()?;
    let mut by: std::collections::BTreeMap<String, serde_json::Value> = std::collections::BTreeMap::new();
    for line in rows.lines() {
        let Ok(r) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        let kind = r["kind"].as_str().unwrap_or("");
        if kind != "habitat.allocated" && kind != "habitat.migrated" {
            continue;
        }
        let Some((key, role)) = r["entity"].as_str().and_then(|e| e.split_once('/')) else { continue };
        let body: serde_json::Value = r["body"].as_str().and_then(|b| serde_json::from_str(b).ok()).unwrap_or_else(|| r["body"].clone());
        let row = by.entry(key.to_string()).or_insert_with(|| serde_json::json!({ "roles": {} }));
        if kind == "habitat.allocated" {
            row["organism"] = body["organism"].clone();
            row["roles"][role]["service"] = body["service"].clone();
            row["roles"][role]["vault"] = body["vault"].clone();
        } else {
            row["roles"][role]["schema_version"] = body["schema_version"].clone();
        }
    }
    let cache: std::collections::BTreeMap<String, serde_json::Value> = allocations().into_iter().collect();
    for (key, row) in by.iter_mut() {
        if let Some(root) = cache.get(key).and_then(|c| c["root"].as_str()) {
            row["root"] = serde_json::json!(root);
        }
    }
    Some(by.into_iter().collect())
}

/// Every allocation row in the habitat, by its directory's key.
fn allocations() -> Vec<(String, serde_json::Value)> {
    let dir = PathBuf::from(local_habitat_dir()).join("allocations");
    let mut out: Vec<(String, serde_json::Value)> = std::fs::read_dir(&dir)
        .map(|es| {
            es.flatten()
                .filter_map(|e| {
                    let key = e.file_name().to_string_lossy().into_owned();
                    let row = std::fs::read_to_string(e.path().join("allocation.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(serde_json::Value::Null);
                    Some((key, row))
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// The versions this toolchain migrates each role to (`memory`, `nerves`),
/// asked of the host at `root`.
fn toolchain_versions(root: &Path) -> Vec<(String, u64)> {
    host_run("schema-versions", root, &[])
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split_once('=').and_then(|(k, v)| v.trim().parse().ok().map(|v| (k.to_string(), v))))
        .collect()
}

/// The roles of `row` recorded behind `versions`, as `role a -> b`.
fn behind(row: &serde_json::Value, versions: &[(String, u64)]) -> Vec<String> {
    versions
        .iter()
        .filter_map(|(role, want)| {
            let had = row["roles"][role.as_str()]["schema_version"].as_u64()?;
            (had < *want).then(|| format!("{role} {had} -> {want}"))
        })
        .collect()
}

/// GH #1653: the habitat's upgrade of the organism at `root`: each of its
/// allocation's roles recorded behind this toolchain's version is migrated
/// (an operator's own servers move with `upgrade`'s owner's DSN instead).
/// What was done, a line each; nothing for an organism with no allocation.
pub(super) fn upgrade_allocation(root: &Path) -> Result<Vec<String>, String> {
    let Some(dir) = allocation_dir(root) else { return Ok(Vec::new()) };
    let Some(row) = std::fs::read_to_string(dir.join("allocation.json")).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) else { return Ok(Vec::new()) };
    let moved = behind(&row, &toolchain_versions(root));
    if moved.is_empty() {
        return Ok(Vec::new());
    }
    let mut out = vec![format!("habitat the allocation records versions behind this toolchain's ({}): migrating it", moved.join(", "))];
    out.extend(migrate_lines(root)?);
    Ok(out)
}

// ---------------------------------------------------------------
// up, status, down (GH #1653)
// ---------------------------------------------------------------

fn genome_dir() -> PathBuf {
    PathBuf::from(local_habitat_dir()).join("genome")
}

/// The systemd user unit the habitat's body runs as, named for the
/// habitat's project, so a test's habitat is never the developer's unit.
fn unit_name() -> String {
    format!("{}.service", local_habitat_project())
}

fn unit_path() -> PathBuf {
    let config = std::env::var("XDG_CONFIG_HOME").ok().filter(|v| !v.is_empty()).unwrap_or_else(|| format!("{}/.config", std::env::var("HOME").unwrap_or_default()));
    PathBuf::from(config).join("systemd/user").join(unit_name())
}

fn pid_file() -> PathBuf {
    PathBuf::from(local_habitat_dir()).join("body.pid")
}

/// The habitat's body as it runs: `unit`, `foreground <pid>`, or `None`.
fn body_state() -> Option<String> {
    if let Ok(pid) = std::fs::read_to_string(pid_file()) {
        if let Ok(pid) = pid.trim().parse::<i32>() {
            if pid > 0 && unsafe { libc::kill(pid, 0) } == 0 {
                return Some(format!("up (pid {pid}, in the foreground)"));
            }
        }
    }
    let active = Command::new("systemctl").args(["--user", "is-active", "--quiet"]).arg(unit_name()).status().is_ok_and(|s| s.success());
    active.then(|| format!("up (systemd user unit {})", unit_name()))
}

fn up(args: &[String]) -> ExitCode {
    let mut foreground = false;
    let mut profile = "local".to_string();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--foreground" => foreground = true,
            "--profile" if i + 1 < args.len() => {
                profile = args[i + 1].clone();
                i += 1;
            }
            other => {
                eprintln!("hale habitat up: `{other}` is not a flag of `up`\n{}", usage_text());
                return ExitCode::from(2);
            }
        }
        i += 1;
    }
    if profile != "local" {
        eprintln!("hale habitat up: the `{profile}` profile is not built: only `local` (docker compose on this machine) is; the hosted one runs on the backends a platform already runs (GH #1653)");
        return ExitCode::from(2);
    }
    // the habitat's memory and nerves are its own allocations: a server of
    // the operator's named here would migrate one place and run another
    if own_servers() {
        eprintln!("hale habitat up: the habitat runs on its own allocations; unset HALE_DNA_MEMORY_DSN_OWNER and HALE_DNA_NATS_URL_OWNER for `hale habitat up`");
        return ExitCode::from(2);
    }
    if let Some(state) = body_state() {
        if foreground {
            eprintln!("hale habitat up: the habitat's body is already {state}; `hale habitat down` stops it");
            return ExitCode::from(1);
        }
    }
    match bring_up() {
        Ok(lines) => {
            for l in lines {
                println!("{l}");
            }
        }
        Err(e) => {
            eprintln!("hale habitat up: {e}");
            return ExitCode::from(1);
        }
    }
    let genome = genome_dir();
    let me = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_else(|_| "hale".into());
    if foreground {
        let _ = std::fs::write(pid_file(), format!("{}\n", std::process::id()));
        println!("body    the habitat's body runs in this process (`hale dna dev {}`)", genome.display());
        use std::os::unix::process::CommandExt;
        let e = Command::new(&me).args(["dna", "dev"]).arg(&genome).arg("--no-iris").env("HALE_DNA_COMPOSE", "off").current_dir(&genome).exec();
        eprintln!("hale habitat up: cannot run {me}: {e}");
        return ExitCode::from(1);
    }
    if let Some(state) = body_state() {
        println!("body    {state}");
        return ExitCode::SUCCESS;
    }
    match start_unit(&me, &genome) {
        Ok(line) => {
            println!("{line}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("hale habitat up: {e}; `hale habitat up --foreground` runs the body in this process");
            ExitCode::from(1)
        }
    }
}

/// The habitat born the first time, its own memory and nerves migrated
/// (its schema is the habitat's to move, at bootstrap and at a toolchain's
/// upgrade), and every tenant's recorded allocation brought up. A tenant is
/// never migrated here: `up` runs under ongoing work, and an organism's
/// allocation moves at its upgrade or with `hale habitat migrate`.
fn bring_up() -> Result<Vec<String>, String> {
    let genome = genome_dir();
    let mut out = vec![format!("habitat local at {} (compose project {})", local_habitat_dir(), local_habitat_project())];
    if manifest_pin(&genome.join("hale.toml")).ok().flatten().is_none() {
        out.extend(birth(&genome)?);
    } else {
        out.extend(migrate_lines(&genome)?);
    }
    let own = allocation_dir(&genome);
    let versions = toolchain_versions(&genome);
    let rows = recorded_allocations().unwrap_or_else(allocations);
    for (key, row) in rows {
        let dir = PathBuf::from(local_habitat_dir()).join("allocations").join(&key);
        if Some(&dir) == own.as_ref() {
            continue;
        }
        if !dir.join("compose.yaml").is_file() || !dir.join("ports.yaml").is_file() {
            continue;
        }
        let up = Command::new("env")
            .args(["COMPOSE_IGNORE_ORPHANS=1", "docker", "compose", "-p"])
            .arg(local_habitat_project())
            .arg("--project-directory")
            .arg(&dir)
            .arg("-f")
            .arg(dir.join("compose.yaml"))
            .arg("-f")
            .arg(dir.join("ports.yaml"))
            .args(["up", "-d", "--wait"])
            .output()
            .map_err(|e| format!("docker compose: {e}"))?;
        if !up.status.success() {
            out.push(format!("note    allocation {key}: did not come up: {}", String::from_utf8_lossy(&up.stderr).trim()));
            continue;
        }
        let moved = behind(&row, &versions);
        let root = row["root"].as_str().unwrap_or("<organism>");
        out.push(format!("up      allocation {key} ({root}){}", if moved.is_empty() { String::new() } else { format!("; behind this toolchain ({}): `hale habitat migrate {root}` or `hale dna upgrade` migrates it", moved.join(", ")) }));
    }
    Ok(out)
}

/// GH #1653: the habitat's birth, the one with no parent: its genome
/// written from the toolchain's, made a repository, born as an organism
/// (its record in its own memory allocation) and given its positions and
/// its law as the Board's to ratify.
fn birth(genome: &Path) -> Result<Vec<String>, String> {
    std::fs::create_dir_all(genome).map_err(|e| format!("create {}: {e}", genome.display()))?;
    for f in hale_dna::HABITAT_FILES {
        let rel = f.path.strip_prefix("dna/habitat/").unwrap_or(f.path);
        let path = genome.join(rel);
        if !path.exists() {
            std::fs::write(&path, f.content).map_err(|e| format!("write {}: {e}", path.display()))?;
        }
    }
    let git = |args: &[&str]| Command::new("git").arg("-C").arg(genome).args(["-c", "user.name=habitat", "-c", "user.email=habitat@localhost"]).args(args).output().map_err(|e| e.to_string());
    if !genome.join(".git").exists() {
        git(&["init", "-q", "-b", "main"])?;
    }
    git(&["add", "-A"])?;
    let _ = git(&["commit", "-q", "-m", "the habitat's genome"])?;
    let mut out = vec![format!("born    the habitat at {} (the one organism with no parent)", genome.display())];
    out.extend(super::init(genome, false)?);
    // the bootstrap's allocations, the habitat's own memory and nerves,
    // made before its record existed: their rows, now it does
    let n = flush_pending()?;
    if n > 0 {
        out.push(format!("recorded {n} allocation row(s) in the habitat's record"));
    }
    // each pack is proposed by its own `hale dna pack apply`, a process of
    // its own: the verb hands itself over to the host (exec), and the
    // habitat's birth goes on after it
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    for (pack, args) in [("habitat", vec!["apply", "habitat", "--on-holes"]), ("standard", vec!["apply", "standard"])] {
        let mut a: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        a.push(genome.display().to_string());
        let ok = Command::new(&me).arg("dna").arg("pack").args(&a).status().is_ok_and(|s| s.success());
        if !ok {
            out.push(format!("note    the `{pack}` pack was not proposed to the habitat's Board: `hale dna pack {}` proposes it", a.join(" ")));
        }
    }
    out.push("proposed the habitat's positions (`habitat`) and its law (`standard`): `hale dna review all approve` ratifies them".to_string());
    Ok(out)
}

fn start_unit(me: &str, genome: &Path) -> Result<String, String> {
    let data = std::env::var("XDG_DATA_HOME").ok().filter(|v| !v.is_empty()).unwrap_or_else(|| format!("{}/.local/share", std::env::var("HOME").unwrap_or_default()));
    let vault = std::env::var("HALE_VAULT_DIR").ok().filter(|v| !v.is_empty()).map(|v| format!("Environment=HALE_VAULT_DIR={v}\n")).unwrap_or_default();
    let unit = format!(
        "[Unit]\nDescription=hale habitat (local) at {dir}\nAfter=network-online.target\n\n[Service]\nWorkingDirectory={g}\nEnvironment=XDG_DATA_HOME={data}\nEnvironment=HALE_DNA_COMPOSE=off\n{vault}ExecStart={me} dna dev {g} --no-iris\nRestart=on-failure\nRestartSec=5\n\n[Install]\nWantedBy=default.target\n",
        dir = local_habitat_dir(),
        g = genome.display()
    );
    let path = unit_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
    }
    std::fs::write(&path, unit).map_err(|e| format!("write {}: {e}", path.display()))?;
    let systemctl = |args: &[&str]| Command::new("systemctl").arg("--user").args(args).output().map_err(|e| format!("systemctl: {e}")).and_then(|o| if o.status.success() { Ok(()) } else { Err(format!("systemctl --user {}: {}", args.join(" "), String::from_utf8_lossy(&o.stderr).trim())) });
    systemctl(&["daemon-reload"])?;
    systemctl(&["enable", "--now", &unit_name()])?;
    Ok(format!("body    up (systemd user unit {}, {})", unit_name(), path.display()))
}

fn status() -> ExitCode {
    let genome = genome_dir();
    println!("habitat local at {} (compose project {})", local_habitat_dir(), local_habitat_project());
    match manifest_pin(&genome.join("hale.toml")).ok().flatten() {
        Some(id) => println!("genome  {} (record {}…)", genome.display(), &id[..12.min(id.len())]),
        None => println!("genome  not born: `hale habitat up` births it"),
    }
    println!("body    {}", body_state().unwrap_or_else(|| "down: `hale habitat up` starts it".to_string()));
    let (rows, from) = match recorded_allocations() {
        Some(rows) => (rows, "the habitat's record"),
        None => (allocations(), "the allocations' cache (the habitat's record is not readable: its memory is not up)"),
    };
    println!("allocations, as {from}:");
    if rows.is_empty() {
        println!("allocations: none");
        return ExitCode::SUCCESS;
    }
    let running = running_services();
    let versions = rows.iter().find_map(|(_, r)| r["root"].as_str().map(PathBuf::from).filter(|p| p.join("hale.toml").is_file())).map(|r| toolchain_versions(&r)).unwrap_or_default();
    for (key, row) in rows {
        println!("allocation {key} ({})", row["root"].as_str().unwrap_or("no organism recorded"));
        for role in ["memory", "nerves", "senses"] {
            let service = format!("{role}-{key}");
            let state = if running.iter().any(|s| s == &service) { "running" } else { "stopped" };
            let recorded = row["roles"][role]["schema_version"].as_u64();
            let want = versions.iter().find(|(r, _)| r == role).map(|(_, v)| *v);
            let version = match (recorded, want) {
                (Some(had), Some(want)) if had < want => format!(", schema {had}, behind this toolchain's {want}: `hale habitat migrate {}` (or `hale dna upgrade` there) migrates it", row["root"].as_str().unwrap_or("<organism>")),
                (Some(had), Some(_)) => format!(", schema {had} (this toolchain's)"),
                (Some(had), None) => format!(", schema {had}"),
                (None, _) => String::new(),
            };
            println!("  {role:<7} {service} {state}{version}");
        }
    }
    ExitCode::SUCCESS
}

/// The running services of the habitat's project.
fn running_services() -> Vec<String> {
    match docker(&["ps", "--filter", &format!("label=com.docker.compose.project={}", local_habitat_project()), "--filter", "status=running", "--format", "{{.Label \"com.docker.compose.service\"}}"]) {
        Docker::Answered(out) => out.lines().map(str::to_string).collect(),
        _ => Vec::new(),
    }
}

fn down() -> ExitCode {
    let mut said = Vec::new();
    if let Ok(pid) = std::fs::read_to_string(pid_file()) {
        if let Ok(pid) = pid.trim().parse::<i32>() {
            if pid > 0 && unsafe { libc::kill(pid, libc::SIGTERM) } == 0 {
                said.push(format!("stopped the habitat's body (pid {pid})"));
            }
        }
        let _ = std::fs::remove_file(pid_file());
    }
    if Command::new("systemctl").args(["--user", "is-active", "--quiet"]).arg(unit_name()).status().is_ok_and(|s| s.success()) {
        match Command::new("systemctl").args(["--user", "stop"]).arg(unit_name()).status() {
            Ok(s) if s.success() => said.push(format!("stopped the habitat's body (systemd user unit {})", unit_name())),
            _ => {
                eprintln!("hale habitat down: systemctl --user stop {} failed", unit_name());
                return ExitCode::from(1);
            }
        }
    }
    let ids = match docker(&["ps", "-q", "--filter", &format!("label=com.docker.compose.project={}", local_habitat_project())]) {
        Docker::Answered(out) => out.lines().map(str::to_string).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        Docker::Absent => Vec::new(),
        Docker::Failed(why) => {
            eprintln!("hale habitat down: {why}");
            return ExitCode::from(1);
        }
    };
    if !ids.is_empty() {
        let mut a = vec!["stop"];
        a.extend(ids.iter().map(String::as_str));
        if let Docker::Failed(why) = docker(&a) {
            eprintln!("hale habitat down: {why}");
            return ExitCode::from(1);
        }
        said.push(format!("stopped {} service(s) of {}", ids.len(), local_habitat_project()));
    }
    if said.is_empty() {
        println!("the habitat is already down");
    } else {
        for l in said {
            println!("{l}");
        }
        println!("its data is kept: `hale habitat up` brings it back");
    }
    ExitCode::SUCCESS
}

// ---------------------------------------------------------------
// the teardown of a test's habitat (GH #1653)
// ---------------------------------------------------------------

/// The file under a root naming the compose projects found rooted in it,
/// one per line, kept until their teardown is confirmed: a volume carries
/// its project's name but not the directory it ran from, so a project
/// whose containers went and whose volumes did not is found again by name.
/// `dna::scratch_compose_down` keeps the same file.
pub(crate) const COMPOSE_FILE: &str = ".dna-compose-projects";

/// What docker answered: its output, nothing to ask (no CLI or no daemon:
/// nothing compose started can be running there), or why it failed.
pub(crate) enum Docker {
    Answered(String),
    Absent,
    Failed(String),
}

fn docker(args: &[&str]) -> Docker {
    match Command::new("docker").args(args).output() {
        Err(_) => Docker::Absent,
        Ok(o) if o.status.success() => Docker::Answered(String::from_utf8_lossy(&o.stdout).into_owned()),
        Ok(o) => {
            let err = String::from_utf8_lossy(&o.stderr).into_owned();
            if err.contains("Cannot connect to the Docker daemon") || err.contains("Is the docker daemon running") {
                Docker::Absent
            } else {
                Docker::Failed(format!("docker {}: {}", args.join(" "), err.trim()))
            }
        }
    }
}

/// The compose project of the local habitat that lives at `dir`, as
/// `dna::habitat_project_at` names it.
pub(crate) fn habitat_project_at(dir: &str) -> String {
    let h = openssl::sha::sha256(dir.as_bytes());
    format!("hale-habitat-local-{}", h[..4].iter().map(|b| format!("{b:02x}")).collect::<String>())
}

/// The projects of docker's `listed` (`<project>|<working dir>` lines)
/// rooted in `root`, and the ones `recorded` names, as
/// `dna::scratch_projects_in` finds them: never the developer's own
/// habitat (under `$HOME`), nor the one this process's `XDG_DATA_HOME`
/// names when it lives outside `root`. The machine's long-lived habitat is
/// no teardown's, whatever a file under a root says.
pub(crate) fn projects_rooted_in(listed: &str, root: &Path, recorded: &str) -> Vec<String> {
    let text = root.display().to_string();
    let under = format!("{text}/");
    let home = std::env::var("HOME").unwrap_or_default();
    let mine = habitat_project_at(&format!("{home}/.local/share/hale/habitat/local"));
    let data = std::env::var("XDG_DATA_HOME").ok().filter(|v| !v.is_empty()).unwrap_or_else(|| format!("{home}/.local/share"));
    let here = format!("{data}/hale/habitat/local");
    let current = if here == text || here.starts_with(&under) { None } else { Some(habitat_project_at(&here)) };
    let mut projects: Vec<String> = listed
        .lines()
        .filter_map(|l| l.split_once('|'))
        .filter(|(p, wd)| !p.is_empty() && (*wd == text || wd.starts_with(&under)))
        .map(|(p, _)| p.to_string())
        .chain(recorded.lines().filter(|l| !l.is_empty()).map(str::to_string))
        .filter(|p| *p != mine && Some(p) != current.as_ref())
        .collect();
    projects.sort();
    projects.dedup();
    projects
}

/// Take down every compose project rooted in `root` — its containers,
/// networks and volumes (`docker compose -p <project> down -v`, which needs
/// no compose file) — and confirm none of it is left, by the project's
/// label on containers and on volumes. The projects are named in
/// `root`'s [`COMPOSE_FILE`] before anything is taken down and the file
/// goes only once nothing is left, so a teardown that did not finish (a
/// volume docker would not remove) is found again and retried by the next.
/// `Err` names what failed or survived; the caller keeps `root`. The one
/// teardown `hale test`, the CLI tests' reap and
/// `scripts/compose-down-under.sh` share (`hale habitat sweep`).
pub(crate) fn down_under(root: &Path) -> Result<(), String> {
    down_under_with(root, &mut |args| docker(args))
}

pub(crate) fn down_under_with(root: &Path, docker: &mut dyn FnMut(&[&str]) -> Docker) -> Result<(), String> {
    let file = root.join(COMPOSE_FILE);
    let recorded = std::fs::read_to_string(&file).unwrap_or_default();
    let listed = match docker(&["ps", "-a", "--filter", "label=com.docker.compose.project", "--format", "{{.Label \"com.docker.compose.project\"}}|{{.Label \"com.docker.compose.project.working_dir\"}}"]) {
        Docker::Absent => return Ok(()),
        Docker::Failed(why) => return Err(why),
        Docker::Answered(out) => out,
    };
    let projects = projects_rooted_in(&listed, root, &recorded);
    if projects.is_empty() {
        let _ = std::fs::remove_file(&file);
        return Ok(());
    }
    let _ = std::fs::create_dir_all(root);
    std::fs::write(&file, projects.iter().map(|p| format!("{p}\n")).collect::<String>()).map_err(|e| format!("write {}: {e}", file.display()))?;
    let mut left = Vec::new();
    for project in &projects {
        if let Docker::Failed(why) = docker(&["compose", "-p", project, "down", "-v", "--remove-orphans"]) {
            left.push(why);
        }
        let label = format!("label=com.docker.compose.project={project}");
        for (what, args) in [("containers", ["ps", "-a", "-q", "--filter", label.as_str()].as_slice()), ("volumes", ["volume", "ls", "-q", "--filter", label.as_str()].as_slice())] {
            match docker(args) {
                Docker::Answered(out) if !out.trim().is_empty() => left.push(format!("compose project {project} still has {what} after its teardown")),
                Docker::Failed(why) => left.push(why),
                _ => {}
            }
        }
    }
    if left.is_empty() {
        let _ = std::fs::remove_file(&file);
        Ok(())
    } else {
        Err(left.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// GH #1653: an allocation's rows are the organism's and the role's,
    /// never a checkout's: allocated once, migrated whenever the version
    /// moves, nothing when it does not; and a recorded version behind the
    /// toolchain's is named.
    #[test]
    fn allocation_rows_follow_the_version() {
        let id = "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
        let mut row = serde_json::json!({ "root": "/a/checkout" });
        next_rows(&mut row, id, "memory", 9, "postgres-owner-1234567890ab", "hale-habitat-local-x");
        let kinds: Vec<&str> = row["pending"].as_array().unwrap().iter().map(|r| r["kind"].as_str().unwrap()).collect();
        assert_eq!(kinds, ["habitat.allocated", "habitat.migrated"]);
        let allocated = row["pending"][0]["body"].as_str().unwrap();
        assert!(allocated.contains("\"organism\":\"1234567890abcdef") && allocated.contains("\"vault\":[\"postgres-owner-1234567890ab\"]") && !allocated.contains("checkout"), "{allocated}");
        assert_eq!(row["pending"][0]["entity"], "1234567890ab/memory");
        // flushed, then migrated again at the same version: nothing owed
        row["pending"] = serde_json::json!([]);
        row["roles"]["memory"]["recorded"] = serde_json::json!(true);
        next_rows(&mut row, id, "memory", 9, "", "p");
        assert!(row["pending"].as_array().unwrap().is_empty());
        // the version moves: one migrated row, from the old version
        next_rows(&mut row, id, "memory", 10, "", "p");
        let moved = &row["pending"][0];
        assert_eq!(moved["kind"], "habitat.migrated");
        assert!(moved["body"].as_str().unwrap().contains("\"from\":9") && moved["body"].as_str().unwrap().contains("\"schema_version\":10"));
        // behind: the recorded version against the toolchain's
        row["roles"]["nerves"] = serde_json::json!({ "schema_version": 1 });
        assert_eq!(behind(&row, &[("memory".into(), 11), ("nerves".into(), 1)]), vec!["memory 10 -> 11".to_string()]);
    }

    fn root(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("hale_habitat_sweep_{}_{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A teardown whose volume did not go keeps the project named under the
    /// root and fails; the next finds it by that name alone (its containers
    /// are gone, so no label points at the root any more), confirms it gone
    /// and only then lets the name go.
    #[test]
    fn a_volume_that_survives_is_retried_by_name() {
        let d = root("retry");
        let p = habitat_project_at(&format!("{}/data/hale/habitat/local", d.display()));
        let listing = format!("{p}|{}/data/hale/habitat/local/allocations/abc\n", d.display());
        let mut volume_left = true;
        let mut first = |args: &[&str]| match args {
            ["ps", "-a", "--filter", ..] => Docker::Answered(listing.clone()),
            ["compose", ..] => Docker::Answered(String::new()),
            ["ps", "-a", "-q", ..] => Docker::Answered(String::new()),
            ["volume", ..] => Docker::Answered(if volume_left { format!("{p}_memory-abc\n") } else { String::new() }),
            _ => Docker::Failed("unexpected".into()),
        };
        let err = down_under_with(&d, &mut first).unwrap_err();
        assert!(err.contains("still has volumes"), "{err}");
        assert_eq!(std::fs::read_to_string(d.join(COMPOSE_FILE)).unwrap(), format!("{p}\n"), "the project stays named");
        // the containers went: nothing labels the root any more, and the
        // volume is still there, so the next teardown fails again, by name
        let mut downs = Vec::new();
        let mut again = |args: &[&str]| match args {
            ["ps", "-a", "--filter", ..] => Docker::Answered(String::new()),
            ["compose", "-p", project, ..] => {
                downs.push(project.to_string());
                Docker::Answered(String::new())
            }
            ["ps", "-a", "-q", ..] => Docker::Answered(String::new()),
            ["volume", ..] => Docker::Answered(format!("{p}_memory-abc\n")),
            _ => Docker::Failed("unexpected".into()),
        };
        assert!(down_under_with(&d, &mut again).is_err());
        assert_eq!(downs, vec![p.clone()], "the recorded project is taken down again");
        assert!(d.join(COMPOSE_FILE).exists());
        // once the volume goes, the name goes
        volume_left = false;
        let mut done = |args: &[&str]| match args {
            ["ps", "-a", "--filter", ..] => Docker::Answered(String::new()),
            ["volume", ..] => Docker::Answered(if volume_left { "x\n".into() } else { String::new() }),
            _ => Docker::Answered(String::new()),
        };
        down_under_with(&d, &mut done).unwrap();
        assert!(!d.join(COMPOSE_FILE).exists(), "confirmed gone, the record goes");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The developer's habitat is never a teardown's, even named in a
    /// root's file.
    #[test]
    fn the_developers_habitat_is_never_taken_down() {
        let d = root("developer");
        let home = std::env::var("HOME").unwrap_or_default();
        let developers = habitat_project_at(&format!("{home}/.local/share/hale/habitat/local"));
        std::fs::write(d.join(COMPOSE_FILE), format!("{developers}\n")).unwrap();
        let mut touched = false;
        let mut dock = |args: &[&str]| {
            if args[0] == "compose" {
                touched = true;
            }
            Docker::Answered(if args[0] == "ps" && args.len() > 3 && args[2] == "--filter" && args[3] == "label=com.docker.compose.project" { format!("{developers}|{home}/.local/share/hale/habitat/local\n") } else { String::new() })
        };
        down_under_with(&d, &mut dock).unwrap();
        assert!(!touched, "nothing was taken down");
        let _ = std::fs::remove_dir_all(&d);
    }
}

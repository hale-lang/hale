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
    r#"usage: hale habitat up [--foreground | --no-body] [--profile local] [--backends own|operator]
       hale habitat graft <dir> [--attach] [--no-library] [--no-pack]
                          [--profile local|remote-body [--remote <url>] [--body <user@host>]]
                          [--with <pack> [--set <param>=<value>]… [--as <who>]]
       hale habitat handover [organism]
       hale habitat status
       hale habitat down
       hale habitat migrate [organism]
       hale habitat sweep <root> | --test-vaults [<root>]

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
                     then bring every recorded allocation up through the
                     compose provider, migrating any whose recorded schema
                     version is behind this toolchain's; record its
                     placement (the host `local` with what it offers, the
                     profile's policy, its head and face on that host, rows
                     of its record); and start the habitat's body: a systemd
                     user unit named for the habitat, or, with
                     --foreground, in this process, and its face (the
                     standard face over its own head; `status` prints its
                     address); --no-body births it and
                     brings its allocations up and starts no body (what a
                     test's habitat needs to graft into). Idempotent. Its
                     backends are chosen at its birth and kept: `own` (the
                     default) provisions memory, the nerves and the senses'
                     store as its project's allocations; `operator` adopts
                     the Postgres and the NATS HALE_DNA_MEMORY_DSN_OWNER and
                     HALE_DNA_NATS_URL_OWNER name, and provisions nothing
                     of its own. The hosted profile is not built.
graft <dir>          birth an organism as the habitat's child, the one way an
                     organism other than the habitat is born: its record's
                     genesis names the habitat's record as its parent, the
                     roles its genome needs (`[dna] needs` in its hale.toml:
                     memory and nerves always, senses unless left out;
                     undeclared, as a foreign heart is, it is exactly the
                     organism's own, never a heart's; a Hale heart may
                     declare `database`, a tenant database of its memory
                     server handed to it as HALE_DNA_DATABASE_DSN) are
                     allocated to it and recorded in the habitat's record,
                     and the `standard` pack is proposed to its Board
                     (--no-pack leaves it for `hale dna pack apply`).
                     <dir> is a new application's directory, or with
                     --attach an application (its workspace's manifest is
                     the organism's) or a repository that is there
                     already. A graft that stopped part way goes on
                     where it stopped when run again; a record born of
                     anything but this habitat is refused. A new
                     application takes --profile remote-body --remote
                     <url> [--body <user@host>]: the record shared over
                     that remote, the body on that server. --with <pack>
                     proposes a pack from the toolchain's nursery as
                     well, planned on the holes the birth proposed (`hale
                     dna pack apply <pack> --on-holes`, its --set and
                     --as passed on), so one `review all approve`
                     ratifies the starting organization whole.
handover [organism]  what the habitat hands the organism's heart, one
                     NAME=value a line, a secret only by its vault slot's
                     name: HALE_DNA_APPLICATION_ID, HALE_DNA_HEAD_URL,
                     HALE_DNA_OIDC_ISSUER, HALE_DNA_OIDC_CLIENT and
                     HALE_DNA_OIDC_CLIENT_VAULT, and for each heart role
                     its genome declares, HALE_DNA_DATABASE_DSN or
                     HALE_DNA_BROKER_URL, _USER, _VAULT, _SUBJECTS,
                     _NODE_USER and _NODE_VAULT. `hale dna dev` hands the
                     heart the same.
status               the habitat, its genome and body, and each
                     allocation: its organism, whether each role's service
                     runs, the schema version memory and the nerves record
                     against this toolchain's, and how a heart's declared
                     migration last ran.
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
                     --test-vaults sweeps the test vaults of processes
                     that are gone (<tmp>/hale-test-vaults-<uid>, or
                     <root>): each one's habitat comes down before the
                     vault goes, and a habitat whose vault went first is
                     found by its compose working directory.

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
        Some("graft") => graft(&args[1..]),
        Some("handover") => {
            let dir = args.get(1).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
            match project(&dir).and_then(|(root, _)| handover(&root, true)) {
                Ok(lines) => {
                    for (k, v) in lines {
                        println!("{k}={v}");
                    }
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("hale habitat handover: {e}");
                    ExitCode::from(1)
                }
            }
        }
        Some("status") if args.len() == 1 => status(),
        Some("down") if args.len() == 1 => down(),
        Some("sweep") if args.get(1).map(String::as_str) == Some("--test-vaults") && args.len() <= 3 => {
            let root = args.get(2).map(PathBuf::from).unwrap_or_else(crate::verbs::test::test_vault_root);
            match crate::verbs::test::sweep_dead_test_vaults(&root) {
                Ok(()) => ExitCode::SUCCESS,
                Err(why) => {
                    eprintln!("hale habitat sweep: {why}");
                    ExitCode::from(1)
                }
            }
        }
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
    // the heart's roles, when the genome declares them
    if needs_at(root).iter().any(|r| r == "database") {
        out.push(provision_database(root, if own_servers() { "operator" } else { "own" })?);
    }
    if needs_at(root).iter().any(|r| r == "broker") {
        out.push(provision_broker(root)?);
    }
    out.extend(run_heart_migrations(root)?);
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
/// the rows; `status` reads the rows when the habitat's memory is up. An
/// allocation on an operator's own server (the habitat's `operator`
/// backends) is recorded the same way, its service `operator`.
pub(super) fn record_allocation(root: &Path, role: &str, version: u64, slots: &str, operator: bool) -> Result<(), String> {
    let Some(dir) = allocation_dir(root) else { return Ok(()) };
    let id = manifest_pin(&root.join("hale.toml")).ok().flatten().unwrap_or_default();
    let path = dir.join("allocation.json");
    let mut row: serde_json::Value = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_else(|| serde_json::json!({}));
    row["organism"] = serde_json::json!(id);
    row["root"] = serde_json::json!(root.display().to_string());
    if !row["roles"].is_object() {
        row["roles"] = serde_json::json!({});
    }
    next_rows(&mut row, &id, role, version, slots, &local_habitat_project(), operator);
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    write_row(&path, &row)?;
    // into the habitat's record now, when it exists; else at its birth
    let _ = flush_pending();
    Ok(())
}

/// The cache `row` after the organism `id`'s `role` was migrated to
/// `version`: the rows the habitat's record is owed added to its pending
/// ones — `habitat.allocated` the first time, `habitat.migrated` whenever
/// the version moved — keyed by the organism and the role alone. A heart
/// role has no version of the habitat's: its schema is the heart's (a
/// declared migration runs the heart's own tool, `run_heart_migrations`),
/// so its allocation is recorded once and carries none.
fn next_rows(row: &mut serde_json::Value, id: &str, role: &str, version: u64, slots: &str, project: &str, operator: bool) {
    let heart = PROVISIONED_HEART_ROLES.contains(&role);
    let had = if heart { row["roles"][role].is_object().then_some(version) } else { row["roles"][role]["schema_version"].as_u64() };
    let recorded = row["roles"][role]["recorded"].as_bool().unwrap_or(false);
    let mut pending: Vec<serde_json::Value> = row["pending"].as_array().cloned().unwrap_or_default();
    // an allocation on the operator's own server is no service of the
    // habitat's project: it is named `operator`
    let service = if operator { "operator".to_string() } else { format!("{role}-{}", &id[..12]) };
    let entity = format!("{}/{role}", &id[..12]);
    let slots: Vec<&str> = slots.split_whitespace().collect();
    if had.is_none() && !recorded {
        // every allocation names its provider and its host: the `compose`
        // provider's on the host `local`, as the `local` profile's policy
        // gives each role (`habitat.policy`), or the operator's own server,
        // adopted, on no host of the habitat's
        let (provider, host) = if operator { ("operator", "") } else { ("compose", "local") };
        pending.push(serde_json::json!({ "kind": "habitat.allocated", "entity": entity, "body": serde_json::json!({ "organism": id, "role": role, "service": service, "project": project, "provider": provider, "host": host, "vault": slots }).to_string() }));
    }
    if !heart && had != Some(version) {
        pending.push(serde_json::json!({ "kind": "habitat.migrated", "entity": entity, "body": serde_json::json!({ "organism": id, "role": role, "from": had, "schema_version": version }).to_string() }));
    }
    // a migration that names no slots keeps the ones the allocation holds
    let kept: Vec<serde_json::Value> = row["roles"][role]["vault"].as_array().cloned().unwrap_or_default();
    let vault = if slots.is_empty() { serde_json::json!(kept) } else { serde_json::json!(slots) };
    row["roles"][role] = if heart {
        serde_json::json!({ "service": service, "vault": vault, "recorded": recorded || had.is_some() })
    } else {
        serde_json::json!({ "service": service, "schema_version": version, "vault": vault, "recorded": recorded || had.is_some() })
    };
    row["pending"] = serde_json::json!(pending);
}

/// One run of a heart's declared migration, a `habitat.migrated
/// <id>/<role>` row owed to the habitat's record: the command, its exit
/// status and its output (its last bytes, bounded), whatever it did.
fn record_migration_run(root: &Path, role: &str, command: &[String], status: &str, output: &str) -> Result<(), String> {
    let Some(dir) = allocation_dir(root) else { return Ok(()) };
    let id = manifest_pin(&root.join("hale.toml")).ok().flatten().unwrap_or_default();
    if id.len() < 12 {
        return Ok(());
    }
    let path = dir.join("allocation.json");
    let mut row: serde_json::Value = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_else(|| serde_json::json!({}));
    let mut pending: Vec<serde_json::Value> = row["pending"].as_array().cloned().unwrap_or_default();
    const KEPT: usize = 4096;
    let tail = if output.len() > KEPT { let mut at = output.len() - KEPT; while !output.is_char_boundary(at) { at += 1; } &output[at..] } else { output };
    pending.push(serde_json::json!({ "kind": "habitat.migrated", "entity": format!("{}/{role}", &id[..12]), "body": serde_json::json!({ "organism": id, "role": role, "command": command, "status": status, "output": tail }).to_string() }));
    row["pending"] = serde_json::json!(pending);
    if row["roles"][role].is_object() {
        row["roles"][role]["migration"] = serde_json::json!(status);
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    write_row(&path, &row)?;
    let _ = flush_pending();
    Ok(())
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
        } else if body["status"].is_string() {
            // a heart's migration run: its status, never a version
            row["roles"][role]["migration"] = body["status"].clone();
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
    // a heart's declared migrations run at every upgrade: its tool keeps
    // its own ledger and decides whether anything moved
    if moved.is_empty() {
        return run_heart_migrations(root);
    }
    let mut out = vec![format!("habitat the allocation records versions behind this toolchain's ({}): migrating it", moved.join(", "))];
    out.extend(migrate_lines(root)?);
    Ok(out)
}

/// GH #1653: a heart's own migration for a heart role it declares,
/// `[dna.migrations.<role>] command = [...]` in its genome's manifest.
/// Declaring one is the heart's choice: the habitat then runs it, with that
/// role's handover in its environment, at graft, by `hale habitat migrate
/// <organism>` and by `hale dna upgrade`, every time (the heart's tool is
/// idempotent and keeps its own ledger; the habitat keeps no version of
/// it). A heart that declares none owns its schema however it likes.
pub(super) struct HeartMigration {
    pub(super) role: String,
    command: Vec<String>,
}

/// The heart migrations the genome at `root` declares, each for a heart
/// role it needs; refused by name when one is malformed, names a key
/// other than `command`, or is for a role the genome does not declare.
pub(super) fn heart_migrations(root: &Path) -> Result<Vec<HeartMigration>, String> {
    let Ok(text) = std::fs::read_to_string(root.join("hale.toml")) else { return Ok(Vec::new()) };
    let table: toml::Table = text.parse().map_err(|e| format!("hale.toml does not parse: {e}"))?;
    let Some(m) = table.get("dna").and_then(|d| d.get("migrations")) else { return Ok(Vec::new()) };
    let m = m.as_table().ok_or("[dna.migrations] is not a table of heart roles")?;
    let needs = needs_at(root);
    let mut out = Vec::new();
    for (role, spec) in m {
        if !PROVISIONED_HEART_ROLES.contains(&role.as_str()) || !needs.iter().any(|r| r == role) {
            return Err(format!("[dna.migrations.{role}]: a heart migration is for a heart role the genome needs ({}), and `{role}` is not one", PROVISIONED_HEART_ROLES.join(", ")));
        }
        let spec = spec.as_table().ok_or_else(|| format!("[dna.migrations.{role}] is not a table"))?;
        if let Some(other) = spec.keys().find(|k| *k != "command") {
            return Err(format!("[dna.migrations.{role}]: `{other}` is not a key of a heart migration; it is its `command` alone, run at every graft, `hale habitat migrate` and `hale dna upgrade` (the tool keeps its own ledger)"));
        }
        let command: Vec<String> = spec.get("command").and_then(|c| c.as_array()).map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
        if command.is_empty() {
            return Err(format!("[dna.migrations.{role}] needs `command`, the argv that migrates it"));
        }
        out.push(HeartMigration { role: role.clone(), command });
    }
    Ok(out)
}

/// Each declared heart migration run, with the role's handover in its
/// environment, in the organism's directory; every run is a
/// `habitat.migrated` row with the command's exit status and output, and
/// one that fails fails the act (the next act runs it again). What was
/// done, a line each.
pub(super) fn run_heart_migrations(root: &Path) -> Result<Vec<String>, String> {
    let migrations = heart_migrations(root)?;
    if migrations.is_empty() {
        return Ok(Vec::new());
    }
    // a migration is a habitat act: the role's allocation is brought up
    // (and waited for) as `hale habitat migrate` brings it up, never
    // assumed up; an upgrade that just restarted the nerves for new
    // passwords waits for them here
    let handed = handover(root, false)?;
    let mut out = Vec::new();
    for m in migrations {
        let mut cmd = Command::new(&m.command[0]);
        cmd.args(&m.command[1..]).current_dir(root);
        for (k, v) in &handed {
            cmd.env(k, v);
        }
        let shown = m.command.join(" ");
        let ran = match cmd.output() {
            Ok(o) => o,
            Err(e) => {
                record_migration_run(root, &m.role, &m.command, "not run", &e.to_string())?;
                return Err(format!("the heart's {} migration (`{shown}`) could not run: {e}", m.role));
            }
        };
        let output = format!("{}{}", String::from_utf8_lossy(&ran.stdout), String::from_utf8_lossy(&ran.stderr));
        let status = ran.status.code().map(|c| format!("exit {c}")).unwrap_or_else(|| ran.status.to_string());
        record_migration_run(root, &m.role, &m.command, &status, &output)?;
        if !ran.status.success() {
            return Err(format!("the heart's {} migration (`{shown}`) failed ({status}): {}", m.role, String::from_utf8_lossy(&ran.stderr).trim()));
        }
        out.push(format!("{:<8}the heart's {} migration ran (`{shown}`, {status})", "heart", m.role));
    }
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

fn backends_file() -> PathBuf {
    PathBuf::from(local_habitat_dir()).join("backends")
}

/// The backends this run of `up` uses: the ones the habitat was born on
/// (kept beside it, and as its `habitat.profile` row), or for a habitat not
/// yet born the ones asked (`own` by default); refused when the asking or
/// the environment disagrees with them.
fn backends_of(asked: Option<&str>) -> Result<String, String> {
    if let Some(a) = asked {
        if a != "own" && a != "operator" {
            return Err(format!("`--backends {a}`: the backends are `own` (the habitat's allocations under docker compose) or `operator` (the Postgres and NATS HALE_DNA_MEMORY_DSN_OWNER and HALE_DNA_NATS_URL_OWNER name)"));
        }
    }
    let born = std::fs::read_to_string(backends_file()).ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
    let backends = match (&born, asked) {
        (Some(b), Some(a)) if b != a => return Err(format!("the habitat was born on `{b}` backends; they are chosen at its birth, not at `up`")),
        (Some(b), _) => b.clone(),
        (None, Some(a)) => a.to_string(),
        (None, None) => "own".to_string(),
    };
    let named = |k: &str| std::env::var(k).is_ok_and(|v| !v.is_empty());
    if backends == "own" && own_servers() {
        return Err("the habitat runs on its own allocations; unset HALE_DNA_MEMORY_DSN_OWNER and HALE_DNA_NATS_URL_OWNER, or birth it with `--backends operator` to adopt them".to_string());
    }
    if backends == "operator" && !(named("HALE_DNA_MEMORY_DSN_OWNER") && named("HALE_DNA_NATS_URL_OWNER")) {
        return Err("the habitat's backends are the operator's: name them in HALE_DNA_MEMORY_DSN_OWNER and HALE_DNA_NATS_URL_OWNER".to_string());
    }
    Ok(backends)
}

fn up(args: &[String]) -> ExitCode {
    let mut foreground = false;
    let mut no_body = false;
    let mut profile = "local".to_string();
    let mut asked: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--foreground" => foreground = true,
            "--no-body" => no_body = true,
            "--backends" if i + 1 < args.len() => {
                asked = Some(args[i + 1].clone());
                i += 1;
            }
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
    // GH #1653: the habitat's backends are chosen at its birth and kept:
    // `own`, allocations of its own project under docker compose, or
    // `operator`, the Postgres and the NATS an operator names
    // (HALE_DNA_MEMORY_DSN_OWNER, HALE_DNA_NATS_URL_OWNER), adopted rather
    // than provisioned. Either way the environment must agree with it: a
    // server named for an `own` habitat would migrate one place and run
    // another, and an `operator` habitat with none named has no backends
    let backends = match backends_of(asked.as_deref()) {
        Ok(b) => b,
        Err(why) => {
            eprintln!("hale habitat up: {why}");
            return ExitCode::from(2);
        }
    };
    if let Some(state) = body_state() {
        if foreground {
            eprintln!("hale habitat up: the habitat's body is already {state}; `hale habitat down` stops it");
            return ExitCode::from(1);
        }
    }
    match bring_up(&backends) {
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
    // GH #1653: born and its allocations up, its body not started: what a
    // test's habitat needs to graft into
    if no_body {
        if let Err(e) = place(&genome, &backends) {
            eprintln!("hale habitat up: {e}");
            return ExitCode::from(1);
        }
        println!("{:<8}not started (--no-body): `hale habitat up` starts it", "body");
        return ExitCode::SUCCESS;
    }
    if foreground {
        match start_face(&me, &genome, true) {
            Ok(line) => println!("{line}"),
            Err(e) => eprintln!("hale habitat up: the habitat's face did not start: {e}"),
        }
        if let Err(e) = place(&genome, &backends) {
            eprintln!("hale habitat up: {e}");
            return ExitCode::from(1);
        }
        let _ = std::fs::write(pid_file(), format!("{}\n", std::process::id()));
        println!("{:<8}the habitat's body runs in this process (`hale dna dev {}`)", "body", genome.display());
        use std::os::unix::process::CommandExt;
        let e = Command::new(&me).args(["dna", "dev"]).arg(&genome).arg("--no-iris").env("HALE_DNA_COMPOSE", "off").current_dir(&genome).exec();
        eprintln!("hale habitat up: cannot run {me}: {e}");
        return ExitCode::from(1);
    }
    match start_face(&me, &genome, false) {
        Ok(line) => println!("{line}"),
        Err(e) => eprintln!("hale habitat up: the habitat's face did not start: {e}"),
    }
    if let Err(e) = place(&genome, &backends) {
        eprintln!("hale habitat up: {e}");
        return ExitCode::from(1);
    }
    if let Some(state) = body_state() {
        println!("{:<8}{state}", "body");
        return ExitCode::SUCCESS;
    }
    match start_unit(&me, &genome, &backends) {
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

/// The habitat's placement, through its providers (phase 4): the host
/// the `compose` provider produces (`local`, with what it offers), the
/// `local` profile's policy (a provider per role, on these backends) and
/// the habitat's own head and face placed on that host, each a row of its
/// record written when the record does not already say it (`habitat-place`,
/// dna/host/placement.hl). What it recorded, a line each, printed.
fn place(genome: &Path, backends: &str) -> Result<(), String> {
    let said = host_run("habitat-place", genome, &[backends.to_string()]).map_err(|e| format!("the habitat's placement was not recorded: {e}"))?;
    print!("{said}");
    Ok(())
}

/// The habitat born the first time, its own memory and nerves migrated
/// (its schema is the habitat's to move, at bootstrap and at a toolchain's
/// upgrade), and every tenant's recorded allocation brought up. A tenant is
/// never migrated here: `up` runs under ongoing work, and an organism's
/// allocation moves at its upgrade or with `hale habitat migrate`.
fn bring_up(backends: &str) -> Result<Vec<String>, String> {
    let genome = genome_dir();
    let mut out = vec![format!("habitat local at {} (compose project {}; {backends} backends)", local_habitat_dir(), local_habitat_project())];
    // born whole once its identity is pinned and no birth is pending: a
    // birth that stopped part way left its steps in `birth.pending`, and is
    // resumed from them (memory that is merely down is never mistaken for
    // a birth that did not happen)
    if manifest_pin(&genome.join("hale.toml")).ok().flatten().is_some() && !birth_pending().exists() {
        out.extend(migrate_lines(&genome)?);
    } else {
        std::fs::create_dir_all(local_habitat_dir()).map_err(|e| format!("create {}: {e}", local_habitat_dir()))?;
        if !backends_file().exists() {
            std::fs::write(backends_file(), format!("{backends}\n")).map_err(|e| format!("write {}: {e}", backends_file().display()))?;
        }
        out.extend(birth(&genome, backends)?);
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
        // the tenant's allocations are the compose provider's to apply
        // (`habitat-apply`, dna/host/placement.hl)
        if let Err(why) = host_run("habitat-apply", &genome, &[dir.display().to_string()]) {
            out.push(format!("{:<8}allocation {key}: did not come up: {why}", "note"));
            continue;
        }
        let moved = behind(&row, &versions);
        let root = row["root"].as_str().unwrap_or("<organism>");
        out.push(format!("{:<8}allocation {key} ({root}){}", "up", if moved.is_empty() { String::new() } else { format!("; behind this toolchain ({}): `hale habitat migrate {root}` or `hale dna upgrade` migrates it", moved.join(", ")) }));
    }
    Ok(out)
}

/// The steps of a birth not yet whole, one a line as each is done
/// (`genesis`, `profile`, `habitat`, `standard`): written when a birth
/// begins and removed when it is whole.
fn birth_pending() -> PathBuf {
    PathBuf::from(local_habitat_dir()).join("birth.pending")
}

fn birth_done(step: &str) -> Result<(), String> {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new().append(true).create(true).open(birth_pending()).map_err(|e| format!("write {}: {e}", birth_pending().display()))?;
    writeln!(f, "{step}").map_err(|e| format!("write {}: {e}", birth_pending().display()))
}

/// GH #1653: the habitat's birth, the one with no parent: its genome
/// written from the toolchain's, made a repository, born as an organism
/// (its record in its own memory allocation) and given its positions and
/// its law as the Board's to ratify. Each step is taken only when it is
/// not done already, so a birth that stopped part way (memory or docker
/// failing after the identity was pinned) is resumed by the next `up`: a
/// pin whose record never reached memory is a birth that did not happen,
/// and is drawn again.
fn birth(genome: &Path, backends: &str) -> Result<Vec<String>, String> {
    std::fs::create_dir_all(genome).map_err(|e| format!("create {}: {e}", genome.display()))?;
    for f in hale_dna::HABITAT_FILES {
        let rel = f.path.strip_prefix("dna/habitat/").unwrap_or(f.path);
        let path = genome.join(rel);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
        }
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
    let mut out = Vec::new();
    if !birth_pending().exists() {
        std::fs::write(birth_pending(), "").map_err(|e| format!("write {}: {e}", birth_pending().display()))?;
    }
    let done = std::fs::read_to_string(birth_pending()).unwrap_or_default();
    let have: Vec<&str> = done.lines().collect();
    // whether the record was born is memory's to say, never a local
    // marker's: a genesis memory holds is resumed, never drawn again
    let manifest = genome.join("hale.toml");
    match genesis_state(genome)? {
        Genesis::Unpinned => {
            out.push(format!("{:<8}the habitat at {} (the one organism with no parent)", "born", genome.display()));
            out.extend(super::init_under(genome, false, super::Lineage::Habitat)?);
        }
        Genesis::Present => {
            // born: the rest of its init, each step of which keeps what
            // is done, against the record memory holds
            out.push(format!("{:<8}the habitat's record is in memory; its birth goes on where it stopped", "resume"));
            out.extend(super::init_under(genome, false, super::Lineage::Habitat)?);
        }
        Genesis::Never(drawn) => {
            // memory answered that this identity's record was never born
            // (or its memory was never provisioned): the identity is drawn
            // again, and what was made for it, holding no record, goes
            forget_allocation(&drawn[..12.min(drawn.len())]);
            unpin(&manifest)?;
            out.push(format!("{:<8}the habitat's earlier birth stopped before its record reached memory: born again", "resume"));
            out.push(format!("{:<8}the habitat at {} (the one organism with no parent)", "born", genome.display()));
            out.extend(super::init_under(genome, false, super::Lineage::Habitat)?);
        }
    }
    // the bootstrap's allocations, the habitat's own memory and nerves,
    // made before its record existed: their rows, now it does
    let n = flush_pending()?;
    if n > 0 {
        out.push(format!("recorded {n} allocation row(s) in the habitat's record"));
    }
    if !have.contains(&"profile") {
        // its profile: the backends it was born on, a row as every
        // provisioning choice is (never a DSN or a URL)
        let profile = PathBuf::from(local_habitat_dir()).join(".profile-row.jsonl");
        let row = serde_json::json!({ "kind": "habitat.profile", "entity": "local", "body": serde_json::json!({ "profile": "local", "backends": backends, "project": local_habitat_project() }).to_string() });
        std::fs::write(&profile, format!("{row}\n")).map_err(|e| format!("write {}: {e}", profile.display()))?;
        let landed = host_run("record-seed", genome, &[profile.display().to_string(), String::new()]);
        let _ = std::fs::remove_file(&profile);
        landed?;
        birth_done("profile")?;
    }
    // each pack is proposed by its own `hale dna pack apply`, a process of
    // its own: the verb hands itself over to the host (exec), and the
    // habitat's birth goes on after it
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut proposed = 0;
    for (pack, args) in [("habitat", vec!["apply", "habitat", "--on-holes"]), ("standard", vec!["apply", "standard"])] {
        if have.contains(&pack) {
            continue;
        }
        let mut a: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        a.push(genome.display().to_string());
        let ok = Command::new(&me).arg("dna").arg("pack").args(&a).status().is_ok_and(|s| s.success());
        if ok {
            proposed += 1;
            birth_done(pack)?;
        } else {
            return Err(format!("the `{pack}` pack was not proposed to the habitat's Board (`hale habitat up` tries again)"));
        }
    }
    if proposed > 0 {
        out.push("proposed the habitat's positions (`habitat`) and its law (`standard`): `hale dna review all approve` ratifies them".to_string());
    }
    // whole
    let _ = std::fs::remove_file(birth_pending());
    Ok(out)
}

/// Whether the habitat's record was born, as memory says.
enum Genesis {
    /// no identity pinned yet
    Unpinned,
    /// memory holds the pinned identity's genesis
    Present,
    /// memory answered that it holds no genesis for the pinned identity,
    /// or the identity's memory allocation was never provisioned (no
    /// volume of it exists): what was made for it holds no record
    Never(String),
}

/// GH #1653: the pinned identity's genesis, asked of memory. Memory that is
/// not up is brought up (the habitat's own, a habitat act) and asked again;
/// memory that still cannot be reached is a refusal, and nothing is
/// destroyed on a guess.
fn genesis_state(genome: &Path) -> Result<Genesis, String> {
    let Some(id) = manifest_pin(&genome.join("hale.toml")).ok().flatten() else { return Ok(Genesis::Unpinned) };
    let key = id[..12.min(id.len())].to_string();
    let asked = super::record_exists(genome);
    if let Ok(present) = asked {
        return Ok(if present { Genesis::Present } else { Genesis::Never(id) });
    }
    // memory could not say: the habitat's own allocation that never had a
    // volume never held a record
    if !own_servers() && !memory_volume_exists(&key) {
        return Ok(Genesis::Never(id));
    }
    let up = memory_migrate(genome);
    match (up, super::record_exists(genome)) {
        (_, Ok(true)) => Ok(Genesis::Present),
        (_, Ok(false)) => Ok(Genesis::Never(id)),
        (Err(e), Err(_)) | (_, Err(e)) => Err(format!("the habitat's memory cannot be reached to say whether its record ({key}…) was born, and nothing is undone on a guess: {e}")),
    }
}

/// Whether the memory volume of the allocation keyed `key` may exist in
/// the habitat's project: false only when docker answers that it has no
/// such volume; anything else (no docker, no daemon, an error) is taken as
/// "it may", so nothing is drawn again on a guess.
fn memory_volume_exists(key: &str) -> bool {
    let name = format!("{}_memory-{key}", local_habitat_project());
    match Command::new("docker").args(["volume", "inspect", &name]).output() {
        Ok(o) if !o.status.success() => !String::from_utf8_lossy(&o.stderr).to_lowercase().contains("no such volume"),
        _ => true,
    }
}

/// The allocation keyed `key` taken down whole (its services and volumes)
/// and its cache removed: what a birth that never reached memory leaves.
/// Only ever called once memory has answered that the identity holds no
/// record (`genesis_state`).
fn forget_allocation(key: &str) {
    let dir = PathBuf::from(local_habitat_dir()).join("allocations").join(key);
    if dir.join("compose.yaml").is_file() && dir.join("ports.yaml").is_file() {
        let _ = Command::new("env")
            .args(["COMPOSE_IGNORE_ORPHANS=1", "docker", "compose", "-p"])
            .arg(local_habitat_project())
            .arg("--project-directory")
            .arg(&dir)
            .arg("-f")
            .arg(dir.join("compose.yaml"))
            .arg("-f")
            .arg(dir.join("ports.yaml"))
            .args(["down", "-v"])
            .output();
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The record identity taken out of the manifest's `dna` table, every
/// other line kept.
fn unpin(manifest: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(manifest).map_err(|e| format!("read {}: {e}", manifest.display()))?;
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| format!("{} does not parse: {e}", manifest.display()))?;
    if let Some(t) = doc.get_mut("dna").and_then(|d| d.as_table_like_mut()) {
        t.remove("record");
    }
    std::fs::write(manifest, doc.to_string()).map_err(|e| format!("write {}: {e}", manifest.display()))
}

fn start_unit(me: &str, genome: &Path, backends: &str) -> Result<String, String> {
    let data = std::env::var("XDG_DATA_HOME").ok().filter(|v| !v.is_empty()).unwrap_or_else(|| format!("{}/.local/share", std::env::var("HOME").unwrap_or_default()));
    let mut vault = std::env::var("HALE_VAULT_DIR").ok().filter(|v| !v.is_empty()).map(|v| format!("Environment=HALE_VAULT_DIR={v}\n")).unwrap_or_default();
    // `operator` backends reach the unit through a file of the habitat's,
    // mode 600, as a provisioned body's env file does: never on the unit's
    // own lines
    if backends == "operator" {
        let env = PathBuf::from(local_habitat_dir()).join("backends.env");
        let text = ["HALE_DNA_MEMORY_DSN_OWNER", "HALE_DNA_NATS_URL_OWNER"].iter().map(|k| format!("{k}={}\n", std::env::var(k).unwrap_or_default())).collect::<String>();
        use std::os::unix::fs::OpenOptionsExt;
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&env).map_err(|e| format!("write {}: {e}", env.display()))?;
        f.write_all(text.as_bytes()).map_err(|e| format!("write {}: {e}", env.display()))?;
        vault.push_str(&format!("EnvironmentFile={}\n", env.display()));
    }
    let unit = format!(
        "[Unit]\nDescription=hale habitat (local) at {dir}\nAfter=network-online.target\n\n[Service]\nWorkingDirectory={g}\nEnvironment=XDG_DATA_HOME={data}\nEnvironment=HALE_DNA_HABITAT={dir}\nEnvironment=HALE_DNA_COMPOSE=off\n{vault}ExecStart={me} dna dev {g} --no-iris\nRestart=on-failure\nRestartSec=5\nTimeoutStopSec={stop}\n\n[Install]\nWantedBy=default.target\n",
        dir = local_habitat_dir(),
        g = genome.display(),
        // `systemctl stop` waits for the body's drain, which reaches its
        // backends before `down` stops them
        stop = drain_window().as_secs()
    );
    let path = unit_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
    }
    std::fs::write(&path, unit).map_err(|e| format!("write {}: {e}", path.display()))?;
    let systemctl = |args: &[&str]| Command::new("systemctl").arg("--user").args(args).output().map_err(|e| format!("systemctl: {e}")).and_then(|o| if o.status.success() { Ok(()) } else { Err(format!("systemctl --user {}: {}", args.join(" "), String::from_utf8_lossy(&o.stderr).trim())) });
    systemctl(&["daemon-reload"])?;
    systemctl(&["enable", "--now", &unit_name()])?;
    Ok(format!("{:<8}up (systemd user unit {}, {})", "body", unit_name(), path.display()))
}

// ---------------------------------------------------------------
// the faces: the habitat's own, and each organism's address
// ---------------------------------------------------------------

/// How many consecutive loopback ports one organism's head takes: the
/// face (the port a browser opens), the head behind it, the API child, the
/// stub provider and the API child's commands, in that order from the
/// block's base. `hale dna head` reads them back from the organism's
/// `dna.head.url`, so the address the habitat records is the address the
/// head serves on.
pub(crate) const HEAD_BLOCK: u16 = 8;

/// The first port a block is looked for from; each habitat starts its
/// search at its own offset, so two habitats on one machine (a test's
/// beside the developer's) do not meet.
const HEAD_BLOCK_FROM: u16 = 8800;

pub(crate) fn head_url_at(root: &Path) -> Option<String> {
    let o = Command::new("git").arg("-C").arg(root).args(["config", "--get", "dna.head.url"]).output().ok()?;
    let t = String::from_utf8_lossy(&o.stdout).trim().to_string();
    (!t.is_empty()).then_some(t)
}

/// The port of a loopback head URL, `None` for any other address.
pub(crate) fn loopback_port(url: &str) -> Option<u16> {
    let rest = url.strip_prefix("http://127.0.0.1:").or_else(|| url.strip_prefix("http://localhost:"))?;
    let port: u16 = rest.trim_end_matches('/').parse().ok()?;
    (port >= 1 && u32::from(port) + u32::from(HEAD_BLOCK) <= 65535).then_some(port)
}

fn block_free(base: u16) -> bool {
    (0..5).all(|i| std::net::TcpListener::bind(("127.0.0.1", base + i)).is_ok())
}

/// The base port of the head block of the organism at `root`: the one its
/// `dna.head.url` names, or, when it names none, the first block from the
/// habitat's starting point that no block in `taken` overlaps and whose
/// ports are free, recorded there.
fn head_block(root: &Path, taken: &[u16]) -> Result<u16, String> {
    if let Some(port) = head_url_at(root).as_deref().and_then(loopback_port) {
        return Ok(port);
    }
    let h = openssl::sha::sha256(local_habitat_dir().as_bytes());
    let blocks = (65000 - u32::from(HEAD_BLOCK_FROM)) / u32::from(HEAD_BLOCK);
    let start = u32::from(u16::from_be_bytes([h[0], h[1]])) % 200;
    for i in 0..blocks {
        let base = HEAD_BLOCK_FROM + (((start + i) % blocks) as u16) * HEAD_BLOCK;
        if taken.iter().any(|t| t.abs_diff(base) < HEAD_BLOCK) || !block_free(base) {
            continue;
        }
        let ok = Command::new("git").arg("-C").arg(root).args(["config", "--local", "dna.head.url", &format!("http://127.0.0.1:{base}")]).status().is_ok_and(|s| s.success());
        if !ok {
            return Err(format!("could not record the head's address in {}'s git config", root.display()));
        }
        return Ok(base);
    }
    Err("no block of free loopback ports for a head".to_string())
}

/// The bases of the head blocks the habitat's grafts hold, from their rows.
fn granted_bases(genome: &Path) -> Vec<u16> {
    rows_of(genome).iter().filter(|r| r["kind"] == "habitat.grafted").filter_map(|r| r["body"].as_str().and_then(|b| serde_json::from_str::<serde_json::Value>(b).ok())).filter_map(|b| b["face"].as_str().and_then(loopback_port)).collect()
}

fn face_pid_file() -> PathBuf {
    PathBuf::from(local_habitat_dir()).join("face.pid")
}

fn face_state_dir() -> PathBuf {
    PathBuf::from(local_habitat_dir()).join("face-state")
}

fn face_unit_name() -> String {
    format!("{}-face.service", local_habitat_project())
}

/// The habitat's face as it runs: a pid, a unit, or `None`.
fn face_running() -> Option<String> {
    if let Ok(pid) = std::fs::read_to_string(face_pid_file()) {
        if let Ok(pid) = pid.trim().parse::<i32>() {
            if alive(pid) {
                return Some(format!("pid {pid}"));
            }
        }
    }
    Command::new("systemctl").args(["--user", "is-active", "--quiet"]).arg(face_unit_name()).status().is_ok_and(|s| s.success()).then(|| format!("systemd user unit {}", face_unit_name()))
}

/// The habitat's face, the standard face over the habitat's own head
/// (`hale dna head` on its genome), started when it is not up: as a second
/// systemd user unit, or, with `foreground`, as a child of this process
/// that outlives its exec. Its head's state is the habitat's `face-state`.
fn start_face(me: &str, genome: &Path, foreground: bool) -> Result<String, String> {
    let base = head_block(genome, &granted_bases(genome))?;
    if let Some(state) = face_running() {
        return Ok(format!("{:<8}up ({state}, http://127.0.0.1:{base}/)", "face"));
    }
    std::fs::create_dir_all(face_state_dir()).map_err(|e| format!("create {}: {e}", face_state_dir().display()))?;
    if foreground {
        use std::os::unix::process::CommandExt;
        let log = PathBuf::from(local_habitat_dir()).join("face.log");
        let out = std::fs::File::create(&log).map_err(|e| format!("write {}: {e}", log.display()))?;
        let err = out.try_clone().map_err(|e| e.to_string())?;
        let child = Command::new(me)
            .args(["dna", "head"])
            .arg(genome)
            .env("HALE_DNA_HEAD_STATE", face_state_dir())
            .stdin(std::process::Stdio::null())
            .stdout(out)
            .stderr(err)
            .process_group(0)
            .spawn()
            .map_err(|e| format!("cannot run {me} dna head: {e}"))?;
        std::fs::write(face_pid_file(), format!("{}\n", child.id())).map_err(|e| format!("write {}: {e}", face_pid_file().display()))?;
        return Ok(format!("{:<8}starting (pid {}, http://127.0.0.1:{base}/; its log {})", "face", child.id(), log.display()));
    }
    let data = std::env::var("XDG_DATA_HOME").ok().filter(|v| !v.is_empty()).unwrap_or_else(|| format!("{}/.local/share", std::env::var("HOME").unwrap_or_default()));
    let vault = std::env::var("HALE_VAULT_DIR").ok().filter(|v| !v.is_empty()).map(|v| format!("Environment=HALE_VAULT_DIR={v}\n")).unwrap_or_default();
    let unit = format!(
        "[Unit]\nDescription=hale habitat (local) face at {dir}\nAfter=network-online.target {body}\n\n[Service]\nWorkingDirectory={g}\nEnvironment=XDG_DATA_HOME={data}\nEnvironment=HALE_DNA_HABITAT={dir}\nEnvironment=HALE_DNA_HEAD_STATE={state}\n{vault}ExecStart={me} dna head {g}\nRestart=on-failure\nRestartSec=5\n\n[Install]\nWantedBy=default.target\n",
        dir = local_habitat_dir(),
        body = unit_name(),
        g = genome.display(),
        state = face_state_dir().display()
    );
    let path = unit_path().with_file_name(face_unit_name());
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
    }
    std::fs::write(&path, unit).map_err(|e| format!("write {}: {e}", path.display()))?;
    let systemctl = |args: &[&str]| Command::new("systemctl").arg("--user").args(args).output().map_err(|e| format!("systemctl: {e}")).and_then(|o| if o.status.success() { Ok(()) } else { Err(format!("systemctl --user {}: {}", args.join(" "), String::from_utf8_lossy(&o.stderr).trim())) });
    systemctl(&["daemon-reload"])?;
    systemctl(&["enable", "--now", &face_unit_name()])?;
    Ok(format!("{:<8}up (systemd user unit {}, http://127.0.0.1:{base}/)", "face", face_unit_name()))
}

/// The habitat's face stopped: its launcher, and the children the head kept
/// in the habitat's face state (the API child), which a stopped head leaves.
fn stop_face(said: &mut Vec<String>) {
    if let Ok(pid) = std::fs::read_to_string(face_pid_file()) {
        if let Ok(pid) = pid.trim().parse::<i32>() {
            if alive(pid) {
                unsafe { libc::kill(pid, libc::SIGTERM) };
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
                while alive(pid) && std::time::Instant::now() < deadline {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                if alive(pid) {
                    unsafe { libc::kill(pid, libc::SIGKILL) };
                }
                said.push(format!("stopped the habitat's face (pid {pid})"));
            }
        }
        let _ = std::fs::remove_file(face_pid_file());
    }
    if Command::new("systemctl").args(["--user", "is-active", "--quiet"]).arg(face_unit_name()).status().is_ok_and(|s| s.success())
        && Command::new("systemctl").args(["--user", "stop"]).arg(face_unit_name()).status().is_ok_and(|s| s.success())
    {
        said.push(format!("stopped the habitat's face (systemd user unit {})", face_unit_name()));
    }
    if let Ok(dir) = std::fs::read_dir(face_state_dir().join("children")) {
        for e in dir.flatten() {
            if e.path().extension().is_some_and(|x| x == "pid") {
                if let Some(pid) = std::fs::read_to_string(e.path()).ok().and_then(|t| t.trim().parse::<i32>().ok()) {
                    if alive(pid) {
                        unsafe { libc::kill(pid, libc::SIGTERM) };
                    }
                }
            }
        }
    }
}

fn status() -> ExitCode {
    let genome = genome_dir();
    println!("habitat local at {} (compose project {})", local_habitat_dir(), local_habitat_project());
    match manifest_pin(&genome.join("hale.toml")).ok().flatten() {
        Some(id) => println!("genome  {} (record {}…)", genome.display(), &id[..12.min(id.len())]),
        None => println!("genome  not born: `hale habitat up` births it"),
    }
    println!("{:<8}{}", "body", body_state().unwrap_or_else(|| "down: `hale habitat up` starts it".to_string()));
    match head_url_at(&genome).as_deref().and_then(loopback_port) {
        Some(port) => println!("{:<8}http://127.0.0.1:{port}/ ({})", "face", face_running().map(|s| format!("up, {s}")).unwrap_or_else(|| "down: `hale habitat up` starts it".to_string())),
        None => println!("{:<8}not assigned: `hale habitat up` assigns its address", "face"),
    }
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
        for role in ["memory", "nerves", "senses", "database"] {
            if role == "database" && !row["roles"]["database"].is_object() {
                continue;
            }
            // the heart's database is a tenant of the memory server
            let service = if role == "database" { format!("memory-{key}") } else { format!("{role}-{key}") };
            let on_operator = row["roles"][role]["service"].as_str() == Some("operator");
            let state = if on_operator { "on the operator's server" } else if running.iter().any(|s| s == &service) { "running" } else { "stopped" };
            let recorded = row["roles"][role]["schema_version"].as_u64();
            let want = versions.iter().find(|(r, _)| r == role).map(|(_, v)| *v);
            // a heart role's schema is the heart's: its last migration run
            let migration = row["roles"][role]["migration"].as_str().map(|m| format!(", its migration last ran with {m}")).unwrap_or_default();
            let version = match (recorded, want) {
                (Some(had), Some(want)) if had < want => format!(", schema {had}, behind this toolchain's {want}: `hale habitat migrate {}` (or `hale dna upgrade` there) migrates it", row["root"].as_str().unwrap_or("<organism>")),
                (Some(had), Some(_)) => format!(", schema {had} (this toolchain's)"),
                (Some(had), None) => format!(", schema {had}"),
                (None, _) => String::new(),
            };
            println!("  {role:<7} {service} {state}{version}{migration}");
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

/// How long the habitat's body may take to drain once told to stop: the
/// host's drain grace (the organization's and its lease's, up to
/// `LOTUS_DRAIN_GRACE_MS`, the node default otherwise) and a margin. Its
/// backends are stopped only after it has gone, so the drain still reaches
/// them.
fn drain_window() -> std::time::Duration {
    let grace: u64 = std::env::var("LOTUS_DRAIN_GRACE_MS").ok().and_then(|v| v.parse().ok()).unwrap_or_else(|| super::NODE_DRAIN_GRACE_MS.parse().unwrap_or(30000));
    std::time::Duration::from_millis(grace + 15000)
}

fn alive(pid: i32) -> bool {
    pid > 0 && unsafe { libc::kill(pid, 0) } == 0
}

fn down() -> ExitCode {
    let mut said = Vec::new();
    stop_face(&mut said);
    if let Ok(pid) = std::fs::read_to_string(pid_file()) {
        if let Ok(pid) = pid.trim().parse::<i32>() {
            if alive(pid) && unsafe { libc::kill(pid, libc::SIGTERM) } == 0 {
                // the body drains on SIGTERM, onto its backends: wait for it
                // (the pid file stays while it lives), and only then stop them
                let deadline = std::time::Instant::now() + drain_window();
                while alive(pid) && std::time::Instant::now() < deadline {
                    std::thread::sleep(std::time::Duration::from_millis(200));
                }
                if alive(pid) {
                    eprintln!("hale habitat down: the habitat's body (pid {pid}) did not drain within {}s; killed", drain_window().as_secs());
                    unsafe { libc::kill(pid, libc::SIGKILL) };
                    while alive(pid) {
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                }
                said.push(format!("stopped the habitat's body (pid {pid}), drained"));
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
    let here = local_habitat_dir();
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
/// docker's compose containers, `<project>|<working dir>` a line.
const COMPOSE_LISTING: [&str; 6] = ["ps", "-a", "--filter", "label=com.docker.compose.project", "--format", "{{.Label \"com.docker.compose.project\"}}|{{.Label \"com.docker.compose.project.working_dir\"}}"];

/// Every compose container's project and working directory, as
/// [`down_under`] reads them; "" without docker.
pub(crate) fn compose_projects_listed() -> Result<String, String> {
    match docker(&COMPOSE_LISTING) {
        Docker::Absent => Ok(String::new()),
        Docker::Failed(why) => Err(why),
        Docker::Answered(out) => Ok(out),
    }
}

pub(crate) fn down_under(root: &Path) -> Result<(), String> {
    down_under_with(root, &mut |args| docker(args))
}

pub(crate) fn down_under_with(root: &Path, docker: &mut dyn FnMut(&[&str]) -> Docker) -> Result<(), String> {
    let file = root.join(COMPOSE_FILE);
    let recorded = std::fs::read_to_string(&file).unwrap_or_default();
    let listed = match docker(&COMPOSE_LISTING) {
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

// ---------------------------------------------------------------
// graft (GH #1653)
// ---------------------------------------------------------------

/// The organism's own roles, the ones the habitat provisions for the
/// organism itself (its record is memory's, its organization reads the
/// nerves, its body's readings go to the senses), and of them the ones it
/// always gets.
const ROLES: [&str; 3] = ["memory", "nerves", "senses"];
const ALWAYS: [&str; 2] = ["memory", "nerves"];
/// The roles a heart may need of its own: never given by default (a
/// foreign heart's infrastructure is its own, and none of it is the
/// habitat's to touch), only when a Hale heart's genome declares one. The
/// local profile provisions `database` (a tenant database of the
/// organism's memory server) and `broker` (a user of its nerves server
/// with a subject space and an inbox of its own); the rest are refused
/// until a heart needs them.
const HEART_ROLES: [&str; 4] = ["database", "broker", "cache", "object-store"];
const PROVISIONED_HEART_ROLES: [&str; 2] = ["database", "broker"];

/// GH #1653: the roles the genome at `dir` needs from the habitat, as its
/// manifest declares them (`[dna] needs = ["memory", "nerves", …]`): the
/// first half of graft, satisfied by its second. None declared (no
/// manifest, or a foreign heart's, which declares nothing) is exactly the
/// organism's own roles, never a heart's: the habitat provides nothing for
/// a heart that did not ask. A heart role is declared by a Hale heart that
/// needs one, and is refused for now (this habitat provisions none); a
/// role it does not know, or a list without memory or the nerves, is
/// refused by name.
pub(super) fn needs_of(manifest_text: Option<&str>) -> Result<Vec<String>, String> {
    let declared = match manifest_text {
        None => None,
        Some(text) => {
            let table: toml::Table = text.parse().map_err(|e| format!("hale.toml does not parse: {e}"))?;
            match table.get("dna").and_then(|d| d.get("needs")) {
                None => None,
                Some(toml::Value::Array(a)) => Some(a.iter().map(|v| v.as_str().map(str::to_string).ok_or_else(|| format!("[dna] needs holds {v}, not a role's name"))).collect::<Result<Vec<_>, _>>()?),
                Some(other) => return Err(format!("[dna] needs is {other}, not a list of roles")),
            }
        }
    };
    let Some(mut needs) = declared else { return Ok(ROLES.iter().map(|r| r.to_string()).collect()) };
    if let Some(r) = needs.iter().find(|r| HEART_ROLES.contains(&r.as_str()) && !PROVISIONED_HEART_ROLES.contains(&r.as_str())) {
        return Err(format!("[dna] needs names the heart role `{r}`, which this habitat does not provision yet (it provisions the organism's own, {}, and the heart roles {})", ROLES.join(", "), PROVISIONED_HEART_ROLES.join(", ")));
    }
    if let Some(r) = needs.iter().find(|r| !ROLES.contains(&r.as_str()) && !PROVISIONED_HEART_ROLES.contains(&r.as_str())) {
        return Err(format!("[dna] needs names `{r}`, which the habitat does not provision (it provisions the organism's own roles, {}, and knows the heart roles {})", ROLES.join(", "), HEART_ROLES.join(", ")));
    }
    for must in ALWAYS {
        if !needs.iter().any(|r| r == must) {
            return Err(format!("[dna] needs leaves out `{must}`, which every organism needs: its record is memory's and its organization reads the nerves"));
        }
    }
    needs.sort_by_key(|r| ROLES.iter().chain(PROVISIONED_HEART_ROLES.iter()).position(|x| x == r));
    needs.dedup();
    Ok(needs)
}

fn graft(args: &[String]) -> ExitCode {
    let mut dir: Option<PathBuf> = None;
    let mut attach = false;
    let mut library = true;
    let mut pack = true;
    let mut with: Option<String> = None;
    let mut who: Option<String> = None;
    let mut sets = Vec::new();
    let mut profile: Option<String> = None;
    let mut remote: Option<String> = None;
    let mut body: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "--attach" => attach = true,
            "--no-library" => library = false,
            "--no-pack" => pack = false,
            "--profile" | "--remote" | "--body" => {
                let Some(v) = args.get(i + 1) else {
                    eprintln!("hale habitat graft: `{a}` takes a value\n{}", usage_text());
                    return ExitCode::from(2);
                };
                match a {
                    "--profile" => profile = Some(v.clone()),
                    "--remote" => remote = Some(v.clone()),
                    _ => body = Some(v.clone()),
                }
                i += 1;
            }
            "--with" | "--set" | "--as" => {
                let Some(v) = args.get(i + 1) else {
                    eprintln!("hale habitat graft: `{a}` takes a value\n{}", usage_text());
                    return ExitCode::from(2);
                };
                match a {
                    "--with" => with = Some(v.clone()),
                    "--set" => sets.push(v.clone()),
                    _ => who = Some(v.clone()),
                }
                i += 1;
            }
            f if f.starts_with("--") => {
                eprintln!("hale habitat graft: `{f}` is not a flag of `graft`\n{}", usage_text());
                return ExitCode::from(2);
            }
            d => {
                if dir.replace(PathBuf::from(d)).is_some() {
                    eprintln!("hale habitat graft: one directory\n{}", usage_text());
                    return ExitCode::from(2);
                }
            }
        }
        i += 1;
    }
    let Some(dir) = dir else {
        eprintln!("hale habitat graft: which directory?\n{}", usage_text());
        return ExitCode::from(2);
    };
    if with.is_none() && (!sets.is_empty() || who.is_some()) {
        eprintln!("hale habitat graft: `--set` and `--as` go with a pack; name the pack with `--with <pack>`");
        return ExitCode::from(2);
    }
    if attach && (profile.is_some() || remote.is_some() || body.is_some()) {
        eprintln!("hale habitat graft: `--profile`, `--remote` and `--body` shape a new application; an attached one keeps its own (`hale dna profile` detects it)");
        return ExitCode::from(2);
    }
    let shape = Shape { profile: profile.as_deref(), remote: remote.as_deref(), body: body.as_deref() };
    match graft_lines(&dir, attach, library, pack, &shape) {
        Ok(lines) => {
            for l in lines {
                println!("{l}");
            }
        }
        Err(e) => {
            eprintln!("hale habitat graft: {e}");
            return ExitCode::from(1);
        }
    }
    // the pack planned on the holes the birth proposed (a process of its
    // own: the verb execs the host); its refusal is the graft's exit
    let Some(pack) = with else { return ExitCode::SUCCESS };
    let mut pack_args = vec!["dna".to_string(), "pack".to_string(), "apply".to_string(), pack, "--on-holes".to_string()];
    for v in sets {
        pack_args.push("--set".to_string());
        pack_args.push(v);
    }
    if let Some(who) = who {
        pack_args.push("--as".to_string());
        pack_args.push(who);
    }
    pack_args.push(dir.to_string_lossy().to_string());
    let _ = std::io::Write::flush(&mut std::io::stdout());
    let status = std::env::current_exe().and_then(|me| Command::new(me).args(&pack_args).status());
    match status {
        Ok(s) if s.success() => ExitCode::SUCCESS,
        Ok(s) => ExitCode::from(s.code().and_then(|c| u8::try_from(c).ok()).unwrap_or(1)),
        Err(e) => {
            eprintln!("hale habitat graft: the `--with` pack: {e}");
            ExitCode::from(1)
        }
    }
}

/// GH #1653: an organism born as the habitat's child: its needs read from
/// its genome, its record's genesis naming the habitat's record, each role
/// it needs allocated to it, and the graft and its needs a row of the
/// habitat's record. The habitat must be born, and the environment must
/// agree with its backends.
/// A new application's profile pieces (`--profile`, `--remote`, `--body`).
struct Shape<'a> {
    profile: Option<&'a str>,
    remote: Option<&'a str>,
    body: Option<&'a str>,
}

fn graft_lines(dir: &Path, attach: bool, library: bool, pack: bool, shape: &Shape) -> Result<Vec<String>, String> {
    let genome = genome_dir();
    let Some(parent) = manifest_pin(&genome.join("hale.toml")).ok().flatten() else {
        return Err("the habitat is not born: `hale habitat up` births it, and every other organism is grafted from it".to_string());
    };
    if !super::record_exists(&genome).unwrap_or(false) {
        return Err("the habitat's record cannot be read (its memory is not up): `hale habitat up` brings it up".to_string());
    }
    let backends = backends_of(None)?;
    if attach && !dir.is_dir() {
        return Err(format!("{} is not there to attach (graft without --attach births a new application there)", dir.display()));
    }
    if !attach && std::fs::read_dir(dir).is_ok_and(|mut d| d.next().is_some()) {
        return Err(format!("{} exists and is not empty: `--attach` grafts the application or repository that is there", dir.display()));
    }
    // the organism's root, as its birth resolves it: an attached
    // application's workspace (its manifest, its pin and its needs are the
    // workspace's), a repository's own directory, or the new application
    let root = if attach { super::organism_root(dir)? } else { dir.to_path_buf() };
    let manifest = std::fs::read_to_string(root.join("hale.toml")).ok();
    // a genome that pins a record already is this habitat's child whose
    // graft stopped before its end, or ended: the graft goes on where it
    // stopped, each step keeping what is done. A record born of anything
    // else is never grafted, and one memory does not hold is refused by
    // the birth itself, naming it
    let pinned = manifest.as_deref().and_then(manifest_pin_in_text);
    if let Some(id) = &pinned {
        let short = &id[..12.min(id.len())];
        // memory decides, never a guess: the genesis is read, after the
        // local setup the birth would make first when it cannot be
        let mut said = pinned_genesis(&root);
        if let PinnedGenesis::Unread(_) = said {
            let _ = super::memory_migrate(&root);
            said = pinned_genesis(&root);
        }
        match said {
            PinnedGenesis::Parent(of) if of == parent => {}
            PinnedGenesis::Parent(of) => {
                let whose = if of.is_empty() { "no parent".to_string() } else { format!("the record {}…", &of[..12.min(of.len())]) };
                return Err(format!("{} pins the record {short}…, born of {whose}, not of this habitat: an organism is grafted once, at its birth", root.display()));
            }
            // memory holds no such record: the birth refuses the pin,
            // naming it
            PinnedGenesis::Absent => {}
            PinnedGenesis::Unread(why) => return Err(format!("{} pins the record {short}…, whose genesis could not be read, so its parent is not known: {why}", root.display())),
        }
    }
    let needs = needs_of(manifest.as_deref())?;
    let mut out = vec![format!("{:<8}{} as the habitat's child (parent {}…; needs {}){}", "graft", root.display(), &parent[..12], needs.join(", "), if pinned.is_some() { "; the graft goes on where it stopped" } else { "" })];
    out.extend(if attach { super::init_under(dir, library, super::Lineage::Child(&parent))? } else { super::new_project_under(dir, shape.profile, shape.remote, shape.body, library, &parent)? });
    let root = if attach { root } else { project(dir)?.0 };
    let child = manifest_pin(&root.join("hale.toml")).ok().flatten().ok_or("the child pins no record after its birth")?;
    // birth allocated memory and the nerves; the rest of what it needs here
    if needs.iter().any(|r| r == "senses") && backends == "own" {
        match host_run("senses-up", &root, &[]) {
            Ok(o) if !o.starts_with("none: ") => {
                record_allocation(&root, "senses", 0, "", false)?;
                out.push(format!("senses  allocated ({})", o.trim().trim_start_matches("HALE_DNA_SENSES_URL=")));
            }
            Ok(o) => out.push(format!("{:<8}senses not up: {}", "note", o.trim().strip_prefix("none: ").unwrap_or(o.trim()))),
            Err(e) => out.push(format!("{:<8}senses not up: {e}", "note")),
        }
    }
    if needs.iter().any(|r| r == "database") {
        out.push(provision_database(&root, &backends)?);
    }
    if needs.iter().any(|r| r == "broker") {
        if backends == "operator" {
            return Err("the `broker` heart role is a user of the organism's own nerves server, which the habitat configures; a habitat on the operator's backends does not configure the operator's NATS".to_string());
        }
        out.push(provision_broker(&root)?);
    }
    // the heart's own migrations, with its roles in place
    out.extend(run_heart_migrations(&root)?);
    // the attach handover: the service client the heart signs in as,
    // mapped in the child's record and its secret drawn, so nothing of it
    // is written by hand
    out.extend(map_service_client(&root)?);
    // the graft is complete when the habitat's record holds its row: one
    // row, however many times the graft goes on
    if rows_of(&genome).iter().any(|r| r["kind"] == "habitat.grafted" && r["entity"] == &child[..12]) {
        out.push(format!("{:<8}the graft of {}… (the habitat's record holds it)", "kept", &child[..12]));
    } else {
        // the organism's address: its head block, taken beside every block
        // the habitat has handed out, and kept in its own `dna.head.url`
        let mut taken = granted_bases(&genome);
        taken.extend(head_url_at(&genome).as_deref().and_then(loopback_port));
        let base = head_block(&root, &taken)?;
        let name = root.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let row = serde_json::json!({ "kind": "habitat.grafted", "entity": &child[..12], "body": serde_json::json!({ "child": child, "parent": parent, "needs": needs, "backends": backends, "name": name, "face": format!("http://127.0.0.1:{base}"), "head": format!("http://127.0.0.1:{}", base + 1) }).to_string() });
        let file = PathBuf::from(local_habitat_dir()).join(".graft-row.jsonl");
        std::fs::write(&file, format!("{row}\n")).map_err(|e| format!("write {}: {e}", file.display()))?;
        let landed = host_run("record-seed", &genome, &[file.display().to_string(), String::new()]);
        let _ = std::fs::remove_file(&file);
        landed?;
        out.push(format!("recorded the graft of {}… in the habitat's record", &child[..12]));
    }
    // the standard practices, the law every organism is born under, for
    // the child's Board (a process of its own: the verb execs the host),
    // proposed once
    if !pack {
        out.push(format!("{:<8}the `standard` pack (--no-pack: `hale dna pack apply standard` proposes it)", "skipped"));
        return Ok(out);
    }
    let proposed = |r: &serde_json::Value| {
        r["kind"] == "batch.proposed" && r["body"].as_str().and_then(|b| serde_json::from_str::<serde_json::Value>(b).ok()).is_some_and(|b| b["pack"].as_str().is_some_and(|p| p.starts_with("standard@")))
    };
    if pinned.is_some() && rows_of(&root).iter().any(proposed) {
        out.push(format!("{:<8}the `standard` pack (proposed to the child's Board already)", "kept"));
        return Ok(out);
    }
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let ok = Command::new(&me).args(["dna", "pack", "apply", "standard"]).arg(&root).status().is_ok_and(|s| s.success());
    out.push(if ok { "proposed the `standard` pack to the child's Board".to_string() } else { format!("{:<8}the `standard` pack was not proposed: `hale dna pack apply standard {}` proposes it", "note", root.display()) });
    Ok(out)
}

/// What memory says of the record the organism at `root` pins (GH #1653):
/// the parent its genesis names ("" for none), that it holds no such
/// record, or why it could not be read.
enum PinnedGenesis {
    Parent(String),
    Absent,
    Unread(String),
}

fn pinned_genesis(root: &Path) -> PinnedGenesis {
    // the clone's kept head DSN, else one derived from memory that is
    // there (nothing brought up, nothing written)
    let head = super::kept_head_dsn(root).ok().flatten().or_else(|| {
        host_run("memory-migrate", root, &["--derive".to_string()]).ok().and_then(|o| o.lines().find_map(|l| l.strip_prefix("HALE_DNA_MEMORY_DSN_HEAD=")).map(str::to_string))
    });
    let ask = |verb: &str| -> Result<String, String> {
        let (mut cmd, _) = super::host_command(verb, root)?;
        if let Some(h) = &head {
            cmd.env("HALE_DNA_MEMORY_DSN_HEAD", h);
        }
        let out = cmd.output().map_err(|e| format!("hale dna {verb}: {e}"))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            return Err(if err.is_empty() { format!("hale dna {verb} failed") } else { err });
        }
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    };
    match ask("record-head") {
        Err(why) => return PinnedGenesis::Unread(why),
        Ok(h) if h.trim() == "none" => return PinnedGenesis::Absent,
        Ok(_) => {}
    }
    let rows = match ask("record-rows") {
        Ok(r) => r,
        Err(why) => return PinnedGenesis::Unread(why),
    };
    let genesis = rows.lines().filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok()).find(|r| r["kind"] == "record.genesis");
    match genesis {
        Some(g) => {
            let body: serde_json::Value = g["body"].as_str().and_then(|b| serde_json::from_str(b).ok()).unwrap_or_default();
            PinnedGenesis::Parent(body["parent"].as_str().unwrap_or("").to_string())
        }
        None => PinnedGenesis::Unread("the record has no genesis row".to_string()),
    }
}

/// The rows of the record of the organism at `root`, none when it cannot
/// be read.
fn rows_of(root: &Path) -> Vec<serde_json::Value> {
    host_run("record-rows", root, &[]).map(|rows| rows.lines().filter_map(|l| serde_json::from_str(l).ok()).collect()).unwrap_or_default()
}

/// GH #1653: the `database` heart role provisioned for the organism at
/// `root`, a tenant database of its memory server, and recorded as its
/// allocation; the heart is handed `HALE_DNA_DATABASE_DSN`, which names
/// its vault slot.
pub(super) fn provision_database(root: &Path, backends: &str) -> Result<String, String> {
    let o = host_run("database-provision", root, &[])?;
    if let Some(why) = o.trim().strip_prefix("none: ") {
        return Err(format!("database: {why}"));
    }
    let slots = o.lines().find_map(|l| l.strip_prefix("allocation.vault_slots=")).unwrap_or("");
    record_allocation(root, "database", 0, slots, backends == "operator")?;
    Ok(format!("database a tenant database of the organism's memory server, for its heart (HALE_DNA_DATABASE_DSN, its password in the vault slot {slots})"))
}

/// GH #1653: the `broker` heart role, recorded as the organism's
/// allocation and then provisioned from that row: its two users, their
/// subject space and their passwords, written into its nerves server's
/// configuration with its other secrets.
fn provision_broker(root: &Path) -> Result<String, String> {
    // the names the broker is handed by, before its row is written: the
    // role is named, as the habitat has not recorded it yet
    let named = handover_with(root, true, "broker")?;
    let name = |k: &str| named.iter().find_map(|(n, v)| (n == k).then(|| v.clone())).unwrap_or_default();
    let vault = name("HALE_DNA_BROKER_VAULT");
    let node_vault = name("HALE_DNA_BROKER_NODE_VAULT");
    if vault.is_empty() || node_vault.is_empty() {
        return Err("the broker's vault slots are not named by the handover".to_string());
    }
    // the habitat's decision is its row: every reader of the organism's
    // heart roles (its secrets, its nerves server's users, its handover)
    // reads the row, never the manifest again
    record_allocation(root, "broker", 0, &format!("{vault} {node_vault}"), false)?;
    // its two users and their passwords, drawn with the organism's other
    // secrets now the allocation names the role; the server rereads them
    let drawn = host_run("secrets-provision", root, &[])?;
    if let Some(why) = drawn.trim().strip_prefix("none: ") {
        return Err(format!("the broker's users were not provisioned: {why}"));
    }
    let lines = handover(root, false)?;
    let get = |k: &str| lines.iter().find_map(|(n, v)| (n == k).then(|| v.clone())).unwrap_or_default();
    Ok(format!("{:<8}{} and its nodes' {} on the organism's nerves server, confined to {} (HALE_DNA_BROKER_*, their passwords in the vault slots {vault} and {node_vault})", "broker", get("HALE_DNA_BROKER_USER"), get("HALE_DNA_BROKER_NODE_USER"), get("HALE_DNA_BROKER_SUBJECTS")))
}

/// The handover the habitat gives the heart of the organism at `root`,
/// `(name, value)`; `derive` provisions nothing.
pub(super) fn handover(root: &Path, derive: bool) -> Result<Vec<(String, String)>, String> {
    handover_with(root, derive, "")
}

/// [`handover`], with heart roles named beside the ones the habitat
/// allocated (`with`, words): a graft names the role it is provisioning
/// before the role's row is written.
fn handover_with(root: &Path, derive: bool, with: &str) -> Result<Vec<(String, String)>, String> {
    let mut args: Vec<String> = if derive { vec!["--derive".to_string()] } else { Vec::new() };
    if !with.is_empty() {
        args.push("--with".to_string());
        args.push(with.to_string());
    }
    let o = host_run("heart-handover", root, &args)?;
    if let Some(why) = o.trim().strip_prefix("none: ") {
        return Err(why.to_string());
    }
    Ok(o.lines().filter(|l| l.starts_with("HALE_DNA_")).filter_map(|l| l.split_once('=').map(|(k, v)| (k.to_string(), v.to_string()))).collect())
}

/// The heart's service client mapped in the child's record (`git config
/// --local --add dna.oidc.service <client>=<client>`, once) and its secret
/// drawn into the vault slot the handover names.
fn map_service_client(root: &Path) -> Result<Vec<String>, String> {
    let lines = handover(root, true).unwrap_or_default();
    let Some(client) = lines.iter().find_map(|(k, v)| (k == "HALE_DNA_OIDC_CLIENT").then(|| v.clone())) else { return Ok(Vec::new()) };
    let mapping = format!("{client}={client}");
    let have = Command::new("git").arg("-C").arg(root).args(["config", "--local", "--get-all", "dna.oidc.service"]).output().map(|o| String::from_utf8_lossy(&o.stdout).lines().any(|l| l.trim() == mapping)).unwrap_or(false);
    if !have {
        let ok = Command::new("git").arg("-C").arg(root).args(["config", "--local", "--add", "dna.oidc.service", &mapping]).status().is_ok_and(|s| s.success());
        if !ok {
            return Err(format!("could not map the service client `{client}` in {}", root.display()));
        }
    }
    let drawn = super::organism_secrets(root, false);
    Ok(vec![format!("{:<8}the heart's service client `{client}` (dna.oidc.service {mapping}; its secret the vault slot oidc-service-{client})", "oidc"), drawn].into_iter().filter(|l| !l.is_empty()).collect())
}

/// The roles the organism at `root` needs, as its manifest declares them.
pub(super) fn needs_at(root: &Path) -> Vec<String> {
    needs_of(std::fs::read_to_string(root.join("hale.toml")).ok().as_deref()).unwrap_or_else(|_| ROLES.iter().map(|r| r.to_string()).collect())
}

fn manifest_pin_in_text(text: &str) -> Option<String> {
    let table: toml::Table = text.parse().ok()?;
    table.get("dna").and_then(|d| d.get("record")).and_then(|v| v.as_str()).map(str::to_string)
}


#[cfg(test)]
mod tests {
    use super::*;

    /// GH #1653: a heart migration is declared for a heart role the
    /// genome needs, as its `command` alone.
    #[test]
    fn heart_migrations_are_declared_per_role() {
        let d = root("migrations");
        std::fs::write(d.join("hale.toml"), "[dna]\nneeds = [\"memory\", \"nerves\", \"database\"]\n\n[dna.migrations.database]\ncommand = [\"api\", \"migrate\"]\n").unwrap();
        let m = heart_migrations(&d).unwrap();
        assert_eq!((m.len(), m[0].role.as_str(), m[0].command.clone()), (1, "database", vec!["api".to_string(), "migrate".to_string()]));
        std::fs::write(d.join("hale.toml"), "[dna]\nneeds = [\"memory\", \"nerves\"]\n\n[dna.migrations.database]\ncommand = [\"api\"]\n").unwrap();
        assert!(heart_migrations(&d).err().unwrap().contains("`database` is not one"), "only for a role the genome needs");
        std::fs::write(d.join("hale.toml"), "[dna]\nneeds = [\"memory\", \"nerves\", \"database\"]\n\n[dna.migrations.database]\ncommand = []\n").unwrap();
        assert!(heart_migrations(&d).err().unwrap().contains("needs `command`"), "a command");
        std::fs::write(d.join("hale.toml"), "[dna]\nneeds = [\"memory\", \"nerves\", \"database\"]\n\n[dna.migrations.database]\ncommand = [\"api\"]\nschema_version = 5\n").unwrap();
        assert!(heart_migrations(&d).err().unwrap().contains("`schema_version` is not a key"), "the habitat keeps no version of the heart's schema");
        std::fs::write(d.join("hale.toml"), "[dna]\nneeds = [\"memory\", \"nerves\", \"database\"]\n").unwrap();
        assert!(heart_migrations(&d).unwrap().is_empty(), "a heart that declares none has nothing run");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// GH #1653: a heart role's allocation is recorded once and carries no
    /// version; the organism's own roles keep theirs.
    #[test]
    fn a_heart_role_carries_no_version() {
        let mut row = serde_json::json!({});
        let id = "0123456789abcdef0123456789abcdef";
        next_rows(&mut row, id, "database", 0, "postgres-heart_x", "p", false);
        next_rows(&mut row, id, "database", 0, "postgres-heart_x", "p", false);
        let pending = row["pending"].as_array().unwrap();
        assert_eq!(pending.len(), 1, "allocated once, and no migration row of the habitat's: {pending:?}");
        assert_eq!(pending[0]["kind"], "habitat.allocated");
        assert!(row["roles"]["database"]["schema_version"].is_null());
        next_rows(&mut row, id, "memory", 3, "", "p", false);
        assert_eq!(row["roles"]["memory"]["schema_version"], 3);
    }

    /// GH #1653: needs by role, the first half of graft.
    #[test]
    fn needs_are_declared_by_role() {
        // undeclared, foreign or not: exactly the organism's own roles, no
        // heart role
        for undeclared in [None, Some("[deps]\n")] {
            let got = needs_of(undeclared).unwrap();
            assert_eq!(got, ROLES, "an undeclared genome gets the organism's own roles");
            assert!(got.iter().all(|r| !HEART_ROLES.contains(&r.as_str())), "and no heart role: {got:?}");
        }
        assert_eq!(needs_of(Some("[dna]\nneeds = [\"database\", \"memory\", \"nerves\"]\n")).unwrap(), ["memory", "nerves", "database"], "a Hale heart declares the database it needs");
        assert_eq!(needs_of(Some("[dna]\nneeds = [\"memory\", \"nerves\", \"broker\"]\n")).unwrap(), ["memory", "nerves", "broker"], "and the broker it needs");
        assert!(needs_of(Some("[dna]\nneeds = [\"memory\", \"nerves\", \"cache\"]\n")).unwrap_err().contains("the heart role `cache`, which this habitat does not provision yet"), "the other heart roles are refused by name");
        assert_eq!(needs_of(Some("[dna]\nneeds = [\"nerves\", \"memory\"]\n")).unwrap(), ["memory", "nerves"], "senses left out");
        assert!(needs_of(Some("[dna]\nneeds = [\"memory\", \"redis\"]\n")).unwrap_err().contains("`redis`, which the habitat does not provision"));
        assert!(needs_of(Some("[dna]\nneeds = [\"memory\"]\n")).unwrap_err().contains("leaves out `nerves`"));
        assert!(needs_of(Some("[dna]\nneeds = \"memory\"\n")).unwrap_err().contains("not a list of roles"));
    }

    /// GH #1653: an allocation's rows are the organism's and the role's,
    /// never a checkout's: allocated once, migrated whenever the version
    /// moves, nothing when it does not; and a recorded version behind the
    /// toolchain's is named.
    #[test]
    fn allocation_rows_follow_the_version() {
        let id = "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
        let mut row = serde_json::json!({ "root": "/a/checkout" });
        next_rows(&mut row, id, "memory", 9, "postgres-owner-1234567890ab", "hale-habitat-local-x", false);
        let kinds: Vec<&str> = row["pending"].as_array().unwrap().iter().map(|r| r["kind"].as_str().unwrap()).collect();
        assert_eq!(kinds, ["habitat.allocated", "habitat.migrated"]);
        let allocated = row["pending"][0]["body"].as_str().unwrap();
        assert!(allocated.contains("\"organism\":\"1234567890abcdef") && allocated.contains("\"vault\":[\"postgres-owner-1234567890ab\"]") && !allocated.contains("checkout"), "{allocated}");
        assert_eq!(row["pending"][0]["entity"], "1234567890ab/memory");
        // phase 4: every allocation names its provider and its host
        assert!(allocated.contains("\"provider\":\"compose\"") && allocated.contains("\"host\":\"local\""), "{allocated}");
        let mut adopted = serde_json::json!({});
        next_rows(&mut adopted, id, "nerves", 2, "", "p", true);
        let on_operator = adopted["pending"][0]["body"].as_str().unwrap();
        assert!(on_operator.contains("\"provider\":\"operator\"") && on_operator.contains("\"host\":\"\"") && on_operator.contains("\"service\":\"operator\""), "{on_operator}");
        // flushed, then migrated again at the same version: nothing owed
        row["pending"] = serde_json::json!([]);
        row["roles"]["memory"]["recorded"] = serde_json::json!(true);
        next_rows(&mut row, id, "memory", 9, "", "p", false);
        assert!(row["pending"].as_array().unwrap().is_empty());
        // the version moves: one migrated row, from the old version
        next_rows(&mut row, id, "memory", 10, "", "p", false);
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

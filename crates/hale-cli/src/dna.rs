//! `hale dna` — the DNA command surface (GH #528, Track C of #521).
//!
//! Phase 1: the DNA core bundled with the toolchain and materialized
//! into a project, the assembly generated as ordinary Hale source,
//! and the Journal seeded from the compiler's own model of the
//! application. Nothing here is a configuration language: every
//! "setting" is a constructor argument in a `.hl` file the project
//! owns, and every generated fact carries its provenance.
//!
//!   hale dna init [app-dir] [--no-library]
//!                               attach the DNA to an existing app
//!   hale dna new <name>         a greenfield app with its DNA
//!   hale dna upgrade [dir]      re-materialize vendor/dna for this toolchain
//!   hale dna run [project]      build, run under LOTUS_OBS, a node relaying onto the nerves; iris inspects the process
//!   hale dna status [--json]    the status projection, from the Journal
//!   hale dna task create <outcome…>  ask for an outcome: a Task, a row a node relays to the organism
//!   hale dna history [<entity>] walk the Journal by causal links
//!   hale dna show org|processes [--json] [project]  the graph's two perspectives, from memory
//!   hale dna review <id> <verdict> a verdict, a row a node relays (the Review decides)
//!   hale dna --embedded-digest  the DNA source this binary embeds, by name (GH #726)
//!
//! Layout after `init` (root = the workspace holding hale.toml):
//!
//!   vendor/dna/*.hl            toolchain-owned copy of dna/core, pinned in hale.lock
//!   dna/assembly.hl            project-owned: the Genome constructor (editable)
//!   <app>/dna_constitution.hl  project-owned: groups + `constitution Project` (in the app's seed)
//!   dna/purpose.hl             project-owned: the declared purpose (ratified by the first Review)
//!   refs/dna/journal           the record, seeded from the topology artifact
//!   .hale/dna/baseline.topology the artifact it was seeded from
//!   hale.toml                  gains [claims] base = "Project" + [environments.local]
//!   <app>/main.hl              gains the imports, the `genome` param, `adopt Project`,
//!                              and the nerves' bindings
//!
//! Toolchain effects are subprocesses: the artifact is cut by
//! `hale check --dump-topology` exactly as a user would, never by a
//! second in-process pipeline.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use serde_json::Value;

/// The record (GH #566 F1): one commit per event on this ref, the events as
/// `journal.jsonl` in its tree, receipts as blobs under `refs/dna/receipts/`.
const RECORD_REF: &str = "refs/dna/journal";
/// The organization's seed (GH #566 F2): a program `init` generates and
/// `hale dna run` runs; the application is never grafted.
const ORG_SEED: &str = "dna/org";
const BASELINE_REL: &str = ".hale/dna/baseline.topology";
/// GH #726: which embedded DNA source materialized `vendor/dna` here.
/// Toolchain-owned like the tree it describes (`/.hale/` is ignored),
/// so it is never part of the project's own source or its reviews.
const PROVENANCE_REL: &str = ".hale/dna/embedded.digest";

/// GH #566 F8: the host is a Hale program (`dna/host`, embedded beside
/// the core and built once into the toolchain cache). A verb that is
/// DNA behaviour — a projection, a relay, supervision — execs it with
/// the project resolved: `host <verb> <root> <seed> <fleet> <plan> …`,
/// with the toolchain and the toolchain version in the environment.
/// This shim resolves the project (the manifest is the compiler's) and
/// forwards the exit status.
/// The host's command for a verb: the project resolved, the host built
/// once into the toolchain cache, the environment the host expects.
fn host_command(verb: &str, dir: &Path) -> Result<(Command, PathBuf), String> {
    let (root, seed) = project(dir)?;
    let seed_rel = seed.strip_prefix(&root).ok().map(|p| p.to_string_lossy().to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| ".".into());
    let (fleet, plan_rel) = match crate::pkg::read_dna_fleet(&root.join("hale.toml"))? {
        Some((name, path)) => (name, path.strip_prefix(&root).unwrap_or(&path).to_string_lossy().to_string()),
        None => (String::new(), String::new()),
    };
    let cache = hale_iris::materialize(&crate::build_env::host_cache_options()).map_err(|e| format!("cannot materialize the toolchain cache: {e}"))?;
    let host = crate::iris::ensure_built_in(&cache, hale_dna::HOST_SEED, hale_dna::HOST_BIN, "the host")?;
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut cmd = Command::new(&host);
    cmd.arg(verb)
        .arg(&root)
        .arg(&seed_rel)
        .arg(&fleet)
        .arg(&plan_rel)
        .current_dir(&root)
        .env("HALE_BIN", &me)
        // GH #583: the genome a harness must never reach. The
        // organization gets it from the host's child_envs, but a
        // verb that runs a model itself — `hale dna models`, whose
        // probe is its own process — is launched from here, and a
        // confined harness with nothing to mask is refused rather
        // than run, so the probe of a generated catalog failed
        // outside `run`/`dev` (the review's second round, finding 2).
        .env("HALE_DNA_GENOME", &root)
        .env("HALE_DNA_TOOLCHAIN", TOOLCHAIN);
    Ok((cmd, root))
}

/// Run a host verb to completion and return what it printed; its
/// complaint when it failed. For the verbs the driver itself needs
/// (`init` seeding the record, `ui` syncing before it serves).
fn host_run(verb: &str, dir: &Path, args: &[String]) -> Result<String, String> {
    let (mut cmd, _) = host_command(verb, dir)?;
    let out = cmd.args(args).output().map_err(|e| format!("hale dna {verb}: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        // the host prefixes its own complaints; the caller prefixes again
        let err = err.strip_prefix("hale dna: ").map(str::to_string).unwrap_or(err);
        return Err(if err.is_empty() { format!("hale dna {verb} failed") } else { err });
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// What `memory-migrate` found (GH #985): the spine's and the head's DSNs
/// once the schema is applied, or no database to apply it to.
enum MemoryPlan {
    Roles { spine: String, head: String, owners: Vec<String> },
    NoDatabase(String),
}

/// The owner's DSN: used to apply memory's schema, never held by a process
/// that runs the organism.
const OWNER_DSN_ENV: &str = "HALE_DNA_MEMORY_DSN_OWNER";

/// Apply memory's schema with the owner's DSN: HALE_DNA_MEMORY_DSN_OWNER,
/// or the database dna/compose.yaml brings up. The host verb does it; a
/// short-lived process, so the owner's DSN never reaches a host that
/// runs the organism.
fn memory_migrate(dir: &Path) -> Result<MemoryPlan, String> {
    let out = host_run("memory-migrate", dir, &[])?;
    let spine = out.lines().find_map(|l| l.strip_prefix("HALE_DNA_MEMORY_DSN_SPINE="));
    let head = out.lines().find_map(|l| l.strip_prefix("HALE_DNA_MEMORY_DSN_HEAD="));
    if let (Some(spine), Some(head)) = (spine, head) {
        let owners = out.lines().filter(|l| l.starts_with("HALE_DNA_MEMORY_DSN_HEAD_")).map(str::to_string).collect();
        return Ok(MemoryPlan::Roles { spine: spine.to_string(), head: head.to_string(), owners });
    }
    let line = out.trim();
    Ok(MemoryPlan::NoDatabase(line.strip_prefix("none: ").unwrap_or(line).to_string()))
}

/// What `nerves-migrate` found (GH #986): the organization's token and
/// each role's NATS URL once the stream is in place, or no server.
enum NervesPlan {
    Roles(Vec<(String, String)>),
    NoServer(String),
}

/// The nerves' owner (GH #986): used to create the stream, never held by
/// a process that runs the organism.
const NATS_OWNER_ENV: &str = "HALE_DNA_NATS_URL_OWNER";
/// How long a node's drain may take on SIGTERM: its organization's own
/// drain, then the lease given back through the record's remote.
const NODE_DRAIN_GRACE_MS: &str = "30000";

/// What a host that runs the organism is never handed: the owners' and
/// the heads' credentials, for memory and for the nerves.
const NOT_THE_HOSTS: [&str; 4] = [OWNER_DSN_ENV, "HALE_DNA_MEMORY_DSN_HEAD", NATS_OWNER_ENV, "HALE_DNA_NATS_URL_HEAD"];

/// Create the organization's stream with the owner's NATS URL:
/// HALE_DNA_NATS_URL_OWNER, or the server dna/compose.yaml brings up. A
/// short-lived host verb does it, as with memory.
fn nerves_migrate(dir: &Path) -> Result<NervesPlan, String> {
    let out = host_run("nerves-migrate", dir, &[])?;
    let roles: Vec<(String, String)> = out
        .lines()
        .filter(|l| l.starts_with("HALE_DNA_NATS_"))
        .filter_map(|l| l.split_once('=').map(|(k, v)| (k.to_string(), v.to_string())))
        .collect();
    if roles.iter().any(|(k, _)| k == "HALE_DNA_NATS_ORG") {
        return Ok(NervesPlan::Roles(roles));
    }
    let line = out.trim();
    Ok(NervesPlan::NoServer(line.strip_prefix("none: ").unwrap_or(line).to_string()))
}

fn host_exec(verb: &str, dir: &Path, args: &[String]) -> ExitCode {
    host_exec_env(verb, dir, args, &[], &[])
}

/// `host_exec` with settings for the host's own environment.
///
/// GH #887: a verb that has to configure the host used to
/// `std::env::set_var` and let the exec'd image inherit it. Mutating
/// the environment is undefined behaviour in a process that has
/// threads — one table, no lock, every concurrent `getenv` racing it
/// — so the settings travel on the `Command` that becomes the host,
/// beside the four `host_command` already sets. `exec` replaces the
/// image with exactly this `Command`'s environment, so what arrives
/// is unchanged.
fn host_exec_env(
    verb: &str,
    dir: &Path,
    args: &[String],
    env: &[(&str, &OsStr)],
    unset: &[&str],
) -> ExitCode {
    let run = || -> Result<i32, String> {
        let (mut cmd, _) = host_command(verb, dir)?;
        for (k, v) in env {
            cmd.env(k, v);
        }
        // GH #986: a node (`run`, `dev`) is its program's `main locus` and
        // drains on SIGTERM, stopping its organization and giving its lease
        // back before it ends — more than the runtime's default grace
        // allows. An operator's own grace wins.
        if (verb == "run" || verb == "dev") && std::env::var_os("LOTUS_DRAIN_GRACE_MS").is_none() {
            cmd.env("LOTUS_DRAIN_GRACE_MS", NODE_DRAIN_GRACE_MS);
        }
        for k in unset {
            cmd.env_remove(k);
        }
        // exec in place: the pid that ran `hale dna <verb>` IS the host,
        // so a signal to it — a supervisor's, a test's — reaches the host
        // rather than an orphaned child that keeps ticking (dozens of
        // `host node` processes survived their tests before this)
        use std::os::unix::process::CommandExt;
        let e = cmd.args(args).exec();
        Err(format!("hale dna {verb}: {e}"))
    };
    match run() {
        Ok(code) => ExitCode::from(code.clamp(0, 255) as u8),
        Err(e) => {
            eprintln!("hale dna: {e}");
            ExitCode::from(1)
        }
    }
}

/// The `hale node` surface. One text for two callers: the usage
/// error (stderr, exit 2) and `--help` (stdout, exit 0) — GH #817.
pub(crate) fn node_usage() -> &'static str {
    "usage: hale node <name> [--repo <clone>] [--fleet <name>] [--tick <ms>]\n\
     \n\
     The agent that expresses a fleet plan's instances on one machine,\n\
     from the record: it reconciles what the plan says this node runs\n\
     against what is running here (GH #566 F5). With HALE_DNA_NATS_URL_APP\n\
     and HALE_DNA_NATS_ORG set (`hale dna nerves migrate` prints them), it\n\
     hands both to every instance, which says what it says onto the\n\
     nerves itself (GH #986).\n"
}

/// `hale node <name> [--repo <clone>] …`: the node agent is the host's
/// (GH #566 F8); the clone is the project.
pub fn node(args: &[String]) -> ExitCode {
    let mut repo = PathBuf::from(".");
    let mut rest: Vec<String> = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--repo" {
            match it.next() {
                Some(r) => repo = PathBuf::from(r),
                None => {
                    eprintln!("hale node: --repo needs a directory");
                    return ExitCode::from(2);
                }
            }
        } else {
            rest.push(a.clone());
        }
    }
    if rest.iter().all(|a| a.starts_with("--")) {
        eprintln!("hale node: a node has a name");
        eprint!("{}", node_usage());
        return ExitCode::from(2);
    }
    // GH #986: the node's program is the host's; it binds nothing of its
    // own and writes no bus route — an instance says what it says onto
    // the nerves itself (GH #987), and the node hands it the credential
    // it was started with. An instance that does imports pond's NATS
    // client from vendor/dna, which a clone does not carry: materialized
    // here for this toolchain, whatever the clone is (only what changed
    // is written)
    if let Err(e) = materialize_vendor(&repo) {
        eprintln!("hale node: {e}");
        return ExitCode::from(2);
    }
    host_exec_env("node", &repo, &rest, &[], &[])
}

/// The `[project]` positional a verb takes: the first arg that is not a
/// flag and names a directory with a manifest (or any non-flag for verbs
/// whose only positional is the project).
fn project_arg(args: &[String], any_positional: bool) -> (PathBuf, Vec<String>) {
    let mut dir = PathBuf::from(".");
    let mut rest = Vec::new();
    let mut taken = false;
    for a in args {
        if !taken && !a.starts_with("--") && (any_positional || Path::new(a).join("hale.toml").exists() || Path::new(a).is_dir()) {
            dir = PathBuf::from(a);
            taken = true;
        } else {
            rest.push(a.clone());
        }
    }
    (dir, rest)
}

pub fn run(args: &[String]) -> ExitCode {
    // `--help` / `-h` right after a verb asks what `hale dna` takes and
    // starts nothing: `hale dna dev --help` used to read the flag as the
    // project directory and bring the seed's compose up (GH #817's rule
    // — the first argument after a subcommand — at the verb level).
    if matches!(args.first().map(String::as_str), Some(v) if !v.starts_with('-'))
        && matches!(args.get(1).map(String::as_str), Some("--help") | Some("-h"))
    {
        return usage(0);
    }
    match args.first().map(String::as_str) {
        Some("init") => {
            // the directory is the first argument that is not a flag
            let dir = args[1..].iter().find(|a| !a.starts_with("--")).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
            report(init(&dir, !args.iter().any(|a| a == "--no-library")))
        }
        Some("new") => match args.get(1) {
            Some(name) if !name.starts_with("--") => {
                // GH #617: `--profile local|remote-body [--remote <url>] [--body <user@host>]`
                // sets the pieces the combination is detected from; nothing is stored as a label
                let flag = |n: &str| args[2..].windows(2).find(|w| w[0] == n).map(|w| w[1].clone());
                report(new_project(Path::new(name), flag("--profile").as_deref(), flag("--remote").as_deref(), flag("--body").as_deref(), !args.iter().any(|a| a == "--no-library")))
            }
            _ => usage(2),
        },
        Some("upgrade") => {
            let dir = args.get(1).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
            report(upgrade(&dir))
        }
        // GH #985: memory's schema, applied with the owner's DSN
        Some("memory") if args.get(1).map(String::as_str) == Some("migrate") => {
            let dir = args.get(2).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
            match memory_migrate(&dir) {
                Ok(MemoryPlan::Roles { spine, head, owners }) => {
                    println!("HALE_DNA_MEMORY_DSN_SPINE={spine}");
                    println!("HALE_DNA_MEMORY_DSN_HEAD={head}");
                    // GH #1026: over a shared record, each owner's head role
                    for line in owners {
                        println!("{line}");
                    }
                    ExitCode::SUCCESS
                }
                Ok(MemoryPlan::NoDatabase(why)) => {
                    eprintln!("hale dna memory migrate: {why}");
                    ExitCode::from(1)
                }
                Err(e) => {
                    eprintln!("hale dna memory migrate: {e}");
                    ExitCode::from(1)
                }
            }
        }
        // GH #986: the nerves' stream, created with the owner's URL
        Some("nerves") if args.get(1).map(String::as_str) == Some("migrate") => {
            let dir = args.get(2).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
            match nerves_migrate(&dir) {
                Ok(NervesPlan::Roles(roles)) => {
                    for (k, v) in roles {
                        println!("{k}={v}");
                    }
                    ExitCode::SUCCESS
                }
                Ok(NervesPlan::NoServer(why)) => {
                    eprintln!("hale dna nerves migrate: {why}");
                    ExitCode::from(1)
                }
                Err(e) => {
                    eprintln!("hale dna nerves migrate: {e}");
                    ExitCode::from(1)
                }
            }
        }
        // GH #988: the senses' store, compose's `senses` service brought up
        Some("senses") if args.get(1).map(String::as_str) == Some("up") => {
            let dir = args.get(2).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
            match host_run("senses-up", &dir, &[]) {
                Ok(out) if !out.starts_with("none: ") => {
                    print!("{out}");
                    ExitCode::SUCCESS
                }
                Ok(out) => {
                    eprintln!("hale dna senses up: {}", out.trim().strip_prefix("none: ").unwrap_or(out.trim()));
                    ExitCode::from(1)
                }
                Err(e) => {
                    eprintln!("hale dna senses up: {e}");
                    ExitCode::from(1)
                }
            }
        }
        // GH #986: the stream deleted with the owner's URL, as memory's
        // schema is dropped
        Some("nerves") if args.get(1).map(String::as_str) == Some("drop") => {
            let dir = args.get(2).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
            match host_run("nerves-drop", &dir, &[]) {
                Ok(out) if !out.starts_with("none: ") => {
                    print!("{out}");
                    ExitCode::SUCCESS
                }
                Ok(out) => {
                    eprintln!("hale dna nerves drop: {}", out.trim().strip_prefix("none: ").unwrap_or(out.trim()));
                    ExitCode::from(1)
                }
                Err(e) => {
                    eprintln!("hale dna nerves drop: {e}");
                    ExitCode::from(1)
                }
            }
        }
        Some("run") => {
            let (dir, rest) = project_arg(&args[1..], true);
            if let Err(e) = vendor_if_absent(&dir) {
                eprintln!("hale dna run: {e}");
                return ExitCode::from(2);
            }
            // GH #985, #986: the host runs on the spine's DSN and the
            // spine's NATS URL alone; an owner's or a head's in this
            // environment does not reach it
            host_exec_env("run", &dir, &rest, &[], &NOT_THE_HOSTS)
        }
        Some("dev") => {
            let (dir, rest) = project_arg(&args[1..], true);
            if let Err(e) = vendor_if_absent(&dir) {
                eprintln!("hale dna dev: {e}");
                return ExitCode::from(2);
            }
            // GH #985: memory's schema is applied here, with the owner's
            // DSN, and the host that runs the organism is handed only the
            // spine's: the owner's is taken out of its environment
            let spine = match memory_migrate(&dir) {
                Ok(MemoryPlan::Roles { spine, .. }) => Some(spine),
                Ok(MemoryPlan::NoDatabase(why)) => {
                    eprintln!("hale dna dev: {why}");
                    None
                }
                Err(e) => {
                    eprintln!("hale dna dev: memory: {e}");
                    return ExitCode::from(2);
                }
            };
            // GH #986: the nerves' stream is created here, with the
            // owner's URL, and the host is handed the spine's URL and the
            // organization's token — never the owner's, nor the head's
            let nerves: Vec<(String, String)> = match nerves_migrate(&dir) {
                // GH #986, #989: and the application's server and the vault
                // name of its credential, which the host's application
                // inherits (started without the spine's, the owner's and the
                // head's) to say its own events. URLs carry no password; a
                // part presents its credential from the vault by name
                Ok(NervesPlan::Roles(roles)) => roles.into_iter().filter(|(k, _)| k == "HALE_DNA_NATS_ORG" || k == "HALE_DNA_NATS_URL_SPINE" || k == "HALE_DNA_NATS_URL_APP" || k == "HALE_DNA_NATS_USER_APP" || k == "HALE_DNA_NATS_VAULT_APP").collect(),
                Ok(NervesPlan::NoServer(why)) => {
                    eprintln!("hale dna dev: {why}");
                    Vec::new()
                }
                Err(e) => {
                    eprintln!("hale dna dev: nerves: {e}");
                    return ExitCode::from(2);
                }
            };
            // GH #988: the senses' store, with the rest of compose — when
            // compose is where this machine's services come from: an
            // environment that names its own memory or nerves (an owner's
            // DSN or URL) names its own store too. No part is handed the
            // store's URL (the store scrapes them), so one that cannot come
            // up is said and nothing stops for it
            let own_servers = ["HALE_DNA_MEMORY_DSN_OWNER", "HALE_DNA_NATS_URL_OWNER"].iter().any(|k| std::env::var(k).map(|v| !v.is_empty()).unwrap_or(false));
            if !own_servers {
                match host_run("senses-up", &dir, &[]) {
                    Ok(out) if !out.starts_with("none: ") => eprintln!("hale dna dev: senses: {}", out.trim()),
                    Ok(out) => eprintln!("hale dna dev: senses: {}", out.trim().strip_prefix("none: ").unwrap_or(out.trim())),
                    Err(e) => eprintln!("hale dna dev: senses: {e}"),
                }
            }
            let mut env: Vec<(&str, &OsStr)> = Vec::new();
            if let Some(spine) = &spine {
                env.push(("HALE_DNA_MEMORY_DSN_SPINE", OsStr::new(spine.as_str())));
            }
            for (k, v) in &nerves {
                env.push((k.as_str(), OsStr::new(v.as_str())));
            }
            host_exec_env("dev", &dir, &rest, &env, &NOT_THE_HOSTS)
        }
        // the plan path a fleet name resolves to in this checkout's manifest
        // (a node asks, at the revision it checked out)
        Some("plan-of") => {
            let root = args.get(1).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
            let name = args.get(2).cloned().unwrap_or_default();
            let manifest = root.join("hale.toml");
            let found = match crate::pkg::read_dna_fleet(&manifest) {
                Ok(Some((n, p))) if name.is_empty() || n == name => Some(p),
                _ => crate::pkg::read_fleets(&manifest).ok().and_then(|f| f.get(&name).map(|rel| root.join(rel))),
            };
            match found {
                Some(p) => {
                    println!("{}", p.strip_prefix(&root).unwrap_or(&p).display());
                    ExitCode::SUCCESS
                }
                None => ExitCode::from(1),
            }
        }
        Some("status") => {
            let (dir, rest) = project_arg(&args[1..], true);
            // GH #726: which embedded DNA source this toolchain
            // carries, and whether vendor/dna came from the same one.
            // Not in `--json`: that form is parsed as one JSON
            // document, and the projection is the host's.
            if !rest.iter().any(|a| a == "--json") {
                print_embedded_provenance(&dir);
            }
            host_exec("status", &dir, &rest)
        }
        // GH #726: the digest of the DNA source THIS binary embeds —
        // the only output, so a fixture can compare it with the
        // working tree's (`--from-tree <dir>`, a checkout holding
        // `dna/core`) before it trusts a mutation result.
        Some("--embedded-digest") => embedded_digest_cmd(&args[1..]),
        // `hale dna task create [--to <locus>] [--as <who>] [--judgment] [--no-wait] <outcome…>`
        // (asking is one kind of task; the face exposes the same operation
        // as `dna.task.create`), and GH #596 W: `hale dna task done <id> …`
        Some("task") => host_exec("task", Path::new("."), &args[1..]),
        // GH #604 rule 5: `hale dna retire <who> [--to <successor>]`
        Some("retire") => host_exec("retire", Path::new("."), &args[1..]),
        // GH #604 rule 3: `hale dna effect resolve <key> --outcome ok|failed`
        Some("effect") => host_exec("effect", Path::new("."), &args[1..]),
        // GH #1086: `hale dna show org | processes [--json] [project]`, the
        // two perspectives over the graph, read from memory
        Some("show") => match args.get(1).map(String::as_str) {
            Some(what @ ("org" | "processes")) => {
                // `--json` and at most one project directory, nothing else:
                // a flag or a path this verb does not take is said, never
                // ignored (the review of #1120)
                let mut dir = PathBuf::from(".");
                let mut forwarded = vec![what.to_string()];
                let mut project = false;
                for a in &args[2..] {
                    if a == "--json" {
                        forwarded.push(a.clone());
                    } else if a.starts_with('-') {
                        eprintln!("hale dna show: `{a}` is not a flag of show (--json)\nusage: hale dna show org | processes [--json] [project]");
                        return ExitCode::from(2);
                    } else if project {
                        eprintln!("hale dna show: one project at most (`{a}` is a second)\nusage: hale dna show org | processes [--json] [project]");
                        return ExitCode::from(2);
                    } else if !Path::new(a).is_dir() {
                        eprintln!("hale dna show: `{a}` is not a directory (the project)");
                        return ExitCode::from(2);
                    } else {
                        project = true;
                        dir = PathBuf::from(a);
                    }
                }
                host_exec("show", &dir, &forwarded)
            }
            _ => {
                eprintln!("usage: hale dna show org | processes [--json] [project]");
                ExitCode::from(2)
            }
        },
        // GH #1087: `hale dna route [--json] (<path>… | --diff <range>)`, the
        // positions a change set must be signed by, from the graph
        Some("route") => {
            // a path is the repository's: one given from a subdirectory is
            // taken from there (GH #1087 review), whatever directory the
            // host reads the record from
            // the organization's root: the nearest directory holding
            // `dna/org`, which a seed of a repository (a `hale.toml` of its
            // own) is not
            let cwd = std::env::current_dir().ok().and_then(|c| c.canonicalize().ok()).unwrap_or_default();
            let root = cwd.ancestors().find(|a| a.join("dna/org").is_dir()).map(|a| a.to_path_buf()).unwrap_or_else(|| cwd.clone());
            let under = cwd.strip_prefix(&root).map(|p| p.to_path_buf()).unwrap_or_default();
            let mut rest = Vec::new();
            let mut it = args[1..].iter();
            while let Some(a) = it.next() {
                if a == "--diff" {
                    rest.push(a.clone());
                    if let Some(v) = it.next() {
                        rest.push(v.clone());
                    }
                } else if a.starts_with('-') {
                    rest.push(a.clone());
                } else {
                    rest.push(repo_path(&root, &under, a));
                }
            }
            host_exec("route", &root, &rest)
        }
        // GH #1091: `hale dna fill <position> <holder> [project] [--as <who>]`,
        // a holder asked of the organization, which proposes it to the Board
        Some("fill") => {
            let mut words = Vec::new();
            let mut forwarded = Vec::new();
            let mut dir = PathBuf::from(".");
            let mut rest = args[1..].iter();
            while let Some(a) = rest.next() {
                if a == "--as" {
                    forwarded.push(a.clone());
                    if let Some(v) = rest.next() {
                        forwarded.push(v.clone());
                    }
                } else if a.starts_with('-') {
                    eprintln!("hale dna fill: `{a}` is not a flag of fill (--as)\nusage: hale dna fill <position> <holder> [project] [--as <who>]");
                    return ExitCode::from(2);
                } else if words.len() < 2 {
                    words.push(a.clone());
                } else if words.len() == 2 && Path::new(a).is_dir() && dir == PathBuf::from(".") {
                    dir = PathBuf::from(a);
                } else {
                    eprintln!("hale dna fill: `{a}` is neither a position, a holder nor a project directory\nusage: hale dna fill <position> <holder> [project] [--as <who>]");
                    return ExitCode::from(2);
                }
            }
            if words.len() < 2 {
                eprintln!("usage: hale dna fill <position> <holder> [project] [--as <who>]");
                return ExitCode::from(2);
            }
            words.extend(forwarded);
            host_exec("fill", &dir, &words)
        }
        Some("history") => {
            let (dir, rest) = project_arg(&args[1..], false);
            host_exec("history", &dir, &rest)
        }
        Some("body") => host_exec("body", Path::new("."), &args[1..]),
        Some("secret") => host_exec("secret", Path::new("."), &args[1..]),
        // GH #989: every secret the organism requires, and whether the vault holds it
        Some("secrets") => {
            let dir = args.get(1).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
            host_exec("secrets", &dir, &[])
        }
        // GH #989: the attached application removed, its broker account revoked
        Some("application") if args.get(1).map(String::as_str) == Some("remove") => {
            let (dir, rest) = project_arg(&args[2..], false);
            host_exec("application-remove", &dir, &rest)
        }
        Some("schedule") => host_exec("schedule", Path::new("."), &args[1..]),
        Some("receipt") => host_exec("receipt", Path::new("."), &args[1..]),
        // GH #615: connections to other records, and what crosses them
        Some("connect") => host_exec("connect", Path::new("."), &args[1..]),
        // GH #602: a person proposes a practice for the Board
        Some("practice") => host_exec("practice", Path::new("."), &args[1..]),
        Some("disconnect") => host_exec("disconnect", Path::new("."), &args[1..]),
        Some("handoff") => host_exec("handoff", Path::new("."), &args[1..]),
        Some("profile") => {
            let (dir, rest) = project_arg(&args[1..], true);
            host_exec("profile", &dir, &rest)
        }
        Some("sync") => {
            let (dir, rest) = project_arg(&args[1..], true);
            host_exec("sync", &dir, &rest)
        }
        // #649: the candidates the record keeps, whatever the review decided
        Some("candidates") => host_exec("candidates", Path::new("."), &args[1..]),
        // #650: the operational memory, adopted by an explicit step
        Some("ledger") => host_exec("ledger", Path::new("."), &args[1..]),
        // #652: requests kept while the service cannot be reached
        Some("board") => {
            let (dir, rest) = project_arg(&args[1..], true);
            host_exec("board", &dir, &rest)
        }
        Some("report") => {
            let (dir, rest) = project_arg(&args[1..], true);
            host_exec("report", &dir, &rest)
        }
        Some("pressure") if args.get(1).map(|a| a == "raise").unwrap_or(false) => host_exec("raise", Path::new("."), &args[2..]),
        Some("pressure") => host_exec("pressure", Path::new("."), &args[1..]),
        Some("concern") if args.get(1).map(|a| a == "raise").unwrap_or(false) => host_exec("concern-raise", Path::new("."), &args[2..]),
        Some("github") => host_exec("github", Path::new("."), &args[1..]),
        Some("fleet") => {
            let (dir, rest) = project_arg(&args[1..], true);
            host_exec("fleet", &dir, &rest)
        }
        Some("ui") => ui_cmd(&args[1..]),
        Some("models") => {
            let (dir, rest) = project_arg(&args[1..], true);
            host_exec("models", &dir, &rest)
        }
        // GH #946: a leg's verbs, as the project's own program over the legs seed
        Some("work") => {
            // the project is only ever the first argument, a directory with a
            // manifest; the verb and every flag's value are the leg's
            let (dir, rest) = match args.get(1) {
                Some(p) if !p.starts_with("--") && Path::new(p).join("hale.toml").exists() => (PathBuf::from(p), args[2..].to_vec()),
                _ => (PathBuf::from("."), args[1..].to_vec()),
            };
            work_exec(&dir, &rest)
        }
        // GH #995: the workflow catalog (dna/org/workflows.hl)
        Some("definitions") => {
            let (dir, rest) = project_arg(&args[1..], true);
            host_exec("definitions", &dir, &rest)
        }
        Some("deploy") => host_exec("deploy", Path::new("."), &args[1..]),
        Some("rollback") => host_exec("rollback", Path::new("."), &args[1..]),
        // a list or a render, or a verdict: the host's (GH #566 F8)
        Some("review") if args[1..].iter().filter(|a| !a.starts_with("--")).count() >= 2 => host_exec("verdict", Path::new("."), &args[1..]),
        Some("review") => host_exec("review", Path::new("."), &args[1..]),
        Some("--help") | Some("-h") | None => usage(if args.is_empty() { 2 } else { 0 }),
        Some(other) => {
            eprintln!("hale dna: unknown subcommand `{other}`");
            usage(2)
        }
    }
}

/// GH #946: the host builds the project's leg (once per key) and answers
/// its path; the leg is then exec'd in place with the verb's arguments,
/// so it is the process the caller sees — its lines stream as it prints
/// them, and a signal to it drains a worker loop and its children.
fn work_exec(dir: &Path, args: &[String]) -> ExitCode {
    let built = match host_run("work", dir, &[]) {
        Ok(text) => text,
        Err(e) => {
            eprintln!("hale dna work: {e}");
            return ExitCode::from(1);
        }
    };
    let leg = serde_json::from_str::<Value>(built.trim())
        .ok()
        .and_then(|v| v.get("leg").and_then(Value::as_str).map(str::to_string));
    let Some(leg) = leg else {
        eprint!("{built}");
        return ExitCode::from(1);
    };
    let mut cmd = Command::new(&leg);
    cmd.args(args).current_dir(dir);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let e = cmd.exec();
        eprintln!("hale dna work: cannot run the leg {leg}: {e}");
        ExitCode::from(1)
    }
    #[cfg(not(unix))]
    {
        match cmd.status() {
            Ok(st) => ExitCode::from(st.code().unwrap_or(1) as u8),
            Err(e) => {
                eprintln!("hale dna work: cannot run the leg {leg}: {e}");
                ExitCode::from(1)
            }
        }
    }
}

/// GH #726 — `hale dna --embedded-digest [--from-tree <dir>]`: the
/// name of a DNA source set, and nothing else on stdout. Without a
/// tree it is what this binary embeds; with one it is what `<dir>`
/// would embed if the toolchain were built from it, by the same
/// algorithm, so a fixture can refuse to believe a mutation run
/// against a binary that predates its edit.
fn embedded_digest_cmd(rest: &[String]) -> ExitCode {
    let mut tree: Option<String> = None;
    let mut i = 0;
    while i < rest.len() {
        if rest[i] == "--from-tree" {
            match rest.get(i + 1) {
                Some(d) => tree = Some(d.clone()),
                None => {
                    eprintln!("hale dna --embedded-digest --from-tree <dir>: name the checkout holding dna/core");
                    return ExitCode::from(2);
                }
            }
            i += 2;
            continue;
        }
        if let Some(d) = rest[i].strip_prefix("--from-tree=") {
            tree = Some(d.to_string());
            i += 1;
            continue;
        }
        eprintln!("hale dna --embedded-digest: unknown argument `{}`", rest[i]);
        return ExitCode::from(2);
    }
    match tree {
        None => {
            println!("{}", hale_dna::EMBEDDED_DIGEST);
            ExitCode::SUCCESS
        }
        Some(dir) => match hale_dna::digest_of_tree(Path::new(&dir)) {
            Ok(d) => {
                println!("{d}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("hale dna --embedded-digest --from-tree {dir}: {e}");
                ExitCode::from(1)
            }
        },
    }
}

/// The provenance line `hale dna status` opens with: what this
/// toolchain embeds, and — when `vendor/dna` was materialized from
/// another source set — that it is stale. Printed before the host is
/// exec'd, so stdout is flushed by hand.
fn print_embedded_provenance(dir: &Path) {
    use std::io::Write as _;
    let mut line = format!("embedded dna: {} (hale {TOOLCHAIN})", hale_dna::embedded_short());
    let root = dir
        .canonicalize()
        .map(|d| crate::shared::workspace::find_workspace_root_pub(&d).unwrap_or(d))
        .unwrap_or_else(|_| dir.to_path_buf());
    if let Some(was) = materialized_digest(&root) {
        if was != hale_dna::EMBEDDED_DIGEST {
            let short: String = was.chars().take(16).collect();
            line.push_str(&format!("; vendor/dna was materialized from {short} — run `hale dna upgrade`"));
        }
    }
    println!("{line}");
    let _ = std::io::stdout().flush();
}

fn usage(code: u8) -> ExitCode {
    eprintln!("usage: hale dna init [app-dir] [--no-library]");
    eprintln!("                                    attach the DNA to an existing application; the record starts with the language and system");
    eprintln!("                                    nodes and the toolchain's library proposed, one Review per family (--no-library leaves out the nodes and the library, not the practices)");
    eprintln!("       hale dna new <name> [--no-library]");
    eprintln!("                                    a greenfield application with its DNA");
    eprintln!("       hale dna upgrade [dir]       re-materialize vendor/dna for this toolchain, and propose this version's practices and library");
    eprintln!("                                    (each changed practice, and the library as new families, supersede the active one once the Board approves)");
    eprintln!("       hale dna memory migrate [dir]");
    eprintln!("                                    apply memory's schema with the owner's DSN (HALE_DNA_MEMORY_DSN_OWNER, or dna/compose.yaml)");
    eprintln!("                                    and print the record's spine and head DSNs (HALE_DNA_MEMORY_DSN_SPINE, …_HEAD)");
    eprintln!("       hale dna nerves migrate [dir]");
    eprintln!("                                    create the organization's NATS JetStream stream with the owner's URL (HALE_DNA_NATS_URL_OWNER,");
    eprintln!("                                    or dna/compose.yaml) and print its token and each role's URL (HALE_DNA_NATS_ORG, …_URL_SPINE)");
    eprintln!("       hale dna nerves drop [dir]   delete the organization's stream, and everything it held, with the owner's URL");
    eprintln!("       hale dna senses up [dir]     bring up the senses' store (compose's `senses` service) and print its read URL");
    eprintln!("       hale dna --embedded-digest [--from-tree <dir>]");
    eprintln!("                                    the digest of the DNA source this binary embeds (nothing else on stdout);");
    eprintln!("                                    with a checkout, what that tree would embed — a mismatch means the binary");
    eprintln!("                                    predates the working tree and a mutation run against it proves nothing");
    eprintln!("       hale dna models [project]    the catalog (dna/org/models.hl): every backend, and one small request to each");
    eprintln!("       hale dna work <verb> …       a leg's verbs against the head's API (--api, --as position:<name>): next, brief,");
    eprintln!("                                    renew, allowance, submit, settle, release, friction, run — the project's performers (dna/org/work.hl);");
    eprintln!("                                    loop --parallel N is a worker: N children, each its own holder; loop --drain ends one");
    eprintln!("       hale dna definitions [project] [--json]");
    eprintln!("                                    the workflow catalog (dna/org/workflows.hl): each definition's revisions and every step's store");
    eprintln!("       hale dna run [project] [--port N] [--no-iris]");
    eprintln!("                                    build and run the organization (dna/org), a node of it: relay the record's requests");
    eprintln!("                                    to it over the nerves (NATS); iris inspects its process");
    eprintln!("       hale dna dev [project] [--port N] [--no-iris] [--observe <secs>]");
    eprintln!("                                    the organization AND the application under one host: rebuild and restart");
    eprintln!("                                    the application on an apply, watch the window, report back");
    eprintln!("       hale dna status [project] [--json]");
    eprintln!("                                    the organism's status projection, from the Journal");
    eprintln!("       hale dna history [<entity>]  walk the Journal by causal links (works offline)");
    eprintln!("       hale dna fill <position> <holder> [project] [--as <who>]");
    eprintln!("                                    ask the organization to propose who holds a position, for the Board");
    eprintln!("       hale dna route [--json] (<path>… | --diff <range>)");
    eprintln!("                                    who must sign a change set, and the gates it is judged against");
    eprintln!("       hale dna show org|processes [--json] [project]");
    eprintln!("                                    the org chart and the process model, as queries over memory");
    eprintln!("       hale dna sync [project]      fetch, reconcile and push the record (refs/dna/*) with origin");
    eprintln!("       hale dna ledger [status | rows | adopt | abandon --why <w>]");
    eprintln!("                                    the operational memory: where the day's work lives, its rows as JSON lines, and the one-way");
    eprintln!("                                    move of it into memory — adopt and abandon are asked in the record; a node carries them out on its tick");
    eprintln!("                                    once the ledger is adopted, a head's write (a task done, a receipt filed …) goes straight into it");
    eprintln!("                                    under the head's role: done when it lands, or refused by memory with the reason");
    eprintln!("       hale dna candidates [<mutation> | drop <mutation> --why <w>]");
    eprintln!("                                    the candidates the record keeps, whatever the review decided; one as a diff; stop keeping one");
    eprintln!("       hale dna new <name> [--profile local|remote-body --remote <url> [--body <user@host>]]");
    eprintln!("                                    the profile sets the pieces (a remote, a body host); the combination is always detected");
    eprintln!("       hale dna profile [project]   the organism's combination, detected from its pieces: record, body, head, fleet, knowledge, trust");
    eprintln!("       hale dna body                who runs this record (the body lease); `body claim --force` takes it from a body that is gone;");
    eprintln!("                                    `body release [--force]` gives it up — both are rows in your name (--as <who>)");
    eprintln!("       hale dna body provision <user@host> [--dsn <postgres://…>] [--dir <path>] [--dry-run]");
    eprintln!("                                    over ssh: the toolchain hale.lock pins, the record's remote cloned, Postgres from");
    eprintln!("                                    dna/compose.yaml or the DSN, a systemd user unit supervising the host; writes nothing");
    eprintln!("                                    when ssh or the toolchain is unavailable. Then `body start|stop|logs [--body <user@host>]`");
    eprintln!("       hale dna receipt [disclose <digest> --to <who> --purpose <p> | show <digest> --purpose <p>]");
    eprintln!("       hale dna receipt hold|release-hold <digest> --why <w> | redact <digest> --why <w> --policy <p>");
    eprintln!("       hale dna receipt file <path> [--class internal|customer|confidential]   file a document as evidence");
    eprintln!("                                    a hold refuses redaction until released; a redaction removes the body and keeps the digest, as a row");
    eprintln!("                                    (a body kept in memory is erased by the body on its tick, and filing it again is refused)");
    eprintln!("                                    protected evidence (customer, confidential): kept in memory alone, sealed there;");
    eprintln!("                                    disclosure and every read are rows in the reader's name (--as <who>)");
    eprintln!("       hale dna schedule [pause <id> | resume <id>]");
    eprintln!("                                    the schedules the record declares (a definition on an interval or a cron, and who");
    eprintln!("                                    convenes it) and their occurrences; pause and resume are rows in your name (--as <who>)");
    eprintln!("       hale dna schedule declare <id> (--every <n>ms|s|m|h|d | --cron <expr>) --definition <id> --convener <position> [--args <json>]");
    eprintln!("                                    a schedule asked of the organization, which declares it or refuses it; an occurrence");
    eprintln!("                                    is an execution of the definition (GH #1143)");
    eprintln!("       hale dna secret set <NAME> [--body <user@host>]");
    eprintln!("                                    a credential from stdin (never argv, never the record) into its slot of the vault");
    eprintln!("                                    (a model key's, FORGE_TOKEN, or OIDC_CLIENT_SECRET) on the body or here; `secret rotate <NAME>`; the record gets `secret.rotated <NAME>` only");
    eprintln!("       hale dna secrets [dir]       every secret the organism requires, and whether the vault holds it (never a value)");
    eprintln!("       hale dna application remove [project] [--as <who>]");
    eprintln!("                                    the attached application removed (`application.detached` in the record) and its");
    eprintln!("                                    broker account revoked: its user, its password and its vault entry (GH #989)");
    eprintln!("       hale dna board [project]     the Board's queue: what needs its verdict, escalations, proposals, reports");
    eprintln!("       hale dna task create [--to <locus>] [--as <who>] [--judgment] [--no-wait] <outcome…>");
    eprintln!("                                    ask for an outcome (--judgment: an assessment, a leg's to perform): a row in the record, which a node relays to the organism; prints the Task born or the refusal");
    eprintln!("                                    (on an adopted ledger it prints the request's digest: see `hale dna ledger`)");
    eprintln!("       hale dna task done <id>      a person reports a handed Task done (--as <who>, --note …); `task reassign <id> --to <who>`");
    eprintln!("                                    under an acceptance practice requiring evidence: --evidence <digest>, or --exception <why> --authorized-by <who>");
    eprintln!("       hale dna task authorize <id> --exception <why>   authorize an exception, in your name (not the assignee's)");
    eprintln!("       hale dna task decide <id>    report a decision someone else made (--decided-by <party> --via <channel> --evidence <digest>, --as <reporter>)");
    eprintln!("       hale dna retire <who>        a person retires: the handed Tasks they hold move to --to <successor>, as rows");
    eprintln!("       hale dna practice propose <name> --text <text> [--because <why>] [--supersedes <digest>]");
    eprintln!("                                    propose a practice for the Board to ratify (a knowledge Review); `hale dna practice` lists them");
    eprintln!("       hale dna connect <record-url> --name <n> --as <position> --purpose <p> --classes <internal,customer,…>");
    eprintln!("                                    propose a connection to another record (a Board Review); `hale dna connect` lists them;");
    eprintln!("                                    `hale dna disconnect <n> --why <w>` closes one");
    eprintln!("       hale dna handoff <n> task <id> | receipt <digest>");
    eprintln!("                                    write one fact into the connected record, with origin, lineage and purpose;");
    eprintln!("                                    `handoff` lists, `handoff accept <id>` accepts one received, `handoff sync` reads acceptances back");
    eprintln!("       hale dna effect resolve <key> an effect whose outcome is unknown after a restart: --outcome ok|failed, in your name");
    eprintln!("       hale dna report [project]    file a report from the record since the last one (report.filed)");
    eprintln!("       hale dna github sync         mirror pending Reviews to pull requests and read their reviews back as verdicts");
    eprintln!("                                    (git config dna.github owner/repo; dna.github.board logins,…; needs `gh`)");
    eprintln!("       hale dna fleet [project]     what the fleet expresses: every instance, its node, revision, model hash, state");
    eprintln!("       hale dna deploy <revision>   express a genome revision through the fleet's nodes (fleet.deploy)");
    eprintln!("       hale dna rollback <mutation> express the base a Mutation was applied on, again");
    eprintln!("                                    (`[dna] fleet = \"<name>\"` in hale.toml names the plan; `hale node <name>` runs a node)");
    eprintln!("       hale dna pressure [raise <source> <what…>]");
    eprintln!("                                    pressure raised and answered; `raise` writes one signal into the record, which a node relays");
    eprintln!("       hale dna concern raise <source> <what…> [--severity N]");
    eprintln!("                                    a concern from a locus path about the part above it; persistent ones become knowledge proposals");
    eprintln!("       hale dna ui [project] [--port N]");
    eprintln!("                                    under `git config dna.principal oidc` a hosted head: sign-in through dna.oidc.issuer,");
    eprintln!("                                    subjects mapped by dna.oidc.member, the secret the vault's oidc-client-<client>;");
    eprintln!("                                    with no principal source it refuses to start (trusted-local is a test fixture's mode)");
    eprintln!("                                    the DNA surface in a browser, from the record alone: the Board's queue, the Reviews");
    eprintln!("                                    with their three views, the fleet, the history; verdicts, intent and pressure from forms");
    eprintln!("       hale dna review              the pending Reviews");
    eprintln!("       hale dna review <id> [--iris] render a Review: source diff, semantic diff, evidence, and the knowledge ratified for");
    eprintln!("                                    the node it is bound to (or system:dna for a change to the organization) when memory is there");
    eprintln!("       hale dna review <family> approve|reject");
    eprintln!("                                    decide every pending Review of a seeded family in turn: purpose, design, operating, using, library (a library family is one Review for all its ideas)");
    eprintln!("       hale dna review <id> approve|revise|reject|abstain [--as <reviewer>] [--authority <a>] [--comment <c>] [--digest <sha>] [--no-wait]");
    eprintln!("                                    write a verdict into the record, which a node relays; the Review decides");
    if code == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(code)
    }
}

fn report(r: Result<Vec<String>, String>) -> ExitCode {
    match r {
        Ok(lines) => {
            for l in lines {
                println!("{l}");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("hale dna: {e}");
            ExitCode::from(1)
        }
    }
}

// ---------------------------------------------------------------
// vendor/dna + hale.lock
// ---------------------------------------------------------------

pub const TOOLCHAIN: &str = env!("CARGO_PKG_VERSION");

/// Write `vendor/dna/<file>.hl` for every core file; returns
/// (written, unchanged).
/// A clone of a project carries no `vendor/dna` (it is gitignored and
/// toolchain-managed): a body started in a fresh clone — a second
/// owner's over a shared record (GH #665) — gets the core materialized
/// first, as `new` and `upgrade` do. Present, it is left as it is.
fn vendor_if_absent(root: &Path) -> Result<(), String> {
    if root.join("dna/org/main.hl").is_file() && !root.join("vendor/dna").is_dir() {
        materialize_vendor(root)?;
    }
    Ok(())
}

fn materialize_vendor(root: &Path) -> Result<(usize, usize), String> {
    let dir = root.join("vendor/dna");
    fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let mut written = 0;
    let mut same = 0;
    for f in hale_dna::FILES {
        let name = f.path.rsplit('/').next().unwrap_or(f.path);
        let p = dir.join(name);
        if fs::read_to_string(&p).map(|s| s == f.content).unwrap_or(false) {
            same += 1;
            continue;
        }
        fs::write(&p, f.content).map_err(|e| format!("write {}: {e}", p.display()))?;
        written += 1;
    }
    // GH #985: the core opens memory through pond's driver, which it
    // imports as `./pond/{db,pq}`: vendored beside it, under vendor/dna
    for f in hale_dna::POND_FILES {
        let rel = f.path.strip_prefix("dna/core/").unwrap_or(f.path);
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
        }
        if fs::read_to_string(&p).map(|s| s == f.content).unwrap_or(false) {
            same += 1;
            continue;
        }
        fs::write(&p, f.content).map_err(|e| format!("write {}: {e}", p.display()))?;
        written += 1;
    }
    // GH #946: the legs seed, beside the core it imports as `..`, so a
    // leg (`hale dna work`) builds from vendor/dna/legs
    for f in hale_dna::LEGS_FILES {
        let rel = f.path.strip_prefix("dna/core/").unwrap_or(f.path);
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
        }
        if fs::read_to_string(&p).map(|s| s == f.content).unwrap_or(false) {
            same += 1;
            continue;
        }
        fs::write(&p, f.content).map_err(|e| format!("write {}: {e}", p.display()))?;
        written += 1;
    }
    // The vendored core says which source set it came from (GH #726):
    // a version alone does not identify it, so the digest of the
    // embedded set is written beside it and refreshed on `upgrade`.
    let readme = dir.join("README.md");
    let readme_text = format!(
        "# vendor/dna — toolchain-owned\n\nThe DNA core (`dna/core` of the hale repository) as shipped by hale {TOOLCHAIN}.\nEmbedded source digest: {}\n(`hale dna --embedded-digest`; two builds of one version can embed different\nsource, so the digest is the provenance and the version is not.)\nRegenerated by `hale dna init` / `hale dna upgrade`; pinned in `hale.lock` as\n`[dna] toolchain`. Do not edit: project-owned source lives in `dna/`.\n",
        hale_dna::EMBEDDED_DIGEST
    );
    if fs::read_to_string(&readme).map(|s| s != readme_text).unwrap_or(true) {
        let _ = fs::write(&readme, &readme_text);
    }
    // …and machine-readably, for a fixture or a `status` that asks
    // which toolchain last materialized this tree. Same shape as
    // `hale --version`, with all 64 digits.
    let prov = root.join(PROVENANCE_REL);
    if let Some(parent) = prov.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(&prov, format!("hale {TOOLCHAIN}\nembedded dna: {}\n", hale_dna::EMBEDDED_DIGEST));
    pin_lock(root)?;
    Ok((written, same))
}

/// The digest recorded when `vendor/dna` was last materialized under
/// `root`, when the record is there.
fn materialized_digest(root: &Path) -> Option<String> {
    let text = fs::read_to_string(root.join(PROVENANCE_REL)).ok()?;
    text.lines().find_map(|l| l.strip_prefix("embedded dna: ")).map(|d| d.trim().to_string())
}

fn pin_lock(root: &Path) -> Result<(), String> {
    let lock_path = root.join("hale.lock");
    let mut lock = crate::pkg::read_lockfile(&lock_path)?;
    lock.dna = Some(crate::pkg::DnaLock { toolchain: TOOLCHAIN.to_string() });
    let text = toml::to_string_pretty(&lock).map_err(|e| format!("serialize hale.lock: {e}"))?;
    fs::write(&lock_path, text).map_err(|e| format!("write {}: {e}", lock_path.display()))
}

// ---------------------------------------------------------------
// init
// ---------------------------------------------------------------

struct App {
    root: PathBuf,
    seed: PathBuf,
    /// The app seed relative to the root, `.` when they coincide.
    seed_rel: String,
    main_file: PathBuf,
    main_name: String,
    project: String,
}

fn locate(app_dir: &Path) -> Result<App, String> {
    let seed = app_dir
        .canonicalize()
        .map_err(|e| format!("{}: {e}", app_dir.display()))?;
    if !seed.is_dir() {
        return Err(format!("{} is not a directory (the app seed)", seed.display()));
    }
    let root = crate::shared::workspace::find_workspace_root_pub(&seed).unwrap_or_else(|| seed.clone());
    let seed_rel = seed
        .strip_prefix(&root)
        .map(|p| {
            let s = p.to_string_lossy().to_string();
            if s.is_empty() {
                ".".to_string()
            } else {
                s
            }
        })
        .unwrap_or_else(|_| ".".to_string());
    let Some((main_file, main_name)) = main_of(&seed)? else {
        return Err(format!(
            "{} declares no `main locus` — the DNA attaches to an application's entrypoint; run `hale dna new <name>` for a fresh one",
            seed.display()
        ));
    };
    let project = root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "app".to_string());
    Ok(App { root, seed, seed_rel, main_file, main_name, project })
}

/// The seed's entry — its file and name — by the entry row over every
/// .hl in it that parses (F.40 phase 3, as `seed_entry_kind` reads it:
/// no import is resolved, and none holds the entry); none when no file
/// declares a top-level `main locus`.
fn main_of(seed: &Path) -> Result<Option<(PathBuf, String)>, String> {
    let mut entries: Vec<PathBuf> = fs::read_dir(seed)
        .map_err(|e| format!("{}: {e}", seed.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("hl"))
        .collect();
    entries.sort();
    let mut parsed: Vec<(PathBuf, hale_syntax::ast::Program)> = Vec::new();
    for p in entries {
        let src = fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
        if let Ok(prog) = hale_syntax::parse_source(&src) {
            parsed.push((p, prog));
        }
    }
    let programs: Vec<&hale_syntax::ast::Program> = parsed.iter().map(|(_, prog)| prog).collect();
    let row = hale_types::entry::entry_row_in(&programs);
    Ok(row.entry().and_then(|m| Some((parsed[m.index_in()?.0].0.clone(), m.name.clone()))))
}

/// GH #1090: `init` on a repository rather than one application — a
/// directory with no Hale source of its own that is no workspace's seed
/// (voice's root, with a seed per process). A directory holding `.hl`
/// files is a seed, and one without a main locus is refused as before.
/// The organization is the repository's; the record is seeded with what
/// the repository holds, as the graph, instead of one application's
/// topology.
fn repository_root(dir: &Path) -> Result<Option<PathBuf>, String> {
    let at = dir.canonicalize().map_err(|e| format!("{}: {e}", dir.display()))?;
    if !at.is_dir() {
        return Ok(None);
    }
    let source = fs::read_dir(&at)
        .map_err(|e| format!("{}: {e}", at.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .any(|p| p.extension().and_then(|x| x.to_str()) == Some("hl"));
    if source {
        return Ok(None);
    }
    match crate::shared::workspace::find_workspace_root_pub(&at) {
        Some(root) if root != at => Ok(None),
        _ => Ok(Some(at)),
    }
}

fn init(app_dir: &Path, library: bool) -> Result<Vec<String>, String> {
    // an application, or (GH #1090) a repository with none at its root
    let app = match repository_root(app_dir)? {
        Some(_) => None,
        None => Some(locate(app_dir)?),
    };
    let root = match &app {
        Some(a) => a.root.clone(),
        None => app_dir.canonicalize().map_err(|e| format!("{}: {e}", app_dir.display()))?,
    };
    let project = match &app {
        Some(a) => a.project.clone(),
        None => root.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "project".to_string()),
    };
    let mut out: Vec<String> = Vec::new();
    // 0. the record is a branch, so the project is a repository
    if ensure_repo(&root)? {
        out.push(format!("git init {} (the record lives on {RECORD_REF})", root.display()));
    }
    let created = |out: &mut Vec<String>, p: &Path, content: &str| -> Result<bool, String> {
        if p.exists() {
            out.push(format!("kept    {} (already exists)", p.display()));
            return Ok(false);
        }
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
        }
        fs::write(p, content).map_err(|e| format!("write {}: {e}", p.display()))?;
        out.push(format!("created {}", p.display()));
        Ok(true)
    };

    // 1. the manifest
    let manifest = root.join("hale.toml");
    if !manifest.exists() {
        created(&mut out, &manifest, "[deps]\n")?;
    }
    // 2. the toolchain-owned core
    let (w, same) = materialize_vendor(&root)?;
    out.push(format!(
        "{} vendor/dna ({} file(s) written, {} unchanged; hale.lock pins toolchain {}, embedded dna {})",
        if w > 0 { "wrote  " } else { "kept   " },
        w,
        same,
        TOOLCHAIN,
        hale_dna::embedded_short()
    ));
    // 3. the artifact, cut by the toolchain as a subprocess — an
    //    application's; a repository's structure is read at step 7
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let cut = match &app {
        Some(app) => {
            let baseline = app.root.join(BASELINE_REL);
            fs::create_dir_all(baseline.parent().unwrap()).map_err(|e| e.to_string())?;
            let st = Command::new(&me)
                .arg("check")
                .arg(&app.seed)
                .arg(format!("--dump-topology={}", baseline.display()))
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::inherit())
                .status()
                .map_err(|e| format!("hale check: {e}"))?;
            let raw = fs::read_to_string(&baseline).map_err(|_| {
                format!(
                    "hale check {} produced no artifact (exit {}); fix the application first",
                    app.seed.display(),
                    st.code().unwrap_or(-1)
                )
            })?;
            let art: Value = serde_json::from_str(&raw).map_err(|e| format!("{}: not a JSON artifact: {e}", baseline.display()))?;
            out.push(format!(
                "cut     {} (schema {}, shape {}, verdict {})",
                baseline.display(),
                art["schema"].as_str().unwrap_or("?"),
                art["shape_hash"].as_str().unwrap_or("?"),
                art["verdict"].as_str().unwrap_or("?")
            ));
            Some((art, raw))
        }
        None => None,
    };
    // 4. the organization: a program of its own (GH #566 F2). The
    //    application is not touched — it carries its own law and is
    //    observable like any Hale binary; the org oversees it from outside.
    let purpose_text = format!(
        "{}: keep {} correct, reviewable and explainable; every change is staged, reviewed, and never applied by the organism itself.",
        project,
        if app.is_some() { "the application" } else { "what this repository builds" }
    );
    let org_dir = root.join(ORG_SEED);
    created(&mut out, &org_dir.join("purpose.hl"), &purpose_hl(&purpose_text))?;
    created(&mut out, &org_dir.join("charter.hl"), &charter_hl(&project))?;
    created(&mut out, &org_dir.join("law.hl"), &org_law_hl())?;
    // GH #583 M1: the catalog, from what this machine has
    let found = discover();
    if created(&mut out, &org_dir.join("models.hl"), &models_hl(&found))? {
        for line in found.report() {
            out.push(format!("models  {line}"));
        }
    }
    // GH #946: the performers a leg of this project runs, like the catalog
    created(&mut out, &org_dir.join("work.hl"), &work_hl(&found))?;
    // GH #995: the workflow catalog — the baseline, and the project's own
    created(&mut out, &org_dir.join("workflows.hl"), WORKFLOWS_HL)?;
    created(&mut out, &org_dir.join("own_workflows.hl"), OWN_WORKFLOWS_HL)?;
    // GH #1091 (B): the one structure policy — the operational roles the
    // project opts into, none by default; the holes follow the graph's
    // edges. A repository's record and an application's are both born with
    // the holes. One written before `init` is kept, and is what they read.
    created(&mut out, &org_dir.join("structure.hl"), &structure_hl(""))?;
    created(&mut out, &org_dir.join("main.hl"), &org_hl(&project, app.as_ref().map(|a| a.seed_rel.as_str())))?;
    // GH #583 K1: dev's environment is compose — the knowledge graph's
    // Postgres, a named volume per repository
    if created(&mut out, &root.join("dna/compose.yaml"), &compose_yaml(&project, &free_compose_ports(&project)))? {
        out.push("memory  dna/compose.yaml: `hale dna dev` brings its Postgres and NATS up, applies memory's schema and creates the nerves' stream (docker compose on PATH); `hale dna run` needs HALE_DNA_MEMORY_DSN_SPINE, HALE_DNA_NATS_URL_SPINE and HALE_DNA_NATS_ORG".to_string());
    }
    created(&mut out, &root.join("dna/senses.yml"), &senses_yml())?;
    let kept = match &app {
        Some(app) => format!("kept    {} (the application is not modified; the organization oversees it from {})", app.main_file.display(), ORG_SEED),
        None => format!("kept    {} (the repository is not modified; the organization oversees it from {})", root.display(), ORG_SEED),
    };
    out.push(kept);
    // 5. the manifest's environments: the application's, and the organization's
    let mtext = fs::read_to_string(&manifest).map_err(|e| e.to_string())?;
    if !mtext.contains("[environments.") {
        let add = match &app {
            Some(app) => format!(
                "\n# hale dna init: the two entrypoints and where each deploys (`hale check --matrix`).\n# The organization adopts its law (dna/org/law.hl) itself; the application keeps its own.\n[claims]\nno_base = true\n\n[environments.local]\nsource_only = true\nentrypoints = [\"{}\"]\n\n[environments.org]\nsource_only = true\nentrypoints = [\"{}\"]\n",
                app.seed_rel, ORG_SEED
            ),
            None => format!(
                "\n# hale dna init: the organization's entrypoint and where it deploys (`hale check --matrix`).\n# The organization adopts its law (dna/org/law.hl) itself; the repository's seeds keep their own.\n[claims]\nno_base = true\n\n[environments.org]\nsource_only = true\nentrypoints = [\"{}\"]\n",
                ORG_SEED
            ),
        };
        fs::write(&manifest, format!("{}{}", mtext, add)).map_err(|e| e.to_string())?;
        out.push(format!("edited  {} ([claims] no_base, {})", manifest.display(), if app.is_some() { "[environments.local], [environments.org]" } else { "[environments.org]" }));
    } else {
        out.push(format!("kept    {} (declares environments already)", manifest.display()));
    }
    // 7. the record, seeded from the artifact — or, for a repository, with
    //    the purpose's Review and what the repository holds, as the graph
    if record_exists(&root)? {
        out.push(format!("kept    {RECORD_REF} (a record exists; not reseeded)"));
    } else {
        match (&app, &cut) {
            (Some(app), Some((art, _))) => {
                let (n, graph) = seed_journal(&app.root, app, art, &purpose_text)?;
                out.push(format!("seeded  {RECORD_REF} ({n} observed event(s): application.attached, structure.observed, responsibility.proposed; then the purpose proposed and the application's graph)"));
                out.push(format!("graph   {} (graph.node, graph.edge)", graph.trim()));
                out.push(seat_initializer(&app.root, true)?);
            }
            _ => {
                // the declared purpose's proposal and the graph in one
                // checked seed: a repository the ingest refuses leaves no
                // record behind, and one with nothing to ingest still has one
                let graph = seed_repository(&root, &purpose_text)?;
                out.push(format!("seeded  {RECORD_REF} (the purpose, proposed; then the graph)"));
                out.push(format!("graph   {} (graph.node, graph.edge)", graph.trim()));
                out.push(seat_initializer(&root, true)?);
            }
        }
        // GH #995: the declared purpose was proposed in that seed, a
        // proposal the Board ratifies like every other
        out.push("seeded  purpose (proposed for the Board: `hale dna review` lists it under `purpose`)".to_string());
        // GH #596 C, #994: the design and the operating practices, as
        // proposals — one Review per practice, listed by family
        for (family, practices) in SEEDED {
            let (d, _, _) = design_upgrade(&root, practices)?;
            out.push(format!("seeded  {family} ({d} practice(s) proposed, one Board Review each: `hale dna review` lists them under `{family}`)"));
        }
        // seed/pull: the nodes knowledge is about, and the smallest library bound to them
        if library {
            out.extend(seed_library(&root, app.as_ref())?);
        } else {
            out.push("skipped library (--no-library: no language or system node, no library idea)".to_string());
        }
    }
    // 8. .gitignore hygiene
    let gi = root.join(".gitignore");
    let mut gtext = fs::read_to_string(&gi).unwrap_or_default();
    let mut added = Vec::new();
    for line in ["/vendor/", "/.hale/", SECRETS_IGNORES[0], SECRETS_IGNORES[1]] {
        if !gtext.lines().any(|l| l.trim() == line) {
            if !gtext.is_empty() && !gtext.ends_with('\n') {
                gtext.push('\n');
            }
            gtext.push_str(line);
            gtext.push('\n');
            added.push(line);
        }
    }
    if !added.is_empty() {
        fs::write(&gi, gtext).map_err(|e| e.to_string())?;
        out.push(format!("edited  {} ({})", gi.display(), added.join(", ")));
    }
    // 8b. GH #989: the organism's secrets, provisioned in one place (the
    // skin's list, dna/host/secrets.hl): drawn into the vault, the nerves'
    // dna/nats.conf (tracked, no secret) written, and the servers'
    // dna/nats.secrets.conf and dna/postgres.secrets (the only files with a
    // password: mode 600, ignored) from the same draw. After the record (the
    // vault names are its organization's) and after .gitignore (the writer
    // refuses a file git would track).
    out.push(organism_secrets(&root, true));
    // 9. format what we generated and touched
    let mut fmt = Command::new(&me);
    fmt.arg("fmt").arg(root.join("dna"));
    if let Some(app) = &app {
        fmt.arg(&app.seed);
    }
    let _ = fmt.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status();
    out.push(String::new());
    out.push("next steps:".to_string());
    out.push(format!("    hale check --matrix {}   # every entrypoint against its law", root.display()));
    out.push(format!("    hale dna dev {}          # the organization and the application under one host, iris attached", root.display()));
    out.push("    the first Review (`purpose`) ratifies the declared purpose (dna/org/purpose.hl): `hale dna review purpose approve --as <you>`".to_string());
    Ok(out)
}

fn upgrade(dir: &Path) -> Result<Vec<String>, String> {
    let start = dir.canonicalize().map_err(|e| format!("{}: {e}", dir.display()))?;
    let root = crate::shared::workspace::find_workspace_root_pub(&start).unwrap_or(start);
    if !root.join("vendor/dna").is_dir() && !root.join("dna").is_dir() {
        return Err(format!("{} has no DNA (no vendor/dna or dna/); run `hale dna init` first", root.display()));
    }
    let (w, same) = materialize_vendor(&root)?;
    let mut out = vec![format!(
        "vendor/dna: {} file(s) rewritten, {} unchanged; hale.lock pins toolchain {}, embedded dna {}. dna/ untouched.",
        w,
        same,
        TOOLCHAIN,
        hale_dna::embedded_short()
    )];
    // GH #583 M1: an organization from before the catalog gets one; its
    // main is the project's and is told, not edited
    let org_dir = root.join(ORG_SEED);
    let catalog = org_dir.join("models.hl");
    if org_dir.join("main.hl").is_file() && !catalog.is_file() {
        let found = discover();
        fs::write(&catalog, models_hl(&found)).map_err(|e| format!("write {}: {e}", catalog.display()))?;
        out.push(format!("created {}", catalog.display()));
        for line in found.report() {
            out.push(format!("models  {line}"));
        }
        let main = fs::read_to_string(org_dir.join("main.hl")).unwrap_or_default();
        // GH #946: agent work is a leg's; an organization from before the
        // legs still performs it in process
        if main.contains("dna::AgentPerformer") {
            out.push(format!(
                "note    {}/main.hl performs agent work in process (dna::AgentPerformer); a leg performs it now: `agent: dna::LegRelay {{ name: \"legs\" }}, agent_reconciler: dna::RelayReplay {{ }}` in the work system, and `hale dna work` claims it",
                ORG_SEED
            ));
        }
        if main.contains("dna::HostedModel") || main.contains("dna::ModelRouter {") {
            out.push(format!(
                "note    {}/main.hl wires its routers inline (dna::HostedModel is now dna::OpenAiChat); point each position at the catalog: `models: leader_models()`, `editor_models()`, `agent_models()`, and `budget: dna::Budget {{ policy: org_budget() }}` on the substrate",
                ORG_SEED
            ));
        }
    }
    // GH #946: an organization from before the legs gets its performers
    let performers = org_dir.join("work.hl");
    if org_dir.join("main.hl").is_file() && !performers.is_file() {
        fs::write(&performers, work_hl(&discover())).map_err(|e| format!("write {}: {e}", performers.display()))?;
        out.push(format!("created {}", performers.display()));
    }
    // GH #995: the workflow catalog's generated half is regenerated to the
    // current shape, whatever an earlier toolchain wrote; the project's
    // own definitions (own_workflows.hl) are the project's and are written
    // only where there are none. Its main is the project's and is told,
    // not edited.
    if org_dir.join("main.hl").is_file() {
        let workflows = org_dir.join("workflows.hl");
        let old = fs::read_to_string(&workflows).ok();
        if old.as_deref() != Some(WORKFLOWS_HL) {
            let was = workflows.is_file();
            fs::write(&workflows, WORKFLOWS_HL).map_err(|e| format!("write {}: {e}", workflows.display()))?;
            out.push(format!("{} {}", if was { "rewrote" } else { "created" }, workflows.display()));
            // what an earlier toolchain's workflows.hl defined of its own is
            // not carried over: it is named, for the project to move
            let dropped: Vec<&str> = old.as_deref().unwrap_or("").lines().map(str::trim).filter(|l| !l.starts_with("//") && (l.contains(".define(") || l.contains(".leaf(") || l.contains(".child("))).collect();
            if !dropped.is_empty() {
                out.push(format!("note    {}: it defined workflows of its own, which the rewrite dropped; move them into {} (`own_workflows`):\n        {}", workflows.display(), org_dir.join("own_workflows.hl").display(), dropped.join("\n        ")));
            }
        }
        // GH #1091 (B): the structure policy, regenerated to the current
        // shape; the roles the project opted into are its own and carried
        // over, and what else the file said is named, never silently lost
        let structure = org_dir.join("structure.hl");
        if let Ok(old) = fs::read_to_string(&structure) {
            let current = structure_hl(&structure_roles(&old));
            if old != current {
                fs::write(&structure, &current).map_err(|e| format!("write {}: {e}", structure.display()))?;
                out.push(format!("rewrote {}", structure.display()));
                let dropped: Vec<&str> = old.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("//") && !current.lines().any(|c| c.trim() == *l)).collect();
                if !dropped.is_empty() {
                    out.push(format!("note    {}: the rewrite kept only the roles operational_roles returns; it dropped:\n        {}", structure.display(), dropped.join("\n        ")));
                }
            }
        }
        let own = org_dir.join("own_workflows.hl");
        if !own.is_file() {
            fs::write(&own, OWN_WORKFLOWS_HL).map_err(|e| format!("write {}: {e}", own.display()))?;
            out.push(format!("created {}", own.display()));
        }
        let main = fs::read_to_string(org_dir.join("main.hl")).unwrap_or_default();
        if !main.contains("catalog: workflows()") {
            out.push(format!("note    {}/main.hl: point the substrate at the catalog, `catalog: workflows()` on dna::Dna, to admit the project's own definitions beside the baseline", ORG_SEED));
        }
    }
    // GH #946, #1104 piece 5: a record from before the seat: this uid
    // mapped to its user, so the head's socket knows the peer; the trust
    // an existing record declares, or does not, is its own
    if record_exists(&root)? {
        out.push(seat_initializer(&root, false)?);
    }
    // GH #596: an organization from before the charter gets one, and
    // the toolchain's design is proposed again where it changed — each
    // changed practice superseding the one it replaces, for the Board
    let charter = org_dir.join("charter.hl");
    if org_dir.join("main.hl").is_file() && !charter.is_file() {
        let project = locate(&root).map(|a| a.project).unwrap_or_else(|_| "the project".to_string());
        fs::write(&charter, charter_hl(&project)).map_err(|e| format!("write {}: {e}", charter.display()))?;
        out.push(format!("created {}", charter.display()));
    }
    // GH #1123: the owners map is retired — the graph is the one org chart.
    // The file goes; what it named is said, to be stated as `holds` edges.
    let owners = org_dir.join("owners");
    if owners.is_file() {
        let text = fs::read_to_string(&owners).unwrap_or_default();
        fs::remove_file(&owners).map_err(|e| format!("remove {}: {e}", owners.display()))?;
        out.push(format!("removed {}", owners.display()));
        let named: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).collect();
        if !named.is_empty() {
            out.push(format!(
                "note    {} named owners, which the graph states now (GH #1123): an `organization` node per owner, `holds(position:<p>, organization:<o>)` per position, `holds(organization:<o>, <person>)` per member — `hale dna fill`. It said:\n        {}",
                owners.display(),
                named.join("\n        ")
            ));
        }
    }
    // GH #1143: the optimize pass is a schedule; the generated field and
    // the loop's monotonic tick an older main.hl carries are rewritten to
    // what the generator writes now, and a hand-edited one is named
    let main_path = org_dir.join("main.hl");
    if let Ok(main_text) = fs::read_to_string(&main_path) {
        let mut next = main_text.clone();
        for (old, new) in SCHEDULE_MAIN_HL {
            next = next.replace(old, new);
        }
        if next != main_text {
            fs::write(&main_path, &next).map_err(|e| format!("write {}: {e}", main_path.display()))?;
            out.push(format!("rewrote {} (the optimize pass is a schedule: `optimize_every_ms` goes, and the loop ticks on the wall clock)", main_path.display()));
        }
        if next.contains("optimize_every_ms") || next.contains("request_tick(std::time::monotonic_ns()") {
            out.push(format!("note    {}: `optimize_every_ms` no longer builds and a monotonic tick names no occurrence; delete the field, tick with `std::time::nanos(std::time::current()) / 1000000`, and declare a cadence other than the seeded one with `hale dna schedule declare` (GH #1143)", main_path.display()));
        }
    }
    // The Board's field is `board`: the generated `membrane:` line an older
    // main.hl carries is rewritten, and one written by hand is named
    if let Ok(main_text) = fs::read_to_string(&main_path) {
        let next = main_text.replace(BOARD_MAIN_HL.0, BOARD_MAIN_HL.1);
        if next != main_text {
            fs::write(&main_path, &next).map_err(|e| format!("write {}: {e}", main_path.display()))?;
            out.push(format!("rewrote {} (the Board's field is `board`, no longer `membrane`)", main_path.display()));
        }
        if next.lines().any(|l| { let t = l.trim_start(); !t.starts_with("//") && t.starts_with("membrane:") }) {
            out.push(format!("note    {}: `membrane:` no longer builds; the Board's field on `dna::Dna` is `board:`", main_path.display()));
        }
    }
    // the generated main.hl's owners map line goes with it
    if let Ok(main_text) = fs::read_to_string(&main_path) {
        if main_text.contains(OWNERS_MAIN_HL) {
            fs::write(&main_path, main_text.replace(OWNERS_MAIN_HL, "")).map_err(|e| format!("write {}: {e}", main_path.display()))?;
            out.push(format!("rewrote {} (the owners map's `ownership:` field, retired)", main_path.display()));
        } else if main_text.contains("dna::Ownership {") && main_text.contains("dna/org/owners") {
            out.push(format!("note    {}: `dna::Ownership {{ path: \"dna/org/owners\" }}` no longer builds; delete the field — ownership is the graph's (GH #1123)", main_path.display()));
        }
    }
    // GH #986, #988, #989: dev's environment. dna/compose.yaml is a
    // generated seed file and is regenerated (a hard cut: its own compose
    // project, `hale-dna-<name>`, the nerves and the senses beside memory,
    // the servers' passwords from the vault's files); dna/senses.yml is
    // created when missing. Then the organism's secrets, provisioned, and
    // a regenerated seed rotates the nerves' passwords. The project's
    // .gitignore gains the secrets files first.
    if root.join("dna").is_dir() {
        let gi = root.join(".gitignore");
        let mut gtext = fs::read_to_string(&gi).unwrap_or_default();
        let mut added = Vec::new();
        for line in SECRETS_IGNORES {
            if !gtext.lines().any(|l| l.trim() == line) {
                if !gtext.is_empty() && !gtext.ends_with('\n') {
                    gtext.push('\n');
                }
                gtext.push_str(line);
                gtext.push('\n');
                added.push(line);
            }
        }
        if !added.is_empty() {
            fs::write(&gi, gtext).map_err(|e| e.to_string())?;
            out.push(format!("edited  {} ({})", gi.display(), added.join(", ")));
        }
        // the seed's name is the one its compose file carries, so a clone in
        // a directory of another name regenerates the same file
        let compose = root.join("dna/compose.yaml");
        let had = fs::read_to_string(&compose).unwrap_or_default();
        let project = compose_seed_of(&had).unwrap_or_else(|| locate(&root).map(|a| a.project).unwrap_or_else(|_| "project".to_string()));
        // and its ports the ones it publishes: another seed's are not taken
        let ports = compose_ports_of(&had).unwrap_or_else(|| free_compose_ports(&project));
        let want = compose_yaml(&project, &ports);
        if had != want {
            fs::write(&compose, &want).map_err(|e| format!("write {}: {e}", compose.display()))?;
            out.push(format!("{} {} (its own compose project, hale-dna-{}; the servers' passwords from the vault)", if had.is_empty() { "created" } else { "rewrote" }, compose.display(), compose_name(&project)));
            let stale = compose_containers_elsewhere(&compose, &format!("hale-dna-{}", compose_name(&project)));
            if !stale.is_empty() {
                out.push(format!("note    the services the previous dna/compose.yaml started are still up under another compose project, on the same ports: stop them (`docker stop {}`) before `hale dna dev`", stale.join(" ")));
            }
        }
        let senses = root.join("dna/senses.yml");
        if !senses.is_file() {
            fs::write(&senses, senses_yml()).map_err(|e| format!("write {}: {e}", senses.display()))?;
            out.push(format!("created {}", senses.display()));
        }
        out.push(organism_secrets(&root, true));
    }
    let main_text = fs::read_to_string(org_dir.join("main.hl")).unwrap_or_default();
    if main_text.contains("main locus Org") && main_text.contains("unix(") {
        out.push(format!(
            "note    {}/main.hl binds its facts to unix sockets, which DNA no longer serves (GH #986): they arrive over the nerves. Replace its `bindings` with the ones `hale dna init` writes today: `import \"vendor/dna/pond/realtime/nats\" as nats;`, the `nerves: nats::NatsConn` param, `placement {{ nerves: pinned; }}`, and each fact bound to `nats::NatsAdapter {{ }}`",
            ORG_SEED
        ));
    }
    if main_text.contains("main locus Org") && main_text.contains("nats::NatsConn {") && !main_text.contains("credential:") {
        out.push(format!(
            "note    {}/main.hl's `nerves` connection presents no credential (GH #989): no URL carries a password now, so it cannot connect. Add `user: \"spine\", credential: std::secret::Credential {{ vault: dna::nerves_role_vault(\"spine\") }},` to its `nats::NatsConn`, as `hale dna init` writes it; an application's own connection takes `user: \"app\"` and the vault entry HALE_DNA_NATS_VAULT_APP names",
            ORG_SEED
        ));
    }
    if main_text.contains("main locus Org") && main_text.contains("bindings {") && !main_text.contains("dna::WorkAllowanceAsk") {
        out.push(format!(
            "note    {}/main.hl binds no `dna::WorkAllowanceAsk` (GH #1131): a leg asks the spine for its attempt's spend before its first model call, and the ask arrives over the nerves; unbound, a leg's attempt is declined with no answer. Add `dna::WorkAllowanceAsk: nats::NatsAdapter {{ }};` to its `bindings`, as `hale dna init` writes today",
            ORG_SEED
        ));
    }
    if main_text.contains("main locus Org") && main_text.contains("bindings {") && !main_text.contains("dna::WorkSubmit") {
        out.push(format!(
            "note    {}/main.hl binds no `dna::WorkSubmit` (GH #946): a leg's outcome, handed back at the head, arrives over the nerves. Add `dna::WorkSubmit: nats::NatsAdapter {{ }};` to its `bindings`, as `hale dna init` writes today",
            ORG_SEED
        ));
    }
    if main_text.contains("self.core.tick(") {
        out.push(format!(
            "note    {}/main.hl calls self.core.tick directly; use self.core.request_tick with the same millisecond clock in the live loop so journal refresh and incoming work run on the owner's queue",
            ORG_SEED
        ));
    }
    if main_text.contains("journal: dna::GitJournal {") {
        out.push(format!(
            "note    {}/main.hl wires the record alone as its journal; the day's work can move to the ledger (GH #646) once it reads both: `journal: dna::RoutedJournal {{ record: dna::GitJournal {{ repo: \".\" }} }}` (the ledger is memory's, GH #985)",
            ORG_SEED
        ));
    }
    if main_text.contains("dna::Leader {") && !main_text.contains("charter: charter()") {
        out.push(format!(
            "note    {}/main.hl builds its Leader without a brief; give it `charter: charter(), purpose: purpose()` so it reads the charter, the purpose, the law and the ratified design before it decides",
            ORG_SEED
        ));
    }
    // GH #985: the knowledge service and its clients are gone; memory is
    // the organism's own handles, which main.hl and law.hl need not name
    if main_text.contains("dna::ServiceLedger") || main_text.contains("dna::KnowledgeClient") {
        out.push(format!(
            "note    {}/main.hl names dna::ServiceLedger or dna::KnowledgeClient, which no longer exist: delete the `ledger:`, `knowledge_client:` and Leader `knowledge:` lines — the organism opens memory itself (HALE_DNA_MEMORY_DSN_SPINE)",
            ORG_SEED
        ));
    }
    let law_text = fs::read_to_string(org_dir.join("law.hl")).unwrap_or_default();
    if law_text.contains("dna::KnowledgeClient") {
        out.push(format!("note    {}/law.hl groups dna::KnowledgeClient; it is dna::MemoryKnowledge now", ORG_SEED));
    }
    if record_exists(&root)? {
        for (family, practices) in SEEDED {
            let (proposed, superseded, waiting) = design_upgrade(&root, practices)?;
            if proposed > 0 {
                out.push(format!("{family}  {proposed} practice(s) proposed ({superseded} superseding an earlier version); the Board decides each: `hale dna review`"));
            }
            if waiting > 0 {
                out.push(format!("{family}  {waiting} practice(s) changed but wait: an earlier replacement is still before the Board (decide it, then `upgrade` again)"));
            }
        }
        // the library of this toolchain's version, for a record that has one:
        // a new family per node, each idea superseding the one it replaces
        // once the Board approves; an application pinned to the old toolchain
        // keeps its bindings until then
        let (proposed, families, superseding, waiting) = library_propose(&root, true)?;
        if families > 0 {
            out.push(format!("library  {families} famil(ies) proposed at {} ({proposed} idea(s), {superseding} superseding an earlier version); the Board decides each: `hale dna review library approve`", library_version()));
        }
        if waiting > 0 {
            out.push(format!("library  {waiting} famil(ies) changed but wait: an earlier version is still before the Board (decide it, then `upgrade` again)"));
        }
    }
    // GH #985: with the owner's DSN given, memory's schema moves to this
    // toolchain's version here; without one, `dev` applies it at start
    let owner = std::env::var(OWNER_DSN_ENV).unwrap_or_default();
    if owner.starts_with("postgres") && record_exists(&root)? {
        if let MemoryPlan::Roles { .. } = memory_migrate(&root)? {
            out.push("memory  schema applied with the owner's DSN; `hale dna memory migrate` prints the spine's".to_string());
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------
// the knowledge graph's environment (GH #583 K1)
// ---------------------------------------------------------------

/// `dna/compose.yaml`: memory's Postgres (with pgvector)
/// for `hale dna dev`, a named volume per repository so the graph
/// outlives the container. Part of the genome, project-owned.
fn compose_yaml(project: &str, ports: &ComposePorts) -> String {
    let name = compose_name(project);
    format!(
        r#"# dna/compose.yaml — the organism's environment for `hale dna dev`
# (project-owned; generated by `hale dna init`). Memory: the organism
# keeps its working memory in this Postgres — the ledger, the graph,
# protected evidence — and the record (refs/dna/*) holds the decided
# half. Nerves (GH #986): facts travel between the organism's parts over
# this NATS JetStream server, one stream per organization, each part
# with its own user (dna/nats.conf). `hale dna dev` runs `docker compose
# -f dna/compose.yaml up -d`, applies memory's schema and creates the
# stream as their owners, and hands the organism its own roles. Beyond
# one machine, point HALE_DNA_MEMORY_DSN_OWNER at a Postgres of your own
# and HALE_DNA_NATS_URL_OWNER at a NATS server configured like
# dna/nats.conf. Senses (GH #988): the store keeps every part's readings
# (dna/senses.yml says which), for the reflexes to read; `hale dna dev`
# brings it up with the rest. All three listen on 127.0.0.1 only.
# Secrets (GH #989): the database's superuser password and the nerves'
# are the vault's, and reach the servers through dna/postgres.secrets
# and dna/nats.secrets.conf, which `hale dna init` and `upgrade` write:
# never tracked, mode 600. The project is this seed's alone, so two
# seeds on one machine never share a container.
name: hale-dna-{name}
services:
  knowledge-db:
    image: pgvector/pgvector:pg16
    environment:
      POSTGRES_USER: dna
      POSTGRES_PASSWORD_FILE: /run/secrets/postgres-owner
      POSTGRES_DB: dna
    ports:
      - "127.0.0.1:{port}:5432"
    volumes:
      - ./postgres.secrets:/run/secrets/postgres-owner:ro
      - knowledge-db:/var/lib/postgresql/data
  nerves:
    image: nats:2
    command: ["-c", "/etc/nats/nats.conf"]
    ports:
      - "127.0.0.1:{nats_port}:4222"
    volumes:
      - ./nats.conf:/etc/nats/nats.conf:ro
      - ./nats.secrets.conf:/etc/nats/nats.secrets.conf:ro
      - nerves:/data
  senses:
    image: {image}
    command: ["--config.file=/etc/prometheus/senses.yml", "--storage.tsdb.path=/prometheus", "--storage.tsdb.retention.time={retention}"]
    extra_hosts:
      - "host.docker.internal:host-gateway"
    ports:
      - "127.0.0.1:{senses_port}:9090"
    volumes:
      - ./senses.yml:/etc/prometheus/senses.yml:ro
      - senses:/prometheus
volumes:
  knowledge-db:
    name: hale-dna-{name}-knowledge
  nerves:
    name: hale-dna-{name}-nerves
  senses:
    name: hale-dna-{name}-senses
"#,
        port = ports.db,
        nats_port = ports.nats,
        senses_port = ports.senses,
        image = PROMETHEUS_IMAGE,
        retention = SENSES_RETENTION,
    )
}

/// The senses' store (GH #988): Prometheus, kept this long.
const PROMETHEUS_IMAGE: &str = "prom/prometheus:v3.5.0";
const SENSES_RETENTION: &str = "7d";

/// `dna/senses.yml`: what the senses' store scrapes (GH #988) — every
/// long-running part's readings, on the ports `dna/core/senses.hl` names
/// (`SENSES_PORT_SPINE`, `_NODE`, `_HEAD`), reached from the store's
/// container over the host gateway. `honor_labels`: a part's own labels
/// (a node's `instance`) are the readings', never the target's.
fn senses_yml() -> String {
    r#"# dna/senses.yml — what the senses' store scrapes (GH #988; project-owned,
# generated by `hale dna init`). Every long-running part of the organism
# serves its readings (std::metrics) on its own port, each series labelled
# `part`: the spine (the node's host) on 9464, a node on 9465, the head on
# 9466 — or on HALE_DNA_SENSES_PORT when its environment names one. The
# store in dna/compose.yaml reaches them over the host gateway and keeps
# what they said for its retention (--storage.tsdb.retention.time); the
# reflexes read it. A part never learns where its readings go.
global:
  scrape_interval: 5s
scrape_configs:
  - job_name: spine
    honor_labels: true
    static_configs:
      - targets: ["host.docker.internal:9464"]
  - job_name: node
    honor_labels: true
    static_configs:
      - targets: ["host.docker.internal:9465"]
  - job_name: head
    honor_labels: true
    static_configs:
      - targets: ["host.docker.internal:9466"]
"#
    .to_string()
}

/// The host ports a seed's compose publishes its services on: memory's
/// Postgres in 54xx, the nerves' NATS in 42xx, the senses' store in 93xx.
#[derive(Debug, Clone, Copy, PartialEq)]
struct ComposePorts {
    db: u16,
    nats: u16,
    senses: u16,
}

/// The ports a seed's compose file already publishes, when it names all
/// three: an upgrade, and a clone's, keep them, so a regenerated file is
/// the file the seed already has.
fn compose_ports_of(text: &str) -> Option<ComposePorts> {
    let published = |inner: &str| {
        text.lines().find_map(|l| {
            let l = l.trim().trim_start_matches("- ").trim_matches('"');
            let rest = l.strip_prefix("127.0.0.1:")?;
            let (host, container) = rest.split_once(':')?;
            (container == inner).then(|| host.parse::<u16>().ok()).flatten()
        })
    };
    Some(ComposePorts { db: published("5432")?, nats: published("4222")?, senses: published("9090")? })
}

/// Free host ports for a new seed's compose, taken at seed time as
/// `dna::free_port` takes a fixture's: each service's candidate comes from
/// the seed's name, so a seed's ports are its own and stable, and one that
/// something on this machine already listens on is stepped past. Hashed
/// alone, two seeds, or a seed and a server the machine already runs (a CI
/// runner's NATS on 4222), could be handed one port, and the second
/// compose up fails.
fn free_compose_ports(project: &str) -> ComposePorts {
    let h = project.bytes().fold(5381u32, |h, b| h.wrapping_mul(33) ^ b as u32);
    let pick = |base: u16| {
        let first = (h % 100) as u16;
        (0..100u16)
            .map(|i| base + (first + i) % 100)
            .find(|p| std::net::TcpListener::bind(("127.0.0.1", *p)).is_ok())
            .unwrap_or(base + first)
    };
    ComposePorts { db: pick(5400), nats: pick(4200), senses: pick(9300) }
}

/// The seed's name a compose file carries: its project (`name:
/// hale-dna-<seed>`) or, in a file from before each seed had one, its
/// memory volume's (`name: hale-dna-<seed>-knowledge`).
fn compose_seed_of(text: &str) -> Option<String> {
    let names: Vec<&str> = text.lines().filter_map(|l| l.trim().strip_prefix("name: hale-dna-")).collect();
    if let Some(top) = text.lines().find_map(|l| l.strip_prefix("name: hale-dna-")) {
        return Some(top.trim().to_string());
    }
    names.iter().find_map(|n| n.trim().strip_suffix("-knowledge")).map(|n| n.to_string())
}

/// The project's name as compose's volumes carry it.
fn compose_name(project: &str) -> String {
    project.replace(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_', "-").to_lowercase()
}

/// The running containers that `compose` (its absolute path) started under
/// a compose project other than `project`: what an older file, before
/// each seed had its own project, left up.
fn compose_containers_elsewhere(compose: &Path, project: &str) -> Vec<String> {
    let file = compose.canonicalize().unwrap_or_else(|_| compose.to_path_buf());
    let out = Command::new("docker")
        .args(["ps", "--filter", &format!("label=com.docker.compose.project.config_files={}", file.display()), "--format", "{{.Names}}\t{{.Label \"com.docker.compose.project\"}}"])
        .output();
    let Ok(out) = out else { return Vec::new() };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .filter(|(_, p)| *p != project)
        .map(|(n, _)| n.to_string())
        .collect()
}

/// The lines the project's .gitignore carries for the servers' secrets
/// (GH #989): the only files with a password are never tracked.
const SECRETS_IGNORES: [&str; 2] = ["/dna/nats.secrets.conf", "/dna/postgres.secrets"];

/// GH #989: the organism's secrets, provisioned by its bootstrap through
/// the host (`secrets-provision`, dna/host/secrets.hl, the skin's one
/// list): what it did, as lines of init's or upgrade's report. `rotate`:
/// the nerves' passwords are drawn anew, as a regenerated seed does.
fn organism_secrets(root: &Path, rotate: bool) -> String {
    let args: Vec<String> = if rotate { vec!["--rotate".to_string()] } else { vec![] };
    match host_run("secrets-provision", root, &args) {
        Ok(out) if !out.starts_with("none: ") => out.trim().to_string(),
        Ok(out) => format!("note    the organism's secrets were not provisioned: {} (`hale dna secrets` lists them)", out.trim().strip_prefix("none: ").unwrap_or(out.trim())),
        Err(e) => format!("note    the organism's secrets were not provisioned: {e}"),
    }
}

// ---------------------------------------------------------------
// the catalog (GH #583 M1)
// ---------------------------------------------------------------

/// What this machine has for models: keys in its vault, servers and
/// harnesses on `PATH`. Nothing found is fine — the catalog is still
/// written, every hosted backend simply is not permitted until its key is
/// in the vault, and every Review waits for the Board.
struct Discovery {
    openai: bool,
    anthropic: bool,
    ollama: Option<String>, // the first model `ollama list` names, when ollama is on PATH
    harnesses: Vec<String>, // `claude`, `codex` on PATH (backends in a later toolchain)
}

fn on_path(bin: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(bin).is_file()))
        .unwrap_or(false)
}

/// The local vault's directory, as `std::secret::vault_local_dir()` reads
/// it: `HALE_VAULT_DIR`, else `$XDG_CACHE_HOME/hale/vault`, else
/// `~/.cache/hale/vault`. None with a real vault (`HALE_VAULT_ADDR`), whose
/// entries are read only where they are presented.
fn vault_local_dir() -> Option<PathBuf> {
    let set = |v: &str| std::env::var(v).ok().filter(|s| !s.is_empty());
    if set("HALE_VAULT_ADDR").is_some() {
        return None;
    }
    if let Some(d) = set("HALE_VAULT_DIR") {
        return Some(PathBuf::from(d));
    }
    if let Some(c) = set("XDG_CACHE_HOME") {
        return Some(PathBuf::from(c).join("hale/vault"));
    }
    set("HOME").map(|h| PathBuf::from(h).join(".cache/hale/vault"))
}

fn discover() -> Discovery {
    // `HALE_DNA_DISCOVER=off`: find nothing. For fixtures, so that a
    // developer's machine — a key in the shell, `claude` on PATH — makes
    // the same organization CI makes, one whose leader has no model that
    // answers and whose plans therefore take the defaults. Otherwise a
    // fixture spends real calls and asserts on a live model's judgment.
    if std::env::var("HALE_DNA_DISCOVER").map(|v| v == "off").unwrap_or(false) {
        return Discovery { openai: false, anthropic: false, ollama: None, harnesses: Vec::new() };
    }
    // a model key is the vault's slot for it (GH #989: `model-<NAME>`, the
    // one source a DNA credential has), never an environment variable
    let key = |v: &str| {
        vault_local_dir()
            .map(|d| std::fs::read_to_string(d.join(format!("model-{v}"))).map(|s| !s.trim().is_empty()).unwrap_or(false))
            .unwrap_or(false)
    };
    let ollama = if on_path("ollama") {
        let first = Command::new("ollama")
            .arg("list")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .skip(1)
                    .filter_map(|l| l.split_whitespace().next().map(str::to_string))
                    .next()
            });
        Some(first.unwrap_or_else(|| "llama3".to_string()))
    } else {
        None
    };
    Discovery {
        openai: key("OPENAI_API_KEY"),
        anthropic: key("ANTHROPIC_API_KEY"),
        ollama,
        harnesses: ["claude", "codex"].iter().filter(|b| on_path(b)).map(|b| b.to_string()).collect(),
    }
}

/// A hosted backend as the catalog writes it: the adapter (what the
/// wire speaks), model, key and its scheme, price per 1k tokens in
/// micro-dollars.
struct Hosted {
    adapter: &'static str, // `dna::OpenAiChat` | `dna::AnthropicMessages`
    model: &'static str,
    endpoint: &'static str,
    key: &'static str, // the model key's name: its vault slot is `model-<key>`
    scheme: &'static str, // `bearer` | `x-api-key`
    input_micros_per_1k: u32,
    output_micros_per_1k: u32,
}

impl Discovery {
    /// The provider the hosted backends speak to: Anthropic's native
    /// Messages API when its key is present (the strongest models),
    /// else OpenAI's chat shape (its key, or a placeholder until one is
    /// set).
    fn hosted(&self) -> (Hosted, Hosted, &'static str) {
        if self.anthropic {
            (
                Hosted { adapter: "dna::AnthropicMessages", model: "claude-opus-5", endpoint: "https://api.anthropic.com/v1/messages", key: "ANTHROPIC_API_KEY", scheme: "x-api-key", input_micros_per_1k: 15000, output_micros_per_1k: 75000 },
                Hosted { adapter: "dna::AnthropicMessages", model: "claude-haiku-4-5-20251001", endpoint: "https://api.anthropic.com/v1/messages", key: "ANTHROPIC_API_KEY", scheme: "x-api-key", input_micros_per_1k: 1000, output_micros_per_1k: 5000 },
                "ANTHROPIC_API_KEY",
            )
        } else {
            (
                Hosted { adapter: "dna::OpenAiChat", model: "gpt-4o", endpoint: "https://api.openai.com/v1/chat/completions", key: "OPENAI_API_KEY", scheme: "bearer", input_micros_per_1k: 2500, output_micros_per_1k: 10000 },
                Hosted { adapter: "dna::OpenAiChat", model: "gpt-4o-mini", endpoint: "https://api.openai.com/v1/chat/completions", key: "OPENAI_API_KEY", scheme: "bearer", input_micros_per_1k: 150, output_micros_per_1k: 600 },
                "OPENAI_API_KEY",
            )
        }
    }
    /// The harness the catalog writes, when one is on PATH: `claude`
    /// first, else `codex`.
    fn harness(&self) -> Option<&'static str> {
        if self.harnesses.iter().any(|h| h == "claude") {
            Some("claude")
        } else if self.harnesses.iter().any(|h| h == "codex") {
            Some("codex")
        } else {
            None
        }
    }
    fn summary(&self) -> String {
        let mut parts = Vec::new();
        parts.push(match (self.anthropic, self.openai) {
            (true, true) => "ANTHROPIC_API_KEY and OPENAI_API_KEY in the vault (Anthropic chosen)".to_string(),
            (true, false) => "ANTHROPIC_API_KEY in the vault".to_string(),
            (false, true) => "OPENAI_API_KEY in the vault".to_string(),
            (false, false) => "no model key in the vault (`hale dna secret set <NAME>`)".to_string(),
        });
        parts.push(match &self.ollama {
            Some(m) => format!("ollama on PATH ({m})"),
            None => "no ollama on PATH".to_string(),
        });
        if !self.harnesses.is_empty() {
            parts.push(format!("{} on PATH", self.harnesses.join(" and ")));
        }
        parts.join(", ")
    }
    /// What `init` prints: what each position was given.
    fn report(&self) -> Vec<String> {
        let (frontier, fast, key) = self.hosted();
        let desk = self.ollama.clone().unwrap_or_else(|| "llama3".to_string());
        let out = vec![
            format!("found   {}", self.summary()),
            format!(
                "frontier = {} · fast = {} ({}{}) · desk = {} (ollama at 127.0.0.1:11434{})",
                frontier.model,
                fast.model,
                key,
                if self.anthropic || self.openai { "" } else { ", not in the vault: hosted backends are not permitted until it is" },
                desk,
                if self.ollama.is_some() { "" } else { ", not found" }
            ),
            match (self.harness(), self.anthropic || self.openai) {
                (Some(h), true) => format!("editor, agent: quick = harness ({h}), deep = frontier · leader: deep = frontier, quick = fast · private = desk · budget 25.00 USD a day (`hale dna models` probes them)"),
                (Some(h), false) => format!("editor, agent, leader: quick = harness ({h}), deep = harness ({h}) · private = desk · budget 25.00 USD a day (`hale dna models` probes them)"),
                (None, _) => "leader, editor, agent: deep = frontier, quick = fast, private = desk · budget 25.00 USD a day (`hale dna models` probes them)".to_string(),
            },
        ];
        out
    }
}

/// `dna/org/work.hl`: the performers a leg of this project runs (GH
/// #946 slice 4), generated at init like the catalog. A person's leg
/// needs none of the three replaced; a program that performs a work
/// kind deterministically is added here and wins for that kind; the
/// model performer is the catalog's router behind the model leg when
/// init found a backend, and `NoModel` — the work is a person's — when
/// it found none, so a fresh project with no key burns no attempts.
fn work_hl(found: &Discovery) -> String {
    let configured = found.openai || found.anthropic || found.ollama.is_some();
    let harness = found.harness().is_some();
    let model = if configured || harness {
        format!(
            "// The model leg: the catalog's agent router behind the performer
// interface, out of process, per task. A rate-limited call is backed
// off inside the attempt and every wait is recorded as evidence. Its
// effect class: `effect_free` for a backend that answers (a lost reply
// may be retried), `uncertain` for one that works in place with its
// own tools (a call that may have acted is never retried by a program;
// the attempt waits, unresolved, for a person). The catalog's router
// {reason}, so the class is `{class}`.
fn model() -> legs::ModelPerformer {{
    return legs::ModelPerformer {{ router: agent_models(), effect: \"{class}\" }};
}}",
            reason = if harness { "puts a harness with tools on a tier" } else { "answers over hosted backends" },
            class = if harness { "uncertain" } else { "effect_free" }
        )
    } else {
        String::from(
            "// No backend was configured when this file was generated, so the
// model takes nothing and agent work is a person's. With a backend in
// dna/org/models.hl, put the catalog behind the leg (`effect_free` for
// hosted backends that answer, `uncertain` for a harness with tools):
//
// fn model() -> legs::ModelPerformer { return legs::ModelPerformer { router: agent_models(), effect: \"effect_free\" }; }
fn model() -> legs::NoModel {
    return legs::NoModel { };
}",
        )
    };
    let text = r#"// dna/org/work.hl — the performers (project-owned; generated by
// `hale dna init`).
//
// `hale dna work` builds this beside the toolchain's legs seed and runs
// it as the project's leg. A leg is handed a brief — the hat as the
// head rendered it, and the lease the Work is held under — and the
// hands it may use, and answers with a performance. Three kinds of
// performer: a person (the brief is rendered as text and the outcome is
// theirs to submit), a deterministic one (a program of yours: it wins
// for the work kinds it takes), and a model (the catalog's router, run
// as a leg). Every performer declares an effect class — `effect_free`,
// `idempotent` or `uncertain`: what a settle that failed may have left
// behind, and whether the loop may run it again (an uncertain one is
// never retried by a program). `hale check` validates all of it; a
// performer that declares no class is refused when the leg starts.

import "vendor/dna/legs" as legs;

// A deterministic performer, when this project has one: give it the
// work kinds it takes (`agent`, `service`, `software`, …) and it wins
// for them, declaring its effect class (`legs::FixedAnswer { kinds:
// "agent", effect: "effect_free", result: "…" }` is the smallest);
// `legs::NoDeterministic { }` takes nothing. A person's leg is `hale dna
// work run` (`--performer person` when the model takes the kind); a
// worker is `hale dna work loop --parallel N`.
fn deterministic() -> legs::NoDeterministic {
    return legs::NoDeterministic { };
}

@@MODEL@@

fn performers() -> legs::PerformerCatalog {
    // a person answers what they are asked; asked again, they answer again
    return legs::PerformerCatalog { person: legs::Person { effect: "idempotent" }, deterministic: deterministic(), model: model() };
}

// The hands a performer may use: git in scratch, the forge through
// `gh`, this toolchain; deploy and the heart's API refuse until GH #987
// hands them over.
fn hands() -> legs::Hands {
    return legs::Hands { };
}
"#;
    text.replace("@@MODEL@@", &model)
}

/// `dna/org/models.hl`: the catalog as source (GH #583 M1). Backend
/// constructor functions by name, a router function per position
/// composed from them, one budget policy, and the probe `hale dna
/// models` runs.
fn models_hl(found: &Discovery) -> String {
    let (frontier, fast, _) = found.hosted();
    let desk_model = found.ollama.clone().unwrap_or_else(|| "llama3".to_string());
    let keyed = found.anthropic || found.openai;
    let has_harness = found.harness().is_some();
    // GH #583 M3: an installed harness edits in an export of the worktree
    // with its own tools; the editor imports the diff under the grant.
    let harness_section = match found.harness() {
        Some("codex") => r#"// ---- the harness ---------------------------------------------------
//
// `codex` on this machine: run per request with its own tools, in an
// EXPORT of the worktree (a plain directory, never `.git`) as its cwd;
// the editor imports what it changed under the grant. The confinement
// masks the repository from the process (Bubblewrap on Linux); with
// none available the harness is refused unless `allow_unconfined` says
// otherwise — that is the org chart's word, and the evidence records
// which it was. `output: "text"`: codex prints its final message.
//
// The export is deliberately not a repository, hence
// `--skip-git-repo-check`; the prompt arrives on stdin, so no flag
// carries it. `--sandbox workspace-write` because a non-interactive
// codex is READ-ONLY by default: without it this backend can answer
// but cannot edit the export it was given, and the attempt comes back
// with nothing changed under the grant. It grants writes in the
// working directory only, and DNA's own confinement still holds the
// genome out of reach. Versions differ: check `codex exec --help` if
// yours refuses these.

fn harness() -> dna::HarnessModel {
    return dna::HarnessModel { name: "quick", command: "codex", argv: "exec
--skip-git-repo-check
--sandbox
workspace-write", output: "text", confinement: dna::Bubblewrap { } };
}

"#
        .to_string(),
        Some(_) => r#"// ---- the harness ---------------------------------------------------
//
// `claude` on this machine: run per request with its own tools, in an
// EXPORT of the worktree (a plain directory, never `.git`) as its cwd;
// the editor imports what it changed under the grant. The confinement
// masks the repository from the process (Bubblewrap on Linux); with
// none available the harness is refused unless `allow_unconfined` says
// otherwise — that is the org chart's word, and the evidence records
// which it was.

fn harness() -> dna::HarnessModel {
    return dna::HarnessModel { name: "quick", command: "claude", confinement: dna::Bubblewrap { } };
}

"#
        .to_string(),
        None => String::new(),
    };
    let hosted = |name: &str, h: &Hosted| {
        format!(
            "{} {{ name: \"{name}\", model: \"{}\", endpoint: \"{}\", credential: dna::HostedCredential {{ key: \"{}\", scheme: \"{}\" }}, input_micros_per_1k: {}, output_micros_per_1k: {} }}",
            h.adapter, h.model, h.endpoint, h.key, h.scheme, h.input_micros_per_1k, h.output_micros_per_1k
        )
    };
    format!(
        r#"// dna/org/models.hl — the model catalog (project-owned; generated by
// `hale dna init` from what this machine had: {summary}).
//
// The catalog is source. A backend is a constructor function; a
// position's router is a function composed from them; the budget is
// one policy the substrate owns. Add a backend by adding a function,
// switch a provider by editing one, give a new position its own
// router by adding one more: `hale check` validates all of it and the
// law keeps the concrete types at every boundary. `hale dna models`
// lists the catalog and sends one small request to each backend.

import "vendor/dna" as dna;

// ---- backends ------------------------------------------------------
//
// A hosted backend is `dna::AnthropicMessages` (the native Messages
// API; the key travels as `x-api-key`) or `dna::OpenAiChat` (the
// OpenAI chat shape: OpenAI, OpenRouter, vLLM, Ollama; the key as a
// bearer token). Either presents its key from a sealed locus that
// never returns it; without the key it is not a permitted backend and
// the router refuses before the wire. Prices are micro-dollars per
// 1k tokens, for the evidence and the budget.

// The strongest model: the Leader's decisions, the editor's assessments.
fn frontier() -> {adapter} {{
    return {frontier};
}}

// A fast, cheap model: classification, drafts, retries.
fn fast() -> {adapter} {{
    return {fast};
}}

// A model on this machine: customer-classed data never leaves it. No
// key, no `external_model` effect.
fn desk() -> dna::LocalModel {{
    return dna::LocalModel {{ name: "private", endpoint: "http://127.0.0.1:11434/v1/chat/completions", model: "{desk}" }};
}}

{harness_section}// ---- positions -----------------------------------------------------
//
// Each position's router: `deep` decides and assesses, `quick` drafts
// and classifies, `private` takes what may not leave the machine.
// They may differ — the judgement of what gets applied on the
// strongest model, the production of candidates on a cheaper one.

fn leader_models() -> dna::ModelRouter {{
    return dna::ModelRouter {{ quick: {leader_quick}, deep: {leader_deep}, private: desk() }};
}}

fn editor_models() -> dna::ModelRouter {{
    return dna::ModelRouter {{ quick: {editor_quick}, deep: {editor_deep}, private: desk() }};
}}

fn agent_models() -> dna::ModelRouter {{
    return dna::ModelRouter {{ quick: {editor_quick}, deep: {editor_deep}, private: desk() }};
}}

// ---- the budget ----------------------------------------------------
//
// One allowance for the whole organization per window ("day", "week"
// or "none"), in micro-dollars. The substrate owns the counter: every
// model call is accounted from its evidence, the counter is rehydrated
// from the record on a restart, and on an exhausted window intent is
// refused and Reviews wait for the Board.

fn org_budget() -> dna::BudgetPolicy {{
    return dna::BudgetPolicy {{ window: "day", allowance_micros: 25000000 }};
}}

// ---- the probe (`hale dna models`) ---------------------------------

fn probe_catalog() -> String {{
    return dna::probe("frontier", frontier()) + dna::probe("fast", fast()) + dna::probe("desk", desk()){probe_harness};
}}
"#,
        summary = found.summary(),
        adapter = frontier.adapter,
        frontier = hosted("deep", &frontier),
        fast = hosted("quick", &fast),
        desk = desk_model,
        harness_section = harness_section,
        leader_quick = if keyed || !has_harness { "fast()" } else { "harness()" },
        leader_deep = if keyed || !has_harness { "frontier()" } else { "harness()" },
        editor_quick = if has_harness { "harness()" } else { "fast()" },
        editor_deep = if keyed || !has_harness { "frontier()" } else { "harness()" },
        probe_harness = if has_harness { " + dna::probe(\"harness\", harness())" } else { "" },
    )
}

// ---------------------------------------------------------------
// run — the stateless host
// ---------------------------------------------------------------

/// The project root and its DNA entrypoint (from `[environments.local]`).
fn project(dir: &Path) -> Result<(PathBuf, PathBuf), String> {
    let start = dir.canonicalize().map_err(|e| format!("{}: {e}", dir.display()))?;
    let root = crate::shared::workspace::find_workspace_root_pub(&start).unwrap_or(start);
    let manifest = root.join("hale.toml");
    let (envs, _) = crate::pkg::read_claims_config(&manifest)?;
    let seed = envs
        .get("local")
        .and_then(|e| e.entrypoints.first().cloned())
        .map(|e| root.join(e))
        .unwrap_or_else(|| root.clone());
    if !root.join("dna").is_dir() {
        return Err(format!("{} has no DNA (no dna/ seed); run `hale dna init` first", root.display()));
    }
    Ok((root, seed))
}

/// Append one event to the Journal from the host — only while the
/// organism is NOT running (the Journal has one writer at a time; the
/// organism's in-memory projection would go stale otherwise).
pub(crate) fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git").arg("-C").arg(root).args(args).output().map_err(|e| format!("git: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim_end_matches('\n').to_string())
}

/// The record lives in the repository; a project without one gets one.
/// True when this call created it.
fn ensure_repo(root: &Path) -> Result<bool, String> {
    if git(root, &["rev-parse", "--git-dir"]).is_ok() {
        return Ok(false);
    }
    git(root, &["init", "-q", "-b", "main"]).map(|_| true)
}

// ---------------------------------------------------------------
// The fleet as the expression (GH #566 F5): `fleet.deploy` rows out,
// `instance.up` / `instance.exited` rows back from the nodes
// ---------------------------------------------------------------

/// `hale dna ui [project] [--port N]`: the DNA surface in a browser,
/// from the record alone (GH #566 F6) — a Hale HTTP server over the
/// offline verbs of `hale dna`, built once into the toolchain cache
/// beside the core, run in the project root with this toolchain.
/// A path given from `under` (a directory of the repository, relative to
/// its root) as the repository names it: joined — an absolute one taken
/// from the root — `.` and `..` resolved, never above the root.
fn repo_path(root: &Path, under: &Path, given: &str) -> String {
    let joined = if Path::new(given).is_absolute() { Path::new(given).strip_prefix(root).map(|p| p.to_path_buf()).unwrap_or_else(|_| PathBuf::from(given)) } else { under.join(given) };
    let mut parts: Vec<String> = Vec::new();
    for c in joined.components() {
        match c {
            std::path::Component::ParentDir => {
                parts.pop();
            }
            std::path::Component::Normal(p) => parts.push(p.to_string_lossy().to_string()),
            _ => {}
        }
    }
    parts.join("/")
}

fn ui_cmd(args: &[String]) -> ExitCode {
    let mut dir = PathBuf::from(".");
    let mut port = "8790".to_string();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--port" => match it.next() {
                Some(p) => port = p.clone(),
                None => {
                    eprintln!("hale dna ui: --port needs a value");
                    return ExitCode::from(2);
                }
            },
            f if f.starts_with("--") => {
                eprintln!("hale dna ui: unknown flag `{f}`");
                return ExitCode::from(2);
            }
            p => dir = PathBuf::from(p),
        }
    }
    let run = || -> Result<i32, String> {
        let (root, _) = project(&dir)?;
        let cache = hale_iris::materialize(&crate::build_env::host_cache_options()).map_err(|e| format!("cannot materialize the toolchain cache: {e}"))?;
        let bin = crate::iris::ensure_built_in(&cache, hale_dna::UI_SEED, hale_dna::UI_BIN, "the surface")?;
        let me = std::env::current_exe().map_err(|e| e.to_string())?;
        let _ = host_run("sync", &root, &[]);
        // exec in place, for the same reason as the host: the pid is the surface
        use std::os::unix::process::CommandExt;
        let e = Command::new(&bin).arg(&port).arg(cache.join(hale_dna::UI_HTML.path)).current_dir(&root).env("HALE_BIN", &me).exec();
        Err(format!("hale dna ui: {e}"))
    };
    match run() {
        Ok(code) => ExitCode::from(code.clamp(0, 255) as u8),
        Err(e) => {
            eprintln!("hale dna ui: {e}");
            ExitCode::from(1)
        }
    }
}

// ---------------------------------------------------------------
// GitHub as the Board's surface (GH #566 F4): a mirror of the record, never the record
// ---------------------------------------------------------------

// ---------------------------------------------------------------
// the Journal as read by the host: status / task create / history
// ---------------------------------------------------------------

// ---------------------------------------------------------------
// new
// ---------------------------------------------------------------

fn new_project(dir: &Path, profile: Option<&str>, remote: Option<&str>, body: Option<&str>, library: bool) -> Result<Vec<String>, String> {
    match profile {
        None | Some("local") => {
            if remote.is_some() || body.is_some() {
                return Err("`--remote` and `--body` set the pieces of `--profile remote-body`; the local profile has neither".to_string());
            }
        }
        Some("remote-body") => {
            if remote.is_none() {
                return Err("`--profile remote-body` needs `--remote <url>`: the record shared over that remote is what the body and every head attach to".to_string());
            }
        }
        Some(other) => return Err(format!("unknown profile `{other}`; profiles are examples of combinations — `local` (everything in this clone) or `remote-body` (the record over a remote, the body on a server); `hale dna profile` detects the combination from the pieces")),
    }
    if dir.exists() && fs::read_dir(dir).map(|mut d| d.next().is_some()).unwrap_or(false) {
        return Err(format!("{} exists and is not empty; `hale dna init` attaches to an existing application", dir.display()));
    }
    fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    ensure_repo(dir)?;
    let name = dir
        .canonicalize()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_else(|| "app".to_string());
    let locus = pascal(&name);
    let main_hl = format!(
        r#"/// {name} — a governed application: the organization in dna/org
/// oversees it (proposes, verifies, reviews, expresses its changes)
/// from outside; nothing of that is in this program. Every `.hl` in
/// this directory shares one scope.
type Ping {{ n: Int = 0; }}
topic Pings {{ payload: Ping; subject: "{sub}.ping"; }}

locus Echo {{
    params {{ seen: Int = 0; }}
    bus {{ subscribe Pings as on_ping; }}
    fn on_ping(p: Ping) {{ self.seen = self.seen + 1; }}
}}

main locus {locus} {{
    params {{ echo: Echo = Echo {{ }}; }}
    bus {{ publish Pings; }}
    run() {{
        Pings <- Ping {{ n: 1 }};
        std::time::sleep(100ms);
        println("{name}: ", self.echo.seen, " ping(s) echoed; the bus is open");
        // The application stays up under `hale dna run`;
        // HALE_DNA_ONESHOT makes a run return after the ping.
        if std::env::var_exists("HALE_DNA_ONESHOT") {{ return; }}
        while true {{ std::time::sleep(100ms); }}
    }}
}}

fn main() {{
    {locus} {{ }};
}}
"#,
        name = name,
        locus = locus,
        sub = name.replace('-', "_"),
    );
    let test_hl = format!(
        r#"// `hale test` discovers *_test.hl recursively; this seed imports the
// application and asserts against it, typechecked next to the code.

import ".." as app;

fn main() {{
    let e = app::Echo {{ }};
    std::test::assert_eq_int(e.seen, 0, "a fresh Echo has seen nothing");
}}
"#
    );
    let gitignore = format!("# the build artifact\n/{name}\n# toolchain-managed (vendor is re-materialized; .hale holds sockets, worktrees, scratch; the record is refs/dna/*)\n/vendor/\n/.hale/\n");
    let mut out = Vec::new();
    for (f, c) in [("hale.toml", "[deps]\n".to_string()), ("main.hl", main_hl), ("tests/main_test.hl", test_hl), (".gitignore", gitignore)] {
        let p = dir.join(f);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        fs::write(&p, c).map_err(|e| format!("write {}: {e}", p.display()))?;
        out.push(format!("created {}", p.display()));
    }
    let mut rest = init(dir, library)?;
    out.append(&mut rest);
    if let Some(url) = remote {
        git(dir, &["remote", "add", "origin", url])?;
        out.push(format!("remote origin = {url} (the record is shared over it: `hale dna sync`, and a body on a server takes the lease there)"));
    }
    if let Some(host) = body {
        git(dir, &["config", "dna.body", host])?;
        out.push(format!("dna.body = {host} (where `hale dna body provision` puts the body; `hale dna profile` reports what runs)"));
    }
    Ok(out)
}

fn pascal(s: &str) -> String {
    let mut out = String::new();
    let mut up = true;
    for ch in s.chars() {
        if ch.is_alphanumeric() {
            if up {
                out.extend(ch.to_uppercase());
                up = false;
            } else {
                out.push(ch);
            }
        } else {
            up = true;
        }
    }
    if out.is_empty() || out.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
        out = format!("App{out}");
    }
    out
}

// ---------------------------------------------------------------
// generated source
// ---------------------------------------------------------------

fn purpose_hl(text: &str) -> String {
    format!(
        r#"// dna/org/purpose.hl — the declared purpose (project-owned).
//
// The first governed workflow is the baseline review: the Review
// named `purpose` ratifies THIS text (its subject digest is the
// sha256 of PURPOSE). Change the text and the Review's digest
// together, or the verdict is refused as stale.

const PURPOSE: String = "{}";

fn purpose() -> String {{
    return PURPOSE;
}}
"#,
        text.replace('\\', "\\\\").replace('"', "\\\"")
    )
}

fn org_hl(project: &str, seed: Option<&str>) -> String {
    // what the organization oversees: one application, or (GH #1090) a
    // repository's seeds
    let oversees = match seed {
        Some(seed) => format!("// The application ({seed}) is not part of this program. It carries its\n// own law and is observed like any Hale binary; this organization\n// proposes changes to it, verifies them, and expresses them."),
        None => "// The repository's seeds are not part of this program. Each carries its\n// own law and is observed like any Hale binary; this organization\n// proposes changes to them, verifies them, and expresses them.".to_string(),
    };
    // what verification builds and the genome is, in the program: the
    // application's seed, or the repository's root
    let seed = seed.unwrap_or(".");
    format!(
        r#"// dna/org/main.hl — the organization that oversees {project}
// (project-owned; edit freely). An org chart is a Hale program:
// positions are loci, routing is the bus, a position's capabilities
// are its effect contract, and `hale check` holds the law in law.hl
// against the wiring you actually built. Grow it by adding positions
// and routes; every setting is a constructor argument.
//
//   Board       the human authority: intent enters through it,
//               escalations and reports leave through it, it owns
//               every grant (its verdicts carry `--authority board`)
//   Leader      the model-backed position holding the project's grant:
//               decides the Reviews inside it, leaves the Board's
//   core        the substrate: the record, the gateways, verification,
//               the editing position, the Reviews, the budget
//   models.hl   the catalog: which model each position calls, and the
//               organization's allowance (`hale dna models` probes it)
//
{oversees}

import "vendor/dna" as dna;
import "vendor/dna/pond/realtime/nats" as nats;

main locus Org {{
    params {{
        core: dna::Dna = dna::Dna {{
            // Two memories read as one (GH #646): the record — one commit per
            // event on refs/dna/journal, what changes this organization,
            // synced to every clone — and the ledger, the day's work, in
            // memory (Postgres, opened as this record's spine role) once
            // `hale dna ledger adopt` has moved it there. Until then every
            // row is the record's.
            journal: dna::RoutedJournal {{
                record: dna::GitJournal {{ repo: "." }}
            }},
            // Models: every position's router comes from the catalog in
            // models.hl (a backend is a constructor function there; a
            // hosted one presents its credential from a sealed locus and
            // is not a permitted backend without it). Every call journals
            // its evidence, never the prompt. Agent work — a judgment, an
            // analysis — is a leg's (GH #946): the relay answers pending,
            // and `hale dna work` claims it, performs it with the performers
            // in work.hl, and hands the outcome back.
            work: dna::WorkSystem {{
                agent: dna::LegRelay {{ name: "legs" }},
                agent_reconciler: dna::RelayReplay {{ }}
            }},
            // The organization's spend: one policy (models.hl), one owner.
            budget: dna::Budget {{ policy: org_budget() }},
            // The Leader's grant, owned by the Board: what the organization
            // may decide on its own terms. An `application` change is outside
            // this grant and escalates to the Board; widen it here, in a
            // reviewed commit, as the record earns it.
            boundary: dna::AutonomyBoundary {{
                child: "{project}",
                grant: dna::Grant {{ child: "{project}", classes: "refactor docs", max_magnitude: 4, review: "pre" }}
            }},
            review_policy: dna::OrgPolicy {{ }},
            board: dna::Board {{ who: "board" }},
            gateway: dna::MutationGateway {{
                leases: dna::GitLeases {{ repo: "." }},
                workspaces: dna::IsolatedWorktrees {{ repo: ".", root: ".hale/dna/worktrees" }},
                repo: dna::LocalGit {{ repo: "." }}
            }},
            verification: dna::HaleVerification {{ receipts: dna::GitReceipts {{ repo: "." }}, scratch: ".hale/dna/scratch", repo: ".", seed: "{seed}" }},
            // The editing position: read, edit, fmt and check inside one
            // worktree — the law says so.
            editor: dna::SourceEditor {{ name: "editor", models: editor_models() }},
            genome_seed: "{seed}",
            // The workflows this organization admits (GH #995): the
            // baseline catalog and the project's own, in workflows.hl.
            catalog: workflows(),
            // GH #596 L: an ask is planned by the leader before it becomes a Mutation
            // (the optimize pass occurs on the cadence a ratified practice
            // declares, `operating/optimize-cadence`: GH #1143)
            planned: true
        }};
        // The Leader: decides the Reviews inside the grant, with the deep
        // tier, reading the source diff and the semantic diff; every
        // decision is a model call with evidence in the record.
        leader: dna::Leader = dna::Leader {{
            name: "leader",
            models: leader_models(),
            receipts: dna::GitReceipts {{ repo: "." }},
            source: dna::SourceReader {{ repo: "." }},
            // GH #596 L: what the leader reads before it thinks — its
            // charter and the purpose from this program, the law from the
            // genome, the ratified practices for `org` from memory
            charter: charter(),
            purpose: purpose()
        }};
        // The nerves (GH #986): the facts that enter this organization —
        // a verdict, an intent, the host's observation report — travel
        // over NATS JetStream. This connection reads them through the
        // organization's durable consumer, as the spine, on its own
        // thread, and hands each to the bus; `hale dna dev` gives it its
        // user and the organization's token.
        nerves: nats::NatsConn = nats::NatsConn {{
            url: dna::nerves_spine_url(),
            // the spine's account: its password is the vault's, presented
            // on CONNECT and nowhere else (GH #989)
            user: "spine",
            credential: std::secret::Credential {{ vault: dna::nerves_role_vault("spine") }},
            name: "organization",
            subject_prefix: dna::nerves_subject_prefix(),
            stream: dna::nerves_stream_here(),
            consumer: nats::ConsumerSpec {{ durable: dna::nerves_durable(), filter: dna::nerves_filter() }},
            // what this organization publishes itself (its Leader's
            // verdicts) is acknowledged by the stream, as the node's is
            jetstream: true,
            run_for_ms: if std::env::var_exists("HALE_DNA_ONESHOT") {{ 1 }} else {{ 0 }}
        }};
    }}
    claims {{ adopt Org; }}
    placement {{ nerves: pinned; }}
    // Where a verdict, an intent and the host's observation report enter:
    // the nerves. A node publishes each once its row is in the record, and
    // again until the record holds the answer; the owning loci decide.
    bindings {{
        dna::ReviewVerdict: nats::NatsAdapter {{ }};
        dna::IntentOffered: nats::NatsAdapter {{ }};
        dna::ExpressionObserved: nats::NatsAdapter {{ }};
        dna::PressureRaised: nats::NatsAdapter {{ }};
        dna::ConcernRaised: nats::NatsAdapter {{ }};
        dna::PracticeRequested: nats::NatsAdapter {{ }};
        dna::HoldRequested: nats::NatsAdapter {{ }};
        dna::KnowledgeNodeRequested: nats::NatsAdapter {{ }};
        dna::KnowledgeBindingRequested: nats::NatsAdapter {{ }};
        dna::KnowledgeEdgeRequested: nats::NatsAdapter {{ }};
        dna::WorkSubmit: nats::NatsAdapter {{ }};
        dna::WorkAllowanceAsk: nats::NatsAdapter {{ }};
    }}
    // The nerves collapsed (a fact the stream would not take): this
    // organization stops, and the host that supervises it stops too and
    // is started again by its unit, both connections fresh. The record
    // keeps every request unanswered until an organization answers it.
    on_failure(c: nats::NatsConn, err: ClosureViolation) {{
        eprintln("organization: the nerves failed (", c.last_error, "); stopping for the host to start it again");
        std::process::exit(dna::NODE_RESTART);
    }}
    run() {{
        if std::env::var_exists("HALE_DNA_ONESHOT") {{ return; }}
        // GH #1143: the schedules the record declares occur on this tick,
        // on the wall clock in milliseconds (an occurrence is named by its
        // time). Queue it with incoming work so a journal refresh completes
        // before a task handler can read or append its cached view. A
        // SIGTERM (the host stopping it) ends it: the runtime drains.
        while !self.draining {{ std::time::sleep(100ms); self.core.request_tick(std::time::nanos(std::time::current()) / 1000000); }}
    }}
}}

fn main() {{
    Org {{ }};
}}
"#
    )
}

/// `dna/org/workflows.hl`: the workflow catalog's generated half (GH
/// #995), written at `new` and rewritten to the current shape at every
/// `upgrade`: the vendored toolchain's `dna::baseline_definitions()`, which
/// an upgrade supersedes by revision, and the project's own definitions
/// from `own_workflows.hl`.
const WORKFLOWS_HL: &str = r#"// dna/org/workflows.hl — the workflow catalog (generated by `hale dna new`
// and rewritten by `hale dna upgrade`; do not edit: your definitions go in
// own_workflows.hl). The definitions this organization admits: DNA's
// baseline, each a chain in which every step writes one store, and the
// project's own beside it. `hale dna definitions` lists them.
//
// The baseline is the vendored toolchain's, so an upgrade supersedes it by
// revision: a Task born under an old revision finishes under it, and a new
// one binds the newest. A definition with a step on a part not built yet
// is listed, and refused at admission naming the part.

import "vendor/dna" as dna;

fn workflows() -> dna::WorkflowCatalog {
    let catalog = dna::baseline_definitions();
    catalog.refuse_own(own_workflows(catalog));
    return catalog;
}
"#;

/// `dna/org/structure.hl` (GH #1091, B): the one structure policy. The
/// holes `init` proposes follow the graph's edges; what no edge implies —
/// the operational roles under each deployment — is the project's to opt
/// into. Generated, and rewritten by `upgrade` to the current shape with
/// the project's roles carried over.
fn structure_hl(roles: &str) -> String {
    format!(
        r#"// dna/org/structure.hl — the structure policy (generated by `hale dna new`
// and `hale dna init`, rewritten by `hale dna upgrade`, which keeps the
// roles below).
//
// The holes `init` proposes follow the graph's edges: a reviewer for each
// process or seed that serves or consumes a contract, a dev and a work item
// for each one a gate guards, an operator under each deployment. What no
// edge implies is here: the operational roles each deployment also gets,
// proposed empty for the Board to fill — none by default. Name them
// space-separated, from `support accounts billing on-call`; any other word
// is refused. They are read once, when `hale dna init` seeds the record:
// to opt in, write this file before `init`, which keeps it.

fn operational_roles() -> String {{
    return "{roles}";
}}
"#
    )
}

/// The roles a `structure.hl` opts into: the literal `operational_roles`
/// returns, "" when the file says none or has no such function.
fn structure_roles(text: &str) -> String {
    let Some(at) = text.find("fn operational_roles") else { return String::new() };
    let rest = &text[at..];
    let Some(q) = rest.find("return \"") else { return String::new() };
    let lit = &rest[q + 8..];
    lit.find('"').map(|e| lit[..e].split_whitespace().collect::<Vec<_>>().join(" ")).unwrap_or_default()
}

/// `dna/org/own_workflows.hl`: the project's own workflow definitions (GH
/// #995). Project-owned: written at `new`, and at `upgrade` only where
/// there is none.
const OWN_WORKFLOWS_HL: &str = r#"// dna/org/own_workflows.hl — this organization's own workflow definitions
// (project-owned; generated by `hale dna new`, never rewritten). They sit
// beside DNA's baseline in the catalog workflows.hl returns.
//
// A definition names the one store each step writes (`record`, `forge`,
// `genome`, `heart`, `graph`, `nerves`, `memory`, `vault`, `host`) — a
// step is where a fact is written — then its members:
//
//     let d = catalog.define("close-month", 1, "record record", "close the month");
//     let e = catalog.leaf("close-month", 1, 0, "books", dna::WorkRequest { requires: "human" }, 1);
//
// "" when every definition was taken, else why one was not.

import "vendor/dna" as dna;

fn own_workflows(catalog: dna::WorkflowCatalog) -> String {
    return "";
}
"#;

/// The foundational law of an organization (GH #566 F2), from the rules
/// an executable org chart lives by: a position's capabilities are its
/// effect contract; nothing applies except through the substrate; the
/// editing position reaches neither git, nor the worktree gateway, nor a
/// deployment, nor the organization's memory; credentials stay sealed.
/// Project-owned: add clauses, don't weaken these.
fn org_law_hl() -> String {
    r#"// dna/org/law.hl — the organization's law (project-owned; generated by
// `hale dna init`). Adopted by the org's main (`adopt Org;`). Add
// clauses; do not weaken these. Growth adds positions and routes that
// still satisfy them: the compiler refuses the rest before anyone reviews.

import "vendor/dna" as dna;

group board = { dna::Board };
group leader = { dna::Leader };
group substrate = { dna::Dna };
group positions = { dna::Leader, dna::SourceEditor, dna::WorktreeTools, dna::LegRelay, dna::RelayReplay, dna::HumanWorkGateway, dna::ServicePerformer, dna::ScriptedPerformer };
group editors = { dna::SourceEditor, dna::WorktreeTools };
group knowledge = { dna::Knowledge, dna::MemoryKnowledge };
group credentials = { dna::CredentialSource, dna::HostedCredential };

constitution Org {
    // Nothing is applied except through the substrate's gate, after a
    // settled Review: the path from any position to `genome_apply` runs
    // through `dna::Dna` or it does not exist.
    apply_only_through_the_substrate: forbid reaches(positions, effects(genome_apply)) avoiding substrate;
    // The editing position holds nothing but its worktree grant — no
    // git, no worktree gateway, no apply, no memory. A wiring that hands
    // it any of them is a build failure with a witness, not a runtime check.
    editors_never_commit: forbid reaches(editors, effects(repo_write));
    editors_never_touch_worktrees: forbid reaches(editors, effects(worktree_io));
    editors_never_apply: forbid reaches(editors, effects(genome_apply));
    editors_never_learn: forbid reaches(editors, knowledge);
    // The Leader decides; the substrate acts. Its verdict reaches the
    // genome only through `dna::Dna` (the bus edges Review -> Dna -> gateway),
    // never by holding a repository or a worktree itself.
    leader_never_commits: forbid reaches(leader, effects(repo_write)) avoiding substrate;
    leader_never_touches_worktrees: forbid reaches(leader, effects(worktree_io)) avoiding substrate;
    // Credentials are read from their source into sealed loci and never returned.
    credentials_sealed: require sealed(all credentials);
    // Money moves only through the substrate, which reserves a spend
    // against the grant before it is made (GH #605): a position that
    // reaches a money effect any other way is a build failure.
    money_only_through_the_substrate: forbid reaches(positions, effects(money)) avoiding substrate;
}
"#
    .to_string()
}

/// The person who runs `init`, as the host names them (`USER`, else `human`).
fn initializer() -> String {
    std::env::var("USER").ok().filter(|u| !u.trim().is_empty()).unwrap_or_else(|| "human".to_string())
}

/// GH #946, GH #1104 piece 5: the record's first seat. The head's socket
/// lists a verb to a peer whose person the record says holds a position;
/// a record that declares `dna.trust = local` is one person's, who holds
/// every position as they hold every authority there, so the seat is
/// the record's local config alone: the initializer's uid mapped to them
/// (`dna.unix.member`), and — only where the record is made, by `new`
/// or `init` — the trust declared. `upgrade` maps the uid and says how
/// to declare, never declaring for a record it did not make: an
/// existing record's authorization is its own. No row in the record —
/// the graph's positions and holders stay the graph's. Says what it did.
fn seat_initializer(root: &Path, declare_trust: bool) -> Result<String, String> {
    let name = initializer();
    let uid = Command::new("id").arg("-u").output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    if uid.is_empty() {
        return Ok(format!("seated  this uid could not be read; map it to {name} for the head's socket: git config --local --add dna.unix.member \"uid:<n>={name}\""));
    }
    let entry = format!("uid:{uid}={name}");
    let have = git(root, &["config", "--local", "--get-all", "dna.unix.member"]).unwrap_or_default();
    if !have.lines().any(|l| l.trim() == entry) {
        git(root, &["config", "--local", "--add", "dna.unix.member", &entry])?;
    }
    // the trust the record declares, wherever git reads it from
    let trust = git(root, &["config", "--get", "dna.trust"]).unwrap_or_default().trim().to_string();
    if declare_trust && trust.is_empty() {
        git(root, &["config", "--local", "dna.trust", "local"])?;
        return Ok(format!("seated  the head's socket knows uid {uid} as {name} (dna.unix.member); the record declares dna.trust = local, where they hold every position"));
    }
    if trust.is_empty() {
        return Ok(format!("seated  the head's socket knows uid {uid} as {name} (dna.unix.member); the record declares no trust, so the graph's holds edges say who holds what — `git config --local dna.trust local` declares one person's record"));
    }
    Ok(format!("seated  the head's socket knows uid {uid} as {name} (dna.unix.member); the record declares dna.trust = {trust}"))
}

/// The seed events, collected then appended to the record in order.
struct Chain {
    lines: Vec<(String, String, String)>,
}

impl Chain {
    fn new() -> Self {
        Chain { lines: Vec::new() }
    }
    fn push(&mut self, kind: &str, entity: &str, body: &str) {
        self.lines.push((kind.to_string(), entity.to_string(), body.to_string()));
    }
}

/// GH #989: the name the record attaches the application by, which its
/// broker account carries (`app-<name>`, publishing on `<org>.app.<name>.>`):
/// the project's name as a subject token, as `dna::nerves_app_name` spells
/// it — lower-case letters and digits, anything else `_`, at most 32.
fn app_account_name(project: &str) -> String {
    project.chars().take(32).map(|c| c.to_ascii_lowercase()).map(|c| if c.is_ascii_lowercase() || c.is_ascii_digit() { c } else { '_' }).collect()
}

/// What `hale check --api` exports for the application: the surfaces it
/// serves and the streams over a hub, as the document the graph ingest reads.
/// None when the toolchain cannot describe it (the graph then holds no
/// surface contract, and the artifact's rows are unchanged).
fn api_description(app: &App) -> Option<String> {
    let me = std::env::current_exe().ok()?;
    let out = Command::new(me).arg("check").arg("--api").arg(&app.seed).stderr(std::process::Stdio::null()).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    if out.status.success() && text.trim_start().starts_with('{') { Some(text) } else { None }
}

/// The application's root seed in the graph: its project directory, or the
/// directory it sits in when the application is not the repository's root.
fn app_seed_name(app: &App) -> String {
    if app.seed_rel == "." { app.project.clone() } else { app.seed_rel.clone() }
}

/// An application's record, seeded in one checked call: the artifact's rows
/// (what it observed), the purpose's proposal, then the graph its directory
/// holds and the holes that graph implies (as for a repository, with the
/// application as the root seed and its API surfaces as contracts). Returns
/// the artifact's row count and what the ingest read.
fn seed_journal(root: &Path, app: &App, art: &Value, purpose: &str) -> Result<(usize, String), String> {
    let mut c = Chain::new();
    let s = |v: &Value| v.as_str().unwrap_or("").to_string();
    let names = |v: &Value| -> Vec<String> { v.as_array().map(|a| a.iter().map(|x| s(x)).collect()).unwrap_or_default() };
    c.push(
        "application.attached",
        &app.seed_rel,
        &serde_json::json!({
            "main": app.main_name, "name": app_account_name(&app.project), "artifact": BASELINE_REL,
            "artifact_digest": s(&art["artifact_digest"]), "shape_hash": s(&art["shape_hash"]),
            "schema": s(&art["schema"]), "verdict": s(&art["verdict"]),
            "toolchain": TOOLCHAIN, "provenance": "observed"
        })
        .to_string(),
    );
    // loci, from the per-locus contracts (schema 1.18+)
    for l in art["contracts"].as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
        let name = s(&l["locus"]);
        let subs: Vec<String> = l["subscribes"].as_array().map(|a| a.iter().map(|x| s(&x["topic"])).collect()).unwrap_or_default();
        let owners: BTreeSet<String> = l["instances"].as_array().map(|a| a.iter().filter_map(|x| x["owner"].as_str().map(|o| o.to_string())).collect()).unwrap_or_default();
        c.push(
            "structure.observed",
            &format!("locus:{name}"),
            &serde_json::json!({
                "kind": "locus", "name": name, "sealed": l["sealed"],
                "params": l["params"], "methods": l["methods"],
                "publishes": l["publishes"], "subscribes": subs,
                "supervises": l["supervises"], "instances": l["instances"],
                "provenance": "observed"
            })
            .to_string(),
        );
        // A GUESS at responsibility, from what the locus does — never ratified here.
        let pubs = names(&l["publishes"]);
        let mut parts: Vec<String> = Vec::new();
        if !subs.is_empty() {
            parts.push(format!("reacts to {}", subs.join(", ")));
        }
        if !pubs.is_empty() {
            parts.push(format!("emits {}", pubs.join(", ")));
        }
        if !owners.is_empty() {
            parts.push(format!("owned by {}", owners.iter().cloned().collect::<Vec<_>>().join(", ")));
        }
        if name == app.main_name {
            parts.push("the entrypoint and root of the ownership tower".to_string());
        }
        c.push(
            "responsibility.proposed",
            &format!("locus:{name}"),
            &serde_json::json!({
                "responsibility": if parts.is_empty() { "unclear from structure alone".to_string() } else { parts.join("; ") },
                "provenance": "inferred", "ratified": false
            })
            .to_string(),
        );
    }
    for t in art["topics"].as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
        c.push(
            "structure.observed",
            &format!("topic:{}", s(&t["name"])),
            &serde_json::json!({"kind": "topic", "name": t["name"], "subject": t["subject"], "shape": t["shape"], "payload_hash": t["payload_hash"], "provenance": "observed"}).to_string(),
        );
    }
    for b in art["bindings"].as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
        c.push(
            "structure.observed",
            &format!("binding:{}:{}", s(&b["topic"]), s(&b["role"])),
            &serde_json::json!({"kind": "binding", "topic": b["topic"], "subject": b["subject"], "transport": b["transport"], "role": b["role"], "loss": b["loss"], "provenance": "observed"}).to_string(),
        );
    }
    for e in art["law"]["effect_classes"].as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
        let name = if e.is_string() { s(e) } else { s(&e["name"]) };
        c.push(
            "structure.observed",
            &format!("effect_class:{name}"),
            &serde_json::json!({"kind": "effect_class", "name": name, "provenance": "observed"}).to_string(),
        );
    }
    for cl in art["claims"].as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
        c.push(
            "structure.observed",
            &format!("claim:{}", s(&cl["name"])),
            &serde_json::json!({"kind": "claim", "name": cl["name"], "form": cl["form"], "result": cl["result"], "source": cl["source"], "provenance": "observed"}).to_string(),
        );
    }
    // through the host, which holds the record's one implementation
    // (GH #646 stage 0): the rows as a file, one JSON object per line
    let dna_dir = root.join(".hale/dna");
    fs::create_dir_all(&dna_dir).map_err(|e| e.to_string())?;
    let path = dna_dir.join(format!("seed.{}.jsonl", std::process::id()));
    let mut text = String::new();
    for (kind, entity, body) in &c.lines {
        text.push_str(&serde_json::json!({"kind": kind, "entity": entity, "body": body}).to_string());
        text.push('\n');
    }
    fs::write(&path, text).map_err(|e| e.to_string())?;
    // GH #995: the declared purpose is proposed in the same seed (a
    // knowledge proposal and the Board's Review: two rows)
    let purpose_path = dna_dir.join(format!("purpose.{}.txt", std::process::id()));
    fs::write(&purpose_path, purpose).map_err(|e| e.to_string())?;
    let mut args = vec![purpose_path.to_string_lossy().to_string(), "--rows".to_string(), path.to_string_lossy().to_string(), "--app".to_string(), app_seed_name(app)];
    let description_path = dna_dir.join(format!("api.{}.json", std::process::id()));
    if let Some(d) = api_description(app) {
        fs::write(&description_path, d).map_err(|e| e.to_string())?;
        args.push("--description".to_string());
        args.push(description_path.to_string_lossy().to_string());
    }
    let out = host_run("graph-ingest", root, &args);
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&purpose_path);
    let _ = fs::remove_file(&description_path);
    Ok((c.lines.len(), out?))
}

/// GH #1090: a repository's record — the declared purpose's proposal
/// (GH #995), then what the repository holds as the graph — seeded by the
/// host's `graph-ingest` in one call that checks
/// every row before any lands, so an ingest the graph refuses leaves no
/// record and `init` can be run again. What it read, by kind.
fn seed_repository(root: &Path, purpose: &str) -> Result<String, String> {
    let dna_dir = root.join(".hale/dna");
    fs::create_dir_all(&dna_dir).map_err(|e| e.to_string())?;
    let path = dna_dir.join(format!("purpose.{}.txt", std::process::id()));
    fs::write(&path, purpose).map_err(|e| e.to_string())?;
    let out = host_run("graph-ingest", root, &[path.to_string_lossy().to_string()]);
    let _ = fs::remove_file(&path);
    out
}

/// Whether the record exists: the host answers `none` or its head.
fn record_exists(root: &Path) -> Result<bool, String> {
    Ok(host_run("record-head", root, &[])?.trim() != "none")
}

/// One seeded family, proposed where the record does not hold its current
/// text (all of it at `init`; what changed, or a family the record has
/// never seen, at `upgrade`): the
/// practices handed to the host as a file, the host deciding against
/// the record. Returns (proposed, of which superseding, waiting on a
/// pending Review).
fn design_upgrade(root: &Path, family: &[SeededPractice]) -> Result<(usize, usize, usize), String> {
    let dna_dir = root.join(".hale/dna");
    fs::create_dir_all(&dna_dir).map_err(|e| e.to_string())?;
    let path = dna_dir.join(format!("design.{}.jsonl", std::process::id()));
    let mut text = String::new();
    for p in family {
        text.push_str(&serde_json::json!({"name": p.name, "text": design_text(p), "schedule": p.schedule, "target": p.target}).to_string());
        text.push('\n');
    }
    fs::write(&path, text).map_err(|e| e.to_string())?;
    let out = host_run("design-upgrade", root, &[path.to_string_lossy().to_string()]);
    let _ = fs::remove_file(&path);
    let out = out?;
    let field = |name: &str| -> Result<usize, String> {
        out.split_whitespace()
            .find_map(|w| w.strip_prefix(&format!("{name}=")))
            .and_then(|v| v.parse().ok())
            .ok_or_else(|| format!("design-upgrade answered oddly: {out}"))
    };
    Ok((field("proposed")?, field("superseding")?, field("waiting")?))
}

// ---------------------------------------------------------------
// GH #596: the leader's charter, and the toolchain's design
// ---------------------------------------------------------------

/// GH #1143: what an older generator wrote in `main.hl` for the optimize
/// cadence and the loop's clock, and what it writes now; `upgrade` takes
/// each exact old text to its new one.
const SCHEDULE_MAIN_HL: [(&str, &str); 3] = [
    (
        "            planned: true,\n            // GH #596 O: the optimize pass — the leader walks the machinery\n            // on this cadence, in milliseconds; 0 is never. The Board's to set.\n            optimize_every_ms: 0\n",
        "            // (the optimize pass occurs on the cadence a ratified practice\n            // declares, `operating/optimize-cadence`: GH #1143)\n            planned: true\n",
    ),
    (
        "        // GH #596 O: the substrate's cadence — the optimize pass fires\n        // every `optimize_every_ms` on the substrate above (0 = never).\n        // Queue it with incoming work so a journal refresh completes\n",
        "        // GH #1143: the schedules the record declares occur on this tick,\n        // on the wall clock in milliseconds (an occurrence is named by its\n        // time). Queue it with incoming work so a journal refresh completes\n",
    ),
    ("self.core.request_tick(std::time::monotonic_ns() / 1000000); }", "self.core.request_tick(std::time::nanos(std::time::current()) / 1000000); }"),
];

/// The Board's field as an older generator wrote it in `main.hl`, and as it
/// writes it now; `upgrade` takes the exact old line to the new one.
const BOARD_MAIN_HL: (&str, &str) = ("            membrane: dna::Board { who: \"board\" },\n", "            board: dna::Board { who: \"board\" },\n");

/// The generated `main.hl`'s owners map field before GH #1123, which
/// `upgrade` takes out.
const OWNERS_MAIN_HL: &str = "            // Owners (GH #664): who admits which position. The map is the
            // genome's file dna/org/owners — empty while this organization
            // is the only owner; once the record is shared, every position
            // names its owner and this body says which it is (dna.owner).
            ownership: dna::Ownership { path: \"dna/org/owners\" },
";

/// `dna/org/charter.hl`: what the leader reads before it thinks. A
/// function returning text, like `purpose`, so a change to the brief
/// is a mutation of the org program the Board reviews.
fn charter_hl(project: &str) -> String {
    let text = format!(
        "You are the architect of {project}'s organization: you propose, the Board decides. \
The Board is whoever holds this record's authority — one person at any level, an IC, a lead, a founder — never an employer's board by implication. \
The organization is a DNA organism, the nervous system of a software-enabled organization whose product may or may not be software. \
Under it are three kinds of thing it can change about itself: the organism (its own positions, laws and purpose), appendages (software it grows to do its own work), and products (software it ships to others). \
They differ by policy, not mechanism: how wide a grant is, who must sign, whether a change ships on approval or waits for a release. \
Every ask enters the record as intent and a Task is born; your first act is to say what the ask is: which kind of thing it concerns, one change or several, what class of change, or that it is not a software change at all. A plan only splits and classifies; it never widens what was asked. \
Nothing applies except through the gate: a change is a candidate the record can show, checked and reviewed, and it lands whole or not at all. \
A grant is the Board's leash on you: inside it you decide, outside it you escalate, and a grant only narrows under failure. \
The record is the organization's memory and yours: read it before you reason. \
Structure follows intent: propose no change to the organism before its purpose, its law and its knowledge exist. \
The organization grows by proposal, a position, a rule, a department, and you may propose your own team: when asks keep falling through to you for triage, a secretary that triages them is a proposal like any other, justified by that signal and by minimal structure and appropriate depth, and born only when the Board ratifies it. \
A software change is one outcome among several; say when an ask is a person's job."
    );
    format!(
        r#"// dna/org/charter.hl — the leader's brief (project-owned; generated by
// `hale dna init`). The leader reads this, the purpose, the law and
// the ratified design before it plans an ask and before it decides a
// Review. Change it as you would any part of the organism: through a
// reviewed change.

const CHARTER: String = "{}";

fn charter() -> String {{
    return CHARTER;
}}
"#,
        text.replace('\\', "\\\\").replace('"', "\\\"")
    )
}

/// One seeded practice (a `design/*`, `operating/*` or `using/*` family): a stable
/// name across versions of the toolchain, and the text the Board
/// ratifies or declines.
struct SeededPractice {
    name: &'static str,
    text: &'static str,
    /// GH #1143: a schedule the practice declares once the Board ratifies
    /// it — `{id, every_ms | cron, definition, args, convener}` as JSON —
    /// or "" for none.
    schedule: &'static str,
    /// A mandate: the graph node (`position:leader`) the text is bound to,
    /// or "" for a practice, which binds to the organization. A practice
    /// with a target is proposed as a `mandate` bound to it.
    target: &'static str,
}

/// The design: how a DNA organization works, as practices bound to
/// `org` — brained's structural knowledge adapted to DNA. Proposed at
/// `init`, one Board Review each; never ratified by the toolchain.
const DESIGN: &[SeededPractice] = &[
    SeededPractice { name: "design/mandate-leader", text: "The Leader: the organization's architect. Decides: what an ask is (its kind, class and count) and the plan that splits and classifies it, and every change inside its grant whose class is not the Board's and that touches no law, widens no effects and crosses no ownership; it proposes its own team when asks keep falling through to it. May not: widen what was asked, ratify a change of class `organization`, `constitutional`, `process-policy` or `topology` (or one that touches law, widens effects or crosses ownership: those are the Board's), write source, or hold a repository or a worktree itself. Cites: the charter, the purpose, the law and the ratified practices; a plan names the rows it stands on. Writes: a plan that only splits and classifies, and a verdict with its reason. Escalates: to the Board whatever the grant does not cover, and any proposal for structure the purpose does not yet ground.", schedule: "", target: "position:leader" },
    SeededPractice { name: "design/mandate-editor", text: "The editor: the performer of an `edit` Work. Decides: how the Work's objective is met inside its target: the change itself, its size and its order. May not: commit, apply, touch a worktree outside its grant or read the knowledge store directly (the law forbids each); ratify, review or sign anything. Cites: the Work's objective and target, the contracts the target serves or consumes, and the knowledge its package carries. Writes: a small change with its evidence first (what was run, against what, what it printed), naming what it did not run. Escalates: back to the Leader, with the row that shows why, an objective the target cannot meet or a change that must cross a contract or another part.", schedule: "", target: "position:editor" },
    SeededPractice { name: "design/mandate-agent", text: "The agent: the performer of a Work that names no kind. Decides: how it carries out the Work's objective with the tools its hat grants, which are none beyond reading until the Board grants more. May not: act outside its grant, ratify or sign anything, or remember anything between tasks (the hat is read per task). Cites: the Work's objective and the knowledge its package carries. Writes: what it did and what it found as rows a reviewer can read, evidence before the claim. Escalates: to the Leader anything the objective did not foresee.", schedule: "", target: "position:agent" },
    SeededPractice { name: "design/principles", text: "Minimal structure: add a position only when it serves the whole; complexity is cost. Clean cuts: responsibilities do not overlap, and work that keeps crossing a boundary says the boundary is wrong. Appropriate depth: specialize only when a domain genuinely bifurcates. Team size: three at least (triangulation), seven at most (the ceiling of attention); beyond seven, decompose. Contract invariance: when a part restructures inside, its parent's contract does not change; a position's capabilities are its effect contract, and the compiler holds it.", schedule: "", target: "" },
    SeededPractice { name: "design/evolution", text: "Start minimal: the Board, the leader, the substrate, one child. Let work reveal where structure is needed. Add operators before supervisors: an operator is cheap, a supervisor adds management. Promote a position to a department only when its domain bifurcates, not before. Re-evaluate on a cadence: structure should match current work, not history.", schedule: "", target: "" },
    SeededPractice { name: "design/structure-follows-intent", text: "Propose no change to the organism before its purpose (what), its law and grants (how) and its knowledge (the domain's terms and practices) exist. A structure proposed without them produces positions with empty identities, useless to anyone holding them. The sequence is: the Board states purpose and how, the leader proposes structure grounded in both, the Board reviews, the substrate materializes.", schedule: "", target: "" },
    SeededPractice { name: "design/standard-equipment", text: "Every part that supervises others is born with its architect: the position that holds the design for its path, proposes the rest of its team, and never decides. The architect's first proposal is usually the expert for its domain; after that, researchers, planners and deliverers as the work requires. At the root, the leader is the organism's architect. A position is fitted on fill with its equipment: the API roles its surface's `requires` names (the broker account and the vault names join them when an application attaches in repository mode); the equipment informs what a holder is fitted with and never what it may decide.", schedule: "", target: "" },
    SeededPractice { name: "design/signals", text: "Read the record for structural signals. Asks that fall through to the leader with no route: routing is incomplete or a position is missing. A position with no work over a window: possibly unnecessary. Concerns accumulating at a child: that subtree is under strain and may need capacity or a different cut. Changes that cross between siblings: their shared parent is missing logic, or the boundary is wrong. A grant that keeps contracting: the work under it is failing and needs a different shape, not a wider leash. Each is an input to a proposal, or to saying the state is clean.", schedule: "", target: "" },
    SeededPractice { name: "design/signaling", text: "Goals flow down: authored above, bound below, they say what the whole wants of the part. Concerns flow up: authored below, bound above, they say what the part cannot solve alone. Initiatives bridge: self-authored, they turn a goal and its concerns into work. The direction is the classification; nothing else labels them. Three concerns from one source become a proposal by that source; a concern that persists across cycles is being ignored.", schedule: "", target: "" },
    SeededPractice { name: "design/optimize", text: "On a cadence the Board sets, walk the machinery, not the work: are the change classes right, are Reviews going to the right authority, is the routing catching what it should, does the topology still fit, is the knowledge still true. Propose one small change with its reasoning, or record that the state is clean. Never propose a large restructure unprompted, and never create work for the sake of activity.", schedule: "", target: "" },
    SeededPractice { name: "design/software-delivery", text: "For an appendage or a product: process boundaries first (what runs, fails and scales independently), then the shapes and verbs that flow between them. Deliver vertical slices that can be demonstrated, never horizontal layers that cannot. Know a change's kind before starting, aesthetic, functional or structural, and update in dependency order. The specification is the source of truth; changes flow from it. The primary test surface is an integration harness through the real system, with the model as the only injected dependency; unit tests sparingly, for pure logic.", schedule: "", target: "" },
];

/// The operating practices (GH #994): how the organism runs, which is what
/// a leader plans within and a reviewer cites — the design says how an
/// organization is shaped. Seeded beside the design, one Board Review
/// each, superseded on `upgrade` the same way; never ratified by the
/// toolchain.
const OPERATING: &[SeededPractice] = &[
    SeededPractice { name: "operating/one-store-per-step", text: "a workflow step writes to exactly one store, by that store's one writer, and the next step reads what the previous one made durable. The record numbers the steps. A step whose subject moved is refused and asked again on the new subject; nothing is half-written. There is no distributed transaction anywhere, and a plan that needs one is wrong.", schedule: "", target: "" },
    SeededPractice { name: "operating/row-first", text: "every live signal is a record row before it is sent. An event names a row by id, is delivered at least once, and is consumed idempotently by that id. A message never admits anything. An event from outside (the heart) is a signal, not a fact: it becomes a row before anything acts on it.", schedule: "", target: "" },
    SeededPractice { name: "operating/readings-never-act", text: "a reading from the senses never acts. It is kept for a window; what matters crosses into the record as a pressure or concern row, and that row is what a workflow answers to. The organism never believes the heart is healthy without the heart's own pulse.", schedule: "", target: "" },
    SeededPractice { name: "operating/legs-hold-nothing", text: "a leg holds nothing between tasks. The hat is read per task, the credential fetched per task, the result settled per task. A leg that remembers is a bug, and a leg that cannot settle within its lease is a violation its owner records.", schedule: "", target: "" },
    SeededPractice { name: "operating/deploy-settles-on-pulse", text: "a deploy is settled on the heart's own first event, or rolled back. A rollback restores the source revision, never the work already done in the world, and is a new step in the record.", schedule: "", target: "" },
    SeededPractice { name: "operating/the-forge-decides", text: "what merges is decided at the forge, by people, and comes back as a verdict row once. The forge is truth for humans; the record is truth for the organism.", schedule: "", target: "" },
    SeededPractice { name: "operating/optimize-cadence", text: "walk the machinery on a cadence: the optimize pass is an execution of optimize-walk, convened by the leader once a day, which proposes one small change or records that the state is clean. The cadence is this practice's; a different one is an amendment the Board ratifies.", schedule: r#"{"id": "optimize", "every_ms": 86400000, "definition": "optimize-walk", "args": "{}", "convener": "position:leader"}"#, target: "" },
];

/// The using practices: how a person or an agent works with the organism,
/// which is what a brief tells whoever is about to act in it — the design
/// says how an organization is shaped, the operating practices how the
/// organism runs. Seeded beside them, one Board Review each, superseded on
/// `upgrade` the same way, and kept by `--no-library` (they are practices,
/// not the library); never ratified by the toolchain.
const USING: &[SeededPractice] = &[
    SeededPractice { name: "using/propose-review-ratify", text: "nothing is in force until the Board says so. A change to the organization is a proposal, then a Review, then a ratification, in that order: a proposal states what changes and why, and names who decides; one that states only what changes cannot be judged. An unratified proposal binds nobody, and a decision is a row in the record, never a conversation.", schedule: "", target: "" },
    SeededPractice { name: "using/change-classes", text: "name the class and the magnitude of a change before the work starts. The class decides who signs: an organization, constitutional, process-policy or topology change is the Board's; a change that touches law, widens effects or crosses ownership is the Board's whatever its class; the rest, inside the grant, is the Leader's. Work begun without a class is classified afterwards by the reviewer, and may be sent back whole.", schedule: "", target: "" },
    SeededPractice { name: "using/ask-the-leader", text: "the Leader plans the work and you carry it out: ask it what it plans for you before you start, and take its plan as the frame. What lies inside your grant and your position's identity you decide alone and record; what changes the plan, the grant or another position's work is the Leader's to decide, and you ask it again, with the row that shows why.", schedule: "", target: "" },
    SeededPractice { name: "using/cut-structure", text: "a position or a part goes when the record says it is not carrying its weight: no work over a window, or concerns pooling at a child that it cannot solve alone. Propose the cut with the rows that show it, and say what takes over its responsibilities; a part is not removed on a feeling, and it is not kept out of habit.", schedule: "", target: "" },
    SeededPractice { name: "using/read-the-record", text: "the record is the organization's memory; a brief is only a view of it. Before acting on what you were told, read the rows it rests on, and when you report, cite rows, not recollection. A statement that no row supports is an opinion, and the organism acts on rows.", schedule: "", target: "" },
    SeededPractice { name: "using/evidence-first", text: "a candidate's evidence is what a reviewer reads, so it comes before the claim. Say what was run, against what, and what it printed; a check that was not run is named as not run, never described as passing. Evidence a reviewer cannot reproduce is a request to be trusted, and the Board does not ratify on trust.", schedule: "", target: "" },
    SeededPractice { name: "using/bind-knowledge", text: "knowledge applies where it is bound. Bind a chapter or a practice to the node it is about (the language, the system, a part of the organization), not to everyone: a package for a target carries the ideas bound to it and no others, and an idea bound too widely crowds out what the work needs.", schedule: "", target: "" },
];

// The toolchain's library: the book per chapter and the spec per section,
// each its text as the toolchain ships it, one idea apiece, generated from
// the tree by build.rs (`(name, title, target node, text, text digest)`; a
// new chapter joins by existing).
include!(concat!(env!("OUT_DIR"), "/library_embed.rs"));

/// The library's version: the toolchain's. `HALE_DNA_LIBRARY_VERSION`
/// names another, for fixtures only: it is how a test makes "a later
/// toolchain" out of the one binary it has, so that `upgrade`'s supersession
/// is exercised against real record history.
fn library_version() -> String {
    match std::env::var("HALE_DNA_LIBRARY_VERSION") {
        Ok(v) if !v.is_empty() => v,
        _ => TOOLCHAIN.to_string(),
    }
}

/// The family an idea is proposed under, from the node it is bound to: the
/// language's ideas are `language`, the system's `design`.
fn library_family(target: &str) -> &'static str {
    if target == "system:dna" { "design" } else { "language" }
}

/// Chapters the book moved: `(name now, name an earlier toolchain proposed it
/// under)`. An upgrade supersedes the active idea under the earlier name when
/// the new name has none, so a record seeded before the move is not left with
/// both.
const LIBRARY_EARLIER: &[(&str, &str)] = &[("library/services/api", "library/api"), ("library/dna/shaping", "library/shaping")];

fn library_earlier(name: &str) -> &'static str {
    LIBRARY_EARLIER.iter().find(|(now, _)| *now == name).map(|(_, e)| *e).unwrap_or("")
}

/// The library as `library-seed` takes it: one JSON object per line, grouped
/// by family. `HALE_DNA_LIBRARY_SUFFIX` appends to every text, for fixtures
/// only (a later toolchain whose text changed).
fn library_ideas() -> String {
    let suffix = std::env::var("HALE_DNA_LIBRARY_SUFFIX").unwrap_or_default();
    let mut text = String::new();
    for family in ["language", "design"] {
        for (name, title, target, body, digest) in LIBRARY {
            if library_family(target) != family {
                continue;
            }
            let body = format!("{body}{suffix}");
            // fixtures only: the library as the toolchain that named its
            // moved chapters by their earlier names proposed it
            let (name, earlier) = if std::env::var("HALE_DNA_LIBRARY_EARLIER_NAMES").is_ok_and(|v| !v.is_empty()) && !library_earlier(name).is_empty() { (library_earlier(name), "") } else { (*name, library_earlier(name)) };
            text.push_str(&serde_json::json!({"name": name, "title": title, "text": body, "target": target, "family": family, "digest": digest, "earlier": earlier}).to_string());
            text.push('\n');
        }
    }
    text
}

/// Hand the library to the host: proposals under their families, and with
/// `upgrading` only for a record that has the library, as the new version's.
/// `(ideas proposed, families, ideas superseding, families waiting)`.
fn library_propose(root: &Path, upgrading: bool) -> Result<(usize, usize, usize, usize), String> {
    let dna_dir = root.join(".hale/dna");
    fs::create_dir_all(&dna_dir).map_err(|e| e.to_string())?;
    let ideas = dna_dir.join(format!("library.{}.jsonl", std::process::id()));
    fs::write(&ideas, library_ideas()).map_err(|e| e.to_string())?;
    let mut args = vec![ideas.to_string_lossy().to_string(), library_version()];
    if upgrading {
        args.push("--upgrade".to_string());
    }
    let out = host_run("library-seed", root, &args);
    let _ = fs::remove_file(&ideas);
    let out = out?;
    let field = |key: &str| -> Result<usize, String> {
        out.split_whitespace()
            .find_map(|w| w.strip_prefix(key))
            .and_then(|v| v.parse().ok())
            .ok_or_else(|| format!("library-seed answered oddly: {out}"))
    };
    Ok((field("proposed=")?, field("families=")?, field("superseding=")?, field("waiting=")?))
}

/// seed/pull: the nodes `language:hale` and `system:dna` (and, for an
/// attached application, `application:<account name>` — the record names
/// the application by `application.attached`, never by a graph node, so the
/// node is added here — with a `written_in` edge to the language), then the
/// library as proposals, one Review per family (`library/language@<version>`,
/// `library/design@<version>`).
fn seed_library(root: &Path, app: Option<&App>) -> Result<Vec<String>, String> {
    let dna_dir = root.join(".hale/dna");
    fs::create_dir_all(&dna_dir).map_err(|e| e.to_string())?;
    let mut lines: Vec<(String, String, String)> = Vec::new();
    let node = |kind: &str, name: &str, text: &str| {
        let body = serde_json::json!({"kind": kind, "name": name, "text": text}).to_string();
        ("graph.node".to_string(), format!("{kind}:{name}"), body)
    };
    lines.push(node("language", "hale", &format!("Hale {TOOLCHAIN}: the language this toolchain compiles")));
    lines.push(node("system", "dna", &format!("DNA {TOOLCHAIN}: the design of a software organization as a record, a graph and a Board")));
    let mut nodes = vec!["language:hale".to_string(), "system:dna".to_string()];
    if let Some(app) = app {
        let name = app_account_name(&app.project);
        let id = format!("application:{name}");
        lines.push(node("application", &name, &format!("the attached application, {}", app.main_name)));
        let edge = serde_json::json!({"kind": "written_in", "members": [{"role": "application", "node": id}, {"role": "language", "node": "language:hale"}]}).to_string();
        lines.push(("graph.edge".to_string(), format!("written_in:{id}|language:hale"), edge));
        nodes.push(id);
    }
    let mut text = String::new();
    for (kind, entity, body) in &lines {
        text.push_str(&serde_json::json!({"kind": kind, "entity": entity, "body": body}).to_string());
        text.push('\n');
    }
    let rows = dna_dir.join(format!("library-rows.{}.jsonl", std::process::id()));
    fs::write(&rows, text).map_err(|e| e.to_string())?;
    let seeded = host_run("record-seed", root, &[rows.to_string_lossy().to_string()]);
    let _ = fs::remove_file(&rows);
    let n: usize = seeded?.trim().parse().map_err(|e| format!("record-seed answered oddly: {e}"))?;
    if n != lines.len() {
        return Err(format!("record-seed appended {n} of {} library rows", lines.len()));
    }
    let (proposed, families, _, _) = library_propose(root, false)?;
    if proposed != LIBRARY.len() {
        return Err(format!("library-seed proposed {proposed} of {} ideas", LIBRARY.len()));
    }
    Ok(vec![
        format!("seeded  graph ({}; the nodes knowledge is about)", nodes.join(", ")),
        if app.is_some() { "seeded  edge (written_in: the application is written in language:hale)".to_string() } else { "skipped edge (no attached application)".to_string() },
        format!("seeded  library ({proposed} idea(s) bound to their nodes, proposed as {families} famil(ies), one Board Review each: `hale dna review` lists them under `library`)"),
    ])
}

/// Every seeded family, by the name the Board lists it under.
const SEEDED: &[(&str, &[SeededPractice])] = &[("design", DESIGN), ("operating", OPERATING), ("using", USING)];

/// A practice's text as this toolchain states it. `HALE_DNA_DESIGN_SUFFIX`
/// appends to every practice of every family, for fixtures only: it is how a test makes
/// "a later toolchain whose text changed" out of the one binary it has,
/// so that `upgrade`'s supersession is exercised against real record
/// history rather than described.
fn design_text(p: &SeededPractice) -> String {
    match std::env::var("HALE_DNA_DESIGN_SUFFIX") {
        Ok(s) if !s.is_empty() => format!("{}{s}", p.text),
        _ => p.text.to_string(),
    }
}

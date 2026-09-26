//! `hale dna` — the DNA command surface (GH #528, Track C of #521).
//!
//! Phase 1: the DNA core bundled with the toolchain and materialized
//! into a project, the assembly generated as ordinary Hale source,
//! and the Journal seeded from the compiler's own model of the
//! application. Nothing here is a configuration language: every
//! "setting" is a constructor argument in a `.hl` file the project
//! owns, and every generated fact carries its provenance.
//!
//!   hale dna init [app-dir]     attach the DNA to an existing app
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
    let cache = hale_iris::materialize().map_err(|e| format!("cannot materialize the toolchain cache: {e}"))?;
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
     against what is running here (GH #566 F5).\n"
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
    // GH #583 K4: the node listens for its instances' concerns on a
    // socket of its own, an env-configured route. The host program binds
    // `dna.concern.raised` to the nerves for the relay it publishes as a
    // node under `run`/`dev` (GH #986); this listen route stands beside
    // that binding, and it is the one LOTUS_BUS_CONFIG DNA still writes,
    // until #987 has the application publish onto the nerves itself
    let clone = repo.canonicalize().unwrap_or(repo.clone());
    let node_dir = clone.join(".hale/node");
    if let Err(e) = fs::create_dir_all(&node_dir) {
        eprintln!("hale node: cannot create {}: {e}", node_dir.display());
        return ExitCode::from(1);
    }
    let sock = node_dir.join("concern.raised.sock");
    let _ = fs::remove_file(&sock);
    let conf = node_dir.join("node.bus.conf");
    // relative, and the host execs with the clone as its working
    // directory: a Unix address holds 108 bytes of path, and an ordinary
    // project path spends most of them (the shakeout's finding 6)
    if let Err(e) = fs::write(&conf, "dna.concern.raised = unix://.hale/node/concern.raised.sock : listen\n") {
        eprintln!("hale node: cannot write {}: {e}", conf.display());
        return ExitCode::from(1);
    }
    host_exec_env(
        "node",
        &repo,
        &rest,
        &[("LOTUS_BUS_CONFIG", conf.as_os_str())],
        &[],
    )
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
    match args.first().map(String::as_str) {
        Some("init") => {
            let dir = args.get(1).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
            report(init(&dir))
        }
        Some("new") => match args.get(1) {
            Some(name) if !name.starts_with("--") => {
                // GH #617: `--profile local|remote-body [--remote <url>] [--body <user@host>]`
                // sets the pieces the combination is detected from; nothing is stored as a label
                let flag = |n: &str| args[2..].windows(2).find(|w| w[0] == n).map(|w| w[1].clone());
                report(new_project(Path::new(name), flag("--profile").as_deref(), flag("--remote").as_deref(), flag("--body").as_deref()))
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
                Ok(NervesPlan::Roles(roles)) => roles.into_iter().filter(|(k, _)| k == "HALE_DNA_NATS_ORG" || k == "HALE_DNA_NATS_URL_SPINE").collect(),
                Ok(NervesPlan::NoServer(why)) => {
                    eprintln!("hale dna dev: {why}");
                    Vec::new()
                }
                Err(e) => {
                    eprintln!("hale dna dev: nerves: {e}");
                    return ExitCode::from(2);
                }
            };
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
        Some("route") => host_exec("route", Path::new("."), &args[1..]),
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
        .map(|d| crate::find_workspace_root_pub(&d).unwrap_or(d))
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
    eprintln!("usage: hale dna init [app-dir]      attach the DNA to an existing application");
    eprintln!("       hale dna new <name>          a greenfield application with its DNA");
    eprintln!("       hale dna upgrade [dir]       re-materialize vendor/dna for this toolchain");
    eprintln!("       hale dna memory migrate [dir]");
    eprintln!("                                    apply memory's schema with the owner's DSN (HALE_DNA_MEMORY_DSN_OWNER, or dna/compose.yaml)");
    eprintln!("                                    and print the record's spine and head DSNs (HALE_DNA_MEMORY_DSN_SPINE, …_HEAD)");
    eprintln!("       hale dna nerves migrate [dir]");
    eprintln!("                                    create the organization's NATS JetStream stream with the owner's URL (HALE_DNA_NATS_URL_OWNER,");
    eprintln!("                                    or dna/compose.yaml) and print its token and each role's URL (HALE_DNA_NATS_ORG, …_URL_SPINE)");
    eprintln!("       hale dna nerves drop [dir]   delete the organization's stream, and everything it held, with the owner's URL");
    eprintln!("       hale dna --embedded-digest [--from-tree <dir>]");
    eprintln!("                                    the digest of the DNA source this binary embeds (nothing else on stdout);");
    eprintln!("                                    with a checkout, what that tree would embed — a mismatch means the binary");
    eprintln!("                                    predates the working tree and a mutation run against it proves nothing");
    eprintln!("       hale dna models [project]    the catalog (dna/org/models.hl): every backend, and one small request to each");
    eprintln!("       hale dna work <verb> …       a leg's verbs against the head's API (--api, --as position:<name>): next, brief,");
    eprintln!("                                    renew, submit, settle, release, friction, run — the project's performers (dna/org/work.hl);");
    eprintln!("                                    loop --parallel N is a worker: N children, each its own holder; loop --drain ends one");
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
    eprintln!("                                    the schedules the org chart declared (an ask on an interval or a cron), as the");
    eprintln!("                                    record has them; pause and resume are rows in your name (--as <who>)");
    eprintln!("       hale dna secret set <NAME> [--body <user@host>]");
    eprintln!("                                    a credential from stdin (never argv, never the record) into ~/.config/hale-dna/<project>.env");
    eprintln!("                                    on the body or here; `secret rotate <NAME>`; the record gets `secret.rotated <NAME>` only");
    eprintln!("       hale dna board [project]     the Board's queue: what needs its verdict, escalations, proposals, reports");
    eprintln!("       hale dna task create [--to <locus>] [--as <who>] [--judgment] [--no-wait] <outcome…>");
    eprintln!("                                    ask for an outcome; --judgment asks for an assessment, a leg's to perform");
    eprintln!("                                    ask for an outcome: a row in the record, which a node relays to the organism; prints the Task born or the refusal");
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
    eprintln!("                                    subjects mapped by dna.oidc.member, the secret in HALE_DNA_OIDC_SECRET");
    eprintln!("                                    the DNA surface in a browser, from the record alone: the Board's queue, the Reviews");
    eprintln!("                                    with their three views, the fleet, the history; verdicts, intent and pressure from forms");
    eprintln!("       hale dna review              the pending Reviews");
    eprintln!("       hale dna review <id> [--iris] render a Review: source diff, semantic diff, evidence (works offline)");
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
    let root = crate::find_workspace_root_pub(&seed).unwrap_or_else(|| seed.clone());
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

/// The seed's main locus — its file and name — parsing every .hl in it;
/// none when no file declares one.
fn main_of(seed: &Path) -> Result<Option<(PathBuf, String)>, String> {
    let mut found = None;
    let mut entries: Vec<PathBuf> = fs::read_dir(seed)
        .map_err(|e| format!("{}: {e}", seed.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("hl"))
        .collect();
    entries.sort();
    for p in &entries {
        let src = fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
        if let Ok(prog) = hale_syntax::parse_source(&src) {
            for item in &prog.items {
                if let hale_syntax::ast::TopDecl::Locus(l) = item {
                    if l.is_main {
                        found = Some((p.clone(), l.name.name.clone()));
                    }
                }
            }
        }
    }
    Ok(found)
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
    match crate::find_workspace_root_pub(&at) {
        Some(root) if root != at => Ok(None),
        _ => Ok(Some(at)),
    }
}

fn init(app_dir: &Path) -> Result<Vec<String>, String> {
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
    let purpose_digest = format!("sha256:{}", hex(&openssl::sha::sha256(purpose_text.as_bytes())));
    let org_dir = root.join(ORG_SEED);
    created(&mut out, &org_dir.join("purpose.hl"), &purpose_hl(&purpose_text))?;
    created(&mut out, &org_dir.join("charter.hl"), &charter_hl(&project))?;
    created(&mut out, &org_dir.join("law.hl"), &org_law_hl())?;
    created(&mut out, &org_dir.join("owners"), owners_text())?;
    // GH #583 M1: the catalog, from what this machine has
    let found = discover();
    if created(&mut out, &org_dir.join("models.hl"), &models_hl(&found))? {
        for line in found.report() {
            out.push(format!("models  {line}"));
        }
    }
    // GH #946: the performers a leg of this project runs, like the catalog
    created(&mut out, &org_dir.join("work.hl"), &work_hl(&found))?;
    created(&mut out, &org_dir.join("main.hl"), &org_hl(&project, &purpose_digest, app.as_ref().map(|a| a.seed_rel.as_str())))?;
    // GH #583 K1: dev's environment is compose — the knowledge graph's
    // Postgres, a named volume per repository
    if created(&mut out, &root.join("dna/compose.yaml"), &compose_yaml(&project))? {
        out.push("memory  dna/compose.yaml: `hale dna dev` brings its Postgres and NATS up, applies memory's schema and creates the nerves' stream (docker compose on PATH); `hale dna run` needs HALE_DNA_MEMORY_DSN_SPINE, HALE_DNA_NATS_URL_SPINE and HALE_DNA_NATS_ORG".to_string());
    }
    created(&mut out, &root.join("dna/nats.conf"), &nats_conf())?;
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
            (Some(app), Some((art, raw))) => {
                let n = seed_journal(&app.root, app, art, raw, &purpose_digest)?;
                out.push(format!("seeded  {RECORD_REF} ({n} event(s): application.attached, structure.observed, responsibility.proposed, review.requested)"));
                out.push(seat_initializer(&app.root)?);
            }
            _ => {
                // the purpose's Review and the graph in one checked seed: a
                // repository the ingest refuses leaves no record behind
                let graph = seed_repository(&root, &purpose_digest)?;
                out.push(format!("seeded  {RECORD_REF} (review.requested, then the graph)"));
                out.push(format!("graph   {} (graph.node, graph.edge)", graph.trim()));
                out.push(seat_initializer(&root)?);
            }
        }
        // GH #596 C, #994: the design and the operating practices, as
        // proposals — one Review per practice, listed by family
        for (family, practices) in SEEDED {
            let (d, _, _) = design_upgrade(&root, practices)?;
            out.push(format!("seeded  {family} ({d} practice(s) proposed, one Board Review each: `hale dna review` lists them under `{family}`)"));
        }
    }
    // 8. .gitignore hygiene
    let gi = root.join(".gitignore");
    let mut gtext = fs::read_to_string(&gi).unwrap_or_default();
    let mut added = Vec::new();
    for line in ["/vendor/", "/.hale/"] {
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
    out.push("    the first Review (`purpose`) ratifies dna/org/purpose.hl: `hale dna review purpose approve --as <you>`".to_string());
    Ok(out)
}

fn upgrade(dir: &Path) -> Result<Vec<String>, String> {
    let start = dir.canonicalize().map_err(|e| format!("{}: {e}", dir.display()))?;
    let root = crate::find_workspace_root_pub(&start).unwrap_or(start);
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
    // GH #946, #1104 piece 5: a record from before the seat: its maker's
    // uid mapped to them, so the head's socket knows the peer
    if record_exists(&root)? {
        out.push(seat_initializer(&root)?);
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
    // GH #664: an organization from before the owners map gets one, empty
    let owners = org_dir.join("owners");
    if org_dir.join("main.hl").is_file() && !owners.is_file() {
        fs::write(&owners, owners_text()).map_err(|e| format!("write {}: {e}", owners.display()))?;
        out.push(format!("created {}", owners.display()));
    }
    // GH #986: the nerves — a NATS server beside memory's Postgres, with
    // one user per family of subjects. The config is new and created; the
    // compose file is the project's, and is told
    let nats = root.join("dna/nats.conf");
    if root.join("dna").is_dir() && !nats.is_file() {
        fs::write(&nats, nats_conf()).map_err(|e| format!("write {}: {e}", nats.display()))?;
        out.push(format!("created {}", nats.display()));
    }
    let nats_text = fs::read_to_string(&nats).unwrap_or_default();
    if nats_text.contains("user: spine") && !nats_text.contains("*.head.>") {
        out.push(
            "note    dna/nats.conf does not let the spine publish `<org>.head.>`: a node tells the face's head every row it lands there (GH #986), and a refused publish collapses its connection. Add \"*.head.>\", \"*.*.head.>\" to the spine user's `publish` list, as `hale dna init` writes it"
                .to_string(),
        );
    }
    let compose_text = fs::read_to_string(root.join("dna/compose.yaml")).unwrap_or_default();
    if !compose_text.is_empty() && !compose_text.contains("nerves:") {
        let project = locate(&root).map(|a| a.project).unwrap_or_else(|_| "project".to_string());
        out.push(format!(
            "note    dna/compose.yaml has no `nerves` service (GH #986); add it and its volume, as `hale dna init` writes them:\n  nerves:\n    image: nats:2\n    command: [\"-c\", \"/etc/nats/nats.conf\"]\n    ports:\n      - \"127.0.0.1:{}:4222\"\n    volumes:\n      - ./nats.conf:/etc/nats/nats.conf:ro\n      - nerves:/data\n  (under volumes:)  nerves:\n    name: hale-dna-{}-nerves",
            compose_nats_port(&project),
            compose_name(&project)
        ));
    }
    let main_text = fs::read_to_string(org_dir.join("main.hl")).unwrap_or_default();
    if main_text.contains("main locus Org") && main_text.contains("unix(") {
        out.push(format!(
            "note    {}/main.hl binds its facts to unix sockets; the membrane is gone (GH #986) and they arrive over the nerves. Replace its `bindings` with the ones `hale dna init` writes today: `import \"vendor/dna/pond/realtime/nats\" as nats;`, the `nerves: nats::NatsConn` param, `placement {{ nerves: pinned; }}`, and each fact bound to `nats::NatsAdapter {{ }}`",
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
    if main_text.contains("main locus Org") && !main_text.contains("ownership: dna::Ownership") {
        out.push(format!(
            "note    {}/main.hl names no owners map; a shared record needs one (GH #664): `ownership: dna::Ownership {{ path: \"dna/org/owners\" }}`",
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
fn compose_yaml(project: &str) -> String {
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
# dna/nats.conf. Both listen on 127.0.0.1 only.
services:
  knowledge-db:
    image: pgvector/pgvector:pg16
    environment:
      POSTGRES_USER: dna
      POSTGRES_PASSWORD: dna
      POSTGRES_DB: dna
    ports:
      - "127.0.0.1:{port}:5432"
    volumes:
      - knowledge-db:/var/lib/postgresql/data
  nerves:
    image: nats:2
    command: ["-c", "/etc/nats/nats.conf"]
    ports:
      - "127.0.0.1:{nats_port}:4222"
    volumes:
      - ./nats.conf:/etc/nats/nats.conf:ro
      - nerves:/data
volumes:
  knowledge-db:
    name: hale-dna-{name}-knowledge
  nerves:
    name: hale-dna-{name}-nerves
"#,
        port = compose_port(project),
        nats_port = compose_nats_port(project),
    )
}

/// The project's name as compose's volumes carry it.
fn compose_name(project: &str) -> String {
    project.replace(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_', "-").to_lowercase()
}

/// `dna/nats.conf`: the nerves' server (GH #986). One user per family of
/// subjects, each allowed only its own (`dna/core/nerves.hl` names them).
/// The passwords are placeholders until the vault (#989), as the Postgres
/// roles' are; the organization's token stands as `*` because this server
/// carries one organization.
fn nats_conf() -> String {
    r#"# dna/nats.conf — the nerves' server for `hale dna dev` (GH #986;
# project-owned, generated by `hale dna init`). One user per family of
# subjects, each allowed only its own; `hale dna dev` hands each part of
# the organism its own. The passwords are PLACEHOLDERS until the vault
# (#989) — the port is bound to 127.0.0.1 in dna/compose.yaml.
port: 4222
jetstream { store_dir: "/data" }
authorization {
  users = [
    # creates the organization's stream; held by no process that runs it
    { user: owner, password: "dna-owner-dev" }
    # the spine: a node's host publishes the organization's facts, and the
    # organization reads them through its durable consumer (`spine`, or
    # over a shared record an owner's `spine_<owner>`, its facts under
    # `<org>.<owner>.dna.>`). `*` stands for the organization's token: this
    # server carries one organization, and a spine user may publish into
    # any organization's subjects on it. One user per organization, each
    # allowed only its own token, is #989's, with the vault. A node also
    # tells the heads every row it lands (`<org>.head.row.landed`).
    { user: spine, password: "dna-spine-dev",
      permissions: { publish: ["*.dna.>", "*.*.dna.>", "*.head.>", "*.*.head.>", "$JS.API.CONSUMER.CREATE.*.*", "$JS.API.CONSUMER.INFO.*.*", "$JS.API.CONSUMER.MSG.NEXT.*.*", "$JS.ACK.*.*.>"], subscribe: ["_INBOX.>"] } }
    # an application: publishes on its own subjects, reads nothing of DNA's (#987)
    { user: app, password: "dna-app-dev",
      permissions: { publish: ["*.app.*.>"], subscribe: ["_INBOX.>"] } }
    # the head: subscribes, publishes nothing
    { user: head, password: "dna-head-dev",
      permissions: { publish: { deny: [">"] }, subscribe: ["*.>"] } }
  ]
}
"#
    .to_string()
}

/// A host port for the project's NATS in 42xx, like `compose_port`.
fn compose_nats_port(project: &str) -> u16 {
    let h = project.bytes().fold(5381u32, |h, b| h.wrapping_mul(33) ^ b as u32);
    4200 + (h % 100) as u16
}

/// A host port for the project's Postgres in 54xx, from the project's
/// name, so two governed repositories on one machine do not collide.
fn compose_port(project: &str) -> u16 {
    let h = project.bytes().fold(5381u32, |h, b| h.wrapping_mul(33) ^ b as u32);
    5400 + (h % 100) as u16
}


// ---------------------------------------------------------------
// the catalog (GH #583 M1)
// ---------------------------------------------------------------

/// What this machine has for models: keys in the environment, servers
/// and harnesses on `PATH`. Nothing found is fine — the catalog is
/// still written, every hosted backend simply is not permitted until a
/// key is set, and every Review waits for the Board.
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

fn discover() -> Discovery {
    // `HALE_DNA_DISCOVER=off`: find nothing. For fixtures, so that a
    // developer's machine — a key in the shell, `claude` on PATH — makes
    // the same organization CI makes, one whose leader has no model that
    // answers and whose plans therefore take the defaults. Otherwise a
    // fixture spends real calls and asserts on a live model's judgment.
    if std::env::var("HALE_DNA_DISCOVER").map(|v| v == "off").unwrap_or(false) {
        return Discovery { openai: false, anthropic: false, ollama: None, harnesses: Vec::new() };
    }
    let key = |v: &str| std::env::var(v).map(|s| !s.trim().is_empty()).unwrap_or(false);
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
    env_var: &'static str,
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
                Hosted { adapter: "dna::AnthropicMessages", model: "claude-opus-5", endpoint: "https://api.anthropic.com/v1/messages", env_var: "ANTHROPIC_API_KEY", scheme: "x-api-key", input_micros_per_1k: 15000, output_micros_per_1k: 75000 },
                Hosted { adapter: "dna::AnthropicMessages", model: "claude-haiku-4-5-20251001", endpoint: "https://api.anthropic.com/v1/messages", env_var: "ANTHROPIC_API_KEY", scheme: "x-api-key", input_micros_per_1k: 1000, output_micros_per_1k: 5000 },
                "ANTHROPIC_API_KEY",
            )
        } else {
            (
                Hosted { adapter: "dna::OpenAiChat", model: "gpt-4o", endpoint: "https://api.openai.com/v1/chat/completions", env_var: "OPENAI_API_KEY", scheme: "bearer", input_micros_per_1k: 2500, output_micros_per_1k: 10000 },
                Hosted { adapter: "dna::OpenAiChat", model: "gpt-4o-mini", endpoint: "https://api.openai.com/v1/chat/completions", env_var: "OPENAI_API_KEY", scheme: "bearer", input_micros_per_1k: 150, output_micros_per_1k: 600 },
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
            (true, true) => "ANTHROPIC_API_KEY and OPENAI_API_KEY set (Anthropic chosen)".to_string(),
            (true, false) => "ANTHROPIC_API_KEY set".to_string(),
            (false, true) => "OPENAI_API_KEY set".to_string(),
            (false, false) => "no API key in the environment".to_string(),
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
                if self.anthropic || self.openai { "" } else { ", not set: hosted backends are not permitted until it is" },
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
    let model = if configured {
        "// The model leg: the catalog's agent router behind the performer
// interface, out of process, per task. A rate-limited call is backed
// off inside the attempt and every wait is recorded as evidence.
fn model() -> legs::ModelPerformer {
    return legs::ModelPerformer { router: agent_models() };
}"
    } else {
        "// No backend was configured when this file was generated, so the
// model takes nothing and agent work is a person's. With a backend in
// dna/org/models.hl, put the catalog behind the leg:
//
// fn model() -> legs::ModelPerformer { return legs::ModelPerformer { router: agent_models() }; }
fn model() -> legs::NoModel {
    return legs::NoModel { };
}"
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
// as a leg). `hale check` validates all of it.

import "vendor/dna/legs" as legs;

// A deterministic performer, when this project has one: give it the
// work kinds it takes (`agent`, `service`, `software`, …) and it wins
// for them. `legs::NoDeterministic { }` takes nothing. A person's leg
// is `hale dna work run` (`--performer person` when the model takes
// the kind); a worker is `hale dna work loop --parallel N`.
fn deterministic() -> legs::NoDeterministic {
    return legs::NoDeterministic { };
}

@@MODEL@@

fn performers() -> legs::PerformerCatalog {
    return legs::PerformerCatalog { person: legs::Person { }, deterministic: deterministic(), model: model() };
}

// The hands a performer may use: git in scratch, the forge through
// `gh`, this toolchain; deploy and the heart's API refuse until GH #987
// hands them over.
fn hands() -> legs::Hands {
    return legs::Hands { };
}
"#;
    text.replace("@@MODEL@@", model)
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
            "{} {{ name: \"{name}\", model: \"{}\", endpoint: \"{}\", credential: dna::HostedCredential {{ env_var: \"{}\", scheme: \"{}\" }}, input_micros_per_1k: {}, output_micros_per_1k: {} }}",
            h.adapter, h.model, h.endpoint, h.env_var, h.scheme, h.input_micros_per_1k, h.output_micros_per_1k
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
    let root = crate::find_workspace_root_pub(&start).unwrap_or(start);
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
        let cache = hale_iris::materialize().map_err(|e| format!("cannot materialize the toolchain cache: {e}"))?;
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
// GitHub as a membrane (GH #566 F4): a mirror of the record, never the record
// ---------------------------------------------------------------

// ---------------------------------------------------------------
// the Journal as read by the host: status / task create / history
// ---------------------------------------------------------------

// ---------------------------------------------------------------
// new
// ---------------------------------------------------------------

fn new_project(dir: &Path, profile: Option<&str>, remote: Option<&str>, body: Option<&str>) -> Result<Vec<String>, String> {
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
    let mut rest = init(dir)?;
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

fn org_hl(project: &str, purpose_digest: &str, seed: Option<&str>) -> String {
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
            // Owners (GH #664): who admits which position. The map is the
            // genome's file dna/org/owners — empty while this organization
            // is the only owner; once the record is shared, every position
            // names its owner and this body says which it is (dna.owner).
            ownership: dna::Ownership {{ path: "dna/org/owners" }},
            // The Leader's grant, owned by the Board: what the organization
            // may decide on its own terms. An `application` change is outside
            // this grant and escalates to the Board; widen it here, in a
            // reviewed commit, as the record earns it.
            boundary: dna::AutonomyBoundary {{
                child: "{project}",
                grant: dna::Grant {{ child: "{project}", classes: "refactor docs", max_magnitude: 4, review: "pre" }}
            }},
            review_policy: dna::OrgPolicy {{ }},
            membrane: dna::Board {{ who: "board" }},
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
            // GH #596 L: an ask is planned by the leader before it becomes a Mutation
            planned: true,
            // GH #596 O: the optimize pass — the leader walks the machinery
            // on this cadence, in milliseconds; 0 is never. The Board's to set.
            optimize_every_ms: 0
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
            name: "organization",
            subject_prefix: dna::nerves_subject_prefix(),
            stream: dna::nerves_stream_here(),
            consumer: nats::ConsumerSpec {{ durable: dna::nerves_durable(), filter: dna::nerves_filter() }},
            // what this organization publishes itself (its Leader's
            // verdicts) is acknowledged by the stream, as the node's is
            jetstream: true,
            run_for_ms: if std::env::var_exists("HALE_DNA_ONESHOT") {{ 1 }} else {{ 0 }}
        }};
        // The baseline review: ratify purpose.hl. The Board's; it settles
        // only on a verdict naming this exact digest.
        purpose: dna::Review = dna::Review {{
            review_id: "purpose",
            question: "ratify the declared purpose?",
            subject_digest: "{purpose_digest}",
            required_authority: "board",
            author: "hale dna init"
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
        // GH #596 O: the substrate's cadence — the optimize pass fires
        // every `optimize_every_ms` on the substrate above (0 = never).
        // Queue it with incoming work so a journal refresh completes
        // before a task handler can read or append its cached view. A
        // SIGTERM (the host stopping it) ends it: the runtime drains.
        while !self.draining {{ std::time::sleep(100ms); self.core.request_tick(std::time::monotonic_ns() / 1000000); }}
    }}
}}

fn main() {{
    Org {{ }};
}}
"#
    )
}

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
group positions = { dna::Leader, dna::SourceEditor, dna::WorktreeTools, dna::AgentPerformer, dna::HumanWorkGateway, dna::ServicePerformer, dna::ScriptedPerformer };
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
/// (`dna.unix.member`), and the trust declared. No row in the record —
/// the graph's positions and holders stay the graph's. Says what it did.
fn seat_initializer(root: &Path) -> Result<String, String> {
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
    // the record declares its trust: local, one person holding every
    // authority, unless it declares another already
    let trust = git(root, &["config", "--local", "--get", "dna.trust"]).unwrap_or_default();
    if trust.trim().is_empty() {
        git(root, &["config", "--local", "dna.trust", "local"])?;
    }
    Ok(format!("seated  the head's socket knows uid {uid} as {name} (dna.unix.member); the record declares dna.trust = local, where they hold every position"))
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

fn seed_journal(root: &Path, app: &App, art: &Value, raw: &str, purpose_digest: &str) -> Result<usize, String> {
    let mut c = Chain::new();
    let s = |v: &Value| v.as_str().unwrap_or("").to_string();
    let names = |v: &Value| -> Vec<String> { v.as_array().map(|a| a.iter().map(|x| s(x)).collect()).unwrap_or_default() };
    c.push(
        "application.attached",
        &app.seed_rel,
        &serde_json::json!({
            "main": app.main_name, "artifact": BASELINE_REL,
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
    c.push(
        "review.requested",
        "review:purpose",
        &serde_json::json!({
            "question": "ratify the declared purpose?", "subject_digest": purpose_digest,
            "required_authority": "board", "author": "hale dna init",
            "baseline": {"artifact_digest": s(&art["artifact_digest"]), "bytes": raw.len()},
            "provenance": "declared"
        })
        .to_string(),
    );
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
    let out = host_run("record-seed", root, &[path.to_string_lossy().to_string()]);
    let _ = fs::remove_file(&path);
    let n: usize = out?.trim().parse().map_err(|e| format!("record-seed answered oddly: {e}"))?;
    if n != c.lines.len() {
        return Err(format!("record-seed appended {n} of {} rows", c.lines.len()));
    }
    Ok(n)
}

/// GH #1090: a repository's record — the Review that ratifies the declared
/// purpose, then what the repository holds as the graph — seeded by the
/// host's `graph-ingest` in one call that checks every row before any
/// lands, so an ingest the graph refuses leaves no record and `init` can
/// be run again. What it read, by kind.
fn seed_repository(root: &Path, purpose_digest: &str) -> Result<String, String> {
    let row = serde_json::json!({
        "kind": "review.requested",
        "entity": "review:purpose",
        "body": serde_json::json!({
            "question": "ratify the declared purpose?", "subject_digest": purpose_digest,
            "required_authority": "board", "author": "hale dna init",
            "provenance": "declared"
        })
        .to_string()
    });
    let dna_dir = root.join(".hale/dna");
    fs::create_dir_all(&dna_dir).map_err(|e| e.to_string())?;
    let path = dna_dir.join(format!("seed.{}.jsonl", std::process::id()));
    fs::write(&path, format!("{row}\n")).map_err(|e| e.to_string())?;
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
        text.push_str(&serde_json::json!({"name": p.name, "text": design_text(p)}).to_string());
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

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// ---------------------------------------------------------------
// GH #596: the leader's charter, and the toolchain's design
// ---------------------------------------------------------------

/// `dna/org/charter.hl`: what the leader reads before it thinks. A
/// function returning text, like `purpose`, so a change to the brief
/// is a mutation of the org program the Board reviews.
/// `dna/org/owners`: who admits which position (GH #664). Empty is one
/// owner, the organization itself.
fn owners_text() -> &'static str {
    "# dna/org/owners — who admits which position (GH #664).\n\
# Empty: this organization is the only owner, and every position is its own.\n\
# A shared record names every position's owner and each owner's members:\n\
#   org = acme\n\
#   org/collections = north\n\
#   acme: alice, carol\n\
#   north: bob\n\
# A position not named takes its nearest named ancestor's owner. The body\n\
# says which owner it is: `git config dna.owner <owner>`. Changing this\n\
# file is a change to the organization, approved by every owner it affects.\n"
}

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

/// One seeded practice (a `design/*` or `operating/*` family): a stable
/// name across versions of the toolchain, and the text the Board
/// ratifies or declines.
struct SeededPractice {
    name: &'static str,
    text: &'static str,
}

/// The design: how a DNA organization works, as practices bound to
/// `org` — brained's structural knowledge adapted to DNA. Proposed at
/// `init`, one Board Review each; never ratified by the toolchain.
const DESIGN: &[SeededPractice] = &[
    SeededPractice { name: "design/principles", text: "Minimal structure: add a position only when it serves the whole; complexity is cost. Clean cuts: responsibilities do not overlap, and work that keeps crossing a boundary says the boundary is wrong. Appropriate depth: specialize only when a domain genuinely bifurcates. Team size: three at least (triangulation), seven at most (the ceiling of attention); beyond seven, decompose. Contract invariance: when a part restructures inside, its parent's contract does not change; a position's capabilities are its effect contract, and the compiler holds it." },
    SeededPractice { name: "design/evolution", text: "Start minimal: the Board, the leader, the substrate, one child. Let work reveal where structure is needed. Add operators before supervisors: an operator is cheap, a supervisor adds management. Promote a position to a department only when its domain bifurcates, not before. Re-evaluate on a cadence: structure should match current work, not history." },
    SeededPractice { name: "design/structure-follows-intent", text: "Propose no change to the organism before its purpose (what), its law and grants (how) and its knowledge (the domain's terms and practices) exist. A structure proposed without them produces positions with empty identities, useless to anyone holding them. The sequence is: the Board states purpose and how, the leader proposes structure grounded in both, the Board reviews, the substrate materializes." },
    SeededPractice { name: "design/standard-equipment", text: "Every part that supervises others is born with its architect: the position that holds the design for its path, proposes the rest of its team, and never decides. The architect's first proposal is usually the expert for its domain; after that, researchers, planners and deliverers as the work requires. At the root, the leader is the organism's architect." },
    SeededPractice { name: "design/signals", text: "Read the record for structural signals. Asks that fall through to the leader with no route: routing is incomplete or a position is missing. A position with no work over a window: possibly unnecessary. Concerns accumulating at a child: that subtree is under strain and may need capacity or a different cut. Changes that cross between siblings: their shared parent is missing logic, or the boundary is wrong. A grant that keeps contracting: the work under it is failing and needs a different shape, not a wider leash. Each is an input to a proposal, or to saying the state is clean." },
    SeededPractice { name: "design/signaling", text: "Goals flow down: authored above, bound below, they say what the whole wants of the part. Concerns flow up: authored below, bound above, they say what the part cannot solve alone. Initiatives bridge: self-authored, they turn a goal and its concerns into work. The direction is the classification; nothing else labels them. Three concerns from one source become a proposal by that source; a concern that persists across cycles is being ignored." },
    SeededPractice { name: "design/optimize", text: "On a cadence the Board sets, walk the machinery, not the work: are the change classes right, are Reviews going to the right authority, is the routing catching what it should, does the topology still fit, is the knowledge still true. Propose one small change with its reasoning, or record that the state is clean. Never propose a large restructure unprompted, and never create work for the sake of activity." },
    SeededPractice { name: "design/software-delivery", text: "For an appendage or a product: process boundaries first (what runs, fails and scales independently), then the shapes and verbs that flow between them. Deliver vertical slices that can be demonstrated, never horizontal layers that cannot. Know a change's kind before starting, aesthetic, functional or structural, and update in dependency order. The specification is the source of truth; changes flow from it. The primary test surface is an integration harness through the real system, with the model as the only injected dependency; unit tests sparingly, for pure logic." },
];

/// The operating practices (GH #994): how the organism runs, which is what
/// a leader plans within and a reviewer cites — the design says how an
/// organization is shaped. Seeded beside the design, one Board Review
/// each, superseded on `upgrade` the same way; never ratified by the
/// toolchain.
const OPERATING: &[SeededPractice] = &[
    SeededPractice { name: "operating/one-store-per-step", text: "a workflow step writes to exactly one store, by that store's one writer, and the next step reads what the previous one made durable. The record numbers the steps. A step whose subject moved is refused and asked again on the new subject; nothing is half-written. There is no distributed transaction anywhere, and a plan that needs one is wrong." },
    SeededPractice { name: "operating/row-first", text: "every live signal is a record row before it is sent. An event names a row by id, is delivered at least once, and is consumed idempotently by that id. A message never admits anything. An event from outside (the heart) is a signal, not a fact: it becomes a row before anything acts on it." },
    SeededPractice { name: "operating/readings-never-act", text: "a reading from the senses never acts. It is kept for a window; what matters crosses into the record as a pressure or concern row, and that row is what a workflow answers to. The organism never believes the heart is healthy without the heart's own pulse." },
    SeededPractice { name: "operating/legs-hold-nothing", text: "a leg holds nothing between tasks. The hat is read per task, the credential fetched per task, the result settled per task. A leg that remembers is a bug, and a leg that cannot settle within its lease is a violation its owner records." },
    SeededPractice { name: "operating/deploy-settles-on-pulse", text: "a deploy is settled on the heart's own first event, or rolled back. A rollback restores the source revision, never the work already done in the world, and is a new step in the record." },
    SeededPractice { name: "operating/the-forge-decides", text: "what merges is decided at the forge, by people, and comes back as a verdict row once. The forge is truth for humans; the record is truth for the organism." },
];

/// Every seeded family, by the name the Board lists it under.
const SEEDED: &[(&str, &[SeededPractice])] = &[("design", DESIGN), ("operating", OPERATING)];

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

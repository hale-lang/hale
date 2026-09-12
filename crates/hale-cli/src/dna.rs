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
//!   hale dna run [project]      build, run under LOTUS_OBS with iris attached, hold the membrane
//!   hale dna status [--json]    the status projection, from the Journal
//!   hale dna ask <intent…>      offer intent over the membrane
//!   hale dna history [<entity>] walk the Journal by causal links
//!   hale dna review <id> <verdict> a verdict over the membrane (the Review decides)
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
//!                              and the membrane bindings
//!
//! Toolchain effects are subprocesses: the artifact is cut by
//! `hale check --dump-topology` exactly as a user would, never by a
//! second in-process pipeline.

use std::collections::BTreeSet;
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
const VERDICT_SOCK_REL: &str = ".hale/dna/hale-dna.review.verdict.sock";
const INTENT_SOCK_REL: &str = ".hale/dna/hale-dna.intent.offered.sock";
const OBSERVED_SOCK_REL: &str = ".hale/dna/hale-dna.expression.observed.sock";
const PRESSURE_SOCK_REL: &str = ".hale/dna/hale-dna.pressure.raised.sock";
const CONCERN_SOCK_REL: &str = ".hale/dna/hale-dna.concern.raised.sock";

/// GH #566 F8: the host is a Hale program (`dna/host`, embedded beside
/// the core and built once into the toolchain cache). A verb that is
/// DNA behaviour — a projection, a relay, supervision — execs it with
/// the project resolved: `host <verb> <root> <seed> <fleet> <plan> …`,
/// with the toolchain, the membrane client and the toolchain version
/// in the environment. This shim resolves the project (the manifest is
/// the compiler's) and forwards the exit status.
fn host_exec(verb: &str, dir: &Path, args: &[String]) -> ExitCode {
    let run = || -> Result<i32, String> {
        let (root, seed) = project(dir)?;
        let seed_rel = seed.strip_prefix(&root).ok().map(|p| p.to_string_lossy().to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| ".".into());
        let (fleet, plan_rel) = match crate::pkg::read_dna_fleet(&root.join("hale.toml"))? {
            Some((name, path)) => (name, path.strip_prefix(&root).unwrap_or(&path).to_string_lossy().to_string()),
            None => (String::new(), String::new()),
        };
        let cache = hale_iris::materialize().map_err(|e| format!("cannot materialize the toolchain cache: {e}"))?;
        let host = crate::iris::ensure_built_in(&cache, hale_dna::HOST_SEED, hale_dna::HOST_BIN, "the host")?;
        let membrane = crate::iris::ensure_built_in(&cache, hale_dna::MEMBRANE_SEED, hale_dna::MEMBRANE_BIN, "the membrane client")?;
        // GH #583 K1: `dev` runs the knowledge service beside the
        // organization; the binary is built here, once, like the others
        let knowledge = if verb == "dev" {
            crate::iris::ensure_built_in(&cache, hale_dna::KNOWLEDGE_SEED, hale_dna::KNOWLEDGE_BIN, "the knowledge service")?
        } else {
            PathBuf::new()
        };
        let me = std::env::current_exe().map_err(|e| e.to_string())?;
        // exec in place: the pid that ran `hale dna <verb>` IS the host,
        // so a signal to it — a supervisor's, a test's — reaches the host
        // rather than an orphaned child that keeps ticking (dozens of
        // `host node` processes survived their tests before this)
        use std::os::unix::process::CommandExt;
        let e = Command::new(&host)
            .arg(verb)
            .arg(&root)
            .arg(&seed_rel)
            .arg(&fleet)
            .arg(&plan_rel)
            .args(args)
            .current_dir(&root)
            .env("HALE_BIN", &me)
            .env("HALE_DNA_MEMBRANE", &membrane)
            .env("HALE_DNA_TOOLCHAIN", TOOLCHAIN)
            .env("HALE_DNA_KNOWLEDGE_BIN", &knowledge)
            .exec();
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
        eprintln!("usage: hale node <name> [--repo <clone>] [--fleet <name>] [--tick <ms>]");
        return ExitCode::from(2);
    }
    host_exec("node", &repo, &rest)
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
            Some(name) => report(new_project(Path::new(name))),
            None => usage(2),
        },
        Some("upgrade") => {
            let dir = args.get(1).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
            report(upgrade(&dir))
        }
        Some("run") => {
            let (dir, rest) = project_arg(&args[1..], true);
            host_exec("run", &dir, &rest)
        }
        Some("dev") => {
            let (dir, rest) = project_arg(&args[1..], true);
            host_exec("dev", &dir, &rest)
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
            host_exec("status", &dir, &rest)
        }
        Some("ask") => host_exec("ask", Path::new("."), &args[1..]),
        Some("history") => {
            let (dir, rest) = project_arg(&args[1..], false);
            host_exec("history", &dir, &rest)
        }
        Some("sync") => {
            let (dir, rest) = project_arg(&args[1..], true);
            host_exec("sync", &dir, &rest)
        }
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
        Some("knowledge") => knowledge_cmd(&args[1..]),
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

fn usage(code: u8) -> ExitCode {
    eprintln!("usage: hale dna init [app-dir]      attach the DNA to an existing application");
    eprintln!("       hale dna new <name>          a greenfield application with its DNA");
    eprintln!("       hale dna upgrade [dir]       re-materialize vendor/dna for this toolchain");
    eprintln!("       hale dna models [project]    the catalog (dna/org/models.hl): every backend, and one small request to each");
    eprintln!("       hale dna knowledge [project] [--port N]");
    eprintln!("                                    the knowledge service in the foreground: the record's ratified knowledge applied into");
    eprintln!("                                    HALE_DNA_KNOWLEDGE_DSN (postgres://…, or `memory`), context packages over HTTP");
    eprintln!("       hale dna run [project] [--port N] [--no-iris]");
    eprintln!("                                    build and run the organization (dna/org) with iris attached; hold the membrane");
    eprintln!("       hale dna dev [project] [--port N] [--no-iris] [--observe <secs>]");
    eprintln!("                                    the organization AND the application under one host: rebuild and restart");
    eprintln!("                                    the application on an apply, watch the window, report back");
    eprintln!("       hale dna status [project] [--json]");
    eprintln!("                                    the organism's status projection, from the Journal");
    eprintln!("       hale dna ask [--to <locus>] <intent…>");
    eprintln!("                                    offer intent over the membrane; prints the Task born or the refusal");
    eprintln!("       hale dna history [<entity>]  walk the Journal by causal links (works offline)");
    eprintln!("       hale dna sync [project]      fetch, reconcile and push the record (refs/dna/*) with origin");
    eprintln!("       hale dna board [project]     the Board's queue: what needs its verdict, escalations, proposals, reports");
    eprintln!("       hale dna report [project]    file a report from the record since the last one (report.filed)");
    eprintln!("       hale dna github sync         mirror pending Reviews to pull requests and read their reviews back as verdicts");
    eprintln!("                                    (git config dna.github owner/repo; dna.github.board logins,…; needs `gh`)");
    eprintln!("       hale dna fleet [project]     what the fleet expresses: every instance, its node, revision, model hash, state");
    eprintln!("       hale dna deploy <revision>   express a genome revision through the fleet's nodes (fleet.deploy)");
    eprintln!("       hale dna rollback <mutation> express the base a Mutation was applied on, again");
    eprintln!("                                    (`[dna] fleet = \"<name>\"` in hale.toml names the plan; `hale node <name>` runs a node)");
    eprintln!("       hale dna pressure [raise <source> <what…>]");
    eprintln!("                                    pressure raised and answered; `raise` publishes one signal on the membrane");
    eprintln!("       hale dna concern raise <source> <what…> [--severity N]");
    eprintln!("                                    a concern from a locus path about the part above it; persistent ones become knowledge proposals");
    eprintln!("       hale dna ui [project] [--port N]");
    eprintln!("                                    the DNA surface in a browser, from the record alone: the Board's queue, the Reviews");
    eprintln!("                                    with their three views, the fleet, the history; verdicts, intent and pressure from forms");
    eprintln!("       hale dna review              the pending Reviews");
    eprintln!("       hale dna review <id> [--iris] render a Review: source diff, semantic diff, evidence (works offline)");
    eprintln!("       hale dna review <id> approve|revise|reject|abstain [--as <reviewer>] [--authority <a>] [--comment <c>] [--digest <sha>] [--no-wait]");
    eprintln!("                                    send a verdict over the membrane; the Review decides");
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
    let readme = dir.join("README.md");
    if !readme.exists() {
        let _ = fs::write(
            &readme,
            format!(
                "# vendor/dna — toolchain-owned\n\nThe DNA core (`dna/core` of the hale repository) as shipped by hale {TOOLCHAIN}.\nRegenerated by `hale dna init` / `hale dna upgrade`; pinned in `hale.lock` as\n`[dna] toolchain`. Do not edit: project-owned source lives in `dna/`.\n"
            ),
        );
    }
    pin_lock(root)?;
    Ok((written, same))
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
    // The main locus: parse every .hl in the seed.
    let mut main_file = None;
    let mut main_name = None;
    let mut entries: Vec<PathBuf> = fs::read_dir(&seed)
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
                        main_file = Some(p.clone());
                        main_name = Some(l.name.name.clone());
                    }
                }
            }
        }
    }
    let (Some(main_file), Some(main_name)) = (main_file, main_name) else {
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

fn init(app_dir: &Path) -> Result<Vec<String>, String> {
    let app = locate(app_dir)?;
    let mut out: Vec<String> = Vec::new();
    // 0. the record is a branch, so the project is a repository
    if ensure_repo(&app.root)? {
        out.push(format!("git init {} (the record lives on {RECORD_REF})", app.root.display()));
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
    let manifest = app.root.join("hale.toml");
    if !manifest.exists() {
        created(&mut out, &manifest, "[deps]\n")?;
    }
    // 2. the toolchain-owned core
    let (w, same) = materialize_vendor(&app.root)?;
    out.push(format!(
        "{} vendor/dna ({} file(s) written, {} unchanged; hale.lock pins toolchain {})",
        if w > 0 { "wrote  " } else { "kept   " },
        w,
        same,
        TOOLCHAIN
    ));
    // 3. the artifact, cut by the toolchain as a subprocess
    let baseline = app.root.join(BASELINE_REL);
    fs::create_dir_all(baseline.parent().unwrap()).map_err(|e| e.to_string())?;
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
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
    // 4. the organization: a program of its own (GH #566 F2). The
    //    application is not touched — it carries its own law and is
    //    observable like any Hale binary; the org oversees it from outside.
    let purpose_text = format!("{}: keep the application correct, reviewable and explainable; every change is staged, reviewed, and never applied by the organism itself.", app.project);
    let purpose_digest = format!("sha256:{}", hex(&openssl::sha::sha256(purpose_text.as_bytes())));
    let org_dir = app.root.join(ORG_SEED);
    created(&mut out, &org_dir.join("purpose.hl"), &purpose_hl(&purpose_text))?;
    created(&mut out, &org_dir.join("law.hl"), &org_law_hl())?;
    // GH #583 M1: the catalog, from what this machine has
    let found = discover();
    if created(&mut out, &org_dir.join("models.hl"), &models_hl(&found))? {
        for line in found.report() {
            out.push(format!("models  {line}"));
        }
    }
    created(&mut out, &org_dir.join("main.hl"), &org_hl(&app.project, &purpose_digest, &app.seed_rel))?;
    // GH #583 K1: dev's environment is compose — the knowledge graph's
    // Postgres, a named volume per repository
    if created(&mut out, &app.root.join("dna/compose.yaml"), &compose_yaml(&app.project))? {
        out.push("knowledge dna/compose.yaml: `hale dna dev` brings its Postgres up and runs the knowledge service against it (docker compose on PATH); `hale dna run` needs HALE_DNA_KNOWLEDGE_DSN".to_string());
    }
    out.push(format!("kept    {} (the application is not modified; the organization oversees it from {})", app.main_file.display(), ORG_SEED));
    // 5. the manifest's environments: the application's, and the organization's
    let mtext = fs::read_to_string(&manifest).map_err(|e| e.to_string())?;
    if !mtext.contains("[environments.") {
        let add = format!(
            "\n# hale dna init: the two entrypoints and where each deploys (`hale check --matrix`).\n# The organization adopts its law (dna/org/law.hl) itself; the application keeps its own.\n[claims]\nno_base = true\n\n[environments.local]\nsource_only = true\nentrypoints = [\"{}\"]\n\n[environments.org]\nsource_only = true\nentrypoints = [\"{}\"]\n",
            app.seed_rel, ORG_SEED
        );
        fs::write(&manifest, format!("{}{}", mtext, add)).map_err(|e| e.to_string())?;
        out.push(format!("edited  {} ([claims] no_base, [environments.local], [environments.org])", manifest.display()));
    } else {
        out.push(format!("kept    {} (declares environments already)", manifest.display()));
    }
    // 7. the record, seeded from the artifact
    if record_head(&app.root).is_some() {
        out.push(format!("kept    {RECORD_REF} (a record exists; not reseeded)"));
    } else {
        let n = seed_journal(&app.root, &app, &art, &raw, &purpose_digest)?;
        out.push(format!("seeded  {RECORD_REF} ({n} event(s): application.attached, structure.observed, responsibility.proposed, review.requested)"));
    }
    // 8. .gitignore hygiene
    let gi = app.root.join(".gitignore");
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
    let _ = Command::new(&me)
        .arg("fmt")
        .arg(app.root.join("dna"))
        .arg(&app.seed)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    out.push(String::new());
    out.push("next steps:".to_string());
    out.push(format!("    hale check --matrix {}   # every entrypoint against its law", app.root.display()));
    out.push(format!("    hale dna dev {}          # the organization and the application under one host, iris attached", app.root.display()));
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
        "vendor/dna: {} file(s) rewritten, {} unchanged; hale.lock pins toolchain {}. dna/ untouched.",
        w, same, TOOLCHAIN
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
        if main.contains("dna::HostedModel") || main.contains("dna::ModelRouter {") {
            out.push(format!(
                "note    {}/main.hl wires its routers inline (dna::HostedModel is now dna::OpenAiChat); point each position at the catalog: `models: leader_models()`, `editor_models()`, `agent_models()`, and `budget: dna::Budget {{ policy: org_budget() }}` on the substrate",
                ORG_SEED
            ));
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------
// the knowledge graph's environment (GH #583 K1)
// ---------------------------------------------------------------

/// `dna/compose.yaml`: the knowledge graph's Postgres (with pgvector)
/// for `hale dna dev`, a named volume per repository so the graph
/// outlives the container. Part of the genome, project-owned.
fn compose_yaml(project: &str) -> String {
    let name = project.replace(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_', "-").to_lowercase();
    format!(
        r#"# dna/compose.yaml — the knowledge graph's environment for `hale dna dev`
# (project-owned; generated by `hale dna init`). The knowledge service
# owns the live, shared half of the graph in this Postgres; the record
# (refs/dna/*) holds the decided half. `hale dna dev` runs
# `docker compose -f dna/compose.yaml up -d`, waits for the database,
# and hands the service its DSN. Beyond one machine, and for
# `hale dna run`, point HALE_DNA_KNOWLEDGE_DSN at a Postgres of your own.
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
volumes:
  knowledge-db:
    name: hale-dna-{name}-knowledge
"#,
        port = compose_port(project),
    )
}

/// A host port for the project's Postgres in 54xx, from the project's
/// name, so two governed repositories on one machine do not collide.
fn compose_port(project: &str) -> u16 {
    let h = project.bytes().fold(5381u32, |h, b| h.wrapping_mul(33) ^ b as u32);
    5400 + (h % 100) as u16
}

/// `hale dna knowledge [project] [--port N]`: the knowledge service in
/// the foreground (GH #583 K1), built once into the toolchain cache,
/// against HALE_DNA_KNOWLEDGE_DSN (`memory` for a store that lives as
/// long as the process). `hale dna dev` runs the same binary itself.
fn knowledge_cmd(args: &[String]) -> ExitCode {
    let mut dir = PathBuf::from(".");
    let mut port = "8791".to_string();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--port" => match it.next() {
                Some(p) => port = p.clone(),
                None => {
                    eprintln!("hale dna knowledge: --port needs a value");
                    return ExitCode::from(2);
                }
            },
            f if f.starts_with("--") => {
                eprintln!("hale dna knowledge: unknown flag `{f}`");
                return ExitCode::from(2);
            }
            p => dir = PathBuf::from(p),
        }
    }
    let run = || -> Result<i32, String> {
        let (root, _) = project(&dir)?;
        if std::env::var("HALE_DNA_KNOWLEDGE_DSN").map(|v| v.trim().is_empty()).unwrap_or(true) {
            return Err("HALE_DNA_KNOWLEDGE_DSN is not set: a postgres:// URL (`hale dna dev` derives one from dna/compose.yaml), or `memory` for a store that lives only as long as this process".to_string());
        }
        let cache = hale_iris::materialize().map_err(|e| format!("cannot materialize the toolchain cache: {e}"))?;
        let bin = crate::iris::ensure_built_in(&cache, hale_dna::KNOWLEDGE_SEED, hale_dna::KNOWLEDGE_BIN, "the knowledge service")?;
        let me = std::env::current_exe().map_err(|e| e.to_string())?;
        use std::os::unix::process::CommandExt;
        let e = Command::new(&bin).arg(&root).arg(&port).current_dir(&root).env("HALE_BIN", &me).exec();
        Err(format!("hale dna knowledge: {e}"))
    };
    match run() {
        Ok(code) => ExitCode::from(code.clamp(0, 255) as u8),
        Err(e) => {
            eprintln!("hale dna knowledge: {e}");
            ExitCode::from(1)
        }
    }
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

fn harness() -> dna::HarnessModel {
    return dna::HarnessModel { name: "quick", command: "codex", argv: "exec
--full-auto", output: "text", confinement: dna::Bubblewrap { } };
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

fn record_head(root: &Path) -> Option<String> {
    git(root, &["rev-parse", "-q", "--verify", RECORD_REF]).ok().filter(|s| !s.is_empty())
}

/// Append one event to the record: one commit on `refs/dna/journal`,
/// compare-and-swapped on the ref, so the organism, the host and a CLI in
/// another clone never lose each other's events. The author is git's
/// identity for the user running this (a human's verdict is theirs).
fn append_journal(root: &Path, kind: &str, entity: &str, body: &str) -> Result<(), String> {
    append_journal_as(root, kind, entity, body, None)
}

pub(crate) fn append_journal_as(root: &Path, kind: &str, entity: &str, body: &str, author: Option<&str>) -> Result<(), String> {
    let dna_dir = root.join(".hale/dna");
    fs::create_dir_all(&dna_dir).map_err(|e| e.to_string())?;
    let pid = std::process::id();
    let tmp = dna_dir.join(format!("journal.host.{pid}.tmp"));
    let idx = dna_dir.join(format!("index.host.{pid}.tmp"));
    let identity: Vec<String> = if git(root, &["config", "user.name"]).map(|n| n.is_empty()).unwrap_or(true) {
        vec!["-c".into(), "user.name=host".into(), "-c".into(), "user.email=host@dna".into()]
    } else {
        Vec::new()
    };
    for _ in 0..3 {
        let head = record_head(root);
        let text = match &head {
            Some(_) => git(root, &["show", &format!("{RECORD_REF}:journal.jsonl")])? + "\n",
            None => String::new(),
        };
        let seq = text.lines().filter(|l| !l.trim().is_empty()).count();
        let who = author.map(|a| a.to_string()).unwrap_or_else(|| git(root, &["config", "user.name"]).ok().filter(|n| !n.is_empty()).unwrap_or_else(|| "host".into()));
        let line = serde_json::json!({"seq": seq, "kind": kind, "entity": entity, "body": body, "author": who}).to_string();
        fs::write(&tmp, format!("{text}{line}\n")).map_err(|e| e.to_string())?;
        let blob = git(root, &["hash-object", "-w", &tmp.to_string_lossy()])?;
        let _ = fs::remove_file(&idx);
        let with_index = |args: &[&str]| -> Result<String, String> {
            let out = Command::new("git").arg("-C").arg(root).env("GIT_INDEX_FILE", &idx).args(args).output().map_err(|e| format!("git: {e}"))?;
            if !out.status.success() {
                return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
            }
            Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
        };
        match &head {
            Some(h) => with_index(&["read-tree", h])?,
            None => with_index(&["read-tree", "--empty"])?,
        };
        with_index(&["update-index", "--add", "--cacheinfo", &format!("100644,{blob},journal.jsonl")])?;
        let tree = with_index(&["write-tree"])?;
        let mut ct: Vec<String> = identity.clone();
        ct.extend(["commit-tree".into(), tree]);
        if let Some(h) = &head {
            ct.extend(["-p".into(), h.clone()]);
        }
        ct.extend(["-m".into(), format!("{kind} {entity}")]);
        let ct_args: Vec<&str> = ct.iter().map(|s| s.as_str()).collect();
        let commit = git(root, &ct_args)?;
        let old = head.clone().unwrap_or_default();
        if git(root, &["update-ref", RECORD_REF, &commit, &old]).is_ok() {
            let _ = fs::remove_file(&tmp);
            let _ = fs::remove_file(&idx);
            return Ok(());
        }
        // the ref moved under us: re-read and re-append at the new tail
    }
    Err("append lost the record's ref race three times".into())
}

// ---------------------------------------------------------------
// the record across clones: sync with origin, and the membrane over it
// ---------------------------------------------------------------

const REMOTE_TRACK: &str = "refs/dna/remote/journal";

/// The remote the record syncs with, when the repository has one.
pub(crate) fn record_remote(root: &Path) -> Option<String> {
    let name = git(root, &["config", "dna.remote"]).ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "origin".into());
    git(root, &["remote", "get-url", &name]).ok().map(|_| name)
}

/// Fetch, reconcile, push (GH #566 F1). The record is linear: when
/// this clone and the remote both appended, the local-only events are
/// re-appended on top of the remote's head (their bodies and authors
/// unchanged; their seq is their new position) and pushed. Receipts
/// travel both ways by refspec. Returns a one-line summary.
pub(crate) fn sync_record(root: &Path) -> Result<String, String> {
    let Some(remote) = record_remote(root) else {
        return Ok("no remote: the record is local".into());
    };
    git(root, &["fetch", "-q", &remote, &format!("+{RECORD_REF}:{REMOTE_TRACK}"), "+refs/dna/receipts/*:refs/dna/receipts/*"]).or_else(|e| {
        // a remote with no record yet is not an error
        if e.contains("couldn't find remote ref") || e.contains("Couldn't find remote ref") { Ok(String::new()) } else { Err(format!("fetch {remote}: {e}")) }
    })?;
    let mut summary = String::new();
    for _ in 0..3 {
        let local = record_head(root);
        let remote_head = git(root, &["rev-parse", "-q", "--verify", REMOTE_TRACK]).ok().filter(|s| !s.is_empty());
        let (Some(l), Some(r)) = (local.clone(), remote_head.clone()) else {
            if local.is_none() && remote_head.is_some() {
                git(root, &["update-ref", RECORD_REF, remote_head.as_deref().unwrap()])?;
                summary = "pulled the record".into();
            }
            break;
        };
        if l == r {
            if summary.is_empty() {
                summary = "up to date".into();
            }
            break;
        }
        if git(root, &["merge-base", "--is-ancestor", &r, &l]).is_ok() {
            // local ahead: push below
            let n = git(root, &["rev-list", "--count", &format!("{r}..{l}")]).unwrap_or_default();
            summary = format!("pushed {n} event(s)");
        } else if git(root, &["merge-base", "--is-ancestor", &l, &r]).is_ok() {
            git(root, &["update-ref", RECORD_REF, &r, &l])?;
            let n = git(root, &["rev-list", "--count", &format!("{l}..{r}")]).unwrap_or_default();
            summary = format!("pulled {n} event(s)");
            break;
        } else {
            // diverged: re-append the local-only events on top of the remote
            let mine = git(root, &["rev-list", "--reverse", &format!("{r}..{l}")])?;
            let mine: Vec<String> = mine.lines().map(|s| s.to_string()).collect();
            let mut events: Vec<(String, String, String, String)> = Vec::new();
            for c in &mine {
                let text = git(root, &["show", &format!("{c}:journal.jsonl")])?;
                let Some(last) = text.lines().filter(|l| !l.trim().is_empty()).last() else { continue };
                let v: Value = serde_json::from_str(last).map_err(|e| e.to_string())?;
                let s = |k: &str| v[k].as_str().unwrap_or("").to_string();
                events.push((s("kind"), s("entity"), s("body"), s("author")));
            }
            git(root, &["update-ref", RECORD_REF, &r, &l])?;
            for (kind, entity, body, author) in &events {
                append_journal_as(root, kind, entity, body, Some(author))?;
            }
            summary = format!("re-appended {} local event(s) onto the remote's {}", events.len(), git(root, &["rev-list", "--count", &format!("{l}..{r}")]).unwrap_or_default());
        }
        match git(root, &["push", "-q", &remote, &format!("{RECORD_REF}:{RECORD_REF}"), "refs/dna/receipts/*:refs/dna/receipts/*"]) {
            Ok(_) => {
                let _ = git(root, &["update-ref", REMOTE_TRACK, &record_head(root).unwrap_or_default()]);
                break;
            }
            Err(_) => {
                // the remote moved again: fetch and go round
                git(root, &["fetch", "-q", &remote, &format!("+{RECORD_REF}:{REMOTE_TRACK}")])?;
            }
        }
    }
    Ok(summary)
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
        let _ = sync_record(&root);
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
// the Journal as read by the host: status / ask / history
// ---------------------------------------------------------------

// ---------------------------------------------------------------
// new
// ---------------------------------------------------------------

fn new_project(dir: &Path) -> Result<Vec<String>, String> {
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
        println("{name}: ", self.echo.seen, " ping(s) echoed; membrane open");
        // The organism stays up for its membrane (`hale dna run`);
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

fn org_hl(project: &str, purpose_digest: &str, seed: &str) -> String {
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
// The application ({seed}) is not part of this program. It carries its
// own law and is observed like any Hale binary; this organization
// proposes changes to it, verifies them, and expresses them.

import "vendor/dna" as dna;

main locus Org {{
    params {{
        core: dna::Dna = dna::Dna {{
            // The record: one commit per event on refs/dna/journal, receipts
            // and leases as refs beside it. Every clone that fetches
            // refs/dna/* has the whole history of what this organization did.
            journal: dna::GitJournal {{ repo: "." }},
            // Models: every position's router comes from the catalog in
            // models.hl (a backend is a constructor function there; a
            // hosted one presents its credential from a sealed locus and
            // is not a permitted backend without it). Every call journals
            // its evidence, never the prompt.
            work: dna::WorkSystem {{
                agent: dna::AgentPerformer {{ name: "agent", models: agent_models() }}
            }},
            // The organization's spend: one policy (models.hl), one owner.
            budget: dna::Budget {{ policy: org_budget() }},
            // The knowledge service, when the host runs one (`hale dna dev`):
            // what the organization ratified for a target is folded into the
            // objective the editor gets; the editor itself never reaches it.
            knowledge_client: dna::KnowledgeClient {{ url_env: "HALE_DNA_KNOWLEDGE_URL" }},
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
            genome_seed: "{seed}"
        }};
        // The Leader: decides the Reviews inside the grant, with the deep
        // tier, reading the source diff and the semantic diff; every
        // decision is a model call with evidence in the record.
        leader: dna::Leader = dna::Leader {{
            name: "leader",
            models: leader_models(),
            receipts: dna::GitReceipts {{ repo: "." }},
            source: dna::SourceReader {{ repo: "." }}
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
    // The membrane: where a verdict, an intent and the host's observation
    // report enter. `hale dna` and iris publish here; the owning loci decide.
    bindings {{
        dna::ReviewVerdict: unix("{verdict}", role: listen);
        dna::IntentOffered: unix("{intent}", role: listen);
        dna::ExpressionObserved: unix("{observed}", role: listen);
        dna::PressureRaised: unix("{pressure}", role: listen);
        dna::ConcernRaised: unix("{concern}", role: listen);
    }}
    run() {{
        if std::env::var_exists("HALE_DNA_ONESHOT") {{ return; }}
        while true {{ std::time::sleep(100ms); }}
    }}
}}

fn main() {{
    Org {{ }};
}}
"#,
        verdict = VERDICT_SOCK_REL,
        intent = INTENT_SOCK_REL,
        observed = OBSERVED_SOCK_REL,
        pressure = PRESSURE_SOCK_REL,
        concern = CONCERN_SOCK_REL,
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
group knowledge = { dna::Knowledge, dna::KnowledgeClient };
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
}
"#
    .to_string()
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
    for (kind, entity, body) in &c.lines {
        append_journal(root, kind, entity, body)?;
    }
    Ok(c.lines.len())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

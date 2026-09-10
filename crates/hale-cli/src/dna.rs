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
        Some("run") => run_organism(&args[1..], false),
        Some("dev") => run_organism(&args[1..], true),
        Some("status") => report(status(&args[1..])),
        Some("ask") => report(ask(&args[1..])),
        Some("history") => report(history(&args[1..])),
        Some("sync") => report(sync_cmd(&args[1..])),
        Some("review") => report(review(&args[1..])),
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
    eprintln!("       hale dna review              the pending Reviews");
    eprintln!("       hale dna review <id> [--iris] render a Review: source diff, semantic diff, evidence (works offline)");
    eprintln!("       hale dna review <id> approve|revise|reject|abstain [--as <reviewer>] [--authority <a>] [--comment <c>] [--digest <sha>]");
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
    created(&mut out, &org_dir.join("main.hl"), &org_hl(&app.project, &purpose_digest, &app.seed_rel))?;
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
    Ok(vec![format!(
        "vendor/dna: {} file(s) rewritten, {} unchanged; hale.lock pins toolchain {}. dna/ untouched.",
        w, same, TOOLCHAIN
    )])
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

/// `hale dna run [project] [--port N] [--no-iris]`.
///
/// A stateless host: it holds no Task state and decides nothing.
/// It cuts a fresh artifact of what is about to run, builds it,
/// execs it with LOTUS_OBS=1 from the project root (the membrane
/// sockets and the Journal are root-relative), waits for the
/// membrane to be bound, launches `hale iris` against it (law view
/// on the fresh artifact, review view diffing it against the
/// baseline `init` cut, membrane attached), and waits for the
/// organism. When the organism exits, iris is reaped and the
/// organism's exit code is ours.
fn run_organism(args: &[String], dev: bool) -> ExitCode {
    let verb = if dev { "dev" } else { "run" };
    let mut dir = PathBuf::from(".");
    let mut port = "8787".to_string();
    let mut iris = true;
    let mut observe_secs: u64 = 15;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--port" => match it.next() {
                Some(p) => port = p.clone(),
                None => {
                    eprintln!("hale dna {verb}: --port needs a value");
                    return ExitCode::from(2);
                }
            },
            "--observe" => match it.next().and_then(|v| v.parse::<u64>().ok()) {
                Some(n) => observe_secs = n,
                None => {
                    eprintln!("hale dna {verb}: --observe needs a number of seconds");
                    return ExitCode::from(2);
                }
            },
            "--no-iris" => iris = false,
            f if f.starts_with("--") => {
                eprintln!("hale dna {verb}: unknown flag `{f}`");
                return ExitCode::from(2);
            }
            p => dir = PathBuf::from(p),
        }
    }
    let (root, seed) = match project(&dir) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("hale dna {verb}: {e}");
            return ExitCode::from(2);
        }
    };
    let org_seed = root.join(ORG_SEED);
    if !org_seed.is_dir() {
        eprintln!("hale dna {verb}: no organization at {} (run `hale dna init`)", org_seed.display());
        return ExitCode::from(2);
    }
    let me = match std::env::current_exe() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("hale dna {verb}: {e}");
            return ExitCode::from(1);
        }
    };
    let dna_dir = root.join(".hale/dna");
    let _ = fs::create_dir_all(&dna_dir);
    let status_path = dna_dir.join("status.json");
    // 1. the expression identity: a fresh artifact of the application
    let current = dna_dir.join("current.topology");
    if let Err(e) = cut_artifact(&me, &seed, &current) {
        eprintln!("hale dna {verb}: {e}");
        return ExitCode::from(1);
    }
    // 2. the organization: built, started from the root, observable
    let org_bin = match build_seed(&me, &org_seed) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("hale dna {verb}: {e}");
            return ExitCode::from(1);
        }
    };
    for sock in [crate::iris::MEMBRANE_VERDICT_SOCK, crate::iris::MEMBRANE_INTENT_SOCK, crate::iris::MEMBRANE_OBSERVED_SOCK] {
        let _ = fs::remove_file(dna_dir.join(sock));
    }
    let mut org = match spawn_process(&org_bin, &root, &me, None) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("hale dna {verb}: {e}");
            return ExitCode::from(1);
        }
    };
    let _ = fs::write(dna_dir.join("org.pid"), org.id().to_string());
    eprintln!("hale dna {verb}: organization (pid {}) from {} under LOTUS_OBS=1", org.id(), root.display());
    // 3. the membrane comes up
    let bound = wait_membrane(&dna_dir, &mut org, 20);
    if bound {
        eprintln!("hale dna {verb}: membrane bound at {}", dna_dir.display());
    } else if org.try_wait().ok().flatten().is_none() {
        eprintln!("hale dna {verb}: the membrane did not come up within 20s; the organization runs unobserved");
    }
    // 4. dev: the application too, under this host, so an apply can be
    //    expressed and observed here
    let app_name = seed.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "app".into());
    let mut app: Option<std::process::Child> = None;
    let mut app_bin: PathBuf = seed.join(&app_name);
    if dev {
        match build_seed(&me, &seed) {
            Ok(b) => app_bin = b,
            Err(e) => {
                eprintln!("hale dna dev: {e}");
                terminate(&mut org);
                return ExitCode::from(1);
            }
        }
        match spawn_process(&app_bin, &root, &me, None) {
            Ok(c) => {
                let _ = fs::write(dna_dir.join("app.pid"), c.id().to_string());
                eprintln!("hale dna dev: expression {} (pid {}) under LOTUS_OBS=1", app_name, c.id());
                app = Some(c);
            }
            Err(e) => {
                eprintln!("hale dna dev: {e}");
                terminate(&mut org);
                return ExitCode::from(1);
            }
        }
    }
    // 5. iris: the law view on the application's artifact, the review
    //    view against the baseline, the organism panel, the membrane form
    write_status(&root, &status_path);
    let mut observer: Option<std::process::Child> = None;
    if iris && bound {
        let baseline = dna_dir.join("baseline.topology");
        observer = launch_iris(&me, &port, &current, if baseline.is_file() { Some(baseline) } else { None }, &dna_dir, &status_path);
    }
    // 6. supervise: the organization's exit is ours. Each tick: sync the
    //    record, relay the membrane facts in it, re-project status, and
    //    answer restart requests — under `dev` by rebuilding and
    //    restarting the application and watching it; under `run` by
    //    saying that no expression is under this host.
    let mut handled: BTreeSet<u64> = BTreeSet::new();
    let mut relayed: BTreeSet<u64> = BTreeSet::new();
    let mut app_gone_reported = false;
    let code = loop {
        match org.try_wait() {
            Ok(Some(st)) => break st.code().unwrap_or(1),
            Ok(None) => {}
            Err(_) => break 1,
        }
        if let Some(a) = app.as_mut() {
            if let Ok(Some(st)) = a.try_wait() {
                if !app_gone_reported {
                    eprintln!("hale dna dev: the expression exited ({}) outside a window; the organization is still up", st.code().unwrap_or(-1));
                    app_gone_reported = true;
                }
            }
        }
        if let Err(e) = sync_record(&root) {
            eprintln!("hale dna {verb}: sync: {e}");
        }
        let n = relay_record_membrane(&root, &mut relayed);
        if n > 0 {
            eprintln!("hale dna {verb}: relayed {n} fact(s) from the record onto the membrane");
            let _ = sync_record(&root);
        }
        write_status(&root, &status_path);
        if let Some((seq, id, body)) = pending_restart(&root, &handled) {
            handled.insert(seq);
            let rollback = body.starts_with("rollback ");
            if !dev {
                eprintln!("hale dna run: {id} requests a restart ({body}); no expression is under this host — `hale dna dev`, or a deployment gateway, expresses it");
                continue;
            }
            eprintln!("hale dna dev: {id} requests a restart ({body})");
            let previous = dna_dir.join("previous.topology");
            let _ = fs::copy(&current, &previous);
            match express(&me, &seed, &current) {
                Err(e) => {
                    eprintln!("hale dna dev: {id}: the candidate does not express: {e}");
                    if !rollback {
                        let body = serde_json::json!({"mutation_id": id, "outcome": "build_failed", "model_hash": "", "detail": e}).to_string();
                        if let Err(e) = publish_on_membrane(&root, "observed", &body) {
                            eprintln!("hale dna dev: could not report on the membrane: {e}");
                        }
                    }
                    let _ = fs::copy(&previous, &current);
                    continue;
                }
                Ok(_) => {}
            }
            let shape = shape_of(&current);
            let expression = format!("{}{} build {}", if rollback { "rollback " } else { "" }, shape, crate::sign::sha256_file(&app_bin).map(|d| d[..12].to_string()).unwrap_or_default());
            if let Some(mut a) = app.take() {
                terminate(&mut a);
            }
            let mut fresh = match spawn_process(&app_bin, &root, &me, Some((&id, &expression))) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("hale dna dev: {e}");
                    break 1;
                }
            };
            let _ = fs::write(dna_dir.join("app.pid"), fresh.id().to_string());
            app_gone_reported = false;
            // the expression carries no organism now: the host records that it restarted
            let _ = append_journal(&root, "expression.restarted", &id, &expression);
            eprintln!("hale dna dev: expression restarted (pid {}) as {}", fresh.id(), expression);
            write_status(&root, &status_path);
            if let Some(mut o) = observer.take() {
                let _ = o.kill();
                let _ = o.wait();
                observer = launch_iris(&me, &port, &current, Some(previous.clone()), &dna_dir, &status_path);
            }
            if rollback {
                app = Some(fresh);
                continue;
            }
            // the observation window: the expression must stay up
            let end = std::time::Instant::now() + std::time::Duration::from_secs(observe_secs);
            let mut crashed: Option<i32> = None;
            while std::time::Instant::now() < end {
                if let Ok(Some(st)) = fresh.try_wait() {
                    crashed = Some(st.code().unwrap_or(-1));
                    break;
                }
                let _ = sync_record(&root);
                write_status(&root, &status_path);
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
            match crashed {
                None => {
                    let body = serde_json::json!({"mutation_id": id, "outcome": "healthy", "model_hash": shape, "detail": format!("up for {observe_secs}s")}).to_string();
                    match publish_on_membrane(&root, "observed", &body) {
                        Ok(()) => eprintln!("hale dna dev: {id} observed healthy for {observe_secs}s as {shape}"),
                        Err(e) => eprintln!("hale dna dev: could not report on the membrane: {e}"),
                    }
                    app = Some(fresh);
                }
                Some(code) => {
                    // the organization is up and decides; the host reports the crash
                    // as an observation and restores the base expression itself
                    eprintln!("hale dna dev: {id}: the expression exited ({code}) inside the observation window");
                    let body = serde_json::json!({"mutation_id": id, "outcome": "crashed", "model_hash": shape, "detail": format!("exited {code} in the observation window")}).to_string();
                    if let Err(e) = publish_on_membrane(&root, "observed", &body) {
                        eprintln!("hale dna dev: could not report on the membrane: {e}");
                    }
                    app = None;
                    // the organization rolls back and requests the old expression;
                    // that request is answered by the next tick like any other
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    };
    if let Some(mut o) = observer {
        let _ = o.kill();
        let _ = o.wait();
    }
    if let Some(mut a) = app {
        terminate(&mut a);
    }
    let _ = fs::remove_file(dna_dir.join("org.pid"));
    let _ = fs::remove_file(dna_dir.join("app.pid"));
    write_status(&root, &status_path);
    eprintln!("hale dna {verb}: organization exited ({code}); {} in {} is the record", RECORD_REF, root.display());
    ExitCode::from(code.clamp(0, 255) as u8)
}

/// Cut the artifact of what is about to run.
fn cut_artifact(me: &Path, seed: &Path, current: &Path) -> Result<(), String> {
    let st = Command::new(me)
        .arg("check")
        .arg(seed)
        .arg(format!("--dump-topology={}", current.display()))
        .stdout(std::process::Stdio::null())
        .status();
    if !matches!(st, Ok(s) if s.success()) {
        return Err(format!("`hale check {}` failed; nothing is run on a program that does not pass", seed.display()));
    }
    Ok(())
}

/// Cut the artifact of what is about to run and build it; the binary.
fn express(me: &Path, seed: &Path, current: &Path) -> Result<PathBuf, String> {
    cut_artifact(me, seed, current)?;
    build_seed(me, seed)
}

/// Build a seed; its binary.
fn build_seed(me: &Path, seed: &Path) -> Result<PathBuf, String> {
    let st = Command::new(me).arg("build").arg(seed).stdout(std::process::Stdio::null()).status();
    if !matches!(st, Ok(s) if s.success()) {
        return Err(format!("`hale build {}` failed", seed.display()));
    }
    let bin_name = seed.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "app".into());
    let bin = seed.join(&bin_name);
    if !bin.is_file() {
        return Err(format!("no binary at {}", bin.display()));
    }
    Ok(bin)
}

/// Start a process from the root, observable, with the toolchain that
/// started it; `restart_for` names the Mutation whose request this
/// restart answers and what the expression is (GH #529 D6).
fn spawn_process(bin: &Path, root: &Path, me: &Path, restart_for: Option<(&str, &str)>) -> Result<std::process::Child, String> {
    let mut c = Command::new(bin);
    c.current_dir(root).env("LOTUS_OBS", "1").env("HALE_BIN", me);
    if let Some((id, expression)) = restart_for {
        c.env("HALE_DNA_RESTART_FOR", id).env("HALE_DNA_EXPRESSION", expression);
    }
    c.spawn().map_err(|e| format!("cannot start {}: {e}", bin.display()))
}

fn wait_membrane(dna_dir: &Path, organism: &mut std::process::Child, secs: u64) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    loop {
        if dna_dir.join(crate::iris::MEMBRANE_VERDICT_SOCK).exists() && dna_dir.join(crate::iris::MEMBRANE_INTENT_SOCK).exists() {
            return true;
        }
        if let Ok(Some(_)) = organism.try_wait() {
            return false;
        }
        if std::time::Instant::now() > deadline {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

fn launch_iris(me: &Path, port: &str, current: &Path, diff_from: Option<PathBuf>, dna_dir: &Path, status_path: &Path) -> Option<std::process::Child> {
    let mut cmd = Command::new(me);
    cmd.arg("iris").arg(port).arg(current);
    if let Some(a) = diff_from {
        cmd.arg("--diff").arg(&a).arg(current);
    }
    cmd.arg("--membrane").arg(dna_dir).arg("--organism").arg(status_path);
    cmd.stdin(std::process::Stdio::null());
    match cmd.spawn() {
        Ok(c) => {
            eprintln!("hale dna run: iris at http://127.0.0.1:{port}/  (l law · 4 review · 5 organism · m membrane)");
            Some(c)
        }
        Err(e) => {
            eprintln!("hale dna run: could not launch hale iris: {e}");
            None
        }
    }
}

/// SIGTERM, a grace period, then SIGKILL; the exit is reaped.
fn terminate(organism: &mut std::process::Child) {
    unsafe {
        libc::kill(organism.id() as i32, libc::SIGTERM);
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        if let Ok(Some(_)) = organism.try_wait() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let _ = organism.kill();
    let _ = organism.wait();
}

/// The latest `expression.restart_requested` this host has not handled
/// and the organism has not already answered with `expression.restarted`.
fn pending_restart(root: &Path, handled: &BTreeSet<u64>) -> Option<(u64, String, String)> {
    let rows = read_journal(root).ok()?;
    let mut out = None;
    for (i, r) in rows.iter().enumerate() {
        if r.kind != "expression.restart_requested" || handled.contains(&r.seq) {
            continue;
        }
        let answered = rows[i + 1..].iter().any(|x| x.kind == "expression.restarted" && x.entity == r.entity);
        if !answered {
            out = Some((r.seq, r.entity.clone(), r.body.clone()));
        }
    }
    out
}

fn shape_of(artifact: &Path) -> String {
    fs::read_to_string(artifact)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v["shape_hash"].as_str().map(|s| s.to_string()))
        .unwrap_or_default()
}

/// Append one event to the Journal from the host — only while the
/// organism is NOT running (the Journal has one writer at a time; the
/// organism's in-memory projection would go stale otherwise).
fn git(root: &Path, args: &[&str]) -> Result<String, String> {
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

fn append_journal_as(root: &Path, kind: &str, entity: &str, body: &str, author: Option<&str>) -> Result<(), String> {
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

/// A receipt (a verification step's output, a diff document), by digest.
fn receipt_read(root: &Path, digest: &str) -> Option<String> {
    let out = Command::new("git").arg("-C").arg(root).args(["cat-file", "-p", &format!("refs/dna/receipts/{digest}")]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).to_string())
}

// ---------------------------------------------------------------
// the record across clones: sync with origin, and the membrane over it
// ---------------------------------------------------------------

const REMOTE_TRACK: &str = "refs/dna/remote/journal";

/// The remote the record syncs with, when the repository has one.
fn record_remote(root: &Path) -> Option<String> {
    let name = git(root, &["config", "dna.remote"]).ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "origin".into());
    git(root, &["remote", "get-url", &name]).ok().map(|_| name)
}

/// Fetch, reconcile, push (GH #566 F1). The record is linear: when
/// this clone and the remote both appended, the local-only events are
/// re-appended on top of the remote's head (their bodies and authors
/// unchanged; their seq is their new position) and pushed. Receipts
/// travel both ways by refspec. Returns a one-line summary.
fn sync_record(root: &Path) -> Result<String, String> {
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

fn sync_cmd(args: &[String]) -> Result<Vec<String>, String> {
    let dir = args.first().map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    let (root, _) = project(&dir)?;
    let s = sync_record(&root)?;
    let rows = read_journal(&root).map(|r| r.len()).unwrap_or(0);
    Ok(vec![format!("record {RECORD_REF}: {s}; {rows} event(s)")])
}

/// The membrane over the record: an `intent.requested` or a
/// `review.verdict` row a CLI appended from any clone is relayed by the
/// host onto the running organism's sockets, once, and the organism's
/// answer (`intent.offered` / `review.settled` / …) goes back into the
/// record like any other event. Rows already answered are skipped, so
/// a restarted host does not replay history.
fn relay_record_membrane(root: &Path, relayed: &mut BTreeSet<u64>) -> usize {
    let Ok(rows) = read_journal(root) else { return 0 };
    let mut n = 0;
    for (i, r) in rows.iter().enumerate() {
        if relayed.contains(&r.seq) {
            continue;
        }
        let answered = |kinds: &[&str]| rows[i + 1..].iter().any(|x| kinds.contains(&x.kind.as_str()) && x.entity == r.entity);
        match r.kind.as_str() {
            "intent.requested" if !answered(&["intent.offered", "intent.refused"]) => {
                let mut b: Value = serde_json::from_str(&r.body).unwrap_or(Value::Null);
                if let Some(o) = b.as_object_mut() {
                    o.insert("intent_id".into(), Value::String(r.entity.clone()));
                }
                if publish_on_membrane(root, "intent", &b.to_string()).is_ok() {
                    relayed.insert(r.seq);
                    n += 1;
                }
            }
            "review.verdict" if !answered(&["review.settled", "review.refused"]) => {
                if publish_on_membrane(root, "verdict", &r.body).is_ok() {
                    relayed.insert(r.seq);
                    n += 1;
                }
            }
            _ => {}
        }
    }
    n
}

/// Re-project the Journal into `status.json` (atomically: write beside,
/// rename over) when it changed.
fn write_status(root: &Path, path: &Path) {
    let Ok(st) = status_projection(root) else { return };
    let text = serde_json::to_string_pretty(&st).unwrap_or_default();
    if fs::read_to_string(path).map(|t| t == text).unwrap_or(false) {
        return;
    }
    let tmp = path.with_extension("json.tmp");
    if fs::write(&tmp, text).is_ok() {
        let _ = fs::rename(&tmp, path);
    }
}

// ---------------------------------------------------------------
// the Journal as read by the host: status / ask / history
// ---------------------------------------------------------------

#[derive(Clone)]
struct Row {
    seq: u64,
    kind: String,
    entity: String,
    body: String,
}

fn read_journal(root: &Path) -> Result<Vec<Row>, String> {
    let Some(_) = record_head(root) else {
        return Err(format!("no record at {RECORD_REF} in {} (run `hale dna init`)", root.display()));
    };
    let text = git(root, &["show", &format!("{RECORD_REF}:journal.jsonl")])?;
    let mut rows = Vec::new();
    for (i, line) in text.lines().filter(|l| !l.trim().is_empty()).enumerate() {
        let v: Value = serde_json::from_str(line).map_err(|e| format!("{RECORD_REF}:journal.jsonl:{}: not JSON: {e}", i + 1))?;
        let s = |k: &str| v[k].as_str().unwrap_or("").to_string();
        rows.push(Row { seq: v["seq"].as_u64().unwrap_or(i as u64), kind: s("kind"), entity: s("entity"), body: s("body") });
    }
    Ok(rows)
}

/// The chain is git's: the ref's commit count is the event count.
fn chain_ok(root: &Path, rows: &[Row]) -> bool {
    match git(root, &["rev-list", "--count", RECORD_REF]) {
        Ok(n) => n.trim().parse::<usize>().map(|c| c == rows.len()).unwrap_or(false),
        Err(_) => rows.is_empty(),
    }
}

fn membrane_up(root: &Path) -> bool {
    let d = root.join(".hale/dna");
    d.join(crate::iris::MEMBRANE_VERDICT_SOCK).exists() && d.join(crate::iris::MEMBRANE_INTENT_SOCK).exists()
}

/// The status projection: everything the Journal can vouch for, plus
/// the expression identity from the artifacts on disk and whether an
/// organism is currently bound to its membrane.
fn status_projection(root: &Path) -> Result<Value, String> {
    let rows = read_journal(root)?;
    let body = |r: &Row| -> Value { serde_json::from_str(&r.body).unwrap_or(Value::String(r.body.clone())) };
    let attached = rows.iter().find(|r| r.kind == "application.attached").map(body).unwrap_or(Value::Null);
    // tasks: born minus settled
    let mut tasks: std::collections::BTreeMap<String, Value> = std::collections::BTreeMap::new();
    for r in &rows {
        if r.kind == "task.born" {
            tasks.insert(r.entity.clone(), serde_json::json!({"id": r.entity, "outcome": r.body, "state": "active", "since": r.seq}));
        } else if let Some(disp) = r.kind.strip_prefix("task.") {
            if let Some(t) = tasks.get_mut(&r.entity) {
                t["state"] = Value::String(disp.to_string());
                t["detail"] = Value::String(r.body.clone());
            }
        }
    }
    let intents_offered = rows.iter().filter(|r| r.kind == "intent.offered").count();
    let intents_refused: Vec<Value> = rows.iter().filter(|r| r.kind == "intent.refused").map(|r| serde_json::json!({"id": r.entity, "reason": r.body})).collect();
    // reviews: requested minus settled, with why
    let mut reviews: std::collections::BTreeMap<String, Value> = std::collections::BTreeMap::new();
    for r in &rows {
        let id = r.entity.strip_prefix("review:").unwrap_or(&r.entity).to_string();
        match r.kind.as_str() {
            "review.requested" => {
                let b = body(r);
                let mut v = serde_json::json!({"id": id, "state": "pending", "question": b["question"], "required_authority": b["required_authority"], "subject_digest": b["subject_digest"], "refusals": []});
                // a mutation's Review carries what a reviewer renders (GH #529 D4)
                if b["mutation_id"].is_string() {
                    for k in ["mutation_id", "change_class", "disposition", "base_commit", "candidate_commit", "candidate_shape", "evidence", "magnitude", "diff_text", "diff_json", "author"] {
                        v[k] = b[k].clone();
                    }
                }
                reviews.insert(id.clone(), v);
            }
            "review.settled" => {
                if let Some(v) = reviews.get_mut(&id) {
                    v["state"] = Value::String("settled".into());
                    v["settled"] = Value::String(r.body.clone());
                }
            }
            "review.refused" => {
                if let Some(v) = reviews.get_mut(&id) {
                    if let Some(a) = v["refusals"].as_array_mut() {
                        a.push(Value::String(r.body.clone()));
                    }
                }
            }
            _ => {}
        }
    }
    // mutations: one record per id, its last disposition and its
    // candidate; the gateway's worktree/candidate/applied rows and the
    // assembly's proposed/<disposition>/failed rows all name the id.
    let mut mutations: std::collections::BTreeMap<String, Value> = std::collections::BTreeMap::new();
    for r in rows.iter().filter(|r| r.kind.starts_with("mutation.")) {
        let kind = r.kind.trim_start_matches("mutation.").to_string();
        let m = mutations.entry(r.entity.clone()).or_insert_with(|| serde_json::json!({"id": r.entity, "candidate": "", "disposition": "", "class": "", "events": []}));
        match kind.as_str() {
            "proposed" => {
                // "task <id> <class>: <objective> (<target>) at <base>"
                let rest = r.body.strip_prefix("task ").unwrap_or(&r.body);
                let (task, rest) = rest.split_once(' ').unwrap_or(("", rest));
                let (class, rest) = rest.split_once(": ").unwrap_or((rest, ""));
                let objective = rest.rsplit_once(" at ").map(|(o, _)| o).unwrap_or(rest);
                m["task"] = Value::String(task.to_string());
                m["class"] = Value::String(class.to_string());
                m["objective"] = Value::String(objective.to_string());
                m["disposition"] = Value::String("proposed".into());
            }
            "worktree" => m["worktree"] = Value::String(r.body.clone()),
            "candidate" => m["candidate"] = Value::String(r.body.clone()),
            other => {
                m["disposition"] = Value::String(other.to_string());
                m["detail"] = Value::String(r.body.clone());
            }
        }
        if let Some(a) = m["events"].as_array_mut() {
            a.push(Value::String(kind));
        }
    }
    // a settled Review moves its mutation on: reviewed (approve) or
    // rejected — unless the assembly already took it further (applied,
    // retained, rolled back, refused), which the mutation rows say.
    for r in rows.iter().filter(|r| r.kind == "review.settled") {
        if let Some(m) = mutations.get_mut(&r.entity) {
            let d = m["disposition"].as_str().unwrap_or("").to_string();
            if matches!(d.as_str(), "" | "proposed" | "review" | "stage" | "escalate" | "release") {
                m["disposition"] = Value::String(if r.body.starts_with("approve") { "reviewed".into() } else { "rejected".into() });
                m["detail"] = Value::String(r.body.clone());
            }
        }
    }
    let mutations: Vec<Value> = mutations.into_values().collect();
    let deferred: Vec<Value> = rows.iter().filter(|r| r.kind == "law.deferred").map(body).collect();
    let restarts = rows.iter().filter(|r| r.kind == "expression.restarted").count();
    let last_restart_request = rows.iter().rev().find(|r| r.kind == "expression.restart_requested").map(|r| serde_json::json!({"mutation": r.entity, "what": r.body, "seq": r.seq}));
    let last_observed = rows.iter().rev().find(|r| r.kind == "expression.observed" || r.kind == "expression.crashed").map(|r| serde_json::json!({"mutation": r.entity, "what": format!("{} {}", r.kind.trim_start_matches("expression."), r.body), "seq": r.seq}));
    let model_calls: Vec<Value> = rows
        .iter()
        .filter(|r| r.kind == "model.called")
        .rev()
        .take(8)
        .map(|r| {
            let mut b = body(r);
            if let Some(o) = b.as_object_mut() {
                o.insert("attempt".into(), Value::String(r.entity.clone()));
                o.insert("seq".into(), Value::from(r.seq));
            }
            b
        })
        .collect();
    let model_calls_total = rows.iter().filter(|r| r.kind == "model.called").count();
    // expression identity: what init saw, what would run now, what is built
    let current = root.join(".hale/dna/current.topology");
    let current_id = fs::read_to_string(&current)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .map(|v| serde_json::json!({"shape_hash": v["shape_hash"], "artifact_digest": v["artifact_digest"], "verdict": v["verdict"]}))
        .unwrap_or(Value::Null);
    let seed = crate::pkg::read_claims_config(&root.join("hale.toml"))
        .ok()
        .and_then(|(envs, _)| envs.get("local").and_then(|e| e.entrypoints.first().cloned()))
        .map(|e| root.join(e))
        .unwrap_or_else(|| root.to_path_buf());
    let bin = seed.join(seed.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
    let build_digest = crate::sign::sha256_file(&bin).ok();
    Ok(serde_json::json!({
        "organism": if membrane_up(root) { "running (membrane bound)" } else { "not running — reading the Journal" },
        "journal": {"ref": RECORD_REF, "revision": rows.len(), "chain": if chain_ok(root, &rows) { "verified" } else { "BROKEN" }},
        "expression": {"attached": attached, "current": current_id, "build_digest": build_digest, "toolchain": TOOLCHAIN, "restarts": restarts, "last_restart_request": last_restart_request, "last_observed": last_observed},
        "intents": {"offered": intents_offered, "refused": intents_refused},
        "tasks": tasks.values().cloned().collect::<Vec<_>>(),
        "reviews": reviews.values().cloned().collect::<Vec<_>>(),
        "mutations": mutations,
        "law_deferred": deferred,
        "model_calls": {"total": model_calls_total, "recent": model_calls},
        "pressure": "not journaled in Phase 1",
        "projected_at": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
    }))
}

fn status(args: &[String]) -> Result<Vec<String>, String> {
    let mut dir = PathBuf::from(".");
    let mut json = false;
    for a in args {
        match a.as_str() {
            "--json" => json = true,
            f if f.starts_with("--") => return Err(format!("status: unknown flag `{f}`")),
            p => dir = PathBuf::from(p),
        }
    }
    let (root, _) = project(&dir)?;
    let st = status_projection(&root)?;
    if json {
        return Ok(vec![serde_json::to_string_pretty(&st).unwrap_or_default()]);
    }
    let mut out = Vec::new();
    let s = |v: &Value| v.as_str().unwrap_or("").to_string();
    out.push(format!("organism:   {}", s(&st["organism"])));
    out.push(format!("journal:    {} event(s), chain {}", st["journal"]["revision"], s(&st["journal"]["chain"])));
    let e = &st["expression"];
    out.push(format!(
        "expression: attached {} (shape {}) · current shape {} · build {}",
        s(&e["attached"]["main"]),
        s(&e["attached"]["shape_hash"]),
        e["current"]["shape_hash"].as_str().unwrap_or("not cut"),
        e["build_digest"].as_str().map(|d| d[..12].to_string()).unwrap_or_else(|| "not built".into())
    ));
    out.push(format!("intents:    {} offered, {} refused", st["intents"]["offered"], st["intents"]["refused"].as_array().map(|a| a.len()).unwrap_or(0)));
    let tasks = st["tasks"].as_array().cloned().unwrap_or_default();
    out.push(format!("tasks:      {}", if tasks.is_empty() { "none".to_string() } else { format!("{}", tasks.len()) }));
    for t in &tasks {
        out.push(format!("  {} [{}] {}", s(&t["id"]), s(&t["state"]), s(&t["outcome"])));
    }
    let reviews = st["reviews"].as_array().cloned().unwrap_or_default();
    let pending = reviews.iter().filter(|r| r["state"] == "pending").count();
    out.push(format!("reviews:    {} pending of {}", pending, reviews.len()));
    for r in &reviews {
        let why = if r["state"] == "pending" {
            format!("needs {} — {}", s(&r["required_authority"]), s(&r["question"]))
        } else {
            format!("settled {}", s(&r["settled"]))
        };
        let refusals = r["refusals"].as_array().map(|a| a.len()).unwrap_or(0);
        out.push(format!("  {} [{}] {}{}", s(&r["id"]), s(&r["state"]), why, if refusals > 0 { format!(" ({refusals} verdict(s) refused)") } else { String::new() }));
    }
    let muts = st["mutations"].as_array().cloned().unwrap_or_default();
    out.push(format!("mutations:  {} (none applies before a human's verdict on the exact candidate)", muts.len()));
    for m in &muts {
        let cand = s(&m["candidate"]);
        let task = s(&m["task"]);
        out.push(format!(
            "  {} [{}] {}: {}{}{}",
            s(&m["id"]),
            s(&m["disposition"]),
            s(&m["class"]),
            m["objective"].as_str().unwrap_or(""),
            if task.is_empty() { String::new() } else { format!(" · task {task}") },
            if cand.is_empty() { String::new() } else { format!(" · candidate {}", &cand[..cand.len().min(12)]) }
        ));
    }
    let deferred = st["law_deferred"].as_array().map(|a| a.len()).unwrap_or(0);
    if deferred > 0 {
        out.push(format!("law:        {} clause(s) deferred at init (see dna_constitution.hl)", deferred));
    }
    Ok(out)
}

/// `hale dna ask [--to <locus>] <intent…>`: publish a typed
/// IntentOffered on the membrane through the embedded client, then
/// read the organism's answer back from the Journal — the Task born
/// or the refusal. The host publishes and reads; it decides nothing.
fn ask(args: &[String]) -> Result<Vec<String>, String> {
    let mut to = String::new();
    let mut no_wait = false;
    let mut words: Vec<String> = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--to" => to = it.next().cloned().ok_or("--to needs a locus")?,
            "--no-wait" => no_wait = true,
            f if f.starts_with("--") => return Err(format!("ask: unknown flag `{f}`")),
            w => words.push(w.to_string()),
        }
    }
    if words.is_empty() {
        return Err("ask: say what should happen — `hale dna ask \"<intent>\"`".into());
    }
    let outcome = words.join(" ");
    let (root, _) = project(Path::new("."))?;
    let _ = sync_record(&root);
    let rows_before = read_journal(&root)?.len();
    let intent_id = format!("i{:x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0));
    let who = std::env::var("USER").unwrap_or_else(|_| "human".into());
    let body = serde_json::json!({"intent_id": intent_id, "outcome": outcome, "from": who, "to": to}).to_string();
    let local = membrane_up(&root);
    if local {
        publish_on_membrane(&root, "intent", &body)?;
    } else {
        // no organism here: the intent goes into the record, and the host
        // beside the organism relays it (GH #566 F1)
        if record_remote(&root).is_none() {
            return Err(format!(
                "the organism is not running here (no membrane under {}) and the repository has no remote to reach one through; start it with `hale dna run`",
                root.join(".hale/dna").display()
            ));
        }
        append_journal(&root, "intent.requested", &intent_id, &serde_json::json!({"outcome": outcome, "from": who, "to": to}).to_string())?;
        sync_record(&root)?;
        if no_wait {
            return Ok(vec![format!("intent {intent_id} requested in the record; the organism answers there (`hale dna sync`, then `status`)")]);
        }
    }
    // the organism's answer, from the record
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(if local { 10 } else { 60 });
    loop {
        if !local {
            let _ = sync_record(&root);
        }
        let rows = read_journal(&root)?;
        if let Some(r) = rows.iter().skip(rows_before).find(|r| r.kind == "intent.refused" && r.entity == intent_id) {
            return Ok(vec![format!("refused: {}", r.body)]);
        }
        if let Some(pos) = rows.iter().skip(rows_before).position(|r| r.kind == "intent.offered" && r.entity == intent_id) {
            if let Some(t) = rows.iter().skip(rows_before + pos).find(|r| r.kind == "task.born") {
                let state = rows.iter().skip(rows_before).filter(|x| x.entity == t.entity && x.kind.starts_with("task.") && x.kind != "task.born").last().map(|x| x.kind.trim_start_matches("task.").to_string()).unwrap_or_else(|| "active".into());
                return Ok(vec![format!("task {} born for intent {} [{}]", t.entity, intent_id, state)]);
            }
        }
        if std::time::Instant::now() > deadline {
            return Err(format!("published intent {intent_id}, but the organism journaled no answer in time (record revision {})", rows.len()));
        }
        std::thread::sleep(std::time::Duration::from_millis(if local { 150 } else { 1000 }));
    }
}

/// Publish one typed fact on the membrane: build (once, in the
/// toolchain cache beside the core it imports) and exec the membrane
/// client with routes to this project's sockets. A listen binding
/// serves many peers (F.13, fixed), so this connects beside an
/// attached iris.
fn publish_on_membrane(root: &Path, kind: &str, body: &str) -> Result<(), String> {
    let cache = hale_iris::materialize().map_err(|e| format!("cannot materialize the toolchain cache: {e}"))?;
    let bin = cache.join(hale_dna::MEMBRANE_BIN);
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    if !bin.is_file() {
        eprintln!("hale dna: building the membrane client ({})", cache.join(hale_dna::MEMBRANE_SEED).display());
        let st = Command::new(&me).arg("build").arg(cache.join(hale_dna::MEMBRANE_SEED)).stdout(std::process::Stdio::null()).status().map_err(|e| e.to_string())?;
        if !st.success() || !bin.is_file() {
            return Err("building the membrane client failed".into());
        }
    }
    let dna_dir = root.join(".hale/dna");
    let conf = std::env::temp_dir().join(format!("hale-dna-membrane-{}.conf", std::process::id()));
    fs::write(
        &conf,
        format!(
            "dna.review.verdict = unix://{}/{} : connect\ndna.intent.offered = unix://{}/{} : connect\ndna.expression.observed = unix://{}/{} : connect\n",
            dna_dir.display(),
            crate::iris::MEMBRANE_VERDICT_SOCK,
            dna_dir.display(),
            crate::iris::MEMBRANE_INTENT_SOCK,
            dna_dir.display(),
            crate::iris::MEMBRANE_OBSERVED_SOCK
        ),
    )
    .map_err(|e| e.to_string())?;
    let out = Command::new(&bin).arg(kind).arg(body).env("LOTUS_BUS_CONFIG", &conf).output().map_err(|e| format!("membrane client: {e}"))?;
    let _ = fs::remove_file(&conf);
    if !out.status.success() {
        return Err(format!("membrane client failed: {}", String::from_utf8_lossy(&out.stderr)));
    }
    Ok(())
}

/// `hale dna review <id> <verdict> [--as R] [--authority A] [--comment C]`:
/// a verdict on a pending Review, over the membrane. The candidate
/// digest is the one the Journal recorded for the request — the
/// organism still pins it against the Review's own — and the answer
/// (settled, or refused with the reason) is read back from the
/// Journal. Track D extends this to staged mutations.
fn review(args: &[String]) -> Result<Vec<String>, String> {
    let mut pos: Vec<String> = Vec::new();
    let mut reviewer = std::env::var("USER").unwrap_or_else(|_| "human".into());
    let mut authority = "board".to_string();
    let mut comment = String::new();
    let mut digest_override: Option<String> = None;
    let mut iris = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--as" => reviewer = it.next().cloned().ok_or("--as needs a reviewer")?,
            "--authority" => authority = it.next().cloned().ok_or("--authority needs a value")?,
            "--comment" => comment = it.next().cloned().ok_or("--comment needs text")?,
            "--digest" => digest_override = Some(it.next().cloned().ok_or("--digest needs the candidate digest you looked at")?),
            "--iris" => iris = true,
            f if f.starts_with("--") => return Err(format!("review: unknown flag `{f}`")),
            w => pos.push(w.to_string()),
        }
    }
    let (root, _) = project(Path::new("."))?;
    if !membrane_up(&root) {
        let _ = sync_record(&root);
    }
    let st = status_projection(&root)?;
    let reviews = st["reviews"].as_array().cloned().unwrap_or_default();
    let s = |v: &Value| v.as_str().unwrap_or("").to_string();
    // ---- no id: the pending Reviews ------------------------------------
    let Some(id) = pos.first().cloned() else {
        let pending: Vec<&Value> = reviews.iter().filter(|r| r["state"] == "pending").collect();
        let mut out = vec![format!("{} pending review(s) of {}", pending.len(), reviews.len())];
        for r in pending {
            out.push(format!("  {} needs {} — {}", s(&r["id"]), s(&r["required_authority"]), s(&r["question"])));
            if r["mutation_id"].is_string() {
                out.push(format!(
                    "      {} · candidate {} · evidence {} · disposition {}",
                    s(&r["change_class"]),
                    short(&s(&r["candidate_commit"])),
                    s(&r["evidence"]),
                    s(&r["disposition"])
                ));
            }
        }
        out.push("render one with `hale dna review <id>`; decide with `hale dna review <id> approve|revise|reject|abstain`".into());
        return Ok(out);
    };
    let r = reviews
        .iter()
        .find(|r| r["id"] == id.as_str())
        .ok_or_else(|| format!("no `review.requested` for `{id}` in the Journal (pending: {})", reviews.iter().filter(|r| r["state"] == "pending").map(|r| s(&r["id"])).collect::<Vec<_>>().join(", ")))?
        .clone();
    let digest = s(&r["subject_digest"]);
    // ---- id only: render what the reviewer decides on ------------------
    let Some(verdict) = pos.get(1).cloned() else {
        return render_review(&root, &r, iris);
    };
    if !["approve", "revise", "reject", "abstain"].contains(&verdict.as_str()) {
        return Err(format!("review: verdict `{verdict}` is not one of approve | revise | reject | abstain"));
    }
    let local = membrane_up(&root);
    if !local && record_remote(&root).is_none() {
        return Err("the organism is not running here (no membrane) and the repository has no remote to reach one through; start it with `hale dna run`".into());
    }
    // The verdict names the candidate the reviewer LOOKED AT: by default
    // the one the request pinned; `--digest` states it explicitly, and
    // the Review refuses a mismatch (the TOCTOU pin, #521 open question 14).
    let named = digest_override.unwrap_or(digest);
    let rows = read_journal(&root)?;
    let before = rows.len();
    let body = serde_json::json!({"review_id": id, "subject_digest": named, "verdict": verdict, "reviewer": reviewer, "authority": authority, "comment": comment}).to_string();
    if local {
        publish_on_membrane(&root, "verdict", &body)?;
    } else {
        // the verdict is a row in the record, in the reviewer's name; the host
        // beside the organism relays it, and the Review answers in the record
        append_journal(&root, "review.verdict", &id, &body)?;
        sync_record(&root)?;
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(if local { 10 } else { 60 });
    loop {
        if !local {
            let _ = sync_record(&root);
        }
        let rows = read_journal(&root)?;
        if let Some(r) = rows.iter().skip(before).find(|r| (r.kind == "review.settled" || r.kind == "review.refused") && r.entity == id) {
            return Ok(vec![if r.kind == "review.settled" { format!("review {id} settled: {}", r.body) } else { format!("review {id} refused the verdict: {}", r.body) }]);
        }
        if std::time::Instant::now() > deadline {
            return Err(format!("published the verdict, but the organism journaled no answer for `{id}` in time"));
        }
        std::thread::sleep(std::time::Duration::from_millis(if local { 150 } else { 1000 }));
    }
}

fn short(sha: &str) -> String {
    sha.chars().take(12).collect()
}

/// What a reviewer decides on (GH #529 D4): the question and the
/// exact candidate; for a mutation, the source diff (git, base to
/// candidate), the semantic diff (`hale model diff --text`, from the
/// receipt on disk), the evidence table (one row per toolchain step,
/// with its content-addressed receipt) and the magnitude vector. All
/// of it from the Journal and the evidence dir — no organism needed.
/// `--iris` opens the review perspective on the same diff beside the
/// organism's status and the membrane form, so the verdict can be
/// sent from either.
fn render_review(root: &Path, r: &Value, iris: bool) -> Result<Vec<String>, String> {
    let s = |v: &Value| v.as_str().unwrap_or("").to_string();
    let id = s(&r["id"]);
    let mut out = vec![
        format!("review {id} [{}]: {}", s(&r["state"]), s(&r["question"])),
        format!("  needs {} · candidate {}{}", s(&r["required_authority"]), s(&r["subject_digest"]), if r["state"] == "pending" { String::new() } else { format!(" · settled {}", s(&r["settled"])) }),
    ];
    for refusal in r["refusals"].as_array().cloned().unwrap_or_default() {
        out.push(format!("  refused a verdict: {}", s(&refusal)));
    }
    if !r["mutation_id"].is_string() {
        out.push(format!("decide: hale dna review {id} approve|revise|reject|abstain [--as <you>] [--comment <c>]"));
        return Ok(out);
    }
    let base = s(&r["base_commit"]);
    let cand = s(&r["candidate_commit"]);
    out.push(format!(
        "  mutation {} ({}) by {} · disposition under the grant: {} · shape {}",
        s(&r["mutation_id"]),
        s(&r["change_class"]),
        s(&r["author"]),
        s(&r["disposition"]),
        s(&r["candidate_shape"])
    ));
    // magnitude: the vector, never a score
    if let Some(m) = r["magnitude"].as_object() {
        let facets: Vec<String> = m
            .iter()
            .filter(|(_, v)| match v {
                Value::Bool(b) => *b,
                Value::Number(n) => n.as_i64().unwrap_or(0) != 0,
                _ => false,
            })
            .map(|(k, v)| if v.is_boolean() { k.clone() } else { format!("{k} {v}") })
            .collect();
        out.push(format!("  magnitude: {}", if facets.is_empty() { "nothing moved in the model".to_string() } else { facets.join(", ") }));
    }
    // source diff
    out.push(String::new());
    out.push(format!("source diff (git {} .. {}):", short(&base), short(&cand)));
    let git = Command::new("git")
        .args(["-C", &root.to_string_lossy(), "diff", "--stat", "-p", &base, &cand])
        .output()
        .map_err(|e| format!("git diff: {e}"))?;
    let text = String::from_utf8_lossy(&git.stdout);
    if git.status.success() {
        for l in text.lines() {
            out.push(format!("  {l}"));
        }
    } else {
        out.push(format!("  (git diff failed: {})", String::from_utf8_lossy(&git.stderr).trim()));
    }
    // semantic diff, from the receipt on disk
    out.push(String::new());
    out.push("semantic diff (hale model diff, baseline .. candidate):".into());
    let diff_text = s(&r["diff_text"]);
    match receipt_read(root, &diff_text) {
        Some(t) => {
            for l in t.lines() {
                out.push(format!("  {l}"));
            }
        }
        None => out.push(format!("  (no semantic diff receipt {diff_text})")),
    }
    // evidence: one row per step, with its receipt
    out.push(String::new());
    out.push(format!("evidence ({}):", s(&r["evidence"])));
    out.push(format!("  {:<10} {:<6} {:<5} {:<14} {}", "step", "ok", "code", "receipt", "bytes"));
    let rows = read_journal(root)?;
    for e in rows.iter().filter(|e| e.kind.starts_with("evidence.") && e.entity == cand && e.kind != "evidence.magnitude") {
        let b: Value = serde_json::from_str(&e.body).unwrap_or(Value::Null);
        out.push(format!(
            "  {:<10} {:<6} {:<5} {:<14} {}",
            e.kind.trim_start_matches("evidence."),
            if b["ok"].as_bool().unwrap_or(false) { "yes" } else { "NO" },
            b["code"],
            short(&s(&b["output_digest"])),
            b["bytes"]
        ));
    }
    out.push("  receipts: refs/dna/receipts/<digest> (git cat-file -p)".to_string());
    out.push(String::new());
    out.push(format!(
        "decide: hale dna review {id} approve|revise|reject|abstain [--as <you>] [--comment <c>] [--digest {}]",
        short(&cand)
    ));
    if iris {
        let diff_json = s(&r["diff_json"]);
        if diff_json.is_empty() {
            return Err("--iris: this review has no semantic diff document".into());
        }
        let me = std::env::current_exe().map_err(|e| e.to_string())?;
        let status = root.join(".hale/dna/status.json");
        let doc = receipt_read(root, &diff_json).ok_or_else(|| format!("--iris: no diff receipt {diff_json}"))?;
        let doc_path = root.join(".hale/dna/scratch").join(format!("review-{id}.diff.json"));
        fs::create_dir_all(doc_path.parent().unwrap()).map_err(|e| e.to_string())?;
        fs::write(&doc_path, doc).map_err(|e| e.to_string())?;
        let mut c = Command::new(&me);
        c.arg("iris")
            .arg("--diff")
            .arg(&doc_path)
            .arg("--membrane")
            .arg(root.join(".hale/dna"))
            .arg("--organism")
            .arg(&status)
            .current_dir(root);
        let child = c.spawn().map_err(|e| format!("hale iris: {e}"))?;
        out.push(format!("iris: review perspective [4] on {} (pid {}); the membrane form sends the verdict", diff_json, child.id()));
    }
    Ok(out)
}

/// `hale dna history [<entity>]`: the Journal in order, or the rows
/// that concern one entity and everything they link to (an intent's
/// Task, a Task's settlement, a Review's verdicts), by id.
fn history(args: &[String]) -> Result<Vec<String>, String> {
    let mut dir = PathBuf::from(".");
    let mut entity: Option<String> = None;
    for a in args {
        if a.starts_with("--") {
            return Err(format!("history: unknown flag `{a}`"));
        } else if Path::new(a).join("hale.toml").exists() {
            dir = PathBuf::from(a);
        } else {
            entity = Some(a.clone());
        }
    }
    let (root, _) = project(&dir)?;
    let rows = read_journal(&root)?;
    let chain = if chain_ok(&root, &rows) { "verified" } else { "BROKEN" };
    let selected: Vec<&Row> = match &entity {
        None => rows.iter().collect(),
        Some(e) => {
            // seed: rows about the entity; expand once through ids that
            // co-occur (a task born for an intent, a settlement for a task).
            let mut ids: BTreeSet<String> = BTreeSet::new();
            ids.insert(e.clone());
            let mentions = |r: &Row, id: &str| r.entity == id || r.entity.ends_with(&format!(":{id}")) || r.body.contains(id);
            let looks_like_id = |t: &str| -> bool {
                let mut ch = t.chars();
                matches!(ch.next(), Some('i') | Some('t') | Some('w') | Some('a') | Some('r')) && ch.clone().next().is_some() && ch.all(|c| c.is_ascii_alphanumeric())
            };
            for _ in 0..3 {
                let snapshot: Vec<String> = ids.iter().cloned().collect();
                for r in &rows {
                    if snapshot.iter().any(|id| mentions(r, id)) {
                        ids.insert(r.entity.trim_start_matches("review:").trim_start_matches("locus:").to_string());
                        // ids named in the body ("i1: write the changelog")
                        for tok in r.body.split(|c: char| c == ':' || c.is_whitespace() || c == ',' || c == '"') {
                            if looks_like_id(tok) && tok.len() <= 20 {
                                ids.insert(tok.to_string());
                            }
                        }
                    }
                }
            }
            rows.iter().filter(|r| ids.iter().any(|id| mentions(r, id))).collect()
        }
    };
    let mut out = vec![format!("record {} — {} event(s), chain {}", RECORD_REF, rows.len(), chain)];
    if let Some(e) = &entity {
        out.push(format!("history of {e}: {} event(s)", selected.len()));
    }
    for r in selected {
        let b = if r.body.len() > 96 { format!("{}…", &r.body[..96]) } else { r.body.clone() };
        out.push(format!("{:>5}  {:<22} {:<28} {}", r.seq, r.kind, r.entity, b));
    }
    Ok(out)
}

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
        r#"// dna/purpose.hl — the declared purpose (project-owned).
//
// The first governed workflow is the baseline review: the Review
// named `purpose` in dna/assembly.hl ratifies THIS text (its
// subject digest is the sha256 of PURPOSE). Change the text and the
// Review's digest together, or the verdict is refused as stale.

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
//               the editing position, the Reviews
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
            // Models: hosted adapters present a credential from a sealed
            // locus (set OPENAI_API_KEY; the material never enters this
            // tree) and are not permitted backends without one; the
            // private slot is a local OpenAI-compatible endpoint. Every
            // call journals its evidence, never the prompt.
            work: dna::WorkSystem {{
                agent: dna::AgentPerformer {{
                    name: "agent",
                    models: dna::ModelRouter {{
                        quick: dna::HostedModel {{ name: "quick", model: "gpt-4o-mini", credential: dna::HostedCredential {{ env_var: "OPENAI_API_KEY" }} }},
                        deep: dna::HostedModel {{ name: "deep", model: "gpt-4o", credential: dna::HostedCredential {{ env_var: "OPENAI_API_KEY" }}, input_micros_per_1k: 2500, output_micros_per_1k: 10000 }},
                        private: dna::LocalModel {{ name: "private", endpoint: "http://127.0.0.1:11434/v1/chat/completions", model: "llama3" }}
                    }}
                }}
            }},
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
            editor: dna::SourceEditor {{
                name: "editor",
                models: dna::ModelRouter {{
                    quick: dna::HostedModel {{ name: "quick", model: "gpt-4o-mini", credential: dna::HostedCredential {{ env_var: "OPENAI_API_KEY" }} }},
                    deep: dna::HostedModel {{ name: "deep", model: "gpt-4o", credential: dna::HostedCredential {{ env_var: "OPENAI_API_KEY" }}, input_micros_per_1k: 2500, output_micros_per_1k: 10000 }},
                    private: dna::LocalModel {{ name: "private", endpoint: "http://127.0.0.1:11434/v1/chat/completions", model: "llama3" }}
                }}
            }},
            genome_seed: "{seed}"
        }};
        // The Leader: decides the Reviews inside the grant, with the deep
        // tier, reading the source diff and the semantic diff; every
        // decision is a model call with evidence in the record.
        leader: dna::Leader = dna::Leader {{
            name: "leader",
            models: dna::ModelRouter {{
                quick: dna::HostedModel {{ name: "quick", model: "gpt-4o-mini", credential: dna::HostedCredential {{ env_var: "OPENAI_API_KEY" }} }},
                deep: dna::HostedModel {{ name: "deep", model: "gpt-4o", credential: dna::HostedCredential {{ env_var: "OPENAI_API_KEY" }}, input_micros_per_1k: 2500, output_micros_per_1k: 10000 }},
                private: dna::LocalModel {{ name: "private", endpoint: "http://127.0.0.1:11434/v1/chat/completions", model: "llama3" }}
            }},
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
group knowledge = { dna::Knowledge };
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

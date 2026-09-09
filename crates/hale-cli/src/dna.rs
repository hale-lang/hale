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
//!   .hale/dna/journal.jsonl    the Journal, seeded from the topology artifact
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

const JOURNAL_REL: &str = ".hale/dna/journal.jsonl";
const BASELINE_REL: &str = ".hale/dna/baseline.topology";
const VERDICT_SOCK_REL: &str = ".hale/dna/hale-dna.review.verdict.sock";
const INTENT_SOCK_REL: &str = ".hale/dna/hale-dna.intent.offered.sock";

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
        Some("run") => run_organism(&args[1..]),
        Some("status") => report(status(&args[1..])),
        Some("ask") => report(ask(&args[1..])),
        Some("history") => report(history(&args[1..])),
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
    eprintln!("                                    build, run under LOTUS_OBS with iris attached, hold the membrane");
    eprintln!("       hale dna status [project] [--json]");
    eprintln!("                                    the organism's status projection, from the Journal");
    eprintln!("       hale dna ask [--to <locus>] <intent…>");
    eprintln!("                                    offer intent over the membrane; prints the Task born or the refusal");
    eprintln!("       hale dna history [<entity>]  walk the Journal by causal links (works offline)");
    eprintln!("       hale dna review <id> approve|revise|reject|abstain [--as <reviewer>] [--authority <a>] [--comment <c>]");
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
    // 4. the project-owned DNA seed
    let purpose_text = format!("{}: keep the application correct, reviewable and explainable; every change is staged, reviewed by a maintainer, and never applied by the organism itself.", app.project);
    let purpose_digest = format!("sha256:{}", hex(&openssl::sha::sha256(purpose_text.as_bytes())));
    created(&mut out, &app.root.join("dna/purpose.hl"), &purpose_hl(&purpose_text))?;
    created(&mut out, &app.root.join("dna/assembly.hl"), &assembly_hl(&app.project, &purpose_digest))?;
    // The law lives IN the application's seed: a constitution names
    // groups the adopting entrypoint must declare (`organism`), and a
    // seed importing the app (its tests) must see both together.
    let holes = holes_of(&art);
    created(&mut out, &app.seed.join("dna_constitution.hl"), &constitution_hl(&app.main_name, &holes))?;
    if !holes.is_empty() {
        out.push(format!(
            "note    `organism_gated` is deferred in dna_constitution.hl: the baseline has {} unresolvable edge(s) ({})",
            holes.len(),
            holes.iter().take(3).cloned().collect::<Vec<_>>().join("; ")
        ));
    }
    // 5. the application's main gains the DNA
    let src = fs::read_to_string(&app.main_file).map_err(|e| e.to_string())?;
    match graft_main(&src, &app.main_name)? {
        Some(edited) => {
            fs::write(&app.main_file, edited).map_err(|e| e.to_string())?;
            out.push(format!(
                "edited  {} (imports, `genome` param, `adopt Project`, membrane bindings)",
                app.main_file.display()
            ));
        }
        None => out.push(format!("kept    {} (already carries the DNA)", app.main_file.display())),
    }
    // 6. the manifest's environments
    let mtext = fs::read_to_string(&manifest).map_err(|e| e.to_string())?;
    if !mtext.contains("[environments.") {
        let add = format!(
            "\n# hale dna init: the law every deployment target carries, and the\n# local target this entrypoint deploys to (`hale check --matrix`).\n[claims]\nbase = \"Project\"\n\n[environments.local]\nsource_only = true\nentrypoints = [\"{}\"]\n",
            app.seed_rel
        );
        fs::write(&manifest, format!("{}{}", mtext, add)).map_err(|e| e.to_string())?;
        out.push(format!("edited  {} ([claims] base, [environments.local])", manifest.display()));
    } else {
        out.push(format!("kept    {} (declares environments already)", manifest.display()));
    }
    // 7. the Journal, seeded from the artifact
    let journal = app.root.join(JOURNAL_REL);
    if journal.exists() {
        out.push(format!("kept    {} (a Journal exists; not reseeded)", journal.display()));
    } else {
        let n = seed_journal(&journal, &app, &art, &raw, &purpose_digest)?;
        out.push(format!("seeded  {} ({n} event(s): application.attached, structure.observed, responsibility.proposed, review.requested)", journal.display()));
    }
    // 8. .gitignore hygiene
    let gi = app.root.join(".gitignore");
    let mut gtext = fs::read_to_string(&gi).unwrap_or_default();
    let mut added = Vec::new();
    for line in ["/vendor/", "/.hale/dna/*.sock"] {
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
    out.push(format!("    hale check --matrix {}   # every entrypoint against Project", app.root.display()));
    out.push(format!("    hale dna run {}          # build, run under LOTUS_OBS, iris attached", app.root.display()));
    out.push("    the first Review (`purpose`) ratifies dna/purpose.hl — answer it in iris [m]".to_string());
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
fn run_organism(args: &[String]) -> ExitCode {
    let mut dir = PathBuf::from(".");
    let mut port = "8787".to_string();
    let mut iris = true;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--port" => match it.next() {
                Some(p) => port = p.clone(),
                None => {
                    eprintln!("hale dna run: --port needs a value");
                    return ExitCode::from(2);
                }
            },
            "--no-iris" => iris = false,
            f if f.starts_with("--") => {
                eprintln!("hale dna run: unknown flag `{f}`");
                return ExitCode::from(2);
            }
            p => dir = PathBuf::from(p),
        }
    }
    let (root, seed) = match project(&dir) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("hale dna run: {e}");
            return ExitCode::from(2);
        }
    };
    let me = match std::env::current_exe() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("hale dna run: {e}");
            return ExitCode::from(1);
        }
    };
    let dna_dir = root.join(".hale/dna");
    let _ = fs::create_dir_all(&dna_dir);
    // 1. a fresh artifact of what is about to run
    let current = dna_dir.join("current.topology");
    let st = Command::new(&me)
        .arg("check")
        .arg(&seed)
        .arg(format!("--dump-topology={}", current.display()))
        .stdout(std::process::Stdio::null())
        .status();
    if !matches!(st, Ok(s) if s.success()) {
        eprintln!("hale dna run: `hale check {}` failed; the organism is not run on a program that does not pass", seed.display());
        return ExitCode::from(1);
    }
    // 2. build
    let st = Command::new(&me).arg("build").arg(&seed).stdout(std::process::Stdio::null()).status();
    if !matches!(st, Ok(s) if s.success()) {
        eprintln!("hale dna run: `hale build {}` failed", seed.display());
        return ExitCode::from(1);
    }
    let bin_name = seed.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "app".into());
    let bin = seed.join(&bin_name);
    if !bin.is_file() {
        eprintln!("hale dna run: no binary at {}", bin.display());
        return ExitCode::from(1);
    }
    // 3. the organism, from the root, observable
    for sock in [dna_dir.join(crate::iris::MEMBRANE_VERDICT_SOCK), dna_dir.join(crate::iris::MEMBRANE_INTENT_SOCK)] {
        let _ = fs::remove_file(&sock);
    }
    let mut organism = match Command::new(&bin).current_dir(&root).env("LOTUS_OBS", "1").spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("hale dna run: cannot start {}: {e}", bin.display());
            return ExitCode::from(1);
        }
    };
    eprintln!("hale dna run: organism {} (pid {}) from {} under LOTUS_OBS=1", bin_name, organism.id(), root.display());
    // 4. the membrane comes up
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let bound = loop {
        let up = dna_dir.join(crate::iris::MEMBRANE_VERDICT_SOCK).exists() && dna_dir.join(crate::iris::MEMBRANE_INTENT_SOCK).exists();
        if up {
            break true;
        }
        if let Ok(Some(_)) = organism.try_wait() {
            break false;
        }
        if std::time::Instant::now() > deadline {
            break false;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    };
    let mut observer: Option<std::process::Child> = None;
    if bound {
        eprintln!("hale dna run: membrane bound at {}", dna_dir.display());
        if iris {
            let baseline = dna_dir.join("baseline.topology");
            let mut cmd = Command::new(&me);
            cmd.arg("iris").arg(&port).arg(&current);
            if baseline.is_file() {
                cmd.arg("--diff").arg(&baseline).arg(&current);
            }
            cmd.arg("--membrane").arg(&dna_dir);
            cmd.stdin(std::process::Stdio::null());
            match cmd.spawn() {
                Ok(c) => {
                    eprintln!("hale dna run: iris at http://127.0.0.1:{port}/  (l law · 4 review vs baseline · m membrane)");
                    observer = Some(c);
                }
                Err(e) => eprintln!("hale dna run: could not launch hale iris: {e}"),
            }
        }
    } else if organism.try_wait().ok().flatten().is_none() {
        eprintln!("hale dna run: the membrane did not come up within 20s; the organism runs unobserved");
    }
    // 5. supervise: the organism's exit is ours
    let code = match organism.wait() {
        Ok(st) => st.code().unwrap_or(1),
        Err(_) => 1,
    };
    if let Some(mut o) = observer {
        let _ = o.kill();
        let _ = o.wait();
    }
    eprintln!("hale dna run: organism exited ({code}); the Journal at {} is the record", root.join(JOURNAL_REL).display());
    ExitCode::from(code.clamp(0, 255) as u8)
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
    prev: String,
    digest: String,
}

fn read_journal(root: &Path) -> Result<Vec<Row>, String> {
    let p = root.join(JOURNAL_REL);
    let text = fs::read_to_string(&p).map_err(|e| format!("no Journal at {}: {e} (run `hale dna init`)", p.display()))?;
    let mut rows = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let v: Value = serde_json::from_str(line).map_err(|e| format!("{}:{}: not JSON: {e}", p.display(), i + 1))?;
        let s = |k: &str| v[k].as_str().unwrap_or("").to_string();
        rows.push(Row { seq: v["seq"].as_u64().unwrap_or(i as u64), kind: s("kind"), entity: s("entity"), body: s("body"), prev: s("prev"), digest: s("digest") });
    }
    Ok(rows)
}

/// The chain, verified exactly as `FileJournal.verify_chain` does.
fn chain_ok(rows: &[Row]) -> bool {
    let mut prev = "genesis".to_string();
    for r in rows {
        if r.prev != prev {
            return false;
        }
        let d = hex(&openssl::sha::sha256(format!("{}|{}|{}|{}", prev, r.kind, r.entity, r.body).as_bytes()));
        if d != r.digest {
            return false;
        }
        prev = r.digest.clone();
    }
    true
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
                reviews.insert(id.clone(), serde_json::json!({"id": id, "state": "pending", "question": b["question"], "required_authority": b["required_authority"], "subject_digest": b["subject_digest"], "refusals": []}));
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
    let mutations: Vec<Value> = rows
        .iter()
        .filter(|r| r.kind.starts_with("mutation."))
        .map(|r| serde_json::json!({"candidate": r.entity, "disposition": r.kind.trim_start_matches("mutation."), "class": r.body}))
        .collect();
    let deferred: Vec<Value> = rows.iter().filter(|r| r.kind == "law.deferred").map(body).collect();
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
        "journal": {"path": JOURNAL_REL, "revision": rows.len(), "chain": if chain_ok(&rows) { "verified" } else { "BROKEN" }},
        "expression": {"attached": attached, "current": current_id, "build_digest": build_digest, "toolchain": TOOLCHAIN},
        "intents": {"offered": intents_offered, "refused": intents_refused},
        "tasks": tasks.values().cloned().collect::<Vec<_>>(),
        "reviews": reviews.values().cloned().collect::<Vec<_>>(),
        "mutations": mutations,
        "law_deferred": deferred,
        "pressure": "not journaled in Phase 1",
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
    let muts = st["mutations"].as_array().map(|a| a.len()).unwrap_or(0);
    out.push(format!("mutations:  {} (every one stops at stage in Phase 1)", muts));
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
    let mut words: Vec<String> = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--to" => to = it.next().cloned().ok_or("--to needs a locus")?,
            f if f.starts_with("--") => return Err(format!("ask: unknown flag `{f}`")),
            w => words.push(w.to_string()),
        }
    }
    if words.is_empty() {
        return Err("ask: say what should happen — `hale dna ask \"<intent>\"`".into());
    }
    let outcome = words.join(" ");
    let (root, _) = project(Path::new("."))?;
    let rows_before = read_journal(&root)?.len();
    if !membrane_up(&root) {
        return Err(format!(
            "the organism is not running (no membrane under {}); start it with `hale dna run`. The Journal has {} event(s).",
            root.join(".hale/dna").display(),
            rows_before
        ));
    }
    let intent_id = format!("i{:x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0));
    let who = std::env::var("USER").unwrap_or_else(|_| "human".into());
    let body = serde_json::json!({"intent_id": intent_id, "outcome": outcome, "from": who, "to": to}).to_string();
    publish_on_membrane(&root, "intent", &body)?;
    // the organism's answer, from the Journal
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
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
            return Err(format!("published intent {intent_id}, but the organism journaled no answer within 10s (Journal revision {})", rows.len()));
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
}

/// Build (once, in the toolchain cache beside the core it imports)
/// and exec the membrane client with routes to this project's
/// sockets.
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
            "dna.review.verdict = unix://{}/{} : connect\ndna.intent.offered = unix://{}/{} : connect\n",
            dna_dir.display(),
            crate::iris::MEMBRANE_VERDICT_SOCK,
            dna_dir.display(),
            crate::iris::MEMBRANE_INTENT_SOCK
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
    let mut authority = "maintainer".to_string();
    let mut comment = String::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--as" => reviewer = it.next().cloned().ok_or("--as needs a reviewer")?,
            "--authority" => authority = it.next().cloned().ok_or("--authority needs a value")?,
            "--comment" => comment = it.next().cloned().ok_or("--comment needs text")?,
            f if f.starts_with("--") => return Err(format!("review: unknown flag `{f}`")),
            w => pos.push(w.to_string()),
        }
    }
    let (Some(id), Some(verdict)) = (pos.first().cloned(), pos.get(1).cloned()) else {
        return Err("review: `hale dna review <review_id> approve|revise|reject|abstain`".into());
    };
    if !["approve", "revise", "reject", "abstain"].contains(&verdict.as_str()) {
        return Err(format!("review: verdict `{verdict}` is not one of approve | revise | reject | abstain"));
    }
    let (root, _) = project(Path::new("."))?;
    let rows = read_journal(&root)?;
    let requested = rows
        .iter()
        .find(|r| r.kind == "review.requested" && (r.entity == format!("review:{id}") || r.entity == id))
        .ok_or_else(|| format!("no `review.requested` for `{id}` in the Journal (pending: {})", rows.iter().filter(|r| r.kind == "review.requested").map(|r| r.entity.trim_start_matches("review:").to_string()).collect::<Vec<_>>().join(", ")))?;
    let digest = serde_json::from_str::<Value>(&requested.body).ok().and_then(|b| b["subject_digest"].as_str().map(|s| s.to_string())).unwrap_or_default();
    if !membrane_up(&root) {
        return Err("the organism is not running (no membrane); start it with `hale dna run`".into());
    }
    let before = rows.len();
    let body = serde_json::json!({"review_id": id, "subject_digest": digest, "verdict": verdict, "reviewer": reviewer, "authority": authority, "comment": comment}).to_string();
    publish_on_membrane(&root, "verdict", &body)?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let rows = read_journal(&root)?;
        if let Some(r) = rows.iter().skip(before).find(|r| (r.kind == "review.settled" || r.kind == "review.refused") && r.entity == id) {
            return Ok(vec![if r.kind == "review.settled" { format!("review {id} settled: {}", r.body) } else { format!("review {id} refused the verdict: {}", r.body) }]);
        }
        if std::time::Instant::now() > deadline {
            return Err(format!("published the verdict, but the organism journaled no answer for `{id}` within 10s"));
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
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
    let chain = if chain_ok(&rows) { "verified" } else { "BROKEN" };
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
    let mut out = vec![format!("journal {} — {} event(s), chain {}", root.join(JOURNAL_REL).display(), rows.len(), chain)];
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
    let name = dir
        .canonicalize()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_else(|| "app".to_string());
    let locus = pascal(&name);
    let main_hl = format!(
        r#"/// {name} — a governed application. The entrypoint is a `main locus`
/// so the DNA can attach: its `genome` param is the assembly
/// (dna/assembly.hl), `Project` is its law (dna/constitution.hl),
/// and the membrane sockets are where a human's verdict or intent
/// enters. Every `.hl` in this directory shares one scope.
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
    let gitignore = format!("# the build artifact\n/{name}\n# toolchain-managed\n/vendor/\n/.hale/dna/*.sock\n");
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

fn assembly_hl(project: &str, purpose_digest: &str) -> String {
    format!(
        r#"// dna/assembly.hl — the Genome: this project's DNA assembly
// (project-owned; edit freely). Every "setting" is a constructor
// argument; `hale check` sees all of it.
//
// Phase 1 defaults: a file Journal (the causal authority), in-memory
// knowledge, a conservative autonomy grant, human review before any
// apply, NO deployment (every mutation stops at `staged`), and the
// local human membrane. Provenance of every fact here: declared.

import "vendor/dna" as dna;

locus Genome {{
    params {{
        core: dna::Dna = dna::Dna {{
            journal: dna::FileJournal {{ path: ".hale/dna/journal.jsonl" }},
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
            boundary: dna::AutonomyBoundary {{
                child: "{project}",
                grant: dna::Grant {{ child: "{project}", classes: "refactor docs", max_magnitude: 4, review: "pre" }}
            }},
            review_policy: dna::HumanBeforeApply {{ }},
            deployment: dna::NoDeployment {{ }},
            membrane: dna::LocalHumanMembrane {{ who: "operator" }}
        }};
        // The baseline review: ratify dna/purpose.hl. Settles only on a
        // maintainer's verdict naming this exact digest.
        purpose: dna::Review = dna::Review {{
            review_id: "purpose",
            question: "ratify the declared purpose?",
            subject_digest: "{purpose_digest}",
            required_authority: "maintainer",
            author: "hale dna init"
        }};
    }}

    fn status() -> String {{
        let b = std::json::Builder {{ }};
        b.begin_object();
        b.string_field("project", "{project}");
        b.field("core", self.core.status());
        b.string_field("purpose_review", if self.purpose.settled {{ self.purpose.outcome }} else {{ "pending" }});
        b.end_object();
        return b.result();
    }}
}}
"#
    )
}

/// The law. `holes` are the application's unresolvable edges from the
/// baseline artifact: a `forbid reaches(organism, …)` clause over an
/// application with a hole fails closed (an unresolvable edge is never
/// a pass), so the app-wide clause is generated ACTIVE only when the
/// baseline has none, and DEFERRED — written out, commented, with the
/// reason — otherwise. The assembly-scoped clauses hold regardless.
fn constitution_hl(main_name: &str, holes: &[String]) -> String {
    let organism_clause = if holes.is_empty() {
        "    // The application itself never reaches an apply except through the
    // gate (active: the baseline artifact has no unresolvable edges).
    organism_gated: forbid reaches(organism, effects(genome_apply)) avoiding dna_gate;
".to_string()
    } else {
        format!(
            "    // DEFERRED by `hale dna init`: the baseline has {} unresolvable
    // edge(s) ({}), and a reachability clause over the application
    // fails closed on them. Resolve the edges (or scope the group) and
    // uncomment:
    // organism_gated: forbid reaches(organism, effects(genome_apply)) avoiding dna_gate;
",
            holes.len(),
            holes.iter().take(3).cloned().collect::<Vec<_>>().join("; ")
        )
    };
    format!(
        r#"// dna_constitution.hl — the law (project-owned; generated by
// `hale dna init`). Adopted by the entrypoint (`adopt Project;`) and
// carried by every environment (`[claims] base` in hale.toml). Add
// clauses; do not weaken these.
//
// It lives in the application's own seed because a constitution
// names groups its adopting entrypoint must declare (`organism`),
// and because whoever imports the application (its tests) must see
// the law and its vocabulary together.

import "vendor/dna" as dna;
import "dna" as genome;

group organism = {{ {main_name} }};
group genome = {{ genome::Genome }};
group dna_gate = {{ dna::Dna }};
group performers = {{ dna::AgentPerformer, dna::HumanWorkGateway, dna::ServicePerformer, dna::ScriptedPerformer }};
group credentials = {{ dna::CredentialSource, dna::HostedCredential }};

constitution Project {{
    // A mutation is applied only THROUGH the assembly's gate, never by
    // a performer — and in Phase 1 the gate is NoDeployment, so nothing
    // applies at all.
    apply_gated: forbid reaches(genome, effects(genome_apply)) avoiding dna_gate;
    performers_never_apply: forbid reaches(performers, effects(genome_apply));
    credentials_sealed: require sealed(all credentials);
{organism_clause}}}
"#
    )
}

/// The application's unresolvable edges, as the artifact reports them.
fn holes_of(art: &Value) -> Vec<String> {
    art["unknowns"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|u| {
                    let f = u["fn"].as_str().unwrap_or("?");
                    let why: Vec<String> = u["reasons"]
                        .as_array()
                        .map(|r| r.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect())
                        .unwrap_or_default();
                    if why.is_empty() {
                        f.to_string()
                    } else {
                        format!("{f}: {}", why.join(", "))
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Graft the DNA onto the application's main file. Returns `None`
/// when it already carries it (idempotent), otherwise the edited
/// source. Edits are span-anchored insertions; nothing existing is
/// rewritten.
fn graft_main(src: &str, main_name: &str) -> Result<Option<String>, String> {
    use hale_syntax::ast::{LocusMember, TopDecl};
    if src.contains("genome::Genome") {
        return Ok(None);
    }
    let prog = hale_syntax::parse_source(src).map_err(|d| {
        format!(
            "the application does not parse: {}",
            d.first().map(|x| x.message.clone()).unwrap_or_default()
        )
    })?;
    let mut inserts: Vec<(usize, String)> = Vec::new();
    // imports
    let last_import_end = prog.imports.iter().map(|i| i.span.end.as_usize()).max();
    let mut import_text = String::new();
    if !prog.imports.iter().any(|i| i.alias.as_deref() == Some("dna")) {
        import_text.push_str("import \"vendor/dna\" as dna;\n");
    }
    if !prog.imports.iter().any(|i| i.alias.as_deref() == Some("genome")) {
        import_text.push_str("import \"dna\" as genome;\n");
    }
    match last_import_end {
        Some(e) => {
            let e = src[e..].find('\n').map(|n| e + n + 1).unwrap_or(e);
            inserts.push((e, import_text));
        }
        None => inserts.push((0, format!("{import_text}\n"))),
    }
    // the main locus
    let Some(TopDecl::Locus(main)) = prog.items.iter().find(|i| matches!(i, TopDecl::Locus(l) if l.is_main)) else {
        return Err("no main locus".into());
    };
    // Missing blocks go at the END of the main's body, before its
    // closing brace — after whatever the author wrote.
    let close_main = block_close(src, main.span.end.as_usize())?;
    // An insertion inside an existing block keeps the author's
    // layout: a one-line block stays one line, a multi-line block
    // gets its own line.
    let inside = |src: &str, block_start: usize, close: usize, line: &str| -> String {
        if src[block_start..close].contains('\n') {
            format!("    {line}\n    ")
        } else {
            format!(" {line} ")
        }
    };
    let mut has_params = false;
    let mut has_claims = false;
    let mut has_bindings = false;
    for m in &main.members {
        match m {
            LocusMember::Params(pb) => {
                has_params = true;
                let close = block_close(src, pb.span.end.as_usize())?;
                inserts.push((close, inside(src, pb.span.start.as_usize(), close, "genome: genome::Genome = genome::Genome { };")));
            }
            LocusMember::Claims(cb) => {
                has_claims = true;
                if !cb.adopts.iter().any(|a| a.name == "Project") {
                    let close = block_close(src, cb.span.end.as_usize())?;
                    inserts.push((close, inside(src, cb.span.start.as_usize(), close, "adopt Project;")));
                }
            }
            LocusMember::Bindings(bb) => {
                has_bindings = true;
                let close = block_close(src, bb.span.end.as_usize())?;
                let line = format!("dna::ReviewVerdict: unix(\"{VERDICT_SOCK_REL}\", role: listen); dna::IntentOffered: unix(\"{INTENT_SOCK_REL}\", role: listen);");
                inserts.push((close, inside(src, bb.span.start.as_usize(), close, &line)));
            }
            _ => {}
        }
    }
    let mut blocks = String::new();
    if !has_params {
        blocks.push_str("\n    params {\n        genome: genome::Genome = genome::Genome { };\n    }");
    }
    if !has_claims {
        blocks.push_str("\n    claims {\n        adopt Project;\n    }");
    }
    if !has_bindings {
        blocks.push_str(&format!(
            "\n    bindings {{\n        dna::ReviewVerdict: unix(\"{VERDICT_SOCK_REL}\", role: listen);\n        dna::IntentOffered: unix(\"{INTENT_SOCK_REL}\", role: listen);\n    }}"
        ));
    }
    if !blocks.is_empty() {
        // `blocks` starts with a newline; the brace it lands before is
        // on its own line, so the body ends "...}\n<blocks>\n}".
        blocks.push('\n');
        inserts.push((close_main, blocks));
    }
    let _ = main_name;
    inserts.sort_by(|a, b| b.0.cmp(&a.0));
    let mut out = src.to_string();
    for (at, text) in inserts {
        out.insert_str(at, &text);
    }
    Ok(Some(out))
}

/// The byte offset of a block's closing `}` given the span end the
/// parser recorded (which may sit on or just past the brace).
fn block_close(src: &str, span_end: usize) -> Result<usize, String> {
    let end = span_end.min(src.len());
    src[..end].rfind('}').ok_or_else(|| "block without a closing brace".to_string())
}

// ---------------------------------------------------------------
// the Journal, seeded from the artifact
// ---------------------------------------------------------------

struct Chain {
    lines: Vec<String>,
    prev: String,
}

impl Chain {
    fn new() -> Self {
        Chain { lines: Vec::new(), prev: "genesis".to_string() }
    }
    /// Exactly `FileJournal.append`: digest = sha256(prev|kind|entity|body).
    fn push(&mut self, kind: &str, entity: &str, body: &str) {
        let seq = self.lines.len();
        let d = hex(&openssl::sha::sha256(format!("{}|{}|{}|{}", self.prev, kind, entity, body).as_bytes()));
        let line = serde_json::json!({
            "seq": seq, "kind": kind, "entity": entity, "body": body, "prev": self.prev, "digest": d
        });
        self.lines.push(line.to_string());
        self.prev = d;
    }
}

fn seed_journal(path: &Path, app: &App, art: &Value, raw: &str, purpose_digest: &str) -> Result<usize, String> {
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
    for h in holes_of(art) {
        c.push(
            "law.deferred",
            "claim:organism_gated",
            &serde_json::json!({"clause": "forbid reaches(organism, effects(genome_apply)) avoiding dna_gate", "reason": h, "provenance": "inferred"}).to_string(),
        );
    }
    c.push(
        "review.requested",
        "review:purpose",
        &serde_json::json!({
            "question": "ratify the declared purpose?", "subject_digest": purpose_digest,
            "required_authority": "maintainer", "author": "hale dna init",
            "baseline": {"artifact_digest": s(&art["artifact_digest"]), "bytes": raw.len()},
            "provenance": "declared"
        })
        .to_string(),
    );
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut text = c.lines.join("\n");
    text.push('\n');
    fs::write(path, text).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(c.lines.len())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

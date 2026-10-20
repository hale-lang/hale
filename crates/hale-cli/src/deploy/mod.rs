//! GH #1477: `hale deploy render <path> --backend compose|kubernetes
//! [--env <name>] [--out <dir>]` — a program's own placement elaborated
//! into a platform description.
//!
//! A pure function of what the program already says: its checked topology
//! (`--dump-topology`: scrape endpoint, bindings), its `main` locus's
//! `std::observe::Probes` param, the fleet plan when the path is one
//! (instances, routes), and `hale.toml`'s `[environments.<name>]` (the
//! backend, the environment values). Nothing is invented: a program that
//! declares no probes renders no probe section. It never builds, applies
//! or contacts anything, and it renders only the programs the caller names
//! (an organism's own), never a heart's.

mod compose;
mod kubernetes;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use hale_syntax::ast::{flat_decls, Expr, Literal, LocusMember, ParamInit, TopDecl};

/// What the runtime serves on a `std::observe::Probes` port.
pub(crate) const LIVENESS_PATH: &str = "/healthz";
pub(crate) const READINESS_PATH: &str = "/readyz";

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Scrape {
    pub port: i64,
    pub path: String,
}

/// One program (a seed, or an artifact a plan names) and what it declares.
#[derive(Debug, Clone)]
pub(crate) struct Prog {
    /// The workload's name: the seed's directory, or the lone instance's id.
    pub name: String,
    pub probes_port: Option<i64>,
    pub scrape: Option<Scrape>,
}

#[derive(Debug, Clone)]
pub(crate) struct Inst {
    pub id: String,
    /// The built binary, relative to the render's base directory.
    pub binary: Option<String>,
    pub prog: usize,
}

pub(crate) struct Model {
    /// Where the binaries are, for the mount: the seed's or the plan's directory.
    pub base: PathBuf,
    pub env_name: Option<String>,
    pub env: BTreeMap<String, String>,
    pub progs: Vec<Prog>,
    /// Sorted by id.
    pub insts: Vec<Inst>,
    /// (publisher instance, subscriber instance), distinct, sorted.
    pub edges: Vec<(String, String)>,
    pub source: String,
}

const USAGE: &str = "\
hale deploy render <path> --backend compose|kubernetes [--env <name>] [--out <dir>]

  <path>: a seed (a directory or .hl file), or a fleet plan (.json)
  --backend: compose | kubernetes; else `backend` of [environments.<name>]
  --env: the [environments.<name>] of the nearest hale.toml: its
    `backend` and its `[environments.<name>.env]` values
  --out: write compose.yaml / kubernetes.yaml into <dir>; else stdout

A pure function of the checked program: it never builds or deploys, and it
renders no probe, scrape or scale section the program does not declare.
";

pub(crate) fn usage() -> &'static str {
    USAGE
}

pub(crate) fn run_deploy(rest: &[String]) -> ExitCode {
    if rest.first().map(String::as_str) != Some("render") {
        eprint!("{USAGE}");
        return ExitCode::from(2);
    }
    let mut path: Option<&str> = None;
    let (mut backend, mut env, mut out) = (None, None, None);
    let mut i = 1;
    while i < rest.len() {
        let a = rest[i].as_str();
        let mut val = |name: &str| -> Option<String> {
            i += 1;
            let v = rest.get(i).cloned();
            if v.is_none() {
                eprintln!("{name} takes a value");
            }
            v
        };
        match a {
            "--help" | "-h" => {
                print!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            "--backend" => match val("--backend") {
                Some(v) => backend = Some(v),
                None => return ExitCode::from(2),
            },
            "--env" => match val("--env") {
                Some(v) => env = Some(v),
                None => return ExitCode::from(2),
            },
            "--out" => match val("--out") {
                Some(v) => out = Some(PathBuf::from(v)),
                None => return ExitCode::from(2),
            },
            _ if a.starts_with('-') => {
                eprintln!("unknown flag `{a}`\n{USAGE}");
                return ExitCode::from(2);
            }
            _ if path.is_none() => path = Some(a),
            _ => {
                eprintln!("one path: `{a}` is a second\n{USAGE}");
                return ExitCode::from(2);
            }
        }
        i += 1;
    }
    let Some(path) = path else {
        eprint!("{USAGE}");
        return ExitCode::from(2);
    };
    let mut notes = Vec::new();
    let (model, env_backend) = match gather(Path::new(path), env.as_deref(), &mut notes) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(1);
        }
    };
    let backend = backend.or(env_backend);
    let dest = match &out {
        Some(d) => d.clone(),
        None => PathBuf::from("."),
    };
    let (file, text) = match backend.as_deref() {
        Some("compose") => (
            "compose.yaml",
            Ok(compose::render(&model, &dest, &mut notes)),
        ),
        Some("kubernetes") => ("kubernetes.yaml", kubernetes::render(&model, &mut notes)),
        Some(other) => {
            eprintln!("unknown backend `{other}`: compose or kubernetes");
            return ExitCode::from(2);
        }
        None => {
            eprintln!("no backend: pass --backend compose|kubernetes, or set `backend` in [environments.<name>] and pass --env");
            return ExitCode::from(2);
        }
    };
    let text = match text {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(1);
        }
    };
    for n in &notes {
        eprintln!("note: {n}");
    }
    match out {
        Some(d) => {
            if let Err(e) =
                std::fs::create_dir_all(&d).and_then(|_| std::fs::write(d.join(file), &text))
            {
                eprintln!("write {}: {e}", d.join(file).display());
                return ExitCode::from(1);
            }
            eprintln!("rendered: {}", d.join(file).display());
        }
        None => print!("{text}"),
    }
    ExitCode::SUCCESS
}

/// The text of `p` relative to `from` (both made absolute), `.` when equal.
pub(crate) fn relative(from: &Path, p: &Path) -> String {
    let abs = |x: &Path| {
        let x = if x.is_absolute() {
            x.to_path_buf()
        } else {
            std::env::current_dir().unwrap_or_default().join(x)
        };
        x.canonicalize().unwrap_or(x)
    };
    let (f, t) = (abs(from), abs(p));
    let fc: Vec<_> = f.components().collect();
    let tc: Vec<_> = t.components().collect();
    let common = fc.iter().zip(&tc).take_while(|(a, b)| a == b).count();
    let mut parts: Vec<String> = fc[common..].iter().map(|_| "..".to_string()).collect();
    parts.extend(
        tc[common..]
            .iter()
            .map(|c| c.as_os_str().to_string_lossy().into_owned()),
    );
    if parts.is_empty() {
        ".".into()
    } else {
        parts.join("/")
    }
}

pub(crate) fn q(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_default()
}

/// A DNS-1123-ish name: lowercase alphanumerics and `-`.
pub(crate) fn dns(s: &str) -> String {
    let mut o: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    while o.starts_with('-') {
        o.remove(0);
    }
    while o.ends_with('-') {
        o.pop();
    }
    o
}

fn gather(
    path: &Path,
    env: Option<&str>,
    notes: &mut Vec<String>,
) -> Result<(Model, Option<String>), String> {
    let is_plan = path.extension().and_then(|e| e.to_str()) == Some("json");
    let anchor = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent().map(Path::to_path_buf).unwrap_or_default()
    };
    let anchor = if anchor.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        anchor
    };
    // the environment: the nearest hale.toml at or above the path
    let (mut env_vals, mut env_backend) = (BTreeMap::new(), None);
    if let Some(name) = env {
        let manifest = nearest_manifest(&anchor).ok_or_else(|| {
            format!(
                "--env {name}: no hale.toml at or above {}",
                anchor.display()
            )
        })?;
        let (envs, _) = crate::pkg::read_claims_config(&manifest)?;
        let spec = envs.get(name).ok_or_else(|| {
            format!(
                "{}: no `[environments.{name}]` (declared: {})",
                manifest.display(),
                envs.keys().cloned().collect::<Vec<_>>().join(", ")
            )
        })?;
        env_vals = spec.env.clone();
        env_backend = spec.backend.clone();
    }
    let mut progs: Vec<Prog> = Vec::new();
    let mut insts: Vec<Inst> = Vec::new();
    let mut edges: Vec<(String, String)> = Vec::new();
    let base;
    if is_plan {
        let plan = crate::fleet::read_plan(path).map_err(|e| e.join("\n"))?;
        base = anchor.clone();
        let mut by_key: BTreeMap<String, usize> = BTreeMap::new();
        for inst in &plan.instances {
            let key = inst.seed.clone().unwrap_or_else(|| inst.artifact.clone());
            let idx = match by_key.get(&key) {
                Some(i) => *i,
                None => {
                    let p = if let Some(seed) = &inst.seed {
                        let dir = anchor.join(seed);
                        seed_prog(&dir, env, notes)?
                    } else {
                        let facts = topology_facts(&anchor.join(&inst.artifact), true)?;
                        notes.push(format!(
                            "{}: an artifact carries no Probes param, so no probe section is rendered for it",
                            inst.id
                        ));
                        Prog {
                            name: inst.id.clone(),
                            probes_port: None,
                            scrape: facts,
                        }
                    };
                    progs.push(p);
                    by_key.insert(key, progs.len() - 1);
                    progs.len() - 1
                }
            };
            let binary = match (&inst.binary, &inst.seed) {
                (Some(b), _) => Some(b.clone()),
                (None, Some(seed)) => Some(format!(
                    "{}/{}",
                    seed.trim_end_matches('/'),
                    dir_name(&anchor.join(seed))
                )),
                _ => None,
            };
            insts.push(Inst {
                id: inst.id.clone(),
                binary,
                prog: idx,
            });
        }
        for r in &plan.routes {
            for p in &r.publishers {
                for s in &r.subscribers {
                    if p.instance != s.instance {
                        edges.push((p.instance.clone(), s.instance.clone()));
                    }
                }
            }
        }
    } else {
        if path.is_dir() && !has_hl(path) {
            return Err(format!(
                "{}: no .hl source here. `render` takes the program of an organism's own: a seed, or a fleet plan \
                 naming its seeds; a repository's heart is deployed by its owner, never from here",
                path.display()
            ));
        }
        base = if path.is_dir() {
            path.to_path_buf()
        } else {
            anchor.clone()
        };
        let p = seed_prog(path, env, notes)?;
        let binary = Some(if path.is_dir() {
            dir_name(path)
        } else {
            path.file_stem().unwrap().to_string_lossy().into_owned()
        });
        insts.push(Inst {
            id: p.name.clone(),
            binary,
            prog: 0,
        });
        progs.push(p);
    }
    // a workload with several instances is named for its program, one alone for its instance
    for (i, p) in progs.iter_mut().enumerate() {
        let mine: Vec<&Inst> = insts.iter().filter(|x| x.prog == i).collect();
        if mine.len() == 1 {
            p.name = mine[0].id.clone();
        }
    }
    insts.sort_by(|a, b| a.id.cmp(&b.id));
    edges.sort();
    edges.dedup();
    Ok((
        Model {
            base,
            env_name: env.map(String::from),
            env: env_vals,
            progs,
            insts,
            edges,
            source: path
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
        },
        env_backend,
    ))
}

fn dir_name(p: &Path) -> String {
    p.file_name()
        .or_else(|| None)
        .map(|s| s.to_string_lossy().into_owned())
        .or_else(|| {
            p.canonicalize()
                .ok()
                .and_then(|c| c.file_name().map(|s| s.to_string_lossy().into_owned()))
        })
        .unwrap_or_else(|| "main".into())
}

fn has_hl(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .any(|e| e.path().extension().and_then(|s| s.to_str()) == Some("hl"))
        })
        .unwrap_or(false)
}

fn nearest_manifest(from: &Path) -> Option<PathBuf> {
    let mut d = from.canonicalize().ok()?;
    loop {
        let m = d.join("hale.toml");
        if m.is_file() {
            return Some(m);
        }
        d = d.parent()?.to_path_buf();
    }
}

/// A seed's facts: the scrape endpoint from its checked topology, the probes
/// from its main locus.
fn seed_prog(seed: &Path, env: Option<&str>, notes: &mut Vec<String>) -> Result<Prog, String> {
    let name = dir_name(seed);
    let tmp = std::env::temp_dir().join(format!(
        "hale_deploy_{}_{}.topology.json",
        std::process::id(),
        dns(&name)
    ));
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let r = std::process::Command::new(me)
        .arg("check")
        .arg(seed)
        .args(env.into_iter().flat_map(|e| ["--env", e]))
        .arg(format!("--dump-topology={}", tmp.display()))
        .output()
        .map_err(|e| format!("hale check {}: {e}", seed.display()))?;
    if !r.status.success() {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!(
            "{} does not check, so there is nothing to render:\n{}{}",
            seed.display(),
            String::from_utf8_lossy(&r.stdout).trim(),
            String::from_utf8_lossy(&r.stderr).trim()
        ));
    }
    let scrape = topology_facts(&tmp, false);
    note_local_bindings(&tmp, &name, notes);
    let _ = std::fs::remove_file(&tmp);
    let scrape = scrape?;
    let probes_port = probes_of(seed, &name, notes);
    Ok(Prog {
        name,
        probes_port,
        scrape,
    })
}

/// A binding over a unix socket or a shared-memory ring names a local path,
/// not a port: it is not published, and containers that mean to share it
/// need a shared volume, which is the operator's.
fn note_local_bindings(artifact: &Path, name: &str, notes: &mut Vec<String>) {
    let Some(v) = std::fs::read_to_string(artifact)
        .ok()
        .and_then(|r| serde_json::from_str::<serde_json::Value>(&r).ok())
    else {
        return;
    };
    let mut topics: Vec<String> = v["bindings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|b| matches!(b["transport"].as_str(), Some("unix" | "shm_ring")))
        .filter_map(|b| b["topic"].as_str().map(String::from))
        .collect();
    topics.sort();
    topics.dedup();
    if !topics.is_empty() {
        notes.push(format!(
            "{name}: binding(s) {} are local paths, not ports: nothing is published for them",
            topics.join(", ")
        ));
    }
}

/// The scrape endpoint of an artifact's `observe` rows (schema 1.21), when
/// the source gave a literal port.
fn topology_facts(artifact: &Path, _plan: bool) -> Result<Option<Scrape>, String> {
    let raw = std::fs::read_to_string(artifact)
        .map_err(|e| format!("read {}: {e}", artifact.display()))?;
    let v: serde_json::Value =
        serde_json::from_str(&raw).map_err(|e| format!("parse {}: {e}", artifact.display()))?;
    let row = v["observe"]
        .as_array()
        .and_then(|rows| rows.iter().find(|r| r["kind"] == "metrics"));
    Ok(row.and_then(|r| {
        Some(Scrape {
            port: r["port"].as_i64()?,
            path: r["path"].as_str().unwrap_or("/metrics").to_string(),
        })
    }))
}

/// The probes' port from the seed's main locus: the literal `port`, a
/// `self.<param>` whose declared default is a literal (the port the program
/// runs on when nothing overrides that param), or the declared default
/// (9090) when the construction names none. A port the source computes is
/// not known here, and renders nothing.
fn probes_of(seed: &Path, name: &str, notes: &mut Vec<String>) -> Option<i64> {
    let files: Vec<PathBuf> = if seed.is_dir() {
        let mut f: Vec<PathBuf> = std::fs::read_dir(seed)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("hl"))
            .filter(|p| !hale_frontend::frontend::is_test_file(p))
            .collect();
        f.sort();
        f
    } else {
        vec![seed.to_path_buf()]
    };
    let mut found: Option<Option<i64>> = None;
    for f in files {
        let Some(prog) = std::fs::read_to_string(&f)
            .ok()
            .and_then(|s| hale_syntax::parse_source(&s).ok())
        else {
            continue;
        };
        for d in flat_decls(&prog.items) {
            let TopDecl::Locus(l) = d else { continue };
            if !l.is_main {
                continue;
            }
            for m in &l.members {
                let LocusMember::Params(pb) = m else { continue };
                let literal_default = |param: &str| {
                    pb.params
                        .iter()
                        .find(|q| q.name.name == param)
                        .and_then(|q| match &q.init {
                            ParamInit::Value(Expr::Literal(Literal::Int(n), _)) => Some(*n as i64),
                            _ => None,
                        })
                };
                for p in &pb.params {
                    if !hale_types::probes::is_probes_param(p) {
                        continue;
                    }
                    found = Some(match &p.init {
                        ParamInit::Value(Expr::Struct { inits, .. }) => {
                            match inits.iter().find(|i| i.name.name == "port") {
                                None => Some(9090),
                                Some(i) => match &i.value {
                                    Expr::Literal(Literal::Int(n), _) => Some(*n as i64),
                                    Expr::Field { receiver, name, .. }
                                        if matches!(receiver.as_ref(), Expr::KwSelf(_)) =>
                                    {
                                        literal_default(&name.name)
                                    }
                                    _ => None,
                                },
                            }
                        }
                        _ => None,
                    });
                }
            }
        }
    }
    match found {
        Some(Some(p)) => Some(p),
        Some(None) => {
            notes.push(format!("{name}: the Probes port is computed by the source, not known here: no probe section is rendered"));
            None
        }
        None => {
            notes.push(format!("{name}: the main locus holds no std::observe::Probes: no healthcheck or probe is rendered"));
            None
        }
    }
}

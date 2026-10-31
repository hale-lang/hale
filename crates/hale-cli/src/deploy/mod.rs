//! GH #1477: `hale deploy render <path> --backend compose|kubernetes
//! [--env <name>] [--placement <file>] [--out <dir>]` — a program's own
//! placement elaborated into a platform description.
//!
//! A pure function of what the program already says: its checked topology
//! (`--dump-topology`: scrape endpoint, bindings), its `main` locus's
//! `std::observe::Probes` param, a fleet seed's `deployment` rows when the
//! path is one (instances, their placement defaults, the derived routes),
//! the allocation input that overrides those defaults per instance
//! (`--placement`), and `hale.toml`'s `[environments.<name>]` (the
//! backend, the environment values). A fleet renders one description per
//! host, since each host is applied on its own. Nothing is invented: a program that
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

/// A route of the fleet as the render declares it: its endpoints by
/// instance and the host each runs on, so a route between two hosts'
/// projects is named in both.
#[derive(Debug, Clone)]
pub(crate) struct RouteDecl {
    pub id: String,
    pub subject: String,
    pub transport: String,
    pub publishers: Vec<(String, String)>,
    pub subscribers: Vec<(String, String)>,
}

pub(crate) struct Model {
    /// The host this description is for (a fleet's), `None` for a seed.
    pub host: Option<String>,
    /// The fleet's routes that touch this host.
    pub routes: Vec<RouteDecl>,
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
hale deploy render <path> --backend compose|kubernetes [--env <name>] [--placement <file>] [--out <dir>]

  <path>: a seed (a directory or .hl file), or a fleet seed (one declaring
    `fleet locus`), which renders one description per host
  --backend: compose | kubernetes; else `backend` of [environments.<name>]
  --env: the [environments.<name>] of the nearest hale.toml: its
    `backend` and its `[environments.<name>.env]` values
  --placement: a fleet's allocation rows, a JSON array of
    {\"instance\", \"host\"} (with \"provider\" and \"role\" when the habitat
    exports them): each instance's host, over its placement default
  --out: write compose.yaml / kubernetes.yaml into <dir> (a fleet's into
    <dir>/<host>/); else stdout, a fleet's hosts as YAML documents

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
    let (mut backend, mut env, mut out, mut placement) = (None, None, None, None);
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
            "--placement" => match val("--placement") {
                Some(v) => placement = Some(PathBuf::from(v)),
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
    let (models, env_backend) = match gather(Path::new(path), env.as_deref(), placement.as_deref(), &mut notes) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(1);
        }
    };
    let backend = backend.or(env_backend);
    let file = match backend.as_deref() {
        Some("compose") => "compose.yaml",
        Some("kubernetes") => "kubernetes.yaml",
        Some(other) => {
            eprintln!("unknown backend `{other}`: compose or kubernetes");
            return ExitCode::from(2);
        }
        None => {
            eprintln!("no backend: pass --backend compose|kubernetes, or set `backend` in [environments.<name>] and pass --env");
            return ExitCode::from(2);
        }
    };
    let mut rendered: Vec<(Option<String>, String)> = Vec::new();
    for model in &models {
        // a fleet's host renders into its own directory
        let dest = match (&out, &model.host) {
            (Some(d), Some(h)) => d.join(h),
            (Some(d), None) => d.clone(),
            (None, _) => PathBuf::from("."),
        };
        let text = if file == "compose.yaml" {
            Ok(compose::render(model, &dest, &mut notes))
        } else {
            kubernetes::render(model, &mut notes)
        };
        match text {
            Ok(t) => rendered.push((model.host.clone(), t)),
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::from(1);
            }
        }
    }
    for n in &notes {
        eprintln!("note: {n}");
    }
    match out {
        Some(d) => {
            for (host, text) in &rendered {
                let dir = match host {
                    Some(h) => d.join(h),
                    None => d.clone(),
                };
                if let Err(e) = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(dir.join(file), text)) {
                    eprintln!("write {}: {e}", dir.join(file).display());
                    return ExitCode::from(1);
                }
                eprintln!("rendered: {}", dir.join(file).display());
            }
        }
        None => {
            for (i, (_, text)) in rendered.iter().enumerate() {
                if i > 0 {
                    println!("---");
                }
                print!("{text}");
            }
        }
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

/// The routes of a fleet that touch a host, as lines of declared
/// endpoints (`instance@host`), for a description's header.
pub(crate) fn route_lines(m: &Model) -> Vec<String> {
    let ends = |v: &[(String, String)]| v.iter().map(|(i, h)| format!("{i}@{h}")).collect::<Vec<_>>().join(", ");
    m.routes
        .iter()
        .map(|r| format!("route {} ({}, {}): {} -> {}", r.id, r.subject, r.transport, ends(&r.publishers), ends(&r.subscribers)))
        .collect()
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

/// One row of the allocation input: the host an instance runs on. The
/// habitat's allocation rows export `provider` and `role` beside them;
/// the render places by host alone.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Allocation {
    instance: String,
    host: String,
    #[serde(default)]
    #[allow(dead_code)]
    provider: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    role: Option<String>,
}

fn read_placement(p: &Path) -> Result<BTreeMap<String, String>, String> {
    let raw = std::fs::read_to_string(p).map_err(|e| format!("read {}: {e}", p.display()))?;
    let rows: Vec<Allocation> = serde_json::from_str(&raw).map_err(|e| format!("parse {}: {e}", p.display()))?;
    let mut out = BTreeMap::new();
    for r in rows {
        if !hale_types::fleet::host_name_ok(&r.host) {
            return Err(format!(
                "{}: instance `{}` is allocated to `{}`, which names no host: a host is one name of \
                 letters, digits, `.`, `_` and `-`, never a path (each host renders into a directory \
                 of its name)",
                p.display(),
                r.instance,
                r.host
            ));
        }
        if out.insert(r.instance.clone(), r.host).is_some() {
            return Err(format!("{}: instance `{}` is allocated twice", p.display(), r.instance));
        }
    }
    Ok(out)
}

/// The deepest directory holding every path.
fn common_dir(paths: &[PathBuf]) -> PathBuf {
    let abs: Vec<PathBuf> = paths.iter().map(|p| p.canonicalize().unwrap_or_else(|_| p.clone())).collect();
    let mut common: Vec<std::path::Component> = match abs.first() {
        Some(p) => p.components().collect(),
        None => return PathBuf::from("."),
    };
    for p in &abs[1..] {
        let c: Vec<_> = p.components().collect();
        let n = common.iter().zip(&c).take_while(|(a, b)| a == b).count();
        common.truncate(n);
    }
    common.iter().collect()
}

fn gather(
    path: &Path,
    env: Option<&str>,
    placement: Option<&Path>,
    notes: &mut Vec<String>,
) -> Result<(Vec<Model>, Option<String>), String> {
    let fleet = crate::deployment::is_fleet_seed(path);
    if placement.is_some() && !fleet {
        return Err("--placement places a fleet's instances: the path is no fleet seed".to_string());
    }
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
    let source = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    if !fleet {
        if path.is_dir() && !has_hl(path) {
            return Err(format!(
                "{}: no .hl source here. `render` takes the program of an organism's own: a seed, or a fleet seed \
                 naming its seeds; a repository's heart is deployed by its owner, never from here",
                path.display()
            ));
        }
        let base = if path.is_dir() {
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
        let insts = vec![Inst { id: p.name.clone(), binary, prog: 0 }];
        let model = Model {
            host: None,
            routes: Vec::new(),
            base,
            env_name: env.map(String::from),
            env: env_vals,
            progs: vec![p],
            insts,
            edges: Vec::new(),
            source,
        };
        return Ok((vec![model], env_backend));
    }

    // A fleet seed: its rows, each instance on the host its allocation
    // names, else its placement default.
    let rows = crate::deployment::derive_deployment(path).map_err(|e| e.join("\n"))?;
    let allocated = match placement {
        Some(p) => read_placement(p)?,
        None => BTreeMap::new(),
    };
    for id in allocated.keys() {
        if !rows.instances.iter().any(|i| &i.id == id) {
            return Err(format!("--placement allocates `{id}`, which fleet `{}` does not deploy", rows.fleet));
        }
    }
    let host_of = |id: &str, default: &Option<String>| -> String {
        allocated
            .get(id)
            .cloned()
            .or_else(|| default.clone())
            .unwrap_or_else(|| "local".to_string())
    };
    let unplaced: Vec<&str> = rows
        .instances
        .iter()
        .filter(|i| i.host.is_none() && !allocated.contains_key(&i.id))
        .map(|i| i.id.as_str())
        .collect();
    if !unplaced.is_empty() {
        notes.push(format!(
            "{}: no host in the source or the allocation, rendered for the host `local`",
            unplaced.join(", ")
        ));
    }
    let mut dirs: Vec<PathBuf> = vec![rows.root.clone()];
    dirs.extend(rows.components.iter().map(|c| c.seed.clone()));
    let base = common_dir(&dirs);
    let mut progs: Vec<Prog> = Vec::new();
    for c in &rows.components {
        let mut p = seed_prog(&c.seed, env, notes)?;
        p.name = c.param.clone();
        progs.push(p);
    }
    let mut placed: Vec<(String, Inst)> = Vec::new();
    for i in &rows.instances {
        let Some(pi) = rows.components.iter().position(|c| c.param == i.component) else { continue };
        let seed = &rows.components[pi].seed;
        let binary = Some(format!("{}/{}", relative(&base, seed), dir_name(seed)));
        placed.push((host_of(&i.id, &i.host), Inst { id: i.id.clone(), binary, prog: pi }));
    }
    let host_by_id: BTreeMap<String, String> = placed.iter().map(|(h, i)| (i.id.clone(), h.clone())).collect();
    let routes: Vec<RouteDecl> = rows
        .routes
        .iter()
        .map(|r| {
            let ends = |v: &[(String, String)]| {
                v.iter().map(|(i, _)| (i.clone(), host_by_id.get(i).cloned().unwrap_or_default())).collect::<Vec<_>>()
            };
            RouteDecl {
                id: r.id.clone(),
                subject: r.subject.clone(),
                transport: r.transport.clone(),
                publishers: ends(&r.publishers),
                subscribers: ends(&r.subscribers),
            }
        })
        .collect();
    let mut hosts: Vec<String> = placed.iter().map(|(h, _)| h.clone()).collect();
    hosts.sort();
    hosts.dedup();
    // each host renders into a directory of its name: never a path
    if let Some(bad) = hosts.iter().find(|h| !hale_types::fleet::host_name_ok(h)) {
        return Err(format!("`{bad}` names no host: a host is one name, never a path"));
    }
    let mut models = Vec::new();
    for h in hosts {
        // the programs with an instance on this host, re-indexed
        let mine: Vec<&Inst> = placed.iter().filter(|(x, _)| *x == h).map(|(_, i)| i).collect();
        let mut keep: Vec<usize> = mine.iter().map(|i| i.prog).collect();
        keep.sort();
        keep.dedup();
        let mut hprogs: Vec<Prog> = keep.iter().map(|k| progs[*k].clone()).collect();
        let mut insts: Vec<Inst> = mine
            .iter()
            .map(|i| Inst {
                id: i.id.clone(),
                binary: i.binary.clone(),
                prog: keep.iter().position(|k| *k == i.prog).unwrap_or(0),
            })
            .collect();
        // a workload with several instances is named for its program, one alone for its instance
        for (pi, p) in hprogs.iter_mut().enumerate() {
            let only: Vec<&Inst> = insts.iter().filter(|x| x.prog == pi).collect();
            if only.len() == 1 {
                p.name = only[0].id.clone();
            }
        }
        insts.sort_by(|a, b| a.id.cmp(&b.id));
        // in-host edges order the services; a route that leaves the host is declared
        let mut edges: Vec<(String, String)> = Vec::new();
        for r in &routes {
            for (p, ph) in &r.publishers {
                for (sub, sh) in &r.subscribers {
                    if *ph == h && *sh == h && p != sub {
                        edges.push((p.clone(), sub.clone()));
                    }
                }
            }
        }
        edges.sort();
        edges.dedup();
        let touching: Vec<RouteDecl> = routes
            .iter()
            .filter(|r| r.publishers.iter().chain(&r.subscribers).any(|(_, x)| *x == h))
            .cloned()
            .collect();
        models.push(Model {
            host: Some(h),
            routes: touching,
            base: base.clone(),
            env_name: env.map(String::from),
            env: env_vals.clone(),
            progs: hprogs,
            insts,
            edges,
            source: source.clone(),
        });
    }
    Ok((models, env_backend))
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
    let scrape = topology_facts(&tmp);
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
fn topology_facts(artifact: &Path) -> Result<Option<Scrape>, String> {
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

//! GH #566 F5 — `hale node <name>`: the agent that expresses, on one
//! machine, the instances a fleet plan assigns to that node.
//!
//! A node owns a clone of the governed repository and nothing else.
//! Every tick it syncs the record, reads the latest `fleet.deploy`
//! row, and — when that names a revision it does not express yet —
//! fetches the revision (`refs/dna/revisions/<rev>`, pushed by whoever
//! deployed), checks it out, and for each of its instances the deploy
//! touches (or that is not running) cuts the artifact, builds the
//! seed, restarts the process, and appends `instance.up` in the node's
//! name: node, instance, revision, model hash, build digest, pid. An
//! instance that exits appends `instance.exited` with its code. That
//! is the expression identity per instance: reported by the node that
//! expresses it, joined to the plan by instance id, judged by the host
//! beside the organization over the window.
//!
//! The node never decides anything. It expresses what the record says
//! and reports what it sees.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use serde_json::Value;

use crate::dna::{append_journal_as, git, normalize, read_journal, record_remote, shape_of, short, sync_record, terminate};

struct Running {
    child: std::process::Child,
    revision: String,
}

pub fn run(args: &[String]) -> ExitCode {
    let mut name: Option<String> = None;
    let mut repo = PathBuf::from(".");
    let mut fleet: Option<String> = None;
    let mut tick_ms: u64 = 1000;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--repo" => match it.next() {
                Some(r) => repo = PathBuf::from(r),
                None => return usage("--repo needs a directory"),
            },
            "--fleet" => match it.next() {
                Some(f) => fleet = Some(f.clone()),
                None => return usage("--fleet needs a name"),
            },
            "--tick" => match it.next().and_then(|v| v.parse::<u64>().ok()) {
                Some(n) => tick_ms = n.max(100),
                None => return usage("--tick needs milliseconds"),
            },
            f if f.starts_with("--") => return usage(&format!("unknown flag `{f}`")),
            n => name = Some(n.to_string()),
        }
    }
    let Some(name) = name else { return usage("a node has a name") };
    let root = match repo.canonicalize() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("hale node {name}: {}: {e}", repo.display());
            return ExitCode::from(2);
        }
    };
    if git(&root, &["rev-parse", "--git-dir"]).is_err() {
        eprintln!("hale node {name}: {} is not a git clone of the governed repository", root.display());
        return ExitCode::from(2);
    }
    let me = match std::env::current_exe() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("hale node {name}: {e}");
            return ExitCode::from(1);
        }
    };
    let node_dir = root.join(".hale/node").join(&name);
    let _ = fs::create_dir_all(&node_dir);
    let author = format!("node/{name}");
    eprintln!("hale node {name}: expressing from {} (record via {})", root.display(), record_remote(&root).unwrap_or_else(|| "no remote".into()));
    let mut running: BTreeMap<String, Running> = BTreeMap::new();
    let mut expressed_seq: u64 = 0;
    loop {
        if let Err(e) = sync_record(&root) {
            eprintln!("hale node {name}: sync: {e}");
        }
        // what exited since the last tick
        let mut gone: Vec<String> = Vec::new();
        for (id, r) in running.iter_mut() {
            if let Ok(Some(st)) = r.child.try_wait() {
                let code = st.code().unwrap_or(-1);
                eprintln!("hale node {name}: instance {id} exited ({code})");
                let body = serde_json::json!({"node": name, "instance": id, "revision": r.revision, "code": code}).to_string();
                let _ = append_journal_as(&root, "instance.exited", id, &body, Some(&author));
                let _ = fs::remove_file(node_dir.join(format!("{id}.pid")));
                gone.push(id.clone());
            }
        }
        for id in &gone {
            running.remove(id);
        }
        if !gone.is_empty() {
            let _ = sync_record(&root);
        }
        // the latest deploy for our fleet
        let rows = read_journal(&root).unwrap_or_default();
        let deploy = rows.iter().rev().find(|r| r.kind == "fleet.deploy").and_then(|r| serde_json::from_str::<Value>(&r.body).ok().map(|b| (r.seq, b))).filter(|(_, b)| fleet.as_deref().map(|f| b["plan"].as_str() == Some(f)).unwrap_or(true));
        if let Some((seq, body)) = deploy {
            if seq != expressed_seq {
                match express(&me, &root, &name, &node_dir, &author, &body, &mut running) {
                    Ok(n) => {
                        expressed_seq = seq;
                        eprintln!("hale node {name}: fleet.deploy #{seq} expressed ({n} instance(s) started)");
                        let _ = sync_record(&root);
                    }
                    Err(e) => eprintln!("hale node {name}: fleet.deploy #{seq}: {e} (retrying next tick)"),
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(tick_ms));
    }
}

fn usage(why: &str) -> ExitCode {
    eprintln!("hale node: {why}");
    eprintln!("usage: hale node <name> [--repo <clone>] [--fleet <name>] [--tick <ms>]");
    eprintln!("       run the instances a fleet plan assigns to node <name>, from the record;");
    eprintln!("       the clone's `origin` (or `dna.remote`) is where the record and revisions come from");
    ExitCode::from(2)
}

/// Express one deploy row: the revision checked out, every instance of
/// this node the deploy touches (or that is not up) rebuilt and
/// restarted at it. Returns how many were started.
fn express(me: &Path, root: &Path, node: &str, node_dir: &Path, author: &str, deploy: &Value, running: &mut BTreeMap<String, Running>) -> Result<usize, String> {
    let rev = deploy["revision"].as_str().unwrap_or("").to_string();
    if rev.is_empty() {
        return Err("a deploy with no revision".into());
    }
    if let Some(remote) = record_remote(root) {
        let _ = git(root, &["fetch", "-q", &remote, "+refs/dna/revisions/*:refs/dna/revisions/*"]);
    }
    git(root, &["rev-parse", "--verify", &format!("{rev}^{{commit}}")]).map_err(|_| format!("revision {} is not here yet", short(&rev)))?;
    git(root, &["checkout", "-q", "--detach", &rev])?;
    let plan_name = deploy["plan"].as_str().unwrap_or("");
    let plan_path = match crate::pkg::read_dna_fleet(&root.join("hale.toml"))? {
        Some((n, p)) if n == plan_name || plan_name.is_empty() => p,
        _ => {
            let fleets = crate::pkg::read_fleets(&root.join("hale.toml"))?;
            match fleets.get(plan_name) {
                Some(rel) => root.join(rel),
                None => return Err(format!("no fleet `{plan_name}` in hale.toml at {}", short(&rev))),
            }
        }
    };
    let plan = crate::fleet::read_plan(&plan_path).map_err(|e| e.join("; "))?;
    let plan_dir = plan_path.parent().unwrap_or(root).to_path_buf();
    let touched: Vec<String> = deploy["touched"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect()).unwrap_or_default();
    let mut started = 0;
    for inst in plan.instances.iter().filter(|i| i.node.as_deref() == Some(node)) {
        let up = running.get(&inst.id).map(|r| r.revision == rev).unwrap_or(false);
        if up && !touched.contains(&inst.id) {
            continue;
        }
        let Some(seed) = inst.seed.as_deref() else {
            eprintln!("hale node {node}: instance {} has no seed to build; skipped", inst.id);
            continue;
        };
        let seed_dir = normalize(&plan_dir.join(seed));
        let artifact = node_dir.join(format!("{}.topology", inst.id));
        let bin = build(me, &seed_dir, &artifact).map_err(|e| format!("instance {}: {e}", inst.id))?;
        let shape = shape_of(&artifact);
        let build_digest = crate::sign::sha256_file(&bin).map(|d| d[..12].to_string()).unwrap_or_default();
        if let Some(mut old) = running.remove(&inst.id) {
            terminate(&mut old.child);
        }
        let child = Command::new(&bin)
            .current_dir(root)
            .env("LOTUS_OBS", "1")
            .env("HALE_BIN", me)
            .env("HALE_DNA_NODE", node)
            .env("HALE_DNA_INSTANCE", &inst.id)
            .env("HALE_DNA_EXPRESSION", format!("{shape} build {build_digest}"))
            .spawn()
            .map_err(|e| format!("instance {}: cannot start {}: {e}", inst.id, bin.display()))?;
        let _ = fs::write(node_dir.join(format!("{}.pid", inst.id)), child.id().to_string());
        let body = serde_json::json!({"node": node, "instance": inst.id, "revision": rev, "model_hash": shape, "build": build_digest, "pid": child.id()}).to_string();
        append_journal_as(root, "instance.up", &inst.id, &body, Some(author))?;
        eprintln!("hale node {node}: instance {} up (pid {}) at {} as {}", inst.id, child.id(), short(&rev), short(&shape));
        running.insert(inst.id.clone(), Running { child, revision: rev.clone() });
        started += 1;
    }
    Ok(started)
}

/// Cut the artifact and build the seed; the binary.
fn build(me: &Path, seed: &Path, artifact: &Path) -> Result<PathBuf, String> {
    let st = Command::new(me).arg("check").arg(seed).arg(format!("--dump-topology={}", artifact.display())).stdout(std::process::Stdio::null()).status();
    if !matches!(st, Ok(s) if s.success()) {
        return Err(format!("`hale check {}` failed; nothing runs that does not pass", seed.display()));
    }
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

//! `hale habitat down --forget`: the habitat removed whole, planned first
//! (phase 4 ruling: destroy is plan-then-confirm, like terraform).
//!
//! One auditor computes the plan and the destroy step executes exactly its
//! items, one by one: what is printed is what is done. The auditor is the
//! habitat's record side (`habitat-forget-plan`, dna/host/forget.hl: its
//! record and memory, its organisms, the vault slots, its Forgejo, its
//! hosts and what it runs on each over SSH) and this machine's side here
//! (the body and face to stop, the compose projects and the volumes no
//! record item names, the habitat's directory, its systemd units, and the
//! notes an earlier forget kept). Nothing is touched until the person types
//! the habitat's name back at a terminal; there is no other way to confirm.
//! An execution that fails stops at that item and says which were done and
//! which were not; run again, the plan is made again from what remains.
//!
//! The one thing the habitat cannot remove is the line each host's
//! `~/.ssh/authorized_keys` took: the forget keeps a note of it beside the
//! habitat's directory, and the next plan shows it until a forget clears it.

use std::collections::BTreeMap;
use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use serde_json::{json, Value};

use super::{
    body_state, docker, face_running, genome_dir, local_habitat_dir, local_habitat_project,
    name_file, stop_processes, unit_path, Docker, DEFAULT_HABITAT_NAME,
};
use crate::dna::{host_run, manifest_pin};

/// The notes a forget keeps of what it could not remove (the authorized key
/// lines), a JSON object a line: `name`, `host`, `line`, `what`.
const NOTES: &str = "forgotten-notes.jsonl";

/// The groups of a plan, in the order they are printed, and their headings.
const GROUPS: [(&str, &str); 7] = [
    ("habitat", "the habitat's record and memory"),
    ("organism", "the grafted organisms"),
    ("compose", "the compose projects and their volumes"),
    ("vault", "the vault (slot names only)"),
    ("forge", "the habitat's Forgejo and its accounts"),
    ("hosts", "the hosts"),
    ("remote", "on the hosts, over SSH"),
];

/// The actions the habitat's record side executes (a host verb), and the
/// ones executed here.
const REMOTE_ACTIONS: [&str; 3] = ["remote-withdraw", "remote-project-down", "remote-dir-rm"];

fn item(
    group: &str,
    subject: &str,
    what: String,
    measure: String,
    recoverable: &str,
    action: &str,
    args: String,
    step: i64,
) -> Value {
    json!({ "group": group, "subject": subject, "what": what, "measure": measure, "recoverable": recoverable, "action": action, "host": "", "args": args, "step": step })
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or("")
}

fn notes_path() -> PathBuf {
    PathBuf::from(local_habitat_dir()).join(NOTES)
}

fn notes() -> Vec<Value> {
    std::fs::read_to_string(notes_path())
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// The habitat's name: the one it was born with, else the one its notes
/// keep, else `local`.
fn habitat_name() -> String {
    std::fs::read_to_string(name_file())
        .ok()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .or_else(|| {
            notes()
                .first()
                .map(|n| s(n, "name").to_string())
                .filter(|n| !n.is_empty())
        })
        .unwrap_or_else(|| DEFAULT_HABITAT_NAME.to_string())
}

/// What this machine's docker holds of the habitat's projects: per project,
/// its containers and networks, and every compose volume with its size.
#[derive(Default)]
struct Held {
    // per project, its containers' and networks' listing lines, sorted
    containers: BTreeMap<String, Vec<String>>,
    networks: BTreeMap<String, Vec<String>>,
    volumes: BTreeMap<String, (String, String)>, // name -> (project, size)
}

fn listed(args: &[&str]) -> Result<String, String> {
    match docker(args) {
        Docker::Answered(out) => Ok(out),
        Docker::Absent => Ok(String::new()),
        Docker::Failed(why) => Err(why),
    }
}

fn held() -> Result<Held, String> {
    let mut h = Held::default();
    for line in listed(&[
        "ps",
        "-a",
        "--filter",
        "label=com.docker.compose.project",
        "--format",
        "{{.Label \"com.docker.compose.project\"}}|{{.Label \"com.docker.compose.service\"}}|{{.Names}}",
    ])?
    .lines()
    .filter(|l| !l.is_empty())
    {
        let mut w = line.splitn(3, '|');
        let (Some(project), Some(service), Some(name)) = (w.next(), w.next(), w.next()) else { continue };
        h.containers.entry(project.to_string()).or_default().push(format!("c|{project}|{service}|{name}"));
    }
    for line in listed(&[
        "network",
        "ls",
        "--filter",
        "label=com.docker.compose.project",
        "--format",
        "{{.Label \"com.docker.compose.project\"}}|{{.Name}}",
    ])?
    .lines()
    .filter(|l| !l.is_empty())
    {
        let Some((project, name)) = line.split_once('|') else {
            continue;
        };
        h.networks
            .entry(project.to_string())
            .or_default()
            .push(format!("n|{project}|{name}"));
    }
    for v in h.containers.values_mut().chain(h.networks.values_mut()) {
        v.sort();
    }
    let sizes = listed(&["system", "df", "-v", "--format", "{{json .Volumes}}"])?;
    let vols: Vec<Value> = serde_json::from_str(sizes.trim()).unwrap_or_default();
    for v in vols {
        let labels = s(&v, "Labels");
        let Some(project) = labels
            .split(',')
            .find_map(|l| l.strip_prefix("com.docker.compose.project="))
        else {
            continue;
        };
        h.volumes.insert(
            s(&v, "Name").to_string(),
            (project.to_string(), s(&v, "Size").to_string()),
        );
    }
    Ok(h)
}

/// The size of the files under `path`, in bytes, and how many entries.
fn du(path: &Path) -> (u64, u64) {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return (0, 0);
    };
    if !meta.is_dir() {
        return (meta.len(), 1);
    }
    let mut total = (0, 1);
    if let Ok(es) = std::fs::read_dir(path) {
        for e in es.flatten() {
            let (b, n) = du(&e.path());
            total.0 += b;
            total.1 += n;
        }
    }
    total
}

fn bytes(n: u64) -> String {
    match n {
        n if n >= 1 << 30 => format!("{:.1} GB", n as f64 / (1u64 << 30) as f64),
        n if n >= 1 << 20 => format!("{:.1} MB", n as f64 / (1u64 << 20) as f64),
        n if n >= 1 << 10 => format!("{:.1} kB", n as f64 / (1u64 << 10) as f64),
        n => format!("{n} B"),
    }
}

/// Whether `project` is one of this machine's habitat projects: the
/// habitat's own, or one named after it (an organism's services), never one
/// a host's items name (an `ssh` host's projects, which a host on this
/// machine's own docker would otherwise list twice).
fn ours(project: &str, remote: &[String]) -> bool {
    let base = local_habitat_project();
    (project == base || project.starts_with(&format!("{base}-")))
        && !remote
            .iter()
            .any(|r| project == r || project.starts_with(&format!("{r}-")))
}

/// The plan, and the memories started to read it.
struct Plan {
    items: Vec<Value>,
    started: Vec<String>,
}

/// The memories of the habitat's project that are stopped (a `down` stops
/// them): the habitat's and each organism's on this machine, started so
/// their records can be read for the plan, and waited for. Nothing else is
/// touched; they are stopped again when nothing is removed.
fn start_memories() -> Vec<String> {
    let project = local_habitat_project();
    let out = listed(&[
        "ps",
        "-a",
        "--filter",
        &format!("label=com.docker.compose.project={project}"),
        "--format",
        "{{.ID}} {{.State}} {{.Label \"com.docker.compose.service\"}}",
    ])
    .unwrap_or_default();
    let stopped: Vec<String> = out
        .lines()
        .filter_map(|l| {
            let mut w = l.split(' ');
            let (id, state, service) = (w.next()?, w.next()?, w.next()?);
            (state != "running" && service.starts_with("memory-")).then(|| id.to_string())
        })
        .collect();
    let mut started = Vec::new();
    for id in stopped {
        if let Docker::Answered(_) = docker(&["start", &id]) {
            started.push(id);
        }
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(90);
    for id in &started {
        while std::time::Instant::now() < deadline {
            let health = listed(&[
                "inspect",
                "--format",
                "{{if .State.Health}}{{.State.Health.Status}}{{else}}{{.State.Status}}{{end}}",
                id,
            ])
            .unwrap_or_default();
            if matches!(health.trim(), "healthy" | "running") {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    }
    started
}

/// The record side of the plan, asked of the habitat's genome.
fn record_items() -> Result<Vec<Value>, String> {
    let genome = genome_dir();
    if manifest_pin(&genome.join("hale.toml"))
        .ok()
        .flatten()
        .is_none()
    {
        return Ok(Vec::new());
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let mut asked = host_run("habitat-forget-plan", &genome, &[]);
    while asked.is_err() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(1000));
        asked = host_run("habitat-forget-plan", &genome, &[]);
    }
    let text = asked.map_err(|e| {
        format!("the habitat's record cannot be read, and the plan is made from it: {e}")
    })?;
    serde_json::from_str::<Vec<Value>>(text.trim())
        .map_err(|e| format!("the habitat's plan did not parse: {e}"))
}

fn audit() -> Result<Plan, String> {
    let started = if manifest_pin(&genome_dir().join("hale.toml"))
        .ok()
        .flatten()
        .is_some()
    {
        start_memories()
    } else {
        Vec::new()
    };
    let mut items = match record_items() {
        Ok(items) => items,
        Err(why) => {
            settle(&started);
            return Err(why);
        }
    };
    let held = match held() {
        Ok(h) => h,
        Err(why) => {
            settle(&started);
            return Err(why);
        }
    };
    // the projects the hosts' items name are theirs
    let remote: Vec<String> = items
        .iter()
        .filter(|i| s(i, "group") == "remote" || s(i, "group") == "hosts")
        .map(|i| s(i, "subject"))
        .filter(|h| !h.is_empty() && *h != "local")
        .map(|h| format!("{}-{}", local_habitat_project(), h.replace('.', "-")))
        .chain(
            items
                .iter()
                .filter(|i| REMOTE_ACTIONS.contains(&s(i, "action")))
                .filter_map(|i| s(i, "args").lines().next().map(str::to_string)),
        )
        .collect();

    // the body and the face, stopped first
    let body = body_state();
    let face = face_running();
    if body.is_some() || face.is_some() {
        let what = [
            body.map(|b| format!("its body, {b}")),
            face.map(|f| format!("its face, {f}")),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join("; ");
        items.push(item(
            "habitat",
            "",
            format!("the habitat's processes stopped: {what}"),
            "processes".into(),
            "yes: `hale habitat up` starts them",
            "stop",
            String::new(),
            10,
        ));
    }

    // the volumes the record's items name, measured; the rest of the
    // habitat's projects here, theirs
    let mut claimed: Vec<String> = Vec::new();
    for i in items.iter_mut() {
        match s(i, "action") {
            "volume-rm" => {
                let names: Vec<String> = s(i, "args").lines().map(str::to_string).collect();
                let sizes: Vec<String> = names
                    .iter()
                    .map(|n| {
                        held.volumes
                            .get(n)
                            .map(|(_, size)| size.clone())
                            .unwrap_or_else(|| "none here".into())
                    })
                    .collect();
                let measure = format!("{} ({})", s(i, "measure"), sizes.join(", "));
                i["measure"] = json!(measure);
                claimed.extend(names);
            }
            // a project's item names its resources: its volumes are its
            "project-rm" => claimed.extend(
                s(i, "args")
                    .lines()
                    .filter_map(|l| l.strip_prefix("v|"))
                    .filter_map(|l| l.split_once('|').map(|(_, n)| n.to_string())),
            ),
            _ => {}
        }
    }
    let covered: Vec<String> = items
        .iter()
        .filter(|i| s(i, "action") == "project-rm")
        .filter_map(|i| s(i, "args").lines().next().map(str::to_string))
        .collect();
    let mut projects: Vec<&String> = held
        .containers
        .keys()
        .chain(held.networks.keys())
        .filter(|p| ours(p, &remote) && !covered.contains(p))
        .collect();
    projects.sort();
    projects.dedup();
    for p in projects {
        // its containers and networks by name: exactly what is removed
        let c = held.containers.get(p).cloned().unwrap_or_default();
        let n = held.networks.get(p).cloned().unwrap_or_default();
        items.push(item(
            "compose",
            "",
            format!("compose project {p}: its containers and networks removed"),
            format!("{} container(s), {} network(s)", c.len(), n.len()),
            "yes: `hale habitat up` makes them again",
            "containers-rm",
            std::iter::once(p.clone())
                .chain(c)
                .chain(n)
                .collect::<Vec<_>>()
                .join("\n"),
            40,
        ));
    }
    for (name, (project, size)) in held
        .volumes
        .iter()
        .filter(|(n, (p, _))| ours(p, &remote) && !claimed.contains(n))
    {
        items.push(item(
            "compose",
            "",
            format!("volume {name} of {project}, deleted"),
            format!("1 volume ({size})"),
            "no: its data is deleted",
            "volume-rm",
            name.clone(),
            50,
        ));
    }

    // this machine's files: the habitat's directory and its units
    let dir = PathBuf::from(local_habitat_dir());
    let entries: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map(|es| {
            es.flatten()
                .map(|e| e.path())
                .filter(|p| p.file_name().is_some_and(|n| n != NOTES))
                .collect()
        })
        .unwrap_or_default();
    if !entries.is_empty() {
        let (b, n) = entries
            .iter()
            .map(|p| du(p))
            .fold((0, 0), |a, x| (a.0 + x.0, a.1 + x.1));
        items.push(item(
            "habitat",
            "",
            format!("the habitat's directory {}: its genome repository, the allocations' files and secrets files, its saplings, logs and state", dir.display()),
            format!("{} in {n} entries", bytes(b)),
            "no: `hale habitat up` births a new habitat, with a new record",
            "dir-clear",
            dir.display().to_string(),
            70,
        ));
    }
    let units: Vec<String> = [
        unit_path(),
        unit_path().with_file_name(super::face_unit_name()),
    ]
    .iter()
    .filter(|p| p.is_file())
    .map(|p| p.display().to_string())
    .collect();
    if !units.is_empty() {
        items.push(item(
            "habitat",
            "",
            format!(
                "its systemd user units, {}: disabled, their files removed",
                units.join(", ")
            ),
            format!("{} file(s)", units.len()),
            "yes: `hale habitat up` writes them again",
            "unit-rm",
            units.join("\n"),
            70,
        ));
    }

    // the notes an earlier forget kept
    for n in notes() {
        items.push(item(
            "hosts",
            s(&n, "host"),
            format!(
                "noted when the habitat was forgotten: {}; the note is cleared",
                s(&n, "what")
            ),
            "1 note".into(),
            "-",
            "note-clear",
            serde_json::to_string(&n).unwrap_or_default(),
            85,
        ));
    }
    Ok(Plan { items, started })
}

/// The plan, printed in its groups.
fn print_plan(items: &[Value], name: &str) {
    println!("the plan to forget the habitat `{name}` (nothing is touched until its name is typed back):");
    for (group, heading) in GROUPS {
        let of: Vec<&Value> = items.iter().filter(|i| s(i, "group") == group).collect();
        if of.is_empty() {
            continue;
        }
        println!();
        println!("{heading}");
        let mut subject = None;
        for i in of {
            let sub = s(i, "subject");
            if (group == "organism" || group == "remote") && subject != Some(sub) {
                println!("  {}{sub}", if group == "remote" { "on " } else { "" });
                subject = Some(sub);
            }
            let indent = if group == "organism" || group == "remote" {
                "    "
            } else {
                "  "
            };
            println!("{indent}- {}", line_of(i));
        }
    }
    let acts = items
        .iter()
        .filter(|i| !matches!(s(i, "action"), "note" | "covered"))
        .count();
    println!();
    println!(
        "{} item(s): {acts} to stop or remove, {} noted or removed with another",
        items.len(),
        items.len() - acts
    );
}

fn line_of(i: &Value) -> String {
    let rec = s(i, "recoverable");
    let measure = s(i, "measure");
    let tail = match (measure, rec) {
        ("" | "-", "" | "-") => String::new(),
        ("" | "-", r) => format!(" (recoverable: {r})"),
        (m, "" | "-") => format!(" ({m})"),
        (m, r) => format!(" ({m}; recoverable: {r})"),
    };
    format!("{}{tail}", s(i, "what"))
}

/// One item executed here: Ok once done, Err why not.
fn execute_local(i: &Value, name: &str) -> Result<(), String> {
    let args: Vec<&str> = s(i, "args").lines().collect();
    let gone = |a: &[&str]| match docker(a) {
        Docker::Failed(why) => Err(why),
        _ => Ok(()),
    };
    match s(i, "action") {
        "note" | "covered" => Ok(()),
        "stop" => stop_processes(&mut Vec::new()),
        "containers-rm" | "project-rm" => {
            // exactly the resources the plan audited, by name: containers,
            // then volumes, then networks; one already gone is no failure
            for kind in ["c|", "v|", "n|"] {
                for line in args.iter().skip(1).filter(|l| l.starts_with(kind)) {
                    let name = line.rsplit('|').next().unwrap_or("");
                    if name.is_empty() || name.starts_with('-') {
                        return Err(format!("`{name}` is no resource's name"));
                    }
                    let (remove, inspect): (Vec<&str>, Vec<&str>) = match kind {
                        "c|" => (vec!["rm", "-f", name], vec!["container", "inspect", name]),
                        "v|" => (
                            vec!["volume", "rm", "-f", name],
                            vec!["volume", "inspect", name],
                        ),
                        _ => (
                            vec!["network", "rm", name],
                            vec!["network", "inspect", name],
                        ),
                    };
                    if let Err(why) = gone(&remove) {
                        if matches!(docker(&inspect), Docker::Answered(_)) {
                            return Err(why);
                        }
                    }
                }
            }
            Ok(())
        }
        "volume-rm" => {
            for v in args {
                gone(&["volume", "rm", "-f", v])?;
            }
            Ok(())
        }
        "vault-rm" => {
            let dir = super::super::vault_local_dir()
                .ok_or("the vault is at HALE_VAULT_ADDR, provisioned out of band")?;
            for slot in args {
                if slot.is_empty() || slot.contains('/') || slot.starts_with('.') {
                    return Err(format!("`{slot}` is no vault slot's name"));
                }
                match std::fs::remove_file(dir.join(slot)) {
                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                        return Err(format!("remove the vault slot {slot}: {e}"))
                    }
                    _ => {}
                }
            }
            Ok(())
        }
        "dir-rm" => {
            let path = PathBuf::from(args.first().copied().unwrap_or(""));
            if !path.starts_with(local_habitat_dir()) || path == PathBuf::from(local_habitat_dir())
            {
                return Err(format!("{} is not the habitat's to remove", path.display()));
            }
            remove_all(&path)
        }
        "dir-clear" => {
            let dir = PathBuf::from(local_habitat_dir());
            for e in std::fs::read_dir(&dir)
                .map_err(|e| format!("read {}: {e}", dir.display()))?
                .flatten()
            {
                if e.file_name() != NOTES {
                    remove_all(&e.path())?;
                }
            }
            Ok(())
        }
        "unit-rm" => {
            for path in args {
                let unit = Path::new(path)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                let _ = Command::new("systemctl")
                    .args(["--user", "disable", "--now"])
                    .arg(&unit)
                    .output();
                match std::fs::remove_file(path) {
                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                        return Err(format!("remove {path}: {e}"))
                    }
                    _ => {}
                }
            }
            let _ = Command::new("systemctl")
                .args(["--user", "daemon-reload"])
                .output();
            Ok(())
        }
        "note-keep" | "unreachable" => {
            let path = notes_path();
            std::fs::create_dir_all(PathBuf::from(local_habitat_dir()))
                .map_err(|e| format!("create {}: {e}", local_habitat_dir()))?;
            let note = json!({ "name": name, "host": s(i, "host"), "line": s(i, "args"), "what": s(i, "what") });
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .map_err(|e| format!("write {}: {e}", path.display()))?;
            writeln!(f, "{note}").map_err(|e| format!("write {}: {e}", path.display()))
        }
        "note-clear" => {
            let keep: Vec<String> = notes()
                .iter()
                .map(|n| n.to_string())
                .filter(|n| *n != s(i, "args"))
                .collect();
            let path = notes_path();
            if keep.is_empty() {
                let _ = std::fs::remove_file(&path);
                let _ = std::fs::remove_dir(local_habitat_dir());
                Ok(())
            } else {
                std::fs::write(&path, keep.join("\n") + "\n")
                    .map_err(|e| format!("write {}: {e}", path.display()))
            }
        }
        other => Err(format!("`{other}` is no action of this machine's")),
    }
}

/// `path` removed, a directory with everything in it; gone already is done.
fn remove_all(path: &Path) -> Result<(), String> {
    let r = if std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir()) {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    };
    match r {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            Err(format!("remove {}: {e}", path.display()))
        }
        _ => Ok(()),
    }
}

/// The items that run on the hosts, executed by the habitat's record side
/// in one go: each one's outcome, in order.
fn execute_remote(batch: &[&Value]) -> Vec<(String, Option<String>)> {
    let file = PathBuf::from(local_habitat_dir()).join(".forget-plan.json");
    let plan = Value::Array(batch.iter().map(|v| (*v).clone()).collect());
    let ran = std::fs::write(&file, plan.to_string())
        .map_err(|e| format!("write {}: {e}", file.display()))
        .and_then(|()| {
            host_run(
                "habitat-teardown-run",
                &genome_dir(),
                &[file.display().to_string()],
            )
        });
    let _ = std::fs::remove_file(&file);
    let report: Vec<Value> = match ran.and_then(|t| {
        serde_json::from_str(t.trim()).map_err(|e| format!("the report did not parse: {e}"))
    }) {
        Ok(r) => r,
        Err(why) => {
            return batch
                .iter()
                .enumerate()
                .map(|(n, _)| ("failed".to_string(), (n == 0).then(|| why.clone())))
                .collect()
        }
    };
    report
        .iter()
        .map(|r| {
            (
                s(r, "state").to_string(),
                Some(s(r, "why").to_string()).filter(|w| !w.is_empty()),
            )
        })
        .collect()
}

/// The plan executed item by item, in its steps' order, stopping at the
/// first that fails. Whether all of it was done.
fn execute(items: &[Value], name: &str) -> bool {
    let mut order: Vec<&Value> = items.iter().collect();
    order.sort_by_key(|i| i["step"].as_i64().unwrap_or(0));
    let mut failed = false;
    let mut k = 0;
    while k < order.len() {
        if failed {
            println!("{:<10}{}", "not done", s(order[k], "what"));
            k += 1;
            continue;
        }
        if REMOTE_ACTIONS.contains(&s(order[k], "action")) {
            let mut end = k;
            while end < order.len() && REMOTE_ACTIONS.contains(&s(order[end], "action")) {
                end += 1;
            }
            let outcomes = execute_remote(&order[k..end]);
            for (n, i) in order[k..end].iter().enumerate() {
                match outcomes.get(n) {
                    Some((state, _)) if state == "done" => {
                        println!("{:<10}{}", "done", s(i, "what"))
                    }
                    Some((state, Some(why))) if state == "failed" => {
                        println!("{:<10}{}: {why}", "FAILED", s(i, "what"));
                        failed = true;
                    }
                    _ => {
                        println!("{:<10}{}", "not done", s(i, "what"));
                        failed = true;
                    }
                }
            }
            k = end;
            continue;
        }
        match execute_local(order[k], name) {
            Ok(()) => println!("{:<10}{}", "done", s(order[k], "what")),
            Err(why) => {
                println!("{:<10}{}: {why}", "FAILED", s(order[k], "what"));
                failed = true;
            }
        }
        k += 1;
    }
    !failed
}

/// What a confirmation binds a plan by: each item's group, subject,
/// action, host, arguments and recoverability, in order (its sizes, counts
/// and wording left out: a volume grows while the person reads), as the
/// rows' `dna::teardown_digest` binds theirs.
fn bound(items: &[Value]) -> Vec<[String; 6]> {
    items
        .iter()
        .map(|i| {
            ["group", "subject", "action", "host", "args", "recoverable"]
                .map(|k| s(i, k).to_string())
        })
        .collect()
}

/// The memories started only to read the records, stopped again.
fn settle(started: &[String]) {
    for id in started {
        let _ = docker(&["stop", id]);
    }
}

pub(super) fn forget(plan_only: bool) -> ExitCode {
    if !plan_only && !std::io::stdin().is_terminal() {
        eprintln!("hale habitat down --forget: stdin is not a terminal; forgetting the habitat removes its record, its organisms and their data, and is confirmed only by typing the habitat's name at a terminal (`--plan` prints the plan)");
        return ExitCode::from(2);
    }
    let name = habitat_name();
    let plan = match audit() {
        Ok(p) => p,
        Err(why) => {
            eprintln!("hale habitat down --forget: {why}");
            return ExitCode::from(1);
        }
    };
    if plan.items.is_empty() {
        settle(&plan.started);
        println!("nothing to forget: the habitat `{name}` holds nothing on this machine");
        return ExitCode::SUCCESS;
    }
    print_plan(&plan.items, &name);
    if !plan.started.is_empty() {
        println!("(the habitat's memories were stopped, and were started to read the records)");
    }
    if plan_only {
        settle(&plan.started);
        return ExitCode::SUCCESS;
    }
    print!("\ntype the habitat's name ({name}) to remove all of the above: ");
    let _ = std::io::stdout().flush();
    let mut typed = String::new();
    let _ = std::io::stdin().lock().read_line(&mut typed);
    if typed.trim() != name {
        settle(&plan.started);
        eprintln!(
            "hale habitat down --forget: `{}` is not the habitat's name; nothing was removed",
            typed.trim()
        );
        return ExitCode::from(1);
    }
    // the plan executed is the plan the person saw: audited again, and
    // refused when what it would remove changed while they read it
    match audit() {
        Ok(now) if bound(&now.items) == bound(&plan.items) => {}
        Ok(_) => {
            settle(&plan.started);
            eprintln!("hale habitat down --forget: what forgetting the habitat would remove changed since its plan was printed (plan_changed); nothing was removed: run it again to plan from what is there now");
            return ExitCode::from(1);
        }
        Err(why) => {
            settle(&plan.started);
            eprintln!("hale habitat down --forget: {why}; nothing was removed");
            return ExitCode::from(1);
        }
    }
    println!();
    if execute(&plan.items, &name) {
        println!("the habitat `{name}` is forgotten");
        ExitCode::SUCCESS
    } else {
        eprintln!("hale habitat down --forget: the forget stopped at an item that failed; the items after it were left, and `hale habitat down --forget` plans again from what remains");
        ExitCode::from(1)
    }
}

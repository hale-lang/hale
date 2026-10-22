//! The habitat's nurseries and saplings (phase 4).
//!
//! A sapling is an organism's seed in a nursery, beside its packs: a
//! repository pinned at a revision with a digest, and the packs it adopts at
//! graft. The habitat's nurseries are rows of its record: the toolchain's
//! bundled nursery always, and each directory or git repository its Board
//! trusts (`habitat.nursery <source>`, the head's `Commands::add_nursery`).
//! What a nursery offers is read into the record by a habitat act
//! (`refresh`, at `hale habitat up` and `hale habitat nursery refresh`): a
//! sapling first seen is a `habitat.sapling <name>` row, and a newer
//! revision of one already pinned is never taken: it is filed as a
//! `habitat.sapling.proposed <name>` row the Board ratifies
//! (`Commands::ratify_sapling`). `hale habitat graft <sapling>` fetches the
//! pinned revision into the habitat's own store, checks its digest, grafts
//! it as a repository, adopts its packs, and records which sapling, revision
//! and digest on the graft's row.
//!
//! Phase 4 adds placement: `hale habitat place <allocation> <host>` and
//! `hale habitat apply [<host>] [--fresh]` are clients of the head's Board
//! rows `Commands::place_allocation` and `Commands::apply_allocations`, and
//! `graft … --host <name>` places the new organism on that host.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::*;

/// The digest a sapling is pinned by: the sha256 of the tree at a commit,
/// as `git ls-tree -r --full-tree` lists it (each file's mode, kind, blob
/// hash and path), `sha256:<hex>`. The commit too.
pub(super) fn digest_at(repo: &Path, revision: &str) -> Result<(String, String), String> {
    let git = |args: &[&str]| -> Result<String, String> {
        let o = Command::new("git").arg("-C").arg(repo).args(args).output().map_err(|e| format!("git: {e}"))?;
        if !o.status.success() {
            return Err(format!("git {}: {}", args.join(" "), String::from_utf8_lossy(&o.stderr).trim()));
        }
        Ok(String::from_utf8_lossy(&o.stdout).to_string())
    };
    let commit = git(&["rev-parse", "--verify", "-q", &format!("{revision}^{{commit}}")])
        .or_else(|_| git(&["rev-parse", "--verify", "-q", &format!("origin/{revision}^{{commit}}")]))
        .map_err(|_| format!("`{revision}` names no commit of {}", repo.display()))?
        .trim()
        .to_string();
    let tree = git(&["ls-tree", "-r", "--full-tree", &commit])?;
    let hex: String = openssl::sha::sha256(tree.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
    Ok((commit, format!("sha256:{hex}")))
}

/// The latest body of each `kind` row of the habitat's record, by entity.
fn latest(rows: &[serde_json::Value], kind: &str) -> BTreeMap<String, serde_json::Value> {
    let mut out = BTreeMap::new();
    for r in rows.iter().filter(|r| r["kind"] == kind) {
        let body = r["body"].as_str().and_then(|b| serde_json::from_str(b).ok()).unwrap_or(serde_json::Value::Null);
        out.insert(r["entity"].as_str().unwrap_or("").to_string(), body);
    }
    out
}

/// Every nursery the habitat's Board trusts, by its source.
pub(super) fn trusted_nurseries() -> Vec<String> {
    let rows = rows_of(&genome_dir());
    latest(&rows, "habitat.nursery").into_iter().filter(|(_, b)| b["state"] != "removed").map(|(k, _)| k).collect()
}

/// The sapling `name` as the habitat's record pins it.
pub(super) fn pinned(name: &str) -> Option<serde_json::Value> {
    latest(&rows_of(&genome_dir()), "habitat.sapling").remove(name)
}

fn sapling_body(s: &crate::dna::pack::Sapling, nursery: &str) -> serde_json::Value {
    serde_json::json!({ "name": s.name, "purpose": s.summary, "repo": s.repo, "revision": s.revision, "digest": s.digest, "packs": s.packs, "set": s.set, "nursery": nursery })
}

/// The rows `rows` appended to the habitat's record (a habitat act's).
pub(super) fn record(rows: &[serde_json::Value]) -> Result<(), String> {
    if rows.is_empty() {
        return Ok(());
    }
    let file = PathBuf::from(local_habitat_dir()).join(format!(".sapling-rows-{}.jsonl", std::process::id()));
    let text: String = rows.iter().map(|r| format!("{r}\n")).collect();
    std::fs::write(&file, text).map_err(|e| format!("write {}: {e}", file.display()))?;
    let landed = host_run("record-seed", &genome_dir(), &[file.display().to_string(), String::new()]);
    let _ = std::fs::remove_file(&file);
    landed.map(|_| ())
}

/// What each nursery the habitat holds offers, read into its record: a
/// sapling first seen pinned (`habitat.sapling`), and a newer revision of
/// one already pinned filed as a proposal (`habitat.sapling.proposed`),
/// never taken. The first nursery to offer a name holds it: the trusted
/// ones in the order of their sources, then the toolchain's (the default). A nursery that
/// cannot be read is said and skipped. What was done, a line each.
pub(super) fn refresh() -> Result<Vec<String>, String> {
    let rows = rows_of(&genome_dir());
    let pins = latest(&rows, "habitat.sapling");
    let proposals = latest(&rows, "habitat.sapling.proposed");
    // the nurseries the Board trusts first, then the toolchain's: the
    // bundled one is the default a trusted one may offer over
    let mut sources: Vec<Option<String>> = latest(&rows, "habitat.nursery").into_iter().filter(|(_, b)| b["state"] != "removed").map(|(k, _)| Some(k)).collect();
    sources.push(None);
    let mut out = Vec::new();
    let mut new_rows = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for src in sources {
        let n = match crate::dna::pack::nursery(src.as_deref()) {
            Ok(n) => n,
            Err(e) => {
                out.push(format!("{:<8}the nursery {} was not read: {e}", "note", src.as_deref().unwrap_or("of the toolchain")));
                continue;
            }
        };
        let nursery = src.clone().unwrap_or_else(|| "toolchain".to_string());
        for s in &n.saplings {
            if seen.contains(&s.name) {
                continue;
            }
            seen.push(s.name.clone());
            let body = sapling_body(s, &nursery);
            match pins.get(&s.name) {
                None => {
                    new_rows.push(serde_json::json!({ "kind": "habitat.sapling", "entity": s.name, "body": body.to_string() }));
                    out.push(format!("{:<8}{} at {} ({}), from {nursery}", "sapling", s.name, s.revision, s.digest));
                }
                Some(p) if p["revision"] == s.revision.as_str() && p["digest"] == s.digest.as_str() => {}
                Some(p) => {
                    let filed = proposals.get(&s.name).is_some_and(|q| q["revision"] == s.revision.as_str() && q["digest"] == s.digest.as_str());
                    if !filed {
                        let mut proposed = body.clone();
                        proposed["from"] = p["revision"].clone();
                        new_rows.push(serde_json::json!({ "kind": "habitat.sapling.proposed", "entity": s.name, "body": proposed.to_string() }));
                        out.push(format!("{:<8}{} {} -> {}: a newer revision in {nursery}, filed for the Board (nothing moves until it ratifies)", "upgrade", s.name, p["revision"].as_str().unwrap_or(""), s.revision));
                    }
                }
            }
        }
    }
    record(&new_rows)?;
    Ok(out)
}

/// Whether `name` is a sapling to graft rather than a directory: a bare
/// name, with no such path, that the habitat's record pins.
pub(super) fn names_a_sapling(arg: &Path) -> bool {
    let s = arg.to_string_lossy();
    !s.contains('/') && !arg.exists() && genome_dir().join("hale.toml").is_file() && pinned(&s).is_some()
}

/// The sapling `name` grafted: its pinned revision fetched into the
/// habitat's own store (`<habitat>/saplings/<name>`), its digest checked
/// (a mismatch refuses, and nothing is grafted), the repository there
/// grafted as the habitat's child, the sapling, revision and digest
/// recorded on the graft's row, and its packs proposed to the child's
/// Board. A graft that stopped part way goes on where it stopped: the
/// store keeps the revision it was checked at. (Where it is placed is D4's:
/// a graft takes a placement there.)
pub(super) fn graft_sapling(name: &str, library: bool, pack: bool, sets: &[String]) -> Result<Vec<String>, String> {
    let current = pinned(name).ok_or_else(|| format!("the habitat pins no sapling `{name}` (`hale habitat nursery list`)"))?;
    let store = PathBuf::from(local_habitat_dir()).join("saplings").join(name);
    // what the graft checked when it began, kept beside the store: a graft
    // that goes on is the revision it checked, whatever is pinned since
    let checked = PathBuf::from(local_habitat_dir()).join("saplings").join(format!("{name}.grafted.json"));
    let begun = manifest_pin(&store.join("hale.toml")).ok().flatten().is_some();
    let pin = if begun {
        match std::fs::read_to_string(&checked).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) {
            Some(p) => p,
            // nothing kept: the checkout's own tree must be the pinned one
            None => {
                let (_, found) = digest_at(&store, "HEAD")?;
                if Some(found.as_str()) != current["digest"].as_str() {
                    return Err(format!("the graft of `{name}` began at a revision the habitat no longer knows ({found}): remove {} to graft it afresh", store.display()));
                }
                current.clone()
            }
        }
    } else {
        current.clone()
    };
    let field = |k: &str| pin[k].as_str().unwrap_or("").to_string();
    let (repo, revision, digest, nursery) = (field("repo"), field("revision"), field("digest"), field("nursery"));
    let git = |args: &[&str], cwd: Option<&Path>| -> Result<(), String> {
        let mut c = Command::new("git");
        if let Some(d) = cwd {
            c.arg("-C").arg(d);
        }
        let o = c.args(args).output().map_err(|e| format!("git: {e}"))?;
        if o.status.success() {
            Ok(())
        } else {
            Err(format!("git {}: {}", args.join(" "), String::from_utf8_lossy(&o.stderr).trim()))
        }
    };
    let mut out = Vec::new();
    if !store.join(".git").is_dir() {
        std::fs::create_dir_all(store.parent().unwrap_or(&store)).map_err(|e| format!("create {}: {e}", store.display()))?;
        git(&["clone", "-q", "--no-checkout", &repo, &store.to_string_lossy()], None).map_err(|e| format!("the sapling `{name}` could not be fetched from {repo}: {e}"))?;
    } else if !begun {
        git(&["fetch", "-q", "--tags", "--force", "origin"], Some(&store)).map_err(|e| format!("the sapling `{name}` could not be fetched from {repo}: {e}"))?;
    }
    // the digest is checked at every graft that has not begun; one that
    // began goes on at the revision it was checked at
    if !begun {
        let (commit, found) = digest_at(&store, &revision)?;
        if found != digest {
            return Err(format!("the sapling `{name}` at {revision} is {found}, not the {digest} its nursery pinned: refused, nothing grafted"));
        }
        git(&["checkout", "-q", "--detach", &commit], Some(&store))?;
        let mut kept = pin.clone();
        kept["commit"] = serde_json::json!(commit);
        std::fs::write(&checked, format!("{kept}\n")).map_err(|e| format!("write {}: {e}", checked.display()))?;
        out.push(format!("{:<8}{name} at {revision} ({}…), its digest {digest} checked", "fetched", &commit[..12]));
    } else {
        out.push(format!("{:<8}{name} goes on at {revision} ({digest}), the revision its graft checked", "kept"));
    }
    let lineage = serde_json::json!({ "sapling": name, "revision": revision, "digest": digest, "nursery": nursery });
    out.extend(graft_lines(&store, true, library, pack, &Shape { profile: None, remote: None, body: None }, Some(&lineage))?);
    // its packs, proposed to the child's Board, from its own nursery (or the
    // toolchain's when its own offers none by the name)
    let packs: Vec<String> = pin["packs"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
    if !packs.is_empty() {
        let n = crate::dna::pack::nursery(if nursery == "toolchain" { None } else { Some(nursery.as_str()) })?;
        let mut set: BTreeMap<String, String> = pin["set"].as_object().map(|o| o.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string())).collect()).unwrap_or_default();
        for s in sets {
            if let Some((k, v)) = s.split_once('=') {
                set.insert(k.trim().to_string(), v.trim().to_string());
            }
        }
        let root = crate::dna::organism_root(&store)?;
        for p in packs {
            crate::dna::pack::propose_pack(&n, &p, &set, &root).map_err(|e| format!("the sapling's pack `{p}` was not proposed: {e}"))?;
            out.push(format!("{:<8}`{p}` proposed to the child's Board, as the sapling adopts it", "pack"));
        }
    }
    // its own services, on the host its graft names
    out.extend(place_services(&crate::dna::organism_root(&store)?)?);
    Ok(out)
}

/// The host `name` as the habitat's record holds it, for a graft's
/// placement: `None` for `local` (nothing to place), an error for a host
/// that is not added or not ready.
pub(super) fn graft_host(name: &str) -> Result<Option<serde_json::Value>, String> {
    if name == "local" {
        return Ok(None);
    }
    let h = latest(&rows_of(&genome_dir()), "habitat.host").remove(name).ok_or_else(|| format!("no host `{name}` was added (`hale habitat host add {name}`)"))?;
    if h["state"] != "ready" {
        return Err(format!("host `{name}` is {}: `hale habitat host probe {name}` checks it again", h["state"].as_str().unwrap_or("not ready")));
    }
    Ok(Some(serde_json::json!({
        "host": name,
        "provider": h["provider"].as_str().unwrap_or("ssh"),
        "address": h["address"].as_str().unwrap_or(name),
        "project": format!("{}-{}", local_habitat_project(), name.replace('.', "-")),
    })))
}

/// The organism's own services (a repository's compose file at its root),
/// an allocation of the host its graft placed it on, and applied there by
/// the head (`Commands::apply_allocations`). Nothing for an organism with no
/// compose file or a graft that names no host.
fn place_services(root: &Path) -> Result<Vec<String>, String> {
    let Some(h) = GRAFT_HOST.get() else { return Ok(Vec::new()) };
    if !root.join("compose.yaml").is_file() {
        return Ok(vec![format!("{:<8}no compose.yaml at its root: no services of its own to place", "note")]);
    }
    let id = manifest_pin(&root.join("hale.toml")).ok().flatten().ok_or("the organism pins no record")?;
    let key = &id[..12];
    let host = h["host"].as_str().unwrap_or("");
    // its own compose project on the host, named for it: its services are
    // its own, never the habitat's
    let project = format!("{}-{key}", h["project"].as_str().unwrap_or(""));
    // the files its services mount beside the compose file: the
    // repository's own, at its root (what it tracks, never what the graft
    // wrote)
    let tracked = Command::new("git").arg("-C").arg(root).args(["ls-files"]).output().map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
    let mounted: Vec<&str> = tracked.lines().filter(|f| !f.contains('/') && *f != "compose.yaml" && *f != "hale.toml").collect();
    let body = serde_json::json!({ "organism": id, "role": "services", "service": "", "project": project, "provider": h["provider"], "host": host, "render_dir": root.display().to_string(), "render_files": "compose.yaml", "render_mounted": mounted.join("\n"), "vault": [] });
    record(&[serde_json::json!({ "kind": "habitat.allocated", "entity": format!("{key}/services"), "body": body.to_string() })])?;
    let mut out = vec![format!("{:<8}its services ({key}/services) on {host}", "placed")];
    out.extend(applied_lines(&head_call("Commands::apply_allocations", serde_json::json!({ "host": host, "fresh": false }))?));
    Ok(out)
}

/// What `Commands::apply_allocations` answered, a line per allocation.
fn applied_lines(v: &serde_json::Value) -> Vec<String> {
    let mut out = Vec::new();
    let list: serde_json::Value = v["applied"].as_str().and_then(|t| serde_json::from_str(t).ok()).unwrap_or_default();
    for a in list.as_array().into_iter().flatten() {
        let f = |k: &str| a[k].as_str().unwrap_or("").to_string();
        if a["ok"] == serde_json::json!(true) {
            out.push(format!("{:<8}{} on {} ({}): {}", "applied", f("allocation"), f("host"), f("provider"), f("state")));
        } else {
            out.push(format!("{:<8}{} on {}: {}", "refused", f("allocation"), f("host"), f("why")));
        }
    }
    if let Some(note) = v["why"].as_str().filter(|w| !w.is_empty()) {
        out.push(format!("{:<8}{note}", "note"));
    }
    out
}

/// The head's `Commands::graft` for `name` on `host`, when the head answers,
/// followed to its end: what the graft said (`said`) and whether it ended
/// well (`ok`, else `code` and `why`); `None` when no head is up.
pub(super) fn head_graft(name: &str, host: &str, library: bool) -> Option<serde_json::Value> {
    if !head_socket().is_some_and(|p| p.exists()) {
        return None;
    }
    let v = match head_call("Commands::graft", serde_json::json!({ "sapling": name, "host": host, "library": library })) {
        Ok(v) => v,
        Err(e) => return Some(serde_json::json!({ "ok": false, "code": "no_answer", "why": e, "said": "" })),
    };
    let Some(log) = v["log"].as_str().filter(|l| !l.is_empty()).map(PathBuf::from) else { return Some(v) };
    // the graft runs on in the head's keeping: its log, until its exit
    let exit = log.with_extension("log.exit");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3600);
    while !exit.is_file() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    let said = std::fs::read_to_string(&log).unwrap_or_default();
    let code = std::fs::read_to_string(&exit).ok().map(|t| t.trim().to_string()).unwrap_or_default();
    Some(if code == "0" {
        serde_json::json!({ "ok": true, "said": said })
    } else {
        serde_json::json!({ "ok": false, "code": "not_grafted", "why": format!("the graft ended with {} (its log: {})", if code.is_empty() { "no status".to_string() } else { format!("exit {code}") }, log.display()), "said": said })
    })
}

/// `hale habitat model-map <organism> <category> <model> [--endpoint <url>]
/// [--key-vault <slot>] [--tools]` and `hale habitat model-probe <organism>
/// <category>`: clients of the head's `Commands::set_model_map` and
/// `Commands::probe_model`.
pub(super) fn model_map_cmd(args: &[String], probe: bool) -> std::process::ExitCode {
    use std::process::ExitCode;
    let usage = if probe { "usage: hale habitat model-probe <organism> <mode>.<size>" } else { "usage: hale habitat model-map <organism> <mode>.<size> <model> [--endpoint <url>] [--key-vault <slot>] [--tools]" };
    let words: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    let flag = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned().unwrap_or_default();
    let (member, payload) = if probe {
        let [org, cat] = words.as_slice() else {
            eprintln!("{usage}");
            return ExitCode::from(2);
        };
        ("Commands::probe_model", serde_json::json!({ "organism": org, "category": cat }))
    } else {
        let endpoint = flag("--endpoint");
        let key_vault = flag("--key-vault");
        let rest: Vec<&String> = words.iter().copied().filter(|w| **w != endpoint && **w != key_vault).collect();
        let [org, cat, model] = rest.as_slice() else {
            eprintln!("{usage}");
            return ExitCode::from(2);
        };
        ("Commands::set_model_map", serde_json::json!({ "organism": org, "category": cat, "model": model, "endpoint": endpoint, "key_vault": key_vault, "tools": args.iter().any(|a| a == "--tools") }))
    };
    match head_call(member, payload) {
        Ok(v) if v["ok"] == serde_json::json!(true) => {
            if probe {
                println!("{:<8}{} at {}: {}", "model", v["model"].as_str().unwrap_or(""), v["endpoint"].as_str().unwrap_or(""), v["answer"].as_str().unwrap_or(""));
            } else {
                println!("{}", v["why"].as_str().unwrap_or(""));
            }
            ExitCode::SUCCESS
        }
        Ok(v) => {
            eprintln!("hale habitat {}: {} ({})", if probe { "model-probe" } else { "model-map" }, v["why"].as_str().unwrap_or(""), v["code"].as_str().unwrap_or(""));
            ExitCode::from(1)
        }
        Err(e) => {
            eprintln!("hale habitat {}: {e}", if probe { "model-probe" } else { "model-map" });
            ExitCode::from(1)
        }
    }
}

/// `hale habitat place <allocation> <host>`.
pub(super) fn place_cmd(args: &[String]) -> std::process::ExitCode {
    use std::process::ExitCode;
    let [allocation, host] = args else {
        eprintln!("usage: hale habitat place <key>/<role> <host>");
        return ExitCode::from(2);
    };
    match head_call("Commands::place_allocation", serde_json::json!({ "allocation": allocation, "host": host })) {
        Ok(v) if v["ok"] == serde_json::json!(true) => {
            println!("{:<8}{allocation} on {host}: `hale habitat apply` applies it", "placed");
            ExitCode::SUCCESS
        }
        Ok(v) => {
            eprintln!("hale habitat place: {} ({})", v["why"].as_str().unwrap_or(""), v["code"].as_str().unwrap_or(""));
            ExitCode::from(1)
        }
        Err(e) => {
            eprintln!("hale habitat place: {e}");
            ExitCode::from(1)
        }
    }
}

/// `hale habitat apply [<host>] [--fresh]`: the habitat's allocations
/// applied where they are placed, and the pulse after; when its own memory
/// or nerves moved, its face (and the head behind it) restarted on the new
/// address.
pub(super) fn apply_cmd(args: &[String]) -> std::process::ExitCode {
    use std::process::ExitCode;
    let fresh = args.iter().any(|a| a == "--fresh");
    let hosts: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    if hosts.len() > 1 || args.iter().any(|a| a.starts_with("--") && a != "--fresh") {
        eprintln!("usage: hale habitat apply [<host>] [--fresh]");
        return ExitCode::from(2);
    }
    let host = hosts.first().map(|h| h.to_string()).unwrap_or_default();
    match head_call("Commands::apply_allocations", serde_json::json!({ "host": host, "fresh": fresh })) {
        Ok(v) => {
            for l in applied_lines(&v) {
                println!("{l}");
            }
            // the habitat's own memory or nerves moved: its head and face
            // pick the new address up on their next start
            if v["restart"] == serde_json::json!(true) {
                match restart_face() {
                    Ok(line) => println!("{line}"),
                    Err(e) => println!("{:<8}the habitat's face did not restart: {e}", "note"),
                }
            }
            if v["ok"] == serde_json::json!(true) {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        Err(e) => {
            eprintln!("hale habitat apply: {e}");
            ExitCode::from(1)
        }
    }
}

/// The socket the habitat's head serves its commands on, as
/// `dna/api/main.hl`'s `head_socket` names it for the habitat's record.
fn head_socket() -> Option<PathBuf> {
    if let Some(given) = std::env::var("HALE_DNA_RPC_SOCKET").ok().filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(given));
    }
    let genome = genome_dir();
    let id = manifest_pin(&genome.join("hale.toml")).ok().flatten()?;
    let name = format!("{}.sock", &id[..12.min(id.len())]);
    let dir = match std::env::var("XDG_RUNTIME_DIR").ok().filter(|v| !v.is_empty()) {
        Some(r) => format!("{r}/hale/dna"),
        None => genome.join(".hale/dna").display().to_string(),
    };
    // an AF_UNIX path has room for 103 bytes
    if dir.len() + 1 + name.len() > 103 {
        let uid = unsafe { libc::getuid() };
        return Some(PathBuf::from(format!("/tmp/hale-{uid}/{name}")));
    }
    Some(PathBuf::from(dir).join(name))
}

/// One call of the habitat head's surface over its socket: the value it
/// answered, or why there was none.
pub(super) fn head_call(member: &str, payload: serde_json::Value) -> Result<serde_json::Value, String> {
    let socket = head_socket().filter(|p| p.exists()).ok_or("the habitat's head is not up (`hale habitat up` starts it)")?;
    let line = crate::api_client::unix_exchange(&socket.display().to_string(), &serde_json::json!({ "call": member, "payload": payload, "id": "habitat-nursery" })).map_err(|e| format!("the habitat's head did not answer: {e}"))?;
    let reply: serde_json::Value = serde_json::from_str(&line).unwrap_or_default();
    if reply["ok"] != serde_json::json!(true) {
        let why = reply["refusal"]["reason"].as_str().or_else(|| reply["error"].as_str()).unwrap_or(&line).to_string();
        return Err(format!("the habitat's head refused {member}: {why}"));
    }
    Ok(reply["value"].clone())
}

/// `hale habitat nursery add <source> | refresh | list | ratify <sapling>`.
pub(super) fn nursery_cmd(args: &[String]) -> std::process::ExitCode {
    use std::process::ExitCode;
    let usage = "usage: hale habitat nursery add <dir|git-url> | refresh | list | ratify <sapling>";
    match (args.first().map(String::as_str), args.get(1), args.len()) {
        (Some("add"), Some(source), 2) => {
            let source = match Path::new(source).canonicalize() {
                Ok(p) if p.is_dir() => p.display().to_string(),
                _ => source.clone(),
            };
            match head_call("Commands::add_nursery", serde_json::json!({ "source": source })) {
                Ok(v) if v["ok"] == serde_json::json!(true) => {
                    println!("{:<8}{source}: the Board trusts it", "nursery");
                    // what the head read of it
                    if let Some(said) = v["why"].as_str().filter(|w| !w.is_empty()) {
                        println!("{said}");
                    }
                }
                Ok(v) => {
                    eprintln!("hale habitat nursery add: {} ({})", v["why"].as_str().unwrap_or(""), v["code"].as_str().unwrap_or(""));
                    return ExitCode::from(1);
                }
                Err(e) => {
                    eprintln!("hale habitat nursery add: {e}");
                    return ExitCode::from(1);
                }
            }
            refreshed()
        }
        (Some("refresh"), None, 1) => refreshed(),
        (Some("ratify"), Some(name), 2) => match head_call("Commands::ratify_sapling", serde_json::json!({ "name": name })) {
            Ok(v) if v["ok"] == serde_json::json!(true) => {
                let pin = pinned(name).unwrap_or_default();
                println!("{:<8}{name} pinned at {} ({}); nothing grafted moves", "sapling", pin["revision"].as_str().unwrap_or(""), pin["digest"].as_str().unwrap_or(""));
                ExitCode::SUCCESS
            }
            Ok(v) => {
                eprintln!("hale habitat nursery ratify: {} ({})", v["why"].as_str().unwrap_or(""), v["code"].as_str().unwrap_or(""));
                ExitCode::from(1)
            }
            Err(e) => {
                eprintln!("hale habitat nursery ratify: {e}");
                ExitCode::from(1)
            }
        },
        (Some("list"), None, 1) => {
            let read = |member: &str, field: &str| head_call(member, serde_json::json!({ "limit": 200 })).map(|v| v[field].as_str().and_then(|t| serde_json::from_str::<serde_json::Value>(t).ok()).unwrap_or_default());
            match (read("Commands::nurseries", "nurseries"), read("Commands::saplings", "saplings")) {
                (Ok(n), Ok(s)) => {
                    for e in n.as_array().into_iter().flatten() {
                        println!("{:<8}{} ({})", "nursery", e["source"].as_str().unwrap_or(""), e["kind"].as_str().unwrap_or(""));
                    }
                    for e in s.as_array().into_iter().flatten() {
                        let proposed = e["proposed"].as_str().filter(|p| !p.is_empty()).map(|p| format!("; {p} proposed")).unwrap_or_default();
                        println!("{:<8}{} at {} ({}) from {}: {}{proposed}", "sapling", e["name"].as_str().unwrap_or(""), e["revision"].as_str().unwrap_or(""), e["digest"].as_str().unwrap_or(""), e["nursery"].as_str().unwrap_or(""), e["purpose"].as_str().unwrap_or(""));
                    }
                    ExitCode::SUCCESS
                }
                (Err(e), _) | (_, Err(e)) => {
                    eprintln!("hale habitat nursery list: {e}");
                    ExitCode::from(1)
                }
            }
        }
        _ => {
            eprintln!("{usage}");
            ExitCode::from(2)
        }
    }
}

fn refreshed() -> std::process::ExitCode {
    match refresh() {
        Ok(lines) => {
            for l in lines {
                println!("{l}");
            }
            std::process::ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("hale habitat nursery: {e}");
            std::process::ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The digest is the tree's, whatever the commit: the same files at two
    /// commits are one digest, and a changed file is another.
    #[test]
    fn a_digest_is_the_trees() {
        let d = std::env::temp_dir().join(format!("hale-sapling-digest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let git = |args: &[&str]| assert!(Command::new("git").arg("-C").arg(&d).args(["-c", "user.name=t", "-c", "user.email=t@t"]).args(args).output().unwrap().status.success(), "{args:?}");
        git(&["init", "-q", "-b", "main"]);
        std::fs::write(d.join("hale.toml"), "[dna]\n").unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "one"]);
        git(&["tag", "v1"]);
        git(&["commit", "-q", "--allow-empty", "-m", "same tree"]);
        git(&["tag", "v1b"]);
        std::fs::write(d.join("README.md"), "more\n").unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "two"]);
        let (c1, a) = digest_at(&d, "v1").unwrap();
        let (c2, b) = digest_at(&d, "v1b").unwrap();
        let (_, c) = digest_at(&d, "main").unwrap();
        assert!(a.starts_with("sha256:") && a.len() == 71, "{a}");
        assert_ne!(c1, c2);
        assert_eq!(a, b, "one tree, one digest");
        assert_ne!(a, c, "another tree, another digest");
        assert!(digest_at(&d, "v9").unwrap_err().contains("names no commit"));
        let _ = std::fs::remove_dir_all(&d);
    }
}

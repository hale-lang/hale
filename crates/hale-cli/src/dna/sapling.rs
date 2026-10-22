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

/// A verb's arguments consumed by position: each of `values` takes the word
/// right after it as its value, whatever that word is; each of `switches`
/// stands alone; every other word is a positional, in its order. `None`: an
/// option the verb does not take, or a value option at the end.
pub(super) struct VerbArgs {
    words: Vec<String>,
    values: BTreeMap<String, String>,
    switches: Vec<String>,
}

pub(super) fn verb_args(args: &[String], values: &[&str], switches: &[&str]) -> Option<VerbArgs> {
    let mut out = VerbArgs { words: Vec::new(), values: BTreeMap::new(), switches: Vec::new() };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if values.contains(&a.as_str()) {
            out.values.insert(a.clone(), it.next()?.clone());
        } else if switches.contains(&a.as_str()) {
            out.switches.push(a.clone());
        } else if a.starts_with("--") {
            return None;
        } else {
            out.words.push(a.clone());
        }
    }
    Some(out)
}

impl VerbArgs {
    fn value(&self, name: &str) -> String {
        self.values.get(name).cloned().unwrap_or_default()
    }
    fn has(&self, name: &str) -> bool {
        self.switches.iter().any(|s| s == name)
    }
}

/// `hale habitat model-map <organism> <category> <model> [--endpoint <url>]
/// [--key-vault <slot>] [--tools]` and `hale habitat model-probe <organism>
/// <category>`: clients of the head's `Commands::set_model_map` and
/// `Commands::probe_model`.
pub(super) fn model_map_cmd(args: &[String], probe: bool) -> std::process::ExitCode {
    use std::process::ExitCode;
    let usage = if probe { "usage: hale habitat model-probe <organism> <mode>.<size>" } else { "usage: hale habitat model-map <organism> <mode>.<size> <model> [--endpoint <url>] [--key-vault <slot>] [--tools]" };
    let Some((member, payload)) = model_map_request(args, probe) else {
        eprintln!("{usage}");
        return ExitCode::from(2);
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

/// The head's member and its payload for `model-map` (or, with `probe`,
/// `model-probe`), the options consumed by position; `None`: not the verb's
/// shape.
fn model_map_request(args: &[String], probe: bool) -> Option<(&'static str, serde_json::Value)> {
    if probe {
        let a = verb_args(args, &[], &[])?;
        let [org, cat] = a.words.as_slice() else { return None };
        return Some(("Commands::probe_model", serde_json::json!({ "organism": org, "category": cat })));
    }
    let a = verb_args(args, &["--endpoint", "--key-vault"], &["--tools"])?;
    let [org, cat, model] = a.words.as_slice() else { return None };
    Some(("Commands::set_model_map", serde_json::json!({ "organism": org, "category": cat, "model": model, "endpoint": a.value("--endpoint"), "key_vault": a.value("--key-vault"), "tools": a.has("--tools") })))
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
    let parsed = verb_args(args, &[], &["--fresh"]).filter(|a| a.words.len() <= 1);
    let Some(a) = parsed else {
        eprintln!("usage: hale habitat apply [<host>] [--fresh]");
        return ExitCode::from(2);
    };
    let fresh = a.has("--fresh");
    let host = a.words.first().cloned().unwrap_or_default();
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

    fn words(line: &str) -> Vec<String> {
        line.split(' ').map(str::to_string).collect()
    }

    /// An option's value is the word right after it, never a word equal to
    /// it elsewhere: a slot or an endpoint named like the organism, the
    /// category or the model leaves all three positionals.
    #[test]
    fn model_map_options_are_consumed_by_position() {
        let (member, p) = model_map_request(&words("voice think.quick local --endpoint http://localhost:8000/v1/chat/completions --key-vault voice --tools"), false).unwrap();
        assert_eq!(member, "Commands::set_model_map");
        assert_eq!((p["organism"].as_str(), p["category"].as_str(), p["model"].as_str()), (Some("voice"), Some("think.quick"), Some("local")));
        assert_eq!((p["key_vault"].as_str(), p["tools"].as_bool()), (Some("voice"), Some(true)));
        let (_, q) = model_map_request(&words("alpha think.quick local --key-vault local"), false).unwrap();
        assert_eq!((q["model"].as_str(), q["key_vault"].as_str(), q["endpoint"].as_str()), (Some("local"), Some("local"), Some("")));
        let (_, r) = model_map_request(&words("--endpoint think.quick alpha think.quick m"), false).unwrap();
        assert_eq!((r["organism"].as_str(), r["category"].as_str(), r["endpoint"].as_str()), (Some("alpha"), Some("think.quick"), Some("think.quick")));
        assert!(model_map_request(&words("alpha think.quick m --endpoint"), false).is_none(), "a value option at the end");
        assert!(model_map_request(&words("alpha think.quick m --other x"), false).is_none());
        assert!(model_map_request(&words("alpha think.quick"), true).is_some_and(|(m, _)| m == "Commands::probe_model"));
        assert!(model_map_request(&words("alpha think.quick --tools"), true).is_none());
    }

    /// `apply`: one host at most, `--fresh` anywhere, nothing else.
    #[test]
    fn apply_takes_one_host_and_fresh() {
        let a = verb_args(&words("--fresh --fresh-host"), &[], &["--fresh"]);
        assert!(a.is_none(), "an option it does not take");
        let b = verb_args(&words("rack-a --fresh"), &[], &["--fresh"]).unwrap();
        assert_eq!((b.words.as_slice(), b.has("--fresh")), (&["rack-a".to_string()][..], true));
        let c = verb_args(&words("--fresh rack-a"), &[], &["--fresh"]).unwrap();
        assert_eq!(c.words, ["rack-a"]);
    }

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

    /// D8: the bundled nursery's `hale` sapling pins the digest of the tree
    /// checked in at dna/nursery/saplings/hale, so the tree a repository
    /// is made from is the one the nursery pins (and an edit to the tree
    /// moves the pin with it).
    #[test]
    fn the_hale_sapling_pins_its_checked_in_tree() {
        let dna = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dna/nursery");
        let index: toml::Table = std::fs::read_to_string(dna.join("nursery.toml")).unwrap().parse().unwrap();
        let hale = index["sapling"].as_array().unwrap().iter().find(|s| s["name"].as_str() == Some("hale")).expect("the bundled nursery offers `hale`");
        let d = std::env::temp_dir().join(format!("hale-sapling-hale-tree-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        for e in std::fs::read_dir(dna.join("saplings/hale")).unwrap().flatten() {
            std::fs::copy(e.path(), d.join(e.file_name())).unwrap();
        }
        let git = |args: &[&str]| assert!(Command::new("git").arg("-C").arg(&d).args(["-c", "user.name=t", "-c", "user.email=t@t"]).args(args).output().unwrap().status.success(), "{args:?}");
        git(&["init", "-q", "-b", "main"]);
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "the hale organism"]);
        let (_, digest) = digest_at(&d, hale["revision"].as_str().unwrap()).unwrap();
        let _ = std::fs::remove_dir_all(&d);
        assert_eq!(Some(digest.as_str()), hale["digest"].as_str(), "the nursery's `hale` digest is the checked-in tree's: update dna/nursery/nursery.toml");
        assert_eq!(hale["packs"].as_array().unwrap()[0].as_str(), Some("hale-organism"));
    }
}

// ---- forge and identity: roles the habitat allocates (phase 4) -------------

/// The `habitat.allocated` body of a role no compose service holds.
fn role_body(organism: &str, role: &str, provider: &str, host: &str, address: &str, repo: &str, key_vault: &str) -> serde_json::Value {
    let vault: Vec<&str> = if key_vault.is_empty() { Vec::new() } else { vec![key_vault] };
    serde_json::json!({ "organism": organism, "role": role, "service": "", "provider": provider, "host": host, "address": address, "repo": repo, "key_vault": key_vault, "vault": vault })
}

/// The forge allocated to the organism keyed `key`, kept beside its
/// allocations for the host to read (`dna::forge_allocated`).
fn forge_cache(key: &str, provider: &str, repo: &str, key_vault: &str, address: &str) -> Result<(), String> {
    let dir = PathBuf::from(local_habitat_dir()).join("allocations").join(key);
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let path = dir.join("forge.json");
    std::fs::write(&path, format!("{}\n", serde_json::json!({ "provider": provider, "repo": repo, "key_vault": key_vault, "address": address }))).map_err(|e| format!("write {}: {e}", path.display()))
}

/// What a grafted organism's roles still need, from the latest
/// `habitat.allocated` bodies and the forge cache as it stands: the rows its
/// record lacks (each recorded once, when absent, so a later Board
/// allocation stands) and the cache when it is missing or says other than
/// the forge row. A graft that stopped part way finishes here on its retry.
/// `fresh` is the forge row to record when the record holds none.
fn roles_needed(allocated: &BTreeMap<String, serde_json::Value>, cache: Option<serde_json::Value>, child: &str, fresh: Option<&serde_json::Value>, issuer: &str) -> (Vec<serde_json::Value>, Option<serde_json::Value>) {
    let key = &child[..12];
    let mut rows = Vec::new();
    let forge_body = match allocated.get(&format!("{key}/forge")) {
        Some(b) => b.clone(),
        None => {
            let b = fresh.cloned().unwrap_or_else(|| role_body(child, "forge", "none", "", "", "", ""));
            rows.push(serde_json::json!({ "kind": "habitat.allocated", "entity": format!("{key}/forge"), "body": b.to_string() }));
            b
        }
    };
    if !allocated.contains_key(&format!("{key}/identity")) {
        rows.push(serde_json::json!({ "kind": "habitat.allocated", "entity": format!("{key}/identity"), "body": role_body(child, "identity", "oidc-stub", "local", issuer, "", "").to_string() }));
    }
    let field = |k: &str| forge_body[k].as_str().unwrap_or("").to_string();
    let wanted = serde_json::json!({ "provider": field("provider"), "repo": field("repo"), "key_vault": field("key_vault"), "address": field("address") });
    (rows, if cache.as_ref() == Some(&wanted) { None } else { Some(wanted) })
}

/// The forge row a newly grafted organism is given: with `forgejo`, its
/// user, token slot and repository at the habitat's Forgejo, made by the
/// host (`habitat-forge-provision`); any other forge, as the provider alone.
/// `root` is where the organism's code lives (`-` when the habitat does not
/// know, and nothing is pushed).
fn forge_body_for(child: &str, name: &str, root: &str, forge: &str) -> Result<serde_json::Value, String> {
    if forge != "forgejo" {
        return Ok(role_body(child, "forge", forge, "", "", "", ""));
    }
    let said = host_run("habitat-forge-provision", &genome_dir(), &[name.to_string(), child[..12].to_string(), root.to_string()])?;
    let field = |k: &str| said.lines().find_map(|l| l.strip_prefix(&format!("{k}="))).unwrap_or("").to_string();
    Ok(role_body(child, "forge", "forgejo", "local", &field("address"), &field("repo"), &field("key_vault")))
}

/// A grafted organism's forge and identity: the forge the graft's `--forge`
/// or the habitat's policy names (the habitat's Forgejo on its own
/// backends, none on an operator's), the identity the
/// habitat's own sign-in at the organism's issuer. Run on every graft, the
/// first and each retry: what is already allocated stays as it is.
pub(super) fn allocate_roles(child: &str, root: &Path) -> Result<Vec<String>, String> {
    let key = &child[..12];
    let rows = rows_of(&genome_dir());
    let policy = latest(&rows, "habitat.policy");
    let forge = GRAFT_FORGE.get().cloned().or_else(|| policy.get("forge").and_then(|p| p["provider"].as_str().map(str::to_string))).unwrap_or_else(|| "none".to_string());
    if !["none", "github", "file", "forgejo"].contains(&forge.as_str()) {
        return Err(format!("`--forge {forge}`: a forge is forgejo, github, file or none"));
    }
    let issuer = Command::new("git").arg("-C").arg(root).args(["config", "--get", "dna.oidc.issuer"]).output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| "http://127.0.0.1:8794".to_string());
    let path = PathBuf::from(local_habitat_dir()).join("allocations").join(key).join("forge.json");
    let cache = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok());
    let allocated = latest(&rows, "habitat.allocated");
    let fresh = if allocated.contains_key(&format!("{key}/forge")) { None } else { Some(forge_body_for(child, &root.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(), &root.display().to_string(), &forge)?) };
    let (missing, rewrite) = roles_needed(&allocated, cache, child, fresh.as_ref(), &issuer);
    if missing.is_empty() && rewrite.is_none() {
        return Ok(Vec::new());
    }
    record(&missing)?;
    if let Some(c) = rewrite {
        let field = |k: &str| c[k].as_str().unwrap_or("").to_string();
        forge_cache(key, &field("provider"), &field("repo"), &field("key_vault"), &field("address"))?;
    }
    let allocated = latest(&rows_of(&genome_dir()), "habitat.allocated");
    let fb = allocated.get(&format!("{key}/forge")).cloned().unwrap_or_default();
    let provider = fb["provider"].as_str().unwrap_or(&forge).to_string();
    let provider = match (fb["repo"].as_str().unwrap_or(""), fb["key_vault"].as_str().unwrap_or("")) {
        ("", _) => provider,
        (repo, "") => format!("{provider} {repo}"),
        (repo, slot) => format!("{provider} {repo}, its token in the vault slot {slot}"),
    };
    let at = allocated.get(&format!("{key}/identity")).and_then(|b| b["address"].as_str()).unwrap_or(&issuer).to_string();
    Ok(vec![format!("{:<8}its forge ({provider}) and identity (oidc-stub at {at})", "roles")])
}

/// `hale habitat forge` arguments, consumed by position: each option takes
/// the word after it, whatever that word is, and the two words left are the
/// organism and the provider (`None`: not that shape).
struct ForgeArgs {
    organism: String,
    provider: String,
    repo: String,
    key_vault: String,
    no_head: bool,
}

fn forge_args(args: &[String]) -> Option<ForgeArgs> {
    let (mut repo, mut key_vault, mut no_head) = (String::new(), String::new(), false);
    let mut words = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--repo" => repo = it.next()?.clone(),
            "--key-vault" => key_vault = it.next()?.clone(),
            "--no-head" => no_head = true,
            w if w.starts_with("--") => return None,
            w => words.push(w.to_string()),
        }
    }
    let [organism, provider] = <[String; 2]>::try_from(words).ok()?;
    Some(ForgeArgs { organism, provider, repo, key_vault, no_head })
}

/// `hale habitat forge <organism> <provider> [--repo r] [--key-vault s]
/// [--no-head]`: the organism's forge allocated, through the head's
/// `Commands::allocate_forge` while it answers, else (or with `--no-head`)
/// as the act itself.
pub(super) fn forge_cmd(args: &[String]) -> std::process::ExitCode {
    use std::process::ExitCode;
    let usage = "usage: hale habitat forge <organism> forgejo|github|file|none [--repo <owner/name>] [--key-vault <slot>] [--no-head]";
    let Some(ForgeArgs { organism, provider, repo, key_vault, no_head }) = forge_args(args) else {
        eprintln!("{usage}");
        return ExitCode::from(2);
    };
    if !no_head && head_socket().is_some_and(|p| p.exists()) {
        return match head_call("Commands::allocate_forge", serde_json::json!({ "organism": organism, "provider": provider, "repo": repo, "key_vault": key_vault })) {
            Ok(v) if v["ok"] == serde_json::json!(true) => {
                println!("{}", v["why"].as_str().unwrap_or(""));
                ExitCode::SUCCESS
            }
            Ok(v) => {
                eprintln!("hale habitat forge: {} ({})", v["why"].as_str().unwrap_or(""), v["code"].as_str().unwrap_or(""));
                ExitCode::from(1)
            }
            Err(e) => {
                eprintln!("hale habitat forge: {e}");
                ExitCode::from(1)
            }
        };
    }
    match allocate_forge(&organism, &provider, &repo, &key_vault) {
        Ok(line) => {
            println!("{line}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("hale habitat forge: {e}");
            ExitCode::from(1)
        }
    }
}

/// The forge `provider` allocated to the organism the habitat grafted as
/// `organism` (`habitat`: the habitat's own): its row, and its cache.
fn allocate_forge(organism: &str, provider: &str, repo: &str, key_vault: &str) -> Result<String, String> {
    if !["none", "github", "file", "forgejo"].contains(&provider) {
        return Err(format!("`{provider}`: a forge is forgejo, github, file or none"));
    }
    let slot = if key_vault.is_empty() && provider == "github" { "forge-token" } else { key_vault };
    if provider == "github" {
        let ok = repo.split_once('/').is_some_and(|(o, n)| !o.is_empty() && !n.is_empty() && !n.contains('/'));
        if !ok {
            return Err("a github forge names its repository, --repo <owner/name>".to_string());
        }
    }
    if !slot.is_empty() && !slot.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)) {
        return Err(format!("`--key-vault {slot}` names no vault slot"));
    }
    let genome = genome_dir();
    let rows = rows_of(&genome);
    let child = if organism == "habitat" {
        manifest_pin(&genome.join("hale.toml")).ok().flatten().ok_or("the habitat is not born")?
    } else {
        let grafted = latest(&rows, "habitat.grafted");
        grafted.iter().find(|(_, b)| b["name"] == organism).map(|(_, b)| b["child"].as_str().unwrap_or("").to_string()).filter(|c| c.len() >= 12).ok_or_else(|| format!("the habitat grafted no organism `{organism}`"))?
    };
    let key = &child[..12];
    // the habitat's Forgejo: the organism's user, token and repository
    // there (its code pushed when the habitat knows where it lives)
    let body = if provider == "forgejo" {
        let root = latest(&rows, "habitat.allocated").values().find_map(|b| (b["organism"] == child.as_str() && !b["root"].as_str().unwrap_or("").is_empty()).then(|| b["root"].as_str().unwrap_or("").to_string())).unwrap_or_else(|| "-".to_string());
        forge_body_for(&child, organism, &root, "forgejo")?
    } else {
        role_body(&child, "forge", provider, "", "", repo, slot)
    };
    let field = |k: &str| body[k].as_str().unwrap_or("").to_string();
    let (repo, slot) = (field("repo"), field("key_vault"));
    let (repo, slot) = (repo.as_str(), slot.as_str());
    record(&[serde_json::json!({ "kind": "habitat.allocated", "entity": format!("{key}/forge"), "body": body.to_string() })])?;
    forge_cache(key, provider, repo, slot, &field("address"))?;
    Ok(format!("{:<8}{organism}'s forge: {provider}{}{}", "forge", if repo.is_empty() { String::new() } else { format!(" {repo}") }, if slot.is_empty() { String::new() } else { format!(" (its token in the vault slot {slot})") }))
}

#[cfg(test)]
mod role_tests {
    use super::*;

    const CHILD: &str = "0123456789abcdef0123456789abcdef01234567";

    fn args(line: &str) -> Vec<String> {
        line.split(' ').map(str::to_string).collect()
    }

    /// An option's value is the word after it, never a word equal to it
    /// elsewhere: a slot named like the organism or the provider leaves both.
    #[test]
    fn forge_options_are_consumed_by_position() {
        let a = forge_args(&args("alpha github --repo owner/project --key-vault alpha")).unwrap();
        assert_eq!((a.organism.as_str(), a.provider.as_str(), a.repo.as_str(), a.key_vault.as_str()), ("alpha", "github", "owner/project", "alpha"));
        let b = forge_args(&args("alpha github --key-vault github --repo o/r --no-head")).unwrap();
        assert_eq!((b.organism.as_str(), b.provider.as_str(), b.key_vault.as_str(), b.no_head), ("alpha", "github", "github", true));
        let c = forge_args(&args("--repo o/r alpha github")).unwrap();
        assert_eq!((c.organism.as_str(), c.provider.as_str(), c.key_vault.as_str()), ("alpha", "github", ""));
        assert!(forge_args(&args("alpha github --repo")).is_none(), "an option without its value");
        assert!(forge_args(&args("alpha")).is_none() && forge_args(&args("alpha github extra")).is_none());
        assert!(forge_args(&args("alpha github --other x")).is_none());
    }

    /// A graft that stopped part way finishes on its retry: absent rows are
    /// recorded, a missing cache is written from the forge row as it stands
    /// (a Board allocation kept), and a whole allocation needs nothing.
    #[test]
    fn roles_resume_without_overwriting() {
        let key = &CHILD[..12];
        let (rows, cache) = roles_needed(&BTreeMap::new(), None, CHILD, None, "http://i");
        assert_eq!(rows.len(), 2);
        assert_eq!(cache.unwrap()["provider"], "none");

        let mut allocated = BTreeMap::new();
        allocated.insert(format!("{key}/forge"), role_body(CHILD, "forge", "github", "", "", "o/r", "slot"));
        let (rows, cache) = roles_needed(&allocated, None, CHILD, None, "http://i");
        assert_eq!(rows.len(), 1, "the identity alone");
        assert_eq!(rows[0]["entity"], format!("{key}/identity"));
        assert_eq!(cache.unwrap(), serde_json::json!({ "provider": "github", "repo": "o/r", "key_vault": "slot", "address": "" }), "the cache follows the row");

        allocated.insert(format!("{key}/identity"), role_body(CHILD, "identity", "oidc-stub", "local", "http://old", "", ""));
        let stale = serde_json::json!({ "provider": "none", "repo": "", "key_vault": "" });
        let (rows, cache) = roles_needed(&allocated, Some(stale), CHILD, None, "http://new");
        assert!(rows.is_empty(), "nothing recorded again");
        assert_eq!(cache.unwrap()["provider"], "github");

        let whole = serde_json::json!({ "provider": "github", "repo": "o/r", "key_vault": "slot", "address": "" });
        let fresh = role_body(CHILD, "forge", "file", "", "", "", "");
        assert_eq!(roles_needed(&allocated, Some(whole), CHILD, Some(&fresh), "http://new"), (Vec::new(), None));

        // a fresh Forgejo allocation is recorded as the host made it, and
        // its cache carries where the forge answers
        let fj = role_body(CHILD, "forge", "forgejo", "local", "http://127.0.0.1:16001", "shop/shop", "forgejo-0123456789ab");
        let (rows, cache) = roles_needed(&BTreeMap::new(), None, CHILD, Some(&fj), "http://i");
        assert!(rows[0]["body"].as_str().unwrap().contains("\"repo\":\"shop/shop\""), "{}", rows[0]);
        assert_eq!(cache.unwrap(), serde_json::json!({ "provider": "forgejo", "repo": "shop/shop", "key_vault": "forgejo-0123456789ab", "address": "http://127.0.0.1:16001" }));
    }
}

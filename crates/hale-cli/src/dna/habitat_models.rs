//! The habitat's models: the one organism it grafted that provides
//! `models` (`[dna.provides.models]`, `dna::provides`) is every organism's
//! model backend, with no person minting a key.
//!
//! For each tenant (every organism the habitat grafted, the provider
//! itself among them) once the provider's services are applied:
//!
//! - a member of the provider's `key` family is drawn in its vault,
//!   labelled by the tenant's name, and the provider's reader vault is
//!   restaged in place where its services run (`reader-vault-restage`), so
//!   its running api reads the new member;
//! - the member's value is copied into the tenant's slot
//!   `model-HABITAT_<KEY>` (its key upper-cased: the habitat's organisms
//!   share one vault, so the slot is named for the tenant);
//! - the provider is asked what it serves (`GET <its link><models_path>`,
//!   the tenant's key as a bearer token), from this machine, where the
//!   tenant's DNA runs;
//! - the tenant's model map is seeded in the name `hale habitat`
//!   (`provides::seed_rows`): never over a row in any other name.
//!
//! The graft does it for the organism it grafts (for every organism when
//! it grafts the provider); `hale habitat models <organism>|--all [--plan]`
//! does it again, for an organism grafted before the provider or a model
//! placed since.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use super::{genome_dir, host_run, local_habitat_dir, rows_of};
use crate::dna::provides;
use crate::dna::secrets;

/// An organism the habitat grafted, live.
#[derive(Debug, Clone)]
struct Organism {
    /// its allocations' key: the record's first 12 characters
    key: String,
    name: String,
    /// its record's identity
    record: String,
    root: PathBuf,
}

/// The habitat's models provider: the organism, and what it declares.
struct Provider {
    org: Organism,
    decl: provides::Models,
}

/// The organisms the habitat holds grafted, in the order it grafted them:
/// each entity's latest `habitat.grafted` row, one removed left out, its
/// directory from its allocation's cache.
fn grafted() -> Vec<Organism> {
    let mut order: Vec<String> = Vec::new();
    let mut latest: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    for r in rows_of(&genome_dir()).iter().filter(|r| r["kind"] == "habitat.grafted") {
        let Some(e) = r["entity"].as_str() else { continue };
        let body = r["body"].as_str().and_then(|b| serde_json::from_str::<serde_json::Value>(b).ok()).unwrap_or_default();
        if !latest.contains_key(e) {
            order.push(e.to_string());
        }
        latest.insert(e.to_string(), body);
    }
    order
        .into_iter()
        .filter_map(|key| {
            let body = latest.remove(&key)?;
            if body["state"].as_str() == Some("removed") {
                return None;
            }
            let cache = PathBuf::from(local_habitat_dir()).join("allocations").join(&key).join("allocation.json");
            let root = std::fs::read_to_string(&cache).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()).and_then(|v| v["root"].as_str().map(PathBuf::from))?;
            Some(Organism { name: body["name"].as_str().unwrap_or("").to_string(), record: body["child"].as_str().unwrap_or("").to_string(), key, root })
        })
        .collect()
}

/// What the organism at `root` provides, read from its manifest (a refused
/// declaration provides nothing).
fn provides_of(root: &Path) -> Option<provides::Models> {
    let text = std::fs::read_to_string(root.join("hale.toml")).ok()?;
    provides::declared(&text).ok().flatten()
}

/// The habitat's models provider among `orgs`: the first that provides
/// `models`.
fn provider_among(orgs: &[Organism]) -> Option<Provider> {
    orgs.iter().find_map(|o| provides_of(&o.root).map(|decl| Provider { org: o.clone(), decl }))
}

/// A graft of the organism whose manifest is `manifest` (pinning `pinned`
/// when its graft goes on), refused when it provides models and the
/// habitat holds another provider already, naming it: a habitat holds one.
/// A `[dna.provides]` its toolchain refuses refuses the graft, saying why.
pub(super) fn refuse_second_provider(manifest: &str, pinned: Option<&str>) -> Result<(), String> {
    let Some(_) = provides::declared(manifest).map_err(|why| format!("hale.toml: {why}"))? else { return Ok(()) };
    if let Some(p) = provider_among(&grafted()) {
        if Some(p.org.record.as_str()) != pinned {
            return Err(format!(
                "the habitat holds one models provider, and `{}` ({}…) provides its models already: `hale habitat organism remove {}` before grafting another",
                p.org.name, p.org.key, p.org.name
            ));
        }
    }
    Ok(())
}

/// The vault's entry `entry` holds a value.
fn held(dir: &Path, entry: &str) -> Option<String> {
    std::fs::read_to_string(dir.join(entry)).ok().filter(|v| !v.is_empty())
}

/// `value` into the vault's entry `entry`, mode 600 (the directory 700
/// when it is made).
fn vault_put(dir: &Path, entry: &str, value: &str) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
    if !dir.is_dir() {
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    }
    let path = dir.join(entry);
    let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&path).map_err(|e| format!("write {}: {e}", path.display()))?;
    f.write_all(value.as_bytes()).map_err(|e| format!("write {}: {e}", path.display()))?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).map_err(|e| format!("chmod {}: {e}", path.display()))
}

/// A value drawn from urandom: 32 hex characters, as the habitat draws
/// every secret it owns.
fn draw() -> Result<String, String> {
    use std::io::Read;
    let mut b = [0u8; 16];
    std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b)).map_err(|e| format!("no randomness: {e}"))?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

/// The tenant's slot its key at the habitat's models is copied into.
pub(super) fn tenant_slot(key: &str) -> String {
    format!("model-HABITAT_{}", key.to_ascii_uppercase())
}

/// Where the provider's service is reached from this machine, as the
/// head's `Commands::allocations` links it: its URL, or why there is none
/// to point a tenant at.
fn provider_url(p: &Provider) -> Result<String, String> {
    let v = super::sapling::head_call("Commands::allocations", serde_json::json!({ "limit": 1000 })).map_err(|e| format!("the habitat's head did not say where {} runs ({e})", p.org.name))?;
    let rows: Vec<serde_json::Value> = v["allocations"].as_str().and_then(|t| serde_json::from_str(t).ok()).unwrap_or_default();
    let allocation = format!("{}/services", p.org.key);
    let row = rows.iter().find(|r| r["allocation"] == allocation.as_str()).ok_or_else(|| format!("{}'s services are not placed ({allocation}): nothing serves the habitat's models yet", p.org.name))?;
    let links = row["links"].as_array().cloned().unwrap_or_default();
    let link = links.iter().find(|l| l["service"] == p.decl.service.as_str()).ok_or_else(|| format!("{}'s `{}` publishes no port (its services are {}): nothing serves the habitat's models yet", p.org.name, p.decl.service, row["state"].as_str().filter(|s| !s.is_empty()).unwrap_or("not applied")))?;
    match link["url"].as_str().filter(|u| !u.is_empty()) {
        Some(u) => Ok(u.trim_end_matches('/').to_string()),
        None => Err(format!("{}'s `{}` is {}: the habitat's organisms do not reach it, and no map is pointed at it", p.org.name, p.decl.service, link["note"].as_str().filter(|n| !n.is_empty()).unwrap_or("published nowhere they reach"))),
    }
}

/// What the provider serves, asked at `url` with the key `key` as a bearer
/// token (handed to curl on its stdin, never its arguments).
fn served_at(url: &str, key: &str) -> Result<Vec<provides::Served>, String> {
    use std::io::Write;
    let mut child = Command::new("curl")
        .args(["-s", "-m", "15", "-w", "\n%{http_code}", "-H", "@-", url])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("curl: {e}"))?;
    if let Some(mut w) = child.stdin.take() {
        w.write_all(format!("Authorization: Bearer {key}\n").as_bytes()).map_err(|e| format!("curl: {e}"))?;
    }
    let out = child.wait_with_output().map_err(|e| format!("curl: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let (body, code) = text.rsplit_once('\n').unwrap_or((&text, "0"));
    match code.trim() {
        "200" => provides::served(body),
        "000" | "0" | "" => Err(format!("{url} did not answer")),
        c => Err(format!("{url} answered {c}")),
    }
}

/// The tenant's latest row of each `(kind, entity)` of its model map.
fn map_rows(root: &Path) -> BTreeMap<(String, String), serde_json::Value> {
    let mut out = BTreeMap::new();
    for r in rows_of(root) {
        let kind = r["kind"].as_str().unwrap_or("");
        if kind != "model.rule" && kind != "model.category" {
            continue;
        }
        let body = r["body"].as_str().and_then(|b| serde_json::from_str::<serde_json::Value>(b).ok()).unwrap_or_default();
        out.insert((kind.to_string(), r["entity"].as_str().unwrap_or("").to_string()), body);
    }
    out
}

/// The habitat's models wired for the tenant `t` from the provider `p`:
/// what was done (or, with `plan`, what would be), a line each. An error
/// is what stopped it, said by the caller as a note: a graft never fails
/// on it.
fn wire(t: &Organism, p: &Provider, plan: bool) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let tag = format!("{:<8}", "models");
    let url = provider_url(p).map_err(|why| format!("{why}: `hale habitat models {}` maps them once it is", t.name))?;
    let label = provides::tenant_label(&t.name)?;
    let vault = crate::dna::vault_local_dir().ok_or("the vault is served over HTTP (HALE_VAULT_ADDR): the habitat draws its models' keys into a local vault alone")?;
    let member = secrets::member_entry(&p.org.record, &p.decl.key, &label);
    let slot = tenant_slot(&t.key);
    let models_url = format!("{url}{}", p.decl.models_path);
    let endpoint = format!("{url}{}", p.decl.path);
    let key = match held(&vault, &member) {
        Some(k) => {
            out.push(format!("{tag}{}'s key at {} ({member}) is held", t.name, p.org.name));
            k
        }
        None if plan => {
            out.push(format!("{tag}would draw {}'s key at {}: the member `{label}` of `{}` ({member}), and restage {}'s reader vault where its services run", t.name, p.org.name, p.decl.key, p.org.name));
            out.push(format!("{tag}would copy it into {}'s slot {slot}, ask {models_url} what it serves, and map {}'s models there ({endpoint}) in the name `{}`", t.name, t.name, provides::SEEDED_BY));
            return Ok(out);
        }
        None => {
            let k = draw()?;
            vault_put(&vault, &member, &k)?;
            out.push(format!("{tag}{}'s key at {} drawn ({member})", t.name, p.org.name));
            match host_run("reader-vault-restage", &genome_dir(), &[p.org.key.clone()]) {
                Ok(_) => out.push(format!("{tag}{}'s reader vault restaged where its services run: its api reads the new member", p.org.name)),
                Err(e) => out.push(format!("{:<8}{}'s reader vault was not restaged ({e}): its api reads the new member once its services are next applied", "note", p.org.name)),
            }
            k
        }
    };
    if plan {
        out.push(format!("{tag}would copy it into {}'s slot {slot}", t.name));
    } else if held(&vault, &slot).as_deref() != Some(key.as_str()) {
        vault_put(&vault, &slot, &key)?;
        out.push(format!("{tag}copied into {}'s slot {slot}", t.name));
    }
    let served = match served_at(&models_url, &key) {
        Ok(s) if !s.is_empty() => s,
        Ok(_) => {
            out.push(format!("{tag}the habitat's models ({}) serve none yet: hale habitat models {} maps them once one is served", p.org.name, t.name));
            return Ok(out);
        }
        Err(why) => {
            out.push(format!("{tag}the habitat's models ({}) serve none yet ({why}): hale habitat models {} maps them once one is served", p.org.name, t.name));
            return Ok(out);
        }
    };
    let rows = provides::seed_rows(&served, &map_rows(&t.root), &endpoint, &slot);
    let think = served[0].id.clone();
    let tools = served.iter().find(|m| m.tools).map(|m| m.id.clone());
    let said = format!("think = {think}; tools, do = {}", tools.as_deref().unwrap_or("none (no served model returns tool calls)"));
    if rows.is_empty() {
        out.push(format!("{tag}{}'s map already says the habitat's models ({said}), or a person's rows hold it", t.name));
        return Ok(out);
    }
    if plan {
        out.push(format!("{tag}would map {} row(s) of {}'s model map at {endpoint} ({said}) in the name `{}`", rows.len(), t.name, provides::SEEDED_BY));
        return Ok(out);
    }
    let text: String = rows.iter().map(|(kind, entity, body)| serde_json::json!({ "kind": kind, "entity": entity, "body": body.to_string() }).to_string() + "\n").collect();
    let dna_dir = t.root.join(".hale/dna");
    std::fs::create_dir_all(&dna_dir).map_err(|e| format!("create {}: {e}", dna_dir.display()))?;
    let file = dna_dir.join(format!("habitat-models-rows.{}.jsonl", std::process::id()));
    std::fs::write(&file, &text).map_err(|e| format!("write {}: {e}", file.display()))?;
    let seeded = host_run("record-seed", &t.root, &[file.to_string_lossy().to_string()]);
    let _ = std::fs::remove_file(&file);
    let n: usize = seeded?.trim().parse().map_err(|e| format!("record-seed answered oddly: {e}"))?;
    out.push(format!("{tag}{}'s model map points at the habitat's models ({}, {endpoint}): {said}; {n} row(s) in the name `{}`", t.name, p.org.name, provides::SEEDED_BY));
    Ok(out)
}

/// `wire`, its refusal a note.
fn wire_lines(t: &Organism, p: &Provider, plan: bool) -> Vec<String> {
    wire(t, p, plan).unwrap_or_else(|why| vec![format!("{:<8}{}'s map is not pointed at the habitat's models: {why}", "note", t.name)])
}

/// After a graft of the organism at `root`: its map pointed at the
/// habitat's models, or, when it is the provider, every organism's. A
/// line each; nothing fails the graft.
pub(super) fn after_graft(root: &Path) -> Vec<String> {
    let Some(record) = super::manifest_pin(&root.join("hale.toml")).ok().flatten() else { return Vec::new() };
    let orgs = grafted();
    let Some(t) = orgs.iter().find(|o| o.record == record) else { return Vec::new() };
    let Some(p) = provider_among(&orgs) else {
        return vec![format!("{:<8}the habitat has no models provider: graft one (voice), and its models are every organism's (`hale habitat models {}`)", "models", t.name)];
    };
    if p.org.record == t.record {
        // the provider itself: every organism it holds, the provider among them
        return orgs.iter().flat_map(|o| wire_lines(o, &p, false)).collect();
    }
    wire_lines(t, &p, false)
}

/// After `hale dna secret rotate <family> --member <label>` in the
/// provider's directory `root`: the tenant `label`'s copy updated and the
/// provider's reader vault restaged. A line each.
pub(super) fn after_rotate(root: &Path, family: &str, label: &str) -> Vec<String> {
    let orgs = grafted();
    let Some(p) = provider_among(&orgs) else { return Vec::new() };
    let same = |a: &Path, b: &Path| a.canonicalize().ok() == b.canonicalize().ok();
    if !same(&p.org.root, root) || p.decl.key != family {
        return Vec::new();
    }
    let mut out = Vec::new();
    match host_run("reader-vault-restage", &genome_dir(), &[p.org.key.clone()]) {
        Ok(_) => out.push(format!("{:<8}{}'s reader vault restaged where its services run", "models", p.org.name)),
        Err(e) => out.push(format!("{:<8}{}'s reader vault was not restaged ({e}): its api reads the member once its services are next applied", "note", p.org.name)),
    }
    let Some(t) = orgs.iter().find(|o| o.name == label) else {
        out.push(format!("{:<8}no organism `{label}` is grafted: no tenant's copy to update", "note"));
        return out;
    };
    let Some(vault) = crate::dna::vault_local_dir() else { return out };
    let member = secrets::member_entry(&p.org.record, &p.decl.key, label);
    match held(&vault, &member).map(|k| vault_put(&vault, &tenant_slot(&t.key), &k)) {
        Some(Ok(())) => out.push(format!("{:<8}{}'s copy updated ({})", "models", t.name, tenant_slot(&t.key))),
        Some(Err(e)) => out.push(format!("{:<8}{}'s copy was not updated: {e}", "note", t.name)),
        None => out.push(format!("{:<8}the vault holds no {member}", "note")),
    }
    out
}

pub(super) const USAGE: &str = "usage: hale habitat models <organism>|--all [--plan]";

/// `hale habitat models <organism>|--all [--plan]`.
pub(super) fn cmd(args: &[String]) -> ExitCode {
    let plan = args.iter().any(|a| a == "--plan");
    let rest: Vec<&String> = args.iter().filter(|a| a.as_str() != "--plan").collect();
    let (all, name) = match rest.as_slice() {
        [a] if a.as_str() == "--all" => (true, String::new()),
        [n] if !n.starts_with('-') => (false, n.to_string()),
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    if let Some(p) = super::remote::pointer() {
        let mut a: Vec<&str> = vec!["models"];
        a.extend(args.iter().map(String::as_str));
        return super::remote::run_there(&p, "models", &a);
    }
    let orgs = grafted();
    let Some(p) = provider_among(&orgs) else {
        eprintln!("hale habitat models: the habitat has no models provider: graft an organism whose hale.toml declares [dna.provides.models] (voice)");
        return ExitCode::from(1);
    };
    let targets: Vec<&Organism> = if all { orgs.iter().collect() } else { orgs.iter().filter(|o| o.name == name || o.key == name).collect() };
    match targets.as_slice() {
        [] => {
            eprintln!("hale habitat models: the habitat grafted no organism `{name}`");
            return ExitCode::from(1);
        }
        [_, _, ..] if !all => {
            eprintln!("hale habitat models: the habitat grafted more than one organism named `{name}`: name it by its key");
            return ExitCode::from(1);
        }
        _ => {}
    }
    let mut failed = false;
    for t in targets {
        match wire(t, &p, plan) {
            Ok(lines) => lines.iter().for_each(|l| println!("{l}")),
            Err(why) => {
                failed = true;
                eprintln!("hale habitat models {}: {why}", t.name);
            }
        }
    }
    if failed {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tenants_slot_is_a_model_key_named_for_it() {
        assert_eq!(tenant_slot("0a1b2c3d4e5f"), "model-HABITAT_0A1B2C3D4E5F");
        // `hale dna secret set|rotate <NAME>` takes it: A-Z, 0-9, _
        let name = tenant_slot("0a1b2c3d4e5f")[6..].to_string();
        assert!(name.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'), "{name}");
    }

    #[test]
    fn a_drawn_key_is_32_hex_characters() {
        let k = draw().unwrap();
        assert_eq!(k.len(), 32);
        assert!(k.chars().all(|c| c.is_ascii_hexdigit()), "{k}");
        assert_ne!(k, draw().unwrap());
    }
}

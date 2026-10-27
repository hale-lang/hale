//! `hale dna pack list | show | apply` — packs from a nursery.
//!
//! A nursery is a directory or a git repository with `nursery.toml` at its
//! root, indexing packs: each a batch of steps in `hale dna apply`'s
//! language with `{param}` placeholders. With no `--from`, the nursery is
//! the toolchain's (`dna/nursery`, embedded). `--from <dir>` reads a
//! directory; `--from <git-url>[#<rev>]` clones the repository into the
//! cache and reads it at that revision (its default branch's head without
//! one). Applying a pack renders it with `--set <param>=<value>` and the
//! index's defaults, and proposes the steps as one batch, the family named
//! `pack/<name>@<version>` and the pack's digest and source recorded with it
//! (`batch.proposed`'s `pack`).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use serde::Deserialize;

const USAGE: &str = "usage: hale dna pack list [--from <dir|git-url[#rev]>]\n       hale dna pack show <name> [--set <param>=<value>]… [--from …]\n       hale dna pack apply <name> [--set <param>=<value>]… [--from …] [--dry-run] [--as <who>] [--on-holes] [project]\n       hale dna pack retire <name> [--dry-run] [--as <who>] [project]";

#[derive(Deserialize)]
struct Index {
    #[serde(default)]
    pack: Vec<Entry>,
    #[serde(default)]
    sapling: Vec<Sapling>,
}

/// A sapling: an organism's seed, pinned by repository, revision and
/// digest, and the packs it adopts at graft (by name, from its own nursery
/// or the toolchain's), with the parameters they take (phase 4). Packs stay
/// what they are; a sapling only names them.
#[derive(Deserialize, Clone)]
pub(super) struct Sapling {
    pub(super) name: String,
    #[serde(default)]
    pub(super) summary: String,
    pub(super) repo: String,
    #[serde(default)]
    pub(super) revision: String,
    #[serde(default)]
    pub(super) digest: String,
    #[serde(default)]
    pub(super) packs: Vec<String>,
    #[serde(default)]
    pub(super) set: BTreeMap<String, String>,
}

#[derive(Deserialize, Clone)]
struct Entry {
    name: String,
    version: String,
    path: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    params: BTreeMap<String, String>,
}

/// A nursery read: its index, how to read a pack's text, and what to
/// record as its source.
pub(super) struct Nursery {
    entries: Vec<Entry>,
    pub(super) saplings: Vec<Sapling>,
    root: Option<PathBuf>,
    pub(super) source: String,
}

impl Nursery {
    fn text(&self, path: &str) -> Result<String, String> {
        match &self.root {
            Some(root) => fs::read_to_string(root.join(path)).map_err(|e| format!("{}: {e}", root.join(path).display())),
            None => hale_dna::NURSERY_FILES
                .iter()
                .find(|f| f.path == format!("dna/nursery/{path}"))
                .map(|f| f.content.to_string())
                .ok_or_else(|| format!("the toolchain's nursery has no `{path}`")),
        }
    }
}

fn index_of(text: &str, what: &str) -> Result<(Vec<Entry>, Vec<Sapling>), String> {
    let idx: Index = toml::from_str(text).map_err(|e| format!("{what}: nursery.toml: {e}"))?;
    for e in &idx.pack {
        if e.name.is_empty() || e.name.contains(char::is_whitespace) || e.version.is_empty() || e.path.is_empty() {
            return Err(format!("{what}: nursery.toml: a pack needs a one-word name, a version and a path"));
        }
    }
    for s in &idx.sapling {
        let word = |w: &str| !w.is_empty() && !w.contains(char::is_whitespace);
        // `repo = "."` with no revision or digest: the nursery's own
        // repository, pinned where it is read (`own_saplings`)
        let own = s.repo == OWN_REPO && s.revision.is_empty() && s.digest.is_empty();
        let pinned = word(&s.repo) && s.repo != OWN_REPO && word(&s.revision) && s.digest.starts_with("sha256:") && s.digest.len() == 71;
        if !word(&s.name) || s.name.contains('/') || !(own || pinned) {
            return Err(format!("{what}: nursery.toml: a sapling needs a one-word name (no `/`) and either a repository, a revision and a `sha256:` digest, or `repo = \".\"` alone (the nursery's own repository)"));
        }
    }
    Ok((idx.pack, idx.sapling))
}

fn stage_key(s: &str) -> String {
    openssl::sha::sha256(s.as_bytes()).iter().take(8).map(|b| format!("{b:02x}")).collect()
}

pub(super) fn staged_nursery_dir(source: &str) -> PathBuf {
    PathBuf::from(super::local_habitat_dir()).join("nurseries").join(stage_key(source))
}

pub(super) fn staged_repo_dir(repo: &str) -> PathBuf {
    PathBuf::from(super::local_habitat_dir()).join("nurseries").join("repos").join(stage_key(repo))
}

fn relative_pack_path(path: &str) -> Result<&Path, String> {
    let p = Path::new(path);
    if p.is_absolute()
        || p.components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return Err(format!(
            "nursery.toml: pack path `{path}` must stay under the nursery root"
        ));
    }
    Ok(p)
}

fn stage_nursery_into(root: &Path, entries: &[Entry], hab: &Path, dst: &Path) -> Result<(), String> {
    let _ = fs::remove_dir_all(dst);
    fs::create_dir_all(dst).map_err(|e| format!("{}: {e}", dst.display()))?;
    fs::copy(root.join("nursery.toml"), dst.join("nursery.toml"))
        .map_err(|e| format!("{}: {e}", dst.join("nursery.toml").display()))?;
    for e in entries {
        let rel = relative_pack_path(&e.path)?;
        let src = root.join(rel);
        if src.starts_with(hab) || src.starts_with(dst) {
            continue;
        }
        if src.is_file() {
            let target = dst.join(rel);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|err| format!("{}: {err}", parent.display()))?;
            }
            fs::copy(&src, &target).map_err(|err| format!("{}: {err}", target.display()))?;
        }
    }
    Ok(())
}

pub(super) fn stage_local_repo(repo: &str) {
    let hab = PathBuf::from(super::local_habitat_dir());
    if !hab.is_dir() || repo.starts_with("toolchain:") {
        return;
    }
    let hab = hab.canonicalize().unwrap_or(hab);
    let src = Path::new(repo);
    if !src.exists() || src.starts_with(&hab) {
        return;
    }
    let dst = staged_repo_dir(repo);
    let _ = fs::remove_dir_all(&dst);
    if let Some(parent) = dst.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = Command::new("git")
        .args(["clone", "-q", "--mirror", repo])
        .arg(&dst)
        .output();
}

fn stage_local_nursery(
    from: &str,
    root: &Path,
    entries: &[Entry],
    saplings: &[Sapling],
) -> Result<(), String> {
    let hab = PathBuf::from(super::local_habitat_dir());
    if !hab.is_dir() {
        return Ok(());
    }
    let hab = hab.canonicalize().unwrap_or(hab);
    if root.starts_with(&hab) {
        return Ok(());
    }
    let canon = root.display().to_string();
    let dst = staged_nursery_dir(&canon);
    stage_nursery_into(root, entries, &hab, &dst)?;
    if from != canon {
        let alias = staged_nursery_dir(from);
        stage_nursery_into(root, entries, &hab, &alias)?;
    }
    for s in saplings {
        stage_local_repo(&s.repo);
    }
    Ok(())
}

/// The digest a sapling is pinned by: the sha256 of the tree at a commit,
/// as `git ls-tree -r --full-tree` lists it (each file's mode, kind, blob
/// hash and path), `sha256:<hex>`. The commit too.
pub(crate) fn digest_at(repo: &Path, revision: &str) -> Result<(String, String), String> {
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

/// A sapling's `repo` naming the nursery's own repository.
const OWN_REPO: &str = ".";

/// The saplings of the nursery's own repository (`repo = "."`) pinned where
/// the nursery was read: the repository it was read from (`repo`), the
/// commit it was read at in `at`, and that tree's digest. So a repository
/// can offer itself as a sapling without naming its own commit, which no
/// file in it can; a later read at a newer commit is a proposal, as for any
/// sapling.
fn own_saplings(saplings: &mut [Sapling], repo: &str, at: &Path, commit: &str) -> Result<(), String> {
    for s in saplings.iter_mut().filter(|s| s.repo == OWN_REPO) {
        let (c, d) = digest_at(at, commit).map_err(|e| format!("the sapling `{}` is the nursery's own repository, which is not a git repository at {}: {e}", s.name, at.display()))?;
        s.repo = repo.to_string();
        s.revision = c;
        s.digest = d;
    }
    Ok(())
}

/// The nursery `--from` names, or the toolchain's.
pub(super) fn nursery(from: Option<&str>) -> Result<Nursery, String> {
    let Some(from) = from else {
        let text = hale_dna::NURSERY_FILES
            .iter()
            .find(|f| f.path == "dna/nursery/nursery.toml")
            .map(|f| f.content)
            .unwrap_or("");
        let (entries, saplings) = index_of(text, "the toolchain's nursery")?;
        if saplings.iter().any(|s| s.repo == OWN_REPO) {
            return Err("the toolchain's nursery has no repository of its own to offer (`repo = \".\"`)".to_string());
        }
        return Ok(Nursery { entries, saplings, root: None, source: format!("toolchain {}", env!("CARGO_PKG_VERSION")) });
    };
    let local = Path::new(from);
    if local.is_dir() {
        let root = local.canonicalize().map_err(|e| format!("{from}: {e}"))?;
        let text = fs::read_to_string(root.join("nursery.toml")).map_err(|_| format!("{from}: no nursery.toml at its root"))?;
        let (entries, mut saplings) = index_of(&text, from)?;
        own_saplings(&mut saplings, &root.display().to_string(), &root, "HEAD")?;
        stage_local_nursery(from, &root, &entries, &saplings)?;
        return Ok(Nursery { entries, saplings, source: root.display().to_string(), root: Some(root) });
    }
    let staged = staged_nursery_dir(from);
    if staged.is_dir() {
        let text = fs::read_to_string(staged.join("nursery.toml")).map_err(|_| format!("{from}: no nursery.toml at its root"))?;
        let (entries, mut saplings) = index_of(&text, from)?;
        own_saplings(&mut saplings, from, &staged, "HEAD")?;
        return Ok(Nursery { entries, saplings, source: from.to_string(), root: Some(staged) });
    }
    // a git repository: cloned once into the cache, fetched, and read at
    // the revision named (the default branch's head without one)
    let (url, rev) = match from.split_once('#') {
        Some((u, r)) => (u, Some(r)),
        None => (from, None),
    };
    let cache = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .ok_or("no cache directory (set XDG_CACHE_HOME or HOME)")?
        .join("hale/nursery");
    let key: String = openssl::sha::sha256(url.as_bytes()).iter().take(8).map(|b| format!("{b:02x}")).collect();
    let dir = cache.join(key);
    let git = |args: &[&str], cwd: Option<&Path>| -> Result<String, String> {
        let mut c = Command::new("git");
        if let Some(d) = cwd {
            c.arg("-C").arg(d);
        }
        let o = c.args(args).output().map_err(|e| format!("git: {e}"))?;
        if !o.status.success() {
            return Err(format!("git {}: {}", args.join(" "), String::from_utf8_lossy(&o.stderr).trim()));
        }
        Ok(String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    if dir.join(".git").is_dir() {
        git(&["fetch", "-q", "--tags", "origin"], Some(&dir))?;
    } else {
        fs::create_dir_all(&cache).map_err(|e| format!("{}: {e}", cache.display()))?;
        git(&["clone", "-q", url, &dir.to_string_lossy()], None)?;
    }
    let want = rev.map(|r| r.to_string()).unwrap_or_else(|| "origin/HEAD".to_string());
    let commit = git(&["rev-parse", "--verify", &format!("{want}^{{commit}}")], Some(&dir)).map_err(|_| format!("{from}: `{want}` names no commit of the nursery"))?;
    git(&["checkout", "-q", "--detach", &commit], Some(&dir))?;
    let text = fs::read_to_string(dir.join("nursery.toml")).map_err(|_| format!("{url}: no nursery.toml at its root"))?;
    let (entries, mut saplings) = index_of(&text, url)?;
    own_saplings(&mut saplings, url, &dir, &commit)?;
    Ok(Nursery { entries, saplings, root: Some(dir), source: format!("{url}@{commit}") })
}

/// The pack's text with its parameters set: every `{param}` replaced, a
/// `? ` line kept only when every parameter it names is set, and a
/// parameter with no value and no default refused by name.
fn render(text: &str, params: &BTreeMap<String, String>, set: &BTreeMap<String, String>) -> Result<String, String> {
    for k in set.keys() {
        if !params.contains_key(k) {
            return Err(format!("`--set {k}=…`: the pack takes no parameter `{k}` (it takes: {})", params.keys().cloned().collect::<Vec<_>>().join(", ")));
        }
    }
    let value = |k: &str| set.get(k).or_else(|| params.get(k)).cloned().unwrap_or_default();
    let mut out = String::new();
    for line in text.lines() {
        let (optional, body) = match line.strip_prefix("? ") {
            Some(rest) => (true, rest),
            None => (false, line),
        };
        let mut rendered = String::new();
        let mut rest = body;
        let mut missing = Vec::new();
        while let Some(open) = rest.find('{') {
            let Some(close) = rest[open..].find('}') else { break };
            let name = &rest[open + 1..open + close];
            rendered.push_str(&rest[..open]);
            if params.contains_key(name) {
                let v = value(name);
                if v.is_empty() && !missing.iter().any(|m| m == name) {
                    missing.push(name.to_string());
                }
                rendered.push_str(&v);
            } else {
                rendered.push_str(&rest[open..=open + close]);
            }
            rest = &rest[open + close + 1..];
        }
        rendered.push_str(rest);
        if !missing.is_empty() {
            if optional {
                continue;
            }
            return Err(format!("the pack needs {}", missing.iter().map(|m| format!("`--set {m}=…`")).collect::<Vec<_>>().join(", ")));
        }
        out.push_str(&rendered);
        out.push('\n');
    }
    Ok(out)
}

/// The pack `name` of the nursery `n` (or, when it offers none by that
/// name, of the toolchain's) proposed to the organism at `dir`, planned on
/// the holes its birth proposed, with each of `set` the pack takes: what a
/// sapling adopts at graft. What the host said, or why it was not proposed.
pub(super) fn propose_pack(n: &Nursery, name: &str, set: &BTreeMap<String, String>, dir: &Path) -> Result<String, String> {
    let (from, entry) = match n.entries.iter().find(|e| e.name == name) {
        Some(e) => (None, e.clone()),
        None => {
            let t = nursery(None)?;
            let e = t.entries.iter().find(|e| e.name == name).cloned().ok_or_else(|| format!("no pack `{name}` in {} nor in the toolchain's nursery", n.source))?;
            (Some(t), e)
        }
    };
    let n = from.as_ref().unwrap_or(n);
    let text = n.text(&entry.path)?;
    let taken: BTreeMap<String, String> = set.iter().filter(|(k, _)| entry.params.contains_key(*k)).map(|(k, v)| (k.clone(), v.clone())).collect();
    let steps = render(&text, &entry.params, &taken).map_err(|e| format!("the pack `{name}`: {e}"))?;
    let digest: String = openssl::sha::sha256(text.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
    let args = vec!["--steps".to_string(), steps, "--pack".to_string(), format!("{}@{} sha256:{digest} {}", entry.name, entry.version, n.source), "--on-holes".to_string()];
    super::host_run("apply", dir, &args)
}

pub(super) fn pack_cmd(args: &[String]) -> ExitCode {
    let sub = args.first().map(String::as_str);
    let mut from: Option<String> = None;
    let mut set = BTreeMap::new();
    let mut words = Vec::new();
    let mut forwarded = Vec::new();
    let mut i = 1;
    while i < args.len() {
        let a = args[i].as_str();
        let value = || args.get(i + 1).cloned();
        match a {
            "--from" | "--set" | "--as" => {
                let Some(v) = value() else {
                    eprintln!("hale dna pack: `{a}` takes a value\n{USAGE}");
                    return ExitCode::from(2);
                };
                match a {
                    "--from" => from = Some(v),
                    "--set" => {
                        let Some((k, val)) = v.split_once('=') else {
                            eprintln!("hale dna pack: `--set {v}`: write it `--set <param>=<value>`");
                            return ExitCode::from(2);
                        };
                        set.insert(k.trim().to_string(), val.trim().to_string());
                    }
                    _ => {
                        forwarded.push(a.to_string());
                        forwarded.push(v);
                    }
                }
                i += 2;
            }
            "--dry-run" | "--on-holes" => {
                forwarded.push(a.to_string());
                i += 1;
            }
            _ if a.starts_with('-') => {
                eprintln!("hale dna pack: `{a}` is not a flag of pack\n{USAGE}");
                return ExitCode::from(2);
            }
            _ => {
                words.push(a.to_string());
                i += 1;
            }
        }
    }
    // a git nursery's packs are adopted only once the Board trusts it
    // (`hale dna nursery add`), and nothing is fetched from one it does not;
    // the toolchain's and a directory of one's own need no trust
    if sub == Some("apply") {
        if let Some(url) = from.as_deref().filter(|f| !Path::new(f).is_dir()) {
            let url = url.split_once('#').map(|(u, _)| u).unwrap_or(url);
            let dir = words.get(1).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
            let trusted = super::host_run("nursery-sources", &dir, &[]).unwrap_or_default();
            // a nursery the habitat's Board trusts (`hale habitat nursery
            // add`) is trusted for its tenants too
            if !trusted.lines().any(|l| l.trim() == url) && !super::habitat::trusts_nursery(url) {
                eprintln!("hale dna pack: the Board does not trust the nursery {url} yet: `hale dna nursery add {url}` proposes it");
                return ExitCode::FAILURE;
            }
        }
    }
    let n = match nursery(from.as_deref()) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("hale dna pack: {e}");
            return ExitCode::FAILURE;
        }
    };
    let find = |name: Option<&String>| -> Result<Entry, String> {
        let Some(name) = name else { return Err(USAGE.to_string()) };
        n.entries.iter().find(|e| &e.name == name).cloned().ok_or_else(|| {
            format!("no pack `{name}` in {} (it offers: {})", n.source, n.entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>().join(", "))
        })
    };
    match sub {
        Some("list") | None => {
            println!("the nursery: {}", n.source);
            for e in &n.entries {
                let params: Vec<String> = e.params.iter().map(|(k, v)| if v.is_empty() { k.clone() } else { format!("{k}={v}") }).collect();
                println!("  {}@{} — {}{}", e.name, e.version, e.summary, if params.is_empty() { String::new() } else { format!(" ({})", params.join(", ")) });
            }
            println!("show one with `hale dna pack show <name>`; adopt one with `hale dna pack apply <name> --set <param>=<value>`");
            ExitCode::SUCCESS
        }
        Some("show") | Some("apply") => {
            let entry = match find(words.first()) {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("hale dna pack: {e}");
                    return ExitCode::from(2);
                }
            };
            let text = match n.text(&entry.path) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("hale dna pack: {e}");
                    return ExitCode::FAILURE;
                }
            };
            let steps = match render(&text, &entry.params, &set) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("hale dna pack {}: {e}", entry.name);
                    return ExitCode::from(2);
                }
            };
            let digest: String = openssl::sha::sha256(text.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
            if sub == Some("show") {
                println!("{}@{} — {}\nfrom {} (sha256:{digest})\n", entry.name, entry.version, entry.summary, n.source);
                print!("{steps}");
                return ExitCode::SUCCESS;
            }
            let dir = words.get(1).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
            // the steps travel to the host as a flag's value, whole: the host
            // keeps an argument's newlines (dna/host/main.hl, `escape_arg`)
            let mut host_args = vec!["--steps".to_string(), steps, "--pack".to_string(), format!("{}@{} sha256:{digest} {}", entry.name, entry.version, n.source)];
            host_args.extend(forwarded);
            super::host_exec("apply", &dir, &host_args)
        }
        Some("retire") => {
            // the latest adoption the Board ratified, undone as one batch
            let Some(name) = words.first() else {
                eprintln!("{USAGE}");
                return ExitCode::from(2);
            };
            let dir = words.get(1).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
            let mut host_args = vec!["--retire-pack".to_string(), name.clone()];
            host_args.extend(forwarded);
            super::host_exec("apply", &dir, &host_args)
        }
        Some(other) => {
            eprintln!("hale dna pack: `{other}` is not a pack verb (list, show, apply, retire)\n{USAGE}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(p: &[(&str, &str)]) -> BTreeMap<String, String> {
        p.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn a_pack_renders_with_its_parameters() {
        let text = "open {part}/dev | Decides: {part}.\n? fill {part}/dev {dev}\n";
        let p = params(&[("part", ""), ("dev", "")]);
        assert_eq!(render(text, &p, &params(&[("part", "api")])).unwrap(), "open api/dev | Decides: api.\n");
        assert_eq!(render(text, &p, &params(&[("part", "api"), ("dev", "ada")])).unwrap(), "open api/dev | Decides: api.\nfill api/dev ada\n");
        let none = render("open {part}/dev | {part} and {part}\n", &p, &params(&[])).unwrap_err();
        assert!(none.ends_with("the pack needs `--set part=…`"), "each parameter named once: {none}");
        assert!(render(text, &p, &params(&[("parts", "x")])).unwrap_err().contains("no parameter `parts`"));
    }

    #[test]
    fn the_toolchains_nursery_reads_whole() {
        let n = nursery(None).unwrap();
        assert!(!n.entries.is_empty());
        for e in &n.entries {
            let text = n.text(&e.path).unwrap();
            let mut set = BTreeMap::new();
            for (k, v) in &e.params {
                if v.is_empty() {
                    set.insert(k.clone(), "x".to_string());
                }
            }
            let steps = render(&text, &e.params, &set).unwrap();
            assert!(!steps.contains('{'), "{} renders every parameter: {steps}", e.name);
        }
    }

    /// The toolchain's nursery carries the organism that makes the
    /// toolchain (`hale`) and no application: an application such as voice
    /// is a nursery of its own, which the Board trusts.
    #[test]
    fn the_toolchains_nursery_carries_the_hale_organism_alone() {
        let n = nursery(None).unwrap();
        let names: Vec<&str> = n.saplings.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["hale"]);
        assert!(n.saplings.iter().all(|s| s.packs.iter().all(|p| n.entries.iter().any(|e| &e.name == p))), "its packs are the nursery's");
        assert!(index_of("[[sapling]]\nname = \"a/b\"\nrepo = \"r\"\nrevision = \"v\"\ndigest = \"sha256:x\"\n", "t").is_err(), "a malformed sapling is refused");
    }

    /// A repository offers itself: `repo = "."` alone is accepted and pinned
    /// where the nursery is read, at that commit with that tree's digest; a
    /// `.` with a revision, or a missing revision elsewhere, is refused.
    #[test]
    fn a_repository_offers_itself_pinned_where_it_is_read() {
        let own = "[[sapling]]\nname = \"app\"\nrepo = \".\"\n";
        let (_, mut saplings) = index_of(own, "t").unwrap();
        assert!(index_of("[[sapling]]\nname = \"app\"\nrepo = \".\"\nrevision = \"main\"\n", "t").is_err(), "`.` takes no revision");
        assert!(index_of("[[sapling]]\nname = \"app\"\nrepo = \"https://x/y\"\n", "t").is_err(), "another repository is pinned");
        let dir = std::env::temp_dir().join(format!("hale_own_sapling_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("nursery.toml"), own).unwrap();
        let git = |args: &[&str]| assert!(Command::new("git").arg("-C").arg(&dir).args(args).output().unwrap().status.success(), "git {args:?}");
        git(&["init", "-q"]);
        git(&["add", "nursery.toml"]);
        git(&["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "x"]);
        own_saplings(&mut saplings, "https://example.org/app", &dir, "HEAD").unwrap();
        let (commit, digest) = digest_at(&dir, "HEAD").unwrap();
        assert_eq!((saplings[0].repo.as_str(), saplings[0].revision.as_str(), saplings[0].digest.as_str()), ("https://example.org/app", commit.as_str(), digest.as_str()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn staging_a_nursery_with_a_nested_habitat_copies_only_declared_packs() {
        let dir = std::env::temp_dir().join(format!("hale_nursery_stage_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("packs")).unwrap();
        let hab = dir.join(".habitat");
        let dst = hab.join("nurseries").join("abc");
        fs::create_dir_all(&hab).unwrap();
        fs::write(dir.join("nursery.toml"), "[[pack]]\nname = \"ops\"\nversion = \"1\"\npath = \"packs/ops.pack\"\n").unwrap();
        fs::write(dir.join("packs/ops.pack"), "open ops/dev | Decides: ops.\n").unwrap();
        fs::write(hab.join("sentinel"), "do not copy").unwrap();
        let (entries, _) = index_of(&fs::read_to_string(dir.join("nursery.toml")).unwrap(), "t").unwrap();
        stage_nursery_into(&dir, &entries, &hab, &dst).unwrap();
        assert!(dst.join("nursery.toml").is_file());
        assert!(dst.join("packs/ops.pack").is_file());
        assert!(!dst.join(".habitat").exists());
        assert!(relative_pack_path("../outside.pack").is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}

/// `hale dna nursery add <source> [--dry-run] [--as <who>] [project]`: a
/// nursery proposed for the Board to trust, as a one-step batch (`nursery
/// <source>`); `hale dna nursery list [project]`: the ones it trusts.
pub(super) fn nursery_cmd(args: &[String]) -> ExitCode {
    match args.first().map(String::as_str) {
        Some("list") => {
            let dir = args.get(1).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
            super::host_exec("nursery-sources", &dir, &[])
        }
        Some("add") => {
            let Some(source) = args.get(1) else {
                eprintln!("usage: hale dna nursery add <dir|git-url> [--dry-run] [--as <who>] [project]");
                return ExitCode::from(2);
            };
            let mut forwarded = Vec::new();
            let mut dir = PathBuf::from(".");
            let mut i = 2;
            while i < args.len() {
                match args[i].as_str() {
                    "--dry-run" => {
                        forwarded.push(args[i].clone());
                        i += 1;
                    }
                    "--as" if i + 1 < args.len() => {
                        forwarded.push(args[i].clone());
                        forwarded.push(args[i + 1].clone());
                        i += 2;
                    }
                    a if !a.starts_with('-') => {
                        dir = PathBuf::from(a);
                        i += 1;
                    }
                    a => {
                        eprintln!("hale dna nursery: `{a}` is not a flag of add (--dry-run, --as)");
                        return ExitCode::from(2);
                    }
                }
            }
            let mut host_args = vec!["--steps".to_string(), format!("nursery {source}\n")];
            host_args.extend(forwarded);
            super::host_exec("apply", &dir, &host_args)
        }
        _ => {
            eprintln!("usage: hale dna nursery add <dir|git-url> [--dry-run] [--as <who>] [project]\n       hale dna nursery list [project]");
            ExitCode::from(2)
        }
    }
}

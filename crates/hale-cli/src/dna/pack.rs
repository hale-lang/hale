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

const USAGE: &str = "usage: hale dna pack list [--from <dir|git-url[#rev]>]\n       hale dna pack show <name> [--set <param>=<value>]… [--from …]\n       hale dna pack apply <name> [--set <param>=<value>]… [--from …] [--dry-run] [--as <who>] [project]\n       hale dna pack retire <name> [--dry-run] [--as <who>] [project]";

#[derive(Deserialize)]
struct Index {
    #[serde(default)]
    pack: Vec<Entry>,
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
struct Nursery {
    entries: Vec<Entry>,
    root: Option<PathBuf>,
    source: String,
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

fn index_of(text: &str, what: &str) -> Result<Vec<Entry>, String> {
    let idx: Index = toml::from_str(text).map_err(|e| format!("{what}: nursery.toml: {e}"))?;
    for e in &idx.pack {
        if e.name.is_empty() || e.name.contains(char::is_whitespace) || e.version.is_empty() || e.path.is_empty() {
            return Err(format!("{what}: nursery.toml: a pack needs a one-word name, a version and a path"));
        }
    }
    Ok(idx.pack)
}

/// The nursery `--from` names, or the toolchain's.
fn nursery(from: Option<&str>) -> Result<Nursery, String> {
    let Some(from) = from else {
        let text = hale_dna::NURSERY_FILES
            .iter()
            .find(|f| f.path == "dna/nursery/nursery.toml")
            .map(|f| f.content)
            .unwrap_or("");
        return Ok(Nursery { entries: index_of(text, "the toolchain's nursery")?, root: None, source: format!("toolchain {}", env!("CARGO_PKG_VERSION")) });
    };
    let local = Path::new(from);
    if local.is_dir() {
        let root = local.canonicalize().map_err(|e| format!("{from}: {e}"))?;
        let text = fs::read_to_string(root.join("nursery.toml")).map_err(|_| format!("{from}: no nursery.toml at its root"))?;
        return Ok(Nursery { entries: index_of(&text, from)?, source: root.display().to_string(), root: Some(root) });
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
    Ok(Nursery { entries: index_of(&text, url)?, root: Some(dir), source: format!("{url}@{commit}") })
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
            "--dry-run" => {
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
}

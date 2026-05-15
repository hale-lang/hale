//! Simple git-based package fetching for Aperio.
//!
//! v1: single developer, no transitive deps, no registry. The
//! user writes `aperio.toml` at the repo root listing direct git
//! dependencies; `aperio fetch` shells out to `git clone` for
//! each one into `lib/<name>/` and pins the resolved commit SHA
//! in `aperio.lock`.
//!
//! Re-fetching is idempotent: if `lib/<name>/.git/HEAD` already
//! matches the locked SHA for that dep, we skip the network. To
//! upgrade, edit the manifest's `rev`/`tag`/`branch`, delete
//! `aperio.lock` (or just `lib/<name>/`), and re-run `fetch`.
//!
//! Path resolution downstream is already in place — the parser's
//! `import "lib/x" as alias;` directive resolves relative to the
//! importer's directory then the workspace root, finding the
//! cloned source automatically. See `spec/projects.md`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

/// `aperio.toml` at the repo root.
#[derive(Deserialize, Default, Debug)]
pub struct Manifest {
    #[serde(default)]
    pub deps: BTreeMap<String, DepSpec>,
}

/// One entry in the `[deps]` table. Exactly zero or one of
/// `rev` / `tag` / `branch` may be set; zero means "default
/// branch". v1 doesn't support version ranges — only specific
/// refs (commit SHAs, tags, branch names).
#[derive(Deserialize, Clone, Debug)]
pub struct DepSpec {
    pub git: String,
    #[serde(default)]
    pub rev: Option<String>,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
}

impl DepSpec {
    /// Returns an error if more than one of {rev, tag, branch}
    /// is set — the spec must be unambiguous.
    fn validate(&self, name: &str) -> Result<(), String> {
        let set = [&self.rev, &self.tag, &self.branch]
            .iter()
            .filter(|x| x.is_some())
            .count();
        if set > 1 {
            return Err(format!(
                "dep `{}` declares more than one of {{rev, tag, branch}}; \
                 pick one",
                name
            ));
        }
        Ok(())
    }
}

/// `aperio.lock` — pins every dep to a resolved commit SHA so
/// re-cloning is reproducible across machines.
#[derive(Serialize, Deserialize, Default, Debug)]
pub struct Lockfile {
    #[serde(default, rename = "dep")]
    pub deps: Vec<LockedDep>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LockedDep {
    pub name: String,
    pub git: String,
    pub sha: String,
}

/// Top-level entry. Reads `aperio.toml` from `repo_root`, ensures
/// every declared dep is cloned under `lib/<name>/` at the right
/// commit, and writes a fresh `aperio.lock`. Existing
/// `aperio.lock` entries are honored — if a dep is already at its
/// locked SHA, no network call is made.
pub fn fetch(repo_root: &Path) -> Result<(), String> {
    let manifest_path = repo_root.join("aperio.toml");
    if !manifest_path.exists() {
        return Err(format!(
            "no aperio.toml at {} — create one with a [deps] section, \
             then run `aperio fetch` again",
            manifest_path.display()
        ));
    }
    let manifest = read_manifest(&manifest_path)?;
    if manifest.deps.is_empty() {
        println!("aperio.toml has no deps; nothing to fetch");
        return Ok(());
    }
    for (name, spec) in &manifest.deps {
        spec.validate(name)?;
    }

    let lock_path = repo_root.join("aperio.lock");
    let prev_locked: BTreeMap<String, String> = read_lockfile(&lock_path)?
        .deps
        .into_iter()
        .map(|d| (d.name, d.sha))
        .collect();

    let lib_dir = repo_root.join("lib");
    fs::create_dir_all(&lib_dir)
        .map_err(|e| format!("create lib/: {}", e))?;

    let mut new_lock = Lockfile { deps: Vec::new() };
    for (name, spec) in &manifest.deps {
        let target = lib_dir.join(name);
        let sha = fetch_one(name, spec, &target, prev_locked.get(name))?;
        new_lock.deps.push(LockedDep {
            name: name.clone(),
            git: spec.git.clone(),
            sha,
        });
    }

    let lock_text = toml::to_string_pretty(&new_lock)
        .map_err(|e| format!("serialize lockfile: {}", e))?;
    fs::write(&lock_path, lock_text)
        .map_err(|e| format!("write {}: {}", lock_path.display(), e))?;
    println!("wrote {}", lock_path.display());
    Ok(())
}

fn read_manifest(path: &Path) -> Result<Manifest, String> {
    let src = fs::read_to_string(path)
        .map_err(|e| format!("read {}: {}", path.display(), e))?;
    toml::from_str(&src).map_err(|e| format!("parse {}: {}", path.display(), e))
}

fn read_lockfile(path: &Path) -> Result<Lockfile, String> {
    if !path.exists() {
        return Ok(Lockfile::default());
    }
    let src = fs::read_to_string(path)
        .map_err(|e| format!("read {}: {}", path.display(), e))?;
    toml::from_str(&src).map_err(|e| format!("parse {}: {}", path.display(), e))
}

/// Ensure `target` is a checked-out clone of `spec.git` at the
/// requested ref. Returns the resolved commit SHA.
fn fetch_one(
    name: &str,
    spec: &DepSpec,
    target: &Path,
    locked_sha: Option<&String>,
) -> Result<String, String> {
    let already_cloned = target.join(".git").exists();

    if already_cloned {
        let cur = git_head(target)?;
        // If we have a locked SHA and the current HEAD matches,
        // there's nothing to do — skip the network.
        if let Some(want) = locked_sha {
            if &cur == want {
                println!("{}: up to date ({})", name, short_sha(&cur));
                return Ok(cur);
            }
        }
        // Otherwise: fetch + checkout the requested ref.
        run_git(target, &["fetch", "--tags", "--prune", "origin"])?;
        let r = resolve_ref(spec);
        run_git(target, &["checkout", "--quiet", &r])?;
    } else {
        let parent = target.parent().ok_or_else(|| {
            format!("target {} has no parent", target.display())
        })?;
        fs::create_dir_all(parent)
            .map_err(|e| format!("mkdir: {}", e))?;
        match (&spec.rev, &spec.tag, &spec.branch) {
            // Pinning by SHA requires a full clone — `--depth 1`
            // with `--branch <sha>` isn't valid git.
            (Some(rev), None, None) => {
                run_in(parent, &["clone", "--quiet", &spec.git, name])?;
                run_git(target, &["checkout", "--quiet", rev])?;
            }
            // Tag or branch: shallow clone is fine.
            (None, Some(r), None) | (None, None, Some(r)) => {
                run_in(
                    parent,
                    &["clone", "--quiet", "--depth", "1", "--branch", r, &spec.git, name],
                )?;
            }
            // No pin: shallow clone of default branch.
            (None, None, None) => {
                run_in(parent, &["clone", "--quiet", "--depth", "1", &spec.git, name])?;
            }
            _ => unreachable!("validate() rejects multi-pin specs"),
        }
    }

    let sha = git_head(target)?;
    println!("{}: at {}", name, short_sha(&sha));
    Ok(sha)
}

fn resolve_ref(spec: &DepSpec) -> String {
    spec.rev
        .clone()
        .or_else(|| spec.tag.clone())
        .or_else(|| spec.branch.clone())
        .unwrap_or_else(|| "HEAD".to_string())
}

fn git_head(repo: &Path) -> Result<String, String> {
    let out = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo)
        .output()
        .map_err(|e| format!("git rev-parse: {}", e))?;
    if !out.status.success() {
        return Err(format!(
            "git rev-parse failed in {}: {}",
            repo.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn run_git(repo: &Path, args: &[&str]) -> Result<(), String> {
    let status = Command::new("git")
        .args(args)
        .current_dir(repo)
        .status()
        .map_err(|e| format!("git: {}", e))?;
    if !status.success() {
        return Err(format!("git {:?} failed in {}", args, repo.display()));
    }
    Ok(())
}

fn run_in(dir: &Path, args: &[&str]) -> Result<(), String> {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .map_err(|e| format!("git: {}", e))?;
    if !status.success() {
        return Err(format!("git {:?} failed in {}", args, dir.display()));
    }
    Ok(())
}

fn short_sha(sha: &str) -> &str {
    if sha.len() >= 12 {
        &sha[..12]
    } else {
        sha
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_manifest() {
        let src = r#"
            [deps]
            helpers = { git = "https://example.com/helpers.git" }
        "#;
        let m: Manifest = toml::from_str(src).expect("parse");
        assert_eq!(m.deps.len(), 1);
        let h = &m.deps["helpers"];
        assert_eq!(h.git, "https://example.com/helpers.git");
        assert!(h.rev.is_none() && h.tag.is_none() && h.branch.is_none());
    }

    #[test]
    fn parses_rev_tag_branch() {
        let src = r#"
            [deps]
            a = { git = "u", rev = "abc123" }
            b = { git = "u", tag = "v0.1.0" }
            c = { git = "u", branch = "main" }
        "#;
        let m: Manifest = toml::from_str(src).expect("parse");
        assert_eq!(m.deps["a"].rev.as_deref(), Some("abc123"));
        assert_eq!(m.deps["b"].tag.as_deref(), Some("v0.1.0"));
        assert_eq!(m.deps["c"].branch.as_deref(), Some("main"));
    }

    #[test]
    fn rejects_multi_pin() {
        let s = DepSpec {
            git: "u".into(),
            rev: Some("a".into()),
            tag: Some("b".into()),
            branch: None,
        };
        assert!(s.validate("x").is_err());
    }

    #[test]
    fn round_trips_lockfile() {
        let lock = Lockfile {
            deps: vec![
                LockedDep {
                    name: "helpers".into(),
                    git: "https://example.com/helpers.git".into(),
                    sha: "abc1234567890abcdef".into(),
                },
                LockedDep {
                    name: "finance".into(),
                    git: "https://example.com/finance.git".into(),
                    sha: "deadbeefcafef00d".into(),
                },
            ],
        };
        let text = toml::to_string_pretty(&lock).expect("serialize");
        let parsed: Lockfile = toml::from_str(&text).expect("parse");
        assert_eq!(parsed.deps.len(), 2);
        assert_eq!(parsed.deps[0].name, "helpers");
        assert_eq!(parsed.deps[1].sha, "deadbeefcafef00d");
    }
}

// Suppress unused warning when only the tests reference PathBuf
// (the function bodies above use it directly via &Path).
#[allow(dead_code)]
fn _phantom_pathbuf_use() -> PathBuf {
    PathBuf::new()
}

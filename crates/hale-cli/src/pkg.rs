//! Simple git-based package fetching for Hale.
//!
//! v1: single developer, no transitive deps, no registry. The
//! user writes `hale.toml` at the repo root listing direct git
//! dependencies; `hale fetch` shells out to `git clone` for
//! each one into `vendor/<name>/` and pins the resolved commit
//! SHA in `hale.lock`.
//!
//! The fetched tree lives at `vendor/` (toolchain-managed), kept
//! separate from `lib/` (hand-maintained by the user). This
//! avoids clobbering hand-vendored source if the user adds a
//! manifest dep that happens to share a name with an existing
//! `lib/<name>/` directory. Consumers reference fetched deps via
//! `import "vendor/<name>" as alias;` (the path is part of the
//! import string, so swapping a hand-vendored lib for a fetched
//! one is an explicit edit, not a silent rebind).
//!
//! Re-fetching is idempotent: if `vendor/<name>/.git/HEAD`
//! already matches the locked SHA for that dep, we skip the
//! network. To upgrade, edit the manifest's `rev`/`tag`/`branch`,
//! delete `hale.lock` (or just `vendor/<name>/`), and re-run
//! `fetch`.
//!
//! Path resolution downstream is already in place — the parser's
//! `import "vendor/x" as alias;` directive resolves relative to
//! the importer's directory then the workspace root, finding the
//! cloned source automatically. See `spec/packages.md` for the
//! full surface and `spec/projects.md` for the resolver.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

/// `hale.toml` at the repo root (or any vendored / hand-
/// maintained lib's root).
#[derive(Deserialize, Default, Debug)]
pub struct Manifest {
    #[serde(default)]
    pub deps: BTreeMap<String, DepSpec>,
    /// Stage-2 FFI (2026-05-22): `[ffi]` section declares the
    /// C-side surface needed to link this lib. Picked up by
    /// `hale build` when this lib is imported — its `link`
    /// libs append to the clang `-l` line and its `csrc` files
    /// compile alongside the runtime. Empty / absent for libs
    /// that don't use `@ffi("c")` declarations.
    #[serde(default)]
    pub ffi: FfiManifest,
    /// GH #409: `[environments.<name>]` — which claimset each
    /// deployment target requires, and which entrypoints deploy
    /// there.
    ///
    /// The constitution is bound HERE rather than in source because
    /// it is a property of where you are deploying, not of the
    /// program: one entrypoint deployed to two environments must
    /// satisfy both claimsets, and it cannot say two different
    /// `adopt` lines. A source-level `adopt` still means "always,
    /// everywhere"; the two compose by union like everything else in
    /// this system.
    #[serde(default)]
    pub environments: BTreeMap<String, EnvSpec>,
    /// GH #409: `[claims] base = "Core"` — a constitution every
    /// environment carries.
    ///
    /// This is what makes "an environment may add law, never drop
    /// it" true of the MECHANISM rather than only of `extends`.
    /// Without it the manifest can bind unrelated constitutions to
    /// dev and prod, and monotonicity is a documentation promise
    /// nothing enforces. With it, every matrix evaluation adopts the
    /// base plus whatever the environment adds — monotone by
    /// construction, which beats a check that has to notice a
    /// violation after the fact.
    #[serde(default)]
    pub claims: ClaimsManifest,
}

/// The `[claims]` section.
#[derive(Deserialize, Default, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct ClaimsManifest {
    pub base: Option<String>,
    /// `no_base = true` — this workspace deliberately has no shared
    /// base, so environments may bind unrelated constitutions.
    ///
    /// Required rather than inferred, for the same reason
    /// `source_only` is: an absent `[claims]` section is
    /// indistinguishable from a misspelled one (`[claim]`), and the
    /// intended workspace baseline would vanish while every
    /// environment still looked explicit and valid.
    #[serde(default)]
    pub no_base: bool,
}

/// One `[environments.<name>]` section.
///
/// `deny_unknown_fields` is load-bearing: without it a misspelled
/// `constituton = "Prod"` parses fine, leaves `constitution` as
/// `None`, and the entrypoint still counts as environment-bound —
/// a typo silently removing all environment law from a deployment
/// that reports success.
#[derive(Deserialize, Default, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct EnvSpec {
    /// The constitution every listed entrypoint must satisfy when
    /// deployed here.
    ///
    /// Absent is an ERROR unless `source_only` says so explicitly.
    /// "No environment law" is a real configuration, but it must be
    /// stated rather than inferred from an omission — an omission is
    /// indistinguishable from a mistake, and this feature exists to
    /// stop law going missing quietly.
    pub constitution: Option<String>,
    /// `source_only = true` — this environment adds no law of its
    /// own; the entrypoints' source-level `adopt` lines (and the
    /// workspace base, if declared) are the whole of it.
    #[serde(default)]
    pub source_only: bool,
    /// Seed directories, relative to the manifest. An entrypoint
    /// listed in NO environment is an error rather than a skip:
    /// silently unconstrained is the failure mode this whole
    /// feature exists to remove.
    #[serde(default)]
    pub entrypoints: Vec<String>,
}

/// `[ffi]` section of `hale.toml`. Paths in `csrc` are resolved
/// relative to the `hale.toml`'s own directory; `link` entries
/// are library names handed to the linker as `-l<name>`.
#[derive(Deserialize, Default, Clone, Debug)]
pub struct FfiManifest {
    #[serde(default)]
    pub link: Vec<String>,
    #[serde(default)]
    pub csrc: Vec<String>,
}

/// Read just the `[ffi]` section from a lib's `hale.toml` if
/// it has one. Returns `None` when the file doesn't exist or has
/// no `[ffi]` section; that's the steady state for pure-Hale
/// libs and isn't an error condition.
pub fn read_lib_ffi(lib_dir: &Path) -> Result<Option<FfiManifest>, String> {
    let manifest_path = lib_dir.join("hale.toml");
    if !manifest_path.exists() {
        return Ok(None);
    }
    let src = fs::read_to_string(&manifest_path)
        .map_err(|e| format!("read {}: {}", manifest_path.display(), e))?;
    let m: Manifest = toml::from_str(&src)
        .map_err(|e| format!("parse {}: {}", manifest_path.display(), e))?;
    if m.ffi.link.is_empty() && m.ffi.csrc.is_empty() {
        return Ok(None);
    }
    Ok(Some(m.ffi))
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

/// `hale.lock` — pins every dep to a resolved commit SHA so
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

/// Top-level entry. Reads `hale.toml` from `repo_root`, ensures
/// every declared dep is cloned under `lib/<name>/` at the right
/// commit, and writes a fresh `hale.lock`. Existing
/// `hale.lock` entries are honored — if a dep is already at its
/// locked SHA, no network call is made.
pub fn fetch(repo_root: &Path) -> Result<(), String> {
    let manifest_path = repo_root.join("hale.toml");
    if !manifest_path.exists() {
        return Err(format!(
            "no hale.toml at {} — create one with a [deps] section, \
             then run `hale fetch` again",
            manifest_path.display()
        ));
    }
    let manifest = read_manifest(&manifest_path)?;
    if manifest.deps.is_empty() {
        println!("hale.toml has no deps; nothing to fetch");
        return Ok(());
    }
    for (name, spec) in &manifest.deps {
        spec.validate(name)?;
    }

    let lock_path = repo_root.join("hale.lock");
    let prev_locked: BTreeMap<String, String> = read_lockfile(&lock_path)?
        .deps
        .into_iter()
        .map(|d| (d.name, d.sha))
        .collect();

    let vendor_dir = repo_root.join("vendor");
    fs::create_dir_all(&vendor_dir)
        .map_err(|e| format!("create vendor/: {}", e))?;

    let mut new_lock = Lockfile { deps: Vec::new() };
    for (name, spec) in &manifest.deps {
        let target = vendor_dir.join(name);
        // Refuse to clone over a directory the user planted by
        // hand: a vendor/<name>/ that exists but isn't a git
        // checkout is hand-maintained source, and silently
        // overwriting it would lose work.
        if target.exists() && !target.join(".git").exists() {
            return Err(format!(
                "vendor/{}/ exists but isn't a git checkout; refusing \
                 to overwrite hand-maintained source. Move or delete \
                 the directory and re-run `hale fetch`.",
                name
            ));
        }
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

/// GH #409: the `[environments.*]` table, or an empty map when the
/// manifest is absent. A missing `hale.toml` is not an error here —
/// most seeds have none — but a malformed one is.
/// The `[environments.*]` table and the `[claims] base`, validated.
pub fn read_claims_config(
    manifest: &Path,
) -> Result<(BTreeMap<String, EnvSpec>, Option<String>), String> {
    if !manifest.exists() {
        return Ok((BTreeMap::new(), None));
    }
    let src = fs::read_to_string(manifest)
        .map_err(|e| format!("read {}: {}", manifest.display(), e))?;
    let m: Manifest = toml::from_str(&src)
        .map_err(|e| format!("parse {}: {}", manifest.display(), e))?;
    for (name, spec) in &m.environments {
        match (&spec.constitution, spec.source_only) {
            (Some(_), true) => {
                return Err(format!(
                    "{}: environment `{}` sets both `constitution` \
                     and `source_only` — say one or the other",
                    manifest.display(),
                    name
                ))
            }
            (None, false) => {
                return Err(format!(
                    "{}: environment `{}` names no `constitution`. \
                     If it deliberately adds no law of its own, say \
                     `source_only = true` — an omission is \
                     indistinguishable from a typo, and a silently \
                     law-free deployment is what this is here to \
                     prevent",
                    manifest.display(),
                    name
                ))
            }
            _ => {}
        }
    }
    if !m.environments.is_empty() {
        match (&m.claims.base, m.claims.no_base) {
            (Some(_), true) => {
                return Err(format!(
                    "{}: `[claims]` sets both `base` and `no_base` — \
                     say one or the other",
                    manifest.display()
                ))
            }
            (None, false) => {
                return Err(format!(
                    "{}: declares environments but no `[claims] base`. \
                     Without one, environments may bind unrelated \
                     constitutions and \"an environment may add law, \
                     never drop it\" is not a property of the \
                     mechanism. Say `[claims] base = \"…\"`, or \
                     `[claims] no_base = true` if that is deliberate",
                    manifest.display()
                ))
            }
            _ => {}
        }
    }
    Ok((m.environments, m.claims.base))
}

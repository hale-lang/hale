//! A build leaves the source tree as it found it.
//!
//! `hale build` used to write a seed's binary beside its source, so the
//! tree collected untracked binaries and `.gitignore` grew one rule per
//! binary (some 25 by the time this landed) — rules that also hid the
//! next binary a fixture forgot to clean up. Now a seed is built with
//! `hale build -o` into `target/seeds/<name>/<name>` (or a scratch
//! directory a test owns), `hale test` / `run` / `replay` build into a
//! per-run scratch directory, and nothing compiled is ignored except
//! `target/`. Three things keep it that way, all scans of the tracked
//! files (like `dna_fixture_ports_are_free.rs`, which closes the class
//! rather than the instances):
//!
//!  1. `.gitignore` names no binary: not `<dir>/<dir>`, not `*/main`, not
//!     `*_test`, not the path of a seed's binary.
//!  2. No tracked file hard-codes the path a seed's binary would have
//!     beside its source (`<seed dir>/<seed dir's own name>`).
//!  3. The CI jobs that build seeds (`dna`, `face-api`, `face-browser`)
//!     end with `git status --porcelain`, and fail on anything a build
//!     left in the tree.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Files that legitimately name a `<seed>/<seed name>` path, each with
/// the reason. The scan fails on an entry with nothing left to allow.
const EXEMPT: &[(&str, &str)] = &[
    (
        "crates/hale-dna/src/lib.rs",
        "names the host inside the toolchain cache's private materialized copy of the sources, not a checkout",
    ),
    (
        "crates/hale-iris/src/lib.rs",
        "names iris's own binaries inside the toolchain cache's private materialized copy, not a checkout",
    ),
    (
        "scripts/warm-dna-cache.sh",
        "reads the host the toolchain cache built into its own copy ($iris_dir), not a checkout",
    ),
    (
        "dna/tests/body_lease_blocked_test.hl",
        "finds the host the toolchain cache built into its own copy of the sources, not a checkout",
    ),
    (
        "dna/tests/body_lease_start_test.hl",
        "finds the host the toolchain cache built into its own copy of the sources, not a checkout",
    ),
    (
        "dna/tests/head_roles_test.hl",
        "copies dna/ into the fixture's scratch root and builds THAT copy, so the binary sits beside a source inside the scratch root",
    ),
    (
        "CHANGELOG.md",
        "records history, and may name a path that no longer exists (the removed dna/oidc/serve/serve binary)",
    ),
];

/// `.gitignore` files that stand for a user's own project repository
/// (a fixture copy of one), which builds with a plain `hale build` and
/// ignores its own binary — the rule this repository dropped for itself.
const USER_PROJECT_GITIGNORES: &[&str] = &["dna/tests/onboarding/voice/"];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(repo_root().join(rel))
        .unwrap_or_else(|e| panic!("cannot read {rel}: {e}"))
}

/// Every tracked path that is still on disk, as `git ls-files` lists
/// them. The scans are over what is committed, so an untracked scratch
/// file cannot fail them (a file deleted but not yet committed is
/// skipped, not read).
fn tracked() -> Vec<String> {
    let out = Command::new("git")
        .args(["ls-files", "-z"])
        .current_dir(repo_root())
        .output()
        .expect("git ls-files (these scans read the tracked files)");
    assert!(out.status.success(), "git ls-files failed");
    String::from_utf8_lossy(&out.stdout)
        .split('\0')
        .filter(|p| !p.is_empty())
        .filter(|p| repo_root().join(p).exists())
        .map(str::to_string)
        .collect()
}

/// Each tracked `<dir>/main.hl` is a seed; its binary, built beside it,
/// would be `<dir>/<basename of dir>`.
fn seed_binaries(files: &[String]) -> BTreeSet<String> {
    files
        .iter()
        .filter_map(|f| f.strip_suffix("/main.hl"))
        .filter_map(|dir| {
            let name = Path::new(dir).file_name()?.to_str()?;
            Some(format!("{dir}/{name}"))
        })
        .collect()
}

#[test]
fn the_scan_sees_the_seeds() {
    let seeds = seed_binaries(&tracked());
    assert!(
        seeds.len() > 100,
        "expected the repository's ~200 seeds, saw {} — the scan reads nothing",
        seeds.len()
    );
    assert!(seeds.contains("dna/api/api"), "dna/api is a seed");
    assert!(seeds.contains("dna/host/host"), "dna/host is a seed");
}

/// The active rules of a `.gitignore`: no comments, no blanks.
fn rules(text: &str) -> Vec<&str> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect()
}

#[test]
fn gitignore_has_no_binary_rules() {
    let files = tracked();
    let seeds = seed_binaries(&files);
    let mut offenders = Vec::new();
    for f in files
        .iter()
        .filter(|f| *f == ".gitignore" || f.ends_with("/.gitignore"))
        .filter(|f| !USER_PROJECT_GITIGNORES.iter().any(|p| f.starts_with(p)))
    {
        let base = Path::new(f).parent().unwrap_or(Path::new(""));
        for rule in rules(&read(f)) {
            let bare = rule.trim_start_matches('!').trim_start_matches('/');
            let segs: Vec<&str> = bare.trim_end_matches('/').split('/').collect();
            let last = *segs.last().unwrap();
            let doubled = segs.len() >= 2 && segs[segs.len() - 2] == last;
            let shaped = last == "main"
                || last.starts_with("*_test")
                || last.ends_with("_test")
                || last.starts_with("repro[");
            let joined = base.join(bare).to_string_lossy().replace('\\', "/");
            if doubled || shaped || seeds.contains(&joined) {
                offenders.push(format!("{f}: {rule}"));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "these .gitignore rules hide a compiled binary ({} found):\n{:#?}\n\n\
         A build no longer writes a binary into the tree — `hale build -o <path>` \
         puts it under target/seeds/<name>/<name> (or a scratch directory), and \
         `hale test` builds into one it removes — so such a rule can only hide \
         the next fixture that forgot. Build with `-o` instead of ignoring the output.",
        offenders.len(),
        offenders
    );
}

/// The path is a whole token: not the tail of a longer name, and not
/// the head of a longer path.
fn names_path(text: &str, path: &str) -> bool {
    text.match_indices(path).any(|(i, _)| {
        let before = text[..i].chars().next_back();
        let after = text[i + path.len()..].chars().next();
        let word = |c: char| c.is_alphanumeric() || "_.-".contains(c);
        !before.map_or(false, word)
            && !after.map_or(false, |c| word(c) || c == '/')
    })
}

#[test]
fn no_tracked_file_hard_codes_a_seed_binary_beside_its_source() {
    let files = tracked();
    let seeds = seed_binaries(&files);
    const SELF: &str = "crates/hale-cli/tests/seeds_build_out_of_tree.rs";
    let exempt: BTreeSet<&str> = EXEMPT.iter().map(|(p, _)| *p).collect();
    let mut used: BTreeSet<&str> = BTreeSet::new();
    let mut offenders = Vec::new();
    for f in &files {
        if f == SELF || f.starts_with("unreleased/") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(repo_root().join(f)) else {
            continue; // not UTF-8: an image, a compiled fixture
        };
        for seed in &seeds {
            if !names_path(&text, seed) {
                continue;
            }
            if exempt.contains(f.as_str()) {
                used.insert(exempt.get(f.as_str()).unwrap());
            } else {
                offenders.push(format!("{f}: {seed}"));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "these files name the path a seed's binary has when built beside its \
         source ({} found):\n{:#?}\n\n\
         Nothing builds there any more. Build with `hale build <seed> -o \
         target/seeds/<name>/<name>` (scripts/warm-and-build.sh does) and name \
         that path — or, in a fixture, a scratch directory it owns. A file \
         that names such a path for a stated reason goes in `EXEMPT`.",
        offenders.len(),
        offenders
    );
    let stale: Vec<&&str> = exempt.iter().filter(|p| !used.contains(**p)).collect();
    assert!(
        stale.is_empty(),
        "EXEMPT names files that no longer name a seed binary: {stale:?}; delete the entries"
    );
}

/// The block of `.github/workflows/tests.yml` that is job `name`.
fn job_block<'a>(yml: &'a str, name: &str) -> &'a str {
    let head = format!("\n  {name}:\n");
    let start = yml
        .find(&head)
        .unwrap_or_else(|| panic!("tests.yml has no job `{name}`"));
    let rest = &yml[start + head.len()..];
    // The next job: a two-space-indented key at the left margin of `jobs:`.
    let end = rest
        .match_indices("\n  ")
        .find(|(i, _)| {
            let after = &rest[i + 3..];
            after.chars().next().map_or(false, |c| c.is_alphanumeric())
                && after.split('\n').next().unwrap_or("").ends_with(':')
        })
        .map(|(i, _)| i)
        .unwrap_or(rest.len());
    &rest[..end]
}

#[test]
fn the_jobs_that_build_seeds_end_by_checking_the_tree() {
    let yml = read(".github/workflows/tests.yml");
    let mut missing = Vec::new();
    for job in ["dna", "face-api", "face-browser"] {
        let block = job_block(&yml, job);
        let checks = block.contains("git status --porcelain")
            || block.contains("*tree-is-clean");
        if !checks {
            missing.push(job);
        }
    }
    assert!(
        missing.is_empty(),
        "these tests.yml jobs build seeds and never check that they left the \
         tree alone: {missing:?}. End each with the `tree-is-clean` step \
         (`git status --porcelain`, failing on any output)."
    );
    // The anchor's own body is the check; a rename that emptied it would
    // leave every alias passing.
    assert!(
        yml.contains("status=$(git status --porcelain --untracked-files=all)"),
        "the tree-is-clean step no longer runs `git status --porcelain`"
    );
}

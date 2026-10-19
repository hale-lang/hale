//! GH #1653: every organism but the habitat is grafted from the habitat,
//! and nothing births one another way.
//!
//! The free-standing births (`hale dna new`, `hale dna init`) are
//! retired: each exits 2 naming the graft that replaces it. So that no
//! test, fixture, script or page births an organism around the graft or
//! teaches one, this refuses a spelling of either verb anywhere in the
//! tree but the one spec sentence that says they are retired. And in the
//! toolchain itself, a record is born only by `habitat.rs`: the habitat at
//! `up` (`Lineage::Habitat`, the one birth with no parent) and a child at
//! `graft`.

#[path = "support/vault.rs"]
mod vault;

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Whether `text` spells a retired birth verb: `dna new` / `dna init` as
/// words (a shell line, or prose wrapped between `dna` and the verb), an
/// argv line `dna\ninit`, or an argument list `"dna", "new"`.
fn spells_a_retired_birth(text: &str) -> Vec<usize> {
    let mut lines = Vec::new();
    let words: Vec<(usize, &str)> = text
        .lines()
        .enumerate()
        .flat_map(|(n, l)| l.split(|c: char| c.is_whitespace() || c == '`' || c == '"' || c == '\'' || c == ',' || c == '[' || c == '(' || c == '/' || c == '#' || c == '*' || c == '|').filter(|w| !w.is_empty()).map(move |w| (n, w)))
        .collect();
    for pair in words.windows(2) {
        let (n, a) = pair[0];
        let (_, b) = pair[1];
        let verb = |w: &str| w == "new" || w == "init" || w.starts_with("new\\") || w.starts_with("init\\") || w == "new;" || w == "init;" || w == "new." || w == "init." || w == "new:" || w == "init:";
        if (a == "dna" && verb(b)) || a.ends_with("dna\\nnew") || a.ends_with("dna\\ninit") || a.contains("dna\\nnew\\") || a.contains("dna\\ninit\\") {
            lines.push(n + 1);
        }
    }
    lines.dedup();
    lines
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for p in entries.flatten().map(|e| e.path()) {
        let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if p.is_dir() {
            // vendored trees of other repositories are theirs, byte for byte
            if matches!(name.as_str(), "node_modules" | "target" | "test-results" | ".git" | "fixtures" | "onboarding") {
                continue;
            }
            collect(&p, out);
        } else if ["hl", "rs", "md", "mjs", "js", "sh", "py", "yml", "yaml", "toml", "pack"].iter().any(|x| name.ends_with(&format!(".{x}"))) {
            out.push(p);
        }
    }
}

#[test]
fn nothing_births_an_organism_around_the_graft() {
    let root = repo_root();
    let mut files = Vec::new();
    for d in ["crates", "dna", "docs/src", "spec", "scripts", ".github", "iris"] {
        collect(&root.join(d), &mut files);
    }
    for f in ["README.md", "AGENTS.md"] {
        files.push(root.join(f));
    }
    let me = root.join("crates/hale-cli/tests/births_go_through_graft.rs");
    let mut offenders = Vec::new();
    for f in &files {
        if *f == me {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(f) else { continue };
        for n in spells_a_retired_birth(&text) {
            let line = text.lines().nth(n - 1).unwrap_or("");
            let rel = f.strip_prefix(&root).unwrap_or(f).display().to_string();
            // the one sentence that says they are retired
            let near = (n.saturating_sub(2)..n + 1).filter_map(|i| text.lines().nth(i)).any(|l| l.contains("are retired"));
            if rel == "spec/dna.md" && near {
                continue;
            }
            offenders.push(format!("{rel}:{n}: {}", line.trim()));
        }
    }
    assert!(files.len() > 500, "only {} files scanned; the scan is broken", files.len());
    assert!(offenders.is_empty(), "a retired birth verb is spelled; an organism is grafted (`hale habitat graft <name>`, or `<dir> --attach`):\n{}", offenders.join("\n"));
}

#[test]
fn the_matcher_sees_every_spelling() {
    for spelled in [
        "exec \"$2\" dna new app",
        "let made = in_dir(scratch, toolchain() + \"\\ndna\\nnew\\nbooks\");",
        "hale(&[\"dna\", \"init\", \".\"], &app)",
        "boundedNative(hale, ['dna', 'new', name])",
        "`hale dna\n  init` seeds",
        "run `hale dna init` first",
    ] {
        assert_eq!(spells_a_retired_birth(spelled).len(), 1, "{spelled}");
    }
    for not in ["hale init demo", "git init -q", "dna newer", "the dna initial row", "hale habitat graft . --attach", "\"dna\", \"upgrade\""] {
        assert!(spells_a_retired_birth(not).is_empty(), "{not}");
    }
}

/// Only `habitat.rs` births a record: the habitat (`Lineage::Habitat`)
/// at `up`, a child (`init_under` / `new_project_under`) at `graft`.
#[test]
fn only_the_habitat_births_a_record() {
    let src = repo_root().join("crates/hale-cli/src");
    let mut files = Vec::new();
    collect(&src, &mut files);
    let mut offenders = Vec::new();
    for f in &files {
        let rel = f.strip_prefix(&src).unwrap().display().to_string();
        let text = std::fs::read_to_string(f).unwrap();
        for (n, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            let births = code.contains("Lineage::Habitat") || code.contains("init_under(") || code.contains("new_project_under(");
            let defines = code.contains("fn init_under(") || code.contains("fn new_project_under(") || code.contains("Lineage::Habitat =>");
            // the one call inside dna.rs: a new application's birth goes on
            // to the attach of the directory it made
            let internal = rel == "dna.rs" && code.contains("init_under(dir, library, Lineage::Child(parent))");
            if births && !defines && !internal && rel != "dna/habitat.rs" {
                offenders.push(format!("src/{rel}:{}: {}", n + 1, line.trim()));
            }
        }
    }
    assert!(offenders.is_empty(), "a record is born outside habitat.rs:\n{}", offenders.join("\n"));
}

/// The retired verbs exit 2 and name the graft that replaces them, with
/// the flags they were given.
#[test]
fn the_retired_verbs_name_their_graft() {
    let d = std::env::temp_dir();
    for (args, graft) in [
        (vec!["dna", "new", "demo", "--no-library"], "hale habitat graft demo --no-library"),
        (vec!["dna", "new", "bodied", "--profile", "remote-body", "--remote", "git@example.invalid:x/y.git"], "hale habitat graft bodied --profile remote-body --remote git@example.invalid:x/y.git"),
        (vec!["dna", "init"], "hale habitat graft . --attach"),
        (vec!["dna", "init", "app", "--with", "founder", "--set", "founder=alice", "--as", "alice"], "hale habitat graft app --attach --with founder --set founder=alice --as alice"),
    ] {
        let out = vault::hale().args(&args).current_dir(&d).output().expect("hale");
        let said = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(2), "{args:?}: {said}");
        assert!(said.contains("retired") && said.contains(graft), "{args:?} names `{graft}`: {said}");
        assert!(out.stdout.is_empty(), "{args:?} births nothing");
    }
}

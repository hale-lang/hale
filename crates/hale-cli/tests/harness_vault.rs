//! No test run touches the developer's vault (GH #989's Deferred line,
//! closed). `hale dna init` and `upgrade` provision the organism's secrets
//! into the vault and `hale dna secret set` fills a slot, so a test run
//! against the real one left a throwaway organization's entries there, or
//! a fake key in a real slot.
//!
//! Three paths keep a test off it. `hale test` runs every `_test.hl` file
//! under a vault of its own (`HALE_VAULT_DIR`, made for the run and
//! removed after it; `HALE_VAULT_ADDR` removed), which everything the
//! fixture starts inherits. A Rust test that runs a vault-writing
//! `hale dna` verb spawns `hale` through `support/vault.rs` (or sets
//! `HALE_VAULT_DIR` on its child). The face's Node harnesses take theirs
//! from `isolatedEnvironment()`.
//!
//! The first test runs `hale test -j 2` over two fixtures that each run
//! `hale dna new`: the organism's secrets go into two vaults, gone after,
//! and the caller's vault, the cache's vault and `~/.config` are left as
//! they were. The second holds every Rust DNA test to its path, every
//! Node allow-listed environment to carrying the override, and every test
//! source to never clearing it (nor the whole environment).

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

// A fixture that runs a vault-writing verb, `hale dna new` (whose init
// provisions the organism's secrets), and writes down which vault it ran
// under (never a value). Two of them, `ORG` apart, run side by side.
const PROBE: &str = r#"fn main() {
    let dir = std::secret::vault_local_dir();
    std::test::assert(!std::env::var_exists("HALE_VAULT_ADDR"), "no real vault reaches a test");
    std::test::assert(std::env::var_exists("HALE_VAULT_DIR") && dir == std::env::var("HALE_VAULT_DIR"), "the vault is the run's own: " + dir);
    std::test::assert(dir != std::env::var("CALLER_VAULT") && dir != std::env::var("XDG_CACHE_HOME") + "/hale/vault", "never the caller's, never the developer's: " + dir);
    std::io::fs::write_file(std::env::var("PROBE_ROOT") + "/ORG.vault", dir) or raise;
    let made = std::process::run("sh\n-c\ncd \"$1\" && HALE_DNA_DISCOVER=off exec \"$2\" dna new ORG\nsh\n" + std::env::var("PROBE_ROOT") + "\n" + std::env::var("HALE_BIN")) or raise;
    std::test::assert_eq_int(made.code, 0, "hale dna new: " + made.stdout + made.stderr);
    std::test::assert(std::secret::Credential { vault: "postgres-owner-ORG" }.ready(), "and the organism's secrets went into it");
}
"#;

// Every entry of a vault directory with its size and modification time.
fn listing(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(dir)
        .map(|es| {
            es.flatten()
                .map(|e| {
                    let m = e.metadata().ok();
                    format!("{:?} {:?} {:?}", e.file_name(), m.as_ref().map(|m| m.len()), m.and_then(|m| m.modified().ok()))
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

#[test]
fn hale_test_runs_each_file_under_a_vault_of_its_own() {
    let d = std::env::temp_dir().join(format!("hale_vault_probe_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    let home = d.join("home");
    let work = d.join("work");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&work).unwrap();
    std::fs::create_dir_all(d.join("probes")).unwrap();
    for org in ["probea", "probeb"] {
        std::fs::write(d.join(format!("probes/{org}_test.hl")), PROBE.replace("ORG", org)).unwrap();
    }
    // the toolchain cache the DNA tests share, warm, so the organism's
    // host is not built from cold; its vault is the "developer's" here
    let cache = std::env::temp_dir().join("hale-tests-iris-cache");
    let before = listing(&cache.join("hale/vault"));
    let caller_vault = home.join("caller-vault");
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["test", "-j", "2"])
        .arg(d.join("probes"))
        .current_dir(&d)
        .env("HOME", &home)
        .env("XDG_CACHE_HOME", &cache)
        .env("HALE_BIN", env!("CARGO_BIN_EXE_hale"))
        .env("PROBE_ROOT", &work)
        .env("CALLER_VAULT", &caller_vault)
        // a caller's own vault and a real vault named: neither reaches the test
        .env("HALE_VAULT_DIR", &caller_vault)
        .env("HALE_VAULT_ADDR", "http://127.0.0.1:9")
        .output()
        .expect("hale test");
    let said = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success() && said.contains("2 passed"), "both probes pass under hale test: {said}");
    let ran: Vec<String> = ["probea", "probeb"].iter().map(|o| std::fs::read_to_string(work.join(format!("{o}.vault"))).unwrap_or_default()).collect();
    assert!(!ran[0].is_empty() && ran[0] != ran[1], "two files side by side, two vaults: {ran:?}");
    assert!(ran.iter().all(|v| !Path::new(v).exists()), "and each is gone once its file has run: {ran:?}");
    assert!(!caller_vault.exists(), "the caller's vault was never written: {said}");
    assert_eq!(listing(&cache.join("hale/vault")), before, "the cache's own vault is untouched");
    assert!(!home.join(".config").exists(), "nothing was written under ~/.config: {said}");
    let _ = std::fs::remove_dir_all(&d);
}

// Whether `text` starts the toolchain itself: `Command::new(env!(..hale))`
// in any spelling, or through a binding (`let bin = env!(..hale);` then
// `Command::new(bin)` / `(&bin)`). Whitespace is squashed first, so a call
// split across lines is the same call.
fn spawns_hale_directly(text: &str) -> bool {
    let squashed: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let exe = "env!(\"CARGO_BIN_EXE_hale\")";
    if squashed.contains(&format!("Command::new({exe})")) {
        return true;
    }
    squashed.match_indices(&format!("={exe};")).any(|(at, _)| {
        let before = &squashed[..at];
        let Some(l) = before.rfind("let") else { return false };
        let name = before[l + 3..].trim_start_matches("mut");
        !name.is_empty() && (squashed.contains(&format!("Command::new({name})")) || squashed.contains(&format!("Command::new(&{name})")))
    })
}

fn test_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            if p.file_name().map_or(false, |n| n == "node_modules" || n == "target") {
                continue;
            }
            test_sources(&p, out);
        } else if p.extension().map_or(false, |x| x == "rs" || x == "hl" || x == "mjs") {
            out.push(p);
        }
    }
}

#[test]
fn no_test_spawns_a_vault_writing_hale_on_the_developers_vault() {
    let root = repo_root();
    let mut offenders = Vec::new();
    // Rust tests that run a vault-writing `hale dna` verb
    for e in std::fs::read_dir(root.join("crates/hale-cli/tests")).unwrap().flatten() {
        let p = e.path();
        if p.extension().map_or(true, |x| x != "rs") || p.file_name().map_or(false, |n| n == "harness_vault.rs") {
            continue;
        }
        let text = std::fs::read_to_string(&p).unwrap();
        // `dev` upgrades and `migrate` draws memory's role passwords, as
        // `new` / `init` / `upgrade` provision and `secret set` fills a slot
        let writes = text.contains("\"dna\"")
            && ["\"new\"", "\"init\"", "\"upgrade\"", "\"secret\"", "\"dev\"", "\"migrate\""].iter().any(|v| text.contains(v));
        // a spawn of the toolchain, however it is spelled, and a vault of
        // the file's own only where one is actually set on a child
        let spawns = spawns_hale_directly(&text);
        if writes && spawns && !text.contains(".env(\"HALE_VAULT_DIR\"") && !p.ends_with("tests/support/vault.rs") {
            offenders.push(format!("{}: spawns hale directly; use support/vault.rs's hale()", p.display()));
        }
    }
    // and no test source clears the override its run was given
    let mut sources = Vec::new();
    for dir in ["crates", "dna"] {
        test_sources(&root.join(dir), &mut sources);
    }
    for p in sources {
        let rel = p.strip_prefix(&root).unwrap().to_string_lossy().to_string();
        let is_test = rel.contains("/tests/") || rel.ends_with("_test.hl");
        if !is_test || rel.ends_with("harness_vault.rs") {
            continue;
        }
        let text = std::fs::read_to_string(&p).unwrap_or_default();
        // clearing the override outright, or the whole environment
        for bad in [
            "unset HALE_VAULT_DIR",
            "-u\\nHALE_VAULT_DIR",
            "env_remove(\"HALE_VAULT_DIR\")",
            "'-u', 'HALE_VAULT_DIR'",
            "env_clear()",
            "env -i",
            "env\\n-i",
            "'env', '-i'",
        ] {
            if text.contains(bad) {
                offenders.push(format!("{rel}: clears the vault override (`{bad}`)"));
            }
        }
        // a Node harness that hands a child an allow-listed environment
        // carries the override in the list (`isolatedEnvironment()` sets it)
        let lists = text.matches("'PATH', 'HOME'").count();
        if lists > text.matches("'HALE_VAULT_DIR'").count() {
            offenders.push(format!("{rel}: an allow-listed environment without 'HALE_VAULT_DIR'"));
        }
    }
    assert!(offenders.is_empty(), "a test would reach the developer's vault:\n{}", offenders.join("\n"));
}

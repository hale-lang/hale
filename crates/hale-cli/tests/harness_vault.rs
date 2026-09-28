//! No test run touches the developer's vault (GH #989's Deferred line,
//! closed). `hale dna init` and `upgrade` provision the organism's secrets
//! into the vault and `hale dna secret set` fills a slot, so a test run
//! against the real one left a throwaway organization's entries there, or
//! a fake key in a real slot.
//!
//! Two paths keep a test off it. `hale test` runs every `_test.hl` file
//! under a vault of its own (`HALE_VAULT_DIR`, made for the run and
//! removed after it; `HALE_VAULT_ADDR` removed), which everything the
//! fixture starts inherits; the first test here runs `hale test` over a
//! fixture that provisions a secret and proves the vault it wrote was not
//! the default one, under a home whose default vault stays empty. A Rust
//! test that runs a vault-writing `hale dna` verb spawns `hale` through
//! `support/vault.rs` (or names `HALE_VAULT_DIR` itself), and the second
//! test holds every such file to that, and every test source to never
//! clearing the override.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

// A fixture that writes a secret where `std::secret` provisions one, and
// says where that was (never the value).
const PROBE: &str = r#"fn main() {
    let dir = std::secret::vault_local_dir();
    std::test::assert(!std::env::var_exists("HALE_VAULT_ADDR"), "no real vault reaches a test");
    std::test::assert(std::env::var_exists("HALE_VAULT_DIR") && dir == std::env::var("HALE_VAULT_DIR"), "the vault is the run's own: " + dir);
    std::test::assert(dir != std::env::var("XDG_CACHE_HOME") + "/hale/vault", "never the developer's: " + dir);
    std::io::fs::write_file(dir + "/probe-entry", "x") or discard;
    std::test::assert(std::secret::Credential { vault: "probe-entry" }.ready(), "and the secret reads back from it");
}
"#;

#[test]
fn hale_test_runs_each_file_under_a_vault_of_its_own() {
    let d = std::env::temp_dir().join(format!("hale_vault_probe_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    let home = d.join("home");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(d.join("probe_test.hl"), PROBE).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("test")
        .arg(d.join("probe_test.hl"))
        .current_dir(&d)
        .env("HOME", &home)
        .env("XDG_CACHE_HOME", home.join(".cache"))
        // a caller's own vault and a real vault named: neither reaches the test
        .env("HALE_VAULT_DIR", home.join(".cache/hale/vault"))
        .env("HALE_VAULT_ADDR", "http://127.0.0.1:9")
        .output()
        .expect("hale test");
    let said = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success() && said.contains("1 passed"), "the probe passes under hale test: {said}");
    assert!(!home.join(".cache/hale/vault").exists(), "the home's vault was never written: {said}");
    let _ = std::fs::remove_dir_all(&d);
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
        let writes = text.contains("\"dna\"") && ["\"new\"", "\"init\"", "\"upgrade\"", "\"secret\""].iter().any(|v| text.contains(v));
        if writes && text.contains("Command::new(env!(\"CARGO_BIN_EXE_hale\"))") && !text.contains("HALE_VAULT_DIR") {
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
        for bad in ["unset HALE_VAULT_DIR", "-u\\nHALE_VAULT_DIR", "env_remove(\"HALE_VAULT_DIR\")", "'-u', 'HALE_VAULT_DIR'"] {
            if text.contains(bad) {
                offenders.push(format!("{rel}: clears the vault override (`{bad}`)"));
            }
        }
    }
    assert!(offenders.is_empty(), "a test would reach the developer's vault:\n{}", offenders.join("\n"));
}

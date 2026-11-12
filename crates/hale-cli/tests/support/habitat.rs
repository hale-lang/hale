//! The habitat a CLI test grafts its organisms into (GH #1653): every
//! organism but the habitat is grafted from one, so a test that makes an
//! organism makes it in a habitat of its test process's own — the one
//! `support/vault.rs`'s `hale()` already points at (`XDG_DATA_HOME` inside
//! the process's vault directory), born here the first time with no body:
//! on the operator's backends when an owner's DSN and NATS are named (CI's:
//! no compose), else its own. It comes down with the vault, when the vault
//! is swept.

use std::path::{Path, PathBuf};
use std::process::Command;

// the including test file's own vault module (`#[path = "support/vault.rs"]
// mod vault;` beside this one), so a graft and every later `hale` of the
// test share one vault and one habitat
use super::vault;

/// The habitat's directory, as `hale` resolves it under `vault::hale()`.
pub fn dir() -> PathBuf {
    vault::dir().join("data/hale/habitat/local")
}

/// The test process's habitat, born: its directory. Threads of one process
/// may each ask: `up` runs under a lock of the process's own, one at a time.
/// `hale habitat up` is idempotent once a habitat is born, but two births of
/// one directory at once are not: under libtest's threads (`cargo test`, the
/// parity job) they raced on the genome's `git init` and its `hale.toml`,
/// failing every test of a file that grafts (2026-11-11). nextest runs each
/// test in a process of its own and never met it.
pub fn habitat() -> PathBuf {
    static BIRTH: std::sync::Mutex<()> = std::sync::Mutex::new(());
    // a test that panicked holding it leaves it poisoned, not the habitat
    let _one = BIRTH.lock().unwrap_or_else(|e| e.into_inner());
    let operator = ["HALE_DNA_MEMORY_DSN_OWNER", "HALE_DNA_NATS_URL_OWNER"].iter().all(|k| std::env::var(k).is_ok_and(|v| !v.is_empty()));
    let mut up = vault::hale();
    up.args(["habitat", "up", "--no-body", "--backends", if operator { "operator" } else { "own" }]).env("HALE_DNA_DISCOVER", "off").env("HALE_BIN", env!("CARGO_BIN_EXE_hale"));
    if !operator {
        up.env_remove("HALE_DNA_MEMORY_DSN_OWNER").env_remove("HALE_DNA_NATS_URL_OWNER");
    }
    let out = up.output().expect("hale habitat up");
    assert!(out.status.success(), "the test's habitat was not born:\n{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    dir()
}

/// `hale habitat graft <dir> <args…>` run in `cwd`, into the test's habitat.
#[allow(dead_code)]
pub fn graft(cwd: &Path, dir: &str, args: &[&str]) -> std::process::Output {
    habitat();
    let mut c: Command = vault::hale();
    c.args(["habitat", "graft", dir]).args(args).current_dir(cwd).env("HALE_DNA_DISCOVER", "off").env("HALE_BIN", env!("CARGO_BIN_EXE_hale"));
    c.output().expect("hale habitat graft")
}

/// The habitat's secrets lent into `vault`, each entry it lacks: a test
/// that runs a graft under a vault of its own (`HALE_VAULT_DIR`) reaches
/// the habitat's record with them.
#[allow(dead_code)]
pub fn lend(to: &Path) {
    habitat();
    let _ = std::fs::create_dir_all(to);
    let Ok(entries) = std::fs::read_dir(vault::dir()) else { return };
    for e in entries.flatten() {
        let target = to.join(e.file_name());
        if e.path().is_file() && !target.exists() {
            let _ = std::fs::copy(e.path(), &target);
        }
    }
}

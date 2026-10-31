//! The record's rows as the toolchain reads them (GH #1590): `hale dna
//! record-rows`, one JSON object per line (`seq`, `kind`, `entity`,
//! `body`, `author`), oldest first. A test reads the record through it and
//! never through the record's storage, so the storage can change under it.

use std::path::Path;
use std::process::Command;

/// A reader's `hale dna` at `app`: memory read with the owner's DSN when the
/// suite names one (the head's DSN a clone keeps names its password by a
/// vault entry, which lives in the vault of whichever process migrated it).
fn reader() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_hale"));
    if let Some(owner) = owner_dsn() {
        c.env("HALE_DNA_MEMORY_DSN_HEAD", owner).env_remove("HALE_DNA_MEMORY_DSN_SPINE");
    }
    c
}

/// The record's rows at `app`, one JSON object per line; "" for a record
/// with no rows, or one that could not be read (a caller polling for a row
/// asks again).
#[allow(dead_code)]
pub fn text(app: &Path) -> String {
    reader()
        .args(["dna", "record-rows"])
        .arg(app)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default()
}

/// The record's Ledger rows at `app` (`hale dna ledger rows`), one JSON
/// object per line; "" when there are none or they could not be read.
#[allow(dead_code)]
pub fn ledger_text(app: &Path) -> String {
    reader()
        .args(["dna", "ledger", "rows"])
        .current_dir(app)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default()
}

/// As `text`, through `cmd` (the test's own `hale`, with its vault): the
/// head's DSN a clone keeps names its password by vault entry, which a
/// `hale` outside the test's vault cannot present.
#[allow(dead_code)]
pub fn text_in(mut cmd: Command, app: &Path) -> String {
    cmd.args(["dna", "record-rows"])
        .arg(app)
        .current_dir(app)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default()
}

/// The Ledger's rows at `app` (`hale dna ledger rows`), through `cmd`, in
/// the same line format: the day's work is the Ledger's (GH #1590). ""
/// when it could not be read.
#[allow(dead_code)]
pub fn ledger_in(mut cmd: Command, app: &Path) -> String {
    cmd.args(["dna", "ledger", "rows"])
        .current_dir(app)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default()
}

/// A receipt's body as memory keeps it (`hale dna record-body`), "" when
/// it keeps none unsealed.
#[allow(dead_code)]
pub fn body(app: &Path, digest: &str) -> String {
    reader()
        .args(["dna", "record-body", digest])
        .current_dir(app)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default()
}

/// As `body`, through `cmd` (the test's own `hale`, with its vault).
#[allow(dead_code)]
pub fn body_in(mut cmd: Command, app: &Path, digest: &str) -> String {
    cmd.args(["dna", "record-body", digest])
        .current_dir(app)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default()
}

/// The owner's DSN the suite runs memory under, when it is set.
#[allow(dead_code)]
pub fn owner_dsn() -> Option<String> {
    std::env::var("HALE_DNA_MEMORY_DSN_OWNER").ok().filter(|d| d.starts_with("postgres"))
}

/// The record's identity `app`'s genome pins (`[dna] record` in hale.toml),
/// "" when it pins none.
#[allow(dead_code)]
pub fn identity(app: &Path) -> String {
    let text = std::fs::read_to_string(app.join("hale.toml")).unwrap_or_default();
    let table: toml::Table = text.parse().unwrap_or_default();
    table.get("dna").and_then(|d| d.get("record")).and_then(|r| r.as_str()).unwrap_or("").to_string()
}

/// The record's schema in memory, `dna_<identity>` as the core names it.
#[allow(dead_code)]
pub fn schema(app: &Path) -> String {
    let id: String = identity(app).to_lowercase().chars().filter(|c| c.is_ascii_alphanumeric()).take(56).collect();
    format!("dna_{id}")
}

/// Drop `app`'s record from memory (its schema and roles), with the owner's
/// DSN; a test's last step, best effort.
#[allow(dead_code)]
pub fn unmigrate(app: &Path, owner: &str) {
    let sch = schema(app);
    if sch == "dna_" {
        return;
    }
    // the roles' names as the core cuts them to fit Postgres's identifiers
    let spine = format!("{}_spine", &sch[..sch.len().min(57)]);
    let head = format!("{}_head", &sch[..sch.len().min(58)]);
    let sql = format!("DROP SCHEMA IF EXISTS {sch} CASCADE; DROP ROLE IF EXISTS {spine}; DROP ROLE IF EXISTS {head}");
    let _ = Command::new("psql").args([owner, "-q", "-c", &sql]).output();
}

/// The spine's DSN for `app`'s record, from `hale dna memory migrate` run
/// through `cmd` (the test's own `hale`, with its vault): what a test hands
/// `hale dna run`, which takes it from its environment alone. "" when
/// memory could not be migrated.
#[allow(dead_code)]
pub fn spine(mut cmd: Command, app: &Path) -> String {
    let out = cmd.args(["dna", "memory", "migrate"]).current_dir(app).output();
    out.ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .and_then(|t| t.lines().find_map(|l| l.strip_prefix("HALE_DNA_MEMORY_DSN_SPINE=").map(str::to_string)))
        .unwrap_or_default()
}

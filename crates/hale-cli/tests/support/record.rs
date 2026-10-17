//! The record's rows as the toolchain reads them (GH #1590): `hale dna
//! record-rows`, one JSON object per line (`seq`, `kind`, `entity`,
//! `body`, `author`), oldest first. A test reads the record through it and
//! never through the record's storage, so the storage can change under it.

use std::path::Path;
use std::process::Command;

/// The record's rows at `app`, one JSON object per line; "" for a record
/// with no rows, or one that could not be read (a caller polling for a row
/// asks again).
pub fn text(app: &Path) -> String {
    Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["dna", "record-rows"])
        .arg(app)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default()
}

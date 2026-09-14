//! GH #604 rule 3 — `hale dna effect resolve <key> --outcome ok|failed`
//! from the CLI. The host's argument parser keeps a value only for the
//! flags it knows take one; `--outcome` was not among them, so its
//! value fell into the positionals and every resolve was refused as
//! "outcome ok|failed" whatever was given.

use std::path::Path;
use std::process::Command;

fn hale(args: &[&str], cwd: &Path) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(args)
        .current_dir(cwd)
        .env("HALE_BIN", env!("CARGO_BIN_EXE_hale"))
        .env("HALE_DNA_DISCOVER", "off")
        .env("XDG_CACHE_HOME", std::env::temp_dir().join("hale-tests-iris-cache"))
        .output()
        .expect("hale");
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

#[test]
fn the_outcome_flag_reaches_the_verb() {
    let d = std::env::temp_dir().join(format!("hale_dna_effres_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let (ok, out) = hale(&["dna", "new", "eff"], &d);
    assert!(ok, "{out}");
    let app = d.join("eff");
    // the outcome parsed: the verb gets as far as looking the key up
    let (ok, out) = hale(&["dna", "effect", "resolve", "apply:nothing", "--outcome", "ok"], &app);
    assert!(!ok && out.contains("no effect `apply:nothing` in the record"), "{out}");
    // and a bad outcome is still refused before the record is read
    let (ok, out) = hale(&["dna", "effect", "resolve", "apply:nothing", "--outcome", "maybe"], &app);
    assert!(!ok && out.contains("--outcome ok|failed"), "{out}");
    let _ = std::fs::remove_dir_all(&d);
}

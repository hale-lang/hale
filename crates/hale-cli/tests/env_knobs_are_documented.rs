//! Every `LOTUS_*` / `HALE_*` environment variable the shipped tree
//! names is in a table of the spec, and every row of those tables is a
//! variable the tree still names.
//!
//! The tables, one per layer, are heading-scoped so prose and constants
//! elsewhere in a spec do not count:
//!
//!   * `spec/runtime.md`, "Build-time and toolchain environment": the
//!     variables `hale-cli`'s `build_options_from_env` turns into
//!     `BuildOptions` fields, and the toolchain's own knobs;
//!   * `spec/runtime.md`, "Diagnostic + tuning env vars": what the C
//!     runtime, the standard library and the api binding read at run
//!     time;
//!   * `spec/dna.md`, "Environment": the DNA's, read by `dna/` and iris
//!     programs and by `hale dna`.
//!
//! What counts as "the tree names it", by grep, like
//! `dna_fixture_ports_are_free`: a string literal that is exactly the
//! name, in shipped source (`crates/*/src`, `build.rs`, the runtime C,
//! the stdlib's `.hl`, the DNA's and iris's `.hl`) or assigned inside
//! one (`"LOTUS_OBS=1\nHALE_BIN="`), or the name as a word in
//! `scripts/*.sh`. Not
//! counted: tests, fixtures and examples, comment lines, and the
//! compile-time variables a build script hands to `env!` (they are
//! not knobs). A knob nobody documents fails here; the fix is a row,
//! or deleting the knob.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// (spec file, the `## ` heading whose table rows name variables)
const TABLES: &[(&str, &str)] = &[
    ("spec/runtime.md", "Build-time and toolchain environment"),
    ("spec/runtime.md", "Diagnostic + tuning env vars"),
    ("spec/dna.md", "Environment"),
];

/// Source roots that belong to the layers `TABLES` covers.
const SOURCE_ROOTS: &[&str] = &["crates", "scripts", "dna", "iris"];

const SKIP_DIRS: &[&str] = &["target", "node_modules", ".git", "fixtures", "vendor", "tests", "acceptance", "examples"];

/// Lines whose names are compile-time build-script plumbing.
const COMPILE_TIME: &[&str] = &["env!(", "option_env!(", "cargo:rustc-env", "cargo:rerun-if-env-changed"];

fn is_name_char(c: char) -> bool {
    c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'
}

/// Does a name start at byte `at`? Not when it is the tail of a longer
/// identifier, but yes after a literal backslash-n, which ends a line of
/// the env file a string spells (`"...\nHALE_X="`): its `n` is not part
/// of the name.
fn starts_a_name(text: &str, at: usize) -> bool {
    text[..at].ends_with("\\n") || !text[..at].chars().next_back().is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The knob names in one cell or line: `LOTUS_`/`HALE_` followed by
/// name characters, not part of a longer identifier.
fn names_in(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for prefix in ["LOTUS_", "HALE_"] {
        for (at, _) in text.match_indices(prefix) {
            if !starts_a_name(text, at) {
                continue;
            }
            let name: String = text[at..].chars().take_while(|c| is_name_char(*c)).collect();
            if name.len() > prefix.len() && name.chars().nth(prefix.len()).is_some_and(|c| c.is_ascii_uppercase()) {
                out.push(name);
            }
        }
    }
    out
}

/// A name that is a whole string literal: `"HALE_X"`.
fn quoted_names_in(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (at, _) in line.match_indices('"') {
        let rest = &line[at + 1..];
        let name: String = rest.chars().take_while(|c| is_name_char(*c)).collect();
        // A name ending in `_` is a prefix a name is built from
        // (`"HALE_DNA_NATS_" + role`), not a variable.
        if (name.starts_with("LOTUS_") || name.starts_with("HALE_"))
            && !name.ends_with('_')
            && rest[name.len()..].starts_with('"')
            && !names_in(&name).is_empty()
        {
            out.push(name);
        }
    }
    out
}

/// A name assigned inside a string: the `NAME=value` of an env file, an
/// `env NAME=value` argv, a `FOO=1 cmd` shell line
/// (`"LOTUS_OBS=1\nHALE_BIN=" + hale`).
fn assigned_names_in(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    for n in names_in(line) {
        let mut from = 0;
        while let Some(at) = line[from..].find(&n) {
            let start = from + at;
            let end = start + n.len();
            if starts_a_name(line, start) && line[end..].starts_with('=') && !line[end..].starts_with("==") {
                out.push(n.clone());
                break;
            }
            from = end;
        }
    }
    out
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = entries.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if p.is_dir() {
            if !SKIP_DIRS.contains(&name) {
                walk(&p, out);
            }
        } else {
            out.push(p);
        }
    }
}

/// name -> the files that name it.
fn scanned() -> BTreeMap<String, BTreeSet<String>> {
    let root = repo();
    let mut found: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for r in SOURCE_ROOTS {
        let mut files = Vec::new();
        walk(&root.join(r), &mut files);
        for f in files {
            let ext = f.extension().and_then(|e| e.to_str()).unwrap_or("");
            let script = ext == "sh";
            if !matches!(ext, "rs" | "c" | "h" | "hl" | "sh") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&f) else { continue };
            let rel = f.strip_prefix(&root).unwrap().display().to_string();
            for line in text.lines() {
                let code = line.trim_start();
                if code.starts_with("//") || (script && code.starts_with('#')) {
                    continue;
                }
                if COMPILE_TIME.iter().any(|c| line.contains(c)) {
                    continue;
                }
                let mut names = if script { names_in(line) } else { quoted_names_in(line) };
                if !script && line.contains('"') {
                    names.extend(assigned_names_in(line));
                }
                for n in names {
                    found.entry(n).or_default().insert(rel.clone());
                }
            }
        }
    }
    found
}

/// The names in the first cell of every table row under `heading`.
fn documented() -> BTreeMap<String, String> {
    let root = repo();
    let mut out = BTreeMap::new();
    for (file, heading) in TABLES {
        let text = std::fs::read_to_string(root.join(file)).unwrap_or_else(|e| panic!("{file}: {e}"));
        let mut current = "";
        let mut seen_heading = false;
        for line in text.lines() {
            if let Some(h) = line.strip_prefix("## ") {
                current = h.trim();
                seen_heading |= current == *heading;
                continue;
            }
            if current == *heading && line.starts_with('|') {
                let first = line.split('|').nth(1).unwrap_or("");
                for n in names_in(first) {
                    out.insert(n, format!("{file}: {heading}"));
                }
            }
        }
        assert!(seen_heading, "{file} has no `## {heading}` section");
    }
    out
}

#[test]
fn every_variable_the_tree_names_is_in_a_table() {
    let tables = documented();
    let missing: Vec<String> = scanned()
        .into_iter()
        .filter(|(n, _)| !tables.contains_key(n))
        .map(|(n, files)| format!("{n}  (named in {})", files.iter().take(3).cloned().collect::<Vec<_>>().join(", ")))
        .collect();
    assert!(
        missing.is_empty(),
        "{} environment variable(s) are named in shipped source but are in no table of {:?}:\n  {}\n\n\
         Add a row (name, effect, default) to the table of the layer that reads it, or delete the knob \
         if nothing documents it and nothing tests it.",
        missing.len(),
        TABLES,
        missing.join("\n  ")
    );
}

#[test]
fn every_table_row_is_a_variable_the_tree_names() {
    let names = scanned();
    let stale: Vec<String> = documented()
        .into_iter()
        .filter(|(n, _)| !names.contains_key(n))
        .map(|(n, at)| format!("{n}  ({at})"))
        .collect();
    assert!(
        stale.is_empty(),
        "these table rows name a variable no shipped source names any more:\n  {}\n\nDelete the row.",
        stale.join("\n  ")
    );
}

#[test]
fn the_scan_is_not_vacuous() {
    assert!(scanned().len() >= 60, "scanned only {} names", scanned().len());
    assert!(documented().len() >= 60, "read only {} table names", documented().len());
    assert!(names_in("`LOTUS_LTO`, `HALE_TIME`").len() == 2);
    assert!(names_in("SOME_HALE_X and LOTUS_ alone").is_empty(), "a longer identifier or a bare prefix is not a knob");
    assert_eq!(quoted_names_in("f(\"HALE_X\", \"HALE_Y z\", \"NOTHALE_Z\", \"HALE_P_\")"), vec!["HALE_X".to_string()]);
    assert_eq!(assigned_names_in("e = \"LOTUS_OBS=1\\nHALE_BIN=\" + h"), vec!["LOTUS_OBS".to_string(), "HALE_BIN".to_string()]);
    assert!(assigned_names_in("if HALE_X == 1 { \"A_HALE_Y=2\" }").is_empty(), "a comparison and a longer identifier are not assignments");
}

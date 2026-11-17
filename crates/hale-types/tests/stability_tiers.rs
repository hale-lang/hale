//! Every stdlib namespace and every spec file carries a stability tier
//! (STABILITY.md § Tiers, GH #1950).
//!
//! `spec/stability.toml` is the table. This test holds it to what it
//! describes, in both directions, so the table cannot fall behind:
//!
//! - every namespace of the stdlib table (`stdlib_surface::SURFACES`,
//!   the namespaces `stdlib_table_answers.txt` lists) has a row, and
//!   every row names a namespace the table has; an `experimental_fns`
//!   entry names a public function of its namespace;
//! - every file under `spec/` has a row, and an `experimental_sections`
//!   entry is a `## ` heading of its file;
//! - the graph registry's Reserved families are exactly the table's
//!   `experimental_families`;
//! - a tier is `stable` or `experimental`, and an experimental row
//!   says why;
//! - the book's generated table (`docs/src/stability.md`) is the
//!   rendering of the file. A changed tier regenerates it:
//!
//! ```sh
//! HALE_REGEN_STABILITY=1 cargo test -p hale-types --test stability_tiers
//! ```

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use hale_graph::registry::{self, State};
use hale_types::stdlib_surface::SURFACES;

const TIERS: &[&str] = &["stable", "experimental"];
const BEGIN: &str = "<!-- BEGIN GENERATED from spec/stability.toml by crates/hale-types/tests/stability_tiers.rs -->";
const END: &str = "<!-- END GENERATED -->";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn table() -> toml::Table {
    let path = repo_root().join("spec/stability.toml");
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    text.parse::<toml::Table>().unwrap_or_else(|e| panic!("parse spec/stability.toml: {e}"))
}

fn section<'a>(t: &'a toml::Table, key: &str) -> &'a toml::Table {
    t.get(key)
        .and_then(|v| v.as_table())
        .unwrap_or_else(|| panic!("spec/stability.toml has no [{key}] table"))
}

fn strings(row: &toml::Table, key: &str, what: &str) -> Vec<String> {
    match row.get(key) {
        None => Vec::new(),
        Some(v) => v
            .as_array()
            .unwrap_or_else(|| panic!("{what}: `{key}` is not an array"))
            .iter()
            .map(|s| s.as_str().unwrap_or_else(|| panic!("{what}: `{key}` holds a non-string")).to_string())
            .collect(),
    }
}

/// One row's tier, checked: a known tier, a reason when experimental, and
/// no unknown key.
fn tier_of(row: &toml::Value, what: &str, keys: &[&str]) -> String {
    let row = row.as_table().unwrap_or_else(|| panic!("{what}: a row is a table (`{{ tier = \"...\" }}`)"));
    for k in row.keys() {
        assert!(
            k == "tier" || k == "why" || keys.contains(&k.as_str()),
            "{what}: unknown key `{k}` (allowed: tier, why, {})",
            keys.join(", ")
        );
    }
    let tier = row
        .get("tier")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| panic!("{what}: no `tier`"));
    assert!(TIERS.contains(&tier), "{what}: tier `{tier}` is not one of {TIERS:?}");
    if tier == "experimental" {
        let why = row.get("why").and_then(|v| v.as_str()).unwrap_or("");
        assert!(!why.trim().is_empty(), "{what}: an experimental row says why (`why = \"...\"`)");
    }
    tier.to_string()
}

fn ns_name(ns: &[&str]) -> String {
    format!("std::{}", ns.join("::"))
}

#[test]
fn every_stdlib_namespace_has_a_tier() {
    let t = table();
    let rows = section(&t, "stdlib");
    let table_ns: BTreeSet<String> = SURFACES.iter().map(|s| ns_name(s.ns)).collect();
    let missing: Vec<&String> = table_ns.iter().filter(|n| !rows.contains_key(n.as_str())).collect();
    assert!(
        missing.is_empty(),
        "stdlib namespaces with no stability tier: {missing:?}.\n\
         A new namespace declares its tier in spec/stability.toml under [stdlib] \
         (`\"std::x\" = {{ tier = \"experimental\", why = \"...\" }}`); see STABILITY.md § Tiers."
    );
    for (name, row) in rows {
        let surf = SURFACES
            .iter()
            .find(|s| &ns_name(s.ns) == name)
            .unwrap_or_else(|| panic!("spec/stability.toml [stdlib] names `{name}`, which the stdlib table does not have"));
        let what = format!("[stdlib] {name}");
        tier_of(row, &what, &["experimental_fns"]);
        let public: BTreeSet<&str> = surf.public().map(|f| f.name).collect();
        for f in strings(row.as_table().unwrap(), "experimental_fns", &what) {
            assert!(public.contains(f.as_str()), "{what}: experimental_fns names `{f}`, not a public fn of the namespace");
        }
    }
}

/// `## ` headings of a markdown file (outside fenced blocks).
fn headings(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut fenced = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if !fenced {
            if let Some(h) = line.strip_prefix("## ") {
                out.insert(h.trim().to_string());
            }
        }
    }
    out
}

#[test]
fn every_spec_file_has_a_tier() {
    let t = table();
    let rows = section(&t, "spec");
    let dir = repo_root().join("spec");
    let mut files = BTreeSet::new();
    for e in fs::read_dir(&dir).unwrap() {
        let e = e.unwrap();
        let name = e.file_name().to_string_lossy().to_string();
        if e.file_type().unwrap().is_file() && name != "stability.toml" {
            files.insert(name);
        }
    }
    let missing: Vec<&String> = files.iter().filter(|f| !rows.contains_key(f.as_str())).collect();
    assert!(
        missing.is_empty(),
        "spec files with no stability tier: {missing:?}.\n\
         A new spec file declares its tier in spec/stability.toml (`[spec.\"x.md\"]` with \
         `tier = \"stable\"` or `\"experimental\"`); see STABILITY.md § Tiers."
    );
    for (name, row) in rows {
        assert!(files.contains(name), "spec/stability.toml [spec] names `{name}`, which is not a file under spec/");
        let what = format!("[spec.\"{name}\"]");
        let tier = tier_of(row, &what, &["experimental_sections", "normative"]);
        let row = row.as_table().unwrap();
        if let Some(n) = row.get("normative") {
            assert!(n.as_bool().is_some(), "{what}: `normative` is a boolean");
        }
        let sections = strings(row, "experimental_sections", &what);
        if sections.is_empty() {
            continue;
        }
        assert_eq!(tier, "stable", "{what}: an experimental file needs no experimental_sections");
        let text = fs::read_to_string(dir.join(name)).unwrap();
        let have = headings(&text);
        for s in sections {
            assert!(have.contains(&s), "{what}: experimental_sections names `{s}`, which is not a `## ` heading of spec/{name}");
        }
    }
}

#[test]
fn reserved_registry_families_are_experimental() {
    let t = table();
    let listed: BTreeSet<String> = strings(section(&t, "registry"), "experimental_families", "[registry]").into_iter().collect();
    let reserved: BTreeSet<String> = registry::families()
        .iter()
        .filter(|f| f.state == State::Reserved)
        .map(|f| f.name.to_string())
        .collect();
    assert_eq!(
        listed, reserved,
        "spec/stability.toml [registry] experimental_families must be exactly the graph registry's \
         Reserved families: a Reserved family has no producer and no syntax, so it carries no promise"
    );
}

/// The book's table: one row per stdlib namespace and per spec file.
fn render(t: &toml::Table) -> String {
    let mut out = String::new();
    out.push_str("### Standard library\n\n| namespace | tier | notes |\n|---|---|---|\n");
    for (name, row) in section(t, "stdlib") {
        let row = row.as_table().unwrap();
        let tier = row["tier"].as_str().unwrap();
        let mut notes = String::new();
        let fns = strings(row, "experimental_fns", name);
        if !fns.is_empty() {
            let fns: Vec<String> = fns.iter().map(|f| format!("`{f}`")).collect();
            notes.push_str(&format!("experimental: {}. ", fns.join(", ")));
        }
        if let Some(why) = row.get("why").and_then(|v| v.as_str()) {
            notes.push_str(why);
        }
        out.push_str(&format!("| `{name}` | {tier} | {} |\n", notes.trim()));
    }
    out.push_str("\n### Specification\n\n| file | tier | notes |\n|---|---|---|\n");
    for (name, row) in section(t, "spec") {
        let row = row.as_table().unwrap();
        let tier = row["tier"].as_str().unwrap();
        let mut notes = String::new();
        if row.get("normative").and_then(|v| v.as_bool()) == Some(false) {
            notes.push_str("non-normative record. ");
        }
        let secs = strings(row, "experimental_sections", name);
        if !secs.is_empty() {
            let secs: Vec<String> = secs.iter().map(|s| format!("§ {}", s.replace('|', "\\|"))).collect();
            notes.push_str(&format!("experimental: {}. ", secs.join("; ")));
        }
        if let Some(why) = row.get("why").and_then(|v| v.as_str()) {
            notes.push_str(why);
        }
        let link = format!("[`spec/{name}`](https://github.com/hale-lang/hale/blob/main/spec/{name})");
        out.push_str(&format!("| {link} | {tier} | {} |\n", notes.trim()));
    }
    let fams = strings(section(t, "registry"), "experimental_families", "[registry]");
    let fams: Vec<String> = fams.iter().map(|f| format!("`{f}`")).collect();
    out.push_str(&format!(
        "\nThe graph registry's Reserved families ({}) are experimental until they have a producer.\n",
        fams.join(", ")
    ));
    out
}

#[test]
fn the_book_shows_the_tiers() {
    let t = table();
    let path = repo_root().join("docs/src/stability.md");
    let page = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let (head, rest) = page.split_once(BEGIN).unwrap_or_else(|| panic!("docs/src/stability.md lacks the line `{BEGIN}`"));
    let (_, tail) = rest.split_once(END).unwrap_or_else(|| panic!("docs/src/stability.md lacks the line `{END}`"));
    let want = format!("{head}{BEGIN}\n{}{END}{tail}", render(&t));
    if std::env::var_os("HALE_REGEN_STABILITY").is_some() {
        fs::write(&path, &want).unwrap();
        return;
    }
    assert!(
        page == want,
        "docs/src/stability.md's table is not spec/stability.toml's rendering; regenerate it:\n  \
         HALE_REGEN_STABILITY=1 cargo test -p hale-types --test stability_tiers"
    );
}

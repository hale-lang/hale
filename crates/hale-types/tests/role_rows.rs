//! The role rows (F.40 phase 4, A4; R4): the roles an environment maps
//! are a projection of the rows, over every seed's snapshot, and the rows
//! are what the program declares (an independent walk), built once.
//!
//! The seeds are the corpus (each program a bare snapshot), every
//! `*_test.hl` under `tests/hale` and `dna/`, and every directory under
//! `dna/` holding a `main locus` that is not a test file's, as the
//! placement golden loads them.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use hale_frontend::frontend::LoadMode;
use hale_frontend::snapshot::{Config, Snapshot};
use hale_frontend::source::Disk;

fn hl_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            hl_files(&p, out);
        } else if p.extension().is_some_and(|e| e == "hl") {
            out.push(p);
        }
    }
}

/// Every seed, by its origin, loaded as `hale check` loads it.
fn snapshots() -> impl Iterator<Item = (String, Snapshot)> {
    let corpus = hale_corpus::all().into_iter().filter_map(|p| {
        let program = hale_syntax::parse_source(&p.source).ok()?;
        let s = Snapshot::from_program(program, Vec::new(), Config::check(false, false)).ok()?;
        Some((p.origin, s))
    });
    let root = hale_corpus::repo_root();
    let mut files = Vec::new();
    hl_files(&root.join("tests/hale"), &mut files);
    hl_files(&root.join("dna"), &mut files);
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut dirs: BTreeSet<PathBuf> = BTreeSet::new();
    for f in files {
        let name = f.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.ends_with("_test.hl") {
            paths.push(f);
        } else if f.starts_with(root.join("dna"))
            && std::fs::read_to_string(&f).is_ok_and(|s| s.lines().any(|l| l.trim_start().starts_with("main locus")))
        {
            dirs.insert(f.parent().unwrap().to_path_buf());
        }
    }
    paths.extend(dirs);
    let on_disk = paths.into_iter().filter_map(move |path| {
        let s = Snapshot::load(&path, LoadMode::WholeSeed, &Disk, Config::check(path.is_dir(), false)).ok()?;
        Some((path.strip_prefix(&root).unwrap_or(&path).display().to_string(), s))
    });
    corpus.chain(on_disk)
}

#[test]
fn the_roles_an_environment_maps_are_a_projection_of_the_rows() {
    fn declared(items: &[hale_syntax::ast::TopDecl], out: &mut BTreeSet<String>) {
        for item in items {
            match item {
                hale_syntax::ast::TopDecl::Role(r) => {
                    out.insert(r.name.name.clone());
                }
                hale_syntax::ast::TopDecl::Module(m) => declared(&m.items, out),
                _ => {}
            }
        }
    }
    let (mut seeds, mut with_roles) = (0usize, 0usize);
    let mut differ: Vec<String> = Vec::new();
    for (origin, s) in snapshots() {
        let Ok(rows) = s.demand_role_rows() else { continue };
        seeds += 1;
        let bundle = s.bundle();
        let mut names = BTreeSet::new();
        for p in bundle.programs.values() {
            declared(&p.items, &mut names);
        }
        let names: Vec<String> = names.into_iter().collect();
        if rows.declared_roles() != names {
            differ.push(format!("{origin}: declared {:?} != {:?}", rows.declared_roles(), names));
        }
        if rows.vocabulary().len() != names.len() {
            differ.push(format!("{origin}: the vocabulary names {} roles, the program {}", rows.vocabulary().len(), names.len()));
        }
        with_roles += usize::from(!rows.roles.is_empty());
        assert_eq!(s.builds()["role_rows"], 1, "{origin}: the rows are built once");
    }
    eprintln!("{seeds} seeds, {with_roles} declaring a role");
    assert!(differ.is_empty(), "{}", differ.join("\n"));
    assert!(seeds > 1000 && with_roles > 0, "the seeds reached too little");
}

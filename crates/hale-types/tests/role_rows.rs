//! The role rows (F.40 phase 4, A4): the roles an environment maps and
//! the vocabulary the api binding is generated with are projections of
//! the rows, over every seed's snapshot. The binding is generated inside
//! the desugar sequence, before the rows exist, so `hale-syntax` keeps
//! the one function the surface computes its list with
//! (`api_gen::role_vocabulary`), and this holds the rows' projection
//! equal to it.
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
    let (mut seeds, mut with_roles, mut served, mut surfaces) = (0usize, 0usize, 0usize, 0usize);
    let mut differ: Vec<String> = Vec::new();
    for (origin, s) in snapshots() {
        let Ok(rows) = s.demand_role_rows() else { continue };
        seeds += 1;
        let bundle = s.bundle();
        let refs: Vec<&hale_syntax::ast::Program> = bundle.programs.values().copied().collect();
        let root = s.demand_entry().ok().and_then(|e| e.root()).and_then(|m| m.decl(&bundle));
        let names = hale_syntax::api_gen::declared_roles(&refs, root);
        if rows.declared_roles() != names {
            differ.push(format!("{origin}: declared {:?} != {:?}", rows.declared_roles(), names));
        }
        let vocabulary = |roles: &[hale_syntax::api_gen::ApiRole]| -> Vec<(String, Vec<String>)> {
            roles.iter().map(|r| (r.name.clone(), r.includes.clone())).collect()
        };
        let list = vocabulary(&hale_syntax::api_gen::role_vocabulary(&refs, rows.served));
        if rows.vocabulary() != list {
            differ.push(format!("{origin}: vocabulary {:?} != {:?}", rows.vocabulary(), list));
        }
        if let Some(surface) = s.api_surface() {
            surfaces += 1;
            if rows.vocabulary() != vocabulary(&surface.roles) {
                differ.push(format!("{origin}: the surface's roles differ from the rows'"));
            }
        }
        with_roles += usize::from(!rows.roles.is_empty());
        served += usize::from(rows.served);
        assert_eq!(s.builds()["api_surface"], 1, "{origin}: the rows are built once");
    }
    eprintln!("{seeds} seeds, {with_roles} declaring a role, {served} served, {surfaces} surfaces");
    assert!(differ.is_empty(), "{}", differ.join("\n"));
    assert!(seeds > 1000 && with_roles > 0 && surfaces > 0, "the seeds reached too little");
}

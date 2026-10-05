//! The differential for F.40 phase 4, A4, deleted with the old
//! functions: over every seed's snapshot, the role rules judged over the
//! AST (`check::check_api_roles`) equal the law over the role rows
//! (`roles::role_laws`), as whole diagnostics in order; and the api
//! entry's rules over the surface the check re-derives from the programs
//! after the sequence equal them over the snapshot's surface, the one
//! the sequence generated the binding from.
//!
//! The seeds are the corpus (every program of `hale_corpus::all()` that
//! parses, each a bare snapshot), every `*_test.hl` under `tests/hale`
//! and `dna/`, and every directory under `dna/` holding a `main locus`
//! that is not a test file's. `api_binding_check.rs` runs the same
//! comparison over each program its tests check.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use hale_frontend::frontend::LoadMode;
use hale_frontend::snapshot::{Config, Snapshot};
use hale_frontend::source::Disk;
use hale_syntax::Diag;

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

/// Every seed on disk, by the set it belongs to.
fn paths() -> Vec<(&'static str, PathBuf)> {
    let root = hale_corpus::repo_root();
    let mut out = Vec::new();
    for (set, dir) in [("tests/hale", "tests/hale"), ("dna", "dna")] {
        let mut files = Vec::new();
        hl_files(&root.join(dir), &mut files);
        let mut dirs: BTreeSet<PathBuf> = BTreeSet::new();
        for f in files {
            let name = f.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.ends_with("_test.hl") {
                out.push((set, f));
            } else if set == "dna"
                && std::fs::read_to_string(&f).is_ok_and(|s| s.lines().any(|l| l.trim_start().starts_with("main locus")))
            {
                dirs.insert(f.parent().unwrap().to_path_buf());
            }
        }
        out.extend(dirs.into_iter().map(|d| (set, d)));
    }
    out
}

/// The fields of a surface the api entry's rules read. (The binding's
/// expressions are not among them: the snapshot's surface is taken
/// before the mint, so their identities differ, and no rule reads one.)
pub fn read(surface: Option<&hale_syntax::api_gen::ApiSurface>) -> String {
    surface.map_or_else(String::new, |s| {
        let b = &s.binding;
        let http = b.http.as_ref().map(|h| (h.principals.is_some(), h.span));
        format!(
            "{:?}",
            (
                (b.bound, b.on_full, b.watch_bound, b.on_watch_full, http, b.span),
                &s.serve_errors,
                &s.ambiguous_replies,
                s.excluded.iter().map(|e| (&e.what, &e.reason)).collect::<Vec<_>>()
            )
        )
    })
}

#[derive(Default)]
struct Totals {
    surfaces: usize,
    seeds: usize,
    unloaded: usize,
    blocked: usize,
    with_role_diag: usize,
    role_diags: usize,
    with_api_diag: usize,
    api_diags: usize,
    differences: Vec<String>,
}

impl Totals {
    fn compare(&mut self, origin: &str, s: &Snapshot) {
        let (Ok(top), Ok(bindings), Ok(bus), Ok(rows), Ok(entry)) =
            (s.demand_scope(), s.demand_bindings(), s.demand_bus_graph(), s.demand_role_rows(), s.demand_entry())
        else {
            self.blocked += 1;
            return;
        };
        self.seeds += 1;
        let bundle = s.bundle();
        let programs: Vec<&hale_syntax::ast::Program> = bundle.programs.values().copied().collect();
        let mut old: Vec<Diag> = Vec::new();
        hale_types::check::check_api_roles(&programs, &top.topics, bindings, &mut old);
        let new = hale_types::roles::role_laws(rows, bus, &top.topics, bindings);
        self.differ(origin, "role", &old, &new);
        self.with_role_diag += usize::from(!old.is_empty());
        self.role_diags += old.len().max(new.len());

        let root = entry.root().and_then(|m| m.decl(&bundle));
        let rederived = hale_syntax::api_gen::api_surface(&programs, root);
        let old = rederived.as_ref().map(hale_types::check::api_binding_rules).unwrap_or_default();
        let new = s.api_surface().map(hale_types::check::api_binding_rules).unwrap_or_default();
        self.differ(origin, "api binding", &old, &new);
        self.with_api_diag += usize::from(!old.is_empty());
        self.api_diags += old.len().max(new.len());
        // What the api entry's rules read of the surface, whole: equal
        // fields judge alike under any rule over them.
        if read(rederived.as_ref()) != read(s.api_surface()) {
            self.differences.push(format!(
                "{origin} (the surface the api rules read):\n  old: {}\n  new: {}",
                read(rederived.as_ref()),
                read(s.api_surface())
            ));
        }
        self.surfaces += usize::from(rederived.is_some());
    }

    fn differ(&mut self, origin: &str, what: &str, old: &[Diag], new: &[Diag]) {
        if old == new {
            return;
        }
        for i in 0..old.len().max(new.len()) {
            if old.get(i) != new.get(i) {
                self.differences.push(format!(
                    "{origin} ({what} #{i}):\n  old: {:?}\n  new: {:?}",
                    old.get(i),
                    new.get(i)
                ));
            }
        }
    }

    fn report(&self, set: &str) -> String {
        format!(
            "{set}: {} seeds ({} not loaded, {} blocked); role: {} seeds with a diagnostic, {} compared; \
             api binding: {} surfaces, {} seeds with a diagnostic, {} compared; {} differences",
            self.seeds,
            self.unloaded,
            self.blocked,
            self.with_role_diag,
            self.role_diags,
            self.surfaces,
            self.with_api_diag,
            self.api_diags,
            self.differences.len()
        )
    }
}

#[test]
fn the_role_law_and_the_snapshots_surface_judge_as_the_old_functions() {
    let mut reports = Vec::new();
    let mut differences = Vec::new();

    let mut corpus = Totals::default();
    for p in hale_corpus::all() {
        let Ok(program) = hale_syntax::parse_source(&p.source) else { continue };
        match Snapshot::from_program(program, Vec::new(), Config::check(false, false)) {
            Ok(s) => corpus.compare(&p.origin, &s),
            Err(_) => corpus.unloaded += 1,
        }
    }
    reports.push(corpus.report("corpus"));
    differences.extend(corpus.differences);

    let root = hale_corpus::repo_root();
    for set in ["tests/hale", "dna"] {
        let mut t = Totals::default();
        for (_, path) in paths().into_iter().filter(|(s, _)| *s == set) {
            let origin = path.strip_prefix(&root).unwrap_or(&path).display().to_string();
            match Snapshot::load(&path, LoadMode::WholeSeed, &Disk, Config::check(path.is_dir(), false)) {
                Ok(s) => t.compare(&origin, &s),
                Err(_) => {
                    eprintln!("not loaded: {origin}");
                    t.unloaded += 1
                }
            }
        }
        reports.push(t.report(set));
        differences.extend(t.differences);
    }
    for r in &reports {
        eprintln!("{r}");
    }
    assert!(differences.is_empty(), "{}", differences.join("\n"));
    assert!(corpus.seeds > 1000 && corpus.with_role_diag > 0, "the corpus reached too little");
}

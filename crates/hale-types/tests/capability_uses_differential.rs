//! The requirements fixpoint's worklist against the pass-by-pass loop it
//! replaced (F.40 phase 4, Q2): over every example fixture, every
//! `tests/hale` program, every DNA `main.hl` (each loaded from disk as the
//! editor loads it) and every program embedded in the Rust tests (as a
//! one-file seed), the use rows — every use, need, chain and hole — and
//! every node's requirement row are the same both ways. The reference
//! stays in the tree only until this has run; then both go. About five
//! seconds on eight threads.
//!
//! ```text
//! cargo test --release -p hale-types --test capability_uses_differential -- --nocapture
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use hale_frontend::frontend::LoadMode;
use hale_frontend::snapshot::{Config, Snapshot};
use hale_frontend::source::{Disk, Overlay};
use hale_types::capability::uses::{
    capability_uses_by_passes, derive_capability_uses, requirement_rows_both_ways, Need,
};

enum Origin {
    Disk(PathBuf),
    Text(String, String),
}

fn walk(dir: &Path, keep: &dyn Fn(&Path) -> bool, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for p in paths {
        if p.is_dir() {
            walk(&p, keep, out);
        } else if p.extension().is_some_and(|e| e == "hl") && keep(&p) {
            out.push(p);
        }
    }
}

#[derive(Default)]
struct Totals {
    programs: usize,
    skipped: usize,
    nodes: usize,
    uses: usize,
    holes: usize,
    links: usize,
}

#[test]
fn the_worklist_answers_as_the_passes_did() {
    let root = hale_corpus::repo_root();
    let mut files = Vec::new();
    walk(&root.join("crates/hale-codegen/tests/fixtures/examples"), &|_| true, &mut files);
    walk(&root.join("tests/hale"), &|_| true, &mut files);
    walk(&root.join("dna"), &|p| p.file_name().is_some_and(|n| n == "main.hl"), &mut files);
    let mut origins: Vec<Origin> = files.into_iter().map(Origin::Disk).collect();
    origins.extend(hale_corpus::embedded().into_iter().map(|p| Origin::Text(p.origin, p.source)));

    let next = AtomicUsize::new(0);
    let totals = Mutex::new(Totals::default());
    let diverged = Mutex::new(Vec::new());
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get()).min(8);
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(origin) = origins.get(i) else { break };
                let (name, loaded) = match origin {
                    Origin::Disk(file) => (
                        file.strip_prefix(&root).unwrap_or(file).display().to_string(),
                        Snapshot::load(file, LoadMode::Editor, &Disk, Config::editor()),
                    ),
                    Origin::Text(name, text) => {
                        let file = PathBuf::from("/hale-test-seed/main.hl");
                        let buffers = BTreeMap::from([(file.clone(), text.clone())]);
                        (name.clone(), Snapshot::load(&file, LoadMode::WholeSeed, &Overlay::new(&buffers), Config::check(true, false)))
                    }
                };
                let Ok(Ok(snap)) = loaded.map(Snapshot::linked) else {
                    totals.lock().unwrap().skipped += 1;
                    continue;
                };
                let Ok(summary) = snap.demand_alloc_summary() else {
                    totals.lock().unwrap().skipped += 1;
                    continue;
                };
                let bundle = snap.bundle();
                let worklist = derive_capability_uses(&bundle, summary);
                let passes = capability_uses_by_passes(&bundle, summary);
                let (rows, reference) = requirement_rows_both_ways(&bundle, summary);
                if worklist != passes {
                    diverged.lock().unwrap().push(format!("{name}: the use rows differ\n  worklist {:?}\n  passes   {:?}", worklist.uses, passes.uses));
                }
                if rows != reference {
                    let at = rows.iter().zip(&reference).position(|(a, b)| a != b).unwrap_or(rows.len().min(reference.len()));
                    diverged.lock().unwrap().push(format!(
                        "{name}: requirement row {at} differs\n  worklist {:?}\n  passes   {:?}",
                        rows.get(at),
                        reference.get(at)
                    ));
                }
                let mut t = totals.lock().unwrap();
                t.programs += 1;
                t.nodes += rows.len();
                t.uses += worklist.uses.len();
                t.holes += worklist.uses.iter().filter(|u| matches!(u.need, Need::Hole(_))).count();
                t.links += worklist.uses.iter().map(|u| u.chain.len()).sum::<usize>();
            });
        }
    });
    let t = totals.into_inner().unwrap();
    let diverged = diverged.into_inner().unwrap();
    println!(
        "{} origins: {} programs compared ({} requirement rows, {} uses, {} of them holes, {} chain links), {} not loaded or blocked; {} divergences",
        origins.len(),
        t.programs,
        t.nodes,
        t.uses,
        t.holes,
        t.links,
        t.skipped,
        diverged.len()
    );
    assert!(diverged.is_empty(), "{}", diverged.join("\n"));
    assert!(t.programs > 500, "the corpus loaded: {} programs", t.programs);
}

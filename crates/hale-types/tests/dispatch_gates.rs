//! The dispatch gates are one set, the stdlib's rows included (F.40
//! phase 4, S9 1 of 3).
//!
//! The snapshot's `dispatch` family (`Snapshot::demand_dispatch_gates`)
//! is the gate set the dispatch plan is derived from: the bus graph's
//! rows keyed by wire, and the stdlib's rows after them, derived once
//! per process over the stdlib's analysis copy. Lowering's graph
//! (`bus_graph::lowering_bus_graph`) is the same rows read through the
//! merged program, the stdlib's merged sites among them. The law: over
//! every view of the corpus examples, the lifecycle fixtures, the Hale
//! tests and the DNA mains, build and harness snapshots, the family's
//! gates are lowering's graph's, column for column and in its order (the
//! subscribers in registration order, which the direct lowering bakes
//! and the plan's digest frames). And since the snapshot derives the
//! one plan from them (S9 2 of 3), the plan each view carries is the one
//! lowering's graph's own gates derive with the arrangement's domains,
//! the derivation lowering ran for itself before: every column, so every
//! plan digest the execution identity frames, is as it was.

use std::path::{Path, PathBuf};

use hale_frontend::frontend::LoadMode;
use hale_frontend::snapshot::{Config, Snapshot, Target};
use hale_frontend::source::Disk;
use hale_model::dispatch_plan::DispatchPlan;
use hale_model::DispatchGate;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

/// The `.hl` files of `dir`, and its directories where `dirs` is set.
fn entries(dir: &str, dirs: bool) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(root().join(dir))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "hl") || (dirs && p.is_dir()))
        .collect();
    out.sort();
    out
}

/// Every directory under `dir` that holds a `main.hl`.
fn mains(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut es: Vec<PathBuf> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).collect();
    es.sort();
    for p in es {
        if p.is_dir() {
            mains(&p, out);
        } else if p.file_name().is_some_and(|n| n == "main.hl") {
            out.push(p.parent().unwrap().to_path_buf());
        }
    }
}

/// One view's two gate sets, the family's and lowering's graph's, and
/// whether the plan the view carries is the one lowering's graph's
/// gates derive (with the arrangement's domains): the plan lowering
/// derived for itself until the family held it (F.40 phase 4, S9 2 of
/// 3), so its digest, which the execution identity frames, is the one
/// every build had. `None` where the snapshot has no lowering view.
fn gate_sets(target: &Path, harness: bool) -> Option<(Vec<DispatchGate>, Vec<DispatchGate>, bool)> {
    let target = target.to_path_buf();
    std::thread::scope(|s| {
        std::thread::Builder::new()
            .stack_size(256 << 20)
            .spawn_scoped(s, move || {
                let config = if harness { Config::harness(Target::host()) } else { Config::build(Target::host()) };
                let snap = Snapshot::load(&target, LoadMode::WholeSeed, &Disk, config).ok()?;
                let view = snap.demand_lowering().ok()?;
                let lowering = view.bus.dispatch_gates();
                let gates = snap.demand_dispatch_gates().expect("a lowered snapshot has its gates").to_vec();
                let domains = snap.demand_arrangement().expect("a lowered snapshot is arranged").domains();
                let its_graphs = view.plan == DispatchPlan::from_gates(&lowering, &domains);
                Some((gates, lowering, its_graphs))
            })
            .unwrap()
            .join()
            .unwrap()
    })
}

/// The stdlib's sinks on `log.**`, as lowering's graph registers them.
const SINKS: [&str; 3] = ["__StdLogStdoutSink", "__StdLogFileSink", "__StdLogConsoleSink"];

/// The law over every view of `targets`: the number of views compared.
fn hold_over(targets: &[PathBuf], min_views: usize) -> usize {
    let mut views = 0;
    let mut broken = Vec::new();
    for t in targets {
        let name = t.strip_prefix(root()).unwrap().display().to_string();
        for harness in [false, true] {
            let Some((gates, lowering, its_graphs)) = gate_sets(t, harness) else { continue };
            views += 1;
            if !its_graphs {
                broken.push(format!("{name} (harness: {harness}): the view's plan is not its graph's"));
            }
            if gates != lowering {
                let only = |a: &[DispatchGate], b: &[DispatchGate]| -> Vec<String> {
                    a.iter().filter(|g| !b.contains(g)).map(|g| format!("{g:?}")).collect()
                };
                broken.push(format!(
                    "{name} (harness: {harness}):\n  the family's {:?}\n  lowering's   {:?}",
                    only(&gates, &lowering),
                    only(&lowering, &gates)
                ));
            }
            // The stdlib's rows are in every view: the logger's subject,
            // its three sinks last, in their declaration order.
            let log = gates.iter().find(|g| g.subject == "log.**").expect("every view dispatches the stdlib's `log.**`");
            let tail: Vec<&str> = log.subscribers.iter().rev().take(3).rev().map(|(l, _)| l.as_str()).collect();
            assert_eq!(tail, SINKS, "{name} (harness: {harness}): the sinks close the subject's subscribers");
        }
    }
    assert!(broken.is_empty(), "the family's gates are not lowering's:\n{}", broken.join("\n"));
    assert!(views >= min_views, "only {views} views lowered");
    eprintln!("{views} views");
    views
}

#[test]
fn the_corpus_examples_gates_are_lowerings() {
    hold_over(&entries("crates/hale-codegen/tests/fixtures/examples", true), 200);
}

#[test]
fn the_lifecycle_fixtures_gates_are_lowerings() {
    hold_over(&entries("crates/hale-codegen/tests/fixtures/lifecycle", false), 110);
}

#[test]
fn the_hale_tests_gates_are_lowerings() {
    hold_over(&entries("tests/hale", false), 128);
}

/// The DNA mains, `fixtures` choosing the ones under a `tests`
/// directory or the rest: two halves, each a test of its own.
fn dna(fixtures: bool) -> Vec<PathBuf> {
    let mut targets = Vec::new();
    mains(&root().join("dna"), &mut targets);
    targets.retain(|t| t.components().any(|c| c.as_os_str() == "tests") == fixtures);
    targets
}

#[test]
fn the_dna_mains_gates_are_lowerings() {
    hold_over(&dna(false), 50);
}

#[test]
fn the_dna_test_fixtures_gates_are_lowerings() {
    hold_over(&dna(true), 45);
}

/// The one program that subscribes `log.**` itself shares the subject
/// with the stdlib's sinks: its own subscriber first, in registration
/// order, then the sinks, the order lowering dispatches in.
#[test]
fn a_program_subscribing_the_loggers_subject_shares_it_with_the_sinks() {
    let (gates, lowering, _) = gate_sets(&root().join("tests/hale/log_fields_test.hl"), false).expect("it lowers");
    assert_eq!(gates, lowering);
    let log = gates.iter().find(|g| g.subject == "log.**").unwrap();
    let loci: Vec<&str> = log.subscribers.iter().map(|(l, _)| l.as_str()).collect();
    assert!(loci.len() > SINKS.len() && !SINKS.contains(&loci[0]), "the program's own subscriber first: {loci:?}");
    assert_eq!(log.publisher_loci, ["__StdLogLogger"], "the logger publishes it");
}

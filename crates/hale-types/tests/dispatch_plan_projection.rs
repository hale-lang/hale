//! The model holds the program's one dispatch plan, projected (F.40
//! phase 4, S9 2 of 3).
//!
//! A snapshot derives its dispatch plan once (`demand_dispatch_plan`,
//! from the one gate set and the arrangement's domains). The lowering
//! view carries that plan; the model holds it projected onto the
//! subjects its own bus sites name and the loci it declares
//! (`DispatchPlan::projected`), and `--dump-model` prints the model's.
//! So the model's plan is lowering's on every subject the model names by
//! construction, which phase 3's law (`dispatch_plan_law.rs`) asserted
//! over two derivations; what is left to hold is the projection itself:
//! the dump prints exactly the snapshot's rows of the model's subjects,
//! and the stdlib's `log.**` row and sinks only where the program names
//! `log.**` itself.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use hale_frontend::frontend::LoadMode;
use hale_frontend::snapshot::{Config, Snapshot, Target};
use hale_frontend::source::Disk;
use hale_model::dispatch_plan::DispatchPlan;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

/// The dump's `dispatch_plan` section as `render_internal` renders a
/// plan: the header and one line per row.
fn section(plan: &DispatchPlan) -> String {
    let (same, total) = plan.same_domain_queued();
    let mut s = format!("dispatch_plan ({total} subjects, {same} same-domain queued):\n");
    for sp in &plan.subjects {
        s.push_str(&format!(
            "  {} {}{} pub[{}] sub[{}]{}\n",
            sp.subject,
            sp.flavor.as_str(),
            sp.ineligible_reason.as_deref().map(|r| format!(" ({r})")).unwrap_or_default(),
            sp.publisher_domains.join(","),
            sp.subscriber_domains.join(","),
            if sp.same_domain { " same-domain" } else { "" }
        ));
    }
    s
}

/// What one build snapshot of `target` says: the snapshot's plan, the
/// model's, the model's dump section, the subjects the checked bus graph
/// names (by wire) and the loci the model declares.
struct Seen {
    plan: DispatchPlan,
    model: DispatchPlan,
    dumped: String,
    subjects: BTreeSet<String>,
    loci: BTreeSet<String>,
    lowered: DispatchPlan,
    dispatch_builds: u32,
}

fn seen(target: &Path) -> Seen {
    let target = target.to_path_buf();
    std::thread::scope(|s| {
        std::thread::Builder::new()
            .stack_size(256 << 20)
            .spawn_scoped(s, move || {
                let snap = Snapshot::load(&target, LoadMode::WholeSeed, &Disk, Config::build(Target::host()))
                    .unwrap_or_else(|_| panic!("{} loads", target.display()));
                let m = snap.demand_model().expect("the model");
                let dump = hale_types::model_builder::render_internal(m);
                let dumped = dump[dump.find("dispatch_plan (").expect("the dump's section")..].to_string();
                let top = snap.demand_scope().unwrap();
                let subjects = snap
                    .demand_bus_graph()
                    .unwrap()
                    .subjects
                    .keys()
                    .map(|k| top.topics.named(k).map_or_else(|| k.clone(), |row| row.wire.clone()))
                    .collect();
                let lowered = snap.demand_lowering().expect("it lowers").plan.clone();
                Seen {
                    plan: snap.demand_dispatch_plan().unwrap().clone(),
                    model: m.analyses.dispatch_plan.clone(),
                    dumped,
                    subjects,
                    loci: m.entities.loci.iter().map(|l| l.name.clone()).collect(),
                    lowered,
                    dispatch_builds: snap.builds()["dispatch"],
                }
            })
            .unwrap()
            .join()
            .unwrap()
    })
}

/// The projection's definition, restated over what the snapshot holds:
/// the dump prints the snapshot's plan on the model's subjects, the
/// subscriber column over the model's loci, sorted.
fn holds(target: &str) -> Seen {
    let s = seen(&root().join(target));
    let subjects: BTreeSet<&str> = s.subjects.iter().map(String::as_str).collect();
    let loci: BTreeSet<&str> = s.loci.iter().map(String::as_str).collect();
    let projected = s.plan.projected(&subjects, &loci);
    assert_eq!(s.model, projected, "{target}: the model holds the snapshot's plan projected");
    assert_eq!(s.dumped, section(&projected), "{target}: the dump prints the projection");
    // Lowering's plan is the snapshot's, the one derivation.
    assert_eq!(s.lowered, s.plan, "{target}: the view carries the snapshot's plan");
    assert_eq!(s.dispatch_builds, 1, "{target}: the plan is derived once for the model and lowering");
    s
}

/// A program that names no `log.**`: the stdlib's row is the plan's
/// (lowering dispatches the logger's subject in every program) and not
/// the model's.
#[test]
fn the_stdlibs_row_is_lowerings_alone_where_the_program_does_not_name_it() {
    let s = holds("crates/hale-codegen/tests/fixtures/examples/91-module-decls");
    assert!(s.plan.subjects.iter().any(|p| p.subject == "log.**"), "the plan dispatches the logger's subject");
    assert!(!s.model.subjects.iter().any(|p| p.subject == "log.**"), "the model names no `log.**`");
    assert!(!s.dumped.contains("log.**"));
    assert!(s.model.subjects.iter().any(|p| p.same_domain), "a same-domain row, the domains the plan's");
}

/// A program that subscribes `log.**` itself shares the row: the model
/// holds it, with the program's own subscriber and not the stdlib's
/// sinks, which lowering's row carries after it in registration order.
#[test]
fn a_shared_subject_is_held_without_the_stdlibs_sinks() {
    let s = holds("tests/hale/log_fields_test.hl");
    let row = |p: &DispatchPlan| p.subjects.iter().find(|r| r.subject == "log.**").cloned().expect("the `log.**` row");
    let ours = row(&s.model);
    let lowering = row(&s.plan);
    assert!(ours.subscribers.iter().all(|(l, _)| !l.starts_with("__StdLog")), "{ours:?}");
    let sinks: Vec<&str> = lowering.subscribers.iter().filter(|(l, _)| l.starts_with("__StdLog")).map(|(l, _)| l.as_str()).collect();
    assert_eq!(sinks, ["__StdLogStdoutSink", "__StdLogFileSink", "__StdLogConsoleSink"]);
    assert_eq!(lowering.subscribers.len(), ours.subscribers.len() + 3);
    assert_eq!((ours.flavor, &ours.ineligible_reason), (lowering.flavor, &lowering.ineligible_reason));
}

/// A DNA main, whose imported seeds' loci are the gates' spelling of
/// them and whose subscribers the model orders differently from
/// lowering's registration order.
#[test]
fn a_dna_mains_dump_is_its_plan_projected() {
    let s = holds("dna/host");
    let reordered = s.plan.subjects.iter().filter(|p| {
        let mut sorted = p.subscribers.clone();
        sorted.sort();
        sorted.dedup();
        sorted != p.subscribers && s.model.subjects.iter().any(|m| m.subject == p.subject)
    });
    assert!(reordered.count() > 0, "a subject whose registration order is not the model's");
}

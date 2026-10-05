//! The model's dispatch plan is lowering's on the subjects they share
//! (F.40 phase 3, C5 2 of 2).
//!
//! Two plans are derived, from two gate sets, by one function
//! (`DispatchPlan::from_gates`) with one domain map
//! (`dispatch_plan::domain_map`, over the arrangement projection the
//! model's rows are made of): the model's from the checked graph's
//! gates, lowering's from the lowering graph's, which are the same rows
//! re-keyed plus the stdlib's (they exist only inside the lowering view,
//! so the two derivations are not one until the stdlib's rows are at the
//! snapshot). The law holds them equal where they overlap: over every
//! view, the model's plan is lowering's restricted to the subjects the
//! model's gates name, column for column, `same_domain` and the domain
//! lists included.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use hale_frontend::frontend::LoadMode;
use hale_frontend::snapshot::{Config, Snapshot, Target};
use hale_frontend::source::Disk;
use hale_model::dispatch_plan::{DispatchFlavor, DispatchPlan, SubjectPlan};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

/// Every column of a row.
type Columns<'a> =
    (&'a str, DispatchFlavor, Option<&'a str>, bool, &'a [(String, String)], &'a [String], &'a [String], bool);

fn columns(s: &SubjectPlan) -> Columns<'_> {
    (
        s.subject.as_str(),
        s.flavor,
        s.ineligible_reason.as_deref(),
        s.payload_flat,
        &s.subscribers,
        &s.publisher_domains,
        &s.subscriber_domains,
        s.same_domain,
    )
}

/// The law: every row of the model's plan is lowering's row for that
/// subject, column for column. Two things about the subscriber column
/// are each plan's own and are not compared, and nothing else is set
/// aside:
///
/// - its order: the model's gates hold a subject's subscribers sorted
///   (merged across sites), lowering's in the order the graph registers
///   them, which the direct lowering bakes and the digest frames;
/// - the stdlib's subscribers: a subject the stdlib's rows also
///   subscribe (the `log.**` sinks, under a program that subscribes
///   `log.**` itself) carries them in lowering's row only, since those
///   rows exist only inside the lowering view. They are loci the model
///   declares none of (`model_loci`), so lowering's column is compared
///   over the model's loci. Their domains are unknown to the map, so
///   they would forfeit lowering's `same_domain` where the model's
///   holds it: the law then fails, as it should.
fn law(model: &DispatchPlan, lowering: &DispatchPlan, model_loci: &BTreeSet<&str>) -> Result<(), String> {
    for m in &model.subjects {
        let Some(l) = lowering.subjects.iter().find(|l| l.subject == m.subject) else {
            return Err(format!("`{}` is in the model's plan and not lowering's", m.subject));
        };
        let mut ours = m.clone();
        ours.subscribers.sort();
        let mut theirs = l.clone();
        theirs.subscribers.retain(|(locus, _)| model_loci.contains(locus.as_str()));
        theirs.subscribers.sort();
        if columns(&ours) != columns(&theirs) {
            return Err(format!("`{}`:\n  model    {:?}\n  lowering {:?}", m.subject, columns(m), columns(l)));
        }
    }
    Ok(())
}

/// The shared rows where lowering's subscribers include loci the model
/// does not declare (the stdlib's): `(subject, those loci)`.
fn stdlib_subscribed(model: &DispatchPlan, lowering: &DispatchPlan, model_loci: &BTreeSet<&str>) -> Vec<String> {
    model
        .subjects
        .iter()
        .filter_map(|m| {
            let l = lowering.subjects.iter().find(|l| l.subject == m.subject)?;
            let extra: Vec<&str> =
                l.subscribers.iter().map(|(locus, _)| locus.as_str()).filter(|x| !model_loci.contains(x)).collect();
            (!extra.is_empty()).then(|| format!("{} {extra:?}", m.subject))
        })
        .collect()
}

/// One view's two plans.
struct Plans {
    model: DispatchPlan,
    lowering: DispatchPlan,
    /// The loci the model declares, by name.
    model_loci: BTreeSet<String>,
}

impl Plans {
    fn loci(&self) -> BTreeSet<&str> {
        self.model_loci.iter().map(String::as_str).collect()
    }
}

/// The model's plan and lowering's, over one snapshot; `None` where the
/// snapshot has no model or no lowering view.
fn plans(target: &Path, harness: bool) -> Option<Plans> {
    let target = target.to_path_buf();
    std::thread::scope(|s| {
        std::thread::Builder::new()
            .stack_size(256 << 20)
            .spawn_scoped(s, move || {
                let config = if harness { Config::harness(Target::host()) } else { Config::build(Target::host()) };
                let snap = Snapshot::load(&target, LoadMode::WholeSeed, &Disk, config).ok()?;
                let lowering = snap.demand_lowering().ok()?.plan.clone();
                let m = snap.demand_model().ok()?;
                Some(Plans {
                    model: DispatchPlan::derive(m),
                    lowering,
                    model_loci: m.entities.loci.iter().map(|l| l.name.clone()).collect(),
                })
            })
            .unwrap()
            .join()
            .unwrap()
    })
}

fn views(dir: &str, keep: fn(&Path) -> bool) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> =
        std::fs::read_dir(root().join(dir)).unwrap().map(|e| e.unwrap().path()).filter(|p| keep(p)).collect();
    out.sort();
    out
}

/// The law over every view of `targets` (the build's and the harness's
/// snapshot of each), and the shared rows the stdlib's subscribers
/// join, `view subject [loci]`.
fn hold_over(targets: &[PathBuf], min_views: usize) -> Vec<String> {
    let mut checked = 0;
    let mut rows = 0;
    let mut same_domain = 0;
    let mut broken = Vec::new();
    let mut stdlib = Vec::new();
    for t in targets {
        let name = t.strip_prefix(root()).unwrap().display().to_string();
        for harness in [false, true] {
            let Some(p) = plans(t, harness) else { continue };
            checked += 1;
            rows += p.model.subjects.len();
            same_domain += p.model.subjects.iter().filter(|s| s.same_domain).count();
            if let Err(e) = law(&p.model, &p.lowering, &p.loci()) {
                broken.push(format!("{name} (harness: {harness}): {e}"));
            }
            for s in stdlib_subscribed(&p.model, &p.lowering, &p.loci()) {
                stdlib.push(format!("{name} (harness: {harness}) {s}"));
            }
        }
    }
    eprintln!("{checked} views, {rows} shared rows, {same_domain} same-domain");
    assert!(broken.is_empty(), "the model's plan is not lowering's on the subjects they share:\n{}", broken.join("\n"));
    // Vacuity: the views were walked, and the shared rows carry domains.
    assert!(checked >= min_views, "only {checked} views had both plans");
    assert!(rows > 0 && same_domain > 0, "{rows} shared rows, {same_domain} same-domain");
    stdlib
}

#[test]
fn the_corpus_examples_plans_agree() {
    let stdlib = hold_over(&views("crates/hale-codegen/tests/fixtures/examples", |p| p.is_dir()), 180);
    assert!(stdlib.is_empty(), "{stdlib:?}");
}

#[test]
fn the_hale_tests_plans_agree() {
    let stdlib = hold_over(&views("tests/hale", |p| p.to_string_lossy().ends_with("_test.hl")), 130);
    // The one program that subscribes `log.**` itself shares the subject
    // with the stdlib's sinks.
    let sinks = r#"log.** ["__StdLogStdoutSink", "__StdLogFileSink", "__StdLogConsoleSink"]"#;
    assert_eq!(
        stdlib,
        [false, true].map(|h| format!("tests/hale/log_fields_test.hl (harness: {h}) {sinks}")),
        "the shared rows the stdlib's subscribers join"
    );
}

/// The five DNA mains, each through two snapshots: about 8 s in a
/// release build (the corpus examples' 194 views take about 3 s, the
/// Hale tests' 130 about 2 s).
#[test]
fn the_dna_mains_plans_agree() {
    let stdlib = hold_over(&views("dna", |p| p.is_dir() && p.join("main.hl").exists()), 10);
    assert!(stdlib.is_empty(), "{stdlib:?}");
}

/// The control: the law fails when any one column of a shared row is
/// perturbed. The subscriber perturbation drops one the model's plan
/// names, which no carve-out covers.
#[test]
fn the_law_sees_every_column() {
    let target = root().join("crates/hale-codegen/tests/fixtures/examples/91-module-decls");
    let p = plans(&target, false).expect("both plans");
    law(&p.model, &p.lowering, &p.loci()).expect("the law holds before the perturbation");
    let at = p.lowering.subjects.iter().position(|s| s.same_domain).expect("a same-domain row");
    assert!(!p.lowering.subjects[at].subscribers.is_empty());
    let perturbations: [(&str, fn(&mut SubjectPlan)); 8] = [
        ("subject", |s| s.subject.push('x')),
        ("flavor", |s| {
            s.flavor = if s.flavor == DispatchFlavor::Dynamic { DispatchFlavor::StaticBucket } else { DispatchFlavor::Dynamic }
        }),
        ("ineligible_reason", |s| s.ineligible_reason = Some("perturbed".to_string())),
        ("payload_flat", |s| s.payload_flat = !s.payload_flat),
        ("subscribers", |s| {
            s.subscribers.pop();
        }),
        ("publisher_domains", |s| s.publisher_domains.push("pool:other".to_string())),
        ("subscriber_domains", |s| s.subscriber_domains.clear()),
        ("same_domain", |s| s.same_domain = !s.same_domain),
    ];
    for (column, perturb) in perturbations {
        let mut perturbed = p.lowering.clone();
        perturb(&mut perturbed.subjects[at]);
        assert!(law(&p.model, &perturbed, &p.loci()).is_err(), "perturbing `{column}` must break the law");
    }
}

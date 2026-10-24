//! Flow policy consumes the canonical classification, independently of
//! instance populations or a proof that a child's run terminates.
#[path = "support/entries.rs"]
mod entries;
use hale_frontend::snapshot::{Config, Snapshot};
use hale_model::{ApplicationModel, ClaimIrTable, EvidenceTable, JudgmentFamily, RelationSet};
use hale_types::topology_projection::{judge_all, project_law_rows, project_shape_hash};
use hale_types::verdict::Verdict;

const BASE: &str = r#"
locus Job { run() { } }
locus Resident { }
locus Owner<T> { accept(k: T) { } release(k: T) { } }
group requests = { Job };
main locus App {
    claims { reclaimed: require flow(all requests); }
}
fn main() { let owner: Owner<Job> = Owner { }; App { }; Job { }; }
"#;

fn prepare(source: &str) -> (Snapshot, ApplicationModel, ClaimIrTable, Vec<u32>) {
    let s = entries::load_files(&[("main.hl", source)], Config::check(true, false));
    let model = s.demand_model().unwrap().clone();
    model.validate().unwrap();
    let table = hale_types::claim_lowering::lower_claims_over(
        &s.bundle(),
        &model,
        s.demand_law_selection().unwrap(),
    );
    table.validate(&model).unwrap();
    let bases = s.bundle().sources.iter().map(|s| s.base).collect();
    (s, model, table, bases)
}
fn verdict(source: &str, expected: Verdict) -> Vec<String> {
    let (s, model, table, bases) = prepare(source);
    let evidence = EvidenceTable::default();
    let (_, judged) = judge_all(&table, &model, &evidence, &bases);
    assert_eq!(judged.len(), 1);
    let j = judged.values().next().unwrap();
    assert_eq!(j.verdict, expected, "{:?}", j.diags);
    assert_eq!(table.rows[0].family(), JudgmentFamily::FlowPolicy);
    assert_eq!(
        JudgmentFamily::FlowPolicy.required_relations(),
        RelationSet::FLOW_CLASSIFICATION
    );
    let checked = entries::checked(&s);
    for d in &j.diags {
        assert!(
            checked
                .iter()
                .any(|c| c.message == d.message && c.span == d.span),
            "check lost {d:?}"
        );
    }
    if expected == Verdict::Holds {
        assert!(checked.is_empty(), "{checked:?}");
    } else {
        assert!(!j.diags.is_empty());
    }
    let (claims, _, laws, _) = project_law_rows(
        &s.bundle(),
        &model,
        &table,
        &evidence,
        &bases,
        &Default::default(),
    );
    assert_eq!(claims[0].result, expected);
    assert_eq!(laws[0].verdict, expected);
    assert_eq!(laws[0].evidence.len(), j.diags.len());
    assert!(laws[0]
        .evidence
        .iter()
        .all(|(_, location)| location.is_some()));
    j.diags.iter().map(|d| d.message.clone()).collect()
}
#[test]
fn specialized_release_proves_the_concrete_policy_despite_unrelated_holes() {
    verdict(BASE, Verdict::Holds);
    let (_, m, _, _) = prepare(BASE);
    assert!(!m.capabilities.exact_flow_classification);
    assert!(m
        .holes
        .iter()
        .any(|h| h.hides == RelationSet::FLOW_CLASSIFICATION));
}
#[test]
fn missing_release_or_specialization_has_a_resident_witness() {
    for source in [
        BASE.replace("release(k: T) { }", ""),
        BASE.replace("let owner: Owner<Job> = Owner { };", ""),
    ] {
        let ds = verdict(&source, Verdict::Violated);
        assert!(
            ds[0].contains("`Job`") && ds[0].contains("is resident"),
            "{ds:?}"
        );
    }
}
#[test]
fn templates_are_uncertified_and_a_known_resident_still_violates() {
    verdict(
        &BASE.replace("{ Job };", "{ Owner };"),
        Verdict::Uncertified,
    );
    verdict(
        &BASE.replace("{ Job };", "{ Owner, Resident };"),
        Verdict::Violated,
    );
}
#[test]
fn absent_empty_or_function_only_selections_are_invalid() {
    for source in [
        BASE.replace("all requests", "all absent"),
        BASE.replace("{ Job };", "{ } may_be_empty;"),
        BASE.replace("{ Job };", "{ main };"),
    ] {
        verdict(&source, Verdict::Invalid);
    }
}
#[test]
fn run_completion_is_not_a_termination_promise() {
    verdict(
        &BASE.replace("run() { }", "run() { while true { yield; } }"),
        Verdict::Holds,
    );
}
#[test]
fn unvouched_absence_does_not_prove_a_resident_counterexample() {
    let source = BASE
        .replace("Owner<T>", "Owner")
        .replace("k: T", "k: Job")
        .replace("Owner<Job>", "Owner");
    let (_, mut m, t, bases) = prepare(&source);
    assert!(m.capabilities.exact_flow_classification);
    m.capabilities.exact_flow_classification = false;
    m.validate().unwrap();
    let (_, judged) = judge_all(&t, &m, &EvidenceTable::default(), &bases);
    assert_eq!(judged[&0].verdict, Verdict::Uncertified);
    let no_release = source.replace("release(k: Job) { }", "");
    let (_, mut m, t, bases) = prepare(&no_release);
    m.capabilities.exact_flow_classification = false;
    let (_, judged) = judge_all(&t, &m, &EvidenceTable::default(), &bases);
    assert_eq!(judged[&0].verdict, Verdict::Uncertified);
}
#[test]
fn flow_classification_is_hashed_independently_of_law_selection() {
    let (_, m, _, _) = prepare(BASE);
    let (_, no_law, _, _) = prepare(&BASE.replace("reclaimed: require flow(all requests);", ""));
    assert_eq!(project_shape_hash(&m), project_shape_hash(&no_law));
    let (_, resident, _, _) = prepare(&BASE.replace("release(k: T) { }", ""));
    assert_ne!(project_shape_hash(&m), project_shape_hash(&resident));
}

#[test]
fn imported_library_and_constitution_bind_the_library_group() {
    let main = "import \"./lib\" as lib; locus Shadow { } group requests = { Shadow }; main locus App { claims { adopt FlowLaw; } } fn main() { App { }; }";
    let library = "locus Job { } locus Owner { accept(k: Job) { } release(k: Job) { } } group requests = { Job }; constitution FlowLaw { reclaimed: require flow(all requests); }";
    for (main, library) in [
        (main.to_string(), library.to_string()),
        (
            main.replace("adopt FlowLaw;", ""),
            library.replace("constitution FlowLaw", "claims"),
        ),
    ] {
        let s = entries::load_files(
            &[("main.hl", &main), ("lib/worker.hl", &library)],
            Config::check(true, false),
        );
        let ds = entries::checked(&s);
        assert!(ds.is_empty(), "{ds:?}");
        let bundle = s.bundle();
        let model = s.demand_model().unwrap();
        let table = hale_types::claim_lowering::lower_claims_over(
            &bundle,
            model,
            s.demand_law_selection().unwrap(),
        );
        let hale_model::ClaimIr::RequireFlow { group } = &table.rows[0].law else {
            panic!()
        };
        let selected: Vec<_> = model
            .relations
            .group_members
            .iter()
            .filter(|m| Some(m.group) == group.group)
            .filter_map(|m| match m.member {
                hale_model::EntityRef::LocusDecl(id) => {
                    Some(&model.entities.loci[id.index()].display)
                }
                _ => None,
            })
            .collect();
        assert!(selected.iter().any(|n| n.ends_with("Job")), "{selected:?}");
        assert!(!selected.iter().any(|n| n.as_str() == "Shadow"));
    }
}

#[test]
fn model_diff_names_a_flow_classification_change() {
    let before = BASE.replace("reclaimed: require flow(all requests);", "");
    let after = before.replace("release(k: T) { }", "");
    let artifact = |source: &str| {
        let (s, _, _, _) = prepare(source);
        entries::dump_topology(&s.bundle())
    };
    let a = hale_types::topology_diff::admit("before", &artifact(&before)).unwrap();
    let b = hale_types::topology_diff::admit("after", &artifact(&after)).unwrap();
    let diff = hale_types::topology_diff::diff(&a, &b);
    assert_eq!(diff["classification"], "model-shape");
    assert!(
        diff["contracts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["locus"] == "Job"
                && row["facet"] == "flow"
                && row["removed"][0] == "flow"
                && row["added"][0] == "resident"),
        "{diff}"
    );
}

#[test]
fn aliases_and_interface_admissions_keep_concrete_flow_membership() {
    let alias = BASE
        .replace("locus Owner<T>", "type Alias = Job; locus Owner")
        .replace("k: T", "k: Alias")
        .replace("Owner<Job>", "Owner");
    verdict(&alias, Verdict::Holds);
    let interface = BASE
        .replace(
            "locus Job { run() { } }",
            "interface Child { fn touch(); } locus Job { fn touch() { } run() { } }",
        )
        .replace("locus Owner<T>", "locus Owner")
        .replace("k: T", "k: Child")
        .replace("Owner<Job>", "Owner");
    verdict(&interface, Verdict::Holds);
}

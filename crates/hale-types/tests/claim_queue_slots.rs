//! Aggregate queue reservations use the compiler's claim and artifact paths.

#[path = "support/entries.rs"]
mod entries;

use hale_frontend::snapshot::Config;
use hale_model::{ApplicationModel, ClaimIrTable, EvidenceTable};
use hale_types::topology_projection::{judge_all, project_law_rows};
use hale_types::verdict::Verdict;

const BASE: &str = r#"
type Message { n: Int; }
topic Work { payload: Message; }
locus Worker {
    params { n: Int = 0; }
    bus { subscribe Work as on_work bounded(8, drop_new); }
    fn on_work(m: Message) { self.n = self.n + m.n; }
}
group workers = { Worker };
main locus App {
    params { w: Worker = Worker { }; other: Worker = Worker { }; third: Worker = Worker { }; }
    bus { publish Work; }
    claims { slots: bound queue_slots <= 24 on instances workers; }
    run() { Work <- Message { n: 1 }; }
}
fn main() { App { }; }
"#;

fn prepare(source: &str) -> (ApplicationModel, ClaimIrTable, Vec<u32>) {
    let snapshot = entries::load_files(&[("main.hl", source)], Config::check(true, false));
    let bundle = snapshot.bundle();
    let model = entries::model_of(&snapshot, &[]);
    assert_eq!(model.validate(), Ok(()));
    let table = hale_types::claim_lowering::lower_claims_over(
        &bundle,
        &model,
        snapshot.demand_law_selection().unwrap(),
    );
    assert_eq!(table.validate(&model), Ok(()));
    let bases = bundle.sources.iter().map(|source| source.base).collect();
    (model, table, bases)
}

fn assert_verdict(source: &str, expected: Verdict) -> Vec<String> {
    let snapshot = entries::load_files(&[("main.hl", source)], Config::check(true, false));
    let bundle = snapshot.bundle();
    let (model, table, bases) = prepare(source);
    let evidence = EvidenceTable::default();
    let (_, judged) = judge_all(&table, &model, &evidence, &bases);
    assert_eq!(judged.len(), 1, "{judged:?}");
    let judgment = judged.values().next().unwrap();
    assert_eq!(judgment.verdict, expected, "{:?}", judgment.diags);
    let checked = entries::checked(&snapshot);
    for diagnostic in &judgment.diags {
        assert!(
            checked
                .iter()
                .any(|d| d.message == diagnostic.message && d.span == diagnostic.span),
            "check lost judgment diagnostic: {diagnostic:?}; {checked:?}"
        );
    }
    if expected == Verdict::Holds {
        assert!(checked.is_empty(), "{checked:?}");
    } else {
        assert!(!judgment.diags.is_empty());
    }
    let (claims, _, laws, _) = project_law_rows(
        &bundle,
        &model,
        &table,
        &evidence,
        &bases,
        &Default::default(),
    );
    assert_eq!(claims[0].result, expected);
    assert_eq!(laws[0].family, hale_model::JudgmentFamily::Resource);
    assert_eq!(laws[0].verdict, expected);
    assert_eq!(laws[0].evidence.len(), judgment.diags.len());
    assert!(laws[0]
        .evidence
        .iter()
        .all(|(_, location)| location.is_some()));
    judgment
        .diags
        .iter()
        .map(|diag| diag.message.clone())
        .collect()
}

#[test]
fn arranged_instances_reserve_capacity_individually() {
    assert_verdict(BASE, Verdict::Holds);
    let diagnostics = assert_verdict(&BASE.replace("<= 24", "<= 23"), Verdict::Violated);
    assert!(
        diagnostics[0].contains("24 subscription queue slots"),
        "{diagnostics:?}"
    );
    assert_eq!(
        diagnostics
            .iter()
            .filter(|d| d.contains("witness:"))
            .count(),
        3
    );
}

#[test]
fn repeated_constructions_are_counted_even_on_one_domain() {
    let source = BASE.replace("params { w: Worker = Worker { }; other: Worker = Worker { }; third: Worker = Worker { }; }",
        "params { w: Worker = Worker { }; other: Worker = Worker { }; }");
    assert_verdict(&source.replace("<= 24", "<= 16"), Verdict::Holds);
    let diagnostics = assert_verdict(&source.replace("<= 24", "<= 15"), Verdict::Violated);
    assert!(diagnostics[0].contains("16 subscription queue slots"));
}

#[test]
fn unbounded_registration_defeats_any_finite_reservation() {
    let source = BASE.replace(" bounded(8, drop_new)", "");
    let diagnostics = assert_verdict(&source, Verdict::Violated);
    assert!(diagnostics[0].contains("unbounded"));
}

#[test]
fn empty_or_missing_or_undeployed_selections_are_invalid() {
    for source in [
        BASE.replace("{ Worker };", "{ } may_be_empty;"),
        BASE.replace("on instances workers", "on instances absent"),
        BASE.replace(
            "group workers = { Worker };",
            "locus Unused { } group workers = { Unused };",
        ),
    ] {
        assert_verdict(&source, Verdict::Invalid);
    }
}

#[test]
fn unknown_population_is_uncertified_but_known_excess_is_violated() {
    for (source, expected) in [
        (BASE.to_string(), Verdict::Uncertified),
        (BASE.replace("<= 24", "<= 23"), Verdict::Violated),
    ] {
        let (mut model, table, bases) = prepare(&source);
        let (index, locus) = model
            .entities
            .loci
            .iter()
            .enumerate()
            .find(|(_, locus)| locus.display == "Worker")
            .unwrap();
        model.holes.push(hale_model::Hole {
            at: hale_model::EntityRef::LocusDecl(hale_model::LocusDeclId(index as u32)),
            kind: hale_model::HoleKind::RuntimeInheritedPlacement,
            hides: hale_model::RelationSet::OWNS.union(hale_model::RelationSet::PLACED),
            reason: "additional dynamic worker population".to_string(),
            authored_site: None,
            provenance: locus.provenance,
        });
        model.capabilities.exact_ownership = false;
        model.capabilities.exact_placement = false;
        assert_eq!(model.validate(), Ok(()));
        let (_, judgments) = judge_all(&table, &model, &EvidenceTable::default(), &bases);
        assert_eq!(judgments.values().next().unwrap().verdict, expected);
    }
}

#[test]
fn reservation_arithmetic_cannot_wrap() {
    let (mut model, table, bases) = prepare(BASE);
    model.relations.subscribes[0].capacity = hale_model::keys::Capacity::Bounded(u64::MAX);
    let (_, judgments) = judge_all(&table, &model, &EvidenceTable::default(), &bases);
    assert_eq!(
        judgments.values().next().unwrap().verdict,
        Verdict::Violated
    );
}

#[test]
fn placement_unknowns_are_irrelevant_to_declared_reservations() {
    let (mut model, table, bases) = prepare(BASE);
    model.relations.placed_in.clear();
    assert_eq!(model.validate(), Ok(()));
    let (_, judgments) = judge_all(&table, &model, &EvidenceTable::default(), &bases);
    assert_eq!(judgments.values().next().unwrap().verdict, Verdict::Holds);
}

#[test]
fn missing_positive_coverage_does_not_prove_an_upper_bound() {
    let (mut model, table, bases) = prepare(BASE);
    model.capabilities.exact_subscribes = false;
    assert_eq!(model.validate(), Ok(()));
    let (_, judgments) = judge_all(&table, &model, &EvidenceTable::default(), &bases);
    assert_eq!(
        judgments.values().next().unwrap().verdict,
        Verdict::Uncertified
    );
}

#[test]
fn an_instantiated_locus_without_queues_has_zero_reservation() {
    let source = BASE
        .replace(
            "bus { subscribe Work as on_work bounded(8, drop_new); }",
            "",
        )
        .replace("topic Work { payload: Message; }", "")
        .replace("bus { publish Work; }", "")
        .replace("Work <- Message { n: 1 };", "")
        .replace("<= 24", "<= 0");
    assert_verdict(&source, Verdict::Holds);
}

#[test]
fn each_subscription_site_contributes_its_own_capacity() {
    let source = BASE
        .replace(
            "topic Work { payload: Message; }",
            "topic Work { payload: Message; } topic Other { payload: Message; }",
        )
        .replace(
            "bounded(8, drop_new);",
            "bounded(8, drop_new); subscribe Other as on_work bounded(4, drop_new);",
        )
        .replace(
            "bus { publish Work; }",
            "bus { publish Work; publish Other; }",
        )
        .replace(
            "Work <- Message { n: 1 };",
            "Work <- Message { n: 1 }; Other <- Message { n: 1 };",
        );
    assert_verdict(&source.replace("<= 24", "<= 36"), Verdict::Holds);
    let diagnostics = assert_verdict(&source.replace("<= 24", "<= 35"), Verdict::Violated);
    assert!(diagnostics[0].contains("36 subscription queue slots"));
}

#[test]
fn dynamic_births_withdraw_a_finite_population_proof() {
    let source = BASE.replace("run() { Work", "run() { let extra = Worker { }; Work");
    assert_verdict(&source, Verdict::Uncertified);
}

#[test]
fn queue_limit_and_group_are_part_of_the_law_identity() {
    let (model, table, _) = prepare(BASE);
    let mut changed = table.clone();
    let hale_model::ClaimIr::QueueSlots { limit, .. } = &mut changed.rows[0].law else {
        panic!()
    };
    *limit += 1;
    assert_ne!(table.semantic_digest(), changed.semantic_digest());
    let payload = changed.rows[0].law_payload_json(&model.entities, &changed.provenance);
    assert!(payload.contains("queue_slots"));
    assert_eq!(changed.validate(&model), Ok(()));
}

#[test]
fn an_unknown_registration_cannot_disappear_from_a_zero_subtotal() {
    let (mut model, table, bases) = prepare(BASE);
    let subscription = model.relations.subscribes[0].clone();
    model.relations.subscribes.clear();
    model.holes.push(hale_model::Hole {
        at: hale_model::EntityRef::Subject(subscription.subject),
        kind: hale_model::HoleKind::DynamicEndpoint,
        hides: hale_model::RelationSet::SUBSCRIBES,
        reason: "unknown subscription ownership and capacity".to_string(),
        authored_site: None,
        provenance: subscription.provenance,
    });
    model.capabilities.exact_subscribes = false;
    assert_eq!(model.validate(), Ok(()));
    let (_, judgments) = judge_all(&table, &model, &EvidenceTable::default(), &bases);
    assert_eq!(
        judgments.values().next().unwrap().verdict,
        Verdict::Uncertified
    );
}

#[test]
fn a_replica_population_is_not_collapsed_to_one_declaration() {
    // Begin with a real checked arrangement, then exercise the query
    // against distinct replica identities. Capacity remains a model row;
    // the claim engine cannot recover or replace it by re-walking source.
    let (mut model, table, bases) = prepare(BASE);
    let mut index = 0;
    for instance in &mut model.entities.locus_instances {
        if model.entities.loci[instance.decl.index()].display == "Worker" {
            instance.path = format!("App.workers[{}]", index);
            instance.replica = Some(index);
            index += 1;
        }
    }
    assert_eq!(index, 3);
    assert_eq!(model.validate(), Ok(()));
    let (_, judgments) = judge_all(&table, &model, &EvidenceTable::default(), &bases);
    assert_eq!(judgments.values().next().unwrap().verdict, Verdict::Holds);
    model.relations.subscribes[0].capacity = hale_model::keys::Capacity::Bounded(9);
    let (_, judgments) = judge_all(&table, &model, &EvidenceTable::default(), &bases);
    assert_eq!(
        judgments.values().next().unwrap().verdict,
        Verdict::Violated
    );
}

#[test]
fn imported_library_and_constitution_keep_their_group_binding() {
    let main = r#"
        import "./lib" as lib;
        locus Shadow { }
        group workers = { Shadow };
        main locus App {
            params { a: lib::Worker = lib::Worker { }; b: lib::Worker = lib::Worker { }; }
            bus { publish lib::Work; }
            claims { adopt QueueLaw; }
            run() { lib::Work <- lib::Message { n: 1 }; }
        }
        fn main() { App { }; }
    "#;
    let library = r#"
        type Message { n: Int; }
        topic Work { payload: Message; }
        locus Worker {
            bus { subscribe Work as receive bounded(8, drop_new); }
            fn receive(m: Message) { }
        }
        group workers = { Worker };
        constitution QueueLaw { slots: bound queue_slots <= 16 on instances workers; }
    "#;
    for (main, library) in [
        (main.to_string(), library.to_string()),
        (
            main.replace("claims { adopt QueueLaw; }", ""),
            library.replace("constitution QueueLaw", "claims"),
        ),
    ] {
        let snapshot = entries::load_files(
            &[("main.hl", &main), ("lib/lib.hl", &library)],
            Config::check(true, false),
        );
        assert!(
            entries::checked(&snapshot).is_empty(),
            "{:?}",
            entries::checked(&snapshot)
        );
        let library = library.replace("<= 16", "<= 15");
        let snapshot = entries::load_files(
            &[("main.hl", &main), ("lib/lib.hl", &library)],
            Config::check(true, false),
        );
        let diags = entries::checked(&snapshot);
        assert!(
            diags
                .iter()
                .any(|d| d.message.contains("16 subscription queue slots")),
            "{diags:?}"
        );
    }
}

//! GH #1327: execution-domain separation is a compiler claim over the
//! snapshot's instance/placement rows, shared by check and artifact.

#[path = "support/entries.rs"]
mod entries;

use hale_frontend::snapshot::Config;
use hale_model::{ApplicationModel, ClaimIrTable, EvidenceTable};
use hale_types::judgment::Judged;
use hale_types::topology_projection::{judge_all, project_law_rows};
use hale_types::verdict::Verdict;

const BASE: &str = r#"
locus Alpha { }
locus Beta { }
group writers = { Alpha };
group readers = { Beta };
main locus App {
    params { a: Alpha = Alpha { }; b: Beta = Beta { }; }
    placement { a: cooperative(pool = write); b: cooperative(pool = read); }
    claims { separate: require separate_domains(all writers, all readers); }
}
fn main() { App { }; }
"#;

fn judge_files(files: &[(&str, &str)]) -> (Verdict, Vec<String>) {
    let snap = entries::load_files(files, Config::check(true, false));
    let bundle = snap.bundle();
    let model = entries::model_of(&snap, &[]);
    assert_eq!(model.validate(), Ok(()));
    let table = hale_types::claim_lowering::lower_claims_over(
        &bundle,
        &model,
        snap.demand_law_selection().expect("selection"),
    );
    assert_eq!(table.validate(&model), Ok(()));
    let bases: Vec<_> = bundle.sources.iter().map(|source| source.base).collect();
    let (_, judged) = judge_all(&table, &model, &EvidenceTable::default(), &bases);
    assert_eq!(judged.len(), 1, "{judged:?}");
    let result = judged.values().next().unwrap();
    let checked = entries::checked(&snap);
    for diag in &result.diags {
        assert!(
            checked
                .iter()
                .any(|d| d.message == diag.message && d.span == diag.span),
            "check lost judgment diagnostic {diag:?}: {checked:?}"
        );
    }
    if result.verdict == Verdict::Holds {
        assert!(checked.is_empty(), "{checked:?}");
    } else {
        assert!(!result.diags.is_empty(), "no silent non-holds verdict");
    }
    let (claims, _, laws, _) = project_law_rows(
        &bundle,
        &model,
        &table,
        &EvidenceTable::default(),
        &bases,
        &Default::default(),
    );
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].result, result.verdict);
    assert_eq!(laws.len(), 1);
    assert_eq!(laws[0].family, hale_model::JudgmentFamily::Placement);
    assert_eq!(laws[0].verdict, result.verdict);
    assert_eq!(laws[0].evidence.len(), result.diags.len());
    assert!(
        laws[0]
            .evidence
            .iter()
            .all(|(_, location)| location.is_some()),
        "every evidence row retains its source: {:?}",
        laws[0].evidence
    );
    (
        result.verdict,
        result.diags.iter().map(|d| d.message.clone()).collect(),
    )
}

fn assert_verdict(source: &str, expected: Verdict) -> Vec<String> {
    let (verdict, diags) = judge_files(&[("main.hl", source)]);
    assert_eq!(verdict, expected, "{diags:?}");
    diags
}

#[test]
fn distinct_cooperative_pools_hold() {
    assert_verdict(BASE, Verdict::Holds);
}

#[test]
fn shared_pool_reports_both_instances_and_domain() {
    let ds = assert_verdict(
        &BASE.replace("pool = read", "pool = write"),
        Verdict::Violated,
    );
    assert!(
        ds[0].contains("App.a") && ds[0].contains("App.b") && ds[0].contains("pool:write"),
        "{ds:?}"
    );
}

#[test]
fn implicit_main_is_a_shared_domain() {
    let src = BASE.replace(
        "placement { a: cooperative(pool = write); b: cooperative(pool = read); }",
        "",
    );
    let ds = assert_verdict(&src, Verdict::Violated);
    assert!(ds[0].contains("domain `main`"), "{ds:?}");
}

#[test]
fn distinct_pinned_threads_hold_even_on_the_same_cpu() {
    let src = BASE
        .replace(
            "cooperative(pool = write)",
            "pinned(cores = { 0 }, replicas = 2)",
        )
        .replace("cooperative(pool = read)", "pinned(cores = { 0 })");
    assert_verdict(&src, Verdict::Holds);
}

#[test]
fn every_instance_of_a_declaration_participates() {
    let src = BASE
        .replace(
            "b: Beta = Beta { };",
            "b: Beta = Beta { }; extra: Alpha = Alpha { };",
        )
        .replace(
            "b: cooperative(pool = read);",
            "b: cooperative(pool = read); extra: cooperative(pool = read);",
        );
    let ds = assert_verdict(&src, Verdict::Violated);
    assert!(
        ds[0].contains("App.extra") && ds[0].contains("pool:read"),
        "{ds:?}"
    );
}

#[test]
fn inherited_domains_are_not_independent_domains() {
    let src = BASE.replace(
        "locus Alpha { }",
        "locus Alpha { params { child: Beta = Beta { }; } }",
    );
    let ds = assert_verdict(&src, Verdict::Violated);
    assert!(
        ds[0].contains("App.a.child") && ds[0].contains("pool:write"),
        "{ds:?}"
    );
}

#[test]
fn selecting_a_locus_does_not_implicitly_select_its_children() {
    let src = BASE
        .replace(
            "locus Alpha { }",
            "locus Alpha { params { child: Leaf = Leaf { }; } } locus Leaf { }",
        )
        .replace("claims {", "run() { let extra = Leaf { }; } claims {");
    assert_verdict(&src, Verdict::Holds);
}

#[test]
fn the_same_instance_in_both_groups_is_a_collision() {
    let src = BASE.replace("group readers = { Beta };", "group readers = { Alpha };");
    let ds = assert_verdict(&src, Verdict::Violated);
    assert!(ds[0].matches("App.a").count() == 2, "{ds:?}");
}

#[test]
fn dynamic_birth_withdraws_a_separation_proof() {
    let src = BASE.replace("claims {", "run() { let extra = Alpha { }; } claims {");
    let ds = assert_verdict(&src, Verdict::Uncertified);
    assert!(
        ds.iter().any(|s| s.contains("outside the arrangement")),
        "{ds:?}"
    );
}

#[test]
fn dynamic_only_population_is_unknown_not_empty() {
    let src = BASE
        .replace("a: Alpha = Alpha { };", "")
        .replace("a: cooperative(pool = write);", "")
        .replace("claims {", "run() { let extra = Alpha { }; } claims {");
    assert_verdict(&src, Verdict::Uncertified);
}

#[test]
fn known_collision_wins_over_additional_dynamic_births() {
    let src = BASE
        .replace("pool = read", "pool = write")
        .replace("claims {", "run() { let extra = Alpha { }; } claims {");
    assert_verdict(&src, Verdict::Violated);
}

#[test]
fn unrelated_dynamic_birth_does_not_poison_the_claim() {
    let src = BASE
        .replace("locus Alpha", "locus Other { } locus Alpha")
        .replace("claims {", "run() { let extra = Other { }; } claims {");
    assert_verdict(&src, Verdict::Holds);
}

#[test]
fn empty_group_cannot_hold_even_with_may_be_empty() {
    let src = BASE.replace(
        "group writers = { Alpha };",
        "group writers = { } may_be_empty;",
    );
    assert_verdict(&src, Verdict::Invalid);
}

#[test]
fn free_function_only_group_is_invalid() {
    let src = BASE.replace(
        "group writers = { Alpha };",
        "fn helper() { } group writers = { helper };",
    );
    assert_verdict(&src, Verdict::Invalid);
}

#[test]
fn undeployed_locus_is_invalid() {
    let src = BASE
        .replace("a: Alpha = Alpha { };", "")
        .replace("a: cooperative(pool = write);", "");
    let ds = assert_verdict(&src, Verdict::Invalid);
    assert!(
        ds[0].contains("Alpha") && ds[0].contains("no instance"),
        "{ds:?}"
    );
}

#[test]
fn unused_member_cannot_be_silently_omitted_from_a_populated_group() {
    let src = BASE
        .replace("locus Alpha", "locus Unused { } locus Alpha")
        .replace(
            "group writers = { Alpha };",
            "group writers = { Alpha, Unused };",
        );
    assert_verdict(&src, Verdict::Invalid);
}

#[test]
fn bad_member_refuses_a_group_even_if_the_resolved_subset_collides() {
    let src = BASE.replace("pool = read", "pool = write").replace(
        "group writers = { Alpha };",
        "group writers = { Alpha, Missing };",
    );
    assert_verdict(&src, Verdict::Invalid);
}

#[test]
fn unknown_group_is_invalid() {
    assert_verdict(&BASE.replace("all writers", "all absent"), Verdict::Invalid);
}

#[test]
fn constitution_adoption_runs_the_same_judgment() {
    let src = BASE.replace("main locus App", "constitution PlacementLaw { separate: require separate_domains(all writers, all readers); } main locus App")
        .replace("claims { separate: require separate_domains(all writers, all readers); }", "claims { adopt PlacementLaw; }");
    assert_verdict(&src, Verdict::Holds);
    assert_verdict(
        &src.replace("pool = read", "pool = write"),
        Verdict::Violated,
    );
}

#[test]
fn imported_group_identity_is_not_its_display_name() {
    let main = r#"
        import "./lib" as lib;
        locus Alpha { }
        group writers = { Alpha };
        group readers = { lib::Alpha };
        main locus App {
            params { a: Alpha = Alpha { }; b: lib::Alpha = lib::Alpha { }; }
            placement { a: cooperative(pool = write); b: cooperative(pool = read); }
            claims { separate: require separate_domains(all writers, all readers); }
        }
        fn main() { App { }; }
    "#;
    let library = "locus Alpha { } group writers = { Alpha };";
    let (verdict, ds) = judge_files(&[("main.hl", main), ("lib/lib.hl", library)]);
    assert_eq!(verdict, Verdict::Holds, "{ds:?}");
    let colliding = main.replace("pool = read", "pool = write");
    let (verdict, ds) = judge_files(&[("main.hl", &colliding), ("lib/lib.hl", library)]);
    assert_eq!(verdict, Verdict::Violated, "{ds:?}");
    assert!(ds[0].contains("readers"), "{ds:?}");
}

#[test]
fn imported_constitution_keeps_its_own_group_bindings() {
    let main = r#"
        import "./lib" as lib;
        locus Alpha { }
        group writers = { Alpha };
        group readers = { Alpha };
        main locus App {
            params { a: lib::Alpha = lib::Alpha { }; b: lib::Beta = lib::Beta { }; }
            placement { a: cooperative(pool = write); b: cooperative(pool = read); }
            claims { adopt PlacementLaw; }
        }
        fn main() { App { }; }
    "#;
    let library = r#"
        locus Alpha { }
        locus Beta { }
        group writers = { Alpha };
        group readers = { Beta };
        constitution PlacementLaw {
            separate: require separate_domains(all writers, all readers);
        }
    "#;
    let (verdict, ds) = judge_files(&[("main.hl", main), ("lib/lib.hl", library)]);
    assert_eq!(verdict, Verdict::Holds, "{ds:?}");
    let colliding = main.replace("pool = read", "pool = write");
    let (verdict, ds) = judge_files(&[("main.hl", &colliding), ("lib/lib.hl", library)]);
    assert_eq!(verdict, Verdict::Violated, "{ds:?}");
    assert!(ds[0].contains("lib::writers"), "{ds:?}");

    // The direct library tier keeps exactly the same seed-local refs,
    // even though the closing seed declares identically named groups.
    let direct_main = main.replace("claims { adopt PlacementLaw; }", "");
    let direct_library = library.replace("constitution PlacementLaw {", "claims {");
    let (verdict, ds) = judge_files(&[("main.hl", &direct_main), ("lib/lib.hl", &direct_library)]);
    assert_eq!(verdict, Verdict::Holds, "{ds:?}");
    let colliding = direct_main.replace("pool = read", "pool = write");
    let (verdict, ds) = judge_files(&[("main.hl", &colliding), ("lib/lib.hl", &direct_library)]);
    assert_eq!(verdict, Verdict::Violated, "{ds:?}");
    assert!(ds[0].contains("claim `lib::separate`"), "{ds:?}");
}

#[test]
fn both_separation_operands_participate_in_constitution_identity() {
    let src = BASE.replace("main locus App", "constitution PlacementLaw { separate: require separate_domains(all writers, all readers); } main locus App")
        .replace("claims { separate: require separate_domains(all writers, all readers); }", "claims { adopt PlacementLaw; }");
    let digest = |source: &str| {
        let snap = entries::load_files(&[("main.hl", source)], Config::check(true, false));
        let bundle = snap.bundle();
        let programs: Vec<_> = bundle.programs.values().copied().collect();
        let adoption = snap.demand_law_selection().unwrap().identities(&programs);
        assert_eq!(adoption.roots.len(), 1);
        adoption.roots[0].digest.clone()
    };
    let original = digest(&src);
    assert_eq!(original, digest(&src), "identity is deterministic");
    assert_ne!(
        original,
        digest(&src.replace("all writers, all readers", "all readers, all readers"))
    );
    assert_ne!(
        original,
        digest(&src.replace("all writers, all readers", "all writers, all writers"))
    );
}

fn mutate_model(change: impl FnOnce(&mut ApplicationModel, &ClaimIrTable)) -> Judged {
    let snap = entries::load_files(&[("main.hl", BASE)], Config::check(true, false));
    let bundle = snap.bundle();
    let mut model = snap.demand_model().unwrap().clone();
    let table = hale_types::claim_lowering::lower_claims_over(
        &bundle,
        &model,
        snap.demand_law_selection().unwrap(),
    );
    change(&mut model, &table);
    let bases: Vec<_> = bundle.sources.iter().map(|source| source.base).collect();
    judge_all(&table, &model, &EvidenceTable::default(), &bases)
        .1
        .into_values()
        .next()
        .unwrap()
}

#[test]
fn removing_a_placement_row_withdraws_the_proof() {
    let judged = mutate_model(|model, _| {
        let instance = model
            .entities
            .locus_instances
            .iter()
            .position(|i| i.path == "App.a")
            .unwrap();
        model
            .relations
            .placed_in
            .retain(|row| row.instance.index() != instance);
        assert_eq!(model.validate(), Ok(()));
    });
    assert_eq!(judged.verdict, Verdict::Uncertified, "{judged:?}");
    assert!(judged
        .diags
        .iter()
        .any(|d| d.message.contains("App.a") && d.message.contains("no modeled")));
}

#[test]
fn mutating_placement_changes_the_judgment_without_rewalking_source() {
    let judged = mutate_model(|model, _| {
        let instance = |path| {
            model
                .entities
                .locus_instances
                .iter()
                .position(|i| i.path == path)
                .unwrap()
        };
        let a = instance("App.a");
        let b = instance("App.b");
        let domain = model
            .relations
            .placed_in
            .iter()
            .find(|p| p.instance.index() == a)
            .unwrap()
            .domain;
        model
            .relations
            .placed_in
            .iter_mut()
            .find(|p| p.instance.index() == b)
            .unwrap()
            .domain = domain;
        assert_eq!(model.validate(), Ok(()));
    });
    assert_eq!(judged.verdict, Verdict::Violated, "{judged:?}");
}

#[test]
fn mutating_group_members_changes_the_selected_instance_population() {
    let judged = mutate_model(|model, table| {
        let hale_model::ClaimIr::RequireSeparateDomains { left, right } = &table.rows[0].law else {
            panic!()
        };
        let member = model
            .relations
            .group_members
            .iter()
            .find(|m| Some(m.group) == left.group)
            .unwrap()
            .member;
        model
            .relations
            .group_members
            .iter_mut()
            .find(|m| Some(m.group) == right.group)
            .unwrap()
            .member = member;
    });
    assert_eq!(judged.verdict, Verdict::Violated, "{judged:?}");
}

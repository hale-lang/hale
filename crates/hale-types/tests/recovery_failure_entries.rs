//! Conditional handler frames come from retained construction evidence, not
//! ownership alone. Runtime liveness, guard feasibility and admission remain
//! outside these facts.
#[path = "support/entries.rs"]
mod entries;
use hale_frontend::snapshot::Config;
use hale_model::recovery::{RecoveryConstructionSource as Source, RecoveryFailureDispatch as Dispatch, RecoveryFailureGap as Gap};
use hale_model::{EntityRef, FunctionId, LocusInstanceId, ModelError};
use hale_types::model_query::{bind_recovery_call, bind_recovery_operand, recovery_handler_invocation, RecoveryEntryGap};

const SOURCE: &str = r#"
locus Child {
    params { n: Int = 0; }
    closure broken { captures: n; epoch inline; }
    fn break_it() { self.n = 1; violate broken; }
}
fn recover(c: Child) { restart(c); }
fn relay(c: Child) { recover(c); }
locus Parent {
    params { a: Child = Child { }; b: Child = Child { }; }
    on_failure(c: Child, e: ClosureViolation) { relay(c); }
}
main locus App { params { p: Parent = Parent { }; } }
fn main() { App { }; }
"#;

fn load(src: &str) -> hale_frontend::snapshot::Snapshot {
    let s = entries::load_files(&[("app.hl", src)], Config::check(true, false));
    let checked = s.demand_check().unwrap();
    assert!(!checked.diags.iter().any(|d| d.is_error()), "{:?}", checked.diags);
    s
}
fn instance(m: &hale_model::ApplicationModel, path: &str) -> LocusInstanceId {
    LocusInstanceId(m.entities.locus_instances.iter().position(|i| i.path == path).unwrap() as u32)
}
fn function(m: &hale_model::ApplicationModel, name: &str) -> FunctionId {
    FunctionId(m.entities.functions.iter().position(|f| f.name == name).unwrap() as u32)
}
fn contexts(m: &hale_model::ApplicationModel, id: LocusInstanceId) -> &[hale_model::recovery::RecoveryFailureContext] {
    &m.analyses.recovery.failure_entries.as_ref().unwrap()[id.index()].contexts
}

#[test]
fn constructed_siblings_supply_distinct_child_arguments_through_helpers() {
    let s = load(SOURCE); let m = s.demand_model().unwrap(); m.validate().unwrap();
    s.demand_placement().unwrap().validate_construction_supervision().unwrap();
    let dump = hale_types::model_builder::render_internal(m);
    assert!(dump.contains("conditional failure entries (construction facts; not runtime admission)"));
    assert!(dump.contains("params initializer"));
    assert!(dump.contains("unknown: root context"));
    for path in ["App.p.a", "App.p.b"] {
        let failed = instance(m, path);
        let entry = recovery_handler_invocation(m, failed, 0).unwrap();
        assert_eq!(entry.invocation.receiver, Some(instance(m, "App.p")));
        assert_eq!(entry.invocation.arguments, vec![Some(failed), None]);
        assert_eq!(entry.construction_path.len(), 3);
        assert!(entry.construction_path.iter().all(Option::is_some));
        assert_eq!(contexts(m, failed)[0].construction, Some(Source::ParamsInitializer));
        let mut frame = entry.invocation;
        for name in ["relay", "recover"] {
            let callee = function(m, name);
            let edge = m.relations.calls.iter().position(|c| c.from == frame.function && c.to == callee).unwrap();
            let bound = bind_recovery_call(m, edge, &frame).unwrap();
            assert_eq!(bound.len(), 1); assert!(bound[0].gaps.is_empty());
            frame = bound[0].invocation.clone();
        }
        let action = m.analyses.recovery.user.as_ref().unwrap().iter().find(|r| r.body == Some(EntityRef::Function(frame.function))).unwrap();
        assert_eq!(bind_recovery_operand(m, &frame, &action.operand), Ok(failed));
    }
}

#[test]
fn overrides_and_defaults_keep_their_construction_sources_and_root_unknown() {
    let src = SOURCE.replace("p: Parent = Parent { }", "p: Parent = Parent { a: Child { } }");
    let s = load(&src); let m = s.demand_model().unwrap();
    assert_eq!(contexts(m, instance(m, "App.p.a"))[0].construction, Some(Source::FieldHolder));
    assert_eq!(contexts(m, instance(m, "App.p.b"))[0].construction, Some(Source::ParamsInitializer));
    assert_eq!(recovery_handler_invocation(m, instance(m, "App"), 0), Err(RecoveryEntryGap::Unknown(Gap::RootContext)));
    assert_eq!(recovery_handler_invocation(m, instance(m, "App.p.a"), 1), Err(RecoveryEntryGap::InvalidContext));
}

#[test]
fn agreed_paths_retain_every_root_construction_and_its_witness() {
    let src = SOURCE.replace("fn main() { App { }; }", "fn main() { App { }; App { p: Parent { a: Child { } } }; }");
    let s = load(&src); let m = s.demand_model().unwrap();
    let failed = instance(m, "App.p.a"); let rows = contexts(m, failed);
    assert_eq!(rows.len(), 2, "agreement on declaration/domain cannot discard a birth context");
    assert_ne!(rows[0].construction_path[0], rows[1].construction_path[0]);
    assert!(rows.iter().any(|r| r.construction == Some(Source::FieldHolder)));
    assert!(rows.iter().any(|r| r.construction == Some(Source::ParamsInitializer)));
    for context in 0..rows.len() {
        assert_eq!(recovery_handler_invocation(m, failed, context).unwrap().invocation.arguments[0], Some(failed));
    }
}

#[test]
fn guarded_alternatives_are_retained_without_a_feasibility_claim() {
    let src = SOURCE.replace("a: Child = Child { }", "flag: Bool = true; a: Child = if self.flag { Child { } } else { Child { } }");
    let s = load(&src); let m = s.demand_model().unwrap();
    let failed = instance(m, "App.p.a"); let rows = contexts(m, failed);
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|r| r.guarded));
    assert_ne!(rows[0].construction_path.last(), rows[1].construction_path.last());
    for context in 0..2 { assert!(recovery_handler_invocation(m, failed, context).unwrap().guarded); }
}

#[test]
fn held_values_and_unconstructed_catalogs_do_not_invent_supervisors() {
    let held = SOURCE.replace("fn main() { App { }; }", "fn main() { let held = Child { }; App { p: Parent { a: held } }; }");
    let s = load(&held); let m = s.demand_model().unwrap();
    let failed = instance(m, "App.p.a"); let parent = instance(m, "App.p");
    assert!(m.relations.owns.iter().any(|r| r.parent == parent && r.child == failed));
    assert_eq!(recovery_handler_invocation(m, failed, 0), Err(RecoveryEntryGap::Unknown(Gap::HeldValue)));
    let s = load(&SOURCE.replace("fn main() { App { }; }", "fn main() { }"));
    let m = s.demand_model().unwrap();
    assert_eq!(recovery_handler_invocation(m, instance(m, "App.p.a"), 0), Err(RecoveryEntryGap::Unknown(Gap::MissingConstruction)));
}

#[test]
fn unhandled_and_unspecialized_routes_are_different_outcomes() {
    let s = load(&SOURCE.replace("    on_failure(c: Child, e: ClosureViolation) { relay(c); }", ""));
    let m = s.demand_model().unwrap();
    assert_eq!(recovery_handler_invocation(m, instance(m, "App.p.a"), 0), Err(RecoveryEntryGap::Unhandled));
    let src = SOURCE.replace("locus Parent {", "locus Parent<T> {")
        .replace("params { a: Child", "params { value: T; a: Child")
        .replace("p: Parent = Parent { }", "p: Parent<Int>")
        .replace("fn main() { App { }; }", "fn main() { let parent: Parent<Int> = Parent { value: 1 }; App { p: parent }; }");
    let s = load(&src); let m = s.demand_model().unwrap();
    assert_eq!(recovery_handler_invocation(m, instance(m, "App.p.a"), 0), Err(RecoveryEntryGap::Unknown(Gap::GenericSpecialization)));
}

#[test]
fn malformed_failure_entries_are_refused_by_model_validation() {
    let s = load(SOURCE); let base = s.demand_model().unwrap();
    let failed = instance(base, "App.p.a");
    for mutation in 0..9 {
        let mut m = base.clone(); let recover = function(&m, "recover");
        let row = &mut m.analyses.recovery.failure_entries.as_mut().unwrap()[failed.index()];
        match mutation {
            0 => row.failed = LocusInstanceId(u32::MAX),
            1 => row.contexts.clear(),
            2 => row.contexts[0].construction = None,
            3 => row.contexts[0].provenance = hale_model::ProvenanceId(u32::MAX),
            4 => if let Dispatch::Handled { ref mut handler, .. } = row.contexts[0].dispatch { *handler = recover; },
            5 => if let Dispatch::Handled { ref mut supervision, .. } = row.contexts[0].dispatch { *supervision = u32::MAX; },
            6 => if let Dispatch::Handled { supervisor, .. } = row.contexts[0].dispatch { row.contexts[0].dispatch = Dispatch::Unhandled { supervisor }; },
            7 => { row.contexts[0].construction_path.pop(); },
            _ => row.contexts[0].construction_path[0] = Some(hale_model::ProvenanceId(u32::MAX)),
        }
        assert!(matches!(m.validate(), Err(ModelError::InvalidRecoveryFact { .. })), "mutation {mutation}");
    }
}

#[test]
fn missing_inventories_and_display_labels_cannot_change_binding_identity() {
    let s = load(SOURCE); let base = s.demand_model().unwrap(); let failed = instance(base, "App.p.a");
    let entry = recovery_handler_invocation(base, failed, 0).unwrap();
    let mut m = base.clone(); m.analyses.recovery.failure_entries = None;
    m.validate().unwrap();
    assert_eq!(recovery_handler_invocation(&m, failed, 0), Err(RecoveryEntryGap::Unavailable));
    let mut m = base.clone();
    for f in &mut m.entities.functions { f.display.push_str(" [witness label]"); }
    assert_eq!(recovery_handler_invocation(&m, failed, 0).unwrap().invocation, entry.invocation);
}

#[test]
fn moving_a_parent_retains_its_childrens_original_supervising_instance() {
    let src = SOURCE.replace("p: Parent = Parent { }", "p: Parent")
        .replace("fn main() { App { }; }", "fn main() { let parent = Parent { }; App { p: parent }; }");
    let s = load(&src); let m = s.demand_model().unwrap();
    for path in ["App.p.a", "App.p.b"] {
        let entry = recovery_handler_invocation(m, instance(m, path), 0).unwrap();
        assert_eq!(entry.invocation.receiver, Some(instance(m, "App.p")));
        assert_eq!(entry.invocation.arguments[0], Some(instance(m, path)));
        assert_eq!(entry.construction_path.len(), 2, "the child was built with Parent before App");
    }
    assert_eq!(recovery_handler_invocation(m, instance(m, "App.p"), 0), Err(RecoveryEntryGap::Unknown(Gap::HeldValue)),
        "holding Parent does not give Parent a new supervisor");
}

#[test]
fn multiple_logical_aliases_of_one_construction_are_not_distinct_actual_objects() {
    let src = SOURCE.replace("p: Parent = Parent { }", "p: Parent; q: Parent")
        .replace("fn main() { App { }; }", "fn main() { let parent = Parent { }; App { p: parent, q: parent }; }");
    let s = load(&src); let m = s.demand_model().unwrap();
    for path in ["App.p.a", "App.q.a"] {
        assert_eq!(recovery_handler_invocation(m, instance(m, path), 0), Err(RecoveryEntryGap::Unknown(Gap::AmbiguousInstance)));
    }
}

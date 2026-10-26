//! Recovery operands are typed while lexical bindings are live, then
//! carried by the canonical model without becoming instance identities.

use hale_frontend::snapshot::{Config, Snapshot};
use hale_model::recovery::{RecoveryBound, RecoveryOperation, RecoveryReceiverType, RecoveryRoot, RecoverySpecialization};
use hale_model::{EntityRef, FunctionId, ModelError, Provenance, ProvenanceId};
use hale_types::model_query::recovery_observations;

const SOURCE: &str = r#"
locus Worker { params { n: Int = 0; } }
locus Other { params { n: Int = 0; } }
locus Keeper { params { peer: Other = Other { }; } }
locus Generic<T> {
    params { peer: T; }
    on_failure(failed: Worker, err: ClosureViolation) { restart(self.peer) for 2; }
}
main locus App {
    params { worker: Worker = Worker { }; peer: Other = Other { }; keeper: Keeper = Keeper { }; limit: Int = 3; }
    on_failure(failed: Worker, err: ClosureViolation) {
        restart(failed) for 3;
        restart_in_place(self.peer) for self.limit + 1;
        { let failed = self.peer; quarantine(failed); }
        quarantine(failed);
        let old = self.peer;
        self.peer = Other { };
        restart(old);
        restart(self.keeper.peer);
        bubble(err);
    }
    fn mk() { let g: Generic<Other> = Generic { peer: Other { } }; }
}
fn helper(value: Other) { restart(value); }
fn main() { App { }; }
"#;

fn snapshot() -> Snapshot {
    Snapshot::from_program(hale_syntax::parse_source(SOURCE).expect("parses"), Vec::new(), Config::check(true, false))
        .unwrap_or_else(|_| panic!("snapshot"))
}

#[test]
fn model_keeps_parameter_bindings_field_paths_shadowing_aliases_and_contexts() {
    let s = snapshot();
    let checked = s.demand_check().unwrap_or_else(|_| panic!("check"));
    assert!(!checked.diags.iter().any(|d| d.is_error()), "{:?}", checked.diags);
    let m = s.demand_model().unwrap_or_else(|_| panic!("model"));
    m.validate().unwrap();
    let handler = m.entities.functions.iter().position(|f| f.name.starts_with("App::on_failure(")).expect("handler");
    let body = EntityRef::Function(FunctionId(handler as u32));
    let observations = recovery_observations(m, body);
    assert!(observations.user_inventory_available && observations.stdlib_inventory_available);
    assert!(observations.unanchored.is_empty());
    let rows = observations.actions;
    assert_eq!(rows.len(), 7);
    assert_eq!(rows[0].operand.root, RecoveryRoot::Parameter { body, ordinal: 0, arity: 2 });
    assert_eq!(rows[0].bound, RecoveryBound::Literal(3));
    assert!(matches!(rows[1].operand.root, RecoveryRoot::CurrentLocus { .. }));
    assert_eq!(rows[1].operand.fields, ["peer"]);
    let RecoveryBound::Expression { provenance } = rows[1].bound else { panic!("dynamic bound") };
    assert!(matches!(m.provenance.records[provenance.index()], Provenance::ForeignSpan { .. } | Provenance::Source { .. }));
    assert_eq!(rows[2].operand.root, RecoveryRoot::Unresolved, "a local shadows the handler parameter");
    assert_eq!(rows[3].operand.root, RecoveryRoot::Parameter { body, ordinal: 0, arity: 2 }, "leaving the block restores the parameter binding");
    assert_eq!(rows[4].operand.root, RecoveryRoot::Unresolved, "a saved alias is not a live field path after replacement");
    assert!(rows[4].operand.fields.is_empty());
    assert_eq!(rows[5].operand.fields, ["keeper", "peer"]);
    assert_eq!(rows[6].operation, RecoveryOperation::Bubble);
    assert_eq!(rows[6].operand.root, RecoveryRoot::Parameter { body, ordinal: 1, arity: 2 });
    assert!(matches!(&rows[6].receiver_type, RecoveryReceiverType::OtherNamed { name } if name == "ClosureViolation"));
    let generic: Vec<_> = m.analyses.recovery.user.as_ref().unwrap().iter()
        .filter(|row| matches!(&row.specialization, RecoverySpecialization::Concrete { symbol } if symbol == "Generic_Other")).collect();
    assert_eq!(generic.len(), 1);
    assert!(matches!(&generic[0].receiver_type, RecoveryReceiverType::Locus { name, .. } if name == "Other"));
    let helper = m.entities.functions.iter().position(|f| f.name == "helper").expect("helper");
    let helper_body = EntityRef::Function(FunctionId(helper as u32));
    let rows = recovery_observations(m, helper_body).actions;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].operand.root, RecoveryRoot::Parameter { body: helper_body, ordinal: 0, arity: 1 });
    let text = hale_types::model_builder::render_internal(m);
    assert!(text.contains("recovery observations (symbolic; instance targets unresolved)"));
    assert!(text.contains("Generic_Other"));
    assert!(text.contains("unresolved value"));
    assert!(text.contains("for runtime expression"));
}

#[test]
fn unavailable_and_unanchored_observations_do_not_become_empty_closed_scopes() {
    let s = snapshot();
    let mut m = s.demand_model().unwrap_or_else(|_| panic!("model")).clone();
    let body = m.analyses.recovery.user.as_ref().unwrap().iter().find_map(|row| row.body).unwrap();
    m.analyses.recovery.user.as_mut().unwrap()[0].body = None;
    m.validate().unwrap();
    assert_eq!(recovery_observations(&m, body).unanchored.len(), 1);
    m.analyses.recovery.user = None;
    let result = recovery_observations(&m, body);
    assert!(!result.user_inventory_available);
    assert!(result.actions.is_empty());
}

#[test]
fn invalid_recovery_references_shapes_and_foreign_aliases_are_refused() {
    let s = snapshot();
    let base = s.demand_model().unwrap_or_else(|_| panic!("model")).clone();
    for mutation in 0..6 {
        let mut m = base.clone();
        let row = &mut m.analyses.recovery.user.as_mut().unwrap()[0];
        match mutation {
            0 => row.body = Some(EntityRef::Function(FunctionId(u32::MAX))),
            1 => row.operand.root = RecoveryRoot::Parameter { body: row.body.unwrap(), ordinal: 2, arity: 2 },
            2 => row.operand.fields.push(String::new()),
            3 => row.bound = RecoveryBound::Expression { provenance: ProvenanceId(u32::MAX) },
            4 => row.specialization = RecoverySpecialization::Concrete { symbol: String::new() },
            5 => { row.operation = RecoveryOperation::Quarantine; row.bound = RecoveryBound::Literal(2); }
            _ => unreachable!(),
        }
        assert!(matches!(m.validate(), Err(ModelError::InvalidRecoveryFact { .. })), "mutation {mutation}");
    }
    // A stdlib inventory cannot smuggle a user's entity/provenance ids into
    // its own namespace, even when the numeric offsets overlap exactly.
    let mut m = base.clone();
    m.analyses.recovery.stdlib = Some(vec![m.analyses.recovery.user.as_ref().unwrap()[0].clone()]);
    assert!(matches!(m.validate(), Err(ModelError::InvalidRecoveryFact { .. })));
}

#[test]
fn separate_default_evaluations_keep_call_witnesses_and_unresolved_contexts() {
    let source = SOURCE.replace("    fn mk()", "    fn again(k: Int = if true { restart(Other { }); 0 } else { 0 }) { }\n    run() { self.again(); self.again(); }\n    fn mk()");
    let s = Snapshot::from_program(hale_syntax::parse_source(&source).expect("parses"), Vec::new(), Config::check(true, false))
        .unwrap_or_else(|_| panic!("snapshot"));
    let checked = s.demand_check().unwrap_or_else(|_| panic!("check"));
    assert!(!checked.diags.iter().any(|d| d.is_error()), "{:?}", checked.diags);
    let m = s.demand_model().unwrap_or_else(|_| panic!("model"));
    let evaluated: Vec<_> = m.analyses.recovery.user.as_ref().unwrap().iter()
        .filter(|row| !row.evaluation.is_empty()).collect();
    assert_eq!(evaluated.len(), 2, "one default statement, two invocation paths");
    assert_ne!(evaluated[0].evaluation, evaluated[1].evaluation);
    for row in evaluated {
        assert_eq!(row.evaluation.len(), 1);
        assert!(row.evaluation[0].is_some());
        assert!(row.body.is_none());
        assert_eq!(row.operand.root, RecoveryRoot::Unresolved);
        assert!(matches!(m.provenance.records[row.provenance.index()], Provenance::ForeignSpan { .. }));
    }
    m.validate().unwrap();

    let nested = SOURCE.replace("    fn mk()", "    run() { outer(); outer(); }\n    fn mk()")
        .replace("fn main()", "fn inner(k: Int = if true { restart(Other { }); 0 } else { 0 }) { }\nfn outer(k: Int = if true { inner(); 0 } else { 0 }) { }\nfn main()");
    let s = Snapshot::from_program(hale_syntax::parse_source(&nested).expect("nested defaults parse"), Vec::new(), Config::check(true, false))
        .unwrap_or_else(|_| panic!("snapshot"));
    let m = s.demand_model().unwrap_or_else(|_| panic!("nested default model"));
    let rows: Vec<_> = m.analyses.recovery.user.as_ref().unwrap().iter().filter(|r| r.evaluation.len() == 2).collect();
    assert_eq!(rows.len(), 2);
    for row in rows {
        assert!(row.evaluation[0].is_some());
        assert!(row.evaluation[1].is_none(), "a nested bare id cannot qualify its source universe");
    }
}

#[test]
fn stdlib_projection_never_joins_overlapping_user_sites_or_source_offsets() {
    use hale_frontend::source::Disk;
    use hale_frontend::frontend::LoadMode;
    use hale_types::placement::SiteUniverse;
    let dir = std::env::temp_dir().join(format!("hale-recovery-facts-{}-foreign", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("app.hl"), SOURCE).unwrap();
    let s = Snapshot::load(&dir, LoadMode::WholeSeed, &Disk, Config::check(true, false))
        .unwrap_or_else(|_| panic!("loaded snapshot"));
    let typed = s.demand_typed_bodies().unwrap_or_else(|_| panic!("typed bodies"));
    let mut foreign = typed.recoveries().find(|(_, r)| r.parent_name.as_deref() == Some("App") && r.child.is_some())
        .expect("a user recovery").1.clone();
    foreign.parent.as_mut().unwrap().universe = SiteUniverse::StdlibAnalysis;
    foreign.child.as_mut().unwrap().universe = SiteUniverse::StdlibAnalysis;
    let foreign_rows = [foreign];
    let inputs = hale_types::model_builder::ModelInputs {
        lifecycle_contexts: s.demand_lifecycle_contexts().unwrap_or_else(|_| panic!("source contexts")),
        top: s.demand_scope().unwrap(),
        bus_graph: s.demand_bus_graph().unwrap(),
        ownership: s.demand_ownership_graph().unwrap(),
        handlers: s.demand_handlers().unwrap(),
        typed,
        stdlib_recoveries: Some(&foreign_rows),
        flows: s.demand_flow_instances().unwrap(),
        effects: s.demand_effects().unwrap(),
        forms: s.demand_forms().unwrap(),
        bindings: s.demand_bindings().unwrap(),
        placement: s.demand_placement().unwrap(),
        arrangement: s.demand_arrangement().unwrap(),
        dispatch_plan: s.demand_dispatch_plan().unwrap(),
        surfaces: s.demand_surface_rows().unwrap(),
    };
    let m = hale_types::model_builder::derive_application_model_over(&s.bundle(), &inputs);
    m.validate().unwrap();
    let foreign = &m.analyses.recovery.stdlib.as_ref().unwrap()[0];
    assert!(foreign.body.is_none());
    assert_eq!(foreign.operand.root, RecoveryRoot::Unresolved);
    assert!(matches!(&foreign.receiver_type, RecoveryReceiverType::ExternalLocus { name } if name == "Worker"));
    assert!(matches!(m.provenance.records[foreign.provenance.index()], Provenance::ForeignSpan { .. }));
    let source = m.analyses.recovery.user.as_ref().unwrap().iter().find(|row| {
        matches!(m.provenance.records[row.provenance.index()], Provenance::Source { .. })
    }).expect("the user recovery is source-backed").provenance;
    let mut invalid = m.clone();
    invalid.analyses.recovery.stdlib.as_mut().unwrap()[0].provenance = source;
    assert!(matches!(invalid.validate(), Err(ModelError::InvalidRecoveryFact { .. })));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn generated_lifecycle_witnesses_do_not_borrow_overlapping_user_source() {
    use hale_frontend::frontend::LoadMode;
    use hale_frontend::source::Disk;
    use hale_types::placement::SiteUniverse;
    let dir = std::env::temp_dir().join(format!("hale-generated-lifecycle-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("app.hl"), SOURCE).unwrap();
    let s = Snapshot::load(&dir, LoadMode::WholeSeed, &Disk, Config::check(true,false))
        .unwrap_or_else(|_| panic!("source snapshot"));
    let original = s.demand_model().unwrap_or_else(|_| panic!("source model"));
    let context = original.analyses.lifecycle_contexts.as_ref().unwrap().contexts.iter()
        .find(|c| c.construction_site.is_some_and(|site| site.universe == SiteUniverse::User))
        .expect("user constructor");
    assert!(matches!(original.provenance.records[context.provenance.index()],Provenance::Source { .. }));
    let site = context.construction_site.unwrap();
    let inputs = hale_types::model_builder::ModelInputs {
        lifecycle_contexts: s.demand_lifecycle_contexts().unwrap(),
        top: s.demand_scope().unwrap(),
        bus_graph: s.demand_bus_graph().unwrap(),
        ownership: s.demand_ownership_graph().unwrap(),
        handlers: s.demand_handlers().unwrap(),
        typed: s.demand_typed_bodies().unwrap(),
        stdlib_recoveries: hale_types::stdlib_bodies::recovery_rows(),
        flows: s.demand_flow_instances().unwrap(),
        effects: s.demand_effects().unwrap(),
        forms: s.demand_forms().unwrap(),
        bindings: s.demand_bindings().unwrap(),
        placement: s.demand_placement().unwrap(),
        arrangement: s.demand_arrangement().unwrap(),
        dispatch_plan: s.demand_dispatch_plan().unwrap(),
        surfaces: s.demand_surface_rows().unwrap(),
    };
    for (origin,expected) in [
        (hale_types::snapshot::Origin::JsonParsers,"lifecycle source: generated JSON parser"),
        (hale_types::snapshot::Origin::ApiSurface,"lifecycle source: generated API surface"),
    ] {
        let mut bundle = s.bundle();
        bundle.snapshot.origins.retain(|(id,_)| *id != site.id);
        bundle.snapshot.origins.push((site.id,origin));
        bundle.snapshot.origins.sort_by_key(|(id,_)| id.index);
        let model = hale_types::model_builder::derive_application_model_over(&bundle,&inputs);
        model.validate().unwrap();
        assert_eq!(hale_types::topology_projection::project_model_half(&model),
            hale_types::topology_projection::project_model_half(original),
            "generation provenance changes only the internal context inventory");
        let projected = model.analyses.lifecycle_contexts.as_ref().unwrap().contexts.iter()
            .find(|c| c.construction_site == Some(site)).unwrap();
        assert!(matches!(&model.provenance.records[projected.provenance.index()],Provenance::Synthetic { origin } if origin == expected));
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

//! Recovery operands bind through canonical call sites to supplied invocation
//! instances. Dispatch and field state remain separate proof obligations.
#[path = "support/entries.rs"]
mod entries;
use hale_frontend::snapshot::{Config, Snapshot};
use hale_model::{EntityRef, FunctionId, FunctionKind, LocusInstanceId};
use hale_types::model_query::{bind_recovery_call, bind_recovery_operand, RecoveryBindingGap, RecoveryInvocation};

const SOURCE: &str = r#"
locus Child { }
fn recover(c: Child) { restart(c); }
fn relay(x: Child) { recover(x); }
locus Parent {
    params { a: Child = Child { }; b: Child = Child { }; }
    on_failure(c: Child, e: ClosureViolation) { relay(c); }
}
main locus App { params { p: Parent = Parent { }; } }
fn main() { App { }; }
"#;

fn snapshot(src: &str) -> Snapshot {
    entries::load_files(&[("app.hl", src)], Config::check(true, false))
}
fn function(m: &hale_model::ApplicationModel, name: &str) -> FunctionId {
    FunctionId(m.entities.functions.iter().position(|f| f.name == name).unwrap() as u32)
}
fn instance(m: &hale_model::ApplicationModel, path: &str) -> LocusInstanceId {
    LocusInstanceId(m.entities.locus_instances.iter().position(|i| i.path == path).unwrap() as u32)
}
fn handler(m: &hale_model::ApplicationModel) -> FunctionId {
    FunctionId(m.entities.functions.iter().position(|f| f.kind == FunctionKind::FailureHandler).unwrap() as u32)
}
fn call(m: &hale_model::ApplicationModel, from: FunctionId, to: FunctionId) -> usize {
    m.relations.calls.iter().position(|c| c.from == from && c.to == to).unwrap()
}
fn premise(m: &hale_model::ApplicationModel, child: &str) -> RecoveryInvocation {
    RecoveryInvocation { function: handler(m), receiver: Some(instance(m, "App.p")), arguments: vec![Some(instance(m, child)), None] }
}

#[test]
fn helper_arguments_preserve_distinct_instances_of_the_same_declaration() {
    let s = snapshot(SOURCE);
    let checked = s.demand_check().unwrap();
    assert!(!checked.diags.iter().any(|d| d.is_error()), "{:?}", checked.diags);
    let m = s.demand_model().unwrap(); m.validate().unwrap();
    let (h, relay, recover) = (handler(m), function(m, "relay"), function(m, "recover"));
    let action = m.analyses.recovery.user.as_ref().unwrap().iter().find(|r| r.body == Some(EntityRef::Function(recover))).unwrap();
    for path in ["App.p.a", "App.p.b"] {
        let first = bind_recovery_call(m, call(m, h, relay), &premise(m, path)).unwrap();
        assert_eq!(first.len(), 1); assert!(first[0].gaps.is_empty());
        let second = bind_recovery_call(m, call(m, relay, recover), &first[0].invocation).unwrap();
        assert_eq!(second.len(), 1); assert!(second[0].gaps.is_empty());
        assert_eq!(bind_recovery_operand(m, &second[0].invocation, &action.operand), Ok(instance(m, path)));
    }
}

#[test]
fn fields_and_saved_aliases_require_state_instead_of_arrangement_path_matching() {
    for (src, expected) in [
        (SOURCE.replace("relay(c);", "relay(self.b);"), RecoveryBindingGap::FieldState),
        (SOURCE.replace("relay(c);", "let saved = self.b; relay(saved);"), RecoveryBindingGap::UnknownOperand),
        (SOURCE.replace("relay(c);", "let c = Child { }; relay(c);"), RecoveryBindingGap::UnknownOperand),
    ] {
        let s = snapshot(&src); let m = s.demand_model().unwrap(); m.validate().unwrap();
        let bound = bind_recovery_call(m, call(m, handler(m), function(m, "relay")), &premise(m, "App.p.a")).unwrap();
        assert_eq!(bound[0].invocation.arguments, vec![None]);
        assert!(bound[0].gaps.iter().any(|g| g.reason == expected));
    }
}

#[test]
fn a_method_receiver_uses_the_actual_argument_instance() {
    let src = SOURCE.replace("locus Child { }", "locus Child { fn fix() { restart(self); } }")
        .replace("relay(c);", "c.fix();");
    let s = snapshot(&src); let m = s.demand_model().unwrap(); m.validate().unwrap();
    let fix = function(m, "Child::fix");
    let bound = bind_recovery_call(m, call(m, handler(m), fix), &premise(m, "App.p.b")).unwrap();
    assert!(bound[0].gaps.is_empty());
    let action = m.analyses.recovery.user.as_ref().unwrap().iter().find(|r| r.body == Some(EntityRef::Function(fix))).unwrap();
    assert_eq!(bind_recovery_operand(m, &bound[0].invocation, &action.operand), Ok(instance(m, "App.p.b")));
}

#[test]
fn omitted_defaults_missing_bindings_and_bad_premises_are_explicit() {
    let src = SOURCE.replace("fn relay(x: Child) { recover(x); }", "fn relay(x: Child, y: Child = Child { }) { recover(y); }");
    let s = snapshot(&src); let base = s.demand_model().unwrap();
    let edge = call(base, handler(base), function(base, "relay"));
    let entry = premise(base, "App.p.a");
    let bound = bind_recovery_call(base, edge, &entry).unwrap();
    assert_eq!(bound[0].invocation.arguments, vec![Some(instance(base, "App.p.a")), None]);
    assert!(bound[0].gaps.iter().any(|g| g.reason == RecoveryBindingGap::OmittedArgument && g.input == hale_types::model_query::RecoveryBindingInput::Argument(1)));
    let mut missing = base.clone(); missing.analyses.recovery.calls = None;
    assert_eq!(bind_recovery_call(&missing, edge, &entry), Err(RecoveryBindingGap::Unavailable));
    let mut missing = base.clone(); missing.analyses.recovery.calls.as_mut().unwrap()[edge].contexts.clear();
    missing.validate().unwrap();
    assert_eq!(bind_recovery_call(&missing, edge, &entry), Err(RecoveryBindingGap::Unavailable));
    let mut invalid = entry; invalid.function = FunctionId(u32::MAX);
    assert_eq!(bind_recovery_call(base, edge, &invalid), Err(RecoveryBindingGap::InvalidInvocation));
}

#[test]
fn forged_call_correlations_and_operand_roots_fail_model_validation() {
    let s = snapshot(SOURCE); let base = s.demand_model().unwrap();
    let edge = call(base, handler(base), function(base, "relay"));
    for variant in 0..4 {
        let mut m = base.clone(); let row = &mut m.analyses.recovery.calls.as_mut().unwrap()[edge];
        match variant {
            0 => row.site += 1,
            1 => row.contexts[0].parameter_arity = 0,
            2 => row.contexts[0].arguments[0].fields.push(String::new()),
            _ => row.contexts[0].arguments[0].root = hale_model::recovery::RecoveryRoot::Parameter { body: EntityRef::Function(row.to), ordinal: 0, arity: 2 },
        }
        assert!(matches!(m.validate(), Err(hale_model::ModelError::InvalidRecoveryFact { .. })));
    }
}

#[test]
fn evaluated_struct_defaults_keep_recoveries_after_temporary_typing_is_undone() {
    let src = SOURCE.replace("locus Parent", "type Settings { n: Int = if true { restart(Child { }); 0 } else { 0 }; }\nlocus Parent")
        .replace("main locus App {", "main locus App { run() { let x = Settings { }; let y = Settings { }; }");
    let s = snapshot(&src); let check = s.demand_check().unwrap();
    assert!(!check.diags.iter().any(|d| d.is_error()), "{:?}", check.diags);
    let m = s.demand_model().unwrap(); m.validate().unwrap();
    let rows: Vec<_> = m.analyses.recovery.user.as_ref().unwrap().iter().filter(|r| !r.evaluation.is_empty()).collect();
    assert_eq!(rows.len(), 2, "each literal evaluates the same default separately");
    assert_ne!(rows[0].evaluation, rows[1].evaluation);
    assert!(rows.iter().all(|r| r.body.is_none()));
    assert!(m.holes.iter().any(|h| h.kind == hale_model::HoleKind::UnmodeledRecovery && h.at == EntityRef::Function(function(m, "App::run"))));
    assert!(rows.iter().all(|r| r.execution_caller == Some(function(m, "App::run"))));
    let src = src.replace("fn main()", "@budget(publish = 0) fn measure() { let x = Settings { }; }\nfn main()");
    let s = snapshot(&src); let m = s.demand_model().unwrap(); let bundle = s.bundle();
    let table = hale_types::claim_lowering::lower_claims(&bundle, m);
    let evidence = hale_types::evidence::derive_certificate_evidence_over(&bundle, &table, m,
        s.demand_effect_certificates().unwrap(), s.demand_alloc_summary().unwrap());
    let rows = hale_types::judgment::judge_certificates(&table, m, &evidence, &[0]);
    let budget = table.rows.iter().find(|r| matches!(r.law, hale_model::ClaimIr::QuantBudget { .. })).unwrap();
    assert_eq!(rows.iter().find(|r| r.ordinal == budget.ordinal).unwrap().verdict, hale_types::verdict::Verdict::Uncertified,
        "a recovery hidden in a struct default cannot certify a zero publish ceiling");
    let helper_src = src.replace("restart(Child { });", "relay(Child { });");
    let s = snapshot(&helper_src); let m = s.demand_model().unwrap(); let bundle = s.bundle();
    let table = hale_types::claim_lowering::lower_claims(&bundle, m);
    let evidence = hale_types::evidence::derive_certificate_evidence_over(&bundle, &table, m,
        s.demand_effect_certificates().unwrap(), s.demand_alloc_summary().unwrap());
    let rows = hale_types::judgment::judge_certificates(&table, m, &evidence, &[0]);
    let budget = table.rows.iter().find(|r| matches!(r.law, hale_model::ClaimIr::QuantBudget { .. })).unwrap();
    assert_eq!(rows.iter().find(|r| r.ordinal == budget.ordinal).unwrap().verdict, hale_types::verdict::Verdict::Uncertified,
        "a default's helper call must retain its downstream recovery gap");
    let mut foreign = m.clone();
    let witness = hale_model::ProvenanceId(foreign.provenance.records.len() as u32);
    foreign.provenance.records.push(hale_model::Provenance::ForeignSpan { span: (0, 1) });
    let observed = foreign.analyses.recovery.evaluated_calls.as_mut().unwrap().iter_mut().find(|c| c.caller.is_some()).unwrap();
    observed.evaluation[0] = Some(witness); observed.provenance = witness;
    foreign.validate().unwrap(); // Unheld text does not change the typed caller identity.
    foreign.analyses.recovery.evaluated_calls.as_mut().unwrap()[0].caller = Some(FunctionId(u32::MAX));
    assert!(matches!(foreign.validate(), Err(hale_model::ModelError::InvalidRecoveryFact { .. })),
        "a caller reference must resolve independently of the source span's display kind");

    // Permit the known constructor effects to isolate the recovery gap.
    // Denying the default's explicit spawn still has to be a violation.
    for (effect_src, expected) in [
        (helper_src.replace("@budget(publish = 0)", "@no_syscall"), hale_types::verdict::Verdict::Uncertified),
        (helper_src.replace("main locus App", "@phase_effects(run: {alloc, spawn}) main locus App"), hale_types::verdict::Verdict::Uncertified),
        (helper_src.replace("main locus App", "@phase_effects(run: {alloc}) main locus App"), hale_types::verdict::Verdict::Violated),
    ] {
        let s = snapshot(&effect_src); let m = s.demand_model().unwrap(); let bundle = s.bundle();
        let table = hale_types::claim_lowering::lower_claims(&bundle, m);
        let evidence = hale_types::evidence::derive_certificate_evidence_over(&bundle, &table, m,
            s.demand_effect_certificates().unwrap(), s.demand_alloc_summary().unwrap());
        let rows = hale_types::judgment::judge_certificates(&table, m, &evidence, &[0]);
        let effect = table.rows.iter().find(|r| matches!(r.law, hale_model::ClaimIr::EffectForbid { .. } | hale_model::ClaimIr::PhaseEffects { .. })).unwrap();
        assert_eq!(rows.iter().find(|r| r.ordinal == effect.ordinal).unwrap().verdict, expected,
            "unknown recovery cannot erase a known spawn violation, or certify an otherwise permitted effect set");
    }

    let pure_src = helper_src.replace("fn main()", "@budget(publish = 0) fn unrelated() { }\nfn main()");
    let s = snapshot(&pure_src); let m = s.demand_model().unwrap(); let bundle = s.bundle();
    let table = hale_types::claim_lowering::lower_claims(&bundle, m);
    let evidence = hale_types::evidence::derive_certificate_evidence_over(&bundle, &table, m,
        s.demand_effect_certificates().unwrap(), s.demand_alloc_summary().unwrap());
    let rows = hale_types::judgment::judge_certificates(&table, m, &evidence, &[0]);
    let pure = function(m, "unrelated");
    let ordinal = table.rows.iter().find(|r| matches!(&r.law, hale_model::ClaimIr::QuantBudget { at: (Some(f), _), .. } if *f == pure)).unwrap().ordinal;
    assert_eq!(rows.iter().find(|r| r.ordinal == ordinal).unwrap().verdict, hale_types::verdict::Verdict::Holds,
        "a bound default evaluation must not poison an unrelated pure function");
    let quiet_src = helper_src.replace("relay(Child { });", "quiet(Child { });")
        .replace("fn main()", "fn quiet(c: Child) { }\nfn main()");
    let s = snapshot(&quiet_src); let m = s.demand_model().unwrap(); let bundle = s.bundle();
    let table = hale_types::claim_lowering::lower_claims(&bundle, m);
    let evidence = hale_types::evidence::derive_certificate_evidence_over(&bundle, &table, m,
        s.demand_effect_certificates().unwrap(), s.demand_alloc_summary().unwrap());
    let rows = hale_types::judgment::judge_certificates(&table, m, &evidence, &[0]);
    let budget = table.rows.iter().find(|r| matches!(r.law, hale_model::ClaimIr::QuantBudget { .. })).unwrap();
    assert_eq!(rows.iter().find(|r| r.ordinal == budget.ordinal).unwrap().verdict, hale_types::verdict::Verdict::Holds,
        "a resolved pure default helper must not be classified as recovery");




}

#[test]
fn a_recovery_free_walk_does_not_capture_unused_operand_rows() {
    let s = snapshot(&SOURCE.replace("restart(c);", ""));
    assert!(s.demand_typed_bodies().unwrap().recovery_calls().is_none());
    assert!(s.demand_model().unwrap().analyses.recovery.calls.is_none());
}

#[test]
fn immutable_locus_aliases_preserve_values_while_receiver_aliases_can_target_the_parent() {
    let src = SOURCE.replace("relay(c);", "let first = c; let saved = first; relay(saved);");
    let s = snapshot(&src); let m = s.demand_model().unwrap();
    let bound = bind_recovery_call(m, call(m, handler(m), function(m, "relay")), &premise(m, "App.p.b")).unwrap();
    assert!(bound[0].gaps.is_empty());
    assert_eq!(bound[0].invocation.arguments, vec![Some(instance(m, "App.p.b"))]);
    let src = SOURCE.replace("relay(c);", "let parent = self; recover_parent(parent);")
        .replace("fn main()", "fn recover_parent(p: Parent) { restart(p); }\nfn main()");
    let s = snapshot(&src); let m = s.demand_model().unwrap(); m.validate().unwrap();
    let target = function(m, "recover_parent");
    let bound = bind_recovery_call(m, call(m, handler(m), target), &premise(m, "App.p.a")).unwrap();
    let action = m.analyses.recovery.user.as_ref().unwrap().iter().find(|r| r.body == Some(EntityRef::Function(target))).unwrap();
    assert_eq!(bind_recovery_operand(m, &bound[0].invocation, &action.operand), Ok(instance(m, "App.p")),
        "a parent target must not be silently rewritten to the failing child");
}

#[test]
fn uncertainty_in_a_scalar_argument_does_not_erase_the_known_child_binding() {
    let src = SOURCE.replace("fn relay(x: Child)", "fn relay(x: Child, n: Int)")
        .replace("relay(c);", "relay(c, 1);");
    let s = snapshot(&src); let m = s.demand_model().unwrap();
    let relay = function(m, "relay"); let recover = function(m, "recover");
    let first = bind_recovery_call(m, call(m, handler(m), relay), &premise(m, "App.p.a")).unwrap();
    assert_eq!(first[0].gaps.len(), 1);
    assert_eq!(first[0].gaps[0].input, hale_types::model_query::RecoveryBindingInput::Argument(1));
    let second = bind_recovery_call(m, call(m, relay, recover), &first[0].invocation).unwrap();
    assert!(second[0].gaps.is_empty());
    let action = m.analyses.recovery.user.as_ref().unwrap().iter().find(|r| r.body == Some(EntityRef::Function(recover))).unwrap();
    assert_eq!(bind_recovery_operand(m, &second[0].invocation, &action.operand), Ok(instance(m, "App.p.a")));
}

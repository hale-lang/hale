//! Source phase membership is shared by certificates and canonical graph roots.
#[path = "support/entries.rs"]
mod entries;

use hale_frontend::snapshot::{Config, Snapshot};
use hale_types::verdict::Verdict;

fn snapshot(source: &str) -> Snapshot {
    entries::load_files(&[("main.hl", source)], Config::check(true, false))
}

fn judged(s: &Snapshot, reports: &hale_types::effects::EffectCertificates) -> (Verdict, Vec<String>) {
    let model = s.demand_model().unwrap();
    model.validate().unwrap();
    let bundle = s.bundle();
    let table = hale_types::claim_lowering::lower_claims(&bundle, model);
    let evidence = hale_types::evidence::derive_certificate_evidence_over(&bundle, &table, model, reports, s.demand_alloc_summary().unwrap());
    let ordinal = table.rows.iter().find(|r| matches!(r.law, hale_model::ClaimIr::PhaseEffects { .. })).unwrap().ordinal;
    let row = hale_types::judgment::judge_certificates(&table, model, &evidence, &[ordinal]).into_iter().find(|r| r.ordinal == ordinal).unwrap();
    (row.verdict, row.diags.into_iter().map(|d| d.message).collect())
}

#[test]
fn automatic_assertions_are_phase_roots_without_authored_hooks() {
    for (phase, epoch) in [("birth", "birth"), ("dissolve", "dissolve"), ("tick", "tick"), ("duration", "duration(1ms)")] {
        let source = format!(r#"
fn observe() -> Int {{ println("epoch operand"); return 0; }}
@phase_effects({phase}: {{}})
locus Worker {{
    params {{ n: Int = 0; }}
    closure audit {{ observe() + self.n ~~ self.n within 0; epoch {epoch}; }}
}}
fn main() {{ Worker {{ }}; }}
"#);
        let s = snapshot(&source);
        let reports = s.demand_effect_certificates().unwrap();
        let report = reports.iter().find(|(r, _)| r.subject == "Worker" && r.form.ends_with(&format!("during {phase}"))).unwrap();
        assert_eq!(report.0.result, Verdict::Violated, "{phase}");
        assert!(report.1.iter().any(|(d, _)| d.message.contains("Worker::closures::audit")));
        assert_eq!(judged(&s, reports).0, Verdict::Violated, "{phase}");
        let model = s.demand_model().unwrap();
        let definition = model.relations.closure_definitions.iter().find(|d| d.name == "audit").unwrap();
        let binding = model.relations.phase_of.iter().find(|p| p.function == definition.function).unwrap();
        assert_eq!(model.entities.phases[binding.phase.index()].name, phase);
        let summary = s.demand_alloc_summary().unwrap();
        let (key, source) = summary.source_phases.iter().find(|(k, _)| k.display() == "Worker::closures::audit").unwrap();
        assert_eq!(source.owner, summary.closure_assertions[key].owner);
        assert_eq!(source.phase, phase);
        assert!(source.hook);
        let mut missing = model.clone();
        missing.relations.phase_of.retain(|p| p.function != definition.function);
        assert!(missing.validate_coverage().is_err(), "a declared automatic epoch cannot lose its source phase");
    }
}

#[test]
fn phase_certificates_join_hooks_and_assertions_before_contextual_unknowns() {
    let source = r#"
@phase_effects(birth: {}, run: {})
locus Worker {
    params { n: Int = 0; }
    birth() { println("hook"); }
    closure audit { self.n ~~ self.n within 0; epoch birth; }
}
fn main() { Worker { }; }
"#;
    let s = snapshot(source);
    let reports = s.demand_effect_certificates().unwrap();
    assert_eq!(reports.iter().find(|(r, _)| r.form.ends_with("during birth")).unwrap().0.result, Verdict::Violated);
    assert_eq!(judged(&s, reports).0, Verdict::Violated);
    let quiet = snapshot(&source.replace("println(\"hook\");", ""));
    assert_eq!(judged(&quiet, quiet.demand_effect_certificates().unwrap()).0, Verdict::Uncertified);
}

#[test]
fn missing_phase_reports_do_not_synthesize_holds_over_real_assertions() {
    let source = r#"
@phase_effects(birth: {})
locus Worker { params { n: Int = 0; } closure audit { self.n ~~ 0 within 0; epoch birth; } }
fn main() { Worker { }; }
"#;
    let s = snapshot(source);
    let (result, diags) = judged(&s, &Vec::new());
    assert_eq!(result, Verdict::Uncertified);
    assert!(diags.iter().any(|d| d.contains("no matching certificate")), "{diags:?}");
    let empty = snapshot(&source.replace("epoch birth", "epoch explicit"));
    assert_eq!(judged(&empty, empty.demand_effect_certificates().unwrap()).0, Verdict::Holds);
    let model = empty.demand_model().unwrap();
    let assertion = model.relations.closure_definitions[0].function;
    assert!(!model.relations.phase_of.iter().any(|p| p.function == assertion));
}

#[test]
fn graph_during_filters_enter_automatic_assertion_operands() {
    let source = r#"
fn observe() -> Int { println("epoch operand"); return 0; }
locus Worker {
    params { n: Int = 0; }
    closure audit { observe() + self.n ~~ self.n within 0; epoch birth; }
}
group workers = { Worker };
main locus App {
    claims { quiet: forbid reaches(workers, effects(stdio)) during birth; }
}
fn main() { Worker { }; App { }; }
"#;
    let s = snapshot(source);
    let model = s.demand_model().unwrap(); model.validate().unwrap();
    let table = hale_types::claim_lowering::lower_claims(&s.bundle(), model);
    let (pre, rows, proofs) = hale_types::judgment::judge_forbid_reaches_with_certificates(&table, model, &[0]);
    assert!(pre.is_empty(), "{pre:?}");
    assert_eq!(rows[0].verdict, Verdict::Violated);
    proofs[&rows[0].ordinal].validate().unwrap();
}

#[test]
fn module_phase_contracts_and_modes_read_the_same_inventory() {
    let declaration = "@phase_effects(bulk: {}) locus Worker { mode bulk() { println(1); } }";
    for declaration in [declaration.to_string(), format!("module inner {{ {declaration} }}")] {
        let s = snapshot(&format!("{declaration}\nfn main() {{ Worker {{ }}; }}"));
        let reports = s.demand_effect_certificates().unwrap();
        assert_eq!(reports.iter().find(|(r, _)| r.subject == "Worker").unwrap().0.result, Verdict::Violated);
        assert_eq!(judged(&s, reports).0, Verdict::Violated);
    }
}

#[test]
fn a_method_named_tick_and_tick_assertions_keep_distinct_source_roles() {
    let source = r#"
@phase_effects(tick: {})
locus Worker {
    params { n: Int = 0; }
    fn tick() { }
    closure audit { self.n ~~ self.n within 0; epoch tick; }
}
fn main() { Worker { }; }
"#;
    let s = snapshot(source);
    let summary = s.demand_alloc_summary().unwrap();
    let roots = summary.source_phases.iter().filter(|(_, p)| p.phase == "tick").collect::<Vec<_>>();
    assert_eq!(roots.len(), 2);
    assert!(!roots.iter().find(|(k, _)| k.fn_name == "tick").unwrap().1.hook);
    assert!(roots.iter().find(|(k, _)| k.fn_name == "closures::audit").unwrap().1.hook);
    assert_eq!(judged(&s, s.demand_effect_certificates().unwrap()).0, Verdict::Uncertified);
}

#[test]
fn explicit_assertions_keep_the_phase_of_their_real_checkpoint_caller() {
    let source = r#"
fn observe() -> Int { println("explicit operand"); return 0; }
@phase_effects(birth: {})
locus Worker {
    params { n: Int = 0; }
    birth() { check_closures(); }
    closure audit { observe() + self.n ~~ self.n within 0; epoch explicit; }
}
fn main() { Worker { }; }
"#;
    let s = snapshot(source);
    assert_eq!(judged(&s, s.demand_effect_certificates().unwrap()).0, Verdict::Violated);
    let model = s.demand_model().unwrap();
    let assertion = model.relations.closure_definitions[0].function;
    assert!(!model.relations.phase_of.iter().any(|p| p.function == assertion));
    assert_eq!(model.relations.closure_evaluations.len(), 1);
}

#[test]
fn imported_phase_roots_do_not_borrow_another_locus_owner() {
    let main = r#"
import "./lib" as lib;
@phase_effects(birth: {}) locus Worker { }
fn main() { Worker { }; lib::Worker { }; }
"#;
    let library = r#"
fn observe() -> Int { println("imported operand"); return 0; }
@phase_effects(birth: {})
locus Worker {
    params { n: Int = 0; }
    closure audit { observe() + self.n ~~ self.n within 0; epoch birth; }
}

"#;
    let s = entries::load_files(&[("main.hl", main), ("lib/worker.hl", library)], Config::check(true, false));
    let model = s.demand_model().unwrap(); model.validate().unwrap();
    let bundle = s.bundle();
    let table = hale_types::claim_lowering::lower_claims(&bundle, model);
    let evidence = hale_types::evidence::derive_certificate_evidence_over(&bundle, &table, model, s.demand_effect_certificates().unwrap(), s.demand_alloc_summary().unwrap());
    let rows = hale_types::judgment::judge_certificates(&table, model, &evidence, &[0]);
    for (name, expected) in [("Worker", Verdict::Holds), ("lib::Worker", Verdict::Violated)] {
        let ordinal = table.rows.iter().find(|r| matches!(&r.law, hale_model::ClaimIr::PhaseEffects { locus, .. } if locus.1.display == name)).unwrap().ordinal;
        assert_eq!(rows.iter().find(|r| r.ordinal == ordinal).unwrap().verdict, expected, "{name}");
    }
}

#[test]
fn failure_handler_phase_names_use_the_actual_handler_bodies() {
    let source = r#"
locus Child { }
@phase_effects(on_failure: {})
locus Worker {
    params { child: Child = Child { }; }
    on_failure(c: Child, error: ClosureViolation) { println("failure body"); }
}
fn main() { Worker { }; }
"#;
    let s = snapshot(source);
    assert_eq!(judged(&s, s.demand_effect_certificates().unwrap()).0, Verdict::Violated);
    let summary = s.demand_alloc_summary().unwrap();
    let (key, phase) = summary.source_phases.iter().find(|(_, p)| p.phase == "on_failure").unwrap();
    assert!(phase.hook);
    assert_eq!(summary.fns[key].entry, Some(hale_types::alloc_summary::EntryKind::FailureHandler));
}

#[test]
fn authored_phase_membership_cannot_be_dropped_into_a_vacuous_certificate() {
    let s = snapshot("@phase_effects(birth: {}) locus Worker { birth() { println(1); } } fn main() { Worker { }; }");
    let mut model = s.demand_model().unwrap().clone();
    model.relations.phase_of.clear();
    assert!(model.validate_coverage().is_err());
    let bundle = s.bundle();
    let table = hale_types::claim_lowering::lower_claims(&bundle, &model);
    let evidence = hale_types::evidence::derive_certificate_evidence_over(&bundle, &table, &model, &Vec::new(), s.demand_alloc_summary().unwrap());
    let rows = hale_types::judgment::judge_certificates(&table, &model, &evidence, &[0]);
    assert_eq!(rows.len(), 1);
    assert!(rows.iter().all(|row| row.verdict != Verdict::Holds));
}

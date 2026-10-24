//! Explicit failure dispatch is behavioral uncertainty even when the
//! handler has no recovery statement. Existing violations still win.
#[path = "support/entries.rs"]
mod entries;

use hale_frontend::snapshot::{Config, Snapshot};
use hale_model::{EntityRef, FunctionId, HoleKind};
use hale_types::alloc_summary::StructuralFailureStep;
use hale_types::verdict::Verdict;

const SOURCE: &str = r#"
locus Child {
    params { n: Int = 0; }
    closure broken { captures: n; epoch inline; }
    @effects(none: {stdio}) fn break_it() { self.n = 1; violate broken; }
}
fn relay(c: Child) { c.break_it(); }
@budget(publish = 0) fn measure(c: Child) { relay(c); }
@effects(none: {stdio}) fn pure() { }
group children = { Child };
main locus App {
    params { child: Child = Child { }; }
    on_failure(c: Child, e: ClosureViolation) { println("failure handler ran"); }
    run() { self.child.break_it(); }
    // claim insertion
}
fn main() { App { }; }
"#;

fn snapshot(src: &str) -> Snapshot {
    entries::load_files(&[("app.hl", src)], Config::check(true, false))
}

fn function(m: &hale_model::ApplicationModel, name: &str) -> FunctionId {
    FunctionId(m.entities.functions.iter().position(|f| f.name == name).unwrap() as u32)
}

#[test]
fn implicit_handler_work_is_not_an_empty_effect_set() {
    let s = snapshot(SOURCE);
    let check = s.demand_check().unwrap();
    assert!(check.diags.iter().any(|d| d.message.contains("implicit failure-handler dispatch")));
    let report = s.demand_effect_certificates().unwrap();
    let failed = report.iter().find(|(row, _)| row.subject == "Child::break_it").unwrap();
    assert_eq!(failed.0.result, Verdict::Uncertified);
    assert!(failed.1.iter().any(|(d, foreign)| d.message.contains("structural failure is raised here") && !foreign));
    assert_eq!(report.iter().find(|(row, _)| row.subject == "pure").unwrap().0.result, Verdict::Holds);
    let m = s.demand_model().unwrap(); m.validate().unwrap();
    for name in ["Child::break_it", "relay", "measure", "App::run"] {
        assert!(m.holes.iter().any(|h| h.at == EntityRef::Function(function(m, name)) && h.kind == HoleKind::UnmodeledFailureDispatch), "{name}");
    }
    assert!(!m.holes.iter().any(|h| h.at == EntityRef::Function(function(m, "pure"))));
}

#[test]
fn budgets_and_graph_claims_share_the_failure_gap() {
    let src = SOURCE.replace("// claim insertion", "claims { quiet: forbid reaches(children, effects(stdio)); }");
    let s = snapshot(&src); let m = s.demand_model().unwrap(); let bundle = s.bundle();
    let table = hale_types::claim_lowering::lower_claims(&bundle, m);
    let evidence = hale_types::evidence::derive_certificate_evidence_over(&bundle, &table, m,
        s.demand_effect_certificates().unwrap(), s.demand_alloc_summary().unwrap());
    let certificates = hale_types::judgment::judge_certificates(&table, m, &evidence, &[0]);
    let budget = table.rows.iter().find(|r| matches!(r.law, hale_model::ClaimIr::QuantBudget { .. })).unwrap();
    assert_eq!(certificates.iter().find(|r| r.ordinal == budget.ordinal).unwrap().verdict, Verdict::Uncertified);
    let (pre, graph, proofs) = hale_types::judgment::judge_forbid_reaches_with_certificates(&table, m, &[0]);
    assert!(pre.is_empty(), "{pre:?}");
    assert_eq!(graph.len(), 1); assert_eq!(graph[0].verdict, Verdict::Uncertified);
    assert!(proofs.is_empty(), "an incomplete failure path supplies no closed-set certificate");
}

#[test]
fn known_effect_and_no_panic_violations_remain_violations() {
    for source in [
        SOURCE.replace("self.n = 1; violate broken;", "println(1); violate broken;"),
        SOURCE.replace("@effects(none: {stdio}) fn break_it", "@no_panic fn break_it"),
    ] {
        let s = snapshot(&source);
        let report = s.demand_effect_certificates().unwrap();
        assert_eq!(report.iter().find(|(row, _)| row.subject == "Child::break_it").unwrap().0.result, Verdict::Violated);
    }
}

#[test]
fn phase_contracts_retain_the_same_gap_without_poisoning_an_empty_hook() {
    let source = SOURCE.replace("main locus App", "@phase_effects(birth: {}, run: {}) main locus App")
        .replace("    run()", "    birth() { }\n    run()");
    let s = snapshot(&source); let report = s.demand_effect_certificates().unwrap();
    let rows: Vec<_> = report.iter().filter(|(row, _)| row.subject == "App").collect();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows.iter().find(|(row, _)| row.form.ends_with("during birth")).unwrap().0.result, Verdict::Holds);
    assert_eq!(rows.iter().find(|(row, _)| row.form.ends_with("during run")).unwrap().0.result, Verdict::Uncertified);
}

#[test]
fn helper_cycles_keep_a_finite_failure_witness_and_pure_cycles_stay_separate() {
    let source = SOURCE.replace("fn relay(c: Child) { c.break_it(); }",
        "fn relay(c: Child) { again(c); } fn again(c: Child) { relay(c); c.break_it(); } fn clean_a() { clean_b(); } fn clean_b() { clean_a(); }");
    let s = snapshot(&source); let summary = s.demand_alloc_summary().unwrap();
    let key = |name: &str| summary.fns.keys().find(|k| k.fn_name == name).unwrap();
    let trace = summary.structural_failure_path(key("measure")).unwrap().unwrap();
    assert_eq!(trace.path.iter().map(|k| k.display()).collect::<Vec<_>>(), ["measure", "relay", "again", "Child::break_it"]);
    assert_eq!(trace.closure, "broken"); assert!(!trace.foreign);
    assert!(summary.structural_failure_path(key("clean_a")).unwrap().is_none());
    assert!(summary.program_rows().structural_failure_path(key("measure")).unwrap().is_some());
}

#[test]
fn missing_or_broken_failure_coverage_cannot_certify_a_pure_effect_assertion() {
    let s = snapshot(SOURCE); let base = s.demand_alloc_summary().unwrap();
    let pure = base.fns.keys().find(|k| k.fn_name == "pure").unwrap().clone();
    let bundle = s.bundle();
    let programs: Vec<_> = bundle.programs.values().copied().collect();
    let span = programs[0].span;
    for variant in 0..3 {
        let mut summary = base.clone();
        match variant {
            0 => summary.structural_failures = None,
            1 => { summary.structural_failures.as_mut().unwrap().insert(pure.clone(), StructuralFailureStep::Through { callee: pure.clone(), span }); }
            _ => {
                let mut absent = pure.clone(); absent.fn_name = "absent".to_string(); absent.decl = None;
                summary.structural_failures.as_mut().unwrap().insert(pure.clone(), StructuralFailureStep::Through { callee: absent, span });
            }
        }
        assert!(summary.structural_failure_path(&pure).is_err());
        let (_, _, _, report) = hale_types::effects::effect_report_three_way(&programs, &summary);
        assert_eq!(report.iter().find(|(row, _)| row.subject == "pure").unwrap().0.result, Verdict::Uncertified);
    }
}

#[test]
fn contracted_stdlib_failure_paths_retain_their_foreign_witness() {
    let source = SOURCE.replace("fn pure() { }", "fn pure() { let b = std::bytes::BytesBuilder { }; b.append_str(\"x\"); }");
    let s = snapshot(&source); let summary = s.demand_alloc_summary().unwrap();
    let key = summary.fns.keys().find(|k| k.fn_name == "pure").unwrap();
    let trace = summary.structural_failure_path(key).unwrap().expect("stdlib append failure");
    assert!(trace.foreign); assert!(trace.path.len() >= 2);
    let own = summary.program_rows();
    let projected = own.structural_failure_path(key).unwrap().unwrap();
    assert!(projected.foreign); assert_eq!(trace.path, projected.path);
    let m = s.demand_model().unwrap(); m.validate().unwrap();
    assert!(m.holes.iter().any(|h| h.at == EntityRef::Function(function(m, "pure")) && h.kind == HoleKind::UnmodeledFailureDispatch));
}

#[test]
fn evaluated_defaults_preserve_the_kind_of_execution_coverage_gap() {
    let source = SOURCE.replace("fn pure() { }", "fn pure() { let settings = Settings { }; }")
        .replace("fn main()", "type Settings { n: Int = if true { fail_helper(); 0 } else { 0 }; }\nfn fail_helper() { Child { }.break_it(); }\nfn unrelated_recovery(c: Child) { restart(c); }\nfn main()");
    let s = snapshot(&source); let m = s.demand_model().unwrap(); m.validate().unwrap();
    let pure = function(m, "pure");
    let gaps: Vec<_> = m.holes.iter().filter(|h| h.at == EntityRef::Function(pure) && h.kind.implicit_execution_families().is_some()).collect();
    assert!(!gaps.is_empty(), "the default reaches a failure helper");
    assert!(gaps.iter().all(|h| h.kind == HoleKind::UnmodeledFailureDispatch && !h.hides.intersects(hale_model::RelationSet::PUBLISHES)),
        "an unrelated recovery cannot change this default's direct publish inventory: {gaps:?}");
    assert!(hale_types::model_query::recovery_cost_gap(m, pure, false).is_some());
    let mixed = source.replace("Child { }.break_it();", "let c = Child { }; c.break_it(); restart(c);");
    let s = snapshot(&mixed); let m = s.demand_model().unwrap(); m.validate().unwrap();
    let pure = function(m, "pure");
    assert!(m.holes.iter().any(|h| h.at == EntityRef::Function(pure) && h.kind == HoleKind::UnmodeledRecovery && h.hides.intersects(hale_model::RelationSet::PUBLISHES)),
        "a default that also reaches actual recovery retains its broader existing gap");
}

const INITIALIZER_SOURCE: &str = r#"
locus Child {
    params { n: Int = 0; }
    closure broken { captures: n; epoch inline; }
    fn break_it() { self.n = 1; violate broken; }
}
locus Settings {
    params {
        child: Child = Child { };
        n: Int = if true { self.child.break_it(); 0 } else { 0 };
    }
    on_failure(c: Child, e: ClosureViolation) { println("default failure handler ran"); }
}
main locus App {
    @effects(none: {stdio}) fn measure() { let settings = Settings { }; }
    @effects(none: {stdio}) fn overridden() { let settings = Settings { n: 4 }; }
    @effects(none: {stdio}) fn untouched() { }
    run() { self.measure(); }
}
fn main() { App { }; }
"#;

#[test]
fn constructor_execution_includes_only_omitted_parameter_defaults() {
    let s = snapshot(INITIALIZER_SOURCE);
    let summary = s.demand_alloc_summary().unwrap();
    let key = |name: &str| summary.fns.keys().find(|k| k.fn_name == name).unwrap();
    let trace = summary.structural_failure_path(key("measure")).unwrap().unwrap();
    assert_eq!(trace.path.iter().map(|k| k.display()).collect::<Vec<_>>(), ["App::measure", "Settings::params::n", "Child::break_it"]);
    assert_eq!(trace.initializers.iter().map(|k| k.field.as_str()).collect::<Vec<_>>(), ["n"]);
    for name in ["overridden", "untouched"] {
        assert!(summary.structural_failure_path(key(name)).unwrap().is_none(), "{name}");
    }
    let report = s.demand_effect_certificates().unwrap();
    for (name, expected) in [("App::measure", Verdict::Uncertified), ("App::overridden", Verdict::Holds), ("App::untouched", Verdict::Holds)] {
        assert_eq!(report.iter().find(|(row, _)| row.subject == name).unwrap().0.result, expected, "{name}");
    }
    let m = s.demand_model().unwrap(); m.validate().unwrap();
    assert!(m.holes.iter().any(|h| h.at == EntityRef::Function(function(m, "App::measure")) && h.kind == HoleKind::UnmodeledFailureDispatch));
    for name in ["App::overridden", "App::untouched"] {
        assert!(!m.holes.iter().any(|h| h.at == EntityRef::Function(function(m, name)) && h.kind == HoleKind::UnmodeledFailureDispatch), "{name}");
    }
    assert!(m.entities.functions.iter().filter(|f| f.name.contains("params::")).all(|f| f.kind == hale_model::FunctionKind::ParameterDefault), "parameter defaults have their own executable-body kind");
}

#[test]
fn constructor_failure_coverage_follows_nested_defaults_and_helpers() {
    let source = INITIALIZER_SOURCE.replace("let settings = Settings { };", "construct();")
        .replace("fn main()", "fn construct() { let wrapper = Wrapper { }; }\nlocus Wrapper { params { settings: Settings = Settings { }; } }\nfn main()");
    let s = snapshot(&source); let summary = s.demand_alloc_summary().unwrap();
    let key = summary.fns.keys().find(|k| k.fn_name == "measure").unwrap();
    let trace = summary.structural_failure_path(key).unwrap().unwrap();
    assert_eq!(trace.path.iter().map(|k| k.display()).collect::<Vec<_>>(), ["App::measure", "construct", "Wrapper::params::settings", "Settings::params::n", "Child::break_it"]);
    assert_eq!(trace.initializers.iter().map(|k| k.field.as_str()).collect::<Vec<_>>(), ["settings", "n"]);
    let projected = summary.program_rows().structural_failure_path(key).unwrap().unwrap();
    assert_eq!(projected.initializers, trace.initializers);
    assert_eq!(s.demand_effect_certificates().unwrap().iter().find(|(row, _)| row.subject == "App::measure").unwrap().0.result, Verdict::Uncertified);
}

#[test]
fn multiple_constructor_sites_keep_the_omission_that_can_execute() {
    let source = INITIALIZER_SOURCE.replace("let settings = Settings { };", "let explicit = Settings { n: 7 }; let omitted = Settings { };");
    let s = snapshot(&source);
    let summary = s.demand_alloc_summary().unwrap();
    let row = summary.fns.values().find(|f| f.key.fn_name == "measure").unwrap();
    assert_eq!(row.constructions.len(), 2);
    assert!(row.constructions.iter().all(|c| c.site.is_some() && c.target.is_some()));
    assert_eq!(row.constructions.iter().filter(|c| c.supplied.contains("n")).count(), 1);
    assert!(summary.structural_failure_path(&row.key).unwrap().is_some());
    assert_eq!(s.demand_effect_certificates().unwrap().iter().find(|(row, _)| row.subject == "App::measure").unwrap().0.result, Verdict::Uncertified);
}

#[test]
fn an_unused_initializer_does_not_poison_a_literal_only_constructor() {
    let source = INITIALIZER_SOURCE.replace("let settings = Settings { };", "let child = Child { };");
    let s = snapshot(&source);
    assert_eq!(s.demand_effect_certificates().unwrap().iter().find(|(row, _)| row.subject == "App::measure").unwrap().0.result, Verdict::Holds);
}

#[test]
fn constructor_aliases_use_the_resolved_declaration_identity() {
    let source = INITIALIZER_SOURCE.replace("let settings = Settings { };", "let settings = Options { };")
        .replace("fn main()", "type Options = Settings;\nfn main()");
    let s = snapshot(&source); let summary = s.demand_alloc_summary().unwrap();
    let measure = summary.fns.values().find(|f| f.key.fn_name == "measure").unwrap();
    let overridden = summary.fns.values().find(|f| f.key.fn_name == "overridden").unwrap();
    assert_eq!(measure.constructions[0].target, overridden.constructions[0].target);
    assert!(summary.structural_failure_path(&measure.key).unwrap().is_some());
    assert_eq!(s.demand_effect_certificates().unwrap().iter().find(|(row, _)| row.subject == "App::measure").unwrap().0.result, Verdict::Uncertified);
}

#[test]
fn initializer_failure_coverage_reaches_budget_evidence_without_recovery_syntax() {
    let source = INITIALIZER_SOURCE.replace("@effects(none: {stdio}) fn measure", "@budget(publish = 0) fn measure");
    let s = snapshot(&source); let m = s.demand_model().unwrap(); let bundle = s.bundle();
    let table = hale_types::claim_lowering::lower_claims(&bundle, m);
    let evidence = hale_types::evidence::derive_certificate_evidence_over(&bundle, &table, m,
        s.demand_effect_certificates().unwrap(), s.demand_alloc_summary().unwrap());
    let certificates = hale_types::judgment::judge_certificates(&table, m, &evidence, &[0]);
    let budget = table.rows.iter().find(|r| matches!(r.law, hale_model::ClaimIr::QuantBudget { .. })).unwrap();
    assert_eq!(certificates.iter().find(|r| r.ordinal == budget.ordinal).unwrap().verdict, Verdict::Uncertified);
}

#[test]
fn initializer_bodies_keep_their_declaration_universe_in_projected_views() {
    let s = snapshot(INITIALIZER_SOURCE); let summary = s.demand_alloc_summary().unwrap();
    let own = summary.program_rows();
    assert!(own.declaration_bodies.iter().all(|b| b.universe == hale_types::placement::SiteUniverse::User));
    for body in summary.declaration_bodies.iter().filter(|body| body.initializer.is_some()) {
        assert_eq!(body.initializer.as_ref().unwrap().owner.universe, body.universe);
    }
    let names: Vec<_> = summary.declaration_bodies.iter()
        .filter(|b| b.summary.key.locus.as_deref() == Some("Settings"))
        .filter_map(|b| b.initializer.as_ref().map(|k| k.field.as_str())).collect();
    assert_eq!(names, ["child", "n"]);
}

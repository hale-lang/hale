//! Source callback absence is a positive inventory fact. It does not prove
//! failure freedom, native costs, or instance-aware recovery containment.
#[path = "support/entries.rs"]
mod entries;
use hale_frontend::snapshot::{Config, Snapshot};
use hale_model::HoleKind;
use hale_types::verdict::Verdict;

const SOURCE: &str = r#"
locus Child {
    closure bad { epoch inline; }
    birth_check { false } -> violate bad;
}
@effects(none: {stdio}) @no_panic @budget(stack_bytes = 999999)
fn construct() { Child { }; }
main locus App { run() { construct(); } }
fn main() { App { }; }
"#;
fn snapshot(source: &str) -> Snapshot {
    entries::load_files(&[("main.hl", source)], Config::check(true, false))
}
#[test]
fn an_empty_complete_census_closes_source_effects_but_not_failure_or_costs() {
    let s = snapshot(SOURCE);
    let summary = s.demand_alloc_summary().unwrap();
    assert!(summary.no_source_failure_handlers());
    assert!(summary.failure_handlers.as_ref().unwrap().declarations.is_empty());
    let reports = s.demand_effect_certificates().unwrap();
    let source = reports.iter().find(|(r, _)| r.subject == "construct" && r.form.contains("effects(stdio)")).unwrap();
    assert_eq!(source.0.result, Verdict::Holds);
    let panic = reports.iter().find(|(r, _)| r.subject == "construct" && r.form.contains(", panic)")).unwrap();
    assert_eq!(panic.0.result, Verdict::Uncertified);
    let model = s.demand_model().unwrap(); model.validate().unwrap();
    let table = hale_types::claim_lowering::lower_claims(&s.bundle(), model);
    let evidence = hale_types::evidence::derive_certificate_evidence_over(&s.bundle(), &table, model, reports, summary);
    let judged = hale_types::judgment::judge_certificates(&table, model, &evidence, &[0]);
    for row in &table.rows {
        if matches!(row.law, hale_model::ClaimIr::QuantBudget { .. } | hale_model::ClaimIr::NoPanic { .. }) {
            assert_eq!(judged.iter().find(|j| j.ordinal == row.ordinal).unwrap().verdict, Verdict::Uncertified);
        }
    }
    assert!(model.holes.iter().any(|h| h.kind == HoleKind::UnmodeledFailureContext));
    assert!(!model.capabilities.exact_costs);
}
#[test]
fn source_header_inventory_includes_unreachable_handlers() {
    let source = SOURCE.replace("main locus App", "locus Dormant { on_failure(c: Child, e: ClosureViolation) { println(\"handler\"); } }\nmain locus App");
    let s = snapshot(&source);
    let summary = s.demand_alloc_summary().unwrap();
    assert!(!summary.no_source_failure_handlers());
    assert_eq!(summary.failure_handlers.as_ref().unwrap().declarations.len(), 1);
    let model = s.demand_model().unwrap(); model.validate().unwrap();
    assert_eq!(model.analyses.failure_sources.as_ref().unwrap().handlers.len(), 1);
    let reports = s.demand_effect_certificates().unwrap();
    assert_eq!(reports.iter().find(|(r, _)| r.subject == "construct" && r.form.contains("effects(stdio)")).unwrap().0.result, Verdict::Uncertified);
}
#[test]
fn missing_census_or_source_bodies_cannot_close_failure_effects() {
    let s = snapshot(SOURCE);
    let base = s.demand_alloc_summary().unwrap();
    let bundle = s.bundle(); let programs: Vec<_> = bundle.programs.values().copied().collect();
    for variant in 0..5 {
        let mut summary = base.clone();
        match variant {
            0 => summary.failure_handlers = None,
            1 => summary.failure_handlers.as_mut().unwrap().complete = false,
            2 => summary.structural_failures = None,
            3 => { let target = summary.birth_checks.keys().find(|key| summary.is_own(key)).unwrap().clone(); summary.fns.remove(&target); }
            _ => { let body = summary.fns.values_mut().find(|f| f.key.fn_name == "construct").unwrap(); body.constructor_checks.clear(); }
        }
        let (_, _, _, reports) = hale_types::effects::effect_report_three_way(&programs, &summary);
        assert_eq!(reports.iter().find(|(r, _)| r.subject == "construct" && r.form.contains("effects(stdio)")).unwrap().0.result, Verdict::Uncertified, "variant {variant}");
    }
}
#[test]
fn native_failure_costs_require_the_positive_source_account() {
    let s = snapshot(SOURCE); let model = s.demand_model().unwrap(); model.validate().unwrap();
    for mutation in 0..6 {
        let mut changed = model.clone();
        match mutation {
            0 => changed.analyses.failure_sources = None,
            1 => changed.analyses.failure_sources.as_mut().unwrap().foreign_handlers = 1,
            2 => changed.analyses.failure_sources.as_mut().unwrap().may_fail.clear(),
            3 => changed.holes.retain(|h| h.kind != HoleKind::UnmodeledFailureContext),
            4 => changed.analyses.failure_sources.as_mut().unwrap().handlers.push(hale_model::FunctionId(0)),
            _ => { let account = changed.analyses.failure_sources.as_mut().unwrap(); account.may_fail.push(hale_model::FunctionId(u32::MAX)); }
        }
        assert!(changed.validate().is_err(), "mutation {mutation}");
    }
}
#[test]
fn known_operand_work_still_violates_the_source_contract() {
    let source = SOURCE.replace("birth_check { false }", "birth_check { operand() }")
        + "\nfn operand() -> Bool { println(\"known\"); return false; }\n";
    let s = snapshot(&source); let reports = s.demand_effect_certificates().unwrap();
    assert_eq!(reports.iter().find(|(r, _)| r.subject == "construct" && r.form.contains("effects(stdio)")).unwrap().0.result, Verdict::Violated);
}

//! Constructor execution preserves default bodies and helper barriers.
#[path = "support/entries.rs"]
mod entries;
use hale_frontend::snapshot::{Config, Snapshot};
use hale_model::{EntityRef, FunctionId, FunctionKind};
use hale_types::verdict::Verdict;

const SOURCE: &str = r#"
fn observe() -> Int { println("initializer observed"); return 1; }
locus Settings { params { n: Int = observe(); } }
group apps = { App };
group observers = { observe };
main locus App {
    @effects(none: {stdio}) fn omitted() { let settings = Settings { }; }
    @effects(none: {stdio}) fn supplied() { let settings = Settings { n: 4 }; }
    run() { }
    claims {
        quiet: forbid reaches(apps, effects(stdio));
        guarded: forbid reaches(apps, effects(stdio)) avoiding observers;
    }
}
fn main() { App { }; }
"#;
fn snapshot(source: &str) -> Snapshot {
    entries::load_files(&[("app.hl", source)], Config::check(true, false))
}
fn function(model: &hale_model::ApplicationModel, name: &str) -> FunctionId {
    FunctionId(model.entities.functions.iter().position(|f| f.name == name).unwrap() as u32)
}

#[test]
fn omitted_defaults_execute_and_supplied_fields_do_not() {
    let s = snapshot(SOURCE);
    let report = s.demand_effect_certificates().unwrap();
    for (name, expected) in [("App::omitted", Verdict::Violated), ("App::supplied", Verdict::Holds)] {
        assert_eq!(report.iter().find(|(row, _)| row.subject == name).unwrap().0.result, expected, "{name}");
    }
    let summary = s.demand_alloc_summary().unwrap();
    let omitted = summary.fns.values().find(|f| f.key.display() == "App::omitted").unwrap();
    let supplied = summary.fns.values().find(|f| f.key.display() == "App::supplied").unwrap();
    assert_eq!(omitted.initializations.len(), 1);
    assert!(omitted.calls.is_empty(), "the constructor is not an authored call");
    assert!(supplied.initializations.is_empty());
    let m = s.demand_model().unwrap(); m.validate().unwrap();
    let initializer = function(m, "Settings::params::n");
    assert_eq!(m.entities.functions[initializer.index()].kind, FunctionKind::ParameterDefault);
    assert_eq!(m.entities.functions[initializer.index()].initializer.as_ref().map(|d| d.field.as_str()), Some("n"));
    assert!(m.relations.initializes.iter().any(|e| e.from == function(m, "App::omitted") && e.to == initializer));
    assert!(!m.relations.calls.iter().any(|e| e.from == function(m, "App::omitted")));
    assert!(m.relations.calls.iter().any(|e| e.from == initializer && e.to == function(m, "observe")));
    assert!(m.entities.functions[initializer.index()].direct_effects.is_empty(), "helper I/O stays on its actual body");
}

#[test]
fn graph_claims_keep_the_helper_barrier_and_checkable_certificates() {
    let s = snapshot(SOURCE); let m = s.demand_model().unwrap(); let bundle = s.bundle();
    let table = hale_types::claim_lowering::lower_claims(&bundle, m);
    let (pre, rows, proofs) = hale_types::judgment::judge_forbid_reaches_with_certificates(&table, m, &[0]);
    assert!(pre.is_empty(), "{pre:?}");
    for (name, expected) in [("quiet", Verdict::Violated), ("guarded", Verdict::Holds)] {
        let clause = table.rows.iter().find(|r| r.name == name).unwrap();
        assert_eq!(rows.iter().find(|r| r.ordinal == clause.ordinal).unwrap().verdict, expected, "{name}");
        proofs[&clause.ordinal].validate().unwrap();
    }
}

#[test]
fn literal_and_pure_helper_defaults_keep_closed_effect_results() {
    let source = SOURCE.replace("println(\"initializer observed\"); ", "");
    let s = snapshot(&source); let report = s.demand_effect_certificates().unwrap();
    assert!(report.iter().all(|(row, _)| row.result == Verdict::Holds));
    let m = s.demand_model().unwrap(); m.validate().unwrap();
    let root = function(m, "App::omitted");
    assert!(!m.holes.iter().any(|h| h.at == EntityRef::Function(root)));
}

#[test]
fn initialization_targets_must_name_the_constructed_locus_parameter() {
    let s = snapshot(SOURCE); let model = s.demand_model().unwrap();
    for variant in 0..3 {
        let mut m = model.clone();
        let edge = m.relations.initializes.first_mut().unwrap();
        match variant {
            0 => edge.to = function(model, "observe"),
            1 => edge.field = "different".to_string(),
            _ => m.entities.functions[edge.to.index()].initializer = None,
        }
        assert!(m.validate().is_err(), "variant {variant}");
    }
}

fn budget_verdict(source: &str) -> Verdict {
    let s = snapshot(source); let model = s.demand_model().unwrap(); let bundle = s.bundle();
    let table = hale_types::claim_lowering::lower_claims(&bundle, model);
    let evidence = hale_types::evidence::derive_certificate_evidence_over(&bundle, &table, model,
        s.demand_effect_certificates().unwrap(), s.demand_alloc_summary().unwrap());
    let rows = hale_types::judgment::judge_certificates(&table, model, &evidence, &[0]);
    let budget = table.rows.iter().find(|row| matches!(row.law, hale_model::ClaimIr::AllocBudget { .. } | hale_model::ClaimIr::QuantBudget { .. })).unwrap();
    rows.iter().find(|row| row.ordinal == budget.ordinal).unwrap().verdict
}

#[test]
fn allocations_and_stack_locals_inside_default_expressions_are_counted() {
    let source = SOURCE.replace("n: Int = observe();", "n: Int = if true { let values: [Int; 64] = [0; 64]; 0 } else { 0 };");
    for (dimension, limit) in [("alloc_per_call", 1), ("stack_bytes", 256)] {
        let tested = source.replace("@effects(none: {stdio}) fn omitted", &format!("@budget({dimension} = {limit}) fn omitted"));
        assert_eq!(budget_verdict(&tested), Verdict::Violated, "{dimension}");
        let overridden = tested.replace("fn omitted() { let settings = Settings { }; }", "fn omitted() { let settings = Settings { n: 4 }; }");
        assert_eq!(budget_verdict(&overridden), Verdict::Holds, "overridden {dimension}");
    }
    let s = snapshot(&source); let summary = s.demand_alloc_summary().unwrap();
    let row = summary.fns.values().find(|row| row.key.display() == "Settings::params::n").unwrap();
    assert!(row.stack_bytes >= 512, "initializer locals are retained by the original body walk");
}

#[test]
fn each_omitted_default_contributes_to_the_blocking_budget() {
    let source = SOURCE.replace("println(\"initializer observed\");", "std::time::sleep(5ms);")
        .replace("params { n: Int = observe(); }", "params { a: Int = observe(); n: Int = observe(); }")
        .replace("@effects(none: {stdio}) fn omitted", "@budget(block_points = 1) fn omitted");
    assert_eq!(budget_verdict(&source), Verdict::Violated);
    let overridden = source.replace("fn omitted() { let settings = Settings { }; }", "fn omitted() { let settings = Settings { n: 4 }; }");
    assert_eq!(budget_verdict(&overridden), Verdict::Holds);
}

#[test]
fn constructor_omission_coverage_cannot_lose_an_execution_edge() {
    let s = snapshot(SOURCE); let model = s.demand_model().unwrap();
    for variant in 0..3 {
        let mut m = model.clone();
        let edge = m.relations.initializes[0].clone();
        match variant {
            0 => m.relations.initializes.clear(),
            1 => m.relations.constructions.retain(|row| row.function != edge.from || row.site != edge.site),
            _ => m.relations.constructions.iter_mut().find(|row| row.function == edge.from && row.site == edge.site).unwrap().supplied.push("n".to_string()),
        }
        assert!(m.validate_coverage().is_err(), "variant {variant}: evidence admission shares omission validation");
        assert!(m.validate().is_err(), "variant {variant}");
    }
}

#[test]
fn graph_bounds_sum_defaults_instead_of_treating_them_as_dispatch_alternatives() {
    let source = SOURCE.replace("fn observe()", "effect step;\n@effects(is: {step}) fn observe()")
        .replace("params { n: Int = observe(); }", "params { a: Int = observe(); n: Int = observe(); }")
        .replace("quiet: forbid", "budgeted: bound step <= 1 on paths from apps;\n        quiet: forbid");
    for (source, expected) in [
        (source.clone(), Verdict::Violated),
        (source.replace("fn omitted() { let settings = Settings { }; }", "fn omitted() { let settings = Settings { n: 4 }; }"), Verdict::Holds),
    ] {
        let s = snapshot(&source); let m = s.demand_model().unwrap(); let bundle = s.bundle();
        let table = hale_types::claim_lowering::lower_claims(&bundle, m);
        let ordinal = table.rows.iter().find(|row| row.name == "budgeted").unwrap().ordinal;
        let rows = hale_types::judgment::judge_bound(&table, m, &[0]);
        assert_eq!(rows.iter().find(|row| row.ordinal == ordinal).unwrap().verdict, expected);
    }
}

#[test]
fn stdlib_defaults_keep_typed_entries_and_the_structured_logging_barrier() {
    let source = r#"
main locus App {
    params { sink: std::log::StdoutSink = std::log::StdoutSink { }; }
    claims { structured: forbid reaches(program, effects(stdio)) avoiding std::log::*; }
}
fn main() { App { }; }
"#;
    let s = snapshot(source); let m = s.demand_model().unwrap(); m.validate().unwrap();
    assert!(m.holes.iter().any(|h| h.kind == hale_model::HoleKind::UnmodeledInitialization
        && h.reason.contains("birth lifecycle body") && h.reason.contains("StdoutSink::birth")));
    let entries: Vec<_> = m.analyses.stdlib_absorption.iter().filter(|a| a.entry_initialization.is_some()).collect();
    assert!(entries.iter().any(|a| a.entry_initialization.as_ref().unwrap().field == "format"));
    assert!(entries.iter().all(|a| a.entry_dispatch.is_none() && a.entry_group.is_none() && a.entry_bus.is_none()));
    let table = hale_types::claim_lowering::lower_claims(&s.bundle(), m);
    let (pre, rows, proofs) = hale_types::judgment::judge_forbid_reaches_with_certificates(&table, m, &[0]);
    assert!(pre.is_empty());
    assert_eq!(rows[0].verdict, Verdict::Uncertified);
    assert!(proofs.is_empty(), "masking the default helper cannot discharge lifecycle context at its constructor");
    let entry = m.analyses.stdlib_absorption.iter().position(|a| a.entry_initialization.is_some()).unwrap();
    for variant in 0..4 {
        let mut bad = m.clone();
        match variant {
            0 => bad.analyses.stdlib_absorption[entry].entry_group = Some(0),
            1 => bad.analyses.stdlib_absorption[entry].entry_bus = Some("log.**".to_string()),
            2 => bad.analyses.stdlib_absorption[entry].entry_initialization.as_mut().unwrap().field = "wrong".to_string(),
            _ => bad.analyses.stdlib_absorption[entry].entry_initialization = None,
        }
        assert!(bad.validate().is_err(), "variant {variant}");
    }
}

#[test]
fn appended_runtime_defaults_keep_their_real_helper_graph() {
    let source = r#"
locus Handler { fn serve(c: std::io::tcp::Accepted) { } }
main locus App {
    fn construct() { let c = std::io::tcp::Connections { handler: Handler { } }; }
}
fn main() { App { }; }
"#;
    let s = snapshot(source); let m = s.demand_model().unwrap(); m.validate().unwrap();
    let route = m.entities.functions.iter().position(|f| f.name == "__StdIoTcpConnections::params::route").unwrap();
    let route = FunctionId(route as u32);
    assert_eq!(m.entities.functions[route.index()].initializer.as_ref().unwrap().field, "route");
    assert!(m.relations.initializes.iter().any(|e| e.from == function(m, "App::construct") && e.to == route));
    assert!(m.relations.calls.iter().any(|e| e.from == route && m.entities.functions[e.to.index()].name == "__tcp_route"));
    let helper = function(m, "__tcp_route");
    assert!(m.entities.functions[helper.index()].direct_effects.contains(&"alloc".to_string()));
    let gaps: Vec<_> = m.holes.iter().filter(|h| h.kind == hale_model::HoleKind::UnmodeledInitialization).collect();
    assert!(!gaps.iter().any(|h| matches!(h.at, hale_model::EntityRef::Function(f) if f == route || f == helper)),
        "the route default and its real helper remain fully represented");
    let parent_gaps: Vec<_> = gaps.iter().filter(|h| h.reason.starts_with("parent ")).collect();
    assert_eq!(parent_gaps.len(), 2, "exactly the flow owner's two bookends are new");
    let serve = function(m, "__StdIoTcpFlows::serve");
    for (stage, gap) in ["accept", "release"].into_iter().zip(parent_gaps) {
        assert_eq!(gap.at, hale_model::EntityRef::Function(serve));
        assert!(gap.reason.starts_with(&format!("parent {stage} protocol `__StdIoTcpFlows::{stage}`;")));
    }
    assert!(gaps.iter().filter(|h| !h.reason.starts_with("parent "))
        .all(|h| h.reason.contains("birth check") || h.reason.contains("lifecycle")),
        "other appended runtime bodies retain their predicate and lifecycle invocation boundaries");
}

//! Argument defaults execute at named call sites before the callee body.
#[path = "support/entries.rs"]
mod entries;
use hale_frontend::snapshot::{Config, Snapshot};
use hale_model::InitializerKind;
use hale_types::verdict::Verdict;

const SOURCE: &str = r#"
fn observe() -> Int { println("argument default ran"); return 1; }
fn calculate(n: Int = observe()) -> Int { return n; }
main locus App {
    @effects(none: {stdio}) fn omitted() { let n = calculate(); }
    @effects(none: {stdio}) fn supplied() { let n = calculate(4); }
    run() { }
}
fn main() { App { }; }
"#;
fn snapshot(source: &str) -> Snapshot {
    entries::load_files(&[("main.hl", source)], Config::check(true, false))
}

#[test]
fn omitted_arguments_execute_without_poisoning_supplied_calls_or_the_callee_body() {
    let s = snapshot(SOURCE); let report = s.demand_effect_certificates().unwrap();
    for (name, expected) in [("App::omitted", Verdict::Violated), ("App::supplied", Verdict::Holds)] {
        assert_eq!(report.iter().find(|(r,_)| r.subject == name).unwrap().0.result, expected, "{name}");
    }
    let summary = s.demand_alloc_summary().unwrap();
    let row = |name| summary.fns.values().find(|f| f.key.display() == name).unwrap();
    assert!(row("calculate").initializations.is_empty(), "defaults run at the caller, not inside the supplied callee body");
    assert_eq!(row("App::omitted").initializations.len(), 1);
    assert!(row("App::supplied").initializations.is_empty());
    let default = &row("App::omitted").initializations[0];
    assert_eq!(default.initializer.kind, InitializerKind::Argument);
    assert_eq!(default.initializer.field, "n");
    assert_eq!(default.target.display(), "calculate::args::n");
    assert_eq!(row("App::omitted").calls.len(), 1, "the default is no extra authored call");
    assert!(row("calculate::args::n").effect_sites.is_empty(), "helper effects remain at the helper");
    assert_eq!(row("calculate::args::n").stack_bytes, 0, "an inline argument adds no native call frame");
}

#[test]
fn concrete_method_defaults_follow_the_same_call_account() {
    let source = SOURCE.replace("fn calculate(n: Int = observe()) -> Int { return n; }",
        "locus Calculator { fn calculate(n: Int = observe()) -> Int { return n; } }")
        .replace("let n = calculate(", "let service = Calculator { }; let n = service.calculate(");
    let s = snapshot(&source);
    for (name, expected) in [("App::omitted", Verdict::Violated), ("App::supplied", Verdict::Holds)] {
        assert_eq!(s.demand_effect_certificates().unwrap().iter().find(|(r,_)| r.subject == name).unwrap().0.result, expected);
    }
    let summary = s.demand_alloc_summary().unwrap();
    assert!(summary.initializers.iter().any(|(key, d)| key.display() == "Calculator::calculate::args::n" && d.kind == InitializerKind::Argument));
    let mode=source.replace("fn calculate(n:","mode bulk(n:").replace("service.calculate(","service.bulk(");
    let s=snapshot(&mode);let m=s.demand_model().unwrap();m.validate().unwrap();
    for (name,expected) in [("App::omitted",Verdict::Violated),("App::supplied",Verdict::Holds)] {
        assert_eq!(s.demand_effect_certificates().unwrap().iter().find(|(r,_)| r.subject == name).unwrap().0.result,expected);
    }
}

#[test]
fn canonical_argument_defaults_keep_the_callable_owner_and_real_call_identity() {
    let s = snapshot(SOURCE); let m = s.demand_model().unwrap(); m.validate().unwrap();
    let id = |name| hale_model::FunctionId(m.entities.functions.iter().position(|f| f.name == name).unwrap() as u32);
    let default = &m.entities.functions[id("calculate::args::n").index()];
    assert_eq!(default.kind,hale_model::FunctionKind::ArgumentDefault);
    assert_eq!(default.argument_owner,Some(id("calculate")));
    assert!(default.owner.is_none() && default.type_owner.is_none());
    assert_eq!(s.identities().site(default.decl.unwrap()).unwrap().kind,hale_syntax::sites::SiteKind::Binder);
    let omitted = id("App::omitted");let supplied = id("App::supplied");
    let edge = m.relations.argument_evaluations.iter().find(|r| r.from == omitted).unwrap();
    assert_eq!((edge.to,edge.callee,edge.argument),(id("calculate::args::n"),id("calculate"),0));
    let call = m.relations.calls.iter().find(|c| c.from == omitted && c.to == edge.callee).unwrap();
    assert_eq!((call.site,call.provenance,call.supplied),(edge.site,edge.provenance,Some(0)));
    assert!(!m.relations.argument_evaluations.iter().any(|r| r.from == supplied || r.from == id("calculate")));
    assert!(!m.relations.calls.iter().any(|c| c.from == omitted && c.to == edge.to));
    assert!(!m.relations.initializes.iter().any(|r| r.to == edge.to));
    for variant in 0..6 {
        let mut changed=m.clone();
        match variant {
            0 => changed.relations.argument_evaluations.clear(),
            1 => changed.entities.functions[edge.to.index()].argument_owner=None,
            2 => changed.relations.argument_evaluations.iter_mut().find(|r| r.from == omitted).unwrap().site=99,
            3 => changed.relations.calls.iter_mut().find(|c| c.from == omitted && c.to == edge.callee).unwrap().supplied=Some(1),
            4 => changed.entities.functions[edge.to.index()].owner=Some(hale_model::LocusDeclId(0)),
            _ => changed.relations.argument_evaluations.iter_mut().find(|r| r.from == omitted).unwrap().parameter="wrong".into(),
        }
        assert!(changed.validate_coverage().is_err(),"variant {variant}");
        assert!(changed.validate().is_err(),"variant {variant}");
    }
}

#[test]
fn graph_claims_retain_the_helper_barrier_across_argument_evaluation() {
    let source = SOURCE.replace("main locus App", "group apps = { App }; group observers = { observe }; group callees = { calculate }; main locus App")
        .replace("run() { }", "run() { } claims { quiet: forbid reaches(apps, effects(stdio)); guarded: forbid reaches(apps, effects(stdio)) avoiding observers; before_body: forbid reaches(apps, effects(stdio)) avoiding callees; }");
    let s=snapshot(&source);let m=s.demand_model().unwrap();m.validate().unwrap();
    let table=hale_types::claim_lowering::lower_claims(&s.bundle(),m);
    let (pre,rows,proofs)=hale_types::judgment::judge_forbid_reaches_with_certificates(&table,m,&[0]);assert!(pre.is_empty());
    for (name,expected) in [("quiet",Verdict::Violated),("guarded",Verdict::Holds),("before_body",Verdict::Violated)] {
        let ordinal=table.rows.iter().find(|r| r.name == name).unwrap().ordinal;
        assert_eq!(rows.iter().find(|r| r.ordinal == ordinal).unwrap().verdict,expected);
        proofs[&ordinal].validate().unwrap();
    }
}

fn budget(source:&str) -> Verdict {
    let s=snapshot(source);let m=s.demand_model().unwrap();let bundle=s.bundle();
    let table=hale_types::claim_lowering::lower_claims(&bundle,m);
    let evidence=hale_types::evidence::derive_certificate_evidence_over(&bundle,&table,m,s.demand_effect_certificates().unwrap(),s.demand_alloc_summary().unwrap());
    let ordinal=table.rows.iter().find(|r| matches!(r.law,hale_model::ClaimIr::AllocBudget { .. } | hale_model::ClaimIr::QuantBudget { .. })).unwrap().ordinal;
    hale_types::judgment::judge_certificates(&table,m,&evidence,&[0]).into_iter().find(|r| r.ordinal == ordinal).unwrap().verdict
}

#[test]
fn omitted_arguments_sum_per_parameter_and_per_call() {
    let source=SOURCE.replace("println(\"argument default ran\");","std::time::sleep(5ms);")
        .replace("n: Int = observe())", "n: Int = observe(), m: Int = observe())")
        .replace("@effects(none: {stdio}) fn omitted", "@budget(block_points = 1) fn omitted");
    assert_eq!(budget(&source),Verdict::Violated);
    assert_eq!(budget(&source.replace("let n = calculate();","let n = calculate(4);")),Verdict::Holds);
    let repeated=source.replace("block_points = 1","block_points = 3").replace("let n = calculate();","let n = calculate(); let m = calculate();");
    assert_eq!(budget(&repeated),Verdict::Violated,"two calls evaluate four defaults");
}

#[test]
fn argument_local_storage_counts_without_fabricating_a_native_frame() {
    let source=SOURCE.replace("n: Int = observe()", "n: Int = if true { let values: [Int; 64] = [0; 64]; 0 } else { 0 }");
    for (dimension,limit) in [("alloc_per_call",0),("stack_bytes",256)] {
        let tested=source.replace("@effects(none: {stdio}) fn omitted",&format!("@budget({dimension} = {limit}) fn omitted"));
        assert_eq!(budget(&tested),Verdict::Violated,"{dimension}");
        assert_eq!(budget(&tested.replace("let n = calculate();","let n = calculate(4);")),Verdict::Holds,"{dimension}");
    }
}

#[test]
fn imported_callable_defaults_retain_the_declaring_function_and_seed() {
    let main=r#"
import "./lib" as lib;
group imported = { lib::* };
main locus App {
    @effects(none: {stdio}) fn omitted() { let n = lib::calculate(); }
    @effects(none: {stdio}) fn supplied() { let n = lib::calculate(4); }
}
fn main() { App { }; }
"#;
    let library="fn observe() -> Int { println(1); return 1; } fn calculate(n: Int = observe()) -> Int { return n; }";
    let s=entries::load_files(&[("main.hl",main),("lib/math.hl",library)],Config::check(true,false));
    let m=s.demand_model().unwrap();m.validate().unwrap();
    let default=hale_model::FunctionId(m.entities.functions.iter().position(|f| f.display == "lib::calculate::args::n").unwrap() as u32);
    let parent=m.entities.functions[default.index()].argument_owner.unwrap();
    assert_eq!(m.entities.functions[parent.index()].display,"lib::calculate");
    let imported=hale_model::GroupId(m.entities.groups.iter().position(|g| g.name == "imported").unwrap() as u32);
    assert!(m.relations.group_members.iter().any(|r| r.group == imported && r.member == hale_model::EntityRef::Function(default)));
    for (name,expected) in [("App::omitted",Verdict::Violated),("App::supplied",Verdict::Holds)] {
        assert_eq!(s.demand_effect_certificates().unwrap().iter().find(|(r,_)| r.subject == name).unwrap().0.result,expected);
    }
}

#[test]
fn the_callee_body_and_fully_supplied_function_value_do_not_evaluate_defaults() {
    let source=SOURCE.replace("fn calculate(n:","@effects(none: {stdio}) fn calculate(n:")
        .replace("let n = calculate(4);","let f = calculate; let n = f(4);");
    let s=snapshot(&source);let reports=s.demand_effect_certificates().unwrap();
    for (name,expected) in [("calculate",Verdict::Holds),("App::omitted",Verdict::Violated),("App::supplied",Verdict::Holds)] {
        assert_eq!(reports.iter().find(|(r,_)| r.subject == name).unwrap().0.result,expected,"{name}");
    }
    let m=s.demand_model().unwrap();m.validate().unwrap();
    let supplied=hale_model::FunctionId(m.entities.functions.iter().position(|f| f.name == "App::supplied").unwrap() as u32);
    assert!(!m.relations.argument_evaluations.iter().any(|e| e.from == supplied));
}

#[test]
fn default_bodies_retain_their_declaring_generic_scope() {
    let source=r#"
import "./lib" as lib;
fn calculate<T>(sample: T, n: Int = T { }.read()) -> Int { return n; }
main locus App { claims { quiet: forbid reaches(program, effects(stdio)); } }
fn main() { App { }; }
"#;
    let library="locus Loud { fn read() -> Int { println(1); return 1; } }";
    let s=entries::load_files(&[("main.hl",source),("lib/loud.hl",library)],Config::check(true,false));
    let m=s.demand_model().unwrap();m.validate().unwrap();
    let table=hale_types::claim_lowering::lower_claims(&s.bundle(),m);
    let (_,rows,proofs)=hale_types::judgment::judge_forbid_reaches_with_certificates(&table,m,&[0]);
    assert_eq!(rows[0].verdict,Verdict::Uncertified,"a generic receiver cannot become a classified empty leaf");
    assert!(proofs.is_empty());
}

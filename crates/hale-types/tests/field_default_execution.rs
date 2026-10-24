//! Data defaults are executable type-owned bodies, not free or locus methods.
#[path = "support/entries.rs"]
mod entries;
use hale_frontend::snapshot::{Config, Snapshot};
use hale_model::{ConstructorTarget, FunctionId, FunctionKind, InitializerKind};
use hale_types::verdict::Verdict;

const SOURCE: &str = r#"
fn observe() -> Int { println("field default ran"); return 1; }
type Settings { n: Int = observe(); }
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
fn snapshot(source: &str) -> Snapshot { entries::load_files(&[("app.hl", source)], Config::check(true, false)) }
fn function(model: &hale_model::ApplicationModel, name: &str) -> FunctionId {
    FunctionId(model.entities.functions.iter().position(|f| f.name == name).unwrap() as u32)
}
fn budget(source: &str) -> Verdict {
    let s = snapshot(source); let m = s.demand_model().unwrap(); let bundle = s.bundle();
    let table = hale_types::claim_lowering::lower_claims(&bundle, m);
    let evidence = hale_types::evidence::derive_certificate_evidence_over(&bundle, &table, m,
        s.demand_effect_certificates().unwrap(), s.demand_alloc_summary().unwrap());
    let ordinal = table.rows.iter().find(|r| matches!(r.law, hale_model::ClaimIr::AllocBudget { .. } | hale_model::ClaimIr::QuantBudget { .. })).unwrap().ordinal;
    hale_types::judgment::judge_certificates(&table, m, &evidence, &[0]).into_iter().find(|r| r.ordinal == ordinal).unwrap().verdict
}

#[test]
fn data_default_execution_is_owned_by_its_type_and_keeps_helper_effects_separate() {
    let s = snapshot(SOURCE); let m = s.demand_model().unwrap(); m.validate().unwrap();
    let reports = s.demand_effect_certificates().unwrap();
    for (name, expected) in [("App::omitted", Verdict::Violated), ("App::supplied", Verdict::Holds)] {
        assert_eq!(reports.iter().find(|(r,_)| r.subject == name).unwrap().0.result, expected, "{name}");
    }
    let id = function(m, "Settings::fields::n"); let f = &m.entities.functions[id.index()];
    assert_eq!(f.kind, FunctionKind::FieldDefault);
    assert_eq!(f.initializer.as_ref().unwrap().kind, InitializerKind::Field);
    let owner = f.type_owner.unwrap(); assert_eq!(m.entities.types[owner.index()].name, "Settings");
    assert!(f.owner.is_none() && f.direct_effects.is_empty());
    assert!(!m.relations.member_of.iter().any(|r| r.function == id));
    assert!(!m.entities.loci.iter().any(|l| l.name == "Settings"));
    assert!(m.relations.initializes.iter().any(|r| r.from == function(m, "App::omitted") && r.to == id && r.owner == ConstructorTarget::Type(owner)));
    assert!(!m.relations.initializes.iter().any(|r| r.from == function(m, "App::supplied")));
    assert!(m.relations.calls.iter().any(|r| r.from == id && r.to == function(m, "observe")));
    let site = s.identities().site(f.decl.unwrap()).unwrap();
    assert_eq!(site.kind, hale_syntax::sites::SiteKind::Field);
}

#[test]
fn data_default_graph_paths_preserve_the_helper_barrier_and_finite_certificates() {
    let s = snapshot(SOURCE); let m = s.demand_model().unwrap(); let table = hale_types::claim_lowering::lower_claims(&s.bundle(), m);
    let (pre, rows, proofs) = hale_types::judgment::judge_forbid_reaches_with_certificates(&table, m, &[0]);
    assert!(pre.is_empty());
    for (name, expected) in [("quiet", Verdict::Violated), ("guarded", Verdict::Holds)] {
        let ordinal = table.rows.iter().find(|r| r.name == name).unwrap().ordinal;
        assert_eq!(rows.iter().find(|r| r.ordinal == ordinal).unwrap().verdict, expected);
        proofs[&ordinal].validate().unwrap();
    }
}

#[test]
fn pure_data_defaults_keep_closed_results_and_do_not_spawn_a_locus() {
    let source = SOURCE.replace("println(\"field default ran\"); ", "")
        .replace("@effects(none: {stdio})", "@effects(none: {stdio, spawn})");
    let s = snapshot(&source); let m = s.demand_model().unwrap(); m.validate().unwrap();
    assert!(s.demand_effect_certificates().unwrap().iter().all(|(r,_)| r.result == Verdict::Holds));
    let summary = s.demand_alloc_summary().unwrap();
    let body = summary.fns.values().find(|f| f.key.display() == "App::omitted").unwrap();
    assert!(!body.effect_sites.iter().any(|s| matches!(s.kind, hale_types::alloc_summary::EffectSiteKind::Spawn(_))));
    let literal = SOURCE.replace("n: Int = observe();", "n: Int = 0;");
    let s = snapshot(&literal); let summary = s.demand_alloc_summary().unwrap();
    assert_eq!(summary.fns.values().find(|f| f.key.display() == "Settings::fields::n").unwrap().stack_bytes, 0, "inline literals invent no native frame");
}

#[test]
fn each_omitted_data_field_contributes_to_the_budget_in_declaration_order() {
    let source = SOURCE.replace("println(\"field default ran\");", "std::time::sleep(5ms);")
        .replace("n: Int = observe();", "z: Int = observe(); n: Int = observe();")
        .replace("@effects(none: {stdio}) fn omitted", "@budget(block_points = 1) fn omitted");
    assert_eq!(budget(&source), Verdict::Violated);
    let supplied = source.replace("fn omitted() { let settings = Settings { }; }", "fn omitted() { let settings = Settings { n: 4 }; }");
    assert_eq!(budget(&supplied), Verdict::Holds);
    let s = snapshot(&source); let m = s.demand_model().unwrap();
    let order: Vec<_> = m.relations.initializes.iter().filter(|r| r.from == function(m,"App::omitted")).map(|r| (r.field.as_str(),r.member)).collect();
    assert_eq!(order, [("z",0),("n",1)]);
}

#[test]
fn allocation_and_inline_local_costs_of_data_defaults_reach_the_caller() {
    let source = SOURCE.replace("n: Int = observe();", "n: Int = if true { let values: [Int; 64] = [0; 64]; 0 } else { 0 };");
    for (dimension, limit) in [("alloc_per_call",1),("stack_bytes",256)] {
        let tested = source.replace("@effects(none: {stdio}) fn omitted", &format!("@budget({dimension} = {limit}) fn omitted"));
        assert_eq!(budget(&tested), Verdict::Violated, "{dimension}");
        let supplied = tested.replace("fn omitted() { let settings = Settings { }; }", "fn omitted() { let settings = Settings { n: 4 }; }");
        assert_eq!(budget(&supplied), Verdict::Holds, "{dimension}");
    }
}

#[test]
fn data_omission_and_owner_metadata_are_validated_for_model_and_evidence() {
    let s = snapshot(SOURCE); let model = s.demand_model().unwrap(); let field = function(model,"Settings::fields::n");
    for variant in 0..5 {
        let mut m = model.clone();
        match variant {
            0 => m.entities.functions[field.index()].type_owner = None,
            1 => m.entities.functions[field.index()].initializer.as_mut().unwrap().kind = InitializerKind::Parameter,
            2 => m.entities.functions[field.index()].owner = Some(hale_model::LocusDeclId(0)),
            3 => m.relations.initializes.clear(),
            _ => { m.relations.constructions.clear(); }
        }
        assert!(m.validate_coverage().is_err(), "variant {variant}");
        assert!(m.validate().is_err(), "variant {variant}");
    }
}

#[test]
fn program_roots_include_unused_type_default_code_without_poisoning_other_functions() {
    let source = r#"
type Settings { n: Int = if true { println("declared field body"); 0 } else { 0 }; }
main locus App {
    @effects(none: {stdio}) fn unrelated() { }
    claims { quiet: forbid reaches(program, effects(stdio)); }
}
fn main() { App { }; }
"#;
    let s = snapshot(source); let m = s.demand_model().unwrap(); m.validate().unwrap();
    assert!(s.demand_effect_certificates().unwrap().iter().all(|(r,_)| r.result == Verdict::Holds));
    let table = hale_types::claim_lowering::lower_claims(&s.bundle(),m);
    let (_, rows, proofs) = hale_types::judgment::judge_forbid_reaches_with_certificates(&table,m,&[0]);
    assert_eq!(rows[0].verdict,Verdict::Violated);
    proofs[&table.rows[0].ordinal].validate().unwrap();
}

#[test]
fn nested_record_and_locus_defaults_share_execution_without_flattening_helpers() {
    let source = SOURCE.replace("type Settings { n: Int = observe(); }", r#"
type Inner { n: Int = observe(); }
type Outer { inner: Inner = Inner { }; }
locus Settings { params { outer: Outer = Outer { }; } }
"#).replace("Settings { n: 4 }", "Settings { outer: Outer { inner: Inner { n: 4 } } }");
    let s = snapshot(&source); let m = s.demand_model().unwrap(); m.validate().unwrap();
    for (name, expected) in [("App::omitted", Verdict::Violated), ("App::supplied", Verdict::Holds)] {
        assert_eq!(s.demand_effect_certificates().unwrap().iter().find(|(r,_)| r.subject == name).unwrap().0.result, expected);
    }
    for (from, to) in [("App::omitted", "Settings::params::outer"), ("Settings::params::outer", "Outer::fields::inner"), ("Outer::fields::inner", "Inner::fields::n")] {
        assert!(m.relations.initializes.iter().any(|r| r.from == function(m,from) && r.to == function(m,to)), "{from} -> {to}");
    }
    let table = hale_types::claim_lowering::lower_claims(&s.bundle(),m);
    let (_, rows, proofs) = hale_types::judgment::judge_forbid_reaches_with_certificates(&table,m,&[0]);
    for (name, expected) in [("quiet",Verdict::Violated),("guarded",Verdict::Holds)] {
        let ordinal = table.rows.iter().find(|r| r.name == name).unwrap().ordinal;
        assert_eq!(rows.iter().find(|r| r.ordinal == ordinal).unwrap().verdict,expected);
        proofs[&ordinal].validate().unwrap();
    }
}

#[test]
fn imported_record_aliases_keep_real_owners_and_do_not_enter_program_roots() {
    let main = r#"
import "./lib" as lib;
type Local = lib::Settings;
group imported = { lib::* };
main locus App {
    @effects(none: {stdio}) fn omitted() { let settings = Local { }; }
    @effects(none: {stdio}) fn supplied() { let settings = Local { n: 4 }; }
    claims { imports_visible: forbid reaches(program, effects(stdio)); }
}
fn main() { App { }; }
"#;
    let library = r#"type Settings { n: Int = if true { println("library field"); 0 } else { 0 }; }"#;
    let s = entries::load_files(&[("main.hl",main),("lib/settings.hl",library)],Config::check(true,false));
    let m = s.demand_model().unwrap(); m.validate().unwrap();
    let id = FunctionId(m.entities.functions.iter().position(|f| f.display == "lib::Settings::fields::n").unwrap() as u32);
    let owner = m.entities.functions[id.index()].type_owner.unwrap();
    assert_eq!(m.entities.types[owner.index()].display,"lib::Settings");
    assert!(m.relations.initializes.iter().any(|r| r.from == function(m,"App::omitted") && r.to == id));
    assert!(!m.relations.initializes.iter().any(|r| r.from == function(m,"App::supplied")));
    for (name, expected) in [("App::omitted",Verdict::Violated),("App::supplied",Verdict::Holds)] {
        assert_eq!(s.demand_effect_certificates().unwrap().iter().find(|(r,_)| r.subject == name).unwrap().0.result,expected);
    }
    let program = hale_model::GroupId(m.entities.groups.iter().position(|g| g.name == "program").unwrap() as u32);
    let imported = hale_model::GroupId(m.entities.groups.iter().position(|g| g.name == "imported").unwrap() as u32);
    assert!(!m.relations.group_members.iter().any(|r| r.group == program && r.member == hale_model::EntityRef::Function(id)));
    assert!(m.relations.group_members.iter().any(|r| r.group == imported && r.member == hale_model::EntityRef::Function(id)));
}

#[test]
fn explicit_failure_inside_a_data_default_withdraws_certification_only_where_evaluated() {
    let child = r#"
locus Child {
    params { n: Int = 0; }
    closure broken { captures: n; epoch inline; }
    fn break_it() { self.n = 1; violate broken; }
}
"#;
    let source = format!("{child}{}",SOURCE.replace("println(\"field default ran\");", "let child = Child { }; child.break_it();"));
    let s = snapshot(&source); let m = s.demand_model().unwrap(); m.validate().unwrap();
    for (name, expected) in [("App::omitted",Verdict::Uncertified),("App::supplied",Verdict::Holds)] {
        assert_eq!(s.demand_effect_certificates().unwrap().iter().find(|(r,_)| r.subject == name).unwrap().0.result,expected);
    }
    assert!(m.holes.iter().any(|h| h.at == hale_model::EntityRef::Function(function(m,"App::omitted")) && h.kind == hale_model::HoleKind::UnmodeledFailureDispatch));
    assert!(!m.holes.iter().any(|h| h.at == hale_model::EntityRef::Function(function(m,"App::supplied")) && h.kind == hale_model::HoleKind::UnmodeledFailureDispatch));
}

//! Constructor lifecycle entries preserve body identity and invocation coverage.
#[path = "support/entries.rs"]
mod entries;
use hale_frontend::snapshot::{Config, Snapshot};
use hale_types::verdict::Verdict;
use hale_model::{EntityRef, HoleKind, LifecycleStage};
const SOURCE: &str = r#"
fn observe() { println("hook ran"); }
locus Child { birth() { observe(); } }
@effects(none: {stdio}) fn construct() { Child { }; }
fn main() { construct(); }
"#;
fn snapshot(source: &str) -> Snapshot {
    entries::load_files(&[("main.hl",source)],Config::check(true,false))
}
#[test]
fn constructor_effects_enter_the_actual_birth_body() {
    let s=snapshot(SOURCE);
    let reports=s.demand_effect_certificates().unwrap();
    let report=reports.iter().find(|(r,_)| r.subject=="construct").unwrap();
    assert_eq!(report.0.result,Verdict::Violated);
    assert!(report.1.iter().any(|(d,_)| d.message.contains("Child::birth")));
    let m=s.demand_model().unwrap();m.validate().unwrap();
    let evaluation=m.relations.lifecycle_evaluations.iter().find(|v| m.entities.functions[v.from.index()].display=="construct").unwrap();
    assert_eq!(evaluation.stage,LifecycleStage::Birth);
    assert_eq!(evaluation.guard,hale_model::LifecycleGuard::Construction);
    let contexts=hale_types::model_query::lifecycle_entry_contexts(m,evaluation.from,evaluation.site,evaluation.stage);
    assert!(contexts.inventory_available && contexts.source_available && !contexts.contexts.is_empty());
    assert_eq!(m.entities.functions[evaluation.to.index()].display,"Child::birth");
    assert!(m.holes.iter().any(|h| h.at==EntityRef::Function(evaluation.from) && h.kind==HoleKind::UnmodeledInitialization && h.provenance==evaluation.provenance));
}

#[test]
fn unminted_analysis_retains_hook_effects_but_cannot_supply_native_identities() {
    let program = hale_syntax::parse_source(SOURCE).unwrap();
    let bundle = hale_types::Bundle::new([("app.hl".to_string(), &program)].into_iter().collect());
    let summary = hale_types::alloc_summary::derive_alloc_summary(&bundle);
    assert!(summary.lifecycle_hooks.keys().any(|owner|
        owner.name == "Child" && owner.declaration.is_none()));
    let caller = summary.fns.values().find(|f| f.key.display() == "construct").unwrap();
    assert_eq!(caller.lifecycle_evaluations.len(), 1);
    assert!(caller.lifecycle_evaluations[0].target.decl.is_none());
    let reports = hale_types::effects::certificate_rows(&[&program], &summary);
    assert_eq!(reports.iter().find(|r| r.subject == "construct").unwrap().result, Verdict::Violated);
    assert!(hale_types::lifecycle_hooks::lower(&summary.lifecycle_hooks, |_| None).is_err());
}
#[test]
fn graph_claims_and_certificates_preserve_the_constructor_hook_boundary() {
    let source=SOURCE.replace("@effects(none: {stdio}) ","").replace("fn main() { construct(); }",r#"
group constructors = { construct };
main locus App { claims { quiet: forbid reaches(constructors, effects(stdio)); } }
fn main() { construct(); App { }; }
"#);
    let s=snapshot(&source);let m=s.demand_model().unwrap();m.validate().unwrap();
    let table=hale_types::claim_lowering::lower_claims(&s.bundle(),m);
    let (pre,rows,proofs)=hale_types::judgment::judge_forbid_reaches_with_certificates(&table,m,&[0]);
    assert!(pre.is_empty(),"{pre:?}");assert_eq!(rows.len(),1);assert_eq!(rows[0].verdict,Verdict::Violated);
    assert!(rows[0].diags.iter().any(|d| d.message.contains("may enter birth; guard construction") && d.message.contains("Child::birth")),"{:?}",rows[0].diags);
    proofs[&rows[0].ordinal].validate().unwrap();
}
#[test]
fn lifecycle_entries_keep_the_constructor_namespace_and_loop_facts() {
    let source=SOURCE.replace("@effects(none: {stdio}) fn construct() { Child { }; }",r#"
type Marker { }
fn construct() { let marker = Marker { }; while false { Child { }; } }
"#);
    let s=snapshot(&source);let m=s.demand_model().unwrap();m.validate().unwrap();
    let v=m.relations.lifecycle_evaluations.iter().find(|v| m.entities.functions[v.from.index()].display=="construct").unwrap();
    assert_eq!(v.site,1);assert!(v.in_loop && v.unbounded);
    let summary=s.demand_alloc_summary().unwrap();let caller=summary.fns.values().find(|f| f.key.display()=="construct").unwrap();
    assert_eq!(caller.lifecycle_evaluations.len(),1);
    assert!(caller.calls.is_empty(),"lifecycle entry is not an authored call");
    assert_eq!(caller.body_evaluations().count(),0,"an unresolved lifecycle frame is not a default/checkpoint frame");
}
#[test]
fn definitions_entries_and_context_coverage_are_mandatory() {
    let s=snapshot(SOURCE);let m=s.demand_model().unwrap();m.validate().unwrap();
    for variant in 0..9 {
        let mut changed=m.clone();
        match variant {
            0=>changed.relations.lifecycle_definitions.clear(),
            1=>changed.relations.lifecycle_evaluations.clear(),
            2=>changed.relations.lifecycle_evaluations[0].stage=LifecycleStage::Run,
            3=>changed.relations.lifecycle_evaluations[0].site+=1,
            4=>changed.relations.lifecycle_definitions[0].empty=true,
            5=>changed.holes.retain(|h| h.kind!=HoleKind::UnmodeledInitialization),
            6=>changed.relations.lifecycle_evaluations[0].in_loop=true,
            7=>changed.relations.lifecycle_evaluations[0].guard=hale_model::LifecycleGuard::Teardown,
            _=>changed.relations.lifecycle_evaluations[0].source=None,
        }
        assert!(changed.validate().is_err(),"variant {variant}");
    }
}
#[test]
fn later_stages_expose_known_work_and_retain_context_uncertainty() {
    for stage in ["run","drain","dissolve"] {
        let source=SOURCE.replace("birth()",&format!("{stage}()"));
        let s=snapshot(&source);let report=s.demand_effect_certificates().unwrap().iter().find(|(r,_)| r.subject=="construct").unwrap();
        assert_eq!(report.0.result,Verdict::Violated,"{stage}");
        let m=s.demand_model().unwrap();m.validate().unwrap();
        let edge=m.relations.lifecycle_evaluations.iter().find(|v| v.stage.as_str()==stage).unwrap();
        assert_eq!(edge.guard,edge.stage.entry_guard());
        assert!(edge.source.is_some());
        let contexts=hale_types::model_query::lifecycle_entry_contexts(m,edge.from,edge.site,edge.stage);
        assert!(contexts.inventory_available && contexts.source_available && !contexts.contexts.is_empty());
        assert!(m.holes.iter().any(|h| h.at==EntityRef::Function(edge.from) && h.kind==HoleKind::UnmodeledInitialization));
        assert!(m.relations.lifecycle_definitions.iter().any(|d| d.stage.as_str()==stage && !d.empty));
        let pure=source.replace("println(\"hook ran\");", "");
        let quiet=snapshot(&pure);
        assert_eq!(quiet.demand_effect_certificates().unwrap().iter().find(|(r,_)| r.subject=="construct").unwrap().0.result,Verdict::Uncertified,"{stage}: a pure possible body cannot erase invocation uncertainty");
        let summary=quiet.demand_alloc_summary().unwrap();
        let caller=summary.fns.values().find(|f| f.key.display()=="construct").unwrap();
        assert_eq!(caller.body_evaluations().count(),0,"{stage}: a lifecycle may-entry is not an ordinary frame");
        assert_eq!(caller.frame_targets().count(),0,"{stage}: no native caller frame is established");
    }
}
#[test]
fn empty_and_synthesized_hooks_keep_their_source_facts_without_invented_entry() {
    let source=SOURCE.replace("observe();","");let s=snapshot(&source);
    assert_eq!(s.demand_effect_certificates().unwrap().iter().find(|(r,_)| r.subject=="construct").unwrap().0.result,Verdict::Holds);
    let m=s.demand_model().unwrap();m.validate().unwrap();assert!(m.relations.lifecycle_evaluations.is_empty());
    let summary=s.demand_alloc_summary().unwrap();let hooks:Vec<_>=summary.lifecycle_hooks.values().flatten().filter(|h| h.key.locus.as_deref()==Some("Child")).collect();
    assert!(hooks.iter().any(|h| h.stage==LifecycleStage::Birth && h.empty && !h.synthesized));
    assert!(hooks.iter().any(|h| h.stage==LifecycleStage::Run && h.empty && h.synthesized));
}

#[test]
fn stdlib_birth_entry_retains_its_own_namespace_and_context_gap() {
    let source=r#"
@effects(none: {stdio}) fn construct() { let builder = std::bytes::BytesBuilder { }; }
fn main() { construct(); }
"#;
    let s=snapshot(source);let m=s.demand_model().unwrap();m.validate().unwrap();
    let absorption=m.analyses.stdlib_absorption.iter().find(|a| a.entry_lifecycle==Some(LifecycleStage::Birth)).unwrap();
    assert_eq!(m.entities.functions[absorption.from.index()].display,"construct");
    assert!(absorption.entry_initialization.is_none() && absorption.entry_dispatch.is_none() && absorption.entry_group.is_none());
    assert_eq!(absorption.nodes[0].lifecycle,Some(LifecycleStage::Birth));
    assert!(absorption.nodes[0].display.ends_with("::birth"));
    for variant in 0..3 {
        let mut changed=m.clone();
        let index=changed.analyses.stdlib_absorption.iter().position(|a| a.entry_lifecycle.is_some()).unwrap();
        match variant {
            0=>changed.analyses.stdlib_absorption[index].entry_lifecycle=Some(LifecycleStage::Run),
            1=>changed.analyses.stdlib_absorption[index].nodes[0].lifecycle=None,
            _=>changed.holes.retain(|h| h.kind!=HoleKind::UnmodeledInitialization),
        }
        assert!(changed.validate().is_err(),"variant {variant}");
    }
}
#[test]
fn imported_hooks_keep_their_actual_owner_and_constructor_binding() {
    let main=r#"
import "./lib" as lib;
locus Child { }
fn construct() { Child { }; lib::Child { }; }
fn main() { construct(); }
"#;
    let library="locus Child { birth() { println(\"library hook\"); } }";
    let s=entries::load_files(&[("main.hl",main),("lib/child.hl",library)],Config::check(true,false));
    let m=s.demand_model().unwrap();m.validate().unwrap();
    let d=m.relations.lifecycle_definitions.iter().find(|d| d.stage==LifecycleStage::Birth).unwrap();
    assert_eq!(m.entities.loci[d.owner.index()].display,"lib::Child");
    let v=m.relations.lifecycle_evaluations.iter().find(|v| v.to==d.function).unwrap();
    assert_eq!(v.site,1);assert_eq!(m.entities.functions[v.from.index()].display,"construct");
}

#[test]
fn foreign_interior_lifecycle_gaps_bind_every_source_entry() {
    let source = r#"
main locus App {
    params {
        sink: std::log::StdoutSink = std::log::StdoutSink { };
        log: std::log::Logger = std::log::Logger { name: "app" };
    }
    run() { self.log.info("started"); }
}
fn main() { App { }; }
"#;
    let s = snapshot(source);
    let m = s.demand_model().unwrap();
    m.validate().unwrap();
    let interior = m.analyses.stdlib_absorption.iter().find(|a|
        m.entities.functions[a.from.index()].display == "App::run"
        && a.entry_lifecycle.is_none()
        && a.nodes.iter().any(|n| n.events.iter().any(|e|
            matches!(e, hale_model::AbsorbedEvent::Lifecycle { .. })))).unwrap();
    assert!(m.holes.iter().any(|h| h.at == EntityRef::Function(interior.from)
        && h.kind == HoleKind::UnmodeledInitialization
        && h.reason.contains("foreign self lifecycle invocation")));
    let mut missing = m.clone();
    missing.holes.retain(|h| h.at != EntityRef::Function(interior.from)
        || h.kind != HoleKind::UnmodeledInitialization);
    assert!(missing.validate().is_err(),
        "a gap on another helper cannot cover this source entry");
}

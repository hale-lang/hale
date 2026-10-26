//! Closure assertion bodies execute at declared epochs and explicit checks.
#[path = "support/entries.rs"]
mod entries;
use hale_frontend::snapshot::Config;
use hale_types::verdict::Verdict;

const SOURCE: &str = r#"
fn observe() -> Int { println("closure ran"); return 0; }
locus Child {
    params { n: Int = 0; }
    closure audit { observe() + self.n ~~ self.n within 0; epoch explicit; }
    @effects(none: {stdio}) fn trigger() { check_closures(); }
    @effects(none: {stdio}) fn idle() { }
}
main locus App { run() { let child = Child { }; child.trigger(); } }
fn main() { App { }; }
"#;

#[test]
fn explicit_checks_evaluate_their_own_closure_assertions() {
    let s=entries::load_files(&[("main.hl",SOURCE)],Config::check(true,false));
    let reports=s.demand_effect_certificates().unwrap();
    for (name,expected) in [("Child::trigger",Verdict::Violated),("Child::idle",Verdict::Holds)] {
        assert_eq!(reports.iter().find(|(r,_)| r.subject==name).unwrap().0.result,expected,"{name}");
    }
}

#[test]
fn unminted_checkpoints_keep_their_resolved_owner_and_known_effects() {
    let program = hale_syntax::parse_source(SOURCE).unwrap();
    let bundle = hale_types::Bundle::new([("app.hl".to_string(), &program)].into_iter().collect());
    let summary = hale_types::alloc_summary::derive_alloc_summary(&bundle);
    let trigger = summary.fns.values().find(|f| f.key.display() == "Child::trigger").unwrap();
    assert_eq!(trigger.closure_evaluations.len(), 1);
    let target = &trigger.closure_evaluations[0].target;
    let owner = &summary.closure_assertions[target].owner;
    assert_eq!(owner.name, "Child");
    assert!(owner.declaration.is_none());
    let reports = hale_types::effects::certificate_rows(&[&program], &summary);
    assert_eq!(reports.iter().find(|r| r.subject == "Child::trigger").unwrap().result, Verdict::Violated);
    assert_eq!(reports.iter().find(|r| r.subject == "Child::idle").unwrap().result, Verdict::Holds);
}


#[test]
fn assertion_bodies_keep_declaration_identity_and_exact_checkpoint_membership() {
    let s=entries::load_files(&[("main.hl",SOURCE)],Config::check(true,false));
    let m=s.demand_model().unwrap();m.validate().unwrap();
    let d=m.relations.closure_definitions.iter().find(|d| d.name=="audit").unwrap();
    let f=&m.entities.functions[d.function.index()];
    assert_eq!(f.kind,hale_model::FunctionKind::ClosureAssertion);
    assert_eq!(f.name,"Child::closures::audit");
    assert_eq!(s.identities().site(f.decl.unwrap()).unwrap().kind,hale_syntax::sites::SiteKind::Closure);
    assert_eq!(d.epoch,hale_model::ClosureEpoch::Explicit);
    let trigger=hale_model::FunctionId(m.entities.functions.iter().position(|f| f.name=="Child::trigger").unwrap() as u32);
    let check=m.relations.closure_checks.iter().find(|c| c.function==trigger).unwrap();
    assert_eq!(s.identities().site(check.source).unwrap().kind,hale_syntax::sites::SiteKind::Call);
    let edge=m.relations.closure_evaluations.iter().find(|c| c.from==trigger).unwrap();
    assert_eq!((edge.to,edge.site,edge.provenance),(d.function,check.site,check.provenance));
    for mutation in 0..6 {
        let mut changed=m.clone();
        match mutation {
            0=>changed.relations.closure_evaluations.clear(),
            1=>changed.relations.closure_checks.clear(),
            2=>changed.relations.closure_definitions.clear(),
            3=>changed.relations.closure_evaluations[0].closure+=1,
            4=>changed.relations.closure_evaluations[0].in_loop=true,
            _=>changed.relations.closure_definitions[0].epoch=hale_model::ClosureEpoch::Birth,
        }
        assert!(changed.validate().is_err(),"mutation {mutation}");
    }
}

#[test]
fn graph_claims_and_proof_masks_follow_the_actual_assertion_path() {
    let source=SOURCE.replace("    @effects(none: {stdio}) fn trigger()", "    fn trigger()")
        .replace("main locus App {", "fn invoke() { let child = Child { }; child.trigger(); } group callers = { invoke }; group assertions = { Child }; main locus App { claims { quiet: forbid reaches(callers, effects(stdio)); guarded: forbid reaches(callers, effects(stdio)) avoiding assertions; }");
    let s=entries::load_files(&[("main.hl",&source)],Config::check(true,false));
    let m=s.demand_model().unwrap();m.validate().unwrap();
    let laws=hale_types::claim_lowering::lower_claims(&s.bundle(),m);
    let (pre,rows,proofs)=hale_types::judgment::judge_forbid_reaches_with_certificates(&laws,m,&[0,1]);
    assert!(pre.is_empty(),"{pre:?}");
    assert_eq!(rows.iter().map(|r| r.verdict).collect::<Vec<_>>(),vec![Verdict::Violated,Verdict::Holds]);
    assert_eq!(proofs.len(),2);
    assert!(rows[0].diags.iter().any(|d| d.message.contains("checks closure")),"{:?}",rows[0].diags);
}

#[test]
fn repeated_checkpoints_count_both_assertions_and_preserve_loop_context() {
    let source=SOURCE.replace("fn observe() -> Int { println(\"closure ran\"); return 0; }", "fn observe() -> Int { let a = [1; 64]; return a[0] - 1; }")
        .replace("@effects(none: {stdio}) fn trigger() { check_closures(); }", "@budget(alloc_per_call = 0) fn trigger() { while self.n < 1 { check_closures(); self.n = self.n + 1; } }")
        .replace("    @effects(none: {stdio}) fn idle()", "    closure other { self.n ~~ observe() + self.n within 0; epoch explicit; }\n    @effects(none: {stdio}) fn idle()");
    let s=entries::load_files(&[("main.hl",&source)],Config::check(true,false));
    let summary=s.demand_alloc_summary().unwrap();
    let caller=summary.fns.values().find(|f| f.key.display()=="Child::trigger").unwrap();
    assert_eq!(caller.closure_evaluations.len(),2);
    assert!(caller.closure_evaluations.iter().all(|e| e.loop_depth>0 && e.in_unbounded_loop));
    assert_eq!(caller.calls.len(),1,"an assertion evaluation is no additional authored call");
    assert!(!hale_types::budget_check::budget_diags_with_renames(&s.bundle().programs.values().copied().collect::<Vec<_>>(),summary,&[]).is_empty());
}

#[test]
fn other_epochs_retain_their_operands_without_becoming_explicit_checks() {
    for epoch in ["birth","dissolve","tick","duration(10ms)"] {
        let source=SOURCE.replace("epoch explicit",&format!("epoch {epoch}"));
        let s=entries::load_files(&[("main.hl",&source)],Config::check(true,false));
        let summary=s.demand_alloc_summary().unwrap();
        let definition=summary.closure_assertions.iter().find(|(_,d)| d.name=="audit").unwrap();
        assert!(summary.fns[definition.0].calls.iter().any(|c| matches!(&c.callee,hale_types::alloc_summary::Callee::Resolved(f) if f.display()=="observe")));
        assert!(summary.fns.values().find(|f| f.key.display()=="Child::trigger").unwrap().closure_evaluations.is_empty());
    }
}


#[test]
fn a_possible_assertion_failure_does_not_certify_an_empty_execution_leaf() {
    let source=SOURCE.replace("observe() + self.n ~~ self.n", "self.n ~~ 0");
    let s=entries::load_files(&[("main.hl",&source)],Config::check(true,false));
    let m=s.demand_model().unwrap();m.validate().unwrap();
    let f=m.entities.functions.iter().position(|f| f.name=="Child::closures::audit").unwrap();
    assert!(m.holes.iter().any(|h| h.at==hale_model::EntityRef::Function(hale_model::FunctionId(f as u32)) && h.kind==hale_model::HoleKind::UnmodeledFailureContext));
}


#[test]
fn unresolved_assertion_failure_blocks_closed_certificates_at_the_checkpoint_only() {
    let source=SOURCE.replace("observe() + self.n ~~ self.n", "self.n ~~ 0")
        + "\nlocus Dormant { on_failure(c: Child, e: ClosureViolation) { } }\n";
    let s=entries::load_files(&[("main.hl",&source)],Config::check(true,false));
    let m=s.demand_model().unwrap();let bundle=s.bundle();
    let table=hale_types::claim_lowering::lower_claims(&bundle,m);
    let evidence=hale_types::evidence::derive_certificate_evidence_over(&bundle,&table,m,s.demand_effect_certificates().unwrap(),s.demand_alloc_summary().unwrap());
    let rows=hale_types::judgment::judge_certificates(&table,m,&evidence,&[0]);
    for (name,expected) in [("Child::trigger",Verdict::Uncertified),("Child::idle",Verdict::Holds)] {
        let ordinal=table.rows.iter().find(|r| r.name==name).unwrap().ordinal;
        assert_eq!(rows.iter().find(|r| r.ordinal==ordinal).unwrap().verdict,expected,"{name}");
    }
}


#[test]
fn argument_checkpoint_context_does_not_borrow_the_declaring_callee_self() {
    let source=r#"fn receiver_observe() -> Int { println("receiver checkpoint"); return 0; }
fn caller_observe() -> Int { println("caller checkpoint"); return 0; }
locus Receiver {
    params { n: Int = 0; }
    closure audit { receiver_observe() + self.n ~~ self.n within 0; epoch explicit; }
    fn receive(value: Int = if true { check_closures(); 0 } else { 0 }) { }
}
main locus App {
    params { n: Int = 0; }
    closure audit { caller_observe() + self.n ~~ self.n within 0; epoch explicit; }
    @effects(none: {stdio}) fn start() { let service = Receiver { }; service.receive(); }
    run() { self.start(); }
}
fn main() { App { }; }
"#;
    for (source,expected) in [(source.to_string(),Verdict::Uncertified),(source.replace("service.receive();","service.receive(0);"),Verdict::Holds)] {
        let s=entries::load_files(&[("main.hl",&source)],Config::check(true,false));
        let summary=s.demand_alloc_summary().unwrap();
        let default=summary.fns.values().find(|f| f.key.display()=="Receiver::receive::args::value").unwrap();
        assert!(default.closure_owner.is_none() && default.closure_evaluations.is_empty(),"the native default checkpoint uses the caller self");
        assert!(default.has_unknown_closure_context());
        assert_eq!(s.demand_effect_certificates().unwrap().iter().find(|(r,_)| r.subject=="App::start").unwrap().0.result,expected,"the ordinary effect reporter must preserve caller-context uncertainty");
        let m=s.demand_model().unwrap();m.validate().unwrap();let bundle=s.bundle();
        let table=hale_types::claim_lowering::lower_claims(&bundle,m);
        let evidence=hale_types::evidence::derive_certificate_evidence_over(&bundle,&table,m,s.demand_effect_certificates().unwrap(),summary);
        let ordinal=table.rows.iter().find(|r| r.name=="App::start").unwrap().ordinal;
        let result=hale_types::judgment::judge_certificates(&table,m,&evidence,&[0]).into_iter().find(|r| r.ordinal==ordinal).unwrap();
        assert_eq!(result.verdict,expected);
    }
}

#[test]
fn checkpoint_operands_follow_closure_declaration_order_before_label_order() {
    let source=SOURCE.replace("closure audit", "closure latent { epoch inline; }\n    closure zeta")
        .replace("    @effects(none: {stdio}) fn trigger()", "    closure alpha { self.n ~~ self.n within 0; epoch explicit; }\n    @effects(none: {stdio}) fn trigger()")
        .replace("check_closures();", "check_closures(); check_closures();");
    let s=entries::load_files(&[("main.hl",&source)],Config::check(true,false));
    let summary=s.demand_alloc_summary().unwrap();
    let trigger=summary.fns.values().find(|f| f.key.display()=="Child::trigger").unwrap();
    assert_eq!(trigger.closure_evaluations.iter().map(|edge| summary.closure_assertions[&edge.target].name.as_str()).collect::<Vec<_>>(),["zeta","alpha","zeta","alpha"]);
    assert_eq!(trigger.closure_evaluations.iter().map(|edge| summary.closure_assertions[&edge.target].ordinal).collect::<Vec<_>>(),[1,2,1,2]);
    s.demand_model().unwrap().validate().unwrap();
}

#[test]
fn construction_exposes_automatic_operand_work_without_certifying_runtime_costs() {
    use hale_types::alloc_summary::EffectSiteKind;
    for epoch in ["birth", "dissolve", "tick", "duration(10ms)"] {
        let source = format!(r#"
fn observe() -> Int {{ println("automatic epoch"); return 0; }}
locus Child {{
    params {{ n: Int = 0; }}
    closure audit {{ observe() + self.n ~~ self.n within 0; epoch {epoch}; }}
    run() {{ let value = 0; }}
}}
@effects(none: {{stdio}}) @budget(stack_bytes = 999999)
fn construct() {{ Child {{ }}; }}
@effects(none: {{stdio}}) fn idle() {{ }}
group builders = {{ construct }};
main locus App {{
    claims {{ quiet: forbid reaches(builders, effects(stdio)); }}
    run() {{ construct(); }}
}}
fn main() {{ App {{ }}; }}
"#);
        let s = entries::load_files(&[("main.hl", &source)], Config::check(true, false));
        let summary = s.demand_alloc_summary().unwrap();
        let caller = summary.fns.values().find(|f| f.key.display() == "construct").unwrap();
        let obligations = caller.effect_sites.iter().filter_map(|s| match &s.kind {
            EffectSiteKind::AutomaticClosure { constructor, target, epoch } => Some((*constructor, target, epoch)),
            _ => None,
        }).collect::<Vec<_>>();
        assert_eq!(obligations.len(), 1, "{epoch}");
        let (ordinal, target, actual_epoch) = obligations[0];
        assert_eq!(ordinal, 0);
        assert_eq!(caller.constructions[0].target, summary.closure_assertions[target].owner.declaration_id());
        assert_eq!(*actual_epoch, summary.closure_assertions[target].epoch);
        assert!(caller.body_evaluations().next().is_some(), "the omitted n default is still ordinary execution");
        assert!(caller.closure_evaluations.is_empty(), "automatic work must not masquerade as an explicit checkpoint");
        let reports = s.demand_effect_certificates().unwrap();
        assert_eq!(reports.iter().find(|(r, _)| r.subject == "construct").unwrap().0.result, Verdict::Violated, "{epoch}");
        assert_eq!(reports.iter().find(|(r, _)| r.subject == "idle").unwrap().0.result, Verdict::Holds, "{epoch}");
        let model = s.demand_model().unwrap();
        model.validate().unwrap();
        let mut missing = model.clone();
        missing.holes.retain(|h| h.kind != hale_model::HoleKind::UnmodeledLifecycleContext);
        assert!(missing.validate_coverage().is_err(), "automatic coverage cannot disappear: {epoch}");
        assert!(missing.validate().is_err(), "automatic coverage cannot disappear: {epoch}");
        let bundle = s.bundle();
        let table = hale_types::claim_lowering::lower_claims(&bundle, model);
        let evidence = hale_types::evidence::derive_certificate_evidence_over(&bundle, &table, model, reports, summary);
        let budget = table.rows.iter().find(|r| matches!(r.law, hale_model::ClaimIr::QuantBudget { .. })).unwrap().ordinal;
        let certificate_results = hale_types::judgment::judge_certificates(&table, model, &evidence, &[0]);
        let budget_result = certificate_results.iter().find(|r| r.ordinal == budget).unwrap();
        assert_eq!(budget_result.verdict, Verdict::Uncertified, "{epoch}: {budget_result:?}");
        let quiet = table.rows.iter().find(|r| r.name == "quiet").unwrap().ordinal;
        let (_, rows) = hale_types::judgment::judge_forbid_reaches(&table, model, &[0]);
        assert_eq!(rows.iter().find(|r| r.ordinal == quiet).unwrap().verdict, Verdict::Violated, "{epoch}");
    }
}

#[test]
fn automatic_epoch_obligations_keep_constructor_scope_and_known_violations() {
    let source = r#"
fn observe() -> Int { println("automatic epoch"); return 0; }
locus Child { closure audit { observe() ~~ 0 within 0; epoch birth; } }
fn make() { Child { }; }
@effects(none: {stdio}) fn caller() { make(); }
@effects(none: {stdio}) fn known() { println("direct"); make(); }
@effects(none: {stdio}) fn idle() { }
main locus App { run() { caller(); } }
fn main() { App { }; }
"#;
    let s = entries::load_files(&[("main.hl", source)], Config::check(true, false));
    let reports = s.demand_effect_certificates().unwrap();
    for (name, result) in [("caller", Verdict::Violated), ("known", Verdict::Violated), ("idle", Verdict::Holds)] {
        assert_eq!(reports.iter().find(|(r, _)| r.subject == name).unwrap().0.result, result, "{name}");
    }
    let rows = s.demand_effects().unwrap();
    let caller = rows.rows.iter().find(|(k, _)| k.display() == "caller").unwrap().1;
    let known = rows.rows.iter().find(|(k, _)| k.display() == "known").unwrap().1;
    assert!(!caller.unknown && caller.known.contains(hale_types::stdlib_surface::EffectSet::STDIO));
    assert!(known.known.contains(hale_types::stdlib_surface::EffectSet::STDIO));
}

#[test]
fn automatic_epoch_obligations_follow_imports_and_evaluated_defaults_only() {
    let main = r#"
import "./lib" as lib;
@effects(none: {stdio}) fn consume(child: lib::Child = lib::Child { }) { }
@effects(none: {stdio}) fn omitted() { consume(); }
@effects(none: {stdio}) fn supplied(child: lib::Child) { consume(child); }
main locus App { run() { omitted(); } }
fn main() { App { }; }
"#;
    let library = "locus Child { params { n: Int = 0; } closure audit { self.n ~~ 0 within 0; epoch birth; } }";
    let s = entries::load_files(&[("main.hl", main), ("lib/child.hl", library)], Config::check(true, false));
    let reports = s.demand_effect_certificates().unwrap();
    for (name, result) in [("omitted", Verdict::Holds), ("supplied", Verdict::Holds), ("consume", Verdict::Holds)] {
        assert_eq!(reports.iter().find(|(r, _)| r.subject == name).unwrap().0.result, result, "{name}");
    }
    let summary = s.demand_alloc_summary().unwrap();
    let default = summary.fns.values().find(|f| f.key.display() == "consume::args::child").unwrap();
    let target = default.effect_sites.iter().find_map(|site| match &site.kind {
        hale_types::alloc_summary::EffectSiteKind::AutomaticClosure { target, .. } => Some(target),
        _ => None,
    }).unwrap();
    let definition = &summary.closure_assertions[target];
    assert_eq!(definition.owner.declaration_id(), default.constructions[0].target);
    assert_eq!(definition.owner.universe, hale_types::placement::SiteUniverse::User);
    let model = s.demand_model().unwrap(); model.validate().unwrap();
    assert!(model.holes.iter().any(|h| h.reason.contains("automatic birth closure")));
}

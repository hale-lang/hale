//! Parent callbacks execute at constructions, including inline default roots.
#[path = "support/entries.rs"]
mod entries;
use hale_frontend::snapshot::{Config, Snapshot};
use hale_types::{
    protocol_execution::{ProtocolRoot, ProtocolStage},
    verdict::Verdict,
};

fn snapshot(source: &str) -> Snapshot {
    entries::load_files(&[("main.hl", source)], Config::check(true, false))
}
fn verdict(s: &Snapshot, subject: &str) -> Verdict {
    s.demand_effect_certificates()
        .unwrap()
        .iter()
        .find(|(r, _)| r.subject == subject)
        .unwrap()
        .0
        .result
}
const DIRECT: &str = r#"
locus Child { }
locus Parent {
    accept(c: Child) { println("accepted"); }
    @effects(none: {stdio}) fn make() { Child { }; }
    run() { self.make(); }
}
fn main() { Parent { }; }
"#;
#[test]
fn direct_accept_and_release_contribute_known_effects() {
    for source in [
        DIRECT.to_string(),
        DIRECT.replace(
            "accept(c: Child) { println(\"accepted\"); }",
            "accept(c: Child) { } release(c: Child) { println(\"released\"); }",
        ),
    ] {
        let s = snapshot(&source);
        assert_eq!(verdict(&s, "Parent::make"), Verdict::Violated);
        let rows = s.demand_effects().unwrap();
        assert!(rows.summary.parent_execution_supplied);
        let body = rows
            .summary
            .fns
            .values()
            .find(|f| f.key.display() == "Parent::make")
            .unwrap();
        assert_eq!(body.protocol_evaluations.len(), 1);
        let edge = &body.protocol_evaluations[0];
        assert!(matches!(edge.root, ProtocolRoot::Construction(0)));
        assert_eq!(edge.constructor_function, body.key);
        assert_eq!(edge.owner.name, "Parent");
        assert_eq!(body.frame_targets().count(), 0);
        assert!(body.calls.is_empty());
        assert_eq!(s.builds()["protocol_execution"], 1);
    }
}
#[test]
fn accepting_ancestor_is_reachable_from_the_actual_constructor() {
    let source = r#"
locus Child { }
locus Middle { @effects(none: {stdio}) fn make() { Child { }; } run() { self.make(); } }
locus Parent { params { middle: Middle = Middle { }; } accept(c: Child) { println("ancestor"); } }
fn main() { Parent { }; }
"#;
    let s = snapshot(source);
    assert_eq!(verdict(&s, "Middle::make"), Verdict::Violated);
    let effects = s.demand_effects().unwrap();
    let body = effects
        .summary
        .fns
        .values()
        .find(|f| f.key.display() == "Middle::make")
        .unwrap();
    assert_eq!(body.protocol_evaluations[0].owner.name, "Parent");
}
#[test]
fn argument_default_is_attributed_to_its_callers_root() {
    let source = r#"
locus Child { }
fn use_child(c: Child = Child { }) { }
locus Parent {
    accept(c: Child) { println("argument"); }
    @effects(none: {stdio}) fn make() { use_child(); }
    run() { self.make(); }
}
@effects(none: {stdio}) fn free() { use_child(); }
fn main() { Parent { }; free(); }
"#;
    let s = snapshot(source);
    assert_eq!(verdict(&s, "Parent::make"), Verdict::Violated);
    assert_eq!(verdict(&s, "free"), Verdict::Holds);
    let effects = s.demand_effects().unwrap();
    let body = effects
        .summary
        .fns
        .values()
        .find(|f| f.key.display() == "Parent::make")
        .unwrap();
    let edge = &body.protocol_evaluations[0];
    assert!(matches!(edge.root, ProtocolRoot::Call(_)));
    assert_ne!(edge.constructor_function, body.key);
    assert_ne!(edge.root_source, edge.constructor_source);
    assert!(effects
        .summary
        .fns
        .values()
        .filter(|f| effects.summary.initializers.contains_key(&f.key))
        .all(|f| f.protocol_evaluations.is_empty()));
}
#[test]
fn ordinary_free_function_does_not_inherit_the_callers_locus() {
    let source = r#"
locus Child { }
fn make() { Child { }; }
locus Parent { accept(c: Child) { println("never"); } @effects(none: {stdio}) fn call() { make(); } run() { self.call(); } }
fn main() { Parent { }; }
"#;
    let s = snapshot(source);
    assert_eq!(verdict(&s, "Parent::call"), Verdict::Holds);
    assert!(s
        .demand_effects()
        .unwrap()
        .summary
        .fns
        .values()
        .all(|f| f.protocol_evaluations.is_empty()));
}
#[test]
fn empty_hooks_have_no_work_and_complete_pure_hooks_close_source_effects() {
    let empty = snapshot(&DIRECT.replace("println(\"accepted\");", ""));
    assert_eq!(verdict(&empty, "Parent::make"), Verdict::Holds);
    let pure = snapshot(&DIRECT.replace("println(\"accepted\");", "let marker = 1;"));
    assert_eq!(verdict(&pure, "Parent::make"), Verdict::Holds);
    let rows = pure.demand_effects().unwrap();
    let body = rows
        .summary
        .fns
        .values()
        .find(|f| f.key.display() == "Parent::make")
        .unwrap();
    assert_eq!(body.protocol_evaluations[0].stage, ProtocolStage::Accept);
}
#[test]
fn parameter_self_does_not_replace_the_callers_construction_context() {
    let source = r#"
locus Child { }
locus Root { params { c: Child = Child { }; } accept(c: Child) { println("inner"); } }
locus Parent { accept(c: Child) { println("outer"); } @effects(none: {stdio}) fn make() { Root { }; } run() { self.make(); } }
@effects(none: {stdio}) fn free() { Root { }; }
fn main() { Parent { }; free(); }
"#;
    let s = snapshot(source);
    assert_eq!(verdict(&s, "Parent::make"), Verdict::Violated);
    assert_eq!(verdict(&s, "free"), Verdict::Holds);
    let rows = s.demand_effects().unwrap();
    let edges: Vec<_> = rows
        .summary
        .fns
        .values()
        .flat_map(|f| &f.protocol_evaluations)
        .collect();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].owner.name, "Parent");
}

#[test]
fn graph_known_parent_path_wins_over_unresolved_invocation_coverage() {
    let source = r#"
locus Child { }
locus Parent {
    accept(c: Child) { println("accepted"); }
    fn make() { Child { }; }
}
fn construct(parent: Parent) { parent.make(); }
group constructors = { construct };
main locus App { claims { quiet: forbid reaches(constructors, effects(stdio)); } }
fn main() { let parent = Parent { }; construct(parent); App { }; }
"#;
    let s = snapshot(source);
    let model = s.demand_model().unwrap();
    model.validate().unwrap();
    let root = model
        .entities
        .functions
        .iter()
        .position(|f| f.display == "Parent::make")
        .unwrap();
    assert!(model.holes.iter().any(|h| h.at
        == hale_model::EntityRef::Function(hale_model::FunctionId(root as u32))
        && h.kind == hale_model::HoleKind::UnmodeledLifecycleContext
        && h.reason.contains("parent accept")));
    let table = hale_types::claim_lowering::lower_claims(&s.bundle(), model);
    let (pre, rows, proofs) =
        hale_types::judgment::judge_forbid_reaches_with_certificates(&table, model, &[0]);
    assert!(pre.is_empty(), "{pre:?}");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].verdict, Verdict::Violated, "{:?}", rows[0].diags);
    assert!(
        rows[0]
            .diags
            .iter()
            .any(|d| d.message.contains("may enter parent accept")),
        "{:?}",
        rows[0].diags
    );
    assert_eq!(proofs.len(), 1);
    assert!(matches!(proofs[&0].certificate, hale_types::reachability_proof::Certificate::Path { .. }));
    proofs[&0].validate().unwrap();
    let pure = snapshot(&source.replace("println(\"accepted\");", "let marker = 1;"));
    let table = hale_types::claim_lowering::lower_claims(&pure.bundle(), pure.demand_model().unwrap());
    let (_, rows, proofs) = hale_types::judgment::judge_forbid_reaches_with_certificates(&table, pure.demand_model().unwrap(), &[0]);
    assert_eq!(rows[0].verdict, Verdict::Holds);
    assert_eq!(proofs.len(), 1, "complete source work with no callbacks can supply a source-graph absence theorem");
    proofs[&0].validate().unwrap();
}

#[test]
fn source_context_query_does_not_demand_execution_or_a_plan() {
    let s = snapshot(DIRECT);
    s.demand_default_evaluations().unwrap();
    assert_eq!(s.builds()["protocol_execution"], 0);
    assert_eq!(s.builds()["model"], 0);
    assert_eq!(s.builds()["lifecycle_order"], 0);
    assert_eq!(verdict(&s, "Parent::make"), Verdict::Violated);
    s.demand_effects().unwrap();
    assert_eq!(s.builds()["protocol_execution"], 1);
    assert_eq!(s.builds()["alloc_summary"], 1);
    assert_eq!(s.builds()["default_evaluation"], 1);
}

#[test]
fn an_omitted_record_field_uses_the_actual_callers_construction_context() {
    let source = r#"
locus Child { }
type Ticket { id: Int = { Child { }; 0 }; }
locus Parent {
    accept(c: Child) { println("field accepted"); }
    @effects(none: {stdio}) fn make() { let ticket = Ticket { }; }
    @effects(none: {stdio}) fn supplied() { let ticket = Ticket { id: 1 }; }
    run() { self.make(); }
}
@effects(none: {stdio}) fn free() { let ticket = Ticket { }; }
fn main() { Parent { }; free(); }
"#;
    let s = snapshot(source);
    assert_eq!(verdict(&s, "Parent::make"), Verdict::Violated);
    assert_eq!(verdict(&s, "Parent::supplied"), Verdict::Holds);
    assert_eq!(verdict(&s, "free"), Verdict::Holds);
    let rows = s.demand_effects().unwrap();
    let body = rows
        .summary
        .fns
        .values()
        .find(|f| f.key.display() == "Parent::make")
        .unwrap();
    let edge = &body.protocol_evaluations[0];
    assert!(matches!(edge.root, ProtocolRoot::Construction(0)));
    assert_eq!(
        rows.summary.initializers[&edge.constructor_function].kind,
        hale_model::InitializerKind::Field
    );
    assert_ne!(edge.root_source, edge.constructor_source);
}

#[test]
fn editing_a_parent_hook_refreshes_contextual_certificates_under_typing_reuse() {
    let empty = snapshot(&DIRECT.replace("println(\"accepted\");", ""));
    assert_eq!(verdict(&empty, "Parent::make"), Verdict::Holds);
    let printing = snapshot(DIRECT).reusing_typing(empty);
    assert_eq!(verdict(&printing, "Parent::make"), Verdict::Violated);
    assert!(printing.typing_reuse().is_some());
    assert_eq!(printing.builds()["protocol_execution"], 1);
    let removed = snapshot(&DIRECT.replace("accept(c: Child) { println(\"accepted\"); }", ""))
        .reusing_typing(printing);
    assert_eq!(verdict(&removed, "Parent::make"), Verdict::Holds);
    assert!(removed.typing_reuse().is_some());
    assert_eq!(removed.builds()["protocol_execution"], 1);
    assert_eq!(removed.builds()["model"], 0);
    assert_eq!(removed.builds()["lifecycle_order"], 0);
}

#[test]
fn an_unrelated_acceptor_does_not_poison_an_unrepresented_child_context() {
    let source = r#"
locus Child { }
locus Unrelated { accept(c: Child) { println("unrelated"); } }
locus Caller { @effects(none: {stdio}) fn make() { Child { }; } run() { self.make(); } }
fn main() { Caller { }; }
"#;
    let s = snapshot(source);
    let mut graph = s.demand_ownership_graph().unwrap().clone();
    // Keep the resolved nominal symbol and complete caller ancestry, while
    // withdrawing the child declaration from this graph's source horizon.
    for site in &mut graph.sites {
        if site.enclosing_locus == "Caller" {
            site.child_decl = None;
        }
    }
    let summary = hale_types::protocol_execution::derive(
        s.demand_alloc_summary().unwrap(),
        &graph,
        s.demand_default_evaluations().unwrap(),
    );
    let bundle = s.bundle();
    let programs: Vec<_> = bundle.programs.values().copied().collect();
    let reports = hale_types::effects::certificate_rows(&programs, &summary);
    assert_eq!(
        reports
            .iter()
            .find(|r| r.subject == "Caller::make")
            .unwrap()
            .result,
        Verdict::Holds
    );
    assert!(summary
        .fns
        .values()
        .all(|f| f.protocol_evaluations.is_empty()));
}

#[test]
fn a_reused_caller_gets_its_parent_helpers_new_effects() {
    let body = DIRECT.replace("println(\"accepted\");", "observe();");
    let source = |printing: bool| {
        format!(
            "{}\nfn untouched() -> Int {{ return 7; }}\nfn observe() {{ {} }}\n",
            body,
            if printing {
                "println(\"observed\");"
            } else {
                ""
            }
        )
    };
    let before = snapshot(&source(false));
    assert_eq!(verdict(&before, "Parent::make"), Verdict::Holds);
    let after = snapshot(&source(true)).reusing_typing(before);
    assert_eq!(verdict(&after, "Parent::make"), Verdict::Violated);
    assert!(
        matches!(after.typing_reuse(),Some(hale_frontend::typing_reuse::TypingReuse::Reused { reused,.. }) if *reused>0),
        "{:?}",
        after.typing_reuse()
    );
    assert_eq!(after.builds()["protocol_execution"], 1);
}
#[test]
fn a_bare_unminted_summary_cannot_certify_through_missing_parent_contexts() {
    let program = hale_syntax::parse_source(DIRECT).unwrap();
    let bundle = hale_types::Bundle::new([("app.hl".to_string(), &program)].into_iter().collect());
    let summary = hale_types::alloc_summary::derive_alloc_summary(&bundle);
    assert!(!summary.parent_execution_supplied);
    let reports = hale_types::effects::certificate_rows(&[&program], &summary);
    assert_eq!(
        reports
            .iter()
            .find(|r| r.subject == "Parent::make")
            .unwrap()
            .result,
        Verdict::Uncertified
    );
    assert!(hale_types::protocol_execution::lower(&summary.protocol_hooks, |_| None).is_err());
}


#[test]
fn canonical_parent_account_refuses_missing_entries_contexts_definitions_and_coverage() {
    let s = snapshot(DIRECT);
    let model = s.demand_model().unwrap();
    model.validate().unwrap();
    assert_eq!(model.relations.protocol_contexts.len(), 1);
    assert_eq!(model.relations.protocol_evaluations.len(), 1);
    let context = &model.relations.protocol_contexts[0];
    let entry = &model.relations.protocol_evaluations[0];
    assert_eq!(model.entities.functions[entry.to.index()].display, "Parent::accept");
    assert!(matches!(context.root, hale_model::ProtocolRoot::Construction(0)));
    let mut missing = model.clone();
    missing.relations.protocol_definitions.clear();
    assert!(missing.validate().is_err());
    let mut missing = model.clone();
    missing.relations.protocol_contexts.clear();
    assert!(missing.validate().is_err());
    let mut missing = model.clone();
    missing.relations.protocol_evaluations.clear();
    assert!(missing.validate().is_err());
    let mut missing = model.clone();
    missing.holes.retain(|h| !(h.at == hale_model::EntityRef::Function(context.from)
        && h.kind == hale_model::HoleKind::UnmodeledLifecycleContext));
    assert!(missing.validate().is_err());
    let mut changed = model.clone();
    changed.relations.protocol_contexts[0].root_source = hale_graph::ids::SiteId { seed: context.constructor_source.seed, index: context.constructor_source.index + 1 };
    assert!(changed.validate().is_err());
    let mut changed = model.clone();
    changed.relations.protocol_contexts[0].root = hale_model::ProtocolRoot::Call(0);
    assert!(changed.validate().is_err());
    let mut changed = model.clone();
    changed.relations.protocol_evaluations[0].stage = ProtocolStage::Release;
    assert!(changed.validate().is_err());
    let mut changed = model.clone();
    changed.relations.protocol_definitions[0].empty = true;
    assert!(changed.validate().is_err());
}

#[test]
fn canonical_context_keeps_nested_defaults_at_their_actual_call_root() {
    let source = r#"
locus Child { }
fn use_child(c: Child = Child { }) { }
locus Parent { accept(c: Child) { println("argument"); } fn make() { use_child(); } }
fn main() { Parent { }; }
"#;
    let s = snapshot(source);
    let model = s.demand_model().unwrap();
    model.validate().unwrap();
    assert_eq!(model.relations.protocol_contexts.len(), 1);
    let c = &model.relations.protocol_contexts[0];
    assert!(matches!(c.root, hale_model::ProtocolRoot::Call(_)));
    assert_ne!(c.from, c.constructor_function);
    assert_ne!(c.root_source, c.constructor_source);
    assert_ne!(c.provenance, c.constructor_provenance);
    assert!(hale_types::model_query::executions(model).any(|v| v.from == c.from
        && matches!(v.site, hale_types::model_query::ExecutionSite::ParentProtocol { .. })));
    let mut changed = model.clone();
    changed.relations.protocol_contexts[0].constructor_function = c.from;
    assert!(changed.validate().is_err(), "an ordinary caller cannot replace the actual default constructor body");
}

#[test]
fn two_parent_bookends_and_multiple_roots_keep_separate_contexts() {
    let source = r#"
locus Child { }
locus Parent {
    accept(c: Child) { println("accept"); }
    release(c: Child) { println("release"); }
    fn make() { Child { }; Child { }; }
}
fn main() { Parent { }; }
"#;
    let s = snapshot(source);
    let model = s.demand_model().unwrap();
    model.validate().unwrap();
    assert_eq!(model.relations.protocol_contexts.len(), 2);
    assert_eq!(model.relations.protocol_evaluations.len(), 4);
    assert!(model.relations.protocol_evaluations.chunks_exact(2).all(|v|
        v[0].stage == ProtocolStage::Accept && v[1].stage == ProtocolStage::Release));
}


#[test]
fn parent_call_roots_project_original_identity_across_interface_alternatives() {
    let source = r#"
interface Worker { fn tick(); }
locus One { fn tick() { } }
locus Two { fn tick() { } }
locus Child { }
fn use_child(c: Child = Child { }) { }
locus Parent {
    accept(c: Child) { println("accepted"); }
    fn make(w: Worker) { w.tick(); use_child(); }
}
fn main() { Parent { }; }
"#;
    let s = snapshot(source);
    let summary = s.demand_protocol_execution().unwrap();
    let source_body = summary.fns.values().find(|f| f.key.display() == "Parent::make").unwrap();
    let raw = source_body.protocol_evaluations[0].root;
    assert_eq!(raw, ProtocolRoot::Call(2));
    let model = s.demand_model().unwrap();
    model.validate().unwrap();
    assert_eq!(model.relations.protocol_contexts.len(), 1);
    let context = &model.relations.protocol_contexts[0];
    assert_eq!(context.root, hale_model::ProtocolRoot::Call(1));
    assert_eq!(context.root_source, source_body.protocol_evaluations[0].root_source);
    let mut changed = model.clone();
    changed.relations.protocol_contexts[0].root = hale_model::ProtocolRoot::Call(2);
    assert!(changed.validate().is_err(), "a source-summary ordinal is not a canonical authored-site ordinal");
}


#[test]
fn nested_argument_default_in_a_record_field_preserves_its_outer_root() {
    let source = r#"
locus Child { }
fn use_child(c: Child = Child { }) { }
type Ticket { id: Int = { use_child(); 0 }; }
locus Parent { accept(c: Child) { println("accepted"); } fn make() { let ticket = Ticket { }; } }
fn main() { Parent { }; }
"#;
    let s = snapshot(source);
    let model = s.demand_model().unwrap();
    model.validate().unwrap();
    assert_eq!(model.relations.protocol_contexts.len(), 1);
    let c = &model.relations.protocol_contexts[0];
    assert_eq!(model.entities.functions[c.from.index()].display, "Parent::make");
    assert_eq!(model.entities.functions[c.constructor_function.index()].display, "use_child::args::c");
    assert_eq!(c.root, hale_model::ProtocolRoot::Construction(0));
}


#[test]
fn a_specialized_child_keeps_its_symbol_and_template_in_distinct_accounts() {
    let source = r#"
locus Cell<T> { params { value: Int = 0; } }
locus Parent {
    accept(c: Cell<Int>) { println("accepted"); }
    fn make() { let child: Cell<Int> = Cell { value: 1 }; }
}
fn main() { Parent { }; }
"#;
    let s = snapshot(source);
    let model = s.demand_model().unwrap();
    model.validate().unwrap();
    assert_eq!(model.relations.protocol_contexts.len(), 1);
    let c = &model.relations.protocol_contexts[0];
    assert_eq!(c.child, "Cell_Int");
    assert_eq!(model.protocol_constructor_symbols(c.constructor_function, c.constructor).into_iter().collect::<Vec<_>>(), vec!["Cell_Int"]);
    let constructor = model.relations.constructions.iter().find(|v| v.function == c.constructor_function && v.site == c.constructor).unwrap();
    let hale_model::ConstructorTarget::Locus(template) = constructor.owner else { panic!("locus") };
    assert_eq!(model.entities.loci[template.index()].name, "Cell");
    let mut changed = model.clone();
    changed.relations.protocol_contexts[0].child = "Cell".into();
    changed.relations.protocol_definitions[0].child = Some("Cell".into());
    assert!(changed.validate().is_err(), "a template name is not the actual constructed specialization");
}

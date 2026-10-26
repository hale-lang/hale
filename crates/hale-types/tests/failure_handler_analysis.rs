//! Failure handlers participate in the canonical body walk, with their
//! declaration identities preserved even when signatures display alike.

#[path = "support/entries.rs"]
mod entries;

use hale_frontend::snapshot::{Config, Snapshot};
use hale_model::{FunctionId, FunctionKind, HoleKind};
use hale_types::alloc_summary::{Callee, EntryKind};
use hale_types::verdict::Verdict;

const SOURCE: &str = r#"
type Event { n: Int = 0; }
topic Events { payload: Event; subject: "recovery.event"; }
locus Child { params { n: Int = 0; } }
locus Other { params { cb: fn (Int) -> Int; } }
fn emit() { println(1); }
fn quiet(v: Int) -> Int { return v; }
locus Parent {
    params { c: Child = Child { }; }
    bus { publish Events; }
    on_failure(c: Child, err: ClosureViolation) {
        emit();
        Events <- Event { n: 1 };
        restart(c);
    }
    on_failure(o: Other, err: ClosureViolation) { o.cb(1); quarantine(o); }
}
group parents = { Parent };
main locus App {
    params { p: Parent = Parent { }; }
    // claim insertion
}
fn main() { App { }; }
"#;

fn snapshot(src: &str) -> Snapshot {
    entries::load_files(&[("app.hl", src)], Config::check(true, false))
}

#[test]
fn handlers_have_unique_summary_effect_and_model_rows_without_duplicate_walks() {
    let s = snapshot(SOURCE);
    let checked = s.demand_check().unwrap_or_else(|_| panic!("check"));
    assert!(!checked.diags.iter().any(|d| d.is_error()), "{:?}", checked.diags);
    let summary = s.demand_alloc_summary().unwrap();
    let handlers: Vec<_> = summary.fns.values().filter(|f| f.entry == Some(EntryKind::FailureHandler)).collect();
    assert_eq!(handlers.len(), 2);
    assert_ne!(handlers[0].key.decl, handlers[1].key.decl);
    for h in &handlers {
        assert!(!summary.declaration_bodies.iter().any(|b| b.summary.key == h.key), "handler walked only once");
        assert!(summary.unbounded_invoked().contains(&h.key), "failure count is not bounded by a retry literal");
    }
    let emit = summary.fns.keys().find(|k| k.fn_name == "emit").unwrap();
    assert!(summary.unbounded_invoked().contains(emit), "helper inherits handler multiplicity");
    let loud = handlers.iter().find(|h| h.calls.iter().any(|c| matches!(&c.callee, Callee::Resolved(k) if k == emit))).unwrap();
    let effects = s.demand_effects().unwrap();
    let row = effects.row(&loud.key).unwrap();
    assert!(row.summarized && row.targets.contains(emit));
    assert!(row.known.contains(hale_types::stdlib_surface::EffectSet::SYSCALL));
    let m = s.demand_model().unwrap();
    m.validate().unwrap();
    let hs: Vec<_> = m.entities.functions.iter().enumerate().filter(|(_, f)| f.kind == FunctionKind::FailureHandler).map(|(i, f)| (FunctionId(i as u32), f)).collect();
    assert_eq!(hs.len(), 2, "no synthetic Parent::on_failure alias row");
    assert!(hs.iter().all(|(_, f)| f.analyzed && f.summarized));
    assert!(!m.holes.iter().any(|h| h.kind == HoleKind::UnanalyzedBody));
    let emit = FunctionId(m.entities.functions.iter().position(|f| f.name == "emit").unwrap() as u32);
    let (loud_id, loud) = hs.iter().find(|(id, _)| m.relations.calls.iter().any(|c| c.from == *id && c.to == emit)).unwrap();
    assert!(loud.effect_lower_bound.iter().any(|e| e == "syscall"));
    assert_eq!(m.relations.publishes.iter().filter(|p| p.function == *loud_id).count(), 1);
    assert!(m.relations.costs.iter().any(|c| c.function == *loud_id), "the event literal's allocation is visible");
    assert!(m.relations.phase_of.iter().any(|p| p.function == *loud_id));
    let artifact: serde_json::Value = serde_json::from_str(&entries::dump_topology_parts(&s.bundle())).unwrap();
    assert_eq!(artifact["phases"][loud.name.as_str()]["kind"], "hook", "a failure handler is an entry hook, not a callable method");
    let unknown = hs.iter().find(|(id, _)| *id != *loud_id).unwrap();
    assert!(m.holes.iter().any(|h| h.at == hale_model::EntityRef::Function(unknown.0) && h.kind == HoleKind::IndirectCall));
    assert!(!summary.by_name.keys().any(|(owner, name)| owner.as_deref() == Some("Parent") && name.starts_with("on_failure")), "handlers are not callable spellings");
}

#[test]
fn duplicate_invalid_signatures_retain_distinct_declarations_and_are_refused() {
    // Duplicate signatures are rejected by the checker and canonical model.
    // Diagnostic construction must still preserve both source declarations.
    let src = SOURCE.replace("on_failure(o: Other", "on_failure(o: Child").replace("o.cb(1);", "quiet(1);");
    let program = hale_syntax::parse_source(&src).unwrap();
    let mut programs = std::collections::BTreeMap::new();
    programs.insert("app.hl".to_string(), &program);
    let m = entries::derive_application_model(&hale_types::Bundle::new(programs));
    assert!(matches!(m.validate(), Err(hale_model::ModelError::NotCanonical { table: "functions", .. })));
    let hs: Vec<_> = m.entities.functions.iter().enumerate().filter(|(_, f)| f.kind == FunctionKind::FailureHandler).map(|(i, f)| (FunctionId(i as u32), f)).collect();
    assert_eq!(hs.len(), 2);
    assert_eq!(hs[0].1.name, hs[1].1.name);
    assert_ne!(hs[0].0, hs[1].0);
    assert_ne!(hs[0].1.decl, hs[1].1.decl);
    assert!(hs.iter().all(|(_, f)| f.analyzed && f.summarized));
    let emit = FunctionId(m.entities.functions.iter().position(|f| f.name == "emit").unwrap() as u32);
    assert_eq!(hs.iter().filter(|(id, _)| m.relations.calls.iter().any(|c| c.from == *id && c.to == emit)).count(), 1);
}

fn judge(src: &str) -> Verdict {
    let s = snapshot(src);
    let m = s.demand_model().unwrap();
    let table = hale_types::claim_lowering::lower_claims(&s.bundle(), m);
    let (pre, rows, proofs) = hale_types::judgment::judge_forbid_reaches_with_certificates(&table, m, &[0]);
    assert!(pre.is_empty(), "{pre:?}");
    assert_eq!(rows.len(), 1);
    for proof in proofs.values() { proof.validate().unwrap(); }
    rows[0].verdict
}

#[test]
fn handler_reachability_retains_all_four_verdicts() {
    let claim = "claims { isolated: forbid reaches(parents, effects(syscall)); }";
    // Recovery execution itself has an honest gap until its implicit
    // lifecycle effects are modeled; isolate call-graph claims here.
    let src = SOURCE.replace("// claim insertion", claim).replace("restart(c);", "").replace("quarantine(o);", "");
    assert_eq!(judge(&src.replace("o.cb(1);", "")), Verdict::Violated, "helper reaches a known syscall");
    assert_eq!(judge(&src), Verdict::Uncertified, "the existing traversal stops at an earlier opaque handler root");
    let unknown = src.replace("emit();", "");
    assert_eq!(judge(&unknown), Verdict::Uncertified, "indirect call is not silently pure");
    let clean = unknown.replace("o.cb(1);", "");
    assert_eq!(judge(&clean), Verdict::Holds, "closed handler graph can certify this limited property");
    assert_eq!(judge(&clean.replace("reaches(parents,", "reaches(missing,")), Verdict::Invalid);
}

#[test]
fn callback_fields_follow_known_function_values_and_preserve_effect_witnesses() {
    let src = SOURCE.replace("params { c: Child = Child { }; }", "params { c: Child = Child { }; o: Other = Other { cb: quiet }; }")
        .replace("fn quiet(v: Int) -> Int { return v; }", "fn quiet(v: Int) -> Int { println(v); return v; }");
    let s = snapshot(&src);
    let m = s.demand_model().unwrap();
    m.validate().unwrap();
    let quiet = FunctionId(m.entities.functions.iter().position(|f| f.name == "quiet").unwrap() as u32);
    let h = m.entities.functions.iter().enumerate().find(|(_, f)| f.kind == FunctionKind::FailureHandler && f.name.contains("Other,")).unwrap();
    let hid = FunctionId(h.0 as u32);
    assert!(m.relations.calls.iter().any(|c| c.from == hid && c.to == quiet), "direct callback fields use the shared value-dispatch relation");
    assert!(h.1.effect_lower_bound.iter().any(|e| e == "syscall"));
    assert!(!m.holes.iter().any(|h| h.at == hale_model::EntityRef::Function(hid) && h.kind == HoleKind::IndirectCall), "known callback needs no fake callback gap");
}

#[test]
fn recovery_execution_is_not_certified_from_explicit_helper_calls_alone() {
    let src = SOURCE.replace("o.cb(1);", "").replace("emit();", "")
        .replace("// claim insertion", "claims { isolated: forbid reaches(parents, effects(syscall)); }");
    let s = snapshot(&src);
    let m = s.demand_model().unwrap();
    m.validate().unwrap();
    assert!(m.holes.iter().any(|h| h.kind == HoleKind::UnmodeledRecovery));
    let mut malformed = m.clone();
    let gap = malformed.holes.iter_mut().find(|h| h.kind == HoleKind::UnmodeledRecovery).unwrap();
    gap.hides = hale_model::RelationSet::CALLS.union(hale_model::RelationSet::EFFECTS).union(hale_model::RelationSet::PUBLISHES);
    assert!(matches!(malformed.validate(), Err(hale_model::ModelError::NotCanonical { table: "holes.shape", .. })),
        "recovery cannot hide its cost uncertainty");
    assert_eq!(judge(&src), Verdict::Uncertified);
    assert_eq!(judge(&src.replace("restart(c);", "").replace("quarantine(o);", "")), Verdict::Holds);
}

#[test]
fn categorical_certificates_keep_recovery_unknown_separate_from_known_violation() {
    let src = SOURCE.replace("fn main()", "@no_syscall fn recover(c: Child) { restart(c); }\nfn main()");
    let inspect = |source: &str, expected| {
        let program = hale_syntax::parse_source(source).unwrap();
        let mut programs = std::collections::BTreeMap::new();
        programs.insert("app.hl".to_string(), &program);
        let reports = entries::effect_certificates(&hale_types::Bundle::new(programs));
        let (row, diagnostics) = reports.iter().find(|(row, _)| row.subject == "recover").unwrap();
        assert_eq!(row.result, expected);
        assert_eq!(diagnostics.is_empty(), expected == Verdict::Holds);
        if expected == Verdict::Uncertified {
            assert!(diagnostics.iter().any(|(d, _)| d.message.contains("effect assertion uncertified")));
            assert!(diagnostics.iter().any(|(d, _)| d.message.contains("recovery operation is here")));
        }
    };
    inspect(&src, Verdict::Uncertified);
    inspect(&src.replace("@no_syscall fn recover(c: Child) { restart(c); }", "@no_syscall fn recover(c: Child) { }"), Verdict::Holds);
    inspect(&src.replace("@no_syscall fn recover(c: Child) { restart(c); }", "@no_syscall fn recover(c: Child) { println(1); restart(c); }"), Verdict::Violated);
}

#[test]
fn runtime_retry_bounds_are_in_the_same_call_and_effect_summary() {
    let src = SOURCE.replace("restart(c);", "restart(c) for retry_limit();")
        .replace("fn emit()", "fn retry_limit() -> Int { println(1); return 3; }\nfn emit()");
    let s = snapshot(&src);
    let effects = s.demand_effects().unwrap();
    let (key, summary) = effects.summary.fns.iter().find(|(_, f)| f.entry == Some(EntryKind::FailureHandler) && f.calls.iter().any(|c| matches!(&c.callee, Callee::Resolved(k) if k.fn_name == "retry_limit"))).unwrap();
    assert!(summary.effect_sites.iter().any(|s| matches!(s.kind, hale_types::alloc_summary::EffectSiteKind::Recovery(_))));
    let row = effects.row(key).unwrap();
    assert!(row.unknown, "implicit recovery work remains unknown");
    assert!(row.known.contains(hale_types::stdlib_surface::EffectSet::SYSCALL), "bound evaluation keeps its definite effect");
}

#[test]
fn phase_certificates_keep_recovery_witnesses_inside_their_report() {
    let src = SOURCE.replace("locus Parent {", "@phase_effects(run: {}) locus Parent {")
        .replace("    bus { publish Events; }", "    bus { publish Events; }\n    run() { restart(self.c); }");
    let program = hale_syntax::parse_source(&src).unwrap();
    let mut programs = std::collections::BTreeMap::new();
    programs.insert("app.hl".to_string(), &program);
    let reports = entries::effect_certificates(&hale_types::Bundle::new(programs));
    let (row, diagnostics) = reports.iter().find(|(row, _)| row.subject == "Parent" && row.form.ends_with("during run")).unwrap();
    assert_eq!(row.result, Verdict::Uncertified);
    assert!(diagnostics.iter().any(|(d, _)| d.message.contains("effect assertion uncertified")));
    assert!(diagnostics.iter().any(|(d, _)| d.message.contains("recovery operation is here")));
    assert!(diagnostics.iter().all(|(_, foreign)| !foreign));
}

#[test]
fn allocation_budget_cannot_ignore_recoverys_implicit_lifecycle_work() {
    let src = SOURCE.replace("fn main()", "@budget(alloc_per_call = 0) fn recover(c: Child) { restart(c); }\nfn main()");
    let inspect = |source: &str| {
        let s = snapshot(source);
        let m = s.demand_model().unwrap();
        let bundle = s.bundle();
        let table = hale_types::claim_lowering::lower_claims(&bundle, m);
        let evidence = hale_types::evidence::derive_certificate_evidence_over(&bundle, &table, m,
            s.demand_effect_certificates().unwrap(), s.demand_alloc_summary().unwrap());
        let rows = hale_types::judgment::judge_certificates(&table, m, &evidence, &[0]);
        let budget = table.rows.iter().find(|r| matches!(r.law, hale_model::ClaimIr::AllocBudget { .. } | hale_model::ClaimIr::QuantBudget { .. })).unwrap();
        rows.iter().find(|r| r.ordinal == budget.ordinal).unwrap().verdict
    };
    assert_eq!(inspect(&src), Verdict::Uncertified,
        "no explicit allocation sites is not a bound on restart's implicit lifecycle work");
    assert_eq!(inspect(&src.replace("fn recover(c: Child) { restart(c); }", "fn relay(c: Child) { restart(c); } fn recover(c: Child) { relay(c); }")), Verdict::Uncertified,
        "the same gap propagates through an explicit helper call");
    assert_eq!(inspect(&src.replace("fn recover(c: Child) { restart(c); }", "fn recover(c: Child) { }")), Verdict::Holds,
        "unrelated handlers must not withdraw this function's closed budget");
    assert_eq!(inspect(&src.replace("fn recover(c: Child) { restart(c); }", "fn recover(c: Child) { let event = Event { n: 1 }; restart(c); }")), Verdict::Violated,
        "a measured explicit allocation already exceeds the zero ceiling");
    let publish = src.replace("alloc_per_call = 0", "publish = 0");
    assert_eq!(inspect(&publish), Verdict::Uncertified, "quantitative ceilings share the same recovery coverage gate");
    assert_eq!(inspect(&publish.replace("fn recover(c: Child) { restart(c); }", "fn recover(c: Child) { }")), Verdict::Holds);

}

#[test]
fn a_symbolic_generic_receiver_is_not_a_resolved_zero_effect_leaf() {
    let src = SOURCE.replace("group parents", "locus Loud { fn poke() { emit(); } }\nlocus Guard<T> { params { peer: T; } on_failure(c: Child, e: ClosureViolation) { self.peer.poke(); } }\ngroup guards = { Guard };\ngroup parents")
        .replace("// claim insertion", "claims { quiet: forbid reaches(guards, effects(syscall)); }\n    run() { let g: Guard<Loud> = Guard { peer: Loud { } }; }");
    let s = snapshot(&src);
    let m = s.demand_model().unwrap();
    m.validate().unwrap();
    let handler = FunctionId(m.entities.functions.iter().position(|f| f.kind == FunctionKind::FailureHandler && f.owner.is_some_and(|l| m.entities.loci[l.index()].name == "Guard")).unwrap() as u32);
    assert!(m.holes.iter().any(|h| h.at == hale_model::EntityRef::Function(handler) && matches!(h.kind, HoleKind::UntypedReceiver { .. })), "a written T is not a concrete dispatch target");
    assert_eq!(judge(&src), Verdict::Uncertified);
    assert_eq!(judge(&src.replace("locus Loud", "locus T { fn poke() { } }\nlocus Loud")), Verdict::Uncertified,
        "the scoped generic T cannot bind to a global locus of the same spelling");
    let helper = src.replace("self.peer.poke();", "invoke_peer(self.peer);")
        .replace("group guards", "fn invoke_peer<T>(peer: T) { peer.poke(); }\ngroup guards");
    assert_eq!(judge(&helper), Verdict::Uncertified, "a free helper's generic scope retains the same dispatch horizon");
    assert_eq!(judge(&src.replace("peer: T;", "peer: Loud;")), Verdict::Violated, "a concrete receiver retains its real call path");
    assert_eq!(judge(&src.replace("self.peer.poke();", "quiet(1);")), Verdict::Holds, "unrelated recovery/flow holes do not poison this closed graph");
    let callback = src.replace("self.peer.poke();", "self.peer();")
        .replace("locus Guard<T>", "type Invoke = fn(); locus Guard<T>")
        .replace("Guard<Loud> = Guard { peer: Loud { } }", "Guard<Invoke> = Guard { peer: emit }");
    assert_eq!(judge(&callback), Verdict::Violated, "a generic function-valued field retains its actual value-dispatch path");
    let aliased = callback.replace("peer: T;", "peer: Invoke;");
    assert_eq!(judge(&aliased), Verdict::Violated, "a function type alias cannot disappear as an unresolved method");
    let silent = callback.replace("group guards", "fn silent() { }\ngroup guards").replace("peer: emit", "peer: silent");
    assert_eq!(judge(&silent), Verdict::Holds, "a known pure field value stays closed");

}

#[test]
fn aliased_handler_receivers_use_the_canonical_locus_template() {
    let law = "claims { quiet: forbid reaches(guards, effects(syscall)); }";
    let source = SOURCE.replace("group parents", "locus Loud { fn poke() { emit(); } }\ntype Peer = Loud;\nlocus Guard { on_failure(c: Peer, e: ClosureViolation) { c.poke(); } }\ngroup guards = { Guard };\ngroup parents")
        .replace("// claim insertion", law);
    let inspect = |src: &str, verdict| {
        let checked = hale_syntax::parse_source(&src.replace(law, "")).unwrap();
        let errors: Vec<_> = entries::check_program(&checked).into_iter().filter(|d| d.is_error()).collect();
        assert!(errors.is_empty(), "alias control must be checkable: {errors:?}");
        assert_eq!(judge(src), verdict);
    };
    inspect(&source, Verdict::Violated);
    inspect(&source.replace("type Peer = Loud;", "type Direct = Loud; type Peer = Direct;"), Verdict::Violated);
    inspect(&source.replace("locus Loud {", "locus Loud<T> {").replace("type Peer = Loud;", "type Peer = Loud<Int>;"), Verdict::Violated);
    inspect(&source.replace("c.poke();", "quiet(1);"), Verdict::Holds);
    let interface = source.replace("type Peer = Loud;", "interface Acting { fn poke(); } type Peer = Acting;");
    inspect(&interface, Verdict::Violated);
    inspect(&interface.replace("fn poke() { emit(); }", "fn poke() { }"), Verdict::Holds);

    // Lexical generic parameters shadow aliases; a global alias of the
    // same spelling cannot make the symbolic receiver a concrete locus.
    let symbolic = source.replace("locus Guard {", "locus Guard<Peer> {");
    inspect(&symbolic, Verdict::Uncertified);
}

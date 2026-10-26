//! Birth predicates are executable source bodies with conditional failure.
#[path = "support/entries.rs"]
mod entries;
use hale_frontend::snapshot::{Config, Snapshot};
use hale_model::{EntityRef, FunctionKind, HoleKind};
use hale_types::verdict::Verdict;

const SOURCE: &str = r#"
fn before() -> Bool { println("birth predicate"); return false; }
@phase_effects(birth: {})
locus Child {
    closure rejected { epoch inline; }
    birth_check { before() } -> violate rejected;
}
fn main() { Child { }; }
"#;
fn snapshot(source: &str) -> Snapshot {
    entries::load_files(&[("main.hl", source)], Config::check(true, false))
}
fn phase_verdict(s: &Snapshot) -> Verdict {
    let m = s.demand_model().unwrap();
    m.validate().unwrap();
    let bundle = s.bundle();
    let table = hale_types::claim_lowering::lower_claims(&bundle, m);
    let reports = s.demand_effect_certificates().unwrap();
    let evidence = hale_types::evidence::derive_certificate_evidence_over(&bundle, &table, m, reports, s.demand_alloc_summary().unwrap());
    let ordinal = table.rows.iter().find(|r| matches!(r.law, hale_model::ClaimIr::PhaseEffects { .. })).unwrap().ordinal;
    hale_types::judgment::judge_certificates(&table, m, &evidence, &[ordinal]).into_iter().find(|r| r.ordinal == ordinal).unwrap().verdict
}
#[test]
fn a_predicate_is_a_birth_phase_root_without_an_authored_hook() {
    let s = snapshot(SOURCE);
    let reports = s.demand_effect_certificates().unwrap();
    let report = reports.iter().find(|(r, _)| r.subject == "Child").unwrap();
    assert_eq!(report.0.result, Verdict::Violated);
    assert!(report.1.iter().any(|(d, _)| d.message.contains("Child::birth_checks::0")));
    assert_eq!(phase_verdict(&s), Verdict::Violated);
    let m = s.demand_model().unwrap();
    let definition = &m.relations.birth_checks[0];
    let body = &m.entities.functions[definition.function.index()];
    assert_eq!(body.kind, FunctionKind::BirthCheck);
    assert_eq!(body.display, "Child::birth_checks::0");
    assert_eq!(s.identities().site(body.decl.unwrap()).unwrap().kind, hale_syntax::sites::SiteKind::BirthCheck);
    assert!(body.analyzed && body.summarized);
    assert_eq!(definition.closure, "rejected");
}

#[test]
fn unminted_predicates_keep_phase_membership_and_constructor_coverage() {
    let program = hale_syntax::parse_source(SOURCE).unwrap();
    let bundle = hale_types::Bundle::new([("app.hl".to_string(), &program)].into_iter().collect());
    let summary = hale_types::alloc_summary::derive_alloc_summary(&bundle);
    let predicate = summary.birth_checks.values().find(|c| c.owner.name == "Child").unwrap();
    assert!(predicate.owner.declaration.is_none());
    let reports = hale_types::effects::certificate_rows(&[&program], &summary);
    assert!(reports.iter().any(|r| r.subject == "Child" && r.result == Verdict::Violated));
    let main = summary.fns.values().find(|f| f.key.display() == "main").unwrap();
    assert!(main.effect_sites.iter().any(|s| matches!(s.kind,
        hale_types::alloc_summary::EffectSiteKind::BirthCheckInvocation { .. })));
}
#[test]
fn multiple_clauses_keep_separate_identities_order_and_payload_calls() {
    let source = SOURCE.replace("    birth_check { before() } -> violate rejected;", "    birth_check { false } -> violate rejected;\n    birth_check { before() } -> violate rejected(note());")
        .replace("fn before()", "fn note() -> Int { println(\"payload\"); return 1; }\nfn before()");
    let s = snapshot(&source);
    let m = s.demand_model().unwrap();
    m.validate().unwrap();
    assert_eq!(m.relations.birth_checks.len(), 2);
    let a = &m.relations.birth_checks[0];
    let b = &m.relations.birth_checks[1];
    assert_eq!((a.ordinal, b.ordinal), (0, 1));
    assert_ne!(m.entities.functions[a.function.index()].decl, m.entities.functions[b.function.index()].decl);
    let summary = s.demand_alloc_summary().unwrap();
    let second = summary.fns.iter().find(|(k, _)| k.display() == "Child::birth_checks::1").unwrap().1;
    assert!(second.calls.iter().any(|c| matches!(&c.callee, hale_types::alloc_summary::Callee::Resolved(k) if k.display() == "before")));
    assert!(second.calls.iter().any(|c| matches!(&c.callee, hale_types::alloc_summary::Callee::Resolved(k) if k.display() == "note")));
    assert!(second.violates.is_empty(), "conditional dispatch is not an unconditional source violate");
    assert_eq!(second.stack_bytes, 0, "the clause is emitted inline without a native function frame");
    assert_eq!(phase_verdict(&s), Verdict::Violated);
}
#[test]
fn a_pure_predicate_retains_conditional_failure_coverage() {
    let s = snapshot(&SOURCE.replace("println(\"birth predicate\"); ", ""));
    let reports = s.demand_effect_certificates().unwrap();
    let row = reports.iter().find(|(r, _)| r.subject == "Child").unwrap();
    assert_eq!(row.0.result, Verdict::Uncertified);
    assert!(row.1.iter().any(|(d, _)| d.message.contains("birth check")));
    assert_eq!(phase_verdict(&s), Verdict::Uncertified);
    let summary = s.demand_alloc_summary().unwrap();
    let key = summary.birth_checks.keys().next().unwrap();
    let failure = summary.structural_failure_path(key).unwrap().unwrap();
    assert!(failure.birth_check && !failure.assertion);
    assert_eq!(failure.closure, "rejected");
}
#[test]
fn birth_check_definitions_phases_and_coverage_are_mandatory() {
    let s = snapshot(SOURCE);
    let m = s.demand_model().unwrap();
    m.validate().unwrap();
    let clause = m.relations.birth_checks[0].function;
    for mutation in 0..8 {
        let mut changed = m.clone();
        match mutation {
            0 => changed.relations.birth_checks.clear(),
            1 => changed.relations.phase_of.retain(|p| p.function != clause),
            2 => changed.relations.birth_checks[0].ordinal += 1,
            3 => changed.relations.birth_checks[0].closure.clear(),
            4 => changed.entities.functions[clause.index()].kind = FunctionKind::Hook,
            5 => changed.holes.retain(|h| !(h.at == EntityRef::Function(clause) && h.kind == HoleKind::UnmodeledFailureDispatch)),
            6 => changed.holes.retain(|h| h.kind != HoleKind::UnmodeledInitialization),
            _ => {
                let phase = changed.relations.phase_of.iter().find(|p| p.function == clause).unwrap().phase;
                changed.entities.phases[phase.index()].name = "run".into();
            }
        }
        assert!(changed.validate().is_err(), "mutation {mutation}");
    }
}
#[test]
fn graph_phase_claims_retain_the_predicate_helper_path() {
    let source = SOURCE.replace("@phase_effects(birth: {})", "group children = { Child };\nmain locus App { claims { quiet: forbid reaches(children, effects(stdio)) during birth; } }");
    let s = snapshot(&source);
    let m = s.demand_model().unwrap();
    m.validate().unwrap();
    let bundle = s.bundle();
    let claims = hale_types::claim_lowering::lower_claims(&bundle, m);
    let (pre, rows, proofs) = hale_types::judgment::judge_forbid_reaches_with_certificates(&claims, m, &[0]);
    assert!(pre.is_empty(), "{pre:?}");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].verdict, Verdict::Violated);
    assert!(rows[0].diags.iter().any(|d| d.message.contains("Child::birth_checks::0") && d.message.contains("before")));
    proofs[&rows[0].ordinal].validate().unwrap();
}
#[test]
fn constructor_predicate_work_remains_known_before_runtime_context_closes() {
    let source = SOURCE.replace("@phase_effects(birth: {})", "").replace("fn main() { Child { }; }", "@effects(none: {stdio}) fn construct() { Child { }; }\n@effects(none: {stdio}) fn unrelated() { }\nfn main() { construct(); }");
    let s = snapshot(&source);
    let reports = s.demand_effect_certificates().unwrap();
    assert_eq!(reports.iter().find(|(r, _)| r.subject == "construct").unwrap().0.result, Verdict::Violated);
    assert_eq!(reports.iter().find(|(r, _)| r.subject == "unrelated").unwrap().0.result, Verdict::Holds);
    let m = s.demand_model().unwrap();
    m.validate().unwrap();
    let construction = m.relations.constructions.iter().find(|c| m.entities.functions[c.function.index()].display == "construct").unwrap();
    assert!(m.holes.iter().any(|h| h.at == EntityRef::Function(construction.function) && h.kind == HoleKind::UnmodeledInitialization && h.provenance == construction.provenance));
}
#[test]
fn imported_predicates_retain_their_actual_locus_and_source_universe() {
    let main = r#"
import "./lib" as lib;
@phase_effects(birth: {}) locus Child { }
fn main() { Child { }; lib::Child { }; }
"#;
    let library = SOURCE.replace("fn main() { Child { }; }", "");
    let s = entries::load_files(&[("main.hl", main), ("lib/child.hl", &library)], Config::check(true, false));
    let reports = s.demand_effect_certificates().unwrap();
    let m = s.demand_model().unwrap();
    let bundle = s.bundle();
    let table = hale_types::claim_lowering::lower_claims(&bundle, m);
    let evidence = hale_types::evidence::derive_certificate_evidence_over(&bundle, &table, m, reports, s.demand_alloc_summary().unwrap());
    let rows = hale_types::judgment::judge_certificates(&table, m, &evidence, &[0]);
    for (name, expected) in [("Child", Verdict::Holds), ("lib::Child", Verdict::Violated)] {
        let ordinal = table.rows.iter().find(|r| matches!(&r.law, hale_model::ClaimIr::PhaseEffects { locus, .. } if locus.1.display == name)).unwrap().ordinal;
        assert_eq!(rows.iter().find(|r| r.ordinal == ordinal).unwrap().verdict, expected, "{name}");
    }
    m.validate().unwrap();
    let clause = &m.relations.birth_checks[0];
    assert_eq!(m.entities.loci[clause.owner.index()].display, "lib::Child");
    let f = &m.entities.functions[clause.function.index()];
    assert_eq!(f.display, "lib::Child::birth_checks::0");
    assert_eq!(s.identities().site(f.decl.unwrap()).unwrap().kind, hale_syntax::sites::SiteKind::BirthCheck);
}

#[test]
fn stdlib_birth_checks_keep_the_constructor_coverage_boundary() {
    let source = r#"
@effects(none: {stdio}) fn probe() {
    let builder = std::bytes::BytesBuilder { };
    let size = builder.len();
}
fn main() { probe(); }
"#;
    let s = snapshot(source);
    let reports = s.demand_effect_certificates().unwrap();
    let report = reports.iter().find(|(r, _)| r.subject == "probe").unwrap();
    assert_eq!(report.0.result, Verdict::Uncertified);
    assert!(report.1.iter().any(|(d, _)| d.message.contains("birth check")), "{:?}", report.1);
    let m = s.demand_model().unwrap();
    m.validate().unwrap();
    let function = hale_model::FunctionId(m.entities.functions.iter().position(|f| f.display == "probe").unwrap() as u32);
    assert!(m.holes.iter().any(|h| h.at == EntityRef::Function(function)
        && h.kind == HoleKind::UnmodeledInitialization && h.reason.contains("birth check")),
        "represented birth work must not erase the independent predicate obligation");
}

#[test]
fn constructor_predicate_entries_keep_their_own_namespace_and_witness() {
    let source = r#"
fn operand() -> Int { println("checked"); return 0; }
type Marker { }
locus Child {
    params { n: Int = 0; }
    closure broken { captures: n; epoch inline; }
    birth_check { operand() < 0 } -> violate broken;
}
fn construct() { let marker = Marker { }; Child { }; }
group constructors = { construct };
main locus App { claims { quiet: forbid reaches(constructors, effects(stdio)); } }
fn main() { construct(); App { }; }
"#;
    let s = snapshot(source); let m = s.demand_model().unwrap(); m.validate().unwrap();
    let edge = &m.relations.constructor_checks[0];
    assert_eq!(edge.site, 1);
    assert_eq!(edge.kind, hale_model::ConstructorCheck::BirthCheck(0));
    assert!(edge.source.is_some());
    assert_eq!(m.entities.functions[edge.from.index()].display, "construct");
    assert_eq!(m.entities.functions[edge.to.index()].kind, FunctionKind::BirthCheck);
    let table = hale_types::claim_lowering::lower_claims(&s.bundle(), m);
    let (pre, rows, proofs) = hale_types::judgment::judge_forbid_reaches_with_certificates(&table, m, &[0]);
    assert!(pre.is_empty()); assert_eq!(rows[0].verdict, Verdict::Violated);
    assert!(rows[0].diags.iter().any(|d| d.message.contains("may evaluate birth check 0") && d.message.contains("operand")));
    proofs[&rows[0].ordinal].validate().unwrap();
    for mutation in 0..5 {
        let mut changed = m.clone();
        match mutation {
            0 => changed.relations.constructor_checks.clear(),
            1 => changed.relations.constructor_checks[0].site = 0,
            2 => changed.relations.constructor_checks[0].source = None,
            3 => changed.relations.constructor_checks[0].in_loop = true,
            _ => changed.relations.constructor_checks[0].kind = hale_model::ConstructorCheck::BirthCheck(1),
        }
        assert!(changed.validate().is_err(), "mutation {mutation}");
    }
}

#[test]
fn foreign_predicate_entries_bind_body_kind_source_and_context() {
    let source = r#"
fn probe() { let builder = std::bytes::BytesBuilder { }; }
fn main() { probe(); }
"#;
    let s = snapshot(source); let m = s.demand_model().unwrap(); m.validate().unwrap();
    let index = m.analyses.stdlib_absorption.iter().position(|a| a.entry_constructor_check.is_some()).unwrap();
    let entry = &m.analyses.stdlib_absorption[index];
    assert_eq!(entry.entry_constructor_check, Some(hale_model::ConstructorCheck::BirthCheck(0)));
    assert_eq!(entry.nodes[0].constructor_check, entry.entry_constructor_check);
    assert!(entry.entry_constructor_source.is_some());
    assert!(entry.entry_lifecycle.is_none() && entry.entry_initialization.is_none());
    assert!(!m.capabilities.exact_calls && !m.capabilities.exact_effects && !m.capabilities.exact_costs);
    for capability in 0..3 {
        let mut forged = m.clone();
        match capability {
            0 => forged.capabilities.exact_calls = true,
            1 => forged.capabilities.exact_effects = true,
            _ => forged.capabilities.exact_costs = true,
        }
        assert!(forged.validate().is_err(), "predicate coverage cannot claim exactness: {capability}");
    }

    for mutation in 0..6 {
        let mut changed = m.clone();
        let a = &mut changed.analyses.stdlib_absorption[index];
        match mutation {
            0 => a.entry_constructor_check = None,
            1 => a.nodes[0].constructor_check = None,
            2 => a.entry_constructor_check = Some(hale_model::ConstructorCheck::BirthCheck(1)),
            3 => a.entry_constructor_source = None,
            4 => a.entry_lifecycle = Some(hale_model::LifecycleStage::Birth),
            _ => changed.holes.retain(|h| h.at != EntityRef::Function(entry.from) || h.kind != HoleKind::UnmodeledLifecycleContext),
        }
        assert!(changed.validate().is_err(), "mutation {mutation}");
    }
}

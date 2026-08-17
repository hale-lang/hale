//! GH #476 Change 4 — every law surface lowers to one `ClaimIr`
//! variant, through the evaluator's own clause enumeration.
//!
//! Two proof obligations:
//!  1. **Exactness** per form: each surface lowers to its variant
//!     with resolved model ids, raw/display spellings, and origin.
//!  2. **Parity with the evaluator**: the claims-family rows agree
//!     with `claims_report_with_identities` outcomes on count, name,
//!     order, and constitution source — the lowering walks EXACTLY
//!     the clauses the evaluator walks — over fixtures AND the
//!     whole corpus (where the lowering must also be total and
//!     lawful).

use std::collections::BTreeMap;

use hale_model::{ClaimIr, ClaimOrigin, CountCmpIr, QuantDimIr, SetIr};
use hale_types::claim_lowering::lower_claims;
use hale_types::model_builder::derive_application_model;
use hale_types::Bundle;

fn lower(src: &str) -> (hale_model::ClaimIrTable, hale_model::ApplicationModel)
{
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut programs = BTreeMap::new();
    programs.insert("app.hl".to_string(), &program);
    let bundle = Bundle::new(programs);
    let model = derive_application_model(&bundle);
    let table = lower_claims(&bundle, &model);
    table.validate(&model).expect("lowered table is lawful");
    (table, model)
}

const ALL_FORMS: &str = r#"
type Reading { sensor: Int = 0; }
topic Readings { payload: Reading; subject: "sense.reading"; }
effect money;
@effects(is: { money })
fn spend(v: Int) -> Int { return v; }
locus Gate {
    params { n: Int = 0; }
    fn relay(v: Int) -> Int { return spend(v); }
}
locus Sink {
    bus { subscribe Readings as on_r; }
    fn on_r(r: Reading) { }
}
group wings = { Gate, Sink };
group gates = { Gate };
main locus App {
    params { g: Gate = Gate { }; s: Sink = Sink { }; }
    claims {
        iso: forbid reaches(gates, wings) via { calls } avoiding gates;
        edge: only edges gates -> wings { publish Readings; };
        spendcap: bound money <= 2 on paths from gates;
        haveit: require subscribes(some wings, topic Readings);
        vault: require sealed(all gates);
        tagged: require attributed(all syscall);
        onewriter: count publishers(topic Readings) == 0;
    }
    run() { println(1); }
}
fn main() { App { }; }
"#;

/// Every claims-block form lowers to its variant with resolved ids.
#[test]
fn every_claim_form_lowers_exactly() {
    let (t, m) = lower(ALL_FORMS);
    let claims: Vec<&hale_model::ClaimRow> = t
        .rows
        .iter()
        .filter(|r| r.origin == ClaimOrigin::Main)
        .collect();
    assert_eq!(claims.len(), 7, "one row per authored claim");
    // Ordinals are authored order.
    assert_eq!(claims[0].name, "iso");
    let ClaimIr::ForbidReaches {
        src,
        via_calls,
        via_bus,
        avoiding,
        ..
    } = &claims[0].law
    else {
        panic!("iso lowers to ForbidReaches: {:?}", claims[0].law)
    };
    assert!(matches!(src, SetIr::Group(g) if g.group.is_some()));
    assert!(*via_calls && !*via_bus);
    assert!(avoiding.as_ref().unwrap().group.is_some());
    let ClaimIr::OnlyEdges { grants, .. } = &claims[1].law else {
        panic!("edge lowers to OnlyEdges")
    };
    assert_eq!(grants.len(), 1);
    assert!(grants[0].publish && grants[0].topic.topic.is_some());
    let ClaimIr::Bound { class, limit, from } = &claims[2].law else {
        panic!("spendcap lowers to Bound")
    };
    assert_eq!(class.name, "money");
    assert_eq!(*limit, 2);
    assert!(from.group.is_some());
    let ClaimIr::RequireEndpoint {
        publishers, topic, ..
    } = &claims[3].law
    else {
        panic!("haveit lowers to RequireEndpoint")
    };
    assert!(!*publishers && topic.topic.is_some());
    assert!(matches!(&claims[4].law, ClaimIr::RequireSealed { group } if group.group.is_some()));
    assert!(matches!(&claims[5].law, ClaimIr::RequireAttributed { class } if class.name == "syscall"));
    let ClaimIr::Count { cmp, n, topic, .. } = &claims[6].law else {
        panic!("onewriter lowers to Count")
    };
    assert!(matches!(cmp, CountCmpIr::Eq) && *n == 0);
    assert!(topic.topic.is_some());
    // `@effects(is:)` did NOT lower — it is a model label, not a law.
    assert!(
        !t.rows.iter().any(|r| r.name == "spend"),
        "carries is classification, not obligation"
    );
    let _ = m;
}

/// Annotation surfaces lower with resolved fn/locus ids; a
/// constitution clause carries its origin.
#[test]
fn annotations_and_origins_lower() {
    let src = r#"
type T { n: Int = 0; }
topic Evt { payload: T; subject: "evt"; }
effect money;
constitution Core {
    quiet: count subscribers(topic Evt) == 0;
}
@effects(none: { syscall, block })
fn pure_math(v: Int) -> Int { return v * 2; }
@budget(stack_bytes = 256) fn tight(v: Int) -> Int { return v; }
@budget(alloc_per_call = 0) fn lean(v: Int) -> Int { return v; }
@no_panic
fn safe(v: Int) -> Int { return v; }
@effects(depends: { "evt" })
@phase_effects(birth: { alloc }, run: {})
locus Worker {
    params { n: Int = 0; }
    bus { subscribe Evt as on_e; }
    @effects(only: { alloc })
    fn on_e(t: T) { }
}
main locus App {
    params { w: Worker = Worker { }; }
    claims { adopt Core; }
    run() { println(1); }
}
fn main() { App { }; }
"#;
    let (t, _m) = lower(src);
    let by = |name: &str, pred: &dyn Fn(&ClaimIr) -> bool| {
        t.rows
            .iter()
            .find(|r| r.name == name && pred(&r.law))
            .unwrap_or_else(|| {
                panic!(
                    "no `{}` row: {:?}",
                    name,
                    t.rows
                        .iter()
                        .map(|r| (&r.name, &r.law))
                        .collect::<Vec<_>>()
                )
            })
    };
    // Adopted clause: constitution origin.
    let quiet =
        by("quiet", &|l| matches!(l, ClaimIr::Count { .. }));
    assert_eq!(
        quiet.origin,
        ClaimOrigin::Constitution {
            name: "Core".to_string()
        }
    );
    // @effects(none:) — resolved fn id.
    let forbid = by("pure_math", &|l| {
        matches!(l, ClaimIr::EffectForbid { .. })
    });
    let ClaimIr::EffectForbid { at, classes } = &forbid.law else {
        unreachable!()
    };
    assert!(at.0.is_some());
    assert_eq!(
        classes.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
        ["syscall", "block"]
    );
    by("safe", &|l| matches!(l, ClaimIr::NoPanic { .. }));
    by("lean", &|l| {
        matches!(l, ClaimIr::AllocBudget { per_call: 0, .. })
    });
    by("tight", &|l| {
        matches!(
            l,
            ClaimIr::QuantBudget {
                dim: QuantDimIr::StackBytes,
                limit: 256,
                ..
            }
        )
    });
    // Locus surfaces — resolved locus ids, method fn resolved.
    let dep = by("Worker", &|l| {
        matches!(l, ClaimIr::DependsSet { .. })
    });
    let ClaimIr::DependsSet { locus, entries } = &dep.law else {
        unreachable!()
    };
    assert!(locus.0.is_some());
    let hale_model::BusSelector::Match { subjects, .. } = &entries[0]
    else {
        panic!("unqualified depends entry is a selector")
    };
    assert!(!subjects.is_empty(), "subject `evt` resolves");
    let pe = by("Worker", &|l| {
        matches!(l, ClaimIr::PhaseEffects { .. })
    });
    let ClaimIr::PhaseEffects { phases, .. } = &pe.law else {
        unreachable!()
    };
    assert_eq!(phases.len(), 2);
    assert_eq!(phases[0].0, "birth");
    assert_eq!(phases[0].1[0].name, "alloc");
    assert!(phases[1].1.is_empty(), "run: {{}} forbids all");
    let only = by("Worker::on_e", &|l| {
        matches!(l, ClaimIr::EffectOnly { .. })
    });
    let ClaimIr::EffectOnly { at, .. } = &only.law else {
        unreachable!()
    };
    assert!(at.0.is_some(), "locus method resolves");
    assert!(t.rows.iter().all(|r| !r.name.is_empty()));
}

/// THE parity property: the claims-family rows match the
/// evaluator's outcomes on count, name, authored order, and
/// constitution source — over the whole corpus, where the lowering
/// must also be total (no panic) and lawful (validate).
#[test]
fn lowering_matches_evaluator_outcomes_over_the_corpus() {
    let mut bad: Vec<String> = Vec::new();
    let mut with_laws = 0usize;
    for p in
        hale_corpus::parseable(|s| hale_syntax::parse_source(s).is_ok())
    {
        let Ok(program) = hale_syntax::parse_source(&p.source) else {
            continue;
        };
        let caught = std::panic::catch_unwind(
            std::panic::AssertUnwindSafe(|| {
                let mut programs = BTreeMap::new();
                programs.insert("app.hl".to_string(), &program);
                let bundle = Bundle::new(programs);
                let model = derive_application_model(&bundle);
                let table = lower_claims(&bundle, &model);
                table
                    .validate(&model)
                    .map_err(|e| format!("{:?}", e))?;
                let programs_v: Vec<&hale_syntax::ast::Program> =
                    vec![&program];
                let top =
                    hale_types::resolve::build_top_scope(&bundle).0;
                let graph = hale_types::bus_graph::build_bus_graph(
                    &bundle, &top,
                );
                let (_d, outcomes, _a) =
                    hale_types::claims::claims_report_with_identities(
                        &programs_v,
                        &graph,
                        &[],
                    );
                let lowered: Vec<(String, Option<String>)> = table
                    .rows
                    .iter()
                    .filter_map(|r| match &r.origin {
                        ClaimOrigin::Main => {
                            Some((r.name.clone(), None))
                        }
                        ClaimOrigin::Constitution { name } => Some((
                            r.name.clone(),
                            Some(name.clone()),
                        )),
                        ClaimOrigin::Library { .. } => {
                            Some((r.name.clone(), None))
                        }
                        _ => None,
                    })
                    .collect();
                let evaluated: Vec<(String, Option<String>)> =
                    outcomes
                        .iter()
                        .map(|o| (o.name.clone(), o.source.clone()))
                        .collect();
                if lowered != evaluated {
                    return Err(format!(
                        "lowered {:?} != evaluated {:?}",
                        lowered, evaluated
                    ));
                }
                Ok::<usize, String>(outcomes.len())
            }),
        );
        match caught {
            Err(_) => bad.push(format!("{}: PANIC", p.origin)),
            Ok(Err(e)) => {
                bad.push(format!("{}: {}", p.origin, e))
            }
            Ok(Ok(n)) => {
                if n > 0 {
                    with_laws += 1;
                }
            }
        }
    }
    assert!(
        with_laws > 10,
        "the corpus must exercise claims ({} programs had any)",
        with_laws
    );
    assert!(
        bad.is_empty(),
        "{} corpus programs diverge:\n{}",
        bad.len(),
        bad.join("\n")
    );
}

/// P1 (round 15): law-SELECTION invalidity is preserved as
/// structured issues — `adopt Missing` produced an evaluator
/// diagnostic but lowered to an empty table, so an IR-only
/// evaluator would have observed "no law" with nothing to reject.
#[test]
fn invalid_law_selection_becomes_issues() {
    let src = r#"
main locus App {
    claims { adopt Missing; }
    run() { println(1); }
}
fn main() { App { }; }
"#;
    let (t, _m) = lower(src);
    assert!(t.rows.is_empty());
    assert!(
        t.issues
            .iter()
            .any(|i| i.message.contains("Missing")),
        "the unknown constitution is a structured issue: {:?}",
        t.issues
    );
}

/// P1 (round 15): the effect-class vocabulary is typed. A declared
/// class resolves to a table row with `declared: true`; a bare
/// reference (interned typo) resolves to a row with
/// `declared: false`; a composed class carries its normalized
/// atomic expansion.
#[test]
fn effect_class_vocabulary_is_typed() {
    let declared_src = r#"
effect money;
effect io = { syscall, block };
@effects(none: { money, io })
fn f(v: Int) -> Int { return v; }
main locus App { run() { println(f(1)); } }
fn main() { App { }; }
"#;
    let (t, m) = lower(declared_src);
    let ClaimIr::EffectForbid { classes, .. } = &t
        .rows
        .iter()
        .find(|r| matches!(r.law, ClaimIr::EffectForbid { .. }))
        .expect("forbid row")
        .law
    else {
        unreachable!()
    };
    let money = &classes[0];
    assert!(!money.builtin);
    let row = &m.entities.effect_classes
        [money.class.expect("declared class resolves").index()];
    assert!(row.declared);
    assert_eq!(
        row.definition,
        hale_model::EffectClassDefinition::Atomic
    );
    let io = &classes[1];
    let io_row = &m.entities.effect_classes
        [io.class.expect("composed class resolves").index()];
    assert!(io_row.declared);
    assert_eq!(
        io_row.definition,
        hale_model::EffectClassDefinition::Composed {
            atoms: vec!["block".to_string(), "syscall".to_string()]
        }
    );

    let typo_src = r#"
@effects(none: { money })
fn f(v: Int) -> Int { return v; }
main locus App { run() { println(f(1)); } }
fn main() { App { }; }
"#;
    let (t2, m2) = lower(typo_src);
    let ClaimIr::EffectForbid { classes, .. } = &t2
        .rows
        .iter()
        .find(|r| matches!(r.law, ClaimIr::EffectForbid { .. }))
        .expect("forbid row")
        .law
    else {
        unreachable!()
    };
    let interned = &m2.entities.effect_classes
        [classes[0].class.expect("interned ref resolves").index()];
    assert!(
        !interned.declared,
        "a bare reference is distinguishable from a declaration"
    );
    // Built-ins never resolve into the user table.
    let (t3, _m3) = lower(
        r#"
@effects(none: { syscall })
fn f(v: Int) -> Int { return v; }
main locus App { run() { println(f(1)); } }
fn main() { App { }; }
"#,
    );
    let ClaimIr::EffectForbid { classes, .. } = &t3.rows[0].law
    else {
        unreachable!()
    };
    assert!(classes[0].builtin && classes[0].class.is_none());
}

/// P1 (round 15): `@effects(publish: {Orders})` is TOPIC-space —
/// the entry resolves to the declared topic even though its wire
/// subject is different, never to a subject-pattern lookup miss.
#[test]
fn publish_set_is_topic_space() {
    let src = r#"
type Order { n: Int = 0; }
topic Orders { payload: Order; subject: "wire.orders"; }
main locus App {
    bus { publish Orders; }
    @effects(publish: { Orders })
    fn send(v: Int) { Orders <- Order { }; }
    run() { self.send(1); }
}
fn main() { App { }; }
"#;
    let (t, m) = lower(src);
    let ClaimIr::EffectPublishSet { entries, .. } = &t
        .rows
        .iter()
        .find(|r| matches!(r.law, ClaimIr::EffectPublishSet { .. }))
        .expect("publish-set row")
        .law
    else {
        unreachable!()
    };
    let hale_model::BusSelector::Match { name, topics, .. } =
        &entries[0]
    else {
        panic!("unqualified spelling is a candidate-set selector")
    };
    assert_eq!(name, "Orders");
    assert_eq!(topics.len(), 1, "the local topic is the candidate");
    assert_eq!(m.entities.topics[topics[0].index()].name, "Orders");
}

/// P1 (round 15): a resolved reference takes its name/display from
/// the model ENTITY — an imported locus method's display demangles
/// the locus half (`kv::Store::bump`), which a whole-symbol lookup
/// missed. Per-reference provenance is distinct from the row's.
#[test]
fn resolved_refs_are_entity_sourced() {
    let lib = r#"
locus __lib_x_kv_Store {
    params { n: Int = 0; }
    @no_panic
    fn bump(v: Int) -> Int { return v; }
}
"#;
    let main_src = r#"
main locus App {
    params { s: __lib_x_kv_Store = __lib_x_kv_Store { }; }
    run() { println(self.s.bump(1)); }
}
fn main() { App { }; }
"#;
    let main_p = hale_syntax::parse_source(main_src).expect("parse");
    let lib_p = hale_syntax::parse_source(lib).expect("parse lib");
    let mut programs = BTreeMap::new();
    programs.insert("app/main.hl".to_string(), &main_p);
    programs.insert("lib/kv.hl".to_string(), &lib_p);
    let mut bundle = Bundle::new(programs);
    bundle.import_renames = vec![(
        vec!["kv".to_string(), "Store".to_string()],
        "__lib_x_kv_Store".to_string(),
    )];
    let model = derive_application_model(&bundle);
    let t = lower_claims(&bundle, &model);
    t.validate(&model).expect("lawful");
    let row = t
        .rows
        .iter()
        .find(|r| matches!(r.law, ClaimIr::NoPanic { .. }))
        .expect("annotation on the imported method lowers");
    let ClaimIr::NoPanic { at } = &row.law else { unreachable!() };
    assert!(at.0.is_some());
    assert_eq!(at.1.raw, "__lib_x_kv_Store::bump");
    assert_eq!(
        at.1.display, "kv::Store::bump",
        "display demangles the locus half (entity-sourced)"
    );
}

/// P1 (round 15): source-bearing references carry their OWN
/// provenance, anchored at the reference — not the clause.
#[test]
fn references_carry_their_own_provenance() {
    let (t, _m) = lower(ALL_FORMS);
    let row = &t.rows[0];
    let ClaimIr::ForbidReaches { src, avoiding, .. } = &row.law
    else {
        unreachable!()
    };
    let SetIr::Group(g) = src else { unreachable!() };
    assert_ne!(
        g.provenance, row.provenance,
        "the group ref anchors at its own span"
    );
    assert_ne!(
        avoiding.as_ref().unwrap().provenance,
        row.provenance
    );
}

/// P1 (round 16): a cyclic effect-class definition stays
/// distinguishable from an atomic class — the evaluator rejects it
/// at the declaration because it resolves to no effect and makes
/// contracts vacuous, and the model preserves that as
/// `EffectClassDefinition::InvalidCycle`.
#[test]
fn cyclic_effect_class_definitions_stay_invalid() {
    let src = r#"
effect a = { b };
effect b = { a };
@effects(none: { a })
fn f(v: Int) -> Int { return v; }
main locus App { run() { println(f(1)); } }
fn main() { App { }; }
"#;
    let (t, m) = lower(src);
    let ClaimIr::EffectForbid { classes, .. } = &t
        .rows
        .iter()
        .find(|r| matches!(r.law, ClaimIr::EffectForbid { .. }))
        .expect("forbid row")
        .law
    else {
        unreachable!()
    };
    let a_row = &m.entities.effect_classes
        [classes[0].class.expect("`a` resolves").index()];
    assert_eq!(
        a_row.definition,
        hale_model::EffectClassDefinition::InvalidCycle,
        "a cycle is NOT an atomic class"
    );
    let b_row = m
        .entities
        .effect_classes
        .iter()
        .find(|c| c.name == "b")
        .expect("b row");
    assert_eq!(
        b_row.definition,
        hale_model::EffectClassDefinition::InvalidCycle
    );
}

/// P1 (round 16): an UNQUALIFIED `@effects(publish:)` reference
/// authored inside an imported seed resolves by trailing name to
/// the merged (mangled) topic — the evaluator's documented
/// cross-seed rule; the library author cannot know the consumer's
/// alias.
#[test]
fn unqualified_imported_publish_ref_gets_candidates() {
    let lib = r#"
type __lib_r_main_Alert { n: Int = 0; }
topic __lib_r_main_Recalled { payload: __lib_r_main_Alert; }
locus __lib_r_main_Relay {
    params { n: Int = 0; }
    bus { publish __lib_r_main_Recalled; }
    @effects(publish: { Recalled })
    fn emit(v: Int) { __lib_r_main_Recalled <- __lib_r_main_Alert { }; }
}
"#;
    let main_src = r#"
main locus App {
    params { r: __lib_r_main_Relay = __lib_r_main_Relay { }; }
    run() { self.r.emit(1); }
}
fn main() { App { }; }
"#;
    let main_p = hale_syntax::parse_source(main_src).expect("parse");
    let lib_p = hale_syntax::parse_source(lib).expect("parse lib");
    let mut programs = BTreeMap::new();
    programs.insert("app/main.hl".to_string(), &main_p);
    programs.insert("lib/relay.hl".to_string(), &lib_p);
    let mut bundle = Bundle::new(programs);
    bundle.import_renames = vec![
        (
            vec!["relay".to_string(), "Alert".to_string()],
            "__lib_r_main_Alert".to_string(),
        ),
        (
            vec!["relay".to_string(), "Recalled".to_string()],
            "__lib_r_main_Recalled".to_string(),
        ),
        (
            vec!["relay".to_string(), "Relay".to_string()],
            "__lib_r_main_Relay".to_string(),
        ),
    ];
    let model = derive_application_model(&bundle);
    let t = lower_claims(&bundle, &model);
    t.validate(&model).expect("lawful");
    let ClaimIr::EffectPublishSet { entries, .. } = &t
        .rows
        .iter()
        .find(|r| matches!(r.law, ClaimIr::EffectPublishSet { .. }))
        .expect("the imported seed's annotation lowers")
        .law
    else {
        unreachable!()
    };
    let hale_model::BusSelector::Match { name, topics, .. } =
        &entries[0]
    else {
        panic!("unqualified library spelling is a candidate set")
    };
    assert_eq!(name, "Recalled");
    assert_eq!(topics.len(), 1);
    assert_eq!(
        model.entities.topics[topics[0].index()].name,
        "__lib_r_main_Recalled",
        "trailing-name match reaches the merged topic"
    );
}

/// P1 (round 17): a LITERAL wire subject in `@effects(publish:)` is
/// a valid certificate the evaluator accepts — the selector must
/// resolve it (as a subject candidate), never document it as
/// unresolved residue.
#[test]
fn literal_publish_subject_resolves_as_subject_candidate() {
    let src = r#"
type Event { n: Int = 0; }
main locus App {
    bus { publish "audit.log" of type Event; }
    @effects(publish: { "audit.log" })
    fn emit(v: Int) { "audit.log" <- Event { }; }
    run() { self.emit(1); }
}
fn main() { App { }; }
"#;
    let (t, m) = lower(src);
    let ClaimIr::EffectPublishSet { entries, .. } = &t
        .rows
        .iter()
        .find(|r| matches!(r.law, ClaimIr::EffectPublishSet { .. }))
        .expect("publish-set row")
        .law
    else {
        unreachable!()
    };
    let hale_model::BusSelector::Match {
        subjects, topics, ..
    } = &entries[0]
    else {
        panic!("literal entry is a selector")
    };
    assert!(topics.is_empty());
    assert_eq!(subjects.len(), 1, "the literal wire subject resolves");
    assert_eq!(
        m.entities.subjects[subjects[0].index()].pattern,
        "audit.log"
    );
}

/// P1 (round 17): an UNQUALIFIED `@effects(depends:)` entry authored
/// in an imported seed tail-matches the merged wire subject — the
/// evaluator's `topic_ref_matches` rule, now shared by both bus-set
/// surfaces.
#[test]
fn unqualified_imported_depends_entry_resolves() {
    let lib = r#"
type __lib_r_main_Alert { n: Int = 0; }
topic __lib_r_main_Recalled { payload: __lib_r_main_Alert; }
@effects(depends: { "Recalled" })
locus __lib_r_main_Sink {
    params { n: Int = 0; }
    bus { subscribe __lib_r_main_Recalled as on_r; }
    fn on_r(a: __lib_r_main_Alert) { }
}
"#;
    let main_src = r#"
main locus App {
    params { s: __lib_r_main_Sink = __lib_r_main_Sink { }; }
    run() { println(1); }
}
fn main() { App { }; }
"#;
    let main_p = hale_syntax::parse_source(main_src).expect("parse");
    let lib_p = hale_syntax::parse_source(lib).expect("parse lib");
    let mut programs = BTreeMap::new();
    programs.insert("app/main.hl".to_string(), &main_p);
    programs.insert("lib/relay.hl".to_string(), &lib_p);
    let mut bundle = Bundle::new(programs);
    bundle.import_renames = vec![
        (
            vec!["relay".to_string(), "Alert".to_string()],
            "__lib_r_main_Alert".to_string(),
        ),
        (
            vec!["relay".to_string(), "Recalled".to_string()],
            "__lib_r_main_Recalled".to_string(),
        ),
        (
            vec!["relay".to_string(), "Sink".to_string()],
            "__lib_r_main_Sink".to_string(),
        ),
    ];
    let model = derive_application_model(&bundle);
    let t = lower_claims(&bundle, &model);
    t.validate(&model).expect("lawful");
    let ClaimIr::DependsSet { entries, .. } = &t
        .rows
        .iter()
        .find(|r| matches!(r.law, ClaimIr::DependsSet { .. }))
        .expect("the imported seed's depends lowers")
        .law
    else {
        unreachable!()
    };
    let hale_model::BusSelector::Match {
        name,
        topics,
        subjects,
        ..
    } = &entries[0]
    else {
        panic!("unqualified depends entry is a selector")
    };
    assert_eq!(name, "Recalled");
    assert!(
        !topics.is_empty() || !subjects.is_empty(),
        "trailing-name match reaches the merged topic/subject"
    );
    assert!(
        topics
            .iter()
            .any(|t| model.entities.topics[t.index()].name
                == "__lib_r_main_Recalled"),
        "the merged topic is a candidate"
    );
}

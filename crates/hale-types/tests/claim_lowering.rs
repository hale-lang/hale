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
    let ClaimIr::DependsSet { locus, subjects } = &dep.law else {
        unreachable!()
    };
    assert!(locus.0.is_some());
    assert!(subjects[0].0.is_some(), "subject `evt` resolves");
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

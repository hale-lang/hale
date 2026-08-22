//! GH #476 Change 5f — `@effects(causes: …)` over the canonical
//! model, held against the evaluator over the corpus.
//!
//! Same discipline as 5a–5e: the engine reproduces the evaluator's
//! diagnostics, and the differential is what makes the cutover safe.
//! One divergence is expected and carved out precisely — the
//! evaluator infers effects from the USER-ONLY summary, so a
//! subscriber whose class comes from a stdlib call reads as pure to
//! it, and the causal contribution vanishes. The model's sets are
//! stdlib-merged, so it sees the class. That is the fail-closed
//! direction, and an undeclared causal effect is exactly what this
//! law exists to catch.

use std::collections::BTreeMap;

use hale_types::model_builder::derive_application_model;
use hale_types::symbol::SourceFile;
use hale_types::Bundle;

fn bundle_of<'a>(
    src: &str,
    program: &'a hale_syntax::ast::Program,
) -> Bundle<'a> {
    let mut programs = BTreeMap::new();
    programs.insert("app.hl".to_string(), program);
    let mut b = Bundle::new(programs);
    b.sources = vec![SourceFile {
        id: 0,
        path: "app.hl".to_string(),
        digest: "0".to_string(),
        base: 0,
        len: src.len() as u32,
    }];
    b
}

fn render(d: &hale_syntax::Diag) -> String {
    format!(
        "[{}..{}] {}",
        d.span.start.as_usize(),
        d.span.end.as_usize(),
        d.message
    )
}

#[test]
fn causes_judgment_matches_the_evaluator_over_the_corpus() {
    let mut compared = 0usize;
    let mut with_rows = 0usize;
    let mut divergences: Vec<String> = Vec::new();
    let mut mismatches: Vec<String> = Vec::new();
    for p in
        hale_corpus::parseable(|s| hale_syntax::parse_source(s).is_ok())
    {
        let Ok(program) = hale_syntax::parse_source(&p.source) else {
            continue;
        };
        let bundle = bundle_of(&p.source, &program);
        // The model describes CHECKED programs only.
        if hale_types::check_bundle_opts(&bundle, false)
            .iter()
            .any(|d| {
                d.is_error()
                    && d.kind != hale_syntax::error::DiagKind::Claim
            })
        {
            continue;
        }
        let model = derive_application_model(&bundle);
        let table =
            hale_types::claim_lowering::lower_claims(&bundle, &model);
        let has_causes = table.rows.iter().any(|r| {
            matches!(r.law, hale_model::ClaimIr::EffectCauses { .. })
        });
        if !has_causes {
            continue;
        }
        compared += 1;
        with_rows += 1;

        let programs_v: Vec<&hale_syntax::ast::Program> = vec![&program];
        let (top, _) = hale_types::resolve::build_top_scope(&bundle);
        let graph = hale_types::bus_graph::build_bus_graph(&bundle, &top);
        let old: Vec<String> =
            hale_types::frontier::causes_diags(&programs_v, &graph)
                .iter()
                .map(render)
                .collect();
        let bases: Vec<u32> =
            bundle.sources.iter().map(|f| f.base).collect();
        let new: Vec<String> =
            hale_types::judgment::judge_causes(&table, &model, &bases)
                .iter()
                .flat_map(|j| j.diags.iter())
                .map(render)
                .collect();
        if old == new {
            continue;
        }
        // The documented divergence: the model may only ADD a
        // violation the user-only summary could not see.
        let old_only: Vec<&String> =
            old.iter().filter(|l| !new.contains(l)).collect();
        if old_only.is_empty() && !new.is_empty() {
            divergences.push(p.origin.clone());
            continue;
        }
        mismatches.push(format!(
            "--- {} ---\n  evaluator:\n{}\n  model:\n{}",
            p.origin,
            old.iter()
                .map(|l| format!("    {}", l))
                .collect::<Vec<_>>()
                .join("\n"),
            new.iter()
                .map(|l| format!("    {}", l))
                .collect::<Vec<_>>()
                .join("\n"),
        ));
    }
    assert!(
        with_rows > 0,
        "the corpus must exercise `causes:` — the differential \
         would pass vacuously"
    );
    assert!(
        mismatches.is_empty(),
        "{} of {} programs diverge outside the documented class:\n{}",
        mismatches.len(),
        compared,
        mismatches.join("\n")
    );
    eprintln!(
        "causes differential: {} programs, {} documented divergences",
        compared,
        divergences.len()
    );
}

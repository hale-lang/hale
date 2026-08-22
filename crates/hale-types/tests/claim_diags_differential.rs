//! GH #476 Change 9 — the check path's claim diagnostics, from one
//! authority.
//!
//! `hale check` reported these four families from a second
//! evaluator (`claims.rs`) that re-derived them from source, while
//! the artifact reported them from the judgment engines over the
//! canonical model. Change 9 deletes the duplicate: check now calls
//! `judgment::claim_law_diags`.
//!
//! This differential is what makes that safe. It holds the two
//! authorities byte-equal over every corpus program — same
//! messages, same spans, same order — for as long as the legacy
//! evaluator still exists to compare against. It is deliberately
//! written as a comparison, not a golden: a golden would only prove
//! the new engine is self-consistent, and the property that matters
//! during a cutover is that users' diagnostics do not move.

use std::collections::BTreeMap;

use hale_types::symbol::SourceFile;
use hale_types::Bundle;

/// Render a diagnostic to the comparable string: kind, span, and
/// message, plus every related note (spelling AND placement are
/// part of the parity claim — a witness that lost its related
/// spans would still read the same in the message).
fn render(d: &hale_syntax::Diag) -> String {
    let mut s = format!(
        "{:?} [{}..{}] {}",
        d.kind,
        d.span.start.as_usize(),
        d.span.end.as_usize(),
        d.message
    );
    for r in &d.related {
        s.push_str(&format!(
            "\n    related [{}..{}] {}",
            r.0.start.as_usize(),
            r.0.end.as_usize(),
            r.1
        ));
    }
    s
}

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

/// The legacy evaluator's diagnostics, restricted to the families
/// the judgment engines own. The legacy arm emits one flat list for
/// every family it judges, and Change 5 migrated exactly these
/// four; `causes`/`depends`/`@budget` never went through it.
fn legacy_arm(bundle: &Bundle<'_>) -> Vec<String> {
    let programs: Vec<&hale_syntax::ast::Program> =
        bundle.programs.values().copied().collect();
    let (top, _) = hale_types::resolve::build_top_scope(bundle);
    let graph = hale_types::bus_graph::build_bus_graph(bundle, &top);
    hale_types::claims::claims_diags(
        &programs,
        &graph,
        &bundle.import_renames,
    )
    .iter()
    .map(render)
    .collect()
}

/// The MODEL arm, as `check` will call it: law selection from its
/// one authority, then the judged laws from theirs.
fn model_arm(bundle: &Bundle<'_>) -> Vec<String> {
    let programs: Vec<&hale_syntax::ast::Program> =
        bundle.programs.values().copied().collect();
    let (top, _) = hale_types::resolve::build_top_scope(bundle);
    let graph = hale_types::bus_graph::build_bus_graph(bundle, &top);
    let mut out = hale_types::claims::selection_diags(
        &programs,
        &graph,
        &bundle.import_renames,
    );
    out.extend(hale_types::judgment::claim_law_diags(bundle));
    out.iter().map(render).collect()
}

#[test]
fn claim_diagnostics_match_the_evaluator_over_the_corpus() {
    let mut compared = 0usize;
    let mut with_diags = 0usize;
    let mut mismatches: Vec<String> = Vec::new();
    let mut documented_divergences = 0usize;
    for program in hale_corpus::all() {
        let Ok(parsed) = hale_syntax::parse_source(&program.source) else {
            continue;
        };
        let bundle = bundle_of(&program.source, &parsed);
        // A model of an ill-typed program describes nothing; the
        // check path only reaches the law block when the bundle
        // typechecks, so hold this differential to the same gate.
        if hale_types::check_bundle_opts(&bundle, false)
            .iter()
            .any(|d| {
                d.is_error()
                    && d.kind != hale_syntax::error::DiagKind::Claim
            })
        {
            continue;
        }
        compared += 1;
        let legacy = legacy_arm(&bundle);
        let model = model_arm(&bundle);
        if !legacy.is_empty() {
            with_diags += 1;
        }
        if legacy != model {
            // The ONE documented divergence (GH #476 Change 5c,
            // recorded by the artifact's `semantics` bump to 2):
            // `require attributed` over a body with an indirect or
            // opaque call is `uncertified`, where the evaluator
            // fail-OPEN held. The model engine is stricter on
            // purpose, so check gains a diagnostic here — a
            // correction, not a regression. Bounded precisely: the
            // model may only ADD, and only `uncertified` lines.
            let legacy_only: Vec<&String> =
                legacy.iter().filter(|l| !model.contains(l)).collect();
            let model_only: Vec<&String> =
                model.iter().filter(|l| !legacy.contains(l)).collect();
            if legacy_only.is_empty()
                && !model_only.is_empty()
                && model_only
                    .iter()
                    .all(|l| l.contains("uncertified:"))
            {
                documented_divergences += 1;
                continue;
            }
            mismatches.push(format!(
                "--- {} ---\n  legacy ({}):\n{}\n  model ({}):\n{}",
                program.origin,
                legacy.len(),
                legacy
                    .iter()
                    .map(|l| format!("    {}", l))
                    .collect::<Vec<_>>()
                    .join("\n"),
                model.len(),
                model
                    .iter()
                    .map(|l| format!("    {}", l))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ));
        }
    }
    assert!(
        compared > 100,
        "differential covered too little: {} programs",
        compared
    );
    assert!(
        with_diags > 0,
        "no corpus program produced a claim diagnostic — the \
         differential would pass vacuously"
    );
    assert!(
        documented_divergences > 0,
        "the corpus no longer exercises the ONE documented \
         divergence (require-attributed over an opaque call, \
         fail-open -> uncertified) — either it regressed to \
         fail-open, or the fixture that proved it is gone"
    );
    assert!(
        mismatches.is_empty(),
        "{} of {} programs disagree between the two authorities:\n{}",
        mismatches.len(),
        compared,
        mismatches.join("\n")
    );
}

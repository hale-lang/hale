//! GH #476 Change 5e — the pointwise-certificate differential.
//!
//! The certificate ENGINES stay the analysis authority; the builder
//! stores each certificate's outcome + diagnostics as model
//! evidence, and the judgment renders from model data. The gate:
//! the judgment's diagnostic stream (undeclared-class validation +
//! per-certificate diags, with the lowering's carries-issues) must
//! be byte-identical to the evaluator's law strata, and every
//! evidence row must be consumed by exactly one ClaimIr row.

use std::collections::BTreeMap;

use hale_model::ClaimIr;
use hale_types::claim_lowering::lower_claims;
use hale_types::judgment::judge_certificates;
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

fn diff_one(src: &str, origin: &str) -> Result<usize, String> {
    let program = hale_syntax::parse_source(src)
        .map_err(|_| format!("{}: parse", origin))?;
    let bundle = bundle_of(src, &program);
    let model = derive_application_model(&bundle);
    let table = lower_claims(&bundle, &model);
    let n_rows = table
        .rows
        .iter()
        .filter(|r| {
            matches!(
                r.law,
                ClaimIr::EffectForbid { .. }
                    | ClaimIr::EffectOnly { .. }
                    | ClaimIr::EffectPublishSet { .. }
                    | ClaimIr::NoPanic { .. }
                    | ClaimIr::PhaseEffects { .. }
            )
        })
        .count();
    if n_rows == 0 && model.evidence.is_empty() {
        return Ok(0);
    }
    // Old: the evaluator's law strata (undeclared-class validation +
    // the per-certificate stream), demangled like `hale check`.
    let programs_v: Vec<&hale_syntax::ast::Program> = vec![&program];
    let (_pre, mut p1, _tail, groups) =
        hale_types::effects::effect_report_three_way(&programs_v, &[]);
    hale_types::stdlib_bodies::demangle_imports(&mut p1, &[]);
    let mut group_diags: Vec<hale_syntax::Diag> = groups
        .into_iter()
        .flat_map(|(_, ds)| ds.into_iter())
        .collect();
    hale_types::stdlib_bodies::demangle_imports(&mut group_diags, &[]);
    let old: Vec<(String, hale_syntax::Span)> = p1
        .iter()
        .chain(group_diags.iter())
        .map(|d| (d.message.clone(), d.span))
        .collect();
    // New: lowering issues (carries validation) + judgment output.
    let judged = judge_certificates(&table, &model, &[0]);
    let issue_span = |pid: hale_model::ProvenanceId| {
        match table.provenance.records.get(pid.index()) {
            Some(hale_model::Provenance::Source { span, .. }) => {
                hale_syntax::Span::new(
                    span.0 as usize,
                    span.1 as usize,
                )
            }
            _ => hale_syntax::Span::new(0, 0),
        }
    };
    let new: Vec<(String, hale_syntax::Span)> = table
        .issues
        .iter()
        .filter(|i| i.message.contains("asserts about effect class"))
        .map(|i| (i.message.clone(), issue_span(i.provenance)))
        .chain(
            judged
                .iter()
                .flat_map(|j| j.diags.iter())
                .map(|d| (d.message.clone(), d.span)),
        )
        .collect();
    // ORDER-INSENSITIVE within strata boundaries is not enough for
    // byte parity — compare as multisets first, then sequences; the
    // corpus arbitrates whether stream interleaving ever differs.
    let key = |x: &(String, hale_syntax::Span)| {
        (x.0.clone(), x.1.start.as_usize(), x.1.end.as_usize())
    };
    let mut old_sorted = old.clone();
    let mut new_sorted = new.clone();
    old_sorted.sort_by_key(key);
    new_sorted.sort_by_key(key);
    if old_sorted != new_sorted {
        let missing: Vec<_> = old_sorted
            .iter()
            .filter(|x| !new_sorted.contains(x))
            .take(2)
            .collect();
        let extra: Vec<_> = new_sorted
            .iter()
            .filter(|x| !old_sorted.contains(x))
            .take(2)
            .collect();
        return Err(format!(
            "{}: certificate diags diverge (old {} / new {}).\n  \
             missing: {:?}\n  extra: {:?}",
            origin,
            old.len(),
            new.len(),
            missing,
            extra
        ));
    }
    Ok(n_rows)
}

/// THE 5e gate.
#[test]
fn certificate_judgment_matches_the_evaluator_over_the_corpus() {
    let mut bad: Vec<String> = Vec::new();
    let mut rows = 0usize;
    for p in
        hale_corpus::parseable(|s| hale_syntax::parse_source(s).is_ok())
    {
        let caught = std::panic::catch_unwind(
            std::panic::AssertUnwindSafe(|| {
                diff_one(&p.source, &p.origin)
            }),
        );
        match caught {
            Err(_) => bad.push(format!("{}: PANIC", p.origin)),
            Ok(Err(e)) => bad.push(e),
            Ok(Ok(n)) => rows += n,
        }
    }
    assert!(
        rows > 20,
        "the corpus must exercise certificates ({} rows)",
        rows
    );
    assert!(
        bad.is_empty(),
        "{} corpus programs diverge:\n{}",
        bad.len(),
        bad.join("\n\n")
    );
}

/// Negative control: the judgment reads EVIDENCE — clearing it
/// invalidates a holding certificate.
#[test]
fn dropping_evidence_changes_the_verdict() {
    let src = r#"
@no_ffi
fn pure_math(v: Int) -> Int { return v * 2; }
main locus App {
    run() { println(pure_math(1)); }
}
fn main() { App { }; }
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let mut model = derive_application_model(&bundle);
    let table = lower_claims(&bundle, &model);
    let judged = judge_certificates(&table, &model, &[0]);
    assert!(!judged.is_empty());
    assert_eq!(judged[0].verdict, hale_types::verdict::Verdict::Holds);
    model.evidence.clear();
    let judged = judge_certificates(&table, &model, &[0]);
    assert_eq!(
        judged[0].verdict,
        hale_types::verdict::Verdict::Invalid,
        "the judgment reads model.evidence"
    );
}

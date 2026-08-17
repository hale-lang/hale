//! GH #476 Change 5a — the reachability-family differential.
//!
//! For every corpus program with `forbid reaches` claims, the new
//! judgment engine (`judgment::judge_forbid_reaches` over
//! `ClaimIr` × `ApplicationModel`) must agree with the
//! authoritative evaluator on VERDICTS and emit byte-identical
//! DIAGNOSTICS (message and span) for the family. The old
//! evaluator stays authoritative until Change 9; this differential
//! is the permanent gate that keeps the two in lockstep until the
//! cutover.

use std::collections::BTreeMap;

use hale_model::ClaimIr;
use hale_types::claim_lowering::lower_claims;
use hale_types::judgment::judge_forbid_reaches;
use hale_types::model_builder::derive_application_model;
use hale_types::symbol::SourceFile;
use hale_types::Bundle;

/// A single-program bundle WITH a populated source table, so model
/// provenance is Source-backed and the engine reconstructs the
/// evaluator's bundle-global spans exactly (base 0, one file).
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

fn family_names(
    table: &hale_model::ClaimIrTable,
) -> Vec<String> {
    table
        .rows
        .iter()
        .filter(|r| matches!(r.law, ClaimIr::ForbidReaches { .. }))
        .map(|r| r.name.clone())
        .collect()
}

/// Old-vs-new for one program. Returns Err(description) on any
/// divergence.
fn diff_one(
    src: &str,
    origin: &str,
) -> Result<usize, String> {
    let program = hale_syntax::parse_source(src)
        .map_err(|_| format!("{}: parse", origin))?;
    let bundle = bundle_of(src, &program);
    let model = derive_application_model(&bundle);
    let table = lower_claims(&bundle, &model);
    let names = family_names(&table);
    if names.is_empty() {
        return Ok(0);
    }
    // Old: evaluator outcomes + diags.
    let programs_v: Vec<&hale_syntax::ast::Program> = vec![&program];
    let top = hale_types::resolve::build_top_scope(&bundle).0;
    let graph = hale_types::bus_graph::build_bus_graph(&bundle, &top);
    let (old_diags, outcomes, _a) =
        hale_types::claims::claims_report_with_identities(
            &programs_v,
            &graph,
            &[],
        );
    // New: engine over the lowered rows.
    let (pre_diags, judged) = judge_forbid_reaches(&table, &model, &[0]);
    // Verdict parity, matched by claim name.
    let old_verdicts: BTreeMap<&str, &hale_types::verdict::Verdict> =
        outcomes
            .iter()
            .filter(|o| names.iter().any(|n| *n == o.name))
            .map(|o| (o.name.as_str(), &o.result))
            .collect();
    let by_ordinal: BTreeMap<u32, &hale_model::ClaimRow> =
        table.rows.iter().map(|r| (r.ordinal, r)).collect();
    for j in &judged {
        let row = by_ordinal[&j.ordinal];
        let Some(old) = old_verdicts.get(row.name.as_str()) else {
            return Err(format!(
                "{}: claim `{}` judged but has no outcome",
                origin, row.name
            ));
        };
        if **old != j.verdict {
            return Err(format!(
                "{}: claim `{}` verdict diverges: old {:?}, new {:?}",
                origin, row.name, old, j.verdict
            ));
        }
    }
    // Diagnostic parity: the old diags belonging to the family
    // (identified by the evaluator's own "claim `NAME`" spelling),
    // in order, must equal the engine's diags in ordinal order.
    let old_family: Vec<(String, hale_syntax::Span)> = old_diags
        .iter()
        .filter(|d| {
            names
                .iter()
                .any(|n| d.message.contains(&format!("claim `{}`", n)))
        })
        .map(|d| (d.message.clone(), d.span))
        .collect();
    let new_family: Vec<(String, hale_syntax::Span)> = pre_diags
        .iter()
        .chain(judged.iter().flat_map(|j| j.diags.iter()))
        .map(|d| (d.message.clone(), d.span))
        .collect();
    if old_family != new_family {
        let first = old_family
            .iter()
            .zip(new_family.iter())
            .position(|(a, b)| a != b)
            .unwrap_or(old_family.len().min(new_family.len()));
        return Err(format!(
            "{}: family diags diverge at {} (old {} / new {}):\n  \
             old: {:?}\n  new: {:?}",
            origin,
            first,
            old_family.len(),
            new_family.len(),
            old_family.get(first),
            new_family.get(first),
        ));
    }
    Ok(names.len())
}

/// THE 5a gate: verdict + diagnostic parity over every corpus
/// program that carries the family.
#[test]
fn reachability_judgment_matches_the_evaluator_over_the_corpus() {
    let mut bad: Vec<String> = Vec::new();
    let mut family_claims = 0usize;
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
            Ok(Ok(n)) => family_claims += n,
        }
    }
    assert!(
        family_claims > 10,
        "the corpus must exercise forbid-reaches ({} claims seen)",
        family_claims
    );
    assert!(
        bad.is_empty(),
        "{} corpus programs diverge:\n{}",
        bad.len(),
        bad.join("\n\n")
    );
}

/// Negative control: the engine READS the calls relation — clearing
/// it flips a violated claim, proving the family's verdicts derive
/// from the rows the model claims they do.
#[test]
fn dropping_call_rows_changes_the_verdict() {
    let src = r#"
locus A {
    params { n: Int = 0; }
    fn go(v: Int) -> Int { return leak(v); }
}
fn leak(v: Int) -> Int { return v; }
group a_side = { A };
group b_side = { leak };
main locus App {
    params { a: A = A { }; }
    claims { iso: forbid reaches(a_side, b_side); }
    run() { println(self.a.go(1)); }
}
fn main() { App { }; }
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let mut model = derive_application_model(&bundle);
    let table = lower_claims(&bundle, &model);
    let (_pre, judged) = judge_forbid_reaches(&table, &model, &[0]);
    assert_eq!(judged.len(), 1);
    assert_eq!(judged[0].verdict, hale_types::verdict::Verdict::Violated);
    // Drop the relation: the violation must disappear.
    model.relations.calls.clear();
    let (_pre, judged) = judge_forbid_reaches(&table, &model, &[0]);
    assert_eq!(
        judged[0].verdict,
        hale_types::verdict::Verdict::Holds,
        "the engine reads relations.calls"
    );
}

/// Negative control: the engine reads typed HOLES — clearing them
/// flips an uncertified claim to Holds, proving the fail-closed
/// verdicts derive from the model's hole rows.
#[test]
fn dropping_holes_changes_the_verdict() {
    let src = r#"
locus A {
    params { n: Int = 0; }
    fn go(f: fn (Int) -> Int, v: Int) -> Int { return f(v); }
}
fn leak(v: Int) -> Int { return v; }
group a_side = { A };
group b_side = { leak };
main locus App {
    params { a: A = A { }; }
    claims { iso: forbid reaches(a_side, b_side); }
    run() { println(1); }
}
fn main() { App { }; }
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let mut model = derive_application_model(&bundle);
    let table = lower_claims(&bundle, &model);
    let (_p, judged) = judge_forbid_reaches(&table, &model, &[0]);
    assert_eq!(
        judged[0].verdict,
        hale_types::verdict::Verdict::Uncertified,
        "the fn-typed param fails closed"
    );
    model.holes.clear();
    let (_p, judged) = judge_forbid_reaches(&table, &model, &[0]);
    assert_eq!(
        judged[0].verdict,
        hale_types::verdict::Verdict::Holds,
        "the engine reads model.holes"
    );
}

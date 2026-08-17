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

/// Review pin: a DECLARATION-ONLY free fn (empty body, no summary
/// row) is still a source/sink decl — the evaluator's fn_set
/// inserts every named free fn, and the judgment universe must
/// match (a group naming it must not become projection-vacuous).
#[test]
fn declaration_only_free_fns_stay_in_the_universe() {
    let src = r#"
fn sink() { }
locus A {
    params { n: Int = 0; }
    fn go(v: Int) { sink(); }
}
group a_side = { A };
group b_side = { sink };
main locus App {
    params { a: A = A { }; }
    claims { iso: forbid reaches(a_side, b_side); }
    run() { self.a.go(1); }
}
fn main() { App { }; }
"#;
    let legacy = {
        let program = hale_syntax::parse_source(src).expect("parse");
        let bundle = bundle_of(src, &program);
        let out = diff_one(src, "declaration-only free fn");
        let _ = bundle;
        let _ = program;
        out
    };
    assert!(legacy.is_ok(), "old/new agree: {:?}", legacy);
    // …and the verdict is the evaluator's, not vacuous-Invalid.
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let model = derive_application_model(&bundle);
    let table = lower_claims(&bundle, &model);
    let (_p, judged) = judge_forbid_reaches(&table, &model, &[0]);
    assert_eq!(
        judged[0].verdict,
        hale_types::verdict::Verdict::Violated,
        "the empty free fn is a reachable destination"
    );
}

/// Review pin: an `effects(C)` destination satisfied INSIDE a
/// stdlib body — the evaluator applies direct_effects to every
/// visited FnKey, and interior nodes must answer the test.
#[test]
fn stdlib_interior_effect_sink_is_found() {
    // The #392 Router recipe: Gate::probe's ALLOC evidence lives
    // inside std::http::Router::dispatch's body — the user fn's own
    // direct effects are clean, so only an interior node can
    // satisfy the destination.
    let src = r#"
locus Hello {
    fn handle(ctx: std::http::Context) -> std::http::Response {
        return std::http::Response {
            status: 200,
            content_type: "text/plain",
            body: "hi"
        };
    }
}
locus Gate {
    fn probe(r: std::http::Router, req: std::http::Request) -> Int {
        let resp = r.dispatch(req);
        return resp.status;
    }
}
group gates = { Gate };
main locus App {
    claims { pure: forbid reaches(gates, effects(alloc)); }
}
fn main() {
    let r = std::http::Router { };
    r.add("GET", "/", Hello { });
    let req = std::http::Request { method: "GET", path: "/", body: "" };
    println(Gate { }.probe(r, req));
}
"#;
    let out = diff_one(src, "stdlib effect sink");
    assert!(out.is_ok(), "old/new agree: {:?}", out);
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let model = derive_application_model(&bundle);
    let table = lower_claims(&bundle, &model);
    let (_p, judged) = judge_forbid_reaches(&table, &model, &[0]);
    assert_eq!(
        judged[0].verdict,
        hale_types::verdict::Verdict::Violated,
        "the alloc happens inside the stdlib body"
    );
}

/// Review pin (round 2): EVERY publish effect site consumes one
/// source-order ordinal — a computed-subject publish authored
/// before a known publish keeps its earlier position, so consumers
/// interleaving rows and holes by site see authored order.
#[test]
fn every_publish_site_consumes_an_ordinal() {
    let src = r#"
topic Sig { payload: Int; subject: "app.sig"; }
locus A {
    params { n: Int = 0; }
    fn go(v: Int) {
        self.n <- v;
        Sig <- v;
    }
}
main locus App {
    params { a: A = A { }; }
    run() { self.a.go(1); }
}
fn main() { App { }; }
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let model = derive_application_model(&bundle);
    let go = model
        .entities
        .functions
        .iter()
        .position(|f| f.display == "A::go")
        .expect("A::go");
    let hole = model
        .holes
        .iter()
        .find(|h| {
            matches!(h.kind, hale_model::HoleKind::ComputedSubject)
                && h.at
                    == hale_model::EntityRef::Function(
                        hale_model::FunctionId(go as u32),
                    )
        })
        .expect("computed-subject hole");
    assert_eq!(
        hole.authored_site,
        Some(0),
        "the computed publish is authored first"
    );
    let known = model
        .relations
        .publishes
        .iter()
        .find(|p| p.function.index() == go)
        .expect("known publish row");
    assert_eq!(
        known.site, 1,
        "the known publish consumed the SECOND ordinal — a computed \
         publish must not leave the counter untouched"
    );
}

/// Review pin (round 2): a `via {{ bus }}` walk consults EVERY hole
/// whose hides-mask intersects PUBLISHES — an unanalyzed body says
/// "my publishes are unknown", so exhausting the known publish rows
/// must not conclude Holds.
#[test]
fn bus_walk_fails_closed_on_publishes_hiding_hole() {
    let src = r#"
topic Sig { payload: Int; subject: "app.sig"; }
locus A {
    params { n: Int = 0; }
    fn go(v: Int) { Sig <- v; }
}
fn quiet(v: Int) -> Int { return v; }
group a_side = { A };
group b_side = { quiet };
main locus App {
    params { a: A = A { }; }
    claims { iso: forbid reaches(a_side, b_side) via { bus }; }
    run() { self.a.go(1); }
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
        hale_types::verdict::Verdict::Holds,
        "no subscriber reaches b_side"
    );
    let go = model
        .entities
        .functions
        .iter()
        .position(|f| f.display == "A::go")
        .expect("A::go");
    model.holes.push(hale_model::Hole {
        at: hale_model::EntityRef::Function(hale_model::FunctionId(
            go as u32,
        )),
        kind: hale_model::HoleKind::UnanalyzedBody,
        hides: hale_model::RelationSet::CALLS
            .union(hale_model::RelationSet::PUBLISHES)
            .union(hale_model::RelationSet::EFFECTS),
        authored_site: None,
        reason: "body not analyzed".to_string(),
        provenance: hale_model::ProvenanceId(0),
    });
    let (_p, judged) = judge_forbid_reaches(&table, &model, &[0]);
    assert_eq!(
        judged[0].verdict,
        hale_types::verdict::Verdict::Uncertified,
        "a PUBLISHES-hiding hole must fail the bus walk closed"
    );
}

/// The converse pin: a PUBLISHES-only hole must NOT poison a
/// `via {{ calls }}` walk — relevance is the hides-mask against the
/// families this row walks, not the hole's existence.
#[test]
fn publishes_only_hole_does_not_poison_a_calls_walk() {
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
    claims { iso: forbid reaches(a_side, b_side) via { calls }; }
    run() { println(self.a.go(1)); }
}
fn main() { App { }; }
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let mut model = derive_application_model(&bundle);
    let table = lower_claims(&bundle, &model);
    let go = model
        .entities
        .functions
        .iter()
        .position(|f| f.display == "A::go")
        .expect("A::go");
    model.holes.push(hale_model::Hole {
        at: hale_model::EntityRef::Function(hale_model::FunctionId(
            go as u32,
        )),
        kind: hale_model::HoleKind::ComputedSubject,
        hides: hale_model::RelationSet::PUBLISHES,
        authored_site: Some(0),
        reason: "publish with computed subject".to_string(),
        provenance: hale_model::ProvenanceId(0),
    });
    let (_p, judged) = judge_forbid_reaches(&table, &model, &[0]);
    assert_eq!(
        judged[0].verdict,
        hale_types::verdict::Verdict::Violated,
        "the calls walk still finds the concrete violation — a \
         PUBLISHES-only hole is irrelevant to it"
    );
}

/// Review pin (round 2): an `effects(C)` destination NEEDS each
/// visited fn's EFFECTS rows — a hole hiding EFFECTS means the
/// known rows are not the whole story, so the claim is Uncertified
/// even when no known row carries the class.
#[test]
fn effects_destination_fails_closed_on_effects_hiding_hole() {
    let src = r#"
effect money;
locus A {
    params { n: Int = 0; }
    fn go(v: Int) -> Int { return v; }
}
group a_side = { A };
main locus App {
    params { a: A = A { }; }
    claims { pure: forbid reaches(a_side, effects(money)); }
    run() { println(self.a.go(1)); }
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
        hale_types::verdict::Verdict::Holds,
        "nothing carries `money`"
    );
    let go = model
        .entities
        .functions
        .iter()
        .position(|f| f.display == "A::go")
        .expect("A::go");
    model.holes.push(hale_model::Hole {
        at: hale_model::EntityRef::Function(hale_model::FunctionId(
            go as u32,
        )),
        kind: hale_model::HoleKind::UnanalyzedBody,
        hides: hale_model::RelationSet::EFFECTS,
        authored_site: None,
        reason: "body not analyzed".to_string(),
        provenance: hale_model::ProvenanceId(0),
    });
    let (_p, judged) = judge_forbid_reaches(&table, &model, &[0]);
    assert_eq!(
        judged[0].verdict,
        hale_types::verdict::Verdict::Uncertified,
        "unknown effects on a reachable fn must fail closed"
    );
}

/// Review pin (round 2): absorption TRUNCATION is saturation, not
/// an ordinary hole — the evaluator maps step-ceiling exhaustion to
/// Violated, and the verdict must match even though the message is
/// already byte-identical.
#[test]
fn absorption_truncation_is_violated_not_uncertified() {
    let src = r#"
locus Gate {
    fn probe(r: std::http::Router, req: std::http::Request) -> Int {
        let resp = r.dispatch(req);
        return resp.status;
    }
}
group gates = { Gate };
main locus App {
    claims { pure: forbid reaches(gates, effects(alloc)); }
}
fn main() {
    let r = std::http::Router { };
    let req = std::http::Request { method: "GET", path: "/", body: "" };
    println(Gate { }.probe(r, req));
}
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let mut model = derive_application_model(&bundle);
    let table = lower_claims(&bundle, &model);
    // Truncate the absorption: interior knowledge stops at the
    // entry, so the walk must report the step ceiling — Violated.
    for a in &mut model.legacy.stdlib_absorption {
        for n in &mut a.nodes {
            n.direct_effects.clear();
            n.events.clear();
        }
        a.nodes[0]
            .events
            .push(hale_model::AbsorbedEvent::Truncated);
    }
    assert!(
        !model.legacy.stdlib_absorption.is_empty(),
        "the fixture absorbs a stdlib call"
    );
    let (_p, judged) = judge_forbid_reaches(&table, &model, &[0]);
    assert_eq!(
        judged[0].verdict,
        hale_types::verdict::Verdict::Violated,
        "step-ceiling truncation is Violated, like Saturated"
    );
    assert!(
        judged[0]
            .diags
            .iter()
            .any(|d| d.message.contains("exceeded")),
        "the step-ceiling diagnostic is emitted"
    );
}

/// Review pin (round 3): hole selection within a space is by
/// AUTHORED site, not the model's canonical (kind, reason) sort —
/// an untyped-receiver hole authored before an indirect-call hole
/// reports first even though IndirectCall sorts first canonically.
#[test]
fn call_holes_refuse_in_authored_order() {
    let src = r#"
locus A {
    params { n: Int = 0; }
    fn go(v: Int) -> Int { return v; }
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
    let go = model
        .entities
        .functions
        .iter()
        .position(|f| f.display == "A::go")
        .expect("A::go");
    let at = hale_model::EntityRef::Function(hale_model::FunctionId(
        go as u32,
    ));
    // Authored FIRST: an untyped receiver. Canonically LAST (the
    // model sorts IndirectCall before UntypedReceiver).
    model.holes.push(hale_model::Hole {
        at,
        kind: hale_model::HoleKind::UntypedReceiver {
            callee: "helper".to_string(),
        },
        hides: hale_model::RelationSet::CALLS,
        authored_site: Some(0),
        reason: "untyped receiver".to_string(),
        provenance: hale_model::ProvenanceId(0),
    });
    model.holes.push(hale_model::Hole {
        at,
        kind: hale_model::HoleKind::IndirectCall,
        hides: hale_model::RelationSet::CALLS,
        authored_site: Some(1),
        reason: "call through fn param".to_string(),
        provenance: hale_model::ProvenanceId(0),
    });
    let (_p, judged) = judge_forbid_reaches(&table, &model, &[0]);
    assert_eq!(
        judged[0].verdict,
        hale_types::verdict::Verdict::Uncertified
    );
    assert!(
        judged[0].diags[0].message.contains("cannot type"),
        "the AUTHORED-first hole selects the diagnostic: {}",
        judged[0].diags[0].message
    );
}

/// Review pin (round 3): a hole at SUBJECT grain hiding SUBSCRIBES
/// refuses bus composition — known subscriber rows are not the
/// whole story, so exhausting them must not certify absence.
#[test]
fn subject_subscribes_hole_fails_bus_walk_closed() {
    let src = r#"
topic Sig { payload: Int; subject: "app.sig"; }
locus A {
    params { n: Int = 0; }
    fn go(v: Int) { Sig <- v; }
}
fn quiet(v: Int) -> Int { return v; }
group a_side = { A };
group b_side = { quiet };
main locus App {
    params { a: A = A { }; }
    claims { iso: forbid reaches(a_side, b_side) via { bus }; }
    run() { self.a.go(1); }
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
        hale_types::verdict::Verdict::Holds,
        "no known subscriber reaches b_side"
    );
    let sid = model
        .entities
        .subjects
        .iter()
        .position(|su| su.pattern == "app.sig")
        .expect("subject");
    model.holes.push(hale_model::Hole {
        at: hale_model::EntityRef::Subject(hale_model::SubjectId(
            sid as u32,
        )),
        kind: hale_model::HoleKind::UnanalyzedBody,
        hides: hale_model::RelationSet::SUBSCRIBES,
        authored_site: None,
        reason: "subscriber set incomplete".to_string(),
        provenance: hale_model::ProvenanceId(0),
    });
    let (_p, judged) = judge_forbid_reaches(&table, &model, &[0]);
    assert_eq!(
        judged[0].verdict,
        hale_types::verdict::Verdict::Uncertified,
        "unknown subscribers must fail the bus walk closed"
    );
}

/// Review pin (round 3): truncation is recorded at the unexpanded
/// FRONTIER and the known prefix stays searchable — a concrete
/// witness inside it wins over the step-ceiling verdict.
#[test]
fn truncation_does_not_hide_a_known_witness() {
    let src = r#"
locus Gate {
    fn probe(r: std::http::Router, req: std::http::Request) -> Int {
        let resp = r.dispatch(req);
        return resp.status;
    }
}
group gates = { Gate };
main locus App {
    claims { pure: forbid reaches(gates, effects(alloc)); }
}
fn main() {
    let r = std::http::Router { };
    let req = std::http::Request { method: "GET", path: "/", body: "" };
    println(Gate { }.probe(r, req));
}
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let mut model = derive_application_model(&bundle);
    let table = lower_claims(&bundle, &model);
    // Append a truncated frontier node beside the intact interior
    // (which still holds the alloc witness) and wire an edge to it.
    let a = model
        .legacy
        .stdlib_absorption
        .first_mut()
        .expect("absorption");
    let frontier = a.nodes.len() as u32;
    a.nodes.push(hale_model::AbsorbedNode {
        display: "std::deep::beyond".to_string(),
        direct_effects: Vec::new(),
        events: vec![hale_model::AbsorbedEvent::Truncated],
    });
    a.nodes[0].events.push(hale_model::AbsorbedEvent::Call {
        target: hale_model::AbsorbedTarget::Interior(frontier),
        dispatch: None,
    });
    let (_p, judged) = judge_forbid_reaches(&table, &model, &[0]);
    assert_eq!(
        judged[0].verdict,
        hale_types::verdict::Verdict::Violated
    );
    assert!(
        !judged[0]
            .diags
            .iter()
            .any(|d| d.message.contains("exceeded")),
        "the concrete witness wins — no step-ceiling verdict: {:?}",
        judged[0]
            .diags
            .iter()
            .map(|d| &d.message)
            .collect::<Vec<_>>()
    );
    assert!(
        judged[0]
            .diags
            .iter()
            .any(|d| d.message.contains("violated")),
        "the known-prefix witness is reported"
    );
}

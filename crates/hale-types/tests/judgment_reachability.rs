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
use hale_types::judgment::{judge_forbid_reaches, judge_only_edges};
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
        .filter(|r| {
            matches!(
                r.law,
                ClaimIr::ForbidReaches { .. }
                    | ClaimIr::OnlyEdges { .. }
            )
        })
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
    // New: engine over the lowered rows — both migrated families,
    // merged back into ordinal order (the evaluator's claim order).
    let (pre_diags, judged_fr) =
        judge_forbid_reaches(&table, &model, &[0]);
    let judged_oe = judge_only_edges(&table, &model, &[0]);
    let mut judged: Vec<hale_types::judgment::Judged> = judged_fr
        .into_iter()
        .chain(judged_oe.into_iter())
        .collect();
    judged.sort_by_key(|j| j.ordinal);
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

/// Negative control (5b): the boundary judgment reads the
/// subscribe relation — clearing it removes the un-granted bus
/// edge and flips the verdict.
#[test]
fn dropping_subscribe_rows_changes_the_only_edges_verdict() {
    let src = r#"
type Cmd { v: Int = 0; }
topic Sneaky { payload: Cmd; subject: "app.sneaky"; }
locus Ops {
    params { n: Int = 0; }
    bus { publish Sneaky; }
    fn act() { Sneaky <- Cmd { }; }
}
locus Core {
    params { n: Int = 0; }
    bus { subscribe Sneaky as on_sneaky; }
    fn on_sneaky(c: Cmd) { self.n = c.v; }
}
group ops = { Ops };
group core = { Core };
main locus App {
    params { o: Ops = Ops { }; c: Core = Core { }; }
    claims { boundary: only edges ops -> core { }; }
    run() { println(1); }
}
fn main() { App { }; }
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let mut model = derive_application_model(&bundle);
    let table = lower_claims(&bundle, &model);
    let judged = judge_only_edges(&table, &model, &[0]);
    assert_eq!(
        judged[0].verdict,
        hale_types::verdict::Verdict::Violated
    );
    model.relations.subscribes.clear();
    let judged = judge_only_edges(&table, &model, &[0]);
    assert_eq!(
        judged[0].verdict,
        hale_types::verdict::Verdict::Holds,
        "the judgment reads relations.subscribes"
    );
}

/// Review pin (5b): an indirect call BEFORE a boundary-crossing
/// call refuses at its authored position — only the Uncertified
/// diagnostic, never violation-then-refusal.
#[test]
fn hole_before_crossing_refuses_first() {
    let src = r#"
fn leak(v: Int) -> Int { return v; }
locus A {
    params { n: Int = 0; }
    fn go(f: fn (Int) -> Int, v: Int) -> Int {
        let x = f(v);
        return leak(x);
    }
}
group a_side = { A };
group b_side = { leak };
main locus App {
    params { a: A = A { }; }
    claims { boundary: only edges a_side -> b_side { }; }
    run() { println(1); }
}
fn main() { App { }; }
"#;
    let out = diff_one(src, "hole before crossing");
    assert!(out.is_ok(), "old/new agree: {:?}", out);
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let model = derive_application_model(&bundle);
    let table = lower_claims(&bundle, &model);
    let judged = judge_only_edges(&table, &model, &[0]);
    assert_eq!(
        judged[0].verdict,
        hale_types::verdict::Verdict::Uncertified
    );
    assert_eq!(
        judged[0].diags.len(),
        1,
        "only the refusal — no violation before it: {:?}",
        judged[0]
            .diags
            .iter()
            .map(|d| &d.message)
            .collect::<Vec<_>>()
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
        kind: hale_model::HoleKind::DynamicEndpoint,
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

/// Review pin (round 4): a WILDCARD subscriber hole covers the
/// subjects its pattern matches — a hole at `audit.**` hiding
/// SUBSCRIBES refuses a bus walk publishing to `audit.event`,
/// exactly as a known wildcard subscription would deliver it.
#[test]
fn wildcard_subscriber_hole_covers_matching_publishes() {
    let src = r#"
topic Ev { payload: Int; subject: "audit.event"; }
locus A {
    params { n: Int = 0; }
    fn go(v: Int) { Ev <- v; }
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
        hale_types::verdict::Verdict::Holds
    );
    // A wildcard subject with an incomplete subscriber set.
    model.entities.subjects.push(hale_model::Subject {
        pattern: "audit.**".to_string(),
        exact: false,
        provenance: hale_model::ProvenanceId(0),
    });
    let wild = (model.entities.subjects.len() - 1) as u32;
    model.holes.push(hale_model::Hole {
        at: hale_model::EntityRef::Subject(hale_model::SubjectId(
            wild,
        )),
        kind: hale_model::HoleKind::DynamicEndpoint,
        hides: hale_model::RelationSet::SUBSCRIBES,
        authored_site: None,
        reason: "subscriber set incomplete".to_string(),
        provenance: hale_model::ProvenanceId(0),
    });
    let (_p, judged) = judge_forbid_reaches(&table, &model, &[0]);
    assert_eq!(
        judged[0].verdict,
        hale_types::verdict::Verdict::Uncertified,
        "`audit.**` may cover `audit.event` — the walk must fail \
         closed"
    );
}

/// Review pin (round 5): TOPIC-grain bus holes have the same reach
/// as subject-grain ones — a hole at Topic(T) hiding SUBSCRIBES
/// refuses a bus walk publishing T.
#[test]
fn topic_grain_subscriber_hole_fails_bus_walk_closed() {
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
        hale_types::verdict::Verdict::Holds
    );
    let tid = model
        .entities
        .topics
        .iter()
        .position(|t| t.name == "Sig")
        .expect("topic");
    model.holes.push(hale_model::Hole {
        at: hale_model::EntityRef::Topic(hale_model::TopicId(
            tid as u32,
        )),
        kind: hale_model::HoleKind::DynamicEndpoint,
        hides: hale_model::RelationSet::SUBSCRIBES,
        authored_site: None,
        reason: "subscriber set incomplete".to_string(),
        provenance: hale_model::ProvenanceId(0),
    });
    let (_p, judged) = judge_forbid_reaches(&table, &model, &[0]);
    assert_eq!(
        judged[0].verdict,
        hale_types::verdict::Verdict::Uncertified,
        "a topic-grain hole must have the same reach as a \
         subject-grain one"
    );
}

/// Review pin (round 6): hole coverage is TYPED — a hole at
/// Topic(Orders) does not block a LITERAL publish whose wire text
/// merely collides with the topic's name.
#[test]
fn topic_hole_does_not_block_literal_publish() {
    let src = r#"
type Cmd { v: Int = 0; }
topic Orders { payload: Cmd; subject: "wire.orders"; }
locus A {
    params { n: Int = 0; }
    fn go(v: Int) { "Orders" <- v; }
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
        hale_types::verdict::Verdict::Holds
    );
    let tid = model
        .entities
        .topics
        .iter()
        .position(|t| t.name == "Orders")
        .expect("topic");
    model.holes.push(hale_model::Hole {
        at: hale_model::EntityRef::Topic(hale_model::TopicId(
            tid as u32,
        )),
        kind: hale_model::HoleKind::DynamicEndpoint,
        hides: hale_model::RelationSet::SUBSCRIBES,
        authored_site: None,
        reason: "subscriber set incomplete".to_string(),
        provenance: hale_model::ProvenanceId(0),
    });
    let (_p, judged) = judge_forbid_reaches(&table, &model, &[0]);
    assert_eq!(
        judged[0].verdict,
        hale_types::verdict::Verdict::Holds,
        "the literal wire address `Orders` is not the topic Orders \
         — a typed topic hole must not reach it"
    );
}

/// Review pin (round 6): a set-level subscriber hole cannot erase
/// an already-proven bus violation — the known F -> H path decides
/// Violated regardless of additional unknown subscribers.
#[test]
fn known_bus_violation_survives_subscriber_hole() {
    let src = r#"
type Cmd { v: Int = 0; }
topic Sig { payload: Cmd; subject: "app.sig"; }
locus A {
    params { n: Int = 0; }
    bus { publish Sig; }
    fn go(v: Int) { Sig <- Cmd { }; }
}
locus B {
    params { n: Int = 0; }
    bus { subscribe Sig as on_sig; }
    fn on_sig(c: Cmd) { self.n = c.v; }
}
group a_side = { A };
group b_side = { B };
main locus App {
    params { a: A = A { }; b: B = B { }; }
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
        hale_types::verdict::Verdict::Violated,
        "the known subscriber path is a counterexample"
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
        kind: hale_model::HoleKind::DynamicEndpoint,
        hides: hale_model::RelationSet::SUBSCRIBES,
        authored_site: None,
        reason: "subscriber set incomplete".to_string(),
        provenance: hale_model::ProvenanceId(0),
    });
    let (_p, judged) = judge_forbid_reaches(&table, &model, &[0]);
    assert_eq!(
        judged[0].verdict,
        hale_types::verdict::Verdict::Violated,
        "additional unknown subscribers cannot un-prove the known \
         path"
    );
}

/// Review pin (round 7): the hole shape matrix is CLOSED — an
/// unknown family bit, an anchor grain no judgment consumes, and
/// an authored site on a set-level hole are each rejected, so a
/// valid model cannot carry holes every judgment silently ignores.
#[test]
fn hole_shape_matrix_is_closed() {
    let src = r#"
topic Sig { payload: Int; subject: "app.sig"; }
locus A {
    params { n: Int = 0; }
    fn go(v: Int) { Sig <- v; }
}
main locus App {
    params { a: A = A { }; }
    run() { self.a.go(1); }
}
fn main() { App { }; }
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let base = derive_application_model(&bundle);
    base.validate().expect("the built model is lawful");
    let locus = hale_model::EntityRef::LocusDecl(
        hale_model::LocusDeclId(0),
    );
    let subject = hale_model::EntityRef::Subject(
        hale_model::SubjectId(0),
    );
    // (anchor, kind, hides, site, why-it-must-be-rejected)
    let bad: Vec<(
        hale_model::EntityRef,
        hale_model::HoleKind,
        hale_model::RelationSet,
        Option<u32>,
        &str,
    )> = vec![
        (
            subject,
            hale_model::HoleKind::DynamicEndpoint,
            hale_model::RelationSet(1 << 31),
            None,
            "an unknown family bit is invisible to every judgment",
        ),
        (
            locus,
            hale_model::HoleKind::DynamicEndpoint,
            hale_model::RelationSet::SUBSCRIBES
                .union(hale_model::RelationSet::CARDINALITY),
            None,
            "no judgment consumes locus-grain endpoint holes",
        ),
        (
            subject,
            hale_model::HoleKind::DynamicEndpoint,
            hale_model::RelationSet::SUBSCRIBES,
            Some(0),
            "a set-level hole has no authored position",
        ),
        (
            subject,
            hale_model::HoleKind::IndirectCall,
            hale_model::RelationSet::CALLS,
            None,
            "a call hole is fn-grain knowledge",
        ),
        (
            hale_model::EntityRef::Function(
                hale_model::FunctionId(0),
            ),
            hale_model::HoleKind::UnanalyzedBody,
            hale_model::RelationSet::SUBSCRIBES,
            None,
            "no judgment consults fn-grain SUBSCRIBES holes — the \
             shape would be a valid invisible hole (round 8)",
        ),
        (
            hale_model::EntityRef::Function(
                hale_model::FunctionId(0),
            ),
            hale_model::HoleKind::IndirectCall,
            hale_model::RelationSet::EFFECTS,
            Some(0),
            "a call hole that drops its REQUIRED CALLS bit is \
             invisible to call traversal while still occupying \
             its site (round 9)",
        ),
    ];
    for (at, kind, hides, site, why) in bad {
        let mut m = base.clone();
        m.holes.push(hale_model::Hole {
            at,
            kind,
            hides,
            authored_site: site,
            reason: "test".to_string(),
            provenance: hale_model::ProvenanceId(0),
        });
        assert!(m.validate().is_err(), "{}", why);
    }
    // …and the shapes the judgments DO consume stay lawful (an
    // endpoint hole contradicts the exact-endpoints capability, so
    // the honest model lowers the flag alongside — that law
    // already existed and stays).
    let mut m = base.clone();
    m.capabilities.exact_bus_endpoints = false;
    m.holes.push(hale_model::Hole {
        at: subject,
        kind: hale_model::HoleKind::DynamicEndpoint,
        hides: hale_model::RelationSet::SUBSCRIBES
            .union(hale_model::RelationSet::CARDINALITY),
        authored_site: None,
        reason: "subscriber set incomplete".to_string(),
        provenance: hale_model::ProvenanceId(0),
    });
    m.validate()
        .expect("subject-grain endpoint holes are a defined shape");
}

/// Review pin (round 7): the typed identity on absorbed publishes
/// is validated — a dangling TopicId and a name disagreement are
/// both refused, never trusted machine data.
#[test]
fn absorbed_publish_identity_is_validated() {
    let src = r#"
locus Gate {
    fn probe(r: std::http::Router, req: std::http::Request) -> Int {
        let resp = r.dispatch(req);
        return resp.status;
    }
}
main locus App {
    params { n: Int = 0; }
}
fn main() {
    let r = std::http::Router { };
    let req = std::http::Request { method: "GET", path: "/", body: "" };
    println(Gate { }.probe(r, req));
}
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let base = derive_application_model(&bundle);
    base.validate().expect("the built model is lawful");
    assert!(
        !base.legacy.stdlib_absorption.is_empty(),
        "the fixture absorbs a stdlib call"
    );
    // Dangling topic id.
    let mut m = base.clone();
    m.legacy.stdlib_absorption[0].nodes[0].events.push(
        hale_model::AbsorbedEvent::Publish {
            subject: "Orders".to_string(),
            declared_topic: Some(hale_model::TopicId(999)),
        },
    );
    assert!(
        m.validate().is_err(),
        "a dangling TopicId can panic a judgment"
    );
}

/// Review pin (round 8): the authored-site event partition — one
/// (function, site) is ONE event. A call hole colliding with a
/// resolved call, a site-less site-shaped hole, a whole-body hole
/// carrying a site, and a computed-subject hole colliding with a
/// known publish are each rejected.
#[test]
fn authored_site_partition_is_validated() {
    let src = r#"
topic Sig { payload: Int; subject: "app.sig"; }
locus A {
    params { n: Int = 0; }
    fn go(v: Int) -> Int { Sig <- v; return leak(v); }
}
fn leak(v: Int) -> Int { return v; }
main locus App {
    params { a: A = A { }; }
    run() { println(self.a.go(1)); }
}
fn main() { App { }; }
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let base = derive_application_model(&bundle);
    base.validate().expect("the built model is lawful");
    let go = base
        .entities
        .functions
        .iter()
        .position(|f| f.display == "A::go")
        .expect("A::go") as u32;
    let call_site = base
        .relations
        .calls
        .iter()
        .find(|c| c.from.0 == go)
        .expect("call row")
        .site;
    let mut with_hole = |kind: hale_model::HoleKind,
                         hides: hale_model::RelationSet,
                         site: Option<u32>|
     -> hale_model::ApplicationModel {
        let mut m = base.clone();
        m.capabilities = hale_model::Capabilities::default();
        m.holes.push(hale_model::Hole {
            at: hale_model::EntityRef::Function(
                hale_model::FunctionId(go),
            ),
            kind,
            hides,
            authored_site: site,
            reason: "test".to_string(),
            provenance: hale_model::ProvenanceId(0),
        });
        m
    };
    // A call hole sharing the resolved call's site.
    assert!(
        with_hole(
            hale_model::HoleKind::IndirectCall,
            hale_model::RelationSet::CALLS
                .union(hale_model::RelationSet::EFFECTS),
            Some(call_site),
        )
        .validate()
        .is_err(),
        "one authored expression cannot be both a resolved call \
         and a hole"
    );
    // A site-shaped hole without its ordinal.
    assert!(
        with_hole(
            hale_model::HoleKind::IndirectCall,
            hale_model::RelationSet::CALLS
                .union(hale_model::RelationSet::EFFECTS),
            None,
        )
        .validate()
        .is_err(),
        "a site-shaped hole requires its authored position"
    );
    // A whole-body hole carrying a site.
    assert!(
        with_hole(
            hale_model::HoleKind::UnanalyzedBody,
            hale_model::RelationSet::CALLS
                .union(hale_model::RelationSet::PUBLISHES)
                .union(hale_model::RelationSet::EFFECTS),
            Some(0),
        )
        .validate()
        .is_err(),
        "a whole-body hole has no single authored position"
    );
    // A computed-subject hole colliding with the known publish.
    let pub_site = base
        .relations
        .publishes
        .iter()
        .find(|p| p.function.0 == go)
        .expect("publish row")
        .site;
    assert!(
        with_hole(
            hale_model::HoleKind::ComputedSubject,
            hale_model::RelationSet::PUBLISHES,
            Some(pub_site),
        )
        .validate()
        .is_err(),
        "one authored expression cannot be both a known publish \
         and a computed-subject hole"
    );
}

/// Review pin (round 8): a set-level PUBLISHES hole poisons a bus
/// walk — an unknown publisher may create an edge the composition
/// cannot see — while a known counterexample still wins.
#[test]
fn publisher_hole_fails_bus_walk_closed() {
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
        hale_types::verdict::Verdict::Holds
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
        kind: hale_model::HoleKind::DynamicEndpoint,
        hides: hale_model::RelationSet::PUBLISHES,
        authored_site: None,
        reason: "publisher set incomplete".to_string(),
        provenance: hale_model::ProvenanceId(0),
    });
    let (_p, judged) = judge_forbid_reaches(&table, &model, &[0]);
    assert_eq!(
        judged[0].verdict,
        hale_types::verdict::Verdict::Uncertified,
        "an unknown publisher may create the edge — fail closed"
    );
}

/// Review pin (round 8): a dispatch label binds to its target — a
/// rendered method that is not the target's own identity is
/// rejected, at interior events and at the entry.
#[test]
fn absorbed_dispatch_binds_to_its_target() {
    let src = r#"
locus Gate {
    fn probe(r: std::http::Router, req: std::http::Request) -> Int {
        let resp = r.dispatch(req);
        return resp.status;
    }
}
main locus App {
    params { n: Int = 0; }
}
fn main() {
    let r = std::http::Router { };
    let req = std::http::Request { method: "GET", path: "/", body: "" };
    println(Gate { }.probe(r, req));
}
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let base = derive_application_model(&bundle);
    base.validate().expect("the built model is lawful");
    // An interior call whose label names a method its target is not.
    let mut m = base.clone();
    let n = m.legacy.stdlib_absorption[0].nodes.len() as u32;
    m.legacy.stdlib_absorption[0].nodes.push(
        hale_model::AbsorbedNode {
            display: "std::x::Ledger::charge_a".to_string(),
            direct_effects: Vec::new(),
            events: Vec::new(),
        },
    );
    m.legacy.stdlib_absorption[0].nodes[0].events.push(
        hale_model::AbsorbedEvent::Call {
            target: hale_model::AbsorbedTarget::Interior(n),
            dispatch: Some((
                "Payer".to_string(),
                "pay".to_string(),
            )),
        },
    );
    assert!(
        m.validate().is_err(),
        "the label says `pay`; the target is `charge_a`"
    );
    // An entry label that is not node zero's method.
    let mut m = base.clone();
    m.legacy.stdlib_absorption[0].entry_dispatch = Some((
        "Payer".to_string(),
        "pay".to_string(),
    ));
    assert!(
        m.validate().is_err(),
        "the entry label must be node zero's own method"
    );
}

/// Review pin (round 8): unresolved residue INSIDE stdlib
/// absorption participates in the exactness account — a CallHole
/// contradicts `exact_calls`, and the builder derives the honest
/// value from real source.
#[test]
fn absorption_residue_lowers_capabilities() {
    let src = r#"
locus Gate {
    fn probe(r: std::http::Router, req: std::http::Request) -> Int {
        let resp = r.dispatch(req);
        return resp.status;
    }
}
main locus App {
    params { n: Int = 0; }
}
fn main() {
    let r = std::http::Router { };
    let req = std::http::Request { method: "GET", path: "/", body: "" };
    println(Gate { }.probe(r, req));
}
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let base = derive_application_model(&bundle);
    base.validate().expect("the built model is lawful");
    let mut m = base.clone();
    m.legacy.stdlib_absorption[0].nodes[0].events.push(
        hale_model::AbsorbedEvent::CallHole(
            hale_model::AbsorbedHoleKind::IndirectCall,
        ),
    );
    m.capabilities.exact_calls = true;
    assert!(
        m.validate().is_err(),
        "an interior CallHole contradicts exact_calls"
    );
}

/// Review pin (round 9): the site partition means exactly ONE
/// event — a second typed hole in the same call or publish site is
/// rejected, never left for the judgment to pick by canonical
/// kind order.
#[test]
fn one_hole_per_authored_site() {
    let src = r#"
topic Sig { payload: Int; subject: "app.sig"; }
locus A {
    params { n: Int = 0; }
    fn go(v: Int) { Sig <- v; }
}
main locus App {
    params { a: A = A { }; }
    run() { self.a.go(1); }
}
fn main() { App { }; }
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let base = derive_application_model(&bundle);
    base.validate().expect("the built model is lawful");
    let go = base
        .entities
        .functions
        .iter()
        .position(|f| f.display == "A::go")
        .expect("A::go") as u32;
    // Two distinct call holes at one site (distinct canonical rows
    // — different kinds — same authored position).
    let mut m = base.clone();
    m.capabilities = hale_model::Capabilities::default();
    for kind in [
        hale_model::HoleKind::IndirectCall,
        hale_model::HoleKind::UntypedReceiver {
            callee: "tick".to_string(),
        },
    ] {
        m.holes.push(hale_model::Hole {
            at: hale_model::EntityRef::Function(
                hale_model::FunctionId(go),
            ),
            kind,
            hides: hale_model::RelationSet::CALLS
                .union(hale_model::RelationSet::EFFECTS),
            authored_site: Some(7),
            reason: "test".to_string(),
            provenance: hale_model::ProvenanceId(0),
        });
    }
    m.holes.sort_by(|a, b| {
        (&a.at, &a.kind, &a.reason).cmp(&(&b.at, &b.kind, &b.reason))
    });
    assert!(
        m.validate().is_err(),
        "two call holes cannot share one authored site"
    );
    // Two computed-subject holes at one publish site.
    let mut m = base.clone();
    m.capabilities = hale_model::Capabilities::default();
    for reason in ["first", "second"] {
        m.holes.push(hale_model::Hole {
            at: hale_model::EntityRef::Function(
                hale_model::FunctionId(go),
            ),
            kind: hale_model::HoleKind::ComputedSubject,
            hides: hale_model::RelationSet::PUBLISHES,
            authored_site: Some(9),
            reason: reason.to_string(),
            provenance: hale_model::ProvenanceId(0),
        });
    }
    m.holes.sort_by(|a, b| {
        (&a.at, &a.kind, &a.reason).cmp(&(&b.at, &b.kind, &b.reason))
    });
    assert!(
        m.validate().is_err(),
        "two computed-subject holes cannot share one publish site"
    );
}

/// Review pin (round 9): the contracted ViaStdlib relation and the
/// absorption sidecar are dual accounts — a contracted row with no
/// realizing interior path, and a re-emergence with no contracted
/// row, are both rejected.
#[test]
fn via_stdlib_rows_agree_with_absorption() {
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
main locus App {
    params { n: Int = 0; }
}
fn main() {
    let r = std::http::Router { };
    r.add("GET", "/", Hello { });
    let req = std::http::Request { method: "GET", path: "/", body: "" };
    println(Gate { }.probe(r, req));
}
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let base = derive_application_model(&bundle);
    base.validate().expect("the built model is lawful");
    assert!(
        base.relations.calls.iter().any(|c| matches!(
            c.dispatch,
            hale_model::DispatchKind::ViaStdlib
        )),
        "the fixture carries a contracted through-stdlib row"
    );
    // A contracted row whose interior is gone: every judgment
    // would discard the only modeled edge.
    let mut m = base.clone();
    m.legacy.stdlib_absorption.clear();
    m.capabilities = hale_model::Capabilities::default();
    assert!(
        m.validate().is_err(),
        "a ViaStdlib row must be realized by an absorption path"
    );
    // A re-emergence the contracted relation denies.
    let mut m = base.clone();
    let hello = m
        .entities
        .functions
        .iter()
        .position(|f| f.display == "Hello::handle")
        .expect("Hello::handle") as u32;
    let probe_entry = m
        .legacy
        .stdlib_absorption
        .iter()
        .position(|a| {
            m.entities.functions[a.from.index()].display
                != "Gate::probe"
        })
        .unwrap_or(0);
    m.legacy.stdlib_absorption[probe_entry].nodes[0]
        .events
        .push(hale_model::AbsorbedEvent::Call {
            target: hale_model::AbsorbedTarget::User(
                hale_model::FunctionId(hello),
            ),
            dispatch: None,
        });
    assert!(
        m.validate().is_err(),
        "a re-emergence must have its contracted row"
    );
}

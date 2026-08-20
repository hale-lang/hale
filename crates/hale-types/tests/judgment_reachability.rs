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
use hale_types::judgment::{
    judge_bound, judge_endpoints, judge_forbid_reaches,
    judge_only_edges,
};
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
                    | ClaimIr::RequireEndpoint { .. }
                    | ClaimIr::RequireSealed { .. }
                    | ClaimIr::RequireAttributed { .. }
                    | ClaimIr::Cover { .. }
                    | ClaimIr::Count { .. }
                    | ClaimIr::Bound { .. }
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
    let judged_ep = judge_endpoints(&table, &model, &[0]);
    let judged_bd = judge_bound(&table, &model, &[0]);
    let mut judged: Vec<hale_types::judgment::Judged> = judged_fr
        .into_iter()
        .chain(judged_oe.into_iter())
        .chain(judged_ep.into_iter())
        .chain(judged_bd.into_iter())
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
    // Rows under the documented 5c divergence: their (new-only)
    // diagnostics are excluded from the byte comparison too.
    let mut carved_out: std::collections::BTreeSet<u32> =
        std::collections::BTreeSet::new();
    for j in &judged {
        let row = by_ordinal[&j.ordinal];
        let Some(old) = old_verdicts.get(row.name.as_str()) else {
            return Err(format!(
                "{}: claim `{}` judged but has no outcome",
                origin, row.name
            ));
        };
        // Documented divergence (5c round 2): the evaluator never
        // sees unanalyzed bodies (module fns, on_failure hooks), so
        // `require attributed` fail-opens to Holds where the model
        // records an EFFECTS-hiding hole and the judgment refuses.
        let attributed_hole_carveout = matches!(
            row.law,
            ClaimIr::RequireAttributed { .. }
        ) && **old == hale_types::verdict::Verdict::Holds
            && j.verdict == hale_types::verdict::Verdict::Uncertified
            && model.holes.iter().any(|h| {
                h.hides
                    .intersects(hale_model::RelationSet::EFFECTS)
            });
        if attributed_hole_carveout {
            carved_out.insert(j.ordinal);
        }
        if **old != j.verdict && !attributed_hole_carveout {
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
    // The evaluator's stream: enumeration diagnostics first (the
    // lowering preserves them as table ISSUES), then the dup
    // pre-pass, then per-row validation+evaluation.
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
    let new_family: Vec<(String, hale_syntax::Span)> = table
        .issues
        .iter()
        .filter(|i| {
            names.iter().any(|n| {
                i.message.contains(&format!("claim `{}`", n))
            })
        })
        .map(|i| (i.message.clone(), issue_span(i.provenance)))
        .chain(
            pre_diags
                .iter()
                .chain(
                    judged
                        .iter()
                        .filter(|j| !carved_out.contains(&j.ordinal))
                        .flat_map(|j| j.diags.iter()),
                )
                .map(|d| (d.message.clone(), d.span)),
        )
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

/// Negative control (5c): the endpoint judgment reads DECLARED
/// publisher ends — clearing declares_publish flips a holding
/// `require publishes` to Violated.
#[test]
fn dropping_declared_ends_changes_the_require_verdict() {
    let src = r#"
type T { n: Int = 0; }
topic Orders { payload: T; subject: "orders"; }
locus Gw {
    params { n: Int = 0; }
    bus { publish Orders; }
    fn send(v: Int) { Orders <- T { }; }
}
group gws = { Gw };
main locus App {
    params { g: Gw = Gw { }; }
    claims { writer: require publishes(some gws, topic Orders); }
    run() { println(1); }
}
fn main() { App { }; }
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let mut model = derive_application_model(&bundle);
    let table = lower_claims(&bundle, &model);
    let judged = judge_endpoints(&table, &model, &[0]);
    assert_eq!(judged[0].verdict, hale_types::verdict::Verdict::Holds);
    model.relations.declares_publish.clear();
    let judged = judge_endpoints(&table, &model, &[0]);
    assert_eq!(
        judged[0].verdict,
        hale_types::verdict::Verdict::Violated,
        "the judgment reads relations.declares_publish"
    );
}

/// Review pin (5c): a COMPOSED user class in `@effects(is:)` is
/// authored purpose — the expanded label set (its atoms) must not
/// hide it from `require attributed`.
#[test]
fn composed_class_counts_as_authored_attribution() {
    let src = r#"
effect io = { syscall, alloc };
type Buf { n: Int = 0; }
@effects(is: { io })
fn make(v: Int) -> Int { let b = Buf { }; return v; }
main locus App {
    claims { tagged: require attributed(all alloc); }
    run() { println(make(1)); }
}
fn main() { App { }; }
"#;
    let out = diff_one(src, "composed attribution");
    assert!(out.is_ok(), "old/new agree: {:?}", out);
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let model = derive_application_model(&bundle);
    let table = lower_claims(&bundle, &model);
    let judged = judge_endpoints(&table, &model, &[0]);
    // `main`/`run` alloc without purpose — the claim violates, but
    // `make` (authored composed purpose) must NOT be in the list.
    assert_eq!(
        judged[0].verdict,
        hale_types::verdict::Verdict::Violated
    );
    let msg = &judged[0].diags[0].message;
    assert!(
        !msg.contains("make"),
        "the composed `io` IS an authored purpose: {}",
        msg
    );
}

/// Negative control (5d): the bound judgment reads carrier LABELS —
/// clearing them zeroes the count and flips the verdict.
#[test]
fn dropping_labels_changes_the_bound_verdict() {
    let src = r#"
effect money;
@effects(is: { money })
fn spend(v: Int) -> Int { return v; }
locus A {
    params { n: Int = 0; }
    fn go(v: Int) -> Int { return spend(v) + spend(v); }
}
group gates = { A };
main locus App {
    params { a: A = A { }; }
    claims { cap: bound money <= 1 on paths from gates; }
    run() { println(self.a.go(1)); }
}
fn main() { App { }; }
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let mut model = derive_application_model(&bundle);
    let table = lower_claims(&bundle, &model);
    let judged = judge_bound(&table, &model, &[0]);
    assert_eq!(
        judged[0].verdict,
        hale_types::verdict::Verdict::Violated,
        "two carrier calls exceed limit 1"
    );
    model.labels.clear();
    let judged = judge_bound(&table, &model, &[0]);
    assert_eq!(
        judged[0].verdict,
        hale_types::verdict::Verdict::Holds,
        "the judgment reads model.labels"
    );
}

/// Review pin (5d): a carrier-bearing stdlib wrapper called from
/// inside a LOOP is unbounded — the entry edge's loop nesting is a
/// real model fact (StdlibAbsorption.entry_in_loop), not a
/// synthesized false. Hand-built: the builder cannot yet produce a
/// carrier-bearing stdlib interior from a simple fixture, and the
/// judgment must hold as coverage grows.
#[test]
fn looped_stdlib_entry_with_carrier_is_unbounded() {
    use hale_model::*;
    let mut prov = ProvenanceTable::default();
    prov.records.push(Provenance::Synthetic {
        origin: "test".to_string(),
    });
    let p = ProvenanceId(0);
    let f = |name: &str| Function {
        analyzed: true,
        name: name.to_string(),
        display: name.to_string(),
        kind: FunctionKind::Free,
        effects: Vec::new(),
        direct_effects: Vec::new(),
        attribution: Vec::new(),
        opaque_call: false,
        carries_user_class: false,
        provenance: p,
    };
    let mut m = ApplicationModel {
        header: ModelHeader {
            semantics: MODEL_SEMANTICS_V1,
            entrypoint: "main".to_string(),
        },
        entities: Entities {
            functions: vec![f("caller")],
            groups: vec![Group {
                name: "roots".to_string(),
                display: "roots".to_string(),
                may_be_empty: false,
                provenance: p,
            }],
            effect_classes: vec![EffectClassDecl {
                name: "money".to_string(),
                declared: true,
                definition: EffectClassDefinition::Atomic,
                provenance: p,
            }],
            ..Entities::default()
        },
        relations: Relations::default(),
        labels: Vec::new(),
        weights: Vec::new(),
        holes: Vec::new(),
        capabilities: Capabilities::default(),
        provenance: prov,
        legacy: LegacyProjection {
            topology_v1_fns: vec![FunctionId(0)],
            topology_v1_calls_via_stdlib: Vec::new(),
            stdlib_absorption: vec![StdlibAbsorption {
                from: FunctionId(0),
                site: 0,
                entry_dispatch: None,
                entry_in_loop: true,
                entry_group: None,
                entry_provenance: p,
                nodes: vec![AbsorbedNode {
                    display: "std::pay::charge".to_string(),
                    carries: vec!["money".to_string()],
                    direct_effects: Vec::new(),
                    events: Vec::new(),
                }],
            }],
        },
    };
    m.relations.group_members.push(GroupMember {
        group: GroupId(0),
        member: EntityRef::Function(FunctionId(0)),
        provenance: p,
    });
    // Lower a `bound money <= 5 on paths from roots` row by hand.
    let mut t = ClaimIrTable::default();
    t.provenance.records.push(Provenance::Synthetic {
        origin: "test".to_string(),
    });
    t.rows.push(ClaimRow {
        ordinal: 0,
        name: "cap".to_string(),
        origin: ClaimOrigin::Main,
        law: ClaimIr::Bound {
            class: EffectClassRef {
                class: Some(EffectClassId(0)),
                builtin: false,
                name: "money".to_string(),
                provenance: ProvenanceId(0),
            },
            limit: 5,
            from: GroupRef {
                group: Some(GroupId(0)),
                name: NameRef {
                    raw: "roots".to_string(),
                    display: "roots".to_string(),
                },
                provenance: ProvenanceId(0),
            },
        },
        provenance: ProvenanceId(0),
    });
    let judged =
        hale_types::judgment::judge_bound(&t, &m, &[0]);
    assert_eq!(
        judged[0].verdict,
        hale_types::verdict::Verdict::Violated,
        "loop-nested carrier entry is unbounded"
    );
    assert!(
        judged[0].diags[0]
            .message
            .contains("reached from inside a loop"),
        "classified as LoopCarrier: {}",
        judged[0].diags[0].message
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
        carries: Vec::new(),
        direct_effects: Vec::new(),
        events: vec![hale_model::AbsorbedEvent::Truncated],
    });
    a.nodes[0].events.push(hale_model::AbsorbedEvent::Call {
        target: hale_model::AbsorbedTarget::Interior(frontier),
        dispatch: None,
        in_loop: false,
        group: None,
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
    m.capabilities.exact_publishes = false;
    m.capabilities.exact_subscribes = false;
    m.capabilities.exact_cardinality = false;
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
            in_loop: false,
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
            carries: Vec::new(),
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
            in_loop: false,
            group: Some(9),
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
            in_loop: false,
            group: None,
        });
    assert!(
        m.validate().is_err(),
        "a re-emergence must have its contracted row"
    );
}

/// Review pin (round 2): the publish space is ONE ordered stream —
/// a computed-subject publish authored BEFORE an ungranted known
/// publish refuses at its position (only the refusal diagnostic).
#[test]
fn computed_publish_before_known_refuses_first() {
    let src = r#"
type Cmd { v: Int = 0; }
topic Sneaky { payload: Cmd; subject: "app.sneaky"; }
locus Ops {
    params { n: Int = 0; }
    bus { publish Sneaky; }
    fn act() {
        self.n <- 1;
        Sneaky <- Cmd { };
    }
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
    let out = diff_one(src, "computed publish first");
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
        "only the refusal — the later crossing is never reported: {:?}",
        judged[0]
            .diags
            .iter()
            .map(|d| &d.message)
            .collect::<Vec<_>>()
    );
}

/// …and the converse: an ungranted known publish authored BEFORE
/// the computed one reports its violation first, THEN refuses.
#[test]
fn known_violation_before_computed_publish_reports_then_refuses() {
    let src = r#"
type Cmd { v: Int = 0; }
topic Sneaky { payload: Cmd; subject: "app.sneaky"; }
locus Ops {
    params { n: Int = 0; }
    bus { publish Sneaky; }
    fn act() {
        Sneaky <- Cmd { };
        self.n <- 1;
    }
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
    let out = diff_one(src, "known violation first");
    assert!(out.is_ok(), "old/new agree: {:?}", out);
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let model = derive_application_model(&bundle);
    let table = lower_claims(&bundle, &model);
    let judged = judge_only_edges(&table, &model, &[0]);
    let msgs: Vec<&String> =
        judged[0].diags.iter().map(|d| &d.message).collect();
    assert!(
        msgs.first().is_some_and(|m| m.contains("violated")),
        "the earlier crossing reports first: {:?}",
        msgs
    );
    assert!(
        msgs.last().is_some_and(|m| m.contains("computed subject")),
        "the refusal follows at its authored position: {:?}",
        msgs
    );
}

/// Review pin (round 3): the boundary check consults SUBJECT-grain
/// SUBSCRIBES holes — an ungranted edge cannot be ruled out when
/// the subject's subscriber set is incomplete.
#[test]
fn subject_subscribes_hole_fails_only_edges_closed() {
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
    fn idle() { }
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
        hale_types::verdict::Verdict::Holds,
        "no known subscriber crosses the boundary"
    );
    let sid = model
        .entities
        .subjects
        .iter()
        .position(|su| su.pattern == "app.sneaky")
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
    let judged = judge_only_edges(&table, &model, &[0]);
    assert_eq!(
        judged[0].verdict,
        hale_types::verdict::Verdict::Uncertified,
        "unknown subscribers must fail the boundary closed"
    );
}

/// Review pin (round 4): the boundary check applies the delivery
/// predicate to subscriber holes — a wildcard hole at `audit.**`
/// covers a publish to `audit.event`.
#[test]
fn wildcard_subscriber_hole_fails_only_edges_closed() {
    let src = r#"
type Cmd { v: Int = 0; }
topic Ev { payload: Cmd; subject: "audit.event"; }
locus Ops {
    params { n: Int = 0; }
    bus { publish Ev; }
    fn act() { Ev <- Cmd { }; }
}
locus Core {
    params { n: Int = 0; }
    fn idle() { }
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
        hale_types::verdict::Verdict::Holds
    );
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
    let judged = judge_only_edges(&table, &model, &[0]);
    assert_eq!(
        judged[0].verdict,
        hale_types::verdict::Verdict::Uncertified,
        "`audit.**` may cover `audit.event` — the boundary must \
         fail closed"
    );
}

/// Review pin (round 5): topic-grain holes reach the boundary
/// check identically to subject-grain ones.
#[test]
fn topic_grain_subscriber_hole_fails_only_edges_closed() {
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
    fn idle() { }
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
        hale_types::verdict::Verdict::Holds
    );
    let tid = model
        .entities
        .topics
        .iter()
        .position(|t| t.name == "Sneaky")
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
    let judged = judge_only_edges(&table, &model, &[0]);
    assert_eq!(
        judged[0].verdict,
        hale_types::verdict::Verdict::Uncertified,
        "a topic-grain hole must reach the boundary check"
    );
}

/// Review pin (round 6): a set-level subscriber hole cannot erase
/// a known ungranted boundary crossing.
#[test]
fn known_boundary_violation_survives_subscriber_hole() {
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
        hale_types::verdict::Verdict::Violated,
        "the known subscriber row is an ungranted crossing"
    );
    let sid = model
        .entities
        .subjects
        .iter()
        .position(|su| su.pattern == "app.sneaky")
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
    let judged = judge_only_edges(&table, &model, &[0]);
    assert_eq!(
        judged[0].verdict,
        hale_types::verdict::Verdict::Violated,
        "unknown extra subscribers cannot un-prove the known \
         crossing"
    );
}

/// …typed identity in the boundary check: a topic hole does not
/// block a literal publish whose text collides with the name.
#[test]
fn topic_hole_does_not_block_literal_publish_in_only_edges() {
    let src = r#"
type Cmd { v: Int = 0; }
topic Orders { payload: Cmd; subject: "wire.orders"; }
locus Ops {
    params { n: Int = 0; }
    fn act(v: Int) { "Orders" <- v; }
}
locus Core {
    params { n: Int = 0; }
    fn idle() { }
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
    let judged = judge_only_edges(&table, &model, &[0]);
    assert_eq!(
        judged[0].verdict,
        hale_types::verdict::Verdict::Holds,
        "the literal wire address is not the topic"
    );
}

/// Review pin (round 8): a set-level PUBLISHES hole poisons the
/// boundary — an unknown publisher may create an ungranted edge —
/// while a known crossing still proves Violated.
#[test]
fn publisher_hole_fails_only_edges_closed() {
    let src = r#"
type Cmd { v: Int = 0; }
topic Sig { payload: Cmd; subject: "app.sig"; }
locus Ops {
    params { n: Int = 0; }
    bus { publish Sig; }
    fn act() { Sig <- Cmd { }; }
}
locus Core {
    params { n: Int = 0; }
    fn idle() { }
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
    let judged = judge_only_edges(&table, &model, &[0]);
    assert_eq!(
        judged[0].verdict,
        hale_types::verdict::Verdict::Uncertified,
        "an unknown publisher may create an ungranted edge"
    );
}

/// Review pin (round 2): `require attributed` consults the model's
/// HOLES, not only `Function.opaque_call` — an on_failure handler
/// enters the universe with an UnanalyzedBody hole hiding EFFECTS,
/// so it may perform the class without an authored purpose and the
/// claim must be Uncertified, never a fail-open Holds.
#[test]
fn attributed_fails_closed_on_unanalyzed_bodies() {
    let held = r#"
main locus App {
    params { n: Int = 0; }
    claims { tagged: require attributed(all syscall); }
}
fn main() { App { }; }
"#;
    let program = hale_syntax::parse_source(held).expect("parse");
    let bundle = bundle_of(held, &program);
    let model = derive_application_model(&bundle);
    let table = lower_claims(&bundle, &model);
    let judged = judge_endpoints(&table, &model, &[0]);
    assert_eq!(
        judged[0].verdict,
        hale_types::verdict::Verdict::Holds,
        "nothing is unattributed or unanalyzed"
    );
    let src = r#"
type Violation { code: Int = 0; }
locus Sup {
    params { n: Int = 0; }
    on_failure(e: Violation) { }
}
main locus App {
    params { s: Sup = Sup { }; }
    claims { tagged: require attributed(all syscall); }
}
fn main() { App { }; }
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let model = derive_application_model(&bundle);
    assert!(
        model.holes.iter().any(|h| h
            .hides
            .intersects(hale_model::RelationSet::EFFECTS)),
        "the on_failure body arrives as an EFFECTS-hiding hole"
    );
    let table = lower_claims(&bundle, &model);
    let judged = judge_endpoints(&table, &model, &[0]);
    assert_eq!(
        judged[0].verdict,
        hale_types::verdict::Verdict::Uncertified,
        "an unanalyzed application-owned body blocks certification"
    );
}

/// Review pin (round 4): endpoint/count judgments consult
/// PUBLISHES/SUBSCRIBES/CARDINALITY holes — known rows are a lower
/// bound, never a proved absence, with monotone cases preserved.
#[test]
fn count_and_require_fail_closed_on_endpoint_holes() {
    let src = r#"
type Cmd { v: Int = 0; }
topic T { payload: Cmd; subject: "app.t"; }
locus Pub {
    params { n: Int = 0; }
    bus { publish T; }
    fn act() { T <- Cmd { }; }
}
group pubs = { Pub };
main locus App {
    params { p: Pub = Pub { }; }
    claims {
        none_yet: count subscribers(topic T) <= 0;
        one_pub: count publishers(topic T) >= 1;
        someone: require subscribes(some pubs, topic T);
    }
    run() { println(1); }
}
fn main() { App { }; }
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let mut model = derive_application_model(&bundle);
    let table = lower_claims(&bundle, &model);
    let by_name = |judged: &[hale_types::judgment::Judged],
                   name: &str|
     -> hale_types::verdict::Verdict {
        let row = table
            .rows
            .iter()
            .find(|r| r.name == name)
            .expect("row");
        judged
            .iter()
            .find(|j| j.ordinal == row.ordinal)
            .expect("judged")
            .verdict
    };
    let judged = judge_endpoints(&table, &model, &[0]);
    assert_eq!(
        by_name(&judged, "none_yet"),
        hale_types::verdict::Verdict::Holds
    );
    assert_eq!(
        by_name(&judged, "someone"),
        hale_types::verdict::Verdict::Violated
    );
    // The reviewer's shape: a hole at T's subject hiding
    // SUBSCRIBES | CARDINALITY.
    let sid = model
        .entities
        .subjects
        .iter()
        .position(|su| su.pattern == "app.t")
        .expect("subject");
    model.holes.push(hale_model::Hole {
        at: hale_model::EntityRef::Subject(hale_model::SubjectId(
            sid as u32,
        )),
        kind: hale_model::HoleKind::DynamicEndpoint,
        hides: hale_model::RelationSet::SUBSCRIBES
            .union(hale_model::RelationSet::CARDINALITY),
        authored_site: None,
        reason: "subscriber set incomplete".to_string(),
        provenance: hale_model::ProvenanceId(0),
    });
    let judged = judge_endpoints(&table, &model, &[0]);
    assert_eq!(
        by_name(&judged, "none_yet"),
        hale_types::verdict::Verdict::Uncertified,
        "count <= 0 cannot hold from an incomplete set"
    );
    assert_eq!(
        by_name(&judged, "someone"),
        hale_types::verdict::Verdict::Uncertified,
        "an absent witness plus a relevant hole is not a violation"
    );
    // Monotone preserved: a known publisher still proves `>= 1`
    // even when the publisher set is ALSO marked incomplete.
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
    let judged = judge_endpoints(&table, &model, &[0]);
    assert_eq!(
        by_name(&judged, "one_pub"),
        hale_types::verdict::Verdict::Holds,
        "enough known rows still prove a lower bound"
    );
}

/// …and `cover`: an apparently-uncovered topic with an incomplete
/// subscriber set is Uncertified, while a concretely uncovered
/// topic still proves the violation.
#[test]
fn cover_fails_closed_on_subscriber_holes() {
    let lib = r#"
type __lib_x_kv_Item { n: Int = 0; }
topic __lib_x_kv_Changed { payload: __lib_x_kv_Item; subject: "kv.changed"; }
"#;
    let main_src = r#"
locus Reader {
    params { n: Int = 0; }
    fn idle() { }
}
group readers = { Reader };
main locus App {
    params { r: Reader = Reader { }; }
    claims { covered: cover topic in seed(kv): subscribed_by(some readers); }
    run() { println(1); }
}
fn main() { App { }; }
"#;
    let main_p =
        hale_syntax::parse_source(main_src).expect("parse main");
    let lib_p = hale_syntax::parse_source(lib).expect("parse lib");
    let mut programs = BTreeMap::new();
    programs.insert("app/main.hl".to_string(), &main_p);
    programs.insert("lib/kv.hl".to_string(), &lib_p);
    let mut bundle = Bundle::new(programs);
    bundle.import_renames = vec![
        (
            vec!["kv".to_string(), "Item".to_string()],
            "__lib_x_kv_Item".to_string(),
        ),
        (
            vec!["kv".to_string(), "Changed".to_string()],
            "__lib_x_kv_Changed".to_string(),
        ),
    ];
    let mut model = derive_application_model(&bundle);
    let table = lower_claims(&bundle, &model);
    let judged = judge_endpoints(&table, &model, &[0]);
    assert_eq!(
        judged[0].verdict,
        hale_types::verdict::Verdict::Violated,
        "concretely uncovered"
    );
    let sid = model
        .entities
        .subjects
        .iter()
        .position(|su| su.pattern == "kv.changed")
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
    let judged = judge_endpoints(&table, &model, &[0]);
    assert_eq!(
        judged[0].verdict,
        hale_types::verdict::Verdict::Uncertified,
        "an incomplete subscriber set cannot prove the violation"
    );
}

/// Review pin (round 6): endpoint counts use TYPED identity — a
/// subject hole whose wire pattern merely equals the topic's NAME
/// does not make the topic's count incomplete (the topic's wire is
/// different), while a hole at the topic's REAL wire still does.
#[test]
fn literal_collision_subject_hole_does_not_block_topic_count() {
    let src = r#"
type Cmd { v: Int = 0; }
topic Orders { payload: Cmd; subject: "wire.orders"; }
locus Pub {
    params { n: Int = 0; }
    bus { publish Orders; }
    fn act() { Orders <- Cmd { }; }
}
main locus App {
    params { p: Pub = Pub { }; }
    claims { none_yet: count subscribers(topic Orders) <= 0; }
    run() { println(1); }
}
fn main() { App { }; }
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let mut model = derive_application_model(&bundle);
    let table = lower_claims(&bundle, &model);
    let judged = judge_endpoints(&table, &model, &[0]);
    assert_eq!(
        judged[0].verdict,
        hale_types::verdict::Verdict::Holds
    );
    // A subject whose WIRE ADDRESS text happens to be "Orders" —
    // not the topic's wire subject "wire.orders".
    model.entities.subjects.push(hale_model::Subject {
        pattern: "Orders".to_string(),
        exact: true,
        provenance: hale_model::ProvenanceId(0),
    });
    let collider = (model.entities.subjects.len() - 1) as u32;
    model.holes.push(hale_model::Hole {
        at: hale_model::EntityRef::Subject(hale_model::SubjectId(
            collider,
        )),
        kind: hale_model::HoleKind::DynamicEndpoint,
        hides: hale_model::RelationSet::SUBSCRIBES,
        authored_site: None,
        reason: "subscriber set incomplete".to_string(),
        provenance: hale_model::ProvenanceId(0),
    });
    let judged = judge_endpoints(&table, &model, &[0]);
    assert_eq!(
        judged[0].verdict,
        hale_types::verdict::Verdict::Holds,
        "the literal wire address `Orders` is not topic Orders' \
         wire — its hole must not touch the topic's count"
    );
    // Control: a hole at the topic's REAL wire flips it.
    let real = model
        .entities
        .subjects
        .iter()
        .position(|su| su.pattern == "wire.orders")
        .expect("real wire");
    model.holes.push(hale_model::Hole {
        at: hale_model::EntityRef::Subject(hale_model::SubjectId(
            real as u32,
        )),
        kind: hale_model::HoleKind::DynamicEndpoint,
        hides: hale_model::RelationSet::SUBSCRIBES,
        authored_site: None,
        reason: "subscriber set incomplete".to_string(),
        provenance: hale_model::ProvenanceId(0),
    });
    let judged = judge_endpoints(&table, &model, &[0]);
    assert_eq!(
        judged[0].verdict,
        hale_types::verdict::Verdict::Uncertified,
        "the topic's real wire subject IS its delivery identity"
    );
}

/// Review pin (round 2): a DECLARATION-ONLY free fn in a `bound`
/// source group contributes zero — the evaluator's fn_set inserts
/// every named free fn, so the group is not projection-vacuous.
#[test]
fn declaration_only_free_fn_bound_counts_zero() {
    let src = r#"
effect money;
fn audit() { }
group auditors = { audit };
main locus App {
    params { n: Int = 0; }
    claims { cap: bound money <= 1 on paths from auditors; }
}
fn main() { App { }; }
"#;
    let out = diff_one(src, "declaration-only bound root");
    assert!(out.is_ok(), "old/new agree: {:?}", out);
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let model = derive_application_model(&bundle);
    let table = lower_claims(&bundle, &model);
    let judged = judge_bound(&table, &model, &[0]);
    assert_eq!(
        judged[0].verdict,
        hale_types::verdict::Verdict::Holds,
        "the empty free fn evaluates with contribution zero"
    );
}

/// Review pin (round 2): user and stdlib ALTERNATIVES of one
/// authored dispatch share the LOCAL site group — one runtime
/// dispatch folds with MAX, never a phantom sum across the local
/// ordinal and the summary-global dispatch-group id.
#[test]
fn mixed_dispatch_alternatives_share_one_group() {
    use hale_model::*;
    let mut prov = ProvenanceTable::default();
    prov.records.push(Provenance::Synthetic {
        origin: "test".to_string(),
    });
    let p = ProvenanceId(0);
    let f = |name: &str| Function {
        analyzed: true,
        name: name.to_string(),
        display: name.to_string(),
        kind: FunctionKind::Free,
        effects: Vec::new(),
        direct_effects: Vec::new(),
        attribution: Vec::new(),
        opaque_call: false,
        carries_user_class: false,
        provenance: p,
    };
    let mut m = ApplicationModel {
        header: ModelHeader {
            semantics: MODEL_SEMANTICS_V1,
            entrypoint: "main".to_string(),
        },
        entities: Entities {
            functions: vec![f("caller"), f("UserConf::pay")],
            groups: vec![Group {
                name: "roots".to_string(),
                display: "roots".to_string(),
                may_be_empty: false,
                provenance: p,
            }],
            effect_classes: vec![EffectClassDecl {
                name: "money".to_string(),
                declared: true,
                definition: EffectClassDefinition::Atomic,
                provenance: p,
            }],
            ..Entities::default()
        },
        relations: Relations::default(),
        labels: vec![LabelRow {
            at: EntityRef::Function(FunctionId(1)),
            label: "money".to_string(),
            provenance: p,
        }],
        weights: Vec::new(),
        holes: Vec::new(),
        capabilities: Capabilities::default(),
        provenance: prov,
        legacy: LegacyProjection {
            topology_v1_fns: vec![FunctionId(0), FunctionId(1)],
            topology_v1_calls_via_stdlib: Vec::new(),
            // The stdlib alternative of the SAME authored dispatch
            // (site 0), carrying a summary-global group id that
            // differs from the local ordinal.
            stdlib_absorption: vec![StdlibAbsorption {
                from: FunctionId(0),
                site: 0,
                entry_dispatch: Some((
                    "Payer".to_string(),
                    "pay".to_string(),
                )),
                entry_in_loop: false,
                entry_group: Some(7),
                entry_provenance: p,
                nodes: vec![AbsorbedNode {
                    display: "std::pay::charge".to_string(),
                    carries: vec!["money".to_string()],
                    direct_effects: Vec::new(),
                    events: Vec::new(),
                }],
            }],
        },
    };
    m.relations.group_members.push(GroupMember {
        group: GroupId(0),
        member: EntityRef::Function(FunctionId(0)),
        provenance: p,
    });
    // The user alternative at the same site 0.
    m.relations.calls.push(Call {
        from: FunctionId(0),
        to: FunctionId(1),
        dispatch: DispatchKind::Interface {
            interface: "Payer".to_string(),
        },
        site: 0,
        in_loop: false,
        unbounded: false,
        provenance: p,
    });
    let mut t = ClaimIrTable::default();
    t.provenance.records.push(Provenance::Synthetic {
        origin: "test".to_string(),
    });
    t.rows.push(ClaimRow {
        ordinal: 0,
        name: "cap".to_string(),
        origin: ClaimOrigin::Main,
        law: ClaimIr::Bound {
            class: EffectClassRef {
                class: Some(EffectClassId(0)),
                builtin: false,
                name: "money".to_string(),
                provenance: ProvenanceId(0),
            },
            limit: 1,
            from: GroupRef {
                group: Some(GroupId(0)),
                name: NameRef {
                    raw: "roots".to_string(),
                    display: "roots".to_string(),
                },
                provenance: ProvenanceId(0),
            },
        },
        provenance: ProvenanceId(0),
    });
    let judged = judge_bound(&t, &m, &[0]);
    assert_eq!(
        judged[0].verdict,
        hale_types::verdict::Verdict::Holds,
        "one dispatch = one group: max(1, 1) = 1, within the \
         limit — a sum would report a phantom 2: {:?}",
        judged[0]
            .diags
            .iter()
            .map(|d| &d.message)
            .collect::<Vec<_>>()
    );
}

/// Review pin (round 2): a `bound` over a cyclically-defined class
/// is Invalid before evaluation — never Holds by counting zero.
#[test]
fn cyclic_bound_class_is_invalid() {
    let src = r#"
effect money;
fn audit() { }
group auditors = { audit };
main locus App {
    params { n: Int = 0; }
    claims { cap: bound money <= 1 on paths from auditors; }
}
fn main() { App { }; }
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let mut model = derive_application_model(&bundle);
    let table = lower_claims(&bundle, &model);
    let money = model
        .entities
        .effect_classes
        .iter()
        .position(|c| c.name == "money")
        .expect("money class");
    model.entities.effect_classes[money].definition =
        hale_model::EffectClassDefinition::InvalidCycle;
    let judged = judge_bound(&table, &model, &[0]);
    assert_eq!(
        judged[0].verdict,
        hale_types::verdict::Verdict::Invalid,
        "a cyclic class is not a countable law"
    );
    assert!(
        judged[0]
            .diags
            .iter()
            .any(|d| d.message.contains("defined in terms of itself")),
        "the refusal names the cycle"
    );
}

/// Review pin (round 3): a fn whose EFFECTS rows are hidden has an
/// UNKNOWN own-count — `bound` must not count zero and certify.
#[test]
fn effects_hole_makes_bound_uncertified() {
    let src = r#"
effect money;
locus A {
    params { n: Int = 0; }
    fn go(v: Int) -> Int { return v; }
}
group roots = { A };
main locus App {
    params { a: A = A { }; }
    claims { cap: bound money <= 1 on paths from roots; }
    run() { println(self.a.go(1)); }
}
fn main() { App { }; }
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let mut model = derive_application_model(&bundle);
    let table = lower_claims(&bundle, &model);
    let judged = judge_bound(&table, &model, &[0]);
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
        reason: "carrier facts incomplete".to_string(),
        provenance: hale_model::ProvenanceId(0),
    });
    let judged = judge_bound(&table, &model, &[0]);
    assert_eq!(
        judged[0].verdict,
        hale_types::verdict::Verdict::Uncertified,
        "hidden carrier facts must fail the count closed"
    );
    assert!(
        judged[0].diags[0].message.contains("unknown"),
        "{}",
        judged[0].diags[0].message
    );
}

/// Review pin (round 3): a publish whose subject's subscriber set
/// is incomplete makes the fan-out's contribution unknown — the
/// bound is Uncertified, never certified from the known rows.
#[test]
fn subject_subscribes_hole_makes_bound_uncertified() {
    let src = r#"
effect money;
topic Sig { payload: Int; subject: "app.sig"; }
locus A {
    params { n: Int = 0; }
    fn go(v: Int) { Sig <- v; }
}
group roots = { A };
main locus App {
    params { a: A = A { }; }
    claims { cap: bound money <= 1 on paths from roots; }
    run() { self.a.go(1); }
}
fn main() { App { }; }
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let mut model = derive_application_model(&bundle);
    let table = lower_claims(&bundle, &model);
    let judged = judge_bound(&table, &model, &[0]);
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
        hides: hale_model::RelationSet::SUBSCRIBES,
        authored_site: None,
        reason: "subscriber set incomplete".to_string(),
        provenance: hale_model::ProvenanceId(0),
    });
    let judged = judge_bound(&table, &model, &[0]);
    assert_eq!(
        judged[0].verdict,
        hale_types::verdict::Verdict::Uncertified,
        "unknown subscribers must fail the count closed"
    );
}

/// Review pin (round 4): `bound` applies the delivery predicate to
/// subscriber holes — a wildcard hole at `audit.**` makes fan-out
/// through `audit.event` unknown.
#[test]
fn wildcard_subscriber_hole_makes_bound_uncertified() {
    let src = r#"
effect money;
topic Ev { payload: Int; subject: "audit.event"; }
locus A {
    params { n: Int = 0; }
    fn go(v: Int) { Ev <- v; }
}
group roots = { A };
main locus App {
    params { a: A = A { }; }
    claims { cap: bound money <= 1 on paths from roots; }
    run() { self.a.go(1); }
}
fn main() { App { }; }
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let mut model = derive_application_model(&bundle);
    let table = lower_claims(&bundle, &model);
    let judged = judge_bound(&table, &model, &[0]);
    assert_eq!(
        judged[0].verdict,
        hale_types::verdict::Verdict::Holds
    );
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
    let judged = judge_bound(&table, &model, &[0]);
    assert_eq!(
        judged[0].verdict,
        hale_types::verdict::Verdict::Uncertified,
        "`audit.**` may cover `audit.event` — the count must fail \
         closed"
    );
}

/// Review pin (round 5): an incomplete count must not erase an
/// already-proven violation — the KNOWN lower bound decides first,
/// and the unknown flag only downgrades a would-be Holds.
#[test]
fn known_violation_survives_effects_hole() {
    let src = r#"
effect money;
@effects(is: { money })
fn charge(v: Int) -> Int { return v; }
locus A {
    params { n: Int = 0; }
    fn go(v: Int) -> Int { return charge(v); }
}
group roots = { A };
main locus App {
    params { a: A = A { }; }
    claims { cap: bound money <= 0 on paths from roots; }
    run() { println(self.a.go(1)); }
}
fn main() { App { }; }
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let mut model = derive_application_model(&bundle);
    let table = lower_claims(&bundle, &model);
    let judged = judge_bound(&table, &model, &[0]);
    assert_eq!(
        judged[0].verdict,
        hale_types::verdict::Verdict::Violated,
        "one known carrier over a limit of zero"
    );
    // Hiding MORE effects cannot un-prove the known violation.
    let charge = model
        .entities
        .functions
        .iter()
        .position(|f| f.display == "charge")
        .expect("charge");
    model.holes.push(hale_model::Hole {
        at: hale_model::EntityRef::Function(hale_model::FunctionId(
            charge as u32,
        )),
        kind: hale_model::HoleKind::UnanalyzedBody,
        hides: hale_model::RelationSet::EFFECTS,
        authored_site: None,
        reason: "carrier facts incomplete".to_string(),
        provenance: hale_model::ProvenanceId(0),
    });
    let judged = judge_bound(&table, &model, &[0]);
    assert_eq!(
        judged[0].verdict,
        hale_types::verdict::Verdict::Violated,
        "the known lower bound already proves the violation"
    );
}

/// …and the fan-out equivalent: known subscriber paths already
/// over the limit stay Violated under a subscriber hole.
#[test]
fn known_fanout_violation_survives_subscriber_hole() {
    let src = r#"
effect money;
topic Sig { payload: Int; subject: "app.sig"; }
@effects(is: { money })
fn charge(v: Int) -> Int { return v; }
locus A {
    params { n: Int = 0; }
    fn go(v: Int) { Sig <- v; }
}
locus B {
    params { n: Int = 0; }
    bus { subscribe Sig as on_sig; }
    fn on_sig(v: Int) { self.n = charge(v); }
}
group roots = { A };
main locus App {
    params { a: A = A { }; b: B = B { }; }
    claims { cap: bound money <= 0 on paths from roots; }
    run() { self.a.go(1); }
}
fn main() { App { }; }
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let mut model = derive_application_model(&bundle);
    let table = lower_claims(&bundle, &model);
    let judged = judge_bound(&table, &model, &[0]);
    assert_eq!(
        judged[0].verdict,
        hale_types::verdict::Verdict::Violated,
        "the known subscriber path already carries one site"
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
    let judged = judge_bound(&table, &model, &[0]);
    assert_eq!(
        judged[0].verdict,
        hale_types::verdict::Verdict::Violated,
        "known contributions still count under the hole"
    );
}

/// Review pin (round 5): topic-grain subscriber holes reach the
/// count walk through the shared index.
#[test]
fn topic_grain_subscriber_hole_makes_bound_uncertified() {
    let src = r#"
effect money;
topic Sig { payload: Int; subject: "app.sig"; }
locus A {
    params { n: Int = 0; }
    fn go(v: Int) { Sig <- v; }
}
group roots = { A };
main locus App {
    params { a: A = A { }; }
    claims { cap: bound money <= 1 on paths from roots; }
    run() { self.a.go(1); }
}
fn main() { App { }; }
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let mut model = derive_application_model(&bundle);
    let table = lower_claims(&bundle, &model);
    let judged = judge_bound(&table, &model, &[0]);
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
    let judged = judge_bound(&table, &model, &[0]);
    assert_eq!(
        judged[0].verdict,
        hale_types::verdict::Verdict::Uncertified,
        "a topic-grain hole must reach the count walk"
    );
}

/// Review pin (round 6): the model REJECTS two direct calls
/// sharing one (from, site) — the schema's contract is that
/// multiple rows at one site are conformer alternatives of one
/// interface dispatch (`bound` folds them with MAX; two direct
/// calls are two sites and must SUM), so the shape the fold relies
/// on is validated, not assumed.
#[test]
fn two_direct_calls_cannot_share_a_site() {
    let src = r#"
fn charge_a(v: Int) -> Int { return v; }
fn charge_b(v: Int) -> Int { return v; }
locus A {
    params { n: Int = 0; }
    fn go(v: Int) -> Int { return charge_a(v) + charge_b(v); }
}
main locus App {
    params { a: A = A { }; }
    run() { println(self.a.go(1)); }
}
fn main() { App { }; }
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let mut model = derive_application_model(&bundle);
    model.validate().expect("the built model is lawful");
    // Force the unlawful shape: both direct calls at one site.
    let mut sites: Vec<u32> = model
        .relations
        .calls
        .iter()
        .filter(|c| {
            matches!(c.dispatch, hale_model::DispatchKind::Direct)
        })
        .map(|c| c.site)
        .collect();
    sites.sort();
    sites.dedup();
    assert!(
        sites.len() >= 2,
        "two direct calls occupy two authored sites"
    );
    let target = sites[0];
    for c in &mut model.relations.calls {
        if matches!(c.dispatch, hale_model::DispatchKind::Direct) {
            c.site = target;
        }
    }
    model
        .relations
        .calls
        .sort_by_key(|c| (c.from.0, c.to.0, c.site));
    assert!(
        model.validate().is_err(),
        "two direct calls at one (from, site) must be rejected"
    );
}

/// Review pin (round 7): interior dispatch identity is validated —
/// a group without a dispatch rendering, and one group id shared
/// by two DIFFERENT dispatches, are both rejected (an arbitrary
/// group bucket would let `bound`'s per-group MAX absorb what must
/// sum).
#[test]
fn interior_dispatch_groups_are_validated() {
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
    let carrier = |m: &mut hale_model::ApplicationModel,
                   display: &str| {
        let n = m.legacy.stdlib_absorption[0].nodes.len() as u32;
        m.legacy.stdlib_absorption[0].nodes.push(
            hale_model::AbsorbedNode {
                display: display.to_string(),
                carries: vec!["money".to_string()],
                direct_effects: Vec::new(),
                events: Vec::new(),
            },
        );
        n
    };
    // The review's counterexample: two dispatch-less calls sharing
    // group 7.
    let mut m = base.clone();
    let a = carrier(&mut m, "std::x::A::pay");
    let b = carrier(&mut m, "std::x::B::pay");
    for t in [a, b] {
        m.legacy.stdlib_absorption[0].nodes[0].events.push(
            hale_model::AbsorbedEvent::Call {
                target: hale_model::AbsorbedTarget::Interior(t),
                dispatch: None,
                in_loop: false,
                group: Some(7),
            },
        );
    }
    assert!(
        m.validate().is_err(),
        "a group without a dispatch rendering is not a defined shape"
    );
    // One group id, two DIFFERENT dispatch identities.
    let mut m = base.clone();
    let a = carrier(&mut m, "std::x::A::pay");
    let b = carrier(&mut m, "std::x::B::refund");
    for (t, method) in [(a, "pay"), (b, "refund")] {
        m.legacy.stdlib_absorption[0].nodes[0].events.push(
            hale_model::AbsorbedEvent::Call {
                target: hale_model::AbsorbedTarget::Interior(t),
                dispatch: Some((
                    "Payer".to_string(),
                    method.to_string(),
                )),
                in_loop: false,
                group: Some(7),
            },
        );
    }
    assert!(
        m.validate().is_err(),
        "one group id inside a node is ONE dispatch"
    );
    // Genuine alternatives of one dispatch stay lawful.
    let mut m = base.clone();
    let a = carrier(&mut m, "std::x::A::pay");
    let b = carrier(&mut m, "std::x::B::pay");
    for t in [a, b] {
        m.legacy.stdlib_absorption[0].nodes[0].events.push(
            hale_model::AbsorbedEvent::Call {
                target: hale_model::AbsorbedTarget::Interior(t),
                dispatch: Some((
                    "Payer".to_string(),
                    "pay".to_string(),
                )),
                in_loop: false,
                group: Some(7),
            },
        );
    }
    m.validate()
        .expect("conformer alternatives of one dispatch are lawful");
}

/// Review pin (round 8): a set-level PUBLISHES hole makes bound's
/// fan-out a lower bound — Uncertified within the limit, while a
/// known over-limit count still proves the violation.
#[test]
fn publisher_hole_makes_bound_uncertified() {
    let src = r#"
effect money;
topic Sig { payload: Int; subject: "app.sig"; }
locus A {
    params { n: Int = 0; }
    fn go(v: Int) { Sig <- v; }
}
group roots = { A };
main locus App {
    params { a: A = A { }; }
    claims { cap: bound money <= 1 on paths from roots; }
    run() { self.a.go(1); }
}
fn main() { App { }; }
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let mut model = derive_application_model(&bundle);
    let table = lower_claims(&bundle, &model);
    let judged = judge_bound(&table, &model, &[0]);
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
    let judged = judge_bound(&table, &model, &[0]);
    assert_eq!(
        judged[0].verdict,
        hale_types::verdict::Verdict::Uncertified,
        "an unknown publisher may add fan-out"
    );
}

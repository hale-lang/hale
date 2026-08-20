//! GH #476 Change 6 — the artifact's law rows, projected from the
//! canonical path.
//!
//! `project_law_rows` renders `claims` rows from ClaimIr
//! (`ClaimRow::claims_form`, one authority) with verdicts from the
//! Change-5 judgments, and `lowered` rows from the evidence
//! sidecar. This differential holds the projection equal to the
//! evaluator-produced rows the artifact carried before, over the
//! whole corpus — name, form, result, source, and ORDER — with the
//! Change-5 documented divergences carved out explicitly (the
//! judgment's verdict is the more-correct one; the artifact adopts
//! it, and the SEMANTICS constant records the change).

use std::collections::BTreeMap;

use hale_types::claim_lowering::lower_claims;
use hale_types::evidence::derive_certificate_evidence;
use hale_types::model_builder::derive_application_model;
use hale_types::symbol::SourceFile;
use hale_types::topology_projection::project_law_rows;
use hale_types::verdict::Verdict;
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
    let evidence =
        derive_certificate_evidence(&bundle, &table, &model);
    let (claims, lowered, _law, _issues) = project_law_rows(
        &bundle,
        &model,
        &table,
        &evidence,
        &[0],
        &std::collections::BTreeMap::new(),
    );

    // ---- old: the evaluator rows the artifact serialized ----
    let programs_v: Vec<&hale_syntax::ast::Program> = vec![&program];
    let top = hale_types::resolve::build_top_scope(&bundle).0;
    let graph =
        hale_types::bus_graph::build_bus_graph(&bundle, &top);
    let (_d, outcomes, _a) =
        hale_types::claims::claims_report_with_identities(
            &programs_v,
            &graph,
            &[],
        );
    let old_lowered = hale_types::effects::certificate_rows(
        &programs_v,
        &[],
    );

    // ---- claims parity (with documented carve-outs) ----
    if outcomes.len() != claims.len() {
        return Err(format!(
            "{}: claims row count diverges: old {} / new {}",
            origin,
            outcomes.len(),
            claims.len()
        ));
    }
    let mut n = 0usize;
    for (o, c) in outcomes.iter().zip(claims.iter()) {
        n += 1;
        // The 5c documented divergence: `require attributed` over
        // an EFFECTS-hiding hole judges Uncertified where the
        // evaluator fail-opens to Holds.
        let attributed_carveout = o
            .form
            .starts_with("require attributed")
            && o.result == Verdict::Holds
            && c.result == Verdict::Uncertified
            && model.holes.iter().any(|h| {
                h.hides.intersects(
                    hale_model::RelationSet::EFFECTS,
                )
            });
        let verdict_ok =
            o.result == c.result || attributed_carveout;
        if o.name != c.name
            || o.form != c.form
            || !verdict_ok
            || o.source != c.source
        {
            return Err(format!(
                "{}: claims row diverges:\n  old: {:?} {:?} {:?} {:?}\n  \
                 new: {:?} {:?} {:?} {:?}",
                origin,
                o.name,
                o.form,
                o.result,
                o.source,
                c.name,
                c.form,
                c.result,
                c.source
            ));
        }
    }

    // ---- lowered parity (the effects family; budget/quant rows
    // keep their old producers and are compared in the artifact
    // itself) ----
    // Merge walk (round 8): the projection may carry EXTRA rows
    // the old evaluator never emitted — the synthetic `Holds`
    // certificate for an implicit lifecycle phase with no hook
    // body (a documented divergence: no hook performs no effects,
    // so the truthful certificate exists even though the legacy
    // walk skipped the phase). Everything else must match in
    // order.
    let mut oi = 0usize;
    for c in lowered.iter() {
        let o = old_lowered.get(oi);
        let matches_old = o.is_some_and(|o| {
            o.subject == c.subject && o.form == c.form
        });
        if !matches_old {
            let synthetic = c.form.contains(" during ")
                && c.result == Verdict::Holds;
            if synthetic {
                continue;
            }
            return Err(format!(
                "{}: lowered row diverges:\n  old: {:?}\n  \
                 new: {:?} {:?} {:?}",
                origin,
                o.map(|o| (&o.subject, &o.form, o.result)),
                c.subject,
                c.form,
                c.result
            ));
        }
        let o = o.expect("matched");
        n += 1;
        // The 5e documented divergences: cyclic and undeclared
        // classes judge Invalid where the evaluator's certificate
        // held vacuously.
        let class_carveout = c.result == Verdict::Invalid
            && o.result == Verdict::Holds;
        if o.result != c.result && !class_carveout {
            return Err(format!(
                "{}: lowered row diverges:\n  old: {:?} {:?} {:?}\n  \
                 new: {:?} {:?} {:?}",
                origin, o.subject, o.form, o.result, c.subject,
                c.form, c.result
            ));
        }
        oi += 1;
    }
    if oi != old_lowered.len() {
        return Err(format!(
            "{}: the projection dropped legacy lowered rows \
             (consumed {} of {})",
            origin,
            oi,
            old_lowered.len()
        ));
    }
    Ok(n)
}

/// THE Change-6 gate.
#[test]
fn projected_law_rows_match_the_evaluator_over_the_corpus() {
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
            Ok(Ok(k)) => rows += k,
        }
    }
    assert!(
        rows > 40,
        "the corpus must exercise law rows ({} seen)",
        rows
    );
    assert!(
        bad.is_empty(),
        "{} corpus programs diverge:\n{}",
        bad.len(),
        bad.join("\n\n")
    );
}

/// The typed sections (GH #476 Change 6): `law` rows are
/// addressable by ordinal with family + machine verdict, the two
/// evidence-tie digests recompute, and capabilities/adequacy carry
/// the typed completeness account.
#[test]
fn typed_sections_are_present_and_recomputable() {
    let src = r#"
effect money;
fn quiet(v: Int) -> Int { return v; }
locus A {
    params { n: Int = 0; }
    fn go(v: Int) -> Int { return quiet(v); }
}
group a_side = { A };
group b_side = { quiet };
main locus App {
    params { a: A = A { }; }
    claims { iso: forbid reaches(a_side, b_side); }
    run() { println(self.a.go(1)); }
}
fn main() { App { }; }
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let art = hale_types::topology::dump_topology(&bundle);
    let v: serde_json::Value =
        serde_json::from_str(&art).expect("valid JSON");
    // law rows: ordinal-addressable, family + verdict typed.
    let rows = v["law"]["rows"].as_array().expect("law.rows");
    let iso = rows
        .iter()
        .find(|r| r["name"] == "iso")
        .expect("the claim row");
    assert_eq!(iso["family"], "reachability");
    assert_eq!(iso["verdict"], "violated");
    assert_eq!(iso["origin"], "main");
    assert!(iso["ordinal"].is_u64());
    assert!(
        iso["file"].as_str().is_some_and(|f| f == "app.hl"),
        "law rows carry source provenance: {}",
        iso
    );
    // the digests recompute against the canonical path — round 7:
    // `law_digest` is the EXTERNAL fingerprint, recomputable from
    // the parsed document alone (canonical serde_json rendering of
    // the rows, fnv1a64).
    fn fnv1a64(bytes: &[u8]) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        for b in bytes {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x100000001b3);
        }
        h
    }
    assert_eq!(
        v["law"]["law_digest"].as_str().unwrap(),
        format!(
            "{:016x}",
            fnv1a64(
                serde_json::to_string(&serde_json::json!({
                    "issues": v["law"]["issues"],
                    "rows": v["law"]["rows"],
                }))
                .unwrap()
                .as_bytes()
            )
        ),
        "law_digest recomputes from the parsed rows + issues"
    );
    let model = derive_application_model(&bundle);
    let table = lower_claims(&bundle, &model);
    let _ = &table;
    assert_eq!(
        v["law"]["inputs_digest"].as_str().unwrap(),
        format!(
            "{:016x}",
            hale_types::evidence::analysis_inputs_digest()
        )
    );
    // capabilities + adequacy: this program is fully analyzable.
    assert_eq!(v["capabilities"]["exact_calls"], true);
    assert_eq!(v["adequacy"]["reachability"], "exact");
    assert_eq!(v["adequacy"]["certificate"], "exact");
}

/// Adequacy, both directions (review round 1): a computed publish
/// subject is unresolved knowledge for the CERTIFICATE family too
/// (`@effects(publish: {…})` cannot prove the subject in-set), so
/// every family degrades; and a fully known program — including a
/// `count` claim, whose multiplicity needs CARDINALITY — is
/// `exact` across the board, because the builder now derives
/// `exact_cardinality` from the closed-world enumeration.
#[test]
fn adequacy_tracks_capabilities_per_family() {
    // Negative control: a computed subject inside a fn carrying a
    // publish-set certificate.
    let src = r#"
type Cmd { v: Int = 0; }
topic Allowed { payload: Cmd; subject: "app.allowed"; }
locus A {
    params { n: Int = 0; }
    fn go(v: Int) { self.n <- v; }
}
main locus App {
    params { a: A = A { }; }
    run() { self.a.go(1); }
}
fn main() { App { }; }
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let art = hale_types::topology::dump_topology(&bundle);
    let v: serde_json::Value =
        serde_json::from_str(&art).expect("valid JSON");
    assert_eq!(v["capabilities"]["exact_publishes"], false);
    for fam in [
        "reachability",
        "boundary",
        "endpoint",
        "bound",
        "certificate",
    ] {
        assert_eq!(
            v["adequacy"][fam], "degraded",
            "a computed publish degrades `{}` — the publish-set \
             certificate cannot prove a computed subject in-set",
            fam
        );
    }

    // Closed world: endpoint counts are exact.
    let src = r#"
type Cmd { v: Int = 0; }
topic T { payload: Cmd; subject: "app.t" ; }
locus Pub {
    params { n: Int = 0; }
    bus { publish T; }
    fn act() { T <- Cmd { }; }
}
main locus App {
    params { p: Pub = Pub { }; }
    claims { one: count publishers(topic T) == 1; }
    run() { println(1); }
}
fn main() { App { }; }
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let art = hale_types::topology::dump_topology(&bundle);
    let v: serde_json::Value =
        serde_json::from_str(&art).expect("valid JSON");
    assert_eq!(v["capabilities"]["exact_cardinality"], true);
    for fam in [
        "reachability",
        "boundary",
        "endpoint",
        "bound",
        "certificate",
    ] {
        assert_eq!(
            v["adequacy"][fam], "exact",
            "a fully known program is exact for `{}`",
            fam
        );
    }
    let law = v["law"]["rows"].as_array().expect("law.rows");
    let one = law
        .iter()
        .find(|r| r["name"] == "one")
        .expect("count row");
    assert_eq!(one["verdict"], "holds");
    assert_eq!(one["law"]["kind"], "count");
    assert_eq!(one["law"]["n"], 1);
    assert_eq!(one["law"]["topic"]["display"], "T");
}

/// The semantics bump, end to end (MODEL_SEMANTICS 2): a
/// certificate naming a cyclically-defined class reports `invalid`
/// in the artifact — and the document verdict is `law_failed` —
/// where semantics 1 replayed a vacuous `holds` / `clean`.
#[test]
fn cyclic_class_artifact_reports_invalid() {
    let src = r#"
effect a = { b };
effect b = { a };
@effects(none: { a })
fn f(v: Int) -> Int { return v; }
main locus App {
    run() { println(f(1)); }
}
fn main() { App { }; }
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let art = hale_types::topology::dump_topology(&bundle);
    let v: serde_json::Value =
        serde_json::from_str(&art).expect("valid JSON");
    assert_eq!(v["semantics"], 2);
    // The three-level story: the legacy `lowered` row preserves
    // the ENGINE's replay (a vacuous holds), the typed law row
    // carries the MACHINE verdict (invalid — a cyclic class is not
    // a valid denotation), and the document verdict follows the
    // machine.
    let lowered = v["lowered"].as_array().expect("lowered");
    assert!(
        lowered.iter().any(|r| r["result"] == "holds"),
        "the engine replay is preserved in `lowered`"
    );
    let law = v["law"]["rows"].as_array().expect("law.rows");
    assert!(
        law.iter().any(|r| r["family"] == "certificate"
            && r["verdict"] == "invalid"),
        "the machine verdict is invalid: {}",
        v["law"]["rows"]
    );
    assert_eq!(v["verdict"], "law_failed");
}

/// Review round 1: NO non-passing law row can coexist with a
/// `clean` document verdict — over every corpus artifact. The
/// unmigrated families carry the old engines' authoritative
/// results (`legacy_unmigrated_verdicts`), so the invariant is
/// total: `clean` ⟺ every application-tier law row holds.
#[test]
fn clean_verdict_implies_every_law_row_holds() {
    let mut bad: Vec<String> = Vec::new();
    let mut law_rows_seen = 0usize;
    for p in
        hale_corpus::parseable(|s| hale_syntax::parse_source(s).is_ok())
    {
        let Ok(program) = hale_syntax::parse_source(&p.source)
        else {
            continue;
        };
        let caught = std::panic::catch_unwind(
            std::panic::AssertUnwindSafe(|| {
                let bundle = bundle_of(&p.source, &program);
                hale_types::topology::dump_topology(&bundle)
            }),
        );
        let Ok(art) = caught else {
            bad.push(format!("{}: PANIC", p.origin));
            continue;
        };
        let Ok(v) =
            serde_json::from_str::<serde_json::Value>(&art)
        else {
            bad.push(format!("{}: invalid JSON", p.origin));
            continue;
        };
        let clean = v["verdict"] == "clean";
        for r in v["law"]["rows"].as_array().into_iter().flatten()
        {
            law_rows_seen += 1;
            if r["family"] == "fleet" {
                continue;
            }
            if clean && r["verdict"] != "holds" {
                bad.push(format!(
                    "{}: clean artifact carries a non-holds law \
                     row: {}",
                    p.origin, r
                ));
            }
        }
    }
    assert!(
        law_rows_seen > 60,
        "the corpus must exercise law rows ({} seen)",
        law_rows_seen
    );
    assert!(
        bad.is_empty(),
        "{} violations:\n{}",
        bad.len(),
        bad.join("\n")
    );
}

/// Review round 1: unmigrated families carry the OLD engines'
/// authoritative results in the law section — a passing `@budget`
/// and a passing `causes:` both read `holds` (with the budget's
/// legacy `lowered` row agreeing), and a violated budget reads
/// `violated`; the `uncertified` no-engine placeholder never
/// reaches the artifact where legacy truth exists.
#[test]
fn unmigrated_rows_carry_legacy_verdicts() {
    let src = r#"
effect money;
topic Sig { payload: Int; subject: "app.sig"; }
@effects(causes: { money })
fn poke(v: Int) { Sig <- v; }
@budget(alloc_per_call = 4)
fn tight(v: Int) -> Int { return v + 1; }
locus Handler {
    params { n: Int = 0; }
    bus { subscribe Sig as on_sig; }
    fn on_sig(v: Int) { self.n = charge(v); }
}
@effects(is: { money })
fn charge(v: Int) -> Int { return v; }
main locus App {
    params { h: Handler = Handler { }; }
    run() { poke(1); println(tight(1)); }
}
fn main() { App { }; }
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let art = hale_types::topology::dump_topology(&bundle);
    let v: serde_json::Value =
        serde_json::from_str(&art).expect("valid JSON");
    let rows = v["law"]["rows"].as_array().expect("law.rows");
    let by_kind = |kind: &str| -> &serde_json::Value {
        rows.iter()
            .find(|r| r["law"]["kind"] == kind)
            .unwrap_or_else(|| panic!("row of kind {}", kind))
    };
    let causes = by_kind("effect_causes");
    assert_eq!(causes["family"], "unmigrated");
    assert_eq!(
        causes["verdict"], "holds",
        "the old causes engine certifies the publish->handler \
         path: {}",
        causes
    );
    let budget = by_kind("alloc_budget");
    assert_eq!(
        budget["verdict"], "holds",
        "the old budget engine's result, not the no-engine \
         placeholder: {}",
        budget
    );
    assert_eq!(v["verdict"], "clean");

    // A VIOLATED budget flows through and fails the document.
    let src = r#"
type P { a: Int = 0; }
@budget(alloc_per_call = 0)
fn hot(v: Int) -> P { return P { a: v }; }
main locus App {
    run() { println(hot(1).a); }
}
fn main() { App { }; }
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let art = hale_types::topology::dump_topology(&bundle);
    let v: serde_json::Value =
        serde_json::from_str(&art).expect("valid JSON");
    let rows = v["law"]["rows"].as_array().expect("law.rows");
    let budget = rows
        .iter()
        .find(|r| r["law"]["kind"] == "alloc_budget")
        .expect("budget row");
    assert_eq!(
        budget["verdict"], "violated",
        "a violated budget carries the legacy verdict: {}",
        budget
    );
    assert_eq!(v["verdict"], "law_failed");
}

/// The typed payload carries the law's OPERANDS (review round 1):
/// a two-class forbid names both classes; per-certificate evidence
/// is keyed by (law ordinal, certificate ordinal) and says WHICH
/// certificate failed.
#[test]
fn typed_payload_carries_operands_and_certs()  {
    let src = r#"
locus A {
    params { n: Int = 0; }
    fn go(v: Int) { self.n <- v; }
}
@effects(none: { syscall, publish })
fn f(v: Int) { A { }.go(v); }
main locus App {
    run() { f(1); }
}
fn main() { App { }; }
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let art = hale_types::topology::dump_topology(&bundle);
    let v: serde_json::Value =
        serde_json::from_str(&art).expect("valid JSON");
    let rows = v["law"]["rows"].as_array().expect("law.rows");
    let forbid = rows
        .iter()
        .find(|r| r["law"]["kind"] == "effect_forbid")
        .expect("forbid row");
    let classes = forbid["law"]["classes"]
        .as_array()
        .expect("classes");
    let names: Vec<&str> = classes
        .iter()
        .filter_map(|c| c["class"].as_str())
        .collect();
    assert_eq!(names, ["syscall", "publish"]);
    assert_eq!(forbid["law"]["at"]["display"], "f");
    // per-certificate evidence: publish violated, syscall holds.
    let certs = forbid["certs"].as_array().expect("certs");
    assert_eq!(certs.len(), 2);
    assert_eq!(certs[0]["ordinal"], 0);
    assert_eq!(certs[0]["result"], "holds");
    assert!(
        certs[0]["form"]
            .as_str()
            .unwrap()
            .contains("effects(syscall)")
    );
    assert_eq!(certs[1]["result"], "violated");
    assert!(
        certs[1]["form"]
            .as_str()
            .unwrap()
            .contains("effects(publish)")
    );
}

/// Review round 2: the legacy bridge never manufactures `holds` —
/// a module-scoped `causes:` is lowered but the old engine's
/// nonrecursive walk never evaluated it, so its row stays
/// `uncertified` (and the document verdict follows); two asserts
/// on one fn share one diagnostic anchor, so when a diagnostic
/// exists neither row can claim it.
#[test]
fn unenumerated_and_ambiguous_rows_stay_uncertified() {
    // Module-scoped: lowered, never evaluated.
    let src = r#"
effect money;
module billing {
    @effects(causes: { money })
    fn poke(v: Int) -> Int { return v; }
}
main locus App {
    params { n: Int = 0; }
    run() { println(1); }
}
fn main() { App { }; }
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let art = hale_types::topology::dump_topology(&bundle);
    let v: serde_json::Value =
        serde_json::from_str(&art).expect("valid JSON");
    let rows = v["law"]["rows"].as_array().expect("law.rows");
    let causes = rows
        .iter()
        .find(|r| r["law"]["kind"] == "effect_causes")
        .expect("the module-scoped row IS lowered");
    assert_eq!(
        causes["verdict"], "uncertified",
        "no old-engine evidence exists for a module-scoped row — \
         a missing diagnostic must not become holds: {}",
        causes
    );
    assert_eq!(
        v["verdict"], "law_failed",
        "an unwitnessed law cannot leave the document clean"
    );
}

/// Review round 2: adequacy answers per RELATION, not per coupled
/// capability — a subscriber-only endpoint hole leaves the
/// certificate family (CALLS | EFFECTS | PUBLISHES) `exact` while
/// the bus-composing families degrade.
#[test]
fn subscriber_only_hole_keeps_certificate_exact() {
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
    let mut model = derive_application_model(&bundle);
    let sid = model
        .entities
        .subjects
        .iter()
        .position(|su| su.pattern == "app.sig")
        .expect("subject");
    // Only SUBSCRIBES is incomplete — publishes stay vouched
    // (round 3: the flags are independent, and adequacy reads the
    // positive account).
    model.capabilities.exact_subscribes = false;
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
    model.validate().expect("lawful");
    let adequacy: std::collections::BTreeMap<_, _> =
        hale_types::topology_projection::family_adequacy(&model)
            .into_iter()
            .collect();
    assert!(
        adequacy[&hale_model::JudgmentFamily::Certificate],
        "SUBSCRIBES-only incompleteness does not touch the \
         certificate family"
    );
    assert!(
        !adequacy[&hale_model::JudgmentFamily::Reachability],
        "the bus-composing families degrade"
    );
    assert!(!adequacy[&hale_model::JudgmentFamily::Endpoint]);
}

/// Review round 2: bus selectors serialize their CANDIDATE sets —
/// the normalized topic identities the selector matched — plus
/// the selector's own source location.
#[test]
fn publish_set_selector_carries_candidates() {
    let src = r#"
type Cmd { v: Int = 0; }
topic Allowed { payload: Cmd; subject: "app.allowed"; }
@effects(publish: { Allowed })
fn f(v: Int) { Allowed <- Cmd { }; }
main locus App {
    run() { f(1); }
}
fn main() { App { }; }
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let art = hale_types::topology::dump_topology(&bundle);
    let v: serde_json::Value =
        serde_json::from_str(&art).expect("valid JSON");
    let rows = v["law"]["rows"].as_array().expect("law.rows");
    let ps = rows
        .iter()
        .find(|r| r["law"]["kind"] == "effect_publish_set")
        .expect("publish-set row");
    let entry = &ps["law"]["entries"][0];
    assert_eq!(entry["name"], "Allowed");
    assert_eq!(
        entry["topics"][0]["name"], "Allowed",
        "the candidate set is the selector's meaning: {}",
        entry
    );
    assert!(
        entry["span"].is_array(),
        "the selector carries its own source location: {}",
        entry
    );
}

/// Review round 3: `opaque` is not a reserved word — a struct
/// whose field is literally named `opaque` has structural shape
/// `opaque:i`, which must survive into the topics section with
/// the shared observation-identity hash (never erased by sentinel
/// string-matching).
#[test]
fn opaque_named_field_shape_is_not_erased() {
    let src = r#"
type Payload { opaque: Int = 0; }
topic Events { payload: Payload; subject: "app.events"; }
locus A {
    params { n: Int = 0; }
    bus { publish Events; }
    fn act() { Events <- Payload { }; }
}
main locus App {
    params { a: A = A { }; }
    run() { self.a.act(); }
}
fn main() { App { }; }
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let model = derive_application_model(&bundle);
    let payload = model
        .entities
        .payloads
        .iter()
        .find(|p| p.shape == "opaque:i")
        .expect("the structural shape is stored");
    assert!(
        !payload.opaque,
        "a field literally named `opaque` is STRUCTURAL — the \
         discriminant is the flag, never the string"
    );
    let art = hale_types::topology::dump_topology(&bundle);
    let v: serde_json::Value =
        serde_json::from_str(&art).expect("valid JSON");
    let topics = v["topics"].as_array().expect("topics");
    let row = topics
        .iter()
        .find(|t| t["name"] == "Events")
        .expect("topic row");
    assert_eq!(
        row["shape"], "opaque:i",
        "the structural shape survives: {}",
        row
    );
    // …and the fused hash matches the shared observation-identity
    // implementation the binary registers with.
    let expected = hale_types::topic_identity::topic_shape_hash(
        "app.events",
        "opaque:i",
    );
    assert_eq!(
        row["payload_hash"].as_str().unwrap(),
        format!("{:016x}", expected),
        "artifact and manifest identities join"
    );
}

/// Review round 3: ONE V1 endpoint renderer — a literal subject
/// whose text collides with a mangled imported symbol renders the
/// SAME spelling in the relation and provenance sections, publish
/// and subscribe forms both.
#[test]
fn literal_collision_renders_identically_in_both_halves() {
    let lib = r#"
type __lib_x_events_Item { n: Int = 0; }
topic __lib_x_events_Changed { payload: __lib_x_events_Item; subject: "ev.changed"; }
"#;
    let main_src = r#"
type Note { n: Int = 0; }
locus Ops {
    params { n: Int = 0; }
    bus {
        publish "__lib_x_events_Changed" of type Note;
        subscribe "__lib_x_events_Changed" as on_ev;
    }
    fn act() { "__lib_x_events_Changed" <- Note { }; }
    fn on_ev(v: Note) { self.n = v.n; }
}
main locus App {
    params { o: Ops = Ops { }; }
    run() { self.o.act(); }
}
fn main() { App { }; }
"#;
    let main_p =
        hale_syntax::parse_source(main_src).expect("parse main");
    let lib_p = hale_syntax::parse_source(lib).expect("parse lib");
    let mut programs = BTreeMap::new();
    programs.insert("app/main.hl".to_string(), &main_p);
    programs.insert("lib/events.hl".to_string(), &lib_p);
    let mut bundle = Bundle::new(programs);
    bundle.import_renames = vec![
        (
            vec!["events".to_string(), "Item".to_string()],
            "__lib_x_events_Item".to_string(),
        ),
        (
            vec!["events".to_string(), "Changed".to_string()],
            "__lib_x_events_Changed".to_string(),
        ),
    ];
    let art = hale_types::topology::dump_topology(&bundle);
    let v: serde_json::Value =
        serde_json::from_str(&art).expect("valid JSON");
    // The V1 rule demangles the colliding literal EVERYWHERE.
    let rel_pub = v["relations"]["publishes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["fn"] == "Ops::act")
        .expect("relation publish row")["subject"]
        .as_str()
        .unwrap()
        .to_string();
    let prov_pub = v["provenance"]["publishes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["fn"] == "Ops::act")
        .expect("provenance publish row")["subject"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        rel_pub, prov_pub,
        "the provenance row must join the relation row it locates"
    );
    assert_eq!(rel_pub, "events::Changed");
    let rel_sub = v["relations"]["subscribes"]
        .as_array()
        .unwrap()
        .first()
        .expect("relation subscribe row")["subject"]
        .as_str()
        .unwrap()
        .to_string();
    let prov_sub = v["provenance"]["subscribes"]
        .as_array()
        .unwrap()
        .first()
        .expect("provenance subscribe row")["subject"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(rel_sub, prov_sub);
    assert_eq!(rel_sub, "events::Changed");
}

/// Review round 3: adequacy reads the POSITIVE account — a false
/// capability with no corresponding hole is a valid model state,
/// and the family reads `degraded`, never `exact` inferred from
/// the absence of recorded unknowns.
#[test]
fn unvouched_capability_degrades_adequacy() {
    let src = r#"
fn f(v: Int) -> Int { return v; }
main locus App {
    run() { println(f(1)); }
}
fn main() { App { }; }
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let mut model = derive_application_model(&bundle);
    model.capabilities.exact_effects = false;
    model.holes.clear();
    model.validate().expect("a withdrawn claim is lawful");
    let adequacy: std::collections::BTreeMap<_, _> =
        hale_types::topology_projection::family_adequacy(&model)
            .into_iter()
            .collect();
    assert!(
        !adequacy[&hale_model::JudgmentFamily::Certificate],
        "not vouched for is not exact — absence of holes is not \
         proof"
    );
    assert!(!adequacy[&hale_model::JudgmentFamily::Reachability]);
}

/// Review round 4: the law rows carry EVIDENCE, not bare verdicts
/// — a violated reachability law serializes its countermodel
/// diagnostics with source locations, and a violated certificate
/// keeps its root/leaf diagnostics.
#[test]
fn law_rows_carry_evidence() {
    let src = r#"
fn leak(v: Int) -> Int { return v; }
locus A {
    params { n: Int = 0; }
    fn go(v: Int) -> Int { return leak(v); }
}
group a_side = { A };
group b_side = { leak };
@effects(none: { publish })
fn f(v: Int) { A { }.go(v); "x" <- v; }
main locus App {
    params { a: A = A { }; }
    claims { iso: forbid reaches(a_side, b_side); }
    run() { println(self.a.go(1)); f(1); }
}
fn main() { App { }; }
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let art = hale_types::topology::dump_topology(&bundle);
    let v: serde_json::Value =
        serde_json::from_str(&art).expect("valid JSON");
    let rows = v["law"]["rows"].as_array().expect("law.rows");
    let iso = rows
        .iter()
        .find(|r| r["name"] == "iso")
        .expect("claim row");
    assert_eq!(iso["verdict"], "violated");
    let ev = iso["evidence"].as_array().expect("evidence");
    assert!(
        ev.iter().any(|d| {
            d["message"]
                .as_str()
                .is_some_and(|m| m.contains("violated"))
                && d["file"] == "app.hl"
                && d["span"].is_array()
        }),
        "the countermodel diagnostics survive with locations: {}",
        iso["evidence"]
    );
    let forbid = rows
        .iter()
        .find(|r| r["law"]["kind"] == "effect_forbid")
        .expect("certificate row");
    let certs = forbid["certs"].as_array().expect("certs");
    let violated = certs
        .iter()
        .find(|c| c["result"] == "violated")
        .expect("the publish certificate violates");
    assert!(
        violated["evidence"]
            .as_array()
            .is_some_and(|d| !d.is_empty()),
        "the certificate keeps its diagnostics: {}",
        violated
    );
}

/// Review round 4: the payload is LOSSLESS — `during` and `seed`
/// are typed references with resolution status, and a user-class
/// budget dimension carries its full class reference.
#[test]
fn payload_refs_are_lossless() {
    let src = r#"
effect money;
type Cmd { v: Int = 0; }
topic T { payload: Cmd; subject: "app.t"; }
locus A {
    params { n: Int = 0; }
    bus { subscribe T as on_t; }
    fn on_t(c: Cmd) { self.n = c.v; }
}
@budget(money = 2)
fn charge(v: Int) -> Int { return v; }
group a_side = { A };
group b_side = { charge };
main locus App {
    params { a: A = A { }; }
    claims {
        iso: forbid reaches(a_side, b_side) during boot;
    }
    run() { println(charge(1)); }
}
fn main() { App { }; }
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let art = hale_types::topology::dump_topology(&bundle);
    let v: serde_json::Value =
        serde_json::from_str(&art).expect("valid JSON");
    let rows = v["law"]["rows"].as_array().expect("law.rows");
    let iso = rows
        .iter()
        .find(|r| r["name"] == "iso")
        .expect("claim row");
    // `boot` is an UNRESOLVED phase — the typed ref records that.
    let during = &iso["law"]["during"];
    assert_eq!(during["display"], "boot");
    assert_eq!(
        during["resolved"], false,
        "an unresolved during keeps its status: {}",
        during
    );
    let qb = rows
        .iter()
        .find(|r| r["law"]["kind"] == "quant_budget")
        .expect("budget row");
    let dim = &qb["law"]["dim"];
    assert_eq!(
        dim["user_class"]["class"], "money",
        "a user-class dimension is a full class reference: {}",
        dim
    );
    assert_eq!(dim["user_class"]["resolved"], true);
}

/// Review round 5: a FOREIGN-space certificate diagnostic (stdlib
/// parse space) must never be re-resolved against bundle sources
/// at the ARTIFACT level. Self-calibrating like the judgment-side
/// pin: measure the stdlib offset, pad the user file until that
/// offset falls numerically INSIDE it, and require the projected
/// evidence entry to carry no location — a containment guess would
/// misfile stdlib evidence as application code.
#[test]
fn foreign_cert_evidence_gets_no_guessed_location() {
    let body = r#"
@effects(none: { alloc })
fn probe(r: std::http::Router, req: std::http::Request) -> Int {
    let resp = r.dispatch(req);
    return resp.status;
}
main locus App {
    params { n: Int = 0; }
    run() {
        let r = std::http::Router { };
        let req = std::http::Request { method: "GET", path: "/", body: "" };
        println(probe(r, req));
    }
}
fn main() { App { }; }
"#;
    // Measure the foreign offset from the evidence sidecar.
    let offset = {
        let program =
            hale_syntax::parse_source(body).expect("parse");
        let bundle = bundle_of(body, &program);
        let model = derive_application_model(&bundle);
        let table = lower_claims(&bundle, &model);
        let evidence =
            derive_certificate_evidence(&bundle, &table, &model);
        let mut found = None;
        for row in &evidence.rows {
            for cert in &row.certs {
                for (msg, pid) in &cert.diags {
                    if !msg.contains("happens here") {
                        continue;
                    }
                    if let hale_model::Provenance::ForeignSpan {
                        span,
                    } = evidence.provenance.records[pid.index()]
                    {
                        found = Some(span.0);
                    }
                }
            }
        }
        found.expect("the stdlib alloc leaf must be foreign")
    };
    let mut padded = body.to_string();
    while (padded.len() as u32) <= offset {
        padded.push_str("// padding to swallow the stdlib offset\n");
    }
    let program =
        hale_syntax::parse_source(&padded).expect("parse");
    let bundle = bundle_of(&padded, &program);
    let art = hale_types::topology::dump_topology(&bundle);
    let v: serde_json::Value =
        serde_json::from_str(&art).expect("valid JSON");
    let rows = v["law"]["rows"].as_array().expect("law.rows");
    let forbid = rows
        .iter()
        .find(|r| r["law"]["kind"] == "effect_forbid")
        .expect("annotation row");
    let mut saw_foreign = false;
    for cert in forbid["certs"].as_array().expect("certs") {
        for e in cert["evidence"].as_array().unwrap_or(&vec![]) {
            let msg = e["message"].as_str().unwrap_or("");
            if !msg.contains("happens here") {
                continue;
            }
            saw_foreign = true;
            assert!(
                e.get("file").is_none_or(|f| f.is_null()),
                "a stdlib-space diagnostic must not be attributed \
                 to a bundle file, even when its offsets fall \
                 inside one numerically: {}",
                e
            );
            assert!(
                e.get("span").is_none_or(|s| s.is_null()),
                "no guessed span either: {}",
                e
            );
        }
    }
    assert!(
        saw_foreign,
        "the stdlib alloc-leaf diagnostic must reach the \
         artifact: {}",
        forbid
    );
}

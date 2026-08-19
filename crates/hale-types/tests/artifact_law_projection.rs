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
    let (claims, lowered, _law) = project_law_rows(
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
    if old_lowered.len() != lowered.len() {
        return Err(format!(
            "{}: lowered row count diverges: old {} / new {}\n  \
             old: {:?}\n  new: {:?}",
            origin,
            old_lowered.len(),
            lowered.len(),
            old_lowered
                .iter()
                .map(|r| (&r.subject, &r.form))
                .collect::<Vec<_>>(),
            lowered
                .iter()
                .map(|r| (&r.subject, &r.form))
                .collect::<Vec<_>>()
        ));
    }
    for (o, c) in old_lowered.iter().zip(lowered.iter()) {
        n += 1;
        // The 5e documented divergences: cyclic and undeclared
        // classes judge Invalid where the evaluator's certificate
        // held vacuously.
        let class_carveout = c.result == Verdict::Invalid
            && o.result == Verdict::Holds;
        let verdict_ok = o.result == c.result || class_carveout;
        if o.subject != c.subject
            || o.form != c.form
            || !verdict_ok
        {
            return Err(format!(
                "{}: lowered row diverges:\n  old: {:?} {:?} {:?}\n  \
                 new: {:?} {:?} {:?}",
                origin, o.subject, o.form, o.result, c.subject,
                c.form, c.result
            ));
        }
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
    // the digests recompute against the canonical path.
    let model = derive_application_model(&bundle);
    let table = lower_claims(&bundle, &model);
    assert_eq!(
        v["law"]["law_digest"].as_str().unwrap(),
        format!("{:016x}", table.semantic_digest())
    );
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
    assert_eq!(v["capabilities"]["exact_bus_endpoints"], false);
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

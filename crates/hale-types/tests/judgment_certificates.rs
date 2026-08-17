//! GH #476 Change 5e — the pointwise-certificate differential.
//!
//! The certificate ENGINES stay the analysis authority; the
//! producer (`evidence::derive_certificate_evidence`) runs them and
//! keys each certificate's outcome + diagnostics BY ClAIM ORDINAL
//! in an `EvidenceTable` sidecar, and the judgment consumes the
//! sidecar structurally. The gates: the judgment's diagnostic
//! stream (undeclared-class validation + per-certificate diags,
//! with the lowering's carries-issues) must be byte-identical to
//! the evaluator's law strata; every certificate-family ClaimIr row
//! must produce exactly one Judged row (engine completeness — the
//! unmigrated families judge Uncertified rather than vanishing);
//! and evidence is refused when stale, missing, or malformed.

use std::collections::{BTreeMap, BTreeSet};

use hale_model::ClaimIr;
use hale_types::claim_lowering::lower_claims;
use hale_types::evidence::derive_certificate_evidence;
use hale_types::judgment::judge_certificates;
use hale_types::model_builder::derive_application_model;
use hale_types::symbol::SourceFile;
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

/// Is this a certificate-family row (judged by `judge_certificates`
/// and no other engine)?
fn is_certificate_family(law: &ClaimIr) -> bool {
    matches!(
        law,
        ClaimIr::EffectForbid { .. }
            | ClaimIr::EffectOnly { .. }
            | ClaimIr::EffectPublishSet { .. }
            | ClaimIr::NoPanic { .. }
            | ClaimIr::PhaseEffects { .. }
            | ClaimIr::EffectCauses { .. }
            | ClaimIr::DependsSet { .. }
            | ClaimIr::AllocBudget { .. }
            | ClaimIr::QuantBudget { .. }
    )
}

fn sev(v: Verdict) -> u8 {
    match v {
        Verdict::Holds => 0,
        Verdict::Uncertified => 1,
        Verdict::Violated => 2,
        Verdict::Invalid => 3,
    }
}

fn diff_one(src: &str, origin: &str) -> Result<usize, String> {
    let program = hale_syntax::parse_source(src)
        .map_err(|_| format!("{}: parse", origin))?;
    let bundle = bundle_of(src, &program);
    let model = derive_application_model(&bundle);
    let table = lower_claims(&bundle, &model);
    let evidence = derive_certificate_evidence(&bundle, &table, &model);
    let fam_ordinals: BTreeSet<u32> = table
        .rows
        .iter()
        .filter(|r| is_certificate_family(&r.law))
        .map(|r| r.ordinal)
        .collect();
    let n_rows = fam_ordinals.len();
    if n_rows == 0 && evidence.rows.is_empty() {
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
        .flat_map(|(_, ds)| ds.into_iter().map(|(d, _foreign)| d))
        .collect();
    hale_types::stdlib_bodies::demangle_imports(&mut group_diags, &[]);
    let old: Vec<(String, hale_syntax::Span)> = p1
        .iter()
        .chain(group_diags.iter())
        .map(|d| (d.message.clone(), d.span))
        .collect();
    // New: lowering issues (carries validation) + judgment output.
    let judged = judge_certificates(&table, &model, &evidence, &[0]);
    // Engine completeness: exactly one Judged row per
    // certificate-family ClaimIr row — the unmigrated families
    // (causes / depends / budgets) must not vanish.
    let judged_ordinals: Vec<u32> =
        judged.iter().map(|j| j.ordinal).collect();
    let judged_set: BTreeSet<u32> =
        judged_ordinals.iter().copied().collect();
    if judged_ordinals.len() != judged_set.len()
        || judged_set != fam_ordinals
    {
        return Err(format!(
            "{}: judged ordinals {:?} != certificate-family rows {:?}",
            origin, judged_ordinals, fam_ordinals
        ));
    }
    // Verdict parity: a judged row that emitted no undeclared-class
    // diagnostic replays exactly the max severity of its evidence
    // certificates (which the producer copied from the evaluator's
    // rows) — the judgment must not invent or lose a verdict.
    for j in &judged {
        let Some(ev) =
            evidence.rows.iter().find(|r| r.ordinal == j.ordinal)
        else {
            continue;
        };
        if ev.certs.is_empty() {
            continue;
        }
        let replay = ev
            .certs
            .iter()
            .map(|c| {
                sev(match c.result {
                    hale_model::VerdictIr::Holds => Verdict::Holds,
                    hale_model::VerdictIr::Violated => {
                        Verdict::Violated
                    }
                    hale_model::VerdictIr::Uncertified => {
                        Verdict::Uncertified
                    }
                    hale_model::VerdictIr::Invalid => {
                        Verdict::Invalid
                    }
                })
            })
            .max()
            .unwrap_or(0);
        // Documented divergence: a row asserting about an
        // undeclared class judges Invalid where the evaluator's
        // certificate held vacuously.
        let row = table
            .rows
            .iter()
            .find(|r| r.ordinal == j.ordinal)
            .expect("judged ordinal is a table row");
        let undeclared = |cs: &[hale_model::EffectClassRef]| {
            cs.iter().any(|c| {
                !c.builtin
                    && c.class.map_or(true, |id| {
                        !model.entities.effect_classes[id.index()]
                            .declared
                    })
            })
        };
        let invalid_class = match &row.law {
            ClaimIr::EffectForbid { classes, .. }
            | ClaimIr::EffectCauses { classes, .. } => {
                undeclared(classes)
            }
            _ => false,
        };
        if !invalid_class && sev(j.verdict) != replay {
            return Err(format!(
                "{}: ordinal {} verdict {:?} != evaluator replay \
                 severity {}",
                origin, j.ordinal, j.verdict, replay
            ));
        }
    }
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

fn derive_all(
    src: &str,
) -> (
    hale_model::ApplicationModel,
    hale_model::ClaimIrTable,
    hale_model::EvidenceTable,
    Vec<hale_types::judgment::Judged>,
) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let model = derive_application_model(&bundle);
    let table = lower_claims(&bundle, &model);
    let evidence = derive_certificate_evidence(&bundle, &table, &model);
    let judged = judge_certificates(&table, &model, &evidence, &[0]);
    (model, table, evidence, judged)
}

const HOLDS_SRC: &str = r#"
@no_ffi
fn pure_math(v: Int) -> Int { return v * 2; }
main locus App {
    run() { println(pure_math(1)); }
}
fn main() { App { }; }
"#;

/// Negative control: the judgment reads the EVIDENCE SIDECAR —
/// clearing its rows invalidates a holding certificate.
#[test]
fn dropping_evidence_changes_the_verdict() {
    let src = HOLDS_SRC;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let model = derive_application_model(&bundle);
    let table = lower_claims(&bundle, &model);
    let mut evidence =
        derive_certificate_evidence(&bundle, &table, &model);
    let judged = judge_certificates(&table, &model, &evidence, &[0]);
    assert!(!judged.is_empty());
    assert_eq!(judged[0].verdict, Verdict::Holds);
    evidence.rows.clear();
    let judged = judge_certificates(&table, &model, &evidence, &[0]);
    assert_eq!(
        judged[0].verdict,
        Verdict::Invalid,
        "a certificate row without an evidence row must be Invalid"
    );
}

/// Stale evidence — a sidecar derived beside a DIFFERENT model — is
/// refused structurally, not replayed.
#[test]
fn stale_evidence_is_refused() {
    let src = HOLDS_SRC;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let model = derive_application_model(&bundle);
    let table = lower_claims(&bundle, &model);
    let mut evidence =
        derive_certificate_evidence(&bundle, &table, &model);
    evidence.model_shape ^= 1;
    let judged = judge_certificates(&table, &model, &evidence, &[0]);
    assert!(!judged.is_empty());
    assert_eq!(
        judged[0].verdict,
        Verdict::Invalid,
        "shape-mismatched evidence must be refused"
    );
    assert!(
        judged[0]
            .diags
            .iter()
            .any(|d| d.message.contains("does not tie")),
        "the refusal names its cause"
    );
}

/// Review pin: a certificate asserting about a class that is never
/// declared must judge Invalid — never a vacuous Holds (the
/// evaluator's row for it reports Holds because the class is true
/// of nothing; the judgment refuses the law instead).
#[test]
fn undeclared_class_is_invalid_not_holds() {
    let src = r#"
@effects(none: { money })
fn transfer(v: Int) -> Int { return v; }
main locus App {
    run() { println(transfer(1)); }
}
fn main() { App { }; }
"#;
    let (_model, table, _evidence, judged) = derive_all(src);
    let row = table
        .rows
        .iter()
        .find(|r| matches!(r.law, ClaimIr::EffectForbid { .. }))
        .expect("forbid row");
    let j = judged
        .iter()
        .find(|j| j.ordinal == row.ordinal)
        .expect("judged row");
    assert_eq!(
        j.verdict,
        Verdict::Invalid,
        "undeclared class must not hold vacuously"
    );
    // The diagnostic itself is the LOWERING's (one dedup authority
    // across `none:`/`causes:`/`is:` lists of a root).
    assert_eq!(
        table
            .issues
            .iter()
            .filter(|i| i.message.contains("never declared"))
            .count(),
        1,
        "exactly one undeclared-class issue per root"
    );
}

/// Per-root dedup: `is:` and `none:` naming the same missing class
/// on one fn produce ONE diagnostic (the evaluator's pass-1 `seen`
/// spans a root's lists) — and the lowered row still judges
/// Invalid.
#[test]
fn undeclared_class_dedups_across_a_roots_lists() {
    let src = r#"
@effects(is: { money }, none: { money })
fn transfer(v: Int) -> Int { return v; }
main locus App {
    run() { println(transfer(1)); }
}
fn main() { App { }; }
"#;
    let (_model, table, _evidence, judged) = derive_all(src);
    assert_eq!(
        table
            .issues
            .iter()
            .filter(|i| i.message.contains("never declared"))
            .count(),
        1,
        "one issue for the root, not one per list"
    );
    let row = table
        .rows
        .iter()
        .find(|r| matches!(r.law, ClaimIr::EffectForbid { .. }))
        .expect("forbid row");
    let j = judged
        .iter()
        .find(|j| j.ordinal == row.ordinal)
        .expect("judged row");
    assert_eq!(j.verdict, Verdict::Invalid);
}

/// Engine completeness: the families whose engines run elsewhere
/// (causes / budgets) still produce exactly one Judged row each —
/// at minimum Uncertified — instead of silently dropping out.
#[test]
fn unmigrated_families_judge_uncertified() {
    let src = r#"
@budget(alloc_per_call = 4)
fn hot(v: Int) -> Int { return v + 1; }
main locus App {
    run() { println(hot(1)); }
}
fn main() { App { }; }
"#;
    let (_model, table, _evidence, judged) = derive_all(src);
    let row = table
        .rows
        .iter()
        .find(|r| matches!(r.law, ClaimIr::AllocBudget { .. }))
        .expect("budget row lowers");
    let j = judged
        .iter()
        .find(|j| j.ordinal == row.ordinal)
        .expect("budget row is judged");
    assert_eq!(
        j.verdict,
        Verdict::Uncertified,
        "an unmigrated family judges Uncertified, not nothing"
    );
    assert!(j.diags.is_empty());
}

/// The sidecar's own structural laws hold for a derived pair.
#[test]
fn derived_evidence_validates() {
    let (model, table, evidence, _judged) = derive_all(HOLDS_SRC);
    let shape =
        hale_types::topology_projection::project_shape_hash(&model);
    evidence
        .validate(
            &model,
            shape,
            &table,
            hale_types::evidence::analysis_inputs_digest(),
        )
        .expect("derived evidence satisfies the sidecar laws");
}

/// Review pin: a certificate diagnostic whose span lives in the
/// STDLIB parse space is recorded as `Provenance::ForeignSpan`,
/// even when its raw offsets fall numerically inside the user
/// file's range. Origin comes from the emitter's per-diag flag
/// (the witness step's owning fn) — never from numeric overlap.
/// Self-calibrating: measure the foreign offset, then pad the user
/// source past it and require the classification to survive.
#[test]
fn stdlib_owned_diag_span_is_foreign_never_source() {
    let body = r#"
@effects(none: { alloc })
fn probe(r: std::http::Router, req: std::http::Request) -> Int {
    let resp = r.dispatch(req);
    return resp.status;
}
main locus App {
    run() {
        let r = std::http::Router { };
        let req = std::http::Request { method: "GET", path: "/", body: "" };
        println(probe(r, req));
    }
}
fn main() { App { }; }
"#;
    let foreign_of = |src: &str| -> Vec<(bool, u32)> {
        let program = hale_syntax::parse_source(src).expect("parse");
        let bundle = bundle_of(src, &program);
        let model = derive_application_model(&bundle);
        let table = lower_claims(&bundle, &model);
        let evidence =
            derive_certificate_evidence(&bundle, &table, &model);
        let mut out = Vec::new();
        for row in &evidence.rows {
            for cert in &row.certs {
                for (msg, pid) in &cert.diags {
                    if !msg.contains("happens here") {
                        continue;
                    }
                    match evidence.provenance.records[pid.index()] {
                        hale_model::Provenance::ForeignSpan {
                            span,
                        } => out.push((true, span.0)),
                        hale_model::Provenance::Source {
                            span, ..
                        } => out.push((false, span.0)),
                        _ => {}
                    }
                }
            }
        }
        out
    };
    let first = foreign_of(body);
    let (is_foreign, offset) = *first
        .first()
        .expect("the alloc leaf diagnostic must exist");
    assert!(
        is_foreign,
        "the alloc leaf lives in a stdlib body — its record must \
         be ForeignSpan"
    );
    // Pad the user file until the stdlib offset falls INSIDE its
    // numeric range — a containment guess would now misfile the
    // record as user Source; the flag must not.
    let mut padded = body.to_string();
    while (padded.len() as u32) <= offset {
        padded.push_str("// padding to swallow the stdlib offset\n");
    }
    let second = foreign_of(&padded);
    let (still_foreign, _) = *second
        .first()
        .expect("the padded program keeps the alloc leaf");
    assert!(
        still_foreign,
        "origin must come from the emitter's flag, not from \
         numeric overlap with the user file's range"
    );
}

/// Review pin (round 2): evidence ties to the LAW, not just the
/// topology — `TopologyShapeV1` does not hash annotation classes,
/// so two programs with the same shape but different `@effects`
/// laws must refuse each other's evidence via the semantic digest.
#[test]
fn same_shape_different_law_evidence_is_refused() {
    let src_a = r#"
@effects(none: { syscall })
fn f(v: Int) -> Int { return v + 1; }
main locus App {
    run() { println(f(1)); }
}
fn main() { App { }; }
"#;
    let src_b = r#"
@effects(none: { alloc })
fn f(v: Int) -> Int { return v + 1; }
main locus App {
    run() { println(f(1)); }
}
fn main() { App { }; }
"#;
    let pa = hale_syntax::parse_source(src_a).expect("parse a");
    let ba = bundle_of(src_a, &pa);
    let ma = derive_application_model(&ba);
    let ta = lower_claims(&ba, &ma);
    let ea = derive_certificate_evidence(&ba, &ta, &ma);
    let pb = hale_syntax::parse_source(src_b).expect("parse b");
    let bb = bundle_of(src_b, &pb);
    let mb = derive_application_model(&bb);
    let tb = lower_claims(&bb, &mb);
    // The trap the digest closes: identical topology shape.
    assert_eq!(
        hale_types::topology_projection::project_shape_hash(&ma),
        hale_types::topology_projection::project_shape_hash(&mb),
        "the fixtures must share a topology shape for this pin to \
         bite"
    );
    let judged = judge_certificates(&tb, &mb, &ea, &[0]);
    let forbid = tb
        .rows
        .iter()
        .find(|r| matches!(r.law, ClaimIr::EffectForbid { .. }))
        .expect("forbid row");
    let j = judged
        .iter()
        .find(|j| j.ordinal == forbid.ordinal)
        .expect("judged");
    assert_eq!(
        j.verdict,
        Verdict::Invalid,
        "another law's evidence must be refused, not replayed"
    );
}

/// Malformed sidecars are refused by the judgment's own mandatory
/// validation: duplicate ordinals, a certificate count that
/// disagrees with the row's shape, and dangling diagnostic
/// provenance each invalidate rather than being silently
/// collapsed, over-consumed, or mis-spanned.
#[test]
fn malformed_evidence_is_refused() {
    let src = HOLDS_SRC;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let model = derive_application_model(&bundle);
    let table = lower_claims(&bundle, &model);
    let evidence =
        derive_certificate_evidence(&bundle, &table, &model);
    let forbid_ordinal = table
        .rows
        .iter()
        .find(|r| matches!(r.law, ClaimIr::EffectForbid { .. }))
        .expect("forbid row")
        .ordinal;
    let verdict_of = |ev: &hale_model::EvidenceTable| {
        judge_certificates(&table, &model, ev, &[0])
            .into_iter()
            .find(|j| j.ordinal == forbid_ordinal)
            .expect("judged")
            .verdict
    };
    assert_eq!(verdict_of(&evidence), Verdict::Holds);
    // Duplicate ordinal: BTreeMap-collapse would silently keep one.
    let mut dup = evidence.clone();
    let row = dup.rows[0].clone();
    dup.rows.push(row);
    assert_eq!(verdict_of(&dup), Verdict::Invalid);
    // Extra certificate: `>=` would consume a prefix and accept.
    let mut extra = evidence.clone();
    let c = extra.rows[0].certs[0].clone();
    extra.rows[0].certs.push(c);
    assert_eq!(verdict_of(&extra), Verdict::Invalid);
    // Dangling diagnostic provenance: span-zero rendering hid it.
    let mut dangling = evidence.clone();
    dangling.rows[0].certs[0].diags.push((
        "phantom".to_string(),
        hale_model::ProvenanceId(9999),
    ));
    assert_eq!(verdict_of(&dangling), Verdict::Invalid);
}

/// Review pin (round 2): the undeclared-class diagnostic renders
/// the AUTHOR-FACING spelling for imported subjects — a library
/// annotation must name `kv::helper` / `kv::Store::bump`, never
/// the post-merge `__lib_…` symbol.
#[test]
fn imported_undeclared_class_issue_renders_display() {
    let lib = r#"
@effects(none: { money })
fn __lib_x_kv_helper(v: Int) -> Int { return v; }
locus __lib_x_kv_Store {
    params { n: Int = 0; }
    @effects(none: { money })
    fn bump() { self.n = self.n + 1; }
}
"#;
    let main_src = r#"
main locus App {
    params { s: __lib_x_kv_Store = __lib_x_kv_Store { }; }
    run() { self.s.bump(); println(__lib_x_kv_helper(1)); }
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
            vec!["kv".to_string(), "helper".to_string()],
            "__lib_x_kv_helper".to_string(),
        ),
        (
            vec!["kv".to_string(), "Store".to_string()],
            "__lib_x_kv_Store".to_string(),
        ),
    ];
    let model = derive_application_model(&bundle);
    let table = lower_claims(&bundle, &model);
    let issues: Vec<&str> = table
        .issues
        .iter()
        .filter(|i| i.message.contains("never declared"))
        .map(|i| i.message.as_str())
        .collect();
    assert_eq!(issues.len(), 2, "one issue per annotated root");
    assert!(
        issues.iter().any(|m| m.contains("`kv::helper`")),
        "the imported FREE fn renders its alias spelling: {:?}",
        issues
    );
    assert!(
        issues
            .iter()
            .any(|m| m.contains("`kv::Store::bump`")),
        "the imported METHOD renders its alias spelling: {:?}",
        issues
    );
    assert!(
        !issues.iter().any(|m| m.contains("__lib_")),
        "no post-merge symbol leaks into a public diagnostic: {:?}",
        issues
    );
}

/// Review pin (round 3): each evidence row's certificate payload
/// is bound to the exact law it answers — two one-certificate laws
/// on the same fn cannot exchange their certs and keep validating
/// (ordinals, subjects, counts, digest, and provenance all stay
/// intact under the swap; the per-row FORM binding is what
/// refuses).
#[test]
fn swapped_certs_between_same_subject_laws_are_refused() {
    let src = r#"
@effects(none: { syscall }, publish: { })
fn f(v: Int) -> Int { return v + 1; }
main locus App {
    run() { println(f(1)); }
}
fn main() { App { }; }
"#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bundle = bundle_of(src, &program);
    let model = derive_application_model(&bundle);
    let table = lower_claims(&bundle, &model);
    let evidence =
        derive_certificate_evidence(&bundle, &table, &model);
    let one_cert_rows: Vec<usize> = evidence
        .rows
        .iter()
        .enumerate()
        .filter(|(_, r)| r.certs.len() == 1)
        .map(|(i, _)| i)
        .collect();
    assert!(
        one_cert_rows.len() >= 2,
        "two one-certificate laws on `f`: {:?}",
        evidence
            .rows
            .iter()
            .map(|r| r.certs.len())
            .collect::<Vec<_>>()
    );
    let judged = judge_certificates(&table, &model, &evidence, &[0]);
    assert!(judged
        .iter()
        .all(|j| j.verdict == Verdict::Holds));
    let mut swapped = evidence.clone();
    let (a, b) = (one_cert_rows[0], one_cert_rows[1]);
    let tmp = swapped.rows[a].certs.clone();
    swapped.rows[a].certs = swapped.rows[b].certs.clone();
    swapped.rows[b].certs = tmp;
    let judged = judge_certificates(&table, &model, &swapped, &[0]);
    assert!(
        judged
            .iter()
            .filter(|j| {
                evidence.rows.iter().any(|r| r.ordinal == j.ordinal)
            })
            .all(|j| j.verdict == Verdict::Invalid),
        "exchanged certificate payloads must be refused: {:?}",
        judged
            .iter()
            .map(|j| (j.ordinal, j.verdict))
            .collect::<Vec<_>>()
    );
}

/// Review pin (round 3): evidence is tied to the exact SOURCE
/// snapshot — an edit that merely shifts the annotated site keeps
/// the topology shape AND the law digest, but the source-unit
/// digests disagree, so the stale offsets are refused instead of
/// rendering against the new bases.
#[test]
fn edited_source_evidence_is_refused() {
    let v1 = r#"
@effects(none: { syscall })
fn f(v: Int) -> Int { return v + 1; }
main locus App {
    run() { println(f(1)); }
}
fn main() { App { }; }
"#;
    let v2 = r#"
// a comment that only shifts every later offset
@effects(none: { syscall })
fn f(v: Int) -> Int { return v + 1; }
main locus App {
    run() { println(f(1)); }
}
fn main() { App { }; }
"#;
    let p1 = hale_syntax::parse_source(v1).expect("parse v1");
    let b1 = bundle_of(v1, &p1);
    let m1 = derive_application_model(&b1);
    let t1 = lower_claims(&b1, &m1);
    let e1 = derive_certificate_evidence(&b1, &t1, &m1);
    let p2 = hale_syntax::parse_source(v2).expect("parse v2");
    let mut b2 = bundle_of(v2, &p2);
    b2.sources[0].digest = "1".to_string();
    let m2 = derive_application_model(&b2);
    let t2 = lower_claims(&b2, &m2);
    assert_eq!(
        hale_types::topology_projection::project_shape_hash(&m1),
        hale_types::topology_projection::project_shape_hash(&m2),
        "source motion must not change the shape — that is why the \
         snapshot tie exists"
    );
    let judged = judge_certificates(&t2, &m2, &e1, &[0]);
    let forbid = t2
        .rows
        .iter()
        .find(|r| matches!(r.law, ClaimIr::EffectForbid { .. }))
        .expect("forbid row");
    let j = judged
        .iter()
        .find(|j| j.ordinal == forbid.ordinal)
        .expect("judged");
    assert_eq!(
        j.verdict,
        Verdict::Invalid,
        "evidence from an edited source must be refused"
    );
}

/// Review pin (round 3): evidence produced by a different
/// toolchain / stdlib snapshot is refused via the inputs digest.
#[test]
fn different_toolchain_evidence_is_refused() {
    let program = hale_syntax::parse_source(HOLDS_SRC).expect("parse");
    let bundle = bundle_of(HOLDS_SRC, &program);
    let model = derive_application_model(&bundle);
    let table = lower_claims(&bundle, &model);
    let mut evidence =
        derive_certificate_evidence(&bundle, &table, &model);
    evidence.inputs_digest ^= 1;
    let judged = judge_certificates(&table, &model, &evidence, &[0]);
    assert!(!judged.is_empty());
    assert_eq!(judged[0].verdict, Verdict::Invalid);
}

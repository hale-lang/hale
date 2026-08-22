//! GH #476 Change 9 (review round 1) — when the model-backed
//! judgment may run at all.
//!
//! A model is a description of a CHECKED program. Some
//! parser-valid, checker-invalid programs deliberately derive
//! UNLAWFUL models — a `where key ==` filter on an unkeyed topic
//! derives a `KeyContract` violation, mirroring the checker's own
//! refusal — and that was harmless only while nothing on the
//! ordinary check path consumed them. Once `hale check` judges
//! claims over the model, an ill-typed program with any claim
//! surface would hand an invalid model to the evidence and judgment
//! code: a debug-build panic at the builder's own assertion, and in
//! release a walk over relations whose indexing assumes lawfulness.

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

/// A keyed filter on an unkeyed topic: the checker refuses it, and
/// the model it derives is unlawful in exactly the mirroring way.
const UNLAWFUL_WITH_CLAIMS: &str = r#"
type Reading { sensor: Int = 0; }
topic Plain { payload: Reading; subject: "p"; }
locus Sub {
    params { seen: Int = 0; }
    bus { subscribe Plain as on_r where key == 3; }
    fn on_r(r: Reading) { self.seen = self.seen + 1; }
}
group subs = { Sub };
main locus App {
    params { s: Sub = Sub { }; }
    bus { publish Plain; }
    claims { wired: require subscribes(some subs, topic Plain); }
    run() { Plain <- Reading { sensor: 1 }; }
}
fn main() { App { }; }
"#;

#[test]
fn an_ill_typed_program_with_claims_is_never_judged() {
    let program =
        hale_syntax::parse_source(UNLAWFUL_WITH_CLAIMS).expect("parse");
    let bundle = bundle_of(UNLAWFUL_WITH_CLAIMS, &program);

    // Premise 1: the program really is refused by the checker.
    let diags = hale_types::check_bundle_opts(&bundle, false);
    assert!(
        diags.iter().any(|d| d.is_error()
            && d.kind != hale_syntax::error::DiagKind::Claim),
        "fixture premise: a non-claim error must be reported: {:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );

    // Premise 2: and the model it derives really is unlawful — this
    // is what makes judging it dangerous rather than merely wrong.
    let model = derive_application_model(&bundle);
    assert!(
        model.validate().is_err(),
        "fixture premise: this program's model must be unlawful, or \
         the test proves nothing about the gate"
    );

    // The obligation: ordinary diagnostics, no judgment.
    assert!(
        !diags
            .iter()
            .any(|d| d.kind == hale_syntax::error::DiagKind::Claim
                && d.message.contains("claim `wired`")),
        "the claim was judged over an invalid model: {:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

/// …and the gate is not a blanket refusal to judge: fix the type
/// error and the same claim is judged. (Without this, a gate that
/// simply never judged anything would pass the test above.)
#[test]
fn the_same_program_is_judged_once_it_typechecks() {
    let src = UNLAWFUL_WITH_CLAIMS
        .replace("topic Plain { payload: Reading; subject: \"p\"; }",
                 "topic Plain { payload: Reading; subject: \"p\"; keyed_by sensor; }");
    let program = hale_syntax::parse_source(&src).expect("parse");
    let bundle = bundle_of(&src, &program);
    let diags = hale_types::check_bundle_opts(&bundle, false);
    assert!(
        !diags.iter().any(|d| d.is_error()
            && d.kind != hale_syntax::error::DiagKind::Claim),
        "fixture premise: the fixed program typechecks: {:?}",
        diags
            .iter()
            .filter(|d| d.is_error())
            .map(|d| &d.message)
            .collect::<Vec<_>>()
    );
    assert!(
        derive_application_model(&bundle).validate().is_ok(),
        "fixture premise: the fixed program's model is lawful"
    );
    // The claim holds here, so the proof that it was JUDGED is that
    // no claim error appears while the machinery ran — check the
    // judgment directly instead.
    let judged = hale_types::judgment::claim_law_diags(&bundle);
    assert!(
        judged.is_empty(),
        "a satisfied claim should judge clean: {:?}",
        judged.iter().map(|d| &d.message).collect::<Vec<_>>()
    );

    // And a VIOLATED claim on a typechecking program is reported —
    // the gate lets real law failures through.
    let violating = src.replace(
        "wired: require subscribes(some subs, topic Plain);",
        "wired: require publishes(some subs, topic Plain);",
    );
    let program2 = hale_syntax::parse_source(&violating).expect("parse");
    let bundle2 = bundle_of(&violating, &program2);
    let diags2 = hale_types::check_bundle_opts(&bundle2, false);
    assert!(
        diags2
            .iter()
            .any(|d| d.kind == hale_syntax::error::DiagKind::Claim
                && d.message.contains("claim `wired`")),
        "a violated claim on a checked program must be reported: {:?}",
        diags2.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}

/// Review round 1, blocker 3: a claim diagnostic must land ON the
/// claim, even when the caller never installed a source map.
///
/// `Bundle::new` leaves `sources` empty, and the public
/// `check_program` API uses exactly that constructor. Claim lowering
/// used to collapse any span it could not place into a synthetic
/// record, and every consumer renders those at 0..0 — so every
/// migrated claim diagnostic anchored at byte zero of the first
/// file. Under the old source evaluator the AST spans were used
/// directly and this could not happen.
#[test]
fn claim_spans_survive_a_bundle_with_no_source_map() {
    const SRC: &str = r#"
locus B { params { n: Int = 0; } fn stop() { self.n = self.n + 1; } }
locus A {
    params { b: B = B { }; }
    fn go() { self.b.stop(); }
}
group src = { A };
group dst = { B };
main locus App {
    params { a: A = A { }; }
    claims {
        isolation: forbid reaches(src, dst);
    }
    run() { self.a.go(); }
}
fn main() { App { }; }
"#;
    let program = hale_syntax::parse_source(SRC).expect("parse");
    // The PUBLIC api — `Bundle::new`, no source map installed.
    let diags = hale_types::check_program(&program);
    let claim = diags
        .iter()
        .find(|d| d.message.contains("claim `isolation`"))
        .unwrap_or_else(|| {
            panic!(
                "fixture premise: the claim must produce a \
                 diagnostic: {:?}",
                diags.iter().map(|d| &d.message).collect::<Vec<_>>()
            )
        });
    assert_ne!(
        (claim.span.start.as_usize(), claim.span.end.as_usize()),
        (0, 0),
        "the claim diagnostic collapsed to byte zero: {}",
        claim.message
    );
    // …and it is not merely nonzero, it is the CLAIM's own span.
    let at = SRC.find("isolation:").expect("claim in fixture");
    assert_eq!(
        claim.span.start.as_usize(),
        at,
        "expected the claim-name span ({}..), got {}..{}: {}",
        at,
        claim.span.start.as_usize(),
        claim.span.end.as_usize(),
        claim.message
    );
}

/// Review round 3 — the whole operand surface, not one operand.
///
/// A refused group has now slipped through in three positions
/// across three rounds: the source domain, then `avoiding`, with the
/// partial/duplicate shapes in between. The pattern is that every
/// GROUP OPERAND of every family is a domain, and a guard that
/// covers the operands someone happened to think of is the same
/// defect waiting. This enumerates every group-bearing position in
/// the lowered law shapes and requires each to refuse the row.
///
/// If a new family or a new group operand is added, the honest way
/// for this test to fail is for it to be missing a case — so it
/// asserts its own coverage count too.
#[test]
fn every_group_operand_position_refuses_a_refused_group() {
    // (label, program) — each puts an unresolvable member in ONE
    // operand position and nowhere else.
    let cases: Vec<(&str, String)> = vec![
        (
            "reaches:src",
            r#"{PRELUDE}
group bad = { Worker, MissingOne };
group other = { Sink };
main locus App {
    params { w: Worker = Worker { }; s: Sink = Sink { }; }
    claims { law_reach_src: forbid reaches(bad, other); }
    run() { self.w.work(); self.s.take(); }
}
"#
            .to_string(),
        ),
        (
            "reaches:dst",
            r#"{PRELUDE}
group other = { Sink };
group bad = { Worker, MissingOne };
main locus App {
    params { w: Worker = Worker { }; s: Sink = Sink { }; }
    claims { law_reach_dst: forbid reaches(other, bad); }
    run() { self.w.work(); self.s.take(); }
}
"#
            .to_string(),
        ),
        (
            "reaches:avoiding",
            r#"{PRELUDE}
group src_g = { Worker };
group dst_g = { Sink };
group bad = { Worker, MissingOne };
main locus App {
    params { w: Worker = Worker { }; s: Sink = Sink { }; }
    claims { law_reach_avoid: forbid reaches(src_g, dst_g) avoiding bad; }
    run() { self.w.work(); self.s.take(); }
}
"#
            .to_string(),
        ),
        (
            "only_edges:src",
            r#"{PRELUDE}
group bad = { Worker, MissingOne };
group other = { Sink };
main locus App {
    params { w: Worker = Worker { }; s: Sink = Sink { }; }
    claims { law_edges_src: only edges bad -> other { }; }
    run() { self.w.work(); self.s.take(); }
}
"#
            .to_string(),
        ),
        (
            "only_edges:dst",
            r#"{PRELUDE}
group other = { Sink };
group bad = { Worker, MissingOne };
main locus App {
    params { w: Worker = Worker { }; s: Sink = Sink { }; }
    claims { law_edges_dst: only edges other -> bad { }; }
    run() { self.w.work(); self.s.take(); }
}
"#
            .to_string(),
        ),
        (
            "bound:from",
            r#"{PRELUDE}
effect money;
group bad = { Worker, MissingOne };
main locus App {
    params { w: Worker = Worker { }; s: Sink = Sink { }; }
    claims { law_bound: bound money <= 0 on paths from bad; }
    run() { self.w.work(); self.s.take(); }
}
"#
            .to_string(),
        ),
        (
            "require_sealed:group",
            r#"{PRELUDE}
group bad = { Worker, MissingOne };
main locus App {
    params { w: Worker = Worker { }; s: Sink = Sink { }; }
    claims { law_sealed: require sealed(all bad); }
    run() { self.w.work(); self.s.take(); }
}
"#
            .to_string(),
        ),
        (
            "require_endpoint:group",
            r#"{PRELUDE}
type M { n: Int = 0; }
topic T { payload: M; subject: "t"; }
group bad = { Worker, MissingOne };
main locus App {
    params { w: Worker = Worker { }; s: Sink = Sink { }; }
    bus { publish T; }
    claims { law_endpoint: require subscribes(some bad, topic T); }
    run() { self.w.work(); self.s.take(); T <- M { n: 1 }; }
}
"#
            .to_string(),
        ),
    ];
    const PRELUDE: &str = r#"
locus Worker { params { n: Int = 0; } fn work() { self.n = self.n + 1; } }
locus Sink { params { n: Int = 0; } fn take() { self.n = self.n + 1; } }
"#;

    let mut checked = 0usize;
    for (label, body) in &cases {
        let src = format!(
            "{}\nfn main() {{ App {{ }}; }}\n",
            body.replace("{PRELUDE}", PRELUDE)
        );
        let program = hale_syntax::parse_source(&src)
            .unwrap_or_else(|e| panic!("{}: parse: {:?}", label, e));
        let bundle = bundle_of(&src, &program);
        let model = derive_application_model(&bundle);
        model
            .validate()
            .unwrap_or_else(|e| panic!("{}: unlawful model: {:?}", label, e));
        let table = hale_types::claim_lowering::lower_claims(&bundle, &model);
        // Premise: exactly the intended group is refused, and the
        // law really was lowered (an unlowered law would vacuously
        // "pass" this test).
        assert_eq!(
            table.group_selection.get("bad"),
            Some(&hale_model::GroupSelection::SelectorFailed),
            "{}: fixture premise: `bad` must be selector-failed, got {:?}",
            label,
            table.group_selection
        );
        let row = table
            .rows
            .iter()
            .find(|r| r.name.starts_with("law_"))
            .unwrap_or_else(|| panic!("{}: the law did not lower", label));

        let evidence = hale_types::evidence::derive_certificate_evidence(
            &bundle, &table, &model,
        );
        let bases: Vec<u32> =
            bundle.sources.iter().map(|f| f.base).collect();
        let (_pre, judged) =
            hale_types::topology_projection::judge_all(
                &table, &model, &evidence, &bases,
            );
        let verdict = judged
            .get(&row.ordinal)
            .map(|j| j.verdict)
            .unwrap_or_else(|| panic!("{}: the law was not judged", label));
        assert_eq!(
            verdict,
            hale_types::verdict::Verdict::Invalid,
            "{}: a law over a refused group was judged anyway",
            label
        );
        checked += 1;
    }
    assert_eq!(
        checked,
        8,
        "every group operand position must be covered; if a family \
         or operand was added, add its case here"
    );
}

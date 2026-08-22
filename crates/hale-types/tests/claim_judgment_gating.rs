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

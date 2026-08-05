//! The topology artifact's integrity digest (schema 1.3+).
//!
//! `shape_hash` is an IDENTITY, not an integrity check. It covers
//! the model half only — deliberately, so that moving a comment or
//! renaming a claim doesn't churn the model's identity — which
//! leaves `topics`, `provenance` and the claim results outside it.
//!
//! That is fine while the only consumer is the compiler that just
//! produced the document. It stops being fine the moment anything
//! trusts an artifact it did not produce:
//!
//!  - a cross-binary consumer joins endpoints on the `topics` rows
//!    (wire subject + payload hash). Verifying `shape_hash` and then
//!    joining on rows outside it means the join key was never
//!    checked.
//!  - a baseline gate that greps the `shape_hash` line out of a file
//!    can be defeated by editing that one line, since nothing forces
//!    the rest of the document to agree with it.
//!
//! `artifact_digest` covers the whole body and is the final key, so
//! everything preceding it is exactly what was hashed.

use std::collections::BTreeMap;

use hale_types::topology::{dump_topology, verify_artifact_digest};
use hale_types::Bundle;

const SRC: &str = r#"
    type T { v: Int; }
    topic Tk { payload: T; subject: "app.t"; }
    locus Sub {
        params { got: Int = 0; }
        bus { subscribe Tk as on_t; }
        fn on_t(t: T) { self.got = t.v; }
    }
    group subs = { Sub };
    main locus App {
        params { s: Sub = Sub { }; }
        claims { wired: require subscribes(some subs, topic Tk); }
    }
    fn main() { App { }; }
"#;

fn dump(src: &str) -> String {
    let program = hale_syntax::parse_source(src).expect("parse");
    let mut programs = BTreeMap::new();
    programs.insert("app.hl".to_string(), &program);
    dump_topology(&Bundle::new(programs))
}

fn artifact() -> String {
    dump(SRC)
}

#[test]
fn a_freshly_emitted_artifact_verifies() {
    let a = artifact();
    assert_eq!(
        verify_artifact_digest(&a),
        Some(true),
        "the emitter and verifier must agree:\n{}",
        a
    );
}

/// The digest must cover the sections `shape_hash` omits. This is
/// the property cross-binary composition depends on: `topics` is the
/// join key, and it is not part of the model identity.
#[test]
fn tampering_with_the_unhashed_topics_section_is_caught() {
    let a = artifact();
    assert!(
        a.contains("app.t"),
        "fixture must carry a wire subject to tamper with:\n{}",
        a
    );
    let tampered = a.replacen("app.t", "app.EVIL", 1);
    assert_eq!(
        verify_artifact_digest(&tampered),
        Some(false),
        "a rewritten wire subject must fail the digest — it is the \
         key a fleet consumer would join endpoints on"
    );
}

/// The other hole: forging the identity line itself.
#[test]
fn forging_the_shape_hash_line_is_caught() {
    let a = artifact();
    let start = a.find("\"shape_hash\": \"").expect("shape_hash present")
        + "\"shape_hash\": \"".len();
    let forged =
        format!("{}dead0000beef0000{}", &a[..start], &a[start + 16..]);
    assert_ne!(forged, a, "the forgery must actually change the text");
    assert_eq!(
        verify_artifact_digest(&forged),
        Some(false),
        "a gate that greps one line out of a baseline is only as \
         trustworthy as the digest over the whole body"
    );
}

/// Claim results and provenance are outside `shape_hash` too, and
/// a consumer that trusts an artifact cares whether they were edited.
#[test]
fn tampering_with_a_claim_result_is_caught() {
    let a = artifact();
    assert!(a.contains("\"holds\""), "fixture must have a holding claim");
    let tampered = a.replacen("\"holds\"", "\"violated\"", 1);
    assert_eq!(verify_artifact_digest(&tampered), Some(false));
}

/// Absent is reported distinctly from invalid. A consumer may accept
/// a pre-1.3 artifact, but must never read "nothing to check" as
/// "checked and intact".
#[test]
fn an_artifact_without_a_digest_is_unverifiable_not_valid() {
    let a = artifact();
    let at = a
        .rfind(hale_types::topology::ARTIFACT_DIGEST_KEY)
        .expect("digest present");
    let older = format!("{}\n}}\n", &a[..at]);
    assert_eq!(
        verify_artifact_digest(&older),
        None,
        "no digest must be None, never Some(true)"
    );
}

/// The digest changes with the model, so it cannot be a constant that
/// happens to match.
#[test]
fn a_different_program_produces_a_different_digest() {
    let a = artifact();
    let other_src = SRC.replace("locus Sub {", "locus Extra { }\n    locus Sub {");
    let b = dump(&other_src);
    assert_eq!(verify_artifact_digest(&b), Some(true));
    assert_ne!(
        a, b,
        "adding a locus must change the artifact"
    );
}

// ---- schema 1.4: one verdict vocabulary --------------------------
//
// Claims and lowered certificates are the same kind of statement at
// different granularity, so they must not disagree about how to
// spell an outcome. Before 1.4 claim rows carried three states while
// lowered rows carried a bool, and `violated` did double duty for
// "disproved" and "could not be proved".

use hale_types::verdict::Verdict;

#[test]
fn only_holds_passes() {
    assert!(Verdict::Holds.passed());
    for v in [Verdict::Violated, Verdict::Uncertified, Verdict::Invalid] {
        assert!(
            !v.passed(),
            "{:?} must not pass — a law that could not be checked has \
             not been satisfied, and treating that as success is the \
             fail-open this vocabulary exists to prevent",
            v
        );
    }
}

#[test]
fn the_document_verdict_reflects_its_rows() {
    let a = artifact();
    let v: serde_json::Value = serde_json::from_str(&a).expect("valid JSON");
    assert_eq!(v["schema"], "1.7");
    assert_eq!(
        v["verdict"], "clean",
        "every claim in the fixture holds: {}",
        a
    );

    // A claim naming a topic nothing subscribes fails, and the
    // document says so without the reader scanning rows.
    let failing = SRC.replace(
        "bus { subscribe Tk as on_t; }",
        "",
    ).replace("fn on_t(t: T) { self.got = t.v; }", "");
    let b = dump(&failing);
    let w: serde_json::Value = serde_json::from_str(&b).expect("valid JSON");
    assert_ne!(
        w["verdict"], "clean",
        "a claim that no longer holds must move the document verdict: {}",
        b
    );
}

/// An unresolvable edge is `uncertified`, not `violated`. Both fail
/// the build — unknown still means fail-closed — but the artifact
/// must record which happened, because the repairs differ and
/// because cross-binary composition needs a propagated unknown to
/// read as "not provable" rather than "disproved".
#[test]
fn an_unresolvable_edge_is_uncertified_not_violated() {
    const INDIRECT: &str = r#"
        locus Sink { params { n: Int = 0; } fn take(x: Int) { self.n = x; } }
        locus Src {
            params { n: Int = 0; }
            fn go(f: fn(Int)) { f(self.n); }
        }
        group srcs = { Src };
        group sinks = { Sink };
        main locus App {
            params { s: Src = Src { }; k: Sink = Sink { }; }
            claims { iso: forbid reaches(srcs, sinks); }
        }
        fn main() { App { }; }
    "#;
    let a = dump(INDIRECT);
    let v: serde_json::Value = serde_json::from_str(&a).expect("valid JSON");
    let claims = v["claims"].as_array().expect("claims array");
    let iso = claims
        .iter()
        .find(|c| c["name"] == "iso")
        .expect("the claim is recorded");
    assert_eq!(
        iso["result"], "uncertified",
        "a call through a fn-typed param is not knowable statically — \
         nothing disproved the claim, so it must not read as \
         `violated`: {}",
        a
    );
    assert_ne!(v["verdict"], "clean", "and it still does not pass");
}

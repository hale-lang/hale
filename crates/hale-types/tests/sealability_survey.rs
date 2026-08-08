//! GH #436: the sealability survey — `hale check --sealable`.
//!
//! `@sealed` is opt-in, so "would this collide with real code?" is a
//! question about an existing codebase that nobody can answer by
//! reading. It is mechanically computable, and leaving it to
//! inspection was the part of "measure before building more" that was
//! wrongly called not-code.
//!
//! Measured over the in-tree corpus when this landed: **148 of 151
//! loci across 94 programs could be sealed with no changes.** The
//! three that could not are the same shape — a parent reading a
//! child's result field directly instead of calling a method, which
//! the no-locus-return rule already discourages.

use hale_syntax::parse_source;
use hale_types::sealability::{render, survey};

const SRC: &str = r#"
    locus Private { params { k: Int = 1; }
        fn use1() -> Int { return self.k; } }
    locus Exposed { params { k: Int = 2; }
        fn use2() -> Int { return self.k; } }
    @sealed locus Already { params { k: Int = 3; }
        fn use3() -> Int { return self.k; } }

    locus Holder {
        params { p: Private = Private { }; e: Exposed = Exposed { }; }
        fn peek() -> Int { return self.e.k; }
        fn proper() -> Int { return self.p.use1(); }
    }

    main locus App {
        params { h: Holder = Holder { }; a: Already = Already { }; }
    }
    fn main() { App { }; }
"#;

fn rows() -> Vec<hale_types::sealability::Sealable> {
    let p = parse_source(SRC).expect("parse");
    survey(&[&p])
}

#[test]
fn a_locus_reached_only_through_methods_is_free_to_seal() {
    let rs = rows();
    let private = rs.iter().find(|r| r.locus == "Private").expect("Private");
    assert!(
        private.blockers.is_empty(),
        "Private is only used via `use1()`, so sealing is a no-op: {:?}",
        private.blockers
    );
}

#[test]
fn a_locus_with_an_external_param_read_is_blocked_and_the_site_is_named() {
    let rs = rows();
    let exposed = rs.iter().find(|r| r.locus == "Exposed").expect("Exposed");
    assert_eq!(
        exposed.blockers.len(),
        1,
        "expected exactly the one read, got {:?}",
        exposed.blockers
    );
    assert!(
        exposed.blockers[0].contains("Exposed.k"),
        "the blocker must name the field to fix: {:?}",
        exposed.blockers
    );
}

#[test]
fn an_already_sealed_locus_reports_as_free_rather_than_being_omitted() {
    // Omitting it would read as "unexamined" in the report.
    let rs = rows();
    let already = rs.iter().find(|r| r.locus == "Already").expect("Already");
    assert!(already.blockers.is_empty(), "{:?}", already.blockers);
}

#[test]
fn every_locus_appears_exactly_once() {
    let rs = rows();
    let mut names: Vec<&str> = rs.iter().map(|r| r.locus.as_str()).collect();
    names.sort();
    assert_eq!(
        names,
        vec!["Already", "App", "Exposed", "Holder", "Private"],
        "the survey must cover the whole bundle, without duplicates"
    );
}

#[test]
fn the_survey_agrees_with_the_checker() {
    // The survey runs the real check against an all-sealed clone
    // rather than reimplementing the rule. This pins that
    // equivalence: a locus the survey calls blocked must actually
    // produce a sealed diagnostic when sealed for real.
    let sealed_for_real = SRC.replace("locus Exposed", "@sealed locus Exposed");
    let p = parse_source(&sealed_for_real).expect("parse");
    let es: Vec<String> = hale_types::check_program(&p)
        .into_iter()
        .filter(|d| d.is_error())
        .map(|d| d.message)
        .collect();
    assert!(
        es.iter().any(|m| m.contains("`@sealed`") && m.contains("Exposed")),
        "the survey said Exposed is blocked; sealing it must error: {es:?}"
    );
}

#[test]
fn the_rendering_leads_with_the_count() {
    // The number is the decision-relevant fact — "can we adopt this?"
    // — so it goes first rather than after a list.
    let out = render(&rows());
    assert!(
        out.starts_with("sealability: 4 of 5 loci"),
        "unexpected header: {out}"
    );
    assert!(out.contains("would break callers"), "{out}");
    assert!(
        out.contains("external access(es)"),
        "the survey covers writes as well as reads now that the \
         sealed rule does — reruns of the real checker track it: {out}"
    );
}

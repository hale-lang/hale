//! A literal subject at a delivery site names only the topic that owns
//! that wire subject (spec/model.md rule 8): a literal that spells a
//! child topic's declared segment, or a topic's name, while the wire
//! differs is delivered on its own bytes and names no topic. The
//! fallback catch-all rule and a send's policy read the wire, not a
//! convenience resolution (outside review of #1282, finding 1).

fn errors(src: &str) -> Vec<String> {
    let program = hale_syntax::parse_source(src).expect("parses");
    hale_types::check_program(&program)
        .into_iter()
        .filter(|d| d.is_error())
        .map(|d| d.message)
        .collect()
}

fn fallback_program(subscribed: &str) -> String {
    format!(
        r#"
type Ev {{ id: Int = 0; }}
topic Root {{ payload: Ev; subject: "root"; }}
topic Child : Root {{
    payload: Ev; subject: "leaf";
    keyed_by id; on_unmatched: fallback;
}}
locus Catcher {{
    bus {{ subscribe "{subscribed}" as on_any of type Ev where key == _; }}
    fn on_any(e: Ev) {{ println(e.id); }}
}}
locus Sender {{
    bus {{ publish Child; }}
    run() {{ Child <- Ev {{ id: 1 }}; }}
}}
main locus App {{ params {{ c: Catcher = Catcher {{ }}; s: Sender = Sender {{ }}; }} }}
fn main() {{ App {{ }}; }}
"#
    )
}

/// `Child`'s wire is `root.leaf`; a catch-all on the literal `leaf`
/// listens elsewhere, so the fallback topic has no catch-all.
#[test]
fn a_catch_all_on_the_childs_segment_is_not_on_its_wire() {
    let errs = errors(&fallback_program("leaf"));
    assert!(
        errs.iter().any(|m| m.contains("declares `on_unmatched: fallback` but")),
        "expected the missing catch-all error, got {errs:?}"
    );
}

/// The same catch-all on the wire subject satisfies the rule.
#[test]
fn a_catch_all_on_the_wire_subject_is_the_topics() {
    let errs = errors(&fallback_program("root.leaf"));
    assert!(
        !errs.iter().any(|m| m.contains("declares `on_unmatched: fallback` but")),
        "the wire catch-all was not seen: {errs:?}"
    );
}

/// A literal that spells the topic's NAME while its wire differs is
/// delivered on the bytes `Child`, which no topic owns, so it carries
/// none of the topic's policies: a fail-policy topic requires an `or`
/// clause on `Child <- …` and requires nothing of `"Child" <- …`.
#[test]
fn a_literal_that_spells_a_topics_name_carries_no_policy() {
    let with = |send: &str| {
        format!(
            r#"
type Ev {{ id: Int = 0; }}
topic Root {{ payload: Ev; subject: "root"; }}
topic Child : Root {{ payload: Ev; subject: "leaf"; keyed_by id; on_unmatched: fail; }}
locus Sender {{
    bus {{ publish Child; }}
    run() {{ {send} }}
}}
main locus App {{ params {{ s: Sender = Sender {{ }}; }} }}
fn main() {{ App {{ }}; }}
"#
        )
    };
    let by_reference = errors(&with("Child <- Ev { id: 1 };"));
    assert!(
        by_reference.iter().any(|m| m.contains("on_unmatched: fail")),
        "a fail-policy topic reference needs its clause: {by_reference:?}"
    );
    let by_literal = errors(&with("\"Child\" <- Ev { id: 1 };"));
    assert!(
        !by_literal.iter().any(|m| m.contains("on_unmatched: fail")),
        "a literal spelling the name carries no policy: {by_literal:?}"
    );
}

//! GH #525 item 1 (2026-09-04): a locus accepts exactly one child
//! type (`spec/types.md`, single-accept-type per parent). A second
//! `accept` clause used to overwrite the first with no diagnostic,
//! so a parent written to own two child types silently owned one.
//! It is now an error naming both clauses; the first stays the
//! locus's accept type.

use hale_syntax::parse_source;
use hale_types::check_program;

const TWO_ACCEPTS: &str = r#"
locus Work { params { id: Int = 0; } run() { } }
locus Task { params { id: Int = 0; } run() { } }

locus Step {
    params { n: Int = 0; }
    accept(w: Work) { }
    accept(t: Task) { }
    run() {
        Work { id: 1 };
    }
}

main locus App {
    params { s: Step = Step { }; }
    run() { }
}
fn main() { App { }; }
"#;

#[test]
fn a_second_accept_clause_is_an_error_naming_both() {
    let prog = parse_source(TWO_ACCEPTS).expect("parse");
    let diags = check_program(&prog);
    let hit = diags
        .iter()
        .find(|d| d.message.contains("declares `accept` twice"))
        .unwrap_or_else(|| {
            panic!(
                "expected a duplicate-accept diagnostic, got: {:?}",
                diags.iter().map(|d| &d.message).collect::<Vec<_>>()
            )
        });
    assert!(
        hit.message.contains("locus `Step`"),
        "names the locus: {}",
        hit.message
    );
    assert!(
        hit.related
            .iter()
            .any(|(_, label)| label.contains("first `accept` declared here")),
        "carries the first clause as related: {:?}",
        hit.related
    );
    // The FIRST clause is the one that survives: `Work` is still
    // accepted, so the `Work { }` literal inside `run()` resolves
    // an owner and raises nothing of its own.
    let (first_span, _) = hit.related[0];
    assert!(
        first_span.start < hit.span.start,
        "related span points at the earlier clause: {:?} vs {:?}",
        first_span,
        hit.span
    );
}

#[test]
fn one_accept_clause_is_clean() {
    let src = TWO_ACCEPTS.replace("    accept(t: Task) { }\n", "");
    let prog = parse_source(&src).expect("parse");
    let diags = check_program(&prog);
    assert!(
        diags.iter().all(|d| !d.message.contains("declares `accept` twice")),
        "got: {:?}",
        diags.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}
